// SPDX-License-Identifier: GPL-2.0-or-later
//! Image adjustments, applied on the host to the chip's linear samples.
//!
//! The order is the usual one of scanning software: white balance on the
//! linear light (temperature and tint), the HP driver's tone curve, levels
//! (black point, white point, gamma), brightness and contrast, saturation,
//! sharpening; for grey and black and white, the grey level is taken after
//! the white balance and the threshold last. Everything up to contrast is
//! one lookup table per channel, so the adjustments can be redrawn on the
//! preview as a slider moves.
//!
//! The chip's samples are never changed: the same adjustments give the
//! same result on the preview and on the scan at full resolution.

use crate::convert;
use crate::geometry;
use crate::profile::Profile;
use crate::session::{Format, Image, Mode};

/// The adjustments of an image. [`Adjustments::default`] gives the HP
/// driver's result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjustments {
    /// Colour temperature, −100 (cooler, bluer) to 100 (warmer, redder).
    pub temperature: f32,
    /// Tint, −100 (greener) to 100 (more magenta).
    pub tint: f32,
    /// Exposure in stops, −3 to 3, on the linear light.
    pub exposure: f32,
    /// Highlights, −100 (recover) to 100: the upper tones only.
    pub highlights: f32,
    /// Shadows, −100 to 100 (open up): the lower tones only.
    pub shadows: f32,
    /// Whites, −100 to 100: the brightest tones and the white point.
    pub whites: f32,
    /// Blacks, −100 (deeper) to 100: the darkest tones and the black point.
    pub blacks: f32,
    /// Vibrance, −100 to 100: saturation that spares colours already vivid.
    pub vibrance: f32,
    /// Black point of the output, 0-254: levels at or below become black.
    pub black: f32,
    /// White point of the output, 1-255: levels at or above become white.
    pub white: f32,
    /// Midtone gamma, 0.2-5 (1: none; above 1 lightens).
    pub gamma: f32,
    /// Brightness, −100 to 100.
    pub brightness: f32,
    /// Contrast, −100 to 100.
    pub contrast: f32,
    /// Saturation, −100 (grey) to 100.
    pub saturation: f32,
    /// Sharpening amount ([`convert::HP_SHARPEN`] as the HP driver, 0 for
    /// none).
    pub sharpen: f32,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            temperature: 0.0,
            tint: 0.0,
            exposure: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            vibrance: 0.0,
            black: 0.0,
            white: 255.0,
            gamma: 1.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            sharpen: convert::HP_SHARPEN,
        }
    }
}

/// How strongly temperature and tint move the channels at ±100.
const BALANCE_RANGE: f32 = 0.35;

impl Adjustments {
    /// Gains of red, green and blue for the white balance.
    pub fn gains(&self) -> [f32; 3] {
        let t = self.temperature.clamp(-100.0, 100.0) / 100.0 * BALANCE_RANGE;
        let m = self.tint.clamp(-100.0, 100.0) / 100.0 * BALANCE_RANGE;
        [1.0 + t, 1.0 - m, 1.0 - t]
    }

    /// Temperature and tint that make a linear sample `rgb` neutral grey
    /// (the eyedropper on something that should be white or grey). The
    /// rest of the adjustments are kept.
    pub fn neutral_at(&self, rgb: [u8; 3]) -> Self {
        let [r, g, b] = rgb.map(|v| f32::from(v).max(1.0));
        // Red and blue to the same level: r(1 + a) = b(1 − a).
        let a = ((b - r) / (b + r)).clamp(-BALANCE_RANGE, BALANCE_RANGE);
        let level = r * (1.0 + a);
        // Green to that level: g(1 − m) = level.
        let m = (1.0 - level / g).clamp(-BALANCE_RANGE, BALANCE_RANGE);
        Self {
            temperature: a / BALANCE_RANGE * 100.0,
            tint: m / BALANCE_RANGE * 100.0,
            ..*self
        }
    }

    /// The per-channel tables: linear chip sample to output level, through
    /// white balance, `tone` (the host tone curve of the channel), levels,
    /// brightness and contrast. `linear` leaves out the tone curve.
    pub fn tables(&self, white: [f64; 3], linear: bool) -> [[u8; 256]; 3] {
        let gains = self.gains();
        let black = self.black.clamp(0.0, 254.0);
        let white_point = self.white.clamp(black + 1.0, 255.0);
        let gamma = self.gamma.clamp(0.2, 5.0);
        let contrast = (100.0 + self.contrast.clamp(-100.0, 100.0)) / 100.0;
        let brightness = self.brightness.clamp(-100.0, 100.0) * 1.28;
        let exposure = 2f32.powf(self.exposure.clamp(-3.0, 3.0));
        let highlights = self.highlights.clamp(-100.0, 100.0) / 100.0;
        let shadows = self.shadows.clamp(-100.0, 100.0) / 100.0;
        let whites = self.whites.clamp(-100.0, 100.0) / 100.0;
        let blacks = self.blacks.clamp(-100.0, 100.0) / 100.0;
        std::array::from_fn(|c| {
            std::array::from_fn(|i| {
                let x = (i as f32 * gains[c] * exposure).min(255.0);
                let v = if linear {
                    x
                } else {
                    255.0 * (f64::from(x) / white[c]).min(1.0).powf(1.0 / 2.2) as f32
                };
                let v = ((v - black) / (white_point - black)).clamp(0.0, 1.0);
                let v = tone_regions(v, highlights, shadows, whites, blacks);
                let v = 255.0 * v.powf(1.0 / gamma);
                let v = (v - 128.0) * contrast + 128.0 + brightness;
                v.round().clamp(0.0, 255.0) as u8
            })
        })
    }
}

/// Highlights, shadows, whites and blacks on a level `t` (0-1): each moves
/// its own band of tones with a smooth weight, so the ends stay put
/// unless whites or blacks move them (amounts −1 to 1).
fn tone_regions(t: f32, highlights: f32, shadows: f32, whites: f32, blacks: f32) -> f32 {
    // Weights peaking at 1/3 (shadows) and 2/3 (highlights).
    let low = 6.75 * t * (1.0 - t) * (1.0 - t);
    let high = 6.75 * t * t * (1.0 - t);
    let mut v = t + 0.25 * shadows * low + 0.25 * highlights * high;
    // Whites and blacks act on the ends and move the end points.
    v += 0.2 * whites * t.powi(3);
    v += 0.2 * blacks * (1.0 - t).powi(3);
    v.clamp(0.0, 1.0)
}

/// Vibrance of 8-bit RGB pixels, −100 to 100: like saturation, weighted by
/// how grey a pixel is, so that vivid colours (and skin) are not overdone.
pub fn vibrance(rgb: &mut [u8], amount: f32) {
    if amount == 0.0 {
        return;
    }
    let k = amount.clamp(-100.0, 100.0) / 100.0;
    for p in rgb.chunks_exact_mut(3) {
        let (r, g, b) = (f32::from(p[0]), f32::from(p[1]), f32::from(p[2]));
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
        let factor = 1.0 + k * (1.0 - sat);
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        for v in p.iter_mut() {
            *v = (luma + (f32::from(*v) - luma) * factor)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
}

/// Levels that stretch the image to the full range: black and white
/// points where 0.1 % of the pixels are darker and brighter (on the mean
/// of the channels).
pub fn auto_levels(image: &Image) -> (f32, f32) {
    let mut histogram = [0u64; 256];
    match image.format {
        Format::Rgb => {
            for p in image.data.chunks_exact(3) {
                let mean = (u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2])) / 3;
                histogram[usize::from(mean)] += 1;
            }
        }
        Format::Grey => image
            .data
            .iter()
            .for_each(|&g| histogram[usize::from(g)] += 1),
        Format::Lineart => return (0.0, 255.0),
    }
    let total: u64 = histogram.iter().sum();
    let cut = total / 1000;
    let mut sum = 0;
    let black = histogram
        .iter()
        .position(|&n| {
            sum += n;
            sum > cut
        })
        .unwrap_or(0);
    sum = 0;
    let white = 255
        - histogram
            .iter()
            .rev()
            .position(|&n| {
                sum += n;
                sum > cut
            })
            .unwrap_or(0);
    if white <= black {
        return (0.0, 255.0);
    }
    (black as f32, white as f32)
}

/// Threshold that best splits grey levels into two groups (Otsu's
/// method), as the HP driver's `Threshold` (white from `255 − threshold`).
pub fn auto_threshold(grey: &[u8]) -> u8 {
    let mut histogram = [0u64; 256];
    grey.iter().for_each(|&g| histogram[usize::from(g)] += 1);
    let total = grey.len() as f64;
    let sum_all: f64 = histogram
        .iter()
        .enumerate()
        .map(|(i, &n)| i as f64 * n as f64)
        .sum();
    let (mut best, mut cut) = (0.0, 128usize);
    let (mut weight, mut sum) = (0.0, 0.0);
    for (t, &n) in histogram.iter().enumerate() {
        weight += n as f64;
        if weight == 0.0 || weight == total {
            continue;
        }
        sum += t as f64 * n as f64;
        let m0 = sum / weight;
        let m1 = (sum_all - sum) / (total - weight);
        let between = weight * (total - weight) * (m0 - m1).powi(2);
        if between > best {
            best = between;
            cut = t + 1;
        }
    }
    255 - cut.min(255) as u8
}

/// Saturation of 8-bit RGB pixels, −100 (grey) to 100 (twice as vivid).
pub fn saturate(rgb: &mut [u8], amount: f32) {
    if amount == 0.0 {
        return;
    }
    let k = 1.0 + amount.clamp(-100.0, 100.0) / 100.0;
    for p in rgb.chunks_exact_mut(3) {
        let luma = 0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]);
        for v in p.iter_mut() {
            *v = (luma + (f32::from(*v) - luma) * k)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
}

/// The finished image from the chip's linear 8-bit RGB samples
/// (`width` × `height`), with `adjustments` in `mode`. `linear` keeps the
/// chip's light levels (no tone curve: for further processing).
#[allow(clippy::too_many_arguments)]
pub fn render(
    rgb: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    mode: Mode,
    adjustments: &Adjustments,
    linear: bool,
    profile: &Profile,
) -> Image {
    let (format, data) = match mode {
        Mode::Colour => {
            let tables = adjustments.tables(geometry::HP_WHITE, linear);
            let mut out: Vec<u8> = rgb
                .iter()
                .enumerate()
                .map(|(i, &v)| tables[i % 3][usize::from(v)])
                .collect();
            saturate(&mut out, adjustments.saturation);
            vibrance(&mut out, adjustments.vibrance);
            convert::sharpen(&mut out, width, 3, adjustments.sharpen);
            (Format::Rgb, out)
        }
        Mode::Grey | Mode::Lineart { .. } => {
            // White balance on the linear light, then one grey level
            // through the tone tables (without the balance again).
            let gains = adjustments.gains();
            let white = geometry::host_white(profile);
            let tables = Adjustments {
                temperature: 0.0,
                tint: 0.0,
                ..*adjustments
            }
            .tables([white; 3], linear);
            let mut grey: Vec<u8> = rgb
                .chunks_exact(3)
                .map(|p| {
                    let sum: f32 = (0..3).map(|c| f32::from(p[c]) * gains[c]).sum();
                    let level = (sum / 3.0).round().clamp(0.0, 255.0) as usize;
                    tables[1][level]
                })
                .collect();
            convert::sharpen(&mut grey, width, 1, adjustments.sharpen);
            match mode {
                Mode::Lineart { threshold } => (
                    Format::Lineart,
                    convert::to_lineart(&grey, width, threshold),
                ),
                _ => (Format::Grey, grey),
            }
        }
    };
    Image {
        width,
        height,
        dpi,
        format,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_hp_result() {
        // The defaults must give what session::process gave before.
        let t = Adjustments::default().tables(geometry::HP_WHITE, false);
        for (table, white) in t.iter().zip(geometry::HP_WHITE) {
            assert_eq!(*table, geometry::host_tone(white, 2.2), "white {white}");
        }
        let linear = Adjustments::default().tables(geometry::HP_WHITE, true);
        assert!(linear[0]
            .iter()
            .enumerate()
            .all(|(i, &v)| usize::from(v) == i));
    }

    #[test]
    fn eyedropper_makes_the_sample_grey() {
        let a = Adjustments::default().neutral_at([120, 100, 80]);
        let g = a.gains();
        let out = [120.0 * g[0], 100.0 * g[1], 80.0 * g[2]];
        assert!(
            (out[0] - out[2]).abs() < 0.5 && (out[0] - out[1]).abs() < 0.5,
            "{out:?}"
        );
        assert!(a.temperature < 0.0, "a warm sample is cooled");
    }

    #[test]
    fn levels_stretch_and_clip() {
        let a = Adjustments {
            black: 50.0,
            white: 200.0,
            ..Adjustments::default()
        };
        let t = a.tables([255.0; 3], true);
        assert_eq!(
            (t[0][50], t[0][125], t[0][200], t[0][230]),
            (0, 128, 255, 255)
        );
    }

    #[test]
    fn auto_tools() {
        // Two groups of grey: Otsu splits between them.
        let mut grey = vec![40u8; 500];
        grey.extend(vec![210u8; 500]);
        let t = auto_threshold(&grey);
        let cut = 255 - t;
        assert!(cut > 40 && cut <= 210, "cut {cut}");
        let img = Image {
            width: 1000,
            height: 1,
            dpi: 300,
            format: Format::Grey,
            data: grey,
        };
        assert_eq!(auto_levels(&img), (40.0, 210.0));
    }

    #[test]
    fn tone_regions_keep_the_ends_and_move_their_band() {
        assert_eq!(tone_regions(0.0, 1.0, 1.0, 0.0, 0.0), 0.0);
        assert_eq!(tone_regions(1.0, 1.0, 1.0, 0.0, 0.0), 1.0);
        // Shadows lift a dark tone more than a light one.
        let dark = tone_regions(0.3, 0.0, 1.0, 0.0, 0.0) - 0.3;
        let light = tone_regions(0.8, 0.0, 1.0, 0.0, 0.0) - 0.8;
        assert!(dark > 0.2 && light < dark / 2.0, "{dark} {light}");
        // Negative highlights darken the light tones.
        assert!(tone_regions(0.7, -1.0, 0.0, 0.0, 0.0) < 0.7);
        // Exposure doubles the linear light.
        let a = Adjustments {
            exposure: 1.0,
            ..Adjustments::default()
        };
        assert_eq!(a.tables([255.0; 3], true)[0][60], 120);
    }

    #[test]
    fn vibrance_spares_vivid_colours() {
        let mut dull = [140, 120, 120];
        let mut vivid = [250, 40, 40];
        vibrance(&mut dull, 100.0);
        vibrance(&mut vivid, 100.0);
        assert!(dull[0] - dull[1] > 20 + 10);
        assert!(i32::from(vivid[0]) - i32::from(vivid[1]) < 210 + 60);
    }

    #[test]
    fn saturation_moves_away_from_grey() {
        let mut p = [200, 100, 100];
        saturate(&mut p, 100.0);
        assert!(p[0] > 200 && p[1] < 100);
        let mut q = [200, 100, 100];
        saturate(&mut q, -100.0);
        assert!(q[0] == q[1] && q[1] == q[2]);
    }
}
