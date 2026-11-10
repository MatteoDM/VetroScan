// SPDX-License-Identifier: GPL-2.0-or-later
//! The driver behind the scanner contract ([`scanner_api::Scanner`]): a
//! chip of the RTS8822 family with the profile of its model.
//!
//! What the application gets is what it used to ask [`session::scan`] for:
//! the chip's linear colour samples (no tone curve, no sharpening), at 8
//! bits and also at 16 when asked, with the carriage brought near the area
//! at high speed. Films go through the adapter in the lid (phase 10 of
//! docs/architecture-plan.md): its lamp, its calibration on the slot
//! without film, the negatives' scan modes; their samples are the film's
//! transmission, turned into the picture by the application.

use crate::device::Button;
use crate::profile::Profile;
use crate::session::{self, Report, Request, Stage};
use crate::status::{Lamp, Status};
use crate::transport::{Chip, Transport};
use crate::{geometry, init, scan};
use scan_imaging::adjust::Adjustments;
use scan_imaging::Mode;
use scanner_api::{
    Adapter, ButtonInfo, Capabilities, ColourCorrection, ColourMatrix, ColourMode, DeviceStatus,
    Error, LampInfo, LampState, Progress, ScanRequest, ScanResult, Scanner, Source, Stripes,
    SuggestedAction,
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
        // The 3770's adapter in its lid (the "TMA"): slides and negatives,
        // both with the negatives' longer line (`session_request`), at the
        // resolutions it was run at.
        transparency: profile.transparency.as_ref().map(|t| Adapter {
            area_inches: (f64::from(t.area[2]) / 254.0, f64::from(t.area[3]) / 254.0),
            sources: vec![Source::Slide, Source::Negative],
            resolutions: crate::scan_modes::NEGATIVE_RESOLUTIONS.to_vec(),
        }),
        buttons: d.buttons.iter().map(|&b| button_info(b)).collect(),
        lamp: Some(LampInfo {
            warm_up: scan::LAMP_WARMUP,
        }),
        tone: profile.tone(),
        // The glass's, and the films': slides are scanned with the
        // negatives' modes, and on the skies of two slides at 1200 dpi the
        // stripes repeat every 5 lines as on negatives (measured on 10
        // November 2026); at 600 dpi measured on negatives only.
        stripes: d
            .stripes
            .iter()
            .map(|&(dpi, repeat)| Stripes {
                source: Source::Glass,
                dpi,
                repeat,
            })
            .chain(profile.transparency.iter().flat_map(|t| {
                [Source::Slide, Source::Negative]
                    .into_iter()
                    .flat_map(|source| {
                        t.stripes.iter().map(move |&(dpi, repeat)| Stripes {
                            source,
                            dpi,
                            repeat,
                        })
                    })
            }))
            .collect(),
        colour: profile
            .transparency
            .iter()
            .filter_map(|t| t.slide_matrix)
            .map(|m| ColourCorrection {
                source: Source::Slide,
                matrix: slide_colour(m),
            })
            .collect(),
    }
}

/// The correction of the slides' colours from HP's matrix `m`
/// (thousandths). HP's rows add up to 0.87-0.94 because its driver follows
/// the matrix with offsets and gammas of its own (`SLIDECOEFFS`): on our
/// picture that would turn grey magenta, so each row is scaled to add up
/// to 1. Applied to the values as displayed (gamma 2.2), where on the
/// slides tried it clipped 0.5 % of the pixels against 2.8 % on linear
/// light.
fn slide_colour(m: [i16; 9]) -> ColourMatrix {
    let mut rows = m.map(|v| f32::from(v) / 1000.0);
    for row in rows.chunks_exact_mut(3) {
        let sum: f32 = row.iter().sum();
        row.iter_mut().for_each(|v| *v /= sum);
    }
    ColourMatrix { rows, gamma: 2.2 }
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

/// The scanner's state for the application; `adapter`: whether the model
/// has a film adapter.
pub fn device_status(s: &Status, adapter: bool) -> DeviceStatus {
    DeviceStatus {
        // Not initialised yet is ready too: a scan starts by initialising.
        ready: !s.executing,
        lamp: Some(match s.lamp {
            Lamp::Off => LampState::Off,
            Lamp::Flatbed => LampState::On,
            Lamp::Transparency => LampState::Transparency,
        }),
        adapter: adapter.then_some(s.tma_attached),
        details: vec![
            ("USB".into(), if s.usb2 { "2.0" } else { "1.1" }.into()),
            ("Chip".into(), format!("RTS8822, chipset {}", s.chipset_id)),
            (
                "Carriage".into(),
                if s.head_at_home {
                    "at home"
                } else {
                    "away from home"
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
/// colour, nothing done on the host. A film's area, given within the
/// adapter's, becomes one of the glass with `profile`'s adapter.
pub fn session_request(request: &ScanRequest, fast_travel: bool, profile: &Profile) -> Request {
    let a = request.area;
    let film = request.source.is_film();
    // Where the adapter's area starts on the glass, in pixels of the scan.
    let (x0, y0) = match (&profile.transparency, film) {
        (Some(t), true) => {
            let origin = geometry::Area::from_mm(
                request.dpi,
                f64::from(t.area[0]) / 10.0,
                f64::from(t.area[1]) / 10.0,
                0.0,
                0.0,
            );
            (origin.x, origin.y)
        }
        _ => (0, 0),
    };
    Request {
        linear: true,
        adjust: Adjustments {
            sharpen: 0.0,
            ..Adjustments::default()
        },
        depth: request.depth,
        fast_travel,
        transparency: film,
        // Slides too: the longer line halves the noise in their shadows
        // (measured on 10 November 2026 against the glass's line, which is
        // HP's for slides), nothing saturated.
        negative: film,
        ..Request::new(
            request.dpi,
            geometry::Area {
                x: x0 + a.x,
                y: y0 + a.y,
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
        let adapter = self.caps.transparency.is_some();
        self.chip
            .status()
            .map(|s| device_status(&s, adapter))
            .map_err(api_error)
    }

    fn scan(
        &mut self,
        request: &ScanRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> scanner_api::Result<ScanResult> {
        scanner_api::check(&self.caps, request)?;
        if request.source.is_film() && !self.chip.status().map_err(api_error)?.tma_attached {
            return Err(Error::Other(
                "the film adapter's cable is not plugged in at the back of the scanner".into(),
            ));
        }
        let session = session_request(request, self.fast_travel, self.profile);
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
        // The adapter for slides and negatives, its area the light measured
        // in the lid (down to 183 mm: the slides' holder reaches further
        // than the negatives').
        let adapter = c.transparency.as_ref().expect("an adapter");
        assert_eq!(adapter.sources, [Source::Slide, Source::Negative]);
        assert!(c.offers(Source::Negative) && c.offers(Source::Slide));
        assert_eq!(c.bed_pixels(Source::Negative, 200), Some((346, 841)));
        assert_eq!(c.bed_pixels(Source::Slide, 200), Some((346, 841)));
        assert_eq!(c.resolutions_for(Source::Negative), [600, 1200]);
        assert_eq!(c.resolutions_for(Source::Slide), [600, 1200]);
        // The stripes, per source: the glass's, and the films' (the same
        // modes for slides and negatives) at both film resolutions.
        assert_eq!(c.stripes_at(Source::Glass, 300), Some(10));
        assert_eq!(c.stripes_at(Source::Negative, 600), Some(5));
        assert_eq!(c.stripes_at(Source::Negative, 1200), Some(5));
        assert_eq!(c.stripes_at(Source::Negative, 300), None);
        assert_eq!(c.stripes_at(Source::Slide, 1200), Some(5));
        assert_eq!(c.stripes_at(Source::Slide, 600), Some(5));
        // HP's colour matrix for slides, its rows scaled to 1: grey stays
        // grey; none for negatives or the glass.
        let slide = c.colour_at(Source::Slide).expect("slides' colour");
        assert_eq!(slide.gamma, 2.2);
        for row in slide.rows.chunks_exact(3) {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{row:?}");
        }
        assert!((slide.rows[0] - 1.330 / 0.941).abs() < 1e-4);
        assert_eq!(c.colour_at(Source::Negative), None);
        assert_eq!(c.colour_at(Source::Glass), None);
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
                source: Source::Glass,
                area: scanner_api::Area {
                    x: 1,
                    y: 2,
                    width: 3,
                    height: 4,
                },
                depth: 16,
            },
            true,
            &HP3770,
        );
        assert!(r.linear && r.fast_travel);
        assert!(!r.transparency && !r.negative);
        assert_eq!(r.mode, Mode::Colour);
        assert_eq!(r.depth, 16);
        assert_eq!(r.adjust.sharpen, 0.0);
        assert_eq!(
            (r.area.x, r.area.y, r.area.width, r.area.height),
            (1, 2, 3, 4)
        );
    }

    /// A negative's area is given within the adapter's: on the glass it is
    /// where the adapter's light falls (87.6 x 76.2 mm in), as `tma-scan`
    /// asks for it.
    #[test]
    fn a_negative_is_scanned_where_the_adapter_lights_the_glass() {
        let (w, h) = capabilities(&HP3770)
            .bed_pixels(Source::Negative, 1200)
            .unwrap();
        let r = session_request(
            &ScanRequest {
                dpi: 1200,
                source: Source::Negative,
                area: scanner_api::Area {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                },
                depth: 16,
            },
            true,
            &HP3770,
        );
        assert!(r.transparency && r.negative);
        let cli = geometry::Area::from_mm(1200, 87.6, 76.2, 44.0, 106.8);
        assert_eq!(
            (r.area.x, r.area.y, r.area.width, r.area.height),
            (cli.x, cli.y, cli.width, cli.height)
        );
        // Slides: the same area, the same longer line (so the same guard on
        // the resolutions, in `session::scan`).
        let slide = session_request(
            &ScanRequest {
                source: Source::Slide,
                dpi: 1200,
                area: scanner_api::Area {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                },
                depth: 16,
            },
            true,
            &HP3770,
        );
        assert!(slide.transparency && slide.negative);
        assert_eq!(slide.area, r.area);
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
                source: Source::Glass,
                area: scanner_api::Area {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 4,
                },
                depth: 8,
            },
            true,
            &HP3770,
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
