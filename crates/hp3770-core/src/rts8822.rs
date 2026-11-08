// SPDX-License-Identifier: GPL-2.0-or-later
//! The driver behind the scanner contract ([`scanner_api::Scanner`]): a
//! chip of the RTS8822 family with the profile of its model.
//!
//! What the application gets is what it used to ask [`session::scan`] for:
//! the chip's linear colour samples (no tone curve, no sharpening), at 8
//! bits and also at 16 when asked, with the carriage brought near the area
//! at high speed.

use crate::device::Button;
use crate::profile::Profile;
use crate::session::{self, Report, Request, Stage};
use crate::status::{Lamp, Status};
use crate::transport::{Chip, Transport};
use crate::{geometry, init, scan};
use scan_imaging::adjust::Adjustments;
use scan_imaging::Mode;
use scanner_api::{
    ButtonInfo, Capabilities, ColourMode, DeviceStatus, Error, LampInfo, LampState, Progress,
    ScanRequest, ScanResult, Scanner, SuggestedAction,
};

/// Name of this backend in the [`scanner_api::Registry`].
pub const BACKEND: &str = "rts8822";

/// What a scanner with this profile can do.
pub fn capabilities(profile: &Profile) -> Capabilities {
    let d = &profile.device;
    Capabilities {
        manufacturer: d.manufacturer.into(),
        model: d.name.into(),
        glass_inches: d.glass_inches(),
        resolutions: d.resolutions.to_vec(),
        modes: vec![ColourMode::Colour, ColourMode::Grey, ColourMode::Lineart],
        depths: vec![8, 16],
        // The 3770 has no adapter for slides (the chip could drive one).
        transparency: false,
        buttons: d.buttons.iter().map(|&b| button_info(b)).collect(),
        lamp: Some(LampInfo {
            warm_up: scan::LAMP_WARMUP,
        }),
        tone: profile.tone(),
    }
}

fn button_info(button: Button) -> ButtonInfo {
    let (id, action) = match button {
        Button::Scan => ("scan", SuggestedAction::Scan),
        Button::Copy => ("copy", SuggestedAction::Copy),
        Button::InstantShare => ("instant-share", SuggestedAction::Share),
    };
    ButtonInfo {
        id: id.into(),
        label: button.name().into(),
        action,
    }
}

/// The scanner's state for the application.
pub fn device_status(s: &Status) -> DeviceStatus {
    DeviceStatus {
        ready: s.init_done && !s.executing,
        lamp: Some(match s.lamp {
            Lamp::Off => LampState::Off,
            Lamp::Flatbed => LampState::On,
            Lamp::Transparency => LampState::Transparency,
        }),
        adapter: None,
        details: vec![
            ("USB".into(), if s.usb2 { "2.0" } else { "1.1" }.into()),
            ("Chip".into(), format!("RTS8822, chipset {}", s.chipset_id)),
            (
                "Carrello".into(),
                if s.head_at_home {
                    "a casa"
                } else {
                    "fuori casa"
                }
                .into(),
            ),
            ("Link".into(), format!("{:#06x}", s.link_word)),
        ],
    }
}

/// The buttons `held` (the chip's mask) as bits of the contract: bit `i`
/// for the `i`-th button of the profile.
pub fn button_bits(profile: &Profile, held: u8) -> u32 {
    profile
        .device
        .buttons
        .iter()
        .enumerate()
        .filter(|(_, b)| held & b.mask() != 0)
        .fold(0, |bits, (i, _)| bits | 1 << i)
}

/// A driver error as the contract tells it.
pub fn api_error(e: crate::Error) -> Error {
    match e {
        crate::Error::Cancelled(_) => Error::Cancelled,
        crate::Error::Transport(why) => Error::Disconnected(why),
        crate::Error::InvalidArgument(why) => Error::InvalidRequest(why),
        e => Error::Other(e.to_string()),
    }
}

/// The request [`session::scan`] gets for a scan of the contract: linear
/// colour, nothing done on the host.
pub fn session_request(request: &ScanRequest, fast_travel: bool) -> Request {
    let a = request.area;
    Request {
        linear: true,
        adjust: Adjustments {
            sharpen: 0.0,
            ..Adjustments::default()
        },
        depth: request.depth,
        fast_travel,
        ..Request::new(
            request.dpi,
            geometry::Area {
                x: a.x,
                y: a.y,
                width: a.width,
                height: a.height,
            },
            Mode::Colour,
        )
    }
}

/// The pixels of the white shading far from their neighbours' (dust on the
/// white strip, or a weak sensor pixel), one line each.
pub fn shading_outliers(report: &Report) -> Vec<String> {
    let mut out = Vec::new();
    for (c, name) in ["red", "green", "blue"].iter().enumerate() {
        let w = &report.white[c];
        for x in 0..w.len() {
            let mut around: Vec<u16> = (x.saturating_sub(6)..(x + 7).min(w.len()))
                .filter(|&i| i.abs_diff(x) > 1)
                .map(|i| w[i])
                .collect();
            if around.is_empty() {
                continue;
            }
            around.sort_unstable();
            let median = f64::from(around[around.len() / 2]);
            let ratio = f64::from(w[x]) / median;
            if !(0.9..=1.1).contains(&ratio) {
                out.push(format!(
                    "{name} pixel {x}: x{:.3} (neighbours x{:.3})",
                    f64::from(w[x]) / 8192.0,
                    median / 8192.0
                ));
            }
        }
    }
    out
}

/// What else a scan tells, for the curious.
fn details(report: &Report) -> Vec<(String, String)> {
    let mut d = vec![
        (
            "lamp".into(),
            if report.warmed_up {
                "was off, warmed up"
            } else {
                "was on"
            }
            .into(),
        ),
        (
            "calibration".into(),
            format!("offsets {:?}, gains {:?}", report.offsets, report.gains),
        ),
        (
            "chip buffer".into(),
            format!(
                "at most {:.1} MB waiting",
                report.peak_buffered as f64 / 1e6
            ),
        ),
    ];
    d.extend(
        shading_outliers(report)
            .into_iter()
            .map(|line| ("white shading".into(), line)),
    );
    d
}

fn progress_of(stage: Stage) -> Progress {
    match stage {
        Stage::Starting => Progress::Starting,
        Stage::WarmingUp => Progress::WarmingUp,
        Stage::Calibrating => Progress::Calibrating,
        Stage::Scanning { done, total } => Progress::Scanning { done, total },
        Stage::Finishing => Progress::Finishing,
    }
}

/// A scanner of the RTS8822 family, open.
#[derive(Debug)]
pub struct Rts8822<T> {
    chip: Chip<T>,
    profile: &'static Profile,
    caps: Capabilities,
    fast_travel: bool,
}

impl<T: Transport> Rts8822<T> {
    /// The scanner reached through `transport`, a model of `profile`.
    pub fn new(transport: T, profile: &'static Profile) -> Self {
        Self {
            chip: Chip::new(transport),
            profile,
            caps: capabilities(profile),
            fast_travel: true,
        }
    }

    /// Whether the carriage goes near the area at high speed before a scan
    /// at 600 dpi and up (on unless switched off).
    pub fn fast_travel(mut self, on: bool) -> Self {
        self.fast_travel = on;
        self
    }

    /// The chip, for what only this driver does (registers, EEPROM).
    pub fn chip(&mut self) -> &mut Chip<T> {
        &mut self.chip
    }
}

impl<T: Transport + Send> Scanner for Rts8822<T> {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn status(&mut self) -> scanner_api::Result<DeviceStatus> {
        self.chip
            .status()
            .map(|s| device_status(&s))
            .map_err(api_error)
    }

    fn scan(
        &mut self,
        request: &ScanRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> scanner_api::Result<ScanResult> {
        scanner_api::check(&self.caps, request)?;
        let session = session_request(request, self.fast_travel);
        let (image, report) = session::scan(&mut self.chip, self.profile, &session, &mut |s| {
            progress(progress_of(s))
        })
        .map_err(api_error)?;
        let details = details(&report);
        Ok(ScanResult {
            image,
            samples16: report.samples16,
            details,
        })
    }

    fn poll_buttons(&mut self) -> scanner_api::Result<u32> {
        let held = self.chip.buttons_held().map_err(api_error)?;
        Ok(button_bits(self.profile, held))
    }

    fn lamp_off(&mut self) -> scanner_api::Result<()> {
        // lamp_flatbed reads the chip's block; the copy a scan keeps is not
        // needed to switch off.
        let mut registers = init::Registers::default();
        self.chip
            .lamp_flatbed(&mut registers, false)
            .map_err(api_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::HP3770;
    use scan_imaging::adjust;

    #[test]
    fn the_3770_capabilities_are_its_profile() {
        let c = capabilities(&HP3770);
        assert_eq!(c.name(), "HP Scanjet 3770");
        assert_eq!(c.glass_inches, geometry::GLASS_INCHES);
        assert_eq!(c.resolutions, geometry::RESOLUTIONS);
        assert_eq!(c.max_dpi(), 2400);
        let labels: Vec<_> = c.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["Scan", "Copy", "Instant Share"]);
        assert_eq!(c.tone, HP3770.tone());
        // The glass in pixels is what the driver accepts, to the pixel.
        for dpi in geometry::RESOLUTIONS {
            let (w, h) = c.glass_pixels(dpi);
            let edge = geometry::Area {
                x: 0,
                y: 0,
                width: w,
                height: h,
            };
            assert!(geometry::scan_window(dpi, edge).is_ok(), "{dpi}");
        }
    }

    #[test]
    fn the_request_is_the_one_the_app_made() {
        let r = session_request(
            &ScanRequest {
                dpi: 600,
                area: scanner_api::Area {
                    x: 1,
                    y: 2,
                    width: 3,
                    height: 4,
                },
                depth: 16,
            },
            true,
        );
        assert!(r.linear && r.fast_travel);
        assert_eq!(r.mode, Mode::Colour);
        assert_eq!(r.depth, 16);
        assert_eq!(r.adjust.sharpen, 0.0);
        assert_eq!(
            (r.area.x, r.area.y, r.area.width, r.area.height),
            (1, 2, 3, 4)
        );
    }

    /// The samples of the contract are the chip's untouched: processing them
    /// afterwards gives what processing in the session gave.
    #[test]
    fn linear_colour_leaves_every_sample_as_it_is() {
        let rgb: Vec<u8> = (0..=255u8)
            .flat_map(|v| [v, 255 - v, v.wrapping_mul(7)])
            .collect();
        let request = session_request(
            &ScanRequest {
                dpi: 300,
                area: scanner_api::Area {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 4,
                },
                depth: 8,
            },
            true,
        );
        let out = session::process(rgb.clone(), 64, 4, &request, &HP3770);
        assert_eq!(out.data, rgb);
        // And so the image the command line made in one go is the same.
        for mode in [Mode::Colour, Mode::Grey, Mode::Lineart { threshold: 55 }] {
            let a = Adjustments::default();
            let direct = adjust::render(&rgb, 64, 4, 300, mode, &a, false, &HP3770.tone());
            let after = adjust::render(&out.data, 64, 4, 300, mode, &a, false, &HP3770.tone());
            assert_eq!(direct, after, "{mode:?}");
        }
    }

    #[test]
    fn buttons_map_to_their_bits() {
        let held = Button::Copy.mask() | Button::InstantShare.mask();
        assert_eq!(button_bits(&HP3770, held), 0b110);
        assert_eq!(button_bits(&HP3770, Button::Scan.mask()), 0b001);
        assert_eq!(button_bits(&HP3770, 0), 0);
    }

    #[test]
    fn errors() {
        assert_eq!(api_error(crate::Error::Cancelled("scan")), Error::Cancelled);
        assert!(matches!(
            api_error(crate::Error::Transport("gone".into())),
            Error::Disconnected(_)
        ));
    }
}
