// SPDX-License-Identifier: GPL-2.0-or-later
//! A whole scan, from the start-up of the chip to the finished image.
//!
//! [`scan`] puts together what the HP driver does for one scan: start-up,
//! carriage home, preparation, lamp warm-up, calibration, scan, stop, then
//! on the host the line arrangement (2400 dpi), the tone curve, the
//! sharpening and the grey or black-and-white conversion. The command line
//! tool and the desktop application both use it.

use crate::arrange;
use crate::error::{Error, Result};
use crate::geometry::{self, Area};
use crate::motor::{park_registers, Direction, MotorPosition};
use crate::profile::Profile;
use crate::scan::LAMP_WARMUP;
use crate::setup::ArrangeLine;
use crate::transport::{Chip, Transport};
use scan_imaging::adjust::{self, Adjustments};

use scan_imaging::{Image, Mode};

/// A scan to make.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Request {
    /// Resolution: one of [`geometry::RESOLUTIONS`].
    pub dpi: u16,
    /// Area of the glass, in pixels of `dpi`.
    pub area: Area,
    /// Colour, grey or black and white.
    pub mode: Mode,
    /// Adjustments applied on the host (white balance, levels, colour,
    /// sharpening). The default gives the HP driver's result.
    pub adjust: Adjustments,
    /// Keep the chip's linear samples (no tone curve).
    pub linear: bool,
    /// Bring the carriage near the area at high speed before a scan at
    /// 600 dpi and up (see [`fast_travel`]).
    pub fast_travel: bool,
    /// Bits per sample asked of the chip: 8, or 16 (the samples are then
    /// also in [`Report::samples16`]).
    pub depth: u8,
    /// Scan with the transparency adapter: its lamp switched on and given
    /// [`TMA_WARMUP`], the calibration on its slot without film
    /// ([`Chip::calibrate_transparency`]); at the end its lamp is switched
    /// off and the flatbed lamp on again, as hp3900 does. The area is still
    /// one of the glass (the adapter's is in
    /// [`crate::profile::Transparency::area`]). Experimental: checked on
    /// one scanner, no capture of the HP driver to compare with.
    pub transparency: bool,
}

/// The transparency adapter's lamp warms up for at least this long (HP's
/// user's manual: at least 30 seconds before scanning slides or negatives).
pub const TMA_WARMUP: std::time::Duration = std::time::Duration::from_secs(30);

impl Request {
    /// A request with the HP driver's defaults for `mode`.
    pub fn new(dpi: u16, area: Area, mode: Mode) -> Self {
        Self {
            dpi,
            area,
            mode,
            adjust: Adjustments::default(),
            linear: false,
            fast_travel: true,
            depth: 8,
            transparency: false,
        }
    }
}

/// Where a scan is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Start-up of the chip, carriage home, preparation.
    Starting,
    /// The lamp was off: waiting for it to warm up.
    WarmingUp,
    /// Offsets, gains and shading.
    Calibrating,
    /// Image data: bytes read so far, out of the total.
    Scanning {
        /// Bytes read.
        done: usize,
        /// Bytes of the whole scan.
        total: usize,
    },
    /// Stop, carriage home, image processing.
    Finishing,
}

/// What [`scan`] learnt besides the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The lamp was off and had to warm up.
    pub warmed_up: bool,
    /// Gains of red, green and blue after calibration.
    pub gains: [u8; 3],
    /// Even offsets of red, green and blue after calibration.
    pub offsets: [u16; 3],
    /// Most image data seen waiting in the chip, in bytes (see
    /// [`crate::scan::Scan::peak_buffered`]).
    pub peak_buffered: usize,
    /// White shading multipliers of red, green and blue, one per pixel of
    /// the chip's line (8192 = 1).
    pub white: [Vec<u16>; 3],
    /// With a depth of 16: the chip's samples (RGB, the chip sends each
    /// most significant byte first), arranged like the image, before any
    /// host processing.
    pub samples16: Option<Vec<u16>>,
}

/// The carriage move that brings a scan at `resolution_y` starting at line
/// `top` (from the home position) near its area, and the top of the scan
/// after it: `(full motor steps, new top)` (`Head_Relocate` in hp3900's
/// `Scan_Start`).
///
/// At 600 dpi and up the carriage would travel to the area at scan speed
/// (about 7.5 s/cm at 2400 dpi). hp3900 moves it first at the speed of
/// the park, leaving a tenth of an inch of travel (`resolution_y / 10`
/// lines) for the scan to get up to speed. Not in the captures, whose
/// high-resolution scans start at the top of the glass.
pub fn fast_travel(profile: &Profile, resolution_y: u16, top: u32) -> Option<(u16, u32)> {
    let lead = u32::from(resolution_y) / 10;
    if resolution_y < 600 || top <= lead * 2 {
        return None;
    }
    let lines = top - lead;
    let steps =
        u64::from(lines) * u64::from(profile.platform.motor_resolution) / u64::from(resolution_y);
    Some((u16::try_from(steps).ok()?, top - lines))
}

/// Makes the scan `request`.
///
/// `progress` is called at every stage and after every piece of image data;
/// returning `false` cancels the scan: the chip is stopped, the carriage
/// brought home and [`Error::Cancelled`] returned.
pub fn scan<T: Transport>(
    chip: &mut Chip<T>,
    profile: &Profile,
    request: &Request,
    progress: &mut dyn FnMut(Stage) -> bool,
) -> Result<(Image, Report)> {
    let (mut params, hw) = geometry::scan_window(request.dpi, request.area)?;
    match request.depth {
        8 => {}
        16 => params.depth = 16,
        d => return Err(Error::InvalidArgument(format!("depth {d}"))),
    }
    let gamma = geometry::identity_gamma();
    if !progress(Stage::Starting) {
        return Err(Error::Cancelled("start"));
    }
    let link = chip.link(profile)?;
    let mut init = link.registers.clone();
    if !link.at_home {
        chip.head_park_home(&park_registers(&init), &profile.park)?;
    }
    chip.prepare_scan(&mut init, profile)?;
    if !link.lamp_was_on {
        // The start-up switched the lamp on: let it warm up.
        if !progress(Stage::WarmingUp) {
            return Err(Error::Cancelled("lamp warm-up"));
        }
        std::thread::sleep(LAMP_WARMUP);
    }
    if request.transparency {
        chip.lamp_transparency(&mut init, true)?;
        if !progress(Stage::WarmingUp) {
            chip.lamp_transparency(&mut init, false)?;
            chip.lamp_flatbed(&mut init, true)?;
            return Err(Error::Cancelled("lamp warm-up"));
        }
        std::thread::sleep(TMA_WARMUP);
    }
    if !progress(Stage::Calibrating) {
        if request.transparency {
            chip.lamp_transparency(&mut init, false)?;
            chip.lamp_flatbed(&mut init, true)?;
        }
        return Err(Error::Cancelled("calibration"));
    }
    let calibration = if request.transparency {
        chip.calibrate_transparency(&mut init, profile, &params)?
    } else {
        chip.calibrate(&mut init, profile, &params)?
    };
    let mut report = Report {
        warmed_up: !link.lamp_was_on,
        gains: calibration.gain_offset.gain,
        offsets: calibration.gain_offset.even_offset,
        peak_buffered: 0,
        white: calibration.white.clone().map(|t| {
            t.chunks_exact(2)
                .map(|w| u16::from_le_bytes([w[0], w[1]]))
                .collect()
        }),
        samples16: None,
    };
    let tables = calibration.tables(Some(&gamma));
    if request.fast_travel {
        if let Some((steps, top)) = fast_travel(profile, params.resolution_y, params.top) {
            chip.motor_move(
                &park_registers(&init),
                &profile.park,
                &MotorPosition {
                    direction: Direction::Forward,
                    e801_bit2: false,
                    steps,
                },
            )?;
            params.top = top;
        }
    }
    let mut job = chip.start_scan(
        &init,
        profile,
        &params,
        &hw,
        &calibration.gain_offset,
        &tables,
    )?;
    let mut raw = Vec::with_capacity(job.total());
    let mut piece = Vec::new();
    let mut stopped = false;
    while chip.read_scan(&mut job, &mut piece)? > 0 {
        raw.extend_from_slice(&piece);
        if !progress(Stage::Scanning {
            done: job.bytes_read(),
            total: job.total(),
        }) {
            stopped = true;
            break;
        }
    }
    report.peak_buffered = job.peak_buffered();
    progress(Stage::Finishing);
    chip.stop_scan(&mut init, profile)?;
    // The chip brings the carriage back by itself below 600 dpi, and not
    // when the scan was cut short: make sure it is home.
    chip.head_park_home(&park_registers(&init), &profile.park)?;
    if request.transparency {
        chip.lamp_transparency(&mut init, false)?;
        chip.lamp_flatbed(&mut init, true)?;
    }
    if stopped {
        return Err(Error::Cancelled("scan"));
    }

    let mut height = raw.len() / job.bytes_per_line;
    raw.truncate(height * job.bytes_per_line);
    let bytes_per_sample = usize::from(params.depth / 8).max(1);
    let mut width = job.bytes_per_line / 3 / bytes_per_sample;
    let mut samples16: Option<Vec<u16>> = (params.depth == 16).then(|| {
        raw.chunks_exact(2)
            .map(|w| u16::from_be_bytes([w[0], w[1]]))
            .collect()
    });
    if hw.arrange_line == ArrangeLine::Software {
        // 2400 dpi: lines arranged here, width doubled.
        let d = arrange::Distances::of(profile, &params, hw.high_resolution);
        height = params.height as usize;
        if let Some(s) = samples16.take() {
            samples16 = Some(arrange::double_width(
                &arrange::arrange(&s, width, height, &d),
                width,
            ));
        } else {
            raw = arrange::double_width(&arrange::arrange(&raw, width, height, &d), width);
        }
        width *= 2;
    }
    if let Some(s) = samples16 {
        // The image gets the top 8 bits of each sample.
        raw = s.iter().map(|&v| (v >> 8) as u8).collect();
        report.samples16 = Some(s);
    }
    Ok((process(raw, width, height, request, profile), report))
}

/// The host side of a scan: the adjustments and the conversion of the
/// chip's linear RGB samples (`width` × `height`), see
/// [`adjust::render`].
pub fn process(
    rgb: Vec<u8>,
    width: usize,
    height: usize,
    request: &Request,
    profile: &Profile,
) -> Image {
    adjust::render(
        &rgb,
        width,
        height,
        request.dpi,
        request.mode,
        &request.adjust,
        request.linear,
        &profile.tone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::HP3770;
    use scan_imaging::Format;

    fn request(mode: Mode) -> Request {
        Request {
            linear: true,
            adjust: Adjustments {
                sharpen: 0.0,
                ..Adjustments::default()
            },
            ..Request::new(
                300,
                Area {
                    x: 0,
                    y: 0,
                    width: 8,
                    height: 2,
                },
                mode,
            )
        }
    }

    #[test]
    fn processing_of_each_mode() {
        let rgb: Vec<u8> = (0..8 * 2).flat_map(|i| [i as u8 * 10, 0, 0]).collect();
        let colour = process(rgb.clone(), 8, 2, &request(Mode::Colour), &HP3770);
        assert_eq!(colour.data, rgb);
        let grey = process(rgb.clone(), 8, 2, &request(Mode::Grey), &HP3770);
        assert_eq!(grey.format, Format::Grey);
        assert_eq!(&grey.data[..3], &[0, 3, 7]);
        let bw = process(
            rgb,
            8,
            2,
            &request(Mode::Lineart { threshold: 200 }),
            &HP3770,
        );
        // White from grey 55: pixels 0-7 are 0..23, pixels 8-15 27..50.
        assert_eq!(bw.data, [0, 0]);
        assert_eq!(bw.to_rgb().len(), 8 * 2 * 3);
    }
}
