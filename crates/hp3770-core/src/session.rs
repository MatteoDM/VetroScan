// SPDX-License-Identifier: GPL-2.0-or-later
//! A whole scan, from the start-up of the chip to the finished image.
//!
//! [`scan`] puts together what the HP driver does for one scan: start-up,
//! carriage home, preparation, lamp warm-up, calibration, scan, stop, then
//! on the host the line arrangement (2400 dpi), the tone curve, the
//! sharpening and the grey or black-and-white conversion. The command line
//! tool and the desktop application both use it.

use crate::arrange;
use crate::convert;
use crate::error::{Error, Result};
use crate::geometry::{self, Area};
use crate::motor::park_registers;
use crate::profile::Profile;
use crate::scan::LAMP_WARMUP;
use crate::setup::ArrangeLine;
use crate::transport::{Chip, Transport};

/// What the image holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 8-bit RGB.
    Colour,
    /// 8-bit grey.
    Grey,
    /// Black and white, white from `255 − threshold` up.
    Lineart {
        /// The HP driver's `Threshold` (default
        /// [`convert::DEFAULT_THRESHOLD`]).
        threshold: u8,
    },
}

/// A scan to make.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Request {
    /// Resolution: one of [`geometry::RESOLUTIONS`].
    pub dpi: u16,
    /// Area of the glass, in pixels of `dpi`.
    pub area: Area,
    /// Colour, grey or black and white.
    pub mode: Mode,
    /// Sharpening amount ([`convert::HP_SHARPEN`] as the HP driver, 0 for
    /// none).
    pub sharpen: f32,
    /// Keep the chip's linear samples (no tone curve).
    pub linear: bool,
}

impl Request {
    /// A request with the HP driver's defaults for `mode`.
    pub fn new(dpi: u16, area: Area, mode: Mode) -> Self {
        Self {
            dpi,
            area,
            mode,
            sharpen: convert::HP_SHARPEN,
            linear: false,
        }
    }
}

/// Pixel layout of an [`Image`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 3 bytes per pixel, red, green, blue.
    Rgb,
    /// 1 byte per pixel.
    Grey,
    /// 1 bit per pixel, first pixel in the high bit, lines padded to a
    /// byte, 1 = white.
    Lineart,
}

/// A finished image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Pixels per line.
    pub width: usize,
    /// Lines.
    pub height: usize,
    /// Resolution in dpi, both ways.
    pub dpi: u16,
    /// Pixel layout of `data`.
    pub format: Format,
    /// The pixels, line after line.
    pub data: Vec<u8>,
}

impl Image {
    /// The image as 8-bit RGB (grey and black and white expanded), for
    /// display.
    pub fn to_rgb(&self) -> Vec<u8> {
        match self.format {
            Format::Rgb => self.data.clone(),
            Format::Grey => self.data.iter().flat_map(|&g| [g, g, g]).collect(),
            Format::Lineart => {
                let stride = self.width.div_ceil(8);
                let mut out = Vec::with_capacity(self.width * self.height * 3);
                for line in self.data.chunks_exact(stride) {
                    for x in 0..self.width {
                        let v = if line[x / 8] & (0x80 >> (x % 8)) != 0 {
                            255
                        } else {
                            0
                        };
                        out.extend_from_slice(&[v, v, v]);
                    }
                }
                out
            }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// The lamp was off and had to warm up.
    pub warmed_up: bool,
    /// Gains of red, green and blue after calibration.
    pub gains: [u8; 3],
    /// Even offsets of red, green and blue after calibration.
    pub offsets: [u16; 3],
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
    let (params, hw) = geometry::scan_window(request.dpi, request.area)?;
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
    if !progress(Stage::Calibrating) {
        return Err(Error::Cancelled("calibration"));
    }
    let calibration = chip.calibrate(&mut init, profile, &params)?;
    let report = Report {
        warmed_up: !link.lamp_was_on,
        gains: calibration.gain_offset.gain,
        offsets: calibration.gain_offset.even_offset,
    };
    let tables = calibration.tables(Some(&gamma));
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
    progress(Stage::Finishing);
    chip.stop_scan(&mut init, profile)?;
    // The chip brings the carriage back by itself below 600 dpi, and not
    // when the scan was cut short: make sure it is home.
    chip.head_park_home(&park_registers(&init), &profile.park)?;
    if stopped {
        return Err(Error::Cancelled("scan"));
    }

    let mut width = job.bytes_per_line / 3;
    let mut height = raw.len() / job.bytes_per_line;
    raw.truncate(height * job.bytes_per_line);
    if hw.arrange_line == ArrangeLine::Software {
        // 2400 dpi: lines arranged here, width doubled.
        let d = arrange::Distances::of(profile, &params, hw.high_resolution);
        height = params.height as usize;
        raw = arrange::double_width(&arrange::arrange(&raw, width, height, &d), width);
        width *= 2;
    }
    Ok((process(raw, width, height, request, profile), report))
}

/// The host side of a scan: tone curve, sharpening and conversion of the
/// chip's linear RGB samples (`width` × `height`).
pub fn process(
    mut rgb: Vec<u8>,
    width: usize,
    height: usize,
    request: &Request,
    profile: &Profile,
) -> Image {
    let identity: [u8; 256] = std::array::from_fn(|i| i as u8);
    let tone = |white: f64| {
        if request.linear {
            identity
        } else {
            geometry::host_tone(white, 2.2)
        }
    };
    let grey_tone = tone(geometry::host_white(profile));
    let (format, data) = match request.mode {
        Mode::Colour => {
            let tones = geometry::HP_WHITE.map(tone);
            for (i, sample) in rgb.iter_mut().enumerate() {
                *sample = tones[i % 3][usize::from(*sample)];
            }
            convert::sharpen(&mut rgb, width, 3, request.sharpen);
            (Format::Rgb, rgb)
        }
        Mode::Grey => {
            let mut grey = convert::to_grey(&rgb, &grey_tone);
            convert::sharpen(&mut grey, width, 1, request.sharpen);
            (Format::Grey, grey)
        }
        Mode::Lineart { threshold } => {
            let mut grey = convert::to_grey(&rgb, &grey_tone);
            convert::sharpen(&mut grey, width, 1, request.sharpen);
            (
                Format::Lineart,
                convert::to_lineart(&grey, width, threshold),
            )
        }
    };
    Image {
        width,
        height,
        dpi: request.dpi,
        format,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::HP3770;

    fn request(mode: Mode) -> Request {
        Request {
            linear: true,
            sharpen: 0.0,
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

    #[test]
    fn lineart_expands_for_display() {
        let img = Image {
            width: 3,
            height: 1,
            dpi: 300,
            format: Format::Lineart,
            data: vec![0b1010_0000],
        };
        assert_eq!(img.to_rgb(), [255, 255, 255, 0, 0, 0, 255, 255, 255]);
    }
}
