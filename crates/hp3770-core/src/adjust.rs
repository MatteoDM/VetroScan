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
    /// Tone curves for all channels together, then red, green and blue.
    pub curve: [Curve; 4],
    /// Noise reduction, 0-100: smooths small differences, keeps edges.
    pub noise: f32,
    /// Screen of printed matter to remove (descreen), in lines per inch;
    /// 0 for none (magazines about 150, newspapers about 85).
    pub descreen: f32,
    /// Black and white against the local brightness instead of one level
    /// for the whole page (pages unevenly lit, shadows of the binding).
    pub adaptive: bool,
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
            curve: [Curve::IDENTITY; 4],
            noise: 0.0,
            descreen: 0.0,
            adaptive: false,
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

/// A tone curve: 2 to [`Curve::MAX`] points (input, output), both 0-1,
/// in order of input, the first and last being the black and white
/// points. Between the points it is a monotone cubic (Fritsch-Carlson):
/// it never overshoots, so it never inverts tones (the solarization of
/// ordinary splines); outside them it is flat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    len: u8,
    points: [[f32; 2]; Curve::MAX],
}

impl Curve {
    /// Most points of a curve, as Photoshop (14 plus the two ends).
    pub const MAX: usize = 16;

    /// The curve that changes nothing.
    pub const IDENTITY: Curve = Curve::fixed([[0.0, 0.0], [1.0, 1.0]]);

    /// A gentle S: shadows a little darker, highlights a little lighter.
    pub const MEDIUM_CONTRAST: Curve =
        Curve::fixed4([[0.0, 0.0], [0.25, 0.215], [0.75, 0.785], [1.0, 1.0]]);

    /// A stronger S.
    pub const STRONG_CONTRAST: Curve =
        Curve::fixed4([[0.0, 0.0], [0.25, 0.16], [0.75, 0.84], [1.0, 1.0]]);

    const fn fixed(p: [[f32; 2]; 2]) -> Curve {
        let mut points = [[0.0; 2]; Curve::MAX];
        points[0] = p[0];
        points[1] = p[1];
        Curve { len: 2, points }
    }

    const fn fixed4(p: [[f32; 2]; 4]) -> Curve {
        let mut points = [[0.0; 2]; Curve::MAX];
        points[0] = p[0];
        points[1] = p[1];
        points[2] = p[2];
        points[3] = p[3];
        Curve { len: 4, points }
    }

    /// A curve through `points` (sorted, kept within 0-1, two points never
    /// at the same input, at most [`Curve::MAX`]). Fewer than two points
    /// give the identity.
    pub fn new(points: &[[f32; 2]]) -> Curve {
        let mut v: Vec<[f32; 2]> = points
            .iter()
            .map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)])
            .collect();
        v.sort_by(|a, b| a[0].total_cmp(&b[0]));
        v.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-3);
        if v.len() < 2 {
            return Curve::IDENTITY;
        }
        v.truncate(Curve::MAX);
        let mut out = [[0.0; 2]; Curve::MAX];
        out[..v.len()].copy_from_slice(&v);
        Curve {
            len: v.len() as u8,
            points: out,
        }
    }

    /// The points.
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points[..usize::from(self.len)]
    }

    pub fn is_identity(&self) -> bool {
        self.points() == Curve::IDENTITY.points()
    }

    /// The output at input `t`.
    pub fn eval(&self, t: f32) -> f32 {
        if self.is_identity() {
            return t;
        }
        let p = self.points();
        let n = p.len();
        let t = t.clamp(0.0, 1.0);
        if t <= p[0][0] {
            return p[0][1];
        }
        if t >= p[n - 1][0] {
            return p[n - 1][1];
        }
        // Secant slopes, then tangents limited so that each piece stays
        // monotone (Fritsch-Carlson).
        let d: Vec<f32> = (0..n - 1)
            .map(|i| (p[i + 1][1] - p[i][1]) / (p[i + 1][0] - p[i][0]))
            .collect();
        let mut m = vec![0.0f32; n];
        m[0] = d[0];
        m[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            m[i] = if d[i - 1] * d[i] <= 0.0 {
                0.0
            } else {
                (d[i - 1] + d[i]) / 2.0
            };
        }
        for i in 0..n - 1 {
            if d[i] == 0.0 {
                m[i] = 0.0;
                m[i + 1] = 0.0;
            } else {
                let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
                let r = a * a + b * b;
                if r > 9.0 {
                    let k = 3.0 / r.sqrt();
                    m[i] = k * a * d[i];
                    m[i + 1] = k * b * d[i];
                }
            }
        }
        let i = (0..n - 1).find(|&i| t < p[i + 1][0]).unwrap_or(n - 2);
        let h = p[i + 1][0] - p[i][0];
        let u = (t - p[i][0]) / h;
        let (u2, u3) = (u * u, u * u * u);
        let v = (2.0 * u3 - 3.0 * u2 + 1.0) * p[i][1]
            + (u3 - 2.0 * u2 + u) * h * m[i]
            + (-2.0 * u3 + 3.0 * u2) * p[i + 1][1]
            + (u3 - u2) * h * m[i + 1];
        v.clamp(0.0, 1.0)
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
        let level = self.levels(white, linear);
        std::array::from_fn(|c| std::array::from_fn(|i| level(c, i as f32)))
    }

    /// The same tables for 12-bit samples (4096 entries, a chip sample of
    /// 16 bits shifted down by 4): the shadows, which the tone curve
    /// stretches most, keep their gradation instead of the steps of 8-bit
    /// samples.
    pub fn tables_fine(&self, white: [f64; 3], linear: bool) -> [Vec<u8>; 3] {
        let level = self.levels(white, linear);
        std::array::from_fn(|c| (0..4096).map(|i| level(c, i as f32 / 16.0)).collect())
    }

    /// The output level of channel `c` for a linear sample `i` in 8-bit
    /// units (fractions allowed).
    fn levels(&self, white: [f64; 3], linear: bool) -> impl Fn(usize, f32) -> u8 + '_ {
        let level = self.levels_exact(white, linear);
        move |c, i| (level(c, i) * 255.0).round().clamp(0.0, 255.0) as u8
    }

    /// The tables for 12-bit samples with 16-bit output levels.
    pub fn tables_fine16(&self, white: [f64; 3], linear: bool) -> [Vec<u16>; 3] {
        let level = self.levels_exact(white, linear);
        std::array::from_fn(|c| {
            (0..4096)
                .map(|i| {
                    (level(c, i as f32 / 16.0) * 65535.0)
                        .round()
                        .clamp(0.0, 65535.0) as u16
                })
                .collect()
        })
    }

    /// The output (0-1) of channel `c` for a linear sample `i` in 8-bit
    /// units.
    fn levels_exact(&self, white: [f64; 3], linear: bool) -> impl Fn(usize, f32) -> f32 + '_ {
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
        move |c: usize, i: f32| {
            let x = (i * gains[c] * exposure).min(255.0);
            let v = if linear {
                x
            } else {
                255.0 * (f64::from(x) / white[c]).min(1.0).powf(1.0 / 2.2) as f32
            };
            let v = ((v - black) / (white_point - black)).clamp(0.0, 1.0);
            let v = tone_regions(v, highlights, shadows, whites, blacks);
            let v = 255.0 * v.powf(1.0 / gamma);
            let v = (v - 128.0) * contrast + 128.0 + brightness;
            let v = (v / 255.0).clamp(0.0, 1.0);
            self.curve[0].eval(self.curve[c + 1].eval(v))
        }
    }
}

/// Box blur of radius `r` across and down, `channels` samples per pixel.
pub fn box_blur(data: &mut [u8], width: usize, channels: usize, r: usize) {
    if r == 0 || width == 0 {
        return;
    }
    let height = data.len() / (width * channels);
    let mut tmp = vec![0u8; data.len()];
    // Across.
    for y in 0..height {
        for c in 0..channels {
            let at = |x: usize| (y * width + x) * channels + c;
            let mut sum: u32 = 0;
            let mut n: u32 = 0;
            for x in 0..=r.min(width - 1) {
                sum += u32::from(data[at(x)]);
                n += 1;
            }
            for x in 0..width {
                tmp[at(x)] = (sum / n) as u8;
                if x + r + 1 < width {
                    sum += u32::from(data[at(x + r + 1)]);
                    n += 1;
                }
                if x >= r {
                    sum -= u32::from(data[at(x - r)]);
                    n -= 1;
                }
            }
        }
    }
    // Down.
    for x in 0..width {
        for c in 0..channels {
            let at = |y: usize| (y * width + x) * channels + c;
            let mut sum: u32 = 0;
            let mut n: u32 = 0;
            for y in 0..=r.min(height.saturating_sub(1)) {
                sum += u32::from(tmp[at(y)]);
                n += 1;
            }
            for y in 0..height {
                data[at(y)] = (sum / n) as u8;
                if y + r + 1 < height {
                    sum += u32::from(tmp[at(y + r + 1)]);
                    n += 1;
                }
                if y >= r {
                    sum -= u32::from(tmp[at(y - r)]);
                    n -= 1;
                }
            }
        }
    }
}

/// Removes the screen of printed matter at `lpi` lines per inch from an
/// image at `dpi`: a blur about as wide as one screen cell (twice, close
/// to a Gaussian).
pub fn descreen(data: &mut [u8], width: usize, channels: usize, dpi: u16, lpi: f32) {
    if lpi <= 0.0 {
        return;
    }
    let r = ((f32::from(dpi) / lpi) * 0.5).round().max(1.0) as usize;
    box_blur(data, width, channels, r);
    box_blur(data, width, channels, r);
}

/// Noise reduction, 0-100: each sample moves towards the mean of its
/// neighbourhood, less where it differs a lot from it (an edge).
pub fn denoise(data: &mut [u8], width: usize, channels: usize, amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let mut smooth = data.to_vec();
    box_blur(&mut smooth, width, channels, 1);
    box_blur(&mut smooth, width, channels, 1);
    let k = amount.clamp(0.0, 100.0) / 100.0;
    for (v, &m) in data.iter_mut().zip(&smooth) {
        let diff = f32::from(m) - f32::from(*v);
        let weight = k * (1.0 - (diff.abs() / 28.0).min(1.0));
        *v = (f32::from(*v) + weight * diff).round().clamp(0.0, 255.0) as u8;
    }
}

/// Black and white against the local brightness: a pixel is white when it
/// is not much darker than the mean of the square around it (about an
/// eighth of an inch), and not darker than a floor. `threshold` (as the
/// HP driver's) makes the page darker or lighter. Packed as
/// [`convert::to_lineart`].
pub fn to_lineart_adaptive(grey: &[u8], width: usize, dpi: u16, threshold: u8) -> Vec<u8> {
    let height = grey.len() / width.max(1);
    let r = (usize::from(dpi) / 16).max(7);
    // Integral image.
    let mut sum = vec![0u64; (width + 1) * (height + 1)];
    for y in 0..height {
        let mut row = 0u64;
        for x in 0..width {
            row += u64::from(grey[y * width + x]);
            sum[(y + 1) * (width + 1) + x + 1] = sum[y * (width + 1) + x + 1] + row;
        }
    }
    let offset = 12.0 + (55.0 - f32::from(threshold)) * 0.25;
    let stride = width.div_ceil(8);
    let mut out = vec![0u8; stride * height];
    for y in 0..height {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(height));
        for x in 0..width {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(width));
            let total = sum[y1 * (width + 1) + x1] + sum[y0 * (width + 1) + x0]
                - sum[y0 * (width + 1) + x1]
                - sum[y1 * (width + 1) + x0];
            let mean = total as f32 / ((y1 - y0) * (x1 - x0)) as f32;
            let g = f32::from(grey[y * width + x]);
            if g >= mean - offset && g > 60.0 {
                out[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    out
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
    render_samples(
        Samples::Eight(rgb),
        width,
        height,
        dpi,
        mode,
        adjustments,
        linear,
        profile,
    )
}

/// The chip's linear RGB samples, 8 or 16 bits.
#[derive(Debug, Clone, Copy)]
pub enum Samples<'a> {
    Eight(&'a [u8]),
    Sixteen(&'a [u16]),
}

/// [`render`] of 8- or 16-bit samples. With 16 bits the tone tables take
/// 12 bits of each sample, so dark tones lifted by the curves stay smooth.
#[allow(clippy::too_many_arguments)]
pub fn render_samples(
    samples: Samples<'_>,
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
            let mut out: Vec<u8> = match samples {
                Samples::Eight(rgb) => {
                    let tables = adjustments.tables(geometry::HP_WHITE, linear);
                    rgb.iter()
                        .enumerate()
                        .map(|(i, &v)| tables[i % 3][usize::from(v)])
                        .collect()
                }
                Samples::Sixteen(rgb) => {
                    let tables = adjustments.tables_fine(geometry::HP_WHITE, linear);
                    rgb.iter()
                        .enumerate()
                        .map(|(i, &v)| tables[i % 3][usize::from(v >> 4)])
                        .collect()
                }
            };
            saturate(&mut out, adjustments.saturation);
            vibrance(&mut out, adjustments.vibrance);
            descreen(&mut out, width, 3, dpi, adjustments.descreen);
            denoise(&mut out, width, 3, adjustments.noise);
            convert::sharpen(&mut out, width, 3, adjustments.sharpen);
            (Format::Rgb, out)
        }
        Mode::Grey | Mode::Lineart { .. } => {
            // White balance on the linear light, then one grey level
            // through the tone tables (without the balance again).
            let gains = adjustments.gains();
            let white = geometry::host_white(profile);
            // One grey level: the overall curve only.
            let tables = Adjustments {
                temperature: 0.0,
                tint: 0.0,
                curve: [
                    adjustments.curve[0],
                    Curve::IDENTITY,
                    Curve::IDENTITY,
                    Curve::IDENTITY,
                ],
                ..*adjustments
            }
            .tables_fine([white; 3], linear);
            let grey_of = |p: [f32; 3]| {
                let sum: f32 = (0..3).map(|c| p[c] * gains[c]).sum();
                let level = (sum / 3.0 * 16.0).round().clamp(0.0, 4095.0) as usize;
                tables[1][level]
            };
            let mut grey: Vec<u8> = match samples {
                Samples::Eight(rgb) => rgb
                    .chunks_exact(3)
                    .map(|p| grey_of([p[0], p[1], p[2]].map(f32::from)))
                    .collect(),
                Samples::Sixteen(rgb) => rgb
                    .chunks_exact(3)
                    .map(|p| grey_of([p[0], p[1], p[2]].map(|v| f32::from(v) / 256.0)))
                    .collect(),
            };
            descreen(&mut grey, width, 1, dpi, adjustments.descreen);
            denoise(&mut grey, width, 1, adjustments.noise);
            convert::sharpen(&mut grey, width, 1, adjustments.sharpen);
            match mode {
                Mode::Lineart { threshold } if adjustments.adaptive => (
                    Format::Lineart,
                    to_lineart_adaptive(&grey, width, dpi, threshold),
                ),
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

/// [`render_samples`] of 16-bit samples with 16-bit output (RGB, or grey
/// for the grey mode; `None` for black and white). The tones come from
/// tables with 16-bit levels; what follows them (colour, noise, descreen,
/// sharpening) is worked out at 8 bits and added as a difference, so flat
/// areas and gradients keep all their levels.
#[allow(clippy::too_many_arguments)]
pub fn render16(
    samples: &[u16],
    width: usize,
    height: usize,
    dpi: u16,
    mode: Mode,
    adjustments: &Adjustments,
    linear: bool,
    profile: &Profile,
) -> Option<Vec<u16>> {
    let finished = render_samples(
        Samples::Sixteen(samples),
        width,
        height,
        dpi,
        mode,
        adjustments,
        linear,
        profile,
    );
    let tone16: Vec<u16> = match mode {
        Mode::Colour => {
            let t = adjustments.tables_fine16(geometry::HP_WHITE, linear);
            samples
                .iter()
                .enumerate()
                .map(|(i, &v)| t[i % 3][usize::from(v >> 4)])
                .collect()
        }
        Mode::Grey => {
            let gains = adjustments.gains();
            let white = geometry::host_white(profile);
            let t = Adjustments {
                temperature: 0.0,
                tint: 0.0,
                curve: [
                    adjustments.curve[0],
                    Curve::IDENTITY,
                    Curve::IDENTITY,
                    Curve::IDENTITY,
                ],
                ..*adjustments
            }
            .tables_fine16([white; 3], linear);
            samples
                .chunks_exact(3)
                .map(|p| {
                    let sum: f32 = (0..3).map(|c| f32::from(p[c]) / 256.0 * gains[c]).sum();
                    let level = (sum / 3.0 * 16.0).round().clamp(0.0, 4095.0) as usize;
                    t[1][level]
                })
                .collect()
        }
        Mode::Lineart { .. } => return None,
    };
    Some(
        tone16
            .iter()
            .zip(&finished.data)
            .map(|(&t, &f)| {
                // The 8-bit tone this sample had before the later steps.
                let t8 = (f32::from(t) / 257.0).round();
                let delta = (f32::from(f) - t8) * 257.0;
                (f32::from(t) + delta).round().clamp(0.0, 65535.0) as u16
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render16_follows_render_and_is_smooth() {
        use crate::profile::HP3770;
        // A slow ramp in the shadows.
        let n = 2000;
        let rgb16: Vec<u16> = (0..n).flat_map(|i| [(i * 3) as u16; 3]).collect();
        let a = Adjustments {
            shadows: 60.0,
            ..Adjustments::default()
        };
        let out = render16(&rgb16, n, 1, 300, Mode::Colour, &a, false, &HP3770).unwrap();
        let eight = render_samples(
            Samples::Sixteen(&rgb16),
            n,
            1,
            300,
            Mode::Colour,
            &a,
            false,
            &HP3770,
        );
        for (x, y) in out.iter().zip(&eight.data) {
            assert!((f32::from(*x) / 257.0 - f32::from(*y)).abs() <= 1.0);
        }
        let levels16: std::collections::BTreeSet<u16> = out.iter().copied().collect();
        let levels8: std::collections::BTreeSet<u8> = eight.data.iter().copied().collect();
        assert!(
            levels16.len() > 4 * levels8.len(),
            "{} vs {}",
            levels16.len(),
            levels8.len()
        );
    }

    #[test]
    fn sixteen_bits_match_eight_and_keep_the_shadows() {
        use crate::profile::HP3770;
        let rgb8: Vec<u8> = (0..=255u8).flat_map(|v| [v, v, v]).collect();
        let rgb16: Vec<u16> = rgb8.iter().map(|&v| u16::from(v) << 8).collect();
        let a = Adjustments::default();
        for mode in [Mode::Colour, Mode::Grey] {
            let eight = render(&rgb8, 256, 1, 300, mode, &a, false, &HP3770);
            let sixteen = render_samples(
                Samples::Sixteen(&rgb16),
                256,
                1,
                300,
                mode,
                &a,
                false,
                &HP3770,
            );
            for (x, y) in eight.data.iter().zip(&sixteen.data) {
                assert!(x.abs_diff(*y) <= 1, "{mode:?}: {x} {y}");
            }
        }
        // The darkest tenth of 8-bit samples (0-20) gives few output
        // levels; 16-bit samples there give many more.
        let dark16: Vec<u16> = (0..20 * 256)
            .step_by(4)
            .flat_map(|v| [v as u16; 3])
            .collect();
        let n = dark16.len() / 3;
        let out = render_samples(
            Samples::Sixteen(&dark16),
            n,
            1,
            300,
            Mode::Colour,
            &a,
            false,
            &HP3770,
        );
        let levels: std::collections::BTreeSet<u8> = out.data.iter().copied().collect();
        let dark8: Vec<u8> = (0..20u8).flat_map(|v| [v; 3]).collect();
        let out8 = render(&dark8, 20, 1, 300, Mode::Colour, &a, false, &HP3770);
        let levels8: std::collections::BTreeSet<u8> = out8.data.iter().copied().collect();
        assert!(
            levels.len() > 3 * levels8.len() / 2,
            "{} vs {}",
            levels.len(),
            levels8.len()
        );
    }

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
    fn curve_is_identity_by_default_and_monotone() {
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            assert_eq!(Curve::IDENTITY.eval(t), t);
        }
        for c in [Curve::MEDIUM_CONTRAST, Curve::STRONG_CONTRAST] {
            assert!(c.eval(0.25) < 0.25 && c.eval(0.75) > 0.75);
            let mut last = 0.0;
            for i in 0..=200 {
                let v = c.eval(i as f32 / 200.0);
                assert!(v >= last - 1e-6, "monotone");
                last = v;
            }
        }
        // It goes through its points.
        let c = Curve::new(&[[0.0, 0.1], [0.3, 0.5], [0.6, 0.55], [1.0, 0.9]]);
        for p in c.points() {
            assert!((c.eval(p[0]) - p[1]).abs() < 1e-5);
        }
        // Flat outside the end points (black and white point moved in).
        let c = Curve::new(&[[0.1, 0.0], [0.9, 1.0]]);
        assert_eq!((c.eval(0.05), c.eval(0.95)), (0.0, 1.0));
        // Sorted, deduplicated, at most 16 points, at least 2.
        let many: Vec<[f32; 2]> = (0..30)
            .map(|i| [i as f32 / 29.0, i as f32 / 29.0])
            .collect();
        assert_eq!(Curve::new(&many).points().len(), Curve::MAX);
        assert_eq!(Curve::new(&[[0.5, 0.5]]), Curve::IDENTITY);
    }

    #[test]
    fn blur_denoise_descreen() {
        // A flat image stays flat.
        let mut flat = vec![100u8; 30 * 10];
        box_blur(&mut flat, 30, 1, 3);
        assert!(flat.iter().all(|&v| v == 100));
        // A strong edge survives the noise reduction.
        let mut edge: Vec<u8> = (0..20 * 5)
            .map(|i| if i % 20 < 10 { 0 } else { 200 })
            .collect();
        let before = edge.clone();
        denoise(&mut edge, 20, 1, 100.0);
        assert_eq!(edge[5], before[5]);
        // A fine screen pattern is flattened.
        let mut screen: Vec<u8> = (0..40 * 40)
            .map(|i| if (i % 40 + i / 40) % 2 == 0 { 50 } else { 200 })
            .collect();
        descreen(&mut screen, 40, 1, 300, 150.0);
        let mid = screen[20 * 40 + 20];
        assert!((110..=140).contains(&mid), "{mid}");
    }

    #[test]
    fn adaptive_threshold_follows_uneven_light() {
        // Text (dark dots) on paper getting darker from left to right: one
        // level loses the right side, the local one keeps paper white.
        let (w, h) = (64, 16);
        let grey: Vec<u8> = (0..w * h)
            .map(|i| {
                let x = i % w;
                let paper = 230 - (x as u32 * 2) as u8;
                if x % 16 == 8 {
                    paper - 90
                } else {
                    paper
                }
            })
            .collect();
        let out = to_lineart_adaptive(&grey, w, 300, 55);
        let white = |x: usize| out[x / 8] & (0x80 >> (x % 8)) != 0;
        assert!(white(2) && white(60), "paper stays white");
        assert!(!white(8) && !white(56), "text stays black");
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
