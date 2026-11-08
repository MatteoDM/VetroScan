// SPDX-License-Identifier: GPL-2.0-or-later
//! From an area of the glass to the window the chip reads.
//!
//! The HP driver takes the area in pixels of the scan resolution, from the
//! top left corner of the glass (the WIA properties `XStart`, `YStart`,
//! `XExtent`, `YExtent`). The window the chip reads is that area moved by
//! the origin of the glass, which depends on the resolution, with the left
//! edge and the width on multiples of 4 pixels.
//!
//! Everything here comes from the captures, where the WIA settings the HP
//! driver accepted are logged next to the windows of its register blocks:
//!
//! | capture | dpi  | WIA x, y, w, h       | chip left, width, top, height |
//! |---------|------|----------------------|-------------------------------|
//! | 01, 02  | 200  | 0, 0, 1654, 2339     | 43, 1656, 69, 2338            |
//! | 04      | 300  | 0, 0, 2480, 3508     | 65, 2480, 102, 3508           |
//! | 09      | 300  | 945, 1459, 591, 591  | 1009, 592, 1561, 591          |
//! | 05      | 600  | 0, 0, 1181, 1181     | 129, 1180, 198, 1180          |
//! | 06      | 1200 | 0, 0, 945, 945       | 259, 944, 373, 944            |
//! | 06b     | 2400 | 0, 0, 1890, 1890     | 259, 944, 746, 1896           |
//!
//! The height the HP driver gives the chip is one line less than asked in
//! some captures (01, 02, 05, 06) and not in others (04, 09); the rule is
//! not known. This module asks the chip for the lines asked.
//!
//! At 2400 dpi the chip reads 1200 pixels per inch across and 2400 lines
//! per inch down, and the host arranges the lines and doubles the width
//! (see [`crate::arrange`]): the window across is in 1200 dpi pixels.

use crate::error::{Error, Result};
use crate::setup::{ArrangeLine, Hardware, ScanParams};

/// Resolutions the HP driver offers for a colour scan.
pub const RESOLUTIONS: [u16; 5] = [200, 300, 600, 1200, 2400];

/// Width and height of the glass in inches (`8.5 × 11.7`, the largest area
/// the HP driver accepts).
pub const GLASS_INCHES: (f64, f64) = (8.5, 11.7);

/// An area of the glass in pixels of the scan resolution, from its top left
/// corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

impl Area {
    /// The area of a rectangle given in millimetres, at `dpi`.
    pub fn from_mm(dpi: u16, left: f64, top: f64, width: f64, height: f64) -> Self {
        let px = |mm: f64| (mm * f64::from(dpi) / 25.4).round().max(0.0) as u32;
        Self {
            x: px(left),
            y: px(top),
            width: px(width),
            height: px(height),
        }
    }
}

/// Origin of the glass in the chip's coordinates: first pixel (at the
/// resolution) and first line (from the home position).
fn origin(dpi: u16) -> Option<(u32, u32)> {
    match dpi {
        200 => Some((43, 69)),
        300 => Some((65, 102)),
        600 => Some((129, 198)),
        1200 => Some((259, 373)),
        2400 => Some((259, 746)),
        _ => None,
    }
}

/// Correction of each resolution's window so that it covers the same
/// part of the glass as the 200 dpi preview, in pixels and lines of the
/// scan (`(across, down)`): the HP driver's origins are not consistent
/// between resolutions. Measured on Matteo's scanner (2026-11-08) by
/// scanning the same 30 × 20 mm at every resolution and matching each
/// image to the 200 dpi one: at 1200 dpi the window started 0.6 mm early
/// and 0.5 mm to the right, at 3, 12.5 and 25 cm from the top alike.
fn alignment(dpi: u16) -> (i32, i32) {
    match dpi {
        300 => (-4, 0),
        600 => (-9, 3),
        1200 => (-23, 31),
        2400 => (-23, 63),
        _ => (0, 0),
    }
}

/// Nearest multiple of 4, halves up (1654 -> 1656, 945 -> 944).
fn round4(v: u32) -> u32 {
    (v + 2) / 4 * 4
}

/// The window the chip reads for `area` at `dpi`, and how the scan uses the
/// chip (as the HP driver: shading and gamma on, lines arranged by the
/// chip, even and odd pixels apart at 1200 dpi), lined up with the 200 dpi
/// preview (see [`alignment`]).
pub fn scan_window(dpi: u16, area: Area) -> Result<(ScanParams, Hardware)> {
    let (mut params, hw) = hp_window(dpi, area)?;
    let (dx, dy) = alignment(dpi);
    params.left = params.left.saturating_add_signed(dx);
    params.top = params.top.saturating_add_signed(dy);
    Ok((params, hw))
}

/// The window the HP driver would read for `area` at `dpi` (the values of
/// its register blocks in the captures).
pub fn hp_window(dpi: u16, area: Area) -> Result<(ScanParams, Hardware)> {
    let (left, top) = origin(dpi).ok_or_else(|| {
        Error::InvalidArgument(format!(
            "{dpi} dpi: the supported resolutions are {RESOLUTIONS:?}"
        ))
    })?;
    let max_w = (GLASS_INCHES.0 * f64::from(dpi)).round() as u32;
    let max_h = (GLASS_INCHES.1 * f64::from(dpi)).round() as u32;
    // Across, the sensor's 1200 dpi at most.
    let across = if dpi > 1200 { 2 } else { 1 };
    let width = round4(area.width / across);
    if width == 0 || area.height == 0 || area.x + area.width > max_w || area.y + area.height > max_h
    {
        return Err(Error::InvalidArgument(format!(
            "area {area:?} is empty or outside the glass ({max_w} x {max_h} at {dpi} dpi)"
        )));
    }
    let params = ScanParams {
        negative: false,
        depth: 8,
        resolution_x: dpi / across as u16,
        resolution_y: dpi,
        left: left + round4(area.x / across),
        top: top + area.y,
        width,
        height: area.height,
    };
    let hw = Hardware {
        black_shading: true,
        white_shading: true,
        gamma: true,
        gamma_size: 0,
        backward: false,
        static_head: false,
        dummy_scan: false,
        compression: false,
        // The chip's line delays are 6 bits: too short at 2400 dpi.
        arrange_line: if dpi > 1200 {
            ArrangeLine::Software
        } else {
            ArrangeLine::Hardware
        },
        high_resolution: dpi >= 1200,
    };
    Ok((params, hw))
}

/// The gamma tables the HP driver sends with its default settings: the
/// identity, once per colour (every capture but the one with brightness
/// and contrast changed). The HP driver applies its gamma to the image on
/// the host.
pub fn identity_gamma() -> Vec<u8> {
    (0..768).map(|i| i as u8).collect()
}

/// Gamma of the tone curve the HP driver applies on the host with its
/// default settings (see [`crate::profile::Profile::tone`]):
/// `255 × (x / white)^(1 / gamma)` of the chip's linear samples, 255 from
/// `white` up.
///
/// Fitted on the same 5 × 5 cm scanned by both drivers (2026-11-07): with
/// `white` = 196 (the white reference 245 × the gain target 80 %) and
/// gamma 2.2 the HP image is matched within 8.7 levels (RMS, all channels;
/// most of the rest is the HP driver's sharpening), against 24 for a plain
/// gamma 2.2. The HP driver's own whites are 196, 186 and 184 for red,
/// green and blue on that sheet (7.8 levels), which tints the paper blue.
pub const HOST_GAMMA: f64 = 2.2;

/// White levels of the HP driver's colour images, red, green and blue (see
/// [`HOST_GAMMA`]): fitted on the same sheet, they leave the paper slightly
/// blue, as the HP driver does.
pub const HP_WHITE: [f64; 3] = [196.0, 186.0, 184.0];

/// White level of the host tone curve of grey images for a profile: where
/// the gain calibration puts the white strip.
pub fn host_white(profile: &crate::profile::Profile) -> f64 {
    let c = &profile.calibration;
    f64::from(c.white_ref[0]) * f64::from(c.gain_target_percent) / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(dpi: u16, x: u32, y: u32, w: u32, h: u32) -> (u32, u32, u32) {
        let area = Area {
            x,
            y,
            width: w,
            height: h,
        };
        let (p, hw) = hp_window(dpi, area).unwrap();
        assert_eq!(hw.high_resolution, dpi >= 1200);
        (p.left, p.width, p.top)
    }

    #[test]
    fn windows_of_the_captures() {
        assert_eq!(window(200, 0, 0, 1654, 2339), (43, 1656, 69));
        assert_eq!(window(300, 0, 0, 2480, 3508), (65, 2480, 102));
        assert_eq!(window(300, 945, 1459, 591, 591), (1009, 592, 1561));
        assert_eq!(window(600, 0, 0, 1181, 1181), (129, 1180, 198));
        assert_eq!(window(1200, 0, 0, 945, 945), (259, 944, 373));
        assert_eq!(window(2400, 0, 0, 1890, 1890), (259, 944, 746));
        // Heights the HP driver kept.
        let a = Area {
            x: 945,
            y: 1459,
            width: 591,
            height: 591,
        };
        assert_eq!(hp_window(300, a).unwrap().0.height, 591);
    }

    #[test]
    fn windows_are_lined_up_with_the_preview() {
        let a = Area {
            x: 400,
            y: 400,
            width: 100,
            height: 100,
        };
        let hp = hp_window(1200, a).unwrap().0;
        let ours = scan_window(1200, a).unwrap().0;
        assert_eq!((ours.left, ours.top), (hp.left - 23, hp.top + 31));
        assert_eq!(scan_window(200, a).unwrap().0, hp_window(200, a).unwrap().0);
    }

    #[test]
    fn millimetres_as_the_capture_client() {
        // 09: 80, 123.5, 50 x 50 mm at 300 dpi.
        let a = Area::from_mm(300, 80.0, 123.5, 50.0, 50.0);
        assert_eq!((a.x, a.y, a.width, a.height), (945, 1459, 591, 591));
    }

    #[test]
    fn rejects_what_the_hp_driver_does_not_offer() {
        let a = Area {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        assert!(scan_window(4800, a).is_err());
        assert!(scan_window(150, a).is_err());
        let too_wide = Area { width: 1701, ..a };
        assert!(scan_window(200, too_wide).is_err());
    }

    #[test]
    fn gamma_tables() {
        let g = identity_gamma();
        assert_eq!(g.len(), 768);
        assert_eq!(&g[254..258], &[254, 255, 0, 1]);
        let white = host_white(&crate::profile::HP3770);
        assert_eq!(white, 196.0);
        let h = crate::profile::HP3770.tone().table(white);
        assert_eq!((h[0], h[196], h[255]), (0, 255, 255));
        // Half the white: 255 x 0.5^(1/2.2).
        assert_eq!(h[98], 186);
    }
}
