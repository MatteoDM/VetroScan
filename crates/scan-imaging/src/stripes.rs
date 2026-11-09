// SPDX-License-Identifier: GPL-2.0-or-later
//! Horizontal stripes a scanner leaves on every scan: each line a little
//! brighter or darker than the next, with a pattern that repeats after the
//! same number of lines.
//!
//! Measured on the HP 3770 on 9 November 2026 with a white sheet: the
//! pattern repeats every 5 lines at 200, 600 and 1200 dpi and every 10 at
//! 300 dpi, 2 to 4 % at 200 and 300 dpi, below 1 % above; the HP driver's
//! scans have the same. It drifts slowly (5.02 lines at 200 dpi, 4.993 at
//! 1200), and from 1200 dpi up the even and odd pixels come from two rows
//! of the sensor, so they see it on different lines.
//!
//! [`remove`] finds the pattern in the scan itself, for the even and the
//! odd columns and each colour apart: each line against the lines around
//! it, as the median over the columns (the subject of a photo changes along
//! a line, the stripes do not); then, for each line, the median over the
//! lines at the same place of the pattern nearby (an edge of the subject
//! falls on a few of them, the stripes on all). Only what repeats after
//! `repeat` lines is taken away: the brightness of the scan stays as it was.

use crate::{Format, Image};

/// Every how many columns of a parity a line is measured.
const STEP: usize = 4;

/// Repetitions of the pattern on each side of a line that its correction
/// comes from: long enough to see past the subject, short enough to
/// follow the drift.
const REACH: usize = 30;

/// Takes away from `image` (linear RGB, 8 bits) the stripes that repeat
/// after `repeat` lines, and the same from `samples16` (its 16-bit
/// samples, same size), which are measured instead when there are some.
/// Returns whether anything was done: not for an image too small to tell
/// the stripes from the subject.
pub fn remove(image: &mut Image, samples16: Option<&mut [u16]>, repeat: usize) -> bool {
    let (width, height) = (image.width, image.height);
    if image.format != Format::Rgb
        || repeat < 2
        || height < 6 * repeat
        || width < 16 * STEP
        || image.data.len() != width * height * 3
    {
        return false;
    }
    let samples16 = samples16.filter(|s| s.len() == width * height * 3);
    let gains = match &samples16 {
        Some(s) => gains(width, height, repeat, |i| f32::from(s[i])),
        None => gains(width, height, repeat, |i| f32::from(image.data[i])),
    };
    for (line, g) in image.data.chunks_exact_mut(width * 3).zip(&gains) {
        for (i, v) in line.iter_mut().enumerate() {
            *v = (f32::from(*v) * g[((i / 3) % 2) * 3 + i % 3])
                .round()
                .min(255.0) as u8;
        }
    }
    if let Some(s) = samples16 {
        for (line, g) in s.chunks_exact_mut(width * 3).zip(&gains) {
            for (i, v) in line.iter_mut().enumerate() {
                *v = (f32::from(*v) * g[((i / 3) % 2) * 3 + i % 3])
                    .round()
                    .min(65535.0) as u16;
            }
        }
    }
    true
}

/// The factor each line is multiplied by, for the even columns (red,
/// green, blue) then the odd ones.
fn gains(
    width: usize,
    height: usize,
    repeat: usize,
    sample: impl Fn(usize) -> f32,
) -> Vec<[f32; 6]> {
    let mut gains = vec![[1.0f32; 6]; height];
    for parity in 0..2 {
        let columns: Vec<usize> = (parity..width).step_by(2 * STEP).collect();
        let n = columns.len();
        for c in 0..3 {
            // The samples measured, as logarithms: the stripes multiply.
            let mut log = vec![0f32; height * n];
            for y in 0..height {
                for (k, &x) in columns.iter().enumerate() {
                    log[y * n + k] = (sample((y * width + x) * 3 + c) + 1.0).ln();
                }
            }
            // Each line against the mean of the 2 × `repeat` lines around
            // it (a whole number of repetitions: the pattern cancels in
            // it), as the median over the columns.
            let span = 2 * repeat;
            let mut sums = vec![0f64; n];
            for y in 0..span {
                for k in 0..n {
                    sums[k] += f64::from(log[y * n + k]);
                }
            }
            let mut start = 0;
            let mut line = vec![0f32; height];
            let mut differences = vec![0f32; n];
            for (y, l) in line.iter_mut().enumerate() {
                let want = y.saturating_sub(repeat).min(height - span);
                while start < want {
                    for k in 0..n {
                        sums[k] +=
                            f64::from(log[(start + span) * n + k]) - f64::from(log[start * n + k]);
                    }
                    start += 1;
                }
                for k in 0..n {
                    differences[k] = log[y * n + k] - (sums[k] / span as f64) as f32;
                }
                *l = median(&mut differences);
            }
            // The pattern: for each line the median of the lines at the
            // same place of it nearby.
            let mut same = Vec::with_capacity(2 * REACH + 1);
            let pattern: Vec<f32> = (0..height)
                .map(|y| {
                    same.clear();
                    let first = y % repeat;
                    let from = y.saturating_sub(REACH * repeat);
                    let to = (y + REACH * repeat).min(height - 1);
                    let mut j = first + (from.saturating_sub(first)).div_ceil(repeat) * repeat;
                    while j <= to {
                        same.push(line[j]);
                        j += repeat;
                    }
                    median(&mut same)
                })
                .collect();
            // Less its mean over one repetition: the brightness stays.
            for (y, g) in gains.iter_mut().enumerate() {
                let from = y.saturating_sub(repeat / 2).min(height - repeat);
                let mean = pattern[from..from + repeat].iter().sum::<f32>() / repeat as f32;
                g[parity * 3 + c] = (mean - pattern[y]).exp();
            }
        }
    }
    gains
}

/// The longest pattern [`measure`] looks for, lines.
pub const MAX_REPEAT: usize = 16;

/// Stripes below this (row to row, a fraction) are not worth taking away.
pub const FAINTEST: f32 = 0.002;

/// The stripes of a scanner, as [`measure`] finds them on a scan of
/// something uniform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measured {
    /// Lines after which the pattern repeats.
    pub repeat: usize,
    /// How much a line differs from the lines around it, a fraction (0.024
    /// for 2.4 %), before [`remove`].
    pub strength: f32,
    /// The same after [`remove`] with `repeat`.
    pub left: f32,
}

/// Finds the stripes on a scan of something uniform (a white sheet, the
/// lid) at one resolution: for each length from 2 to [`MAX_REPEAT`] lines
/// it works out what [`remove`] would take away, and keeps the shortest
/// that takes away nearly as much as the best (a pattern of 5 lines also
/// repeats after 10). `None` when there are no stripes worth taking away
/// ([`FAINTEST`]), when nothing repeats, or when the scan is too small.
pub fn measure(image: &Image, samples16: Option<&[u16]>) -> Option<Measured> {
    let (width, height) = (image.width, image.height);
    if image.format != Format::Rgb
        || height < 6 * MAX_REPEAT
        || width < 16 * STEP
        || image.data.len() != width * height * 3
    {
        return None;
    }
    let samples16 = samples16.filter(|s| s.len() == width * height * 3);
    let sample = |i: usize| match samples16 {
        Some(s) => f32::from(s[i]),
        None => f32::from(image.data[i]),
    };
    // The mean of each line, for the even and the odd columns and each
    // colour: a line multiplied by its gain has its mean multiplied too.
    let mut means = vec![[0f64; 6]; height];
    for (y, m) in means.iter_mut().enumerate() {
        for x in 0..width {
            for c in 0..3 {
                m[(x % 2) * 3 + c] += f64::from(sample((y * width + x) * 3 + c));
            }
        }
    }
    let before = variation(&means, None);
    if before < FAINTEST {
        return None;
    }
    let tried: Vec<(usize, f32)> = (2..=MAX_REPEAT)
        .map(|repeat| {
            let g = gains(width, height, repeat, sample);
            (repeat, variation(&means, Some(&g)))
        })
        .collect();
    let best = tried.iter().map(|t| t.1).fold(f32::MAX, f32::min);
    // Nothing repeats: the filter would take little away.
    if best > 0.6 * before {
        return None;
    }
    let (repeat, left) = tried
        .into_iter()
        .find(|&(_, left)| left <= best * 1.15 + 0.0002)?;
    Some(Measured {
        repeat,
        strength: before,
        left,
    })
}

/// How much the lines differ from the 15 around them (the root mean
/// square of the ratio less one), averaged over the six series of
/// `means`, each line multiplied by its `gains` if given.
fn variation(means: &[[f64; 6]], gains: Option<&[[f32; 6]]>) -> f32 {
    let k = 7;
    if means.len() <= 2 * k {
        return 0.0;
    }
    let mut total = 0.0;
    for s in 0..6 {
        let series: Vec<f64> = means
            .iter()
            .enumerate()
            .map(|(y, m)| m[s] * gains.map_or(1.0, |g| f64::from(g[y][s])))
            .collect();
        let mut sum = 0.0;
        for y in k..series.len() - k {
            let smooth = series[y - k..=y + k].iter().sum::<f64>() / (2 * k + 1) as f64;
            if smooth > 0.0 {
                sum += (series[y] / smooth - 1.0).powi(2);
            }
        }
        total += (sum / (series.len() - 2 * k) as f64).sqrt();
    }
    (total / 6.0) as f32
}

/// The median of `values` (reordered), 0 for none.
fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mid = values.len() / 2;
    let (_, m, _) = values.select_nth_unstable_by(mid, f32::total_cmp);
    *m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up scan: a grey page with a darker photo on it (an edge
    /// across every line, and one along them), and the stripes of
    /// `amplitude` repeating every `period` lines on top, the odd columns
    /// `shift` lines later.
    fn scan(width: usize, height: usize, period: f32, amplitude: f32, shift: f32) -> Image {
        let mut data = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let subject = if (width / 4..width * 3 / 4).contains(&x)
                    && (height / 3..height * 2 / 3).contains(&y)
                {
                    60.0 + 40.0 * (x as f32 / width as f32)
                } else {
                    180.0
                };
                let t = y as f32 + if x % 2 == 1 { shift } else { 0.0 };
                for c in 0..3 {
                    let phase = std::f32::consts::TAU * t / period + c as f32;
                    let stripe =
                        1.0 + amplitude * phase.sin() + 0.4 * amplitude * (2.0 * phase).cos();
                    data.push((subject * stripe).round().min(255.0) as u8);
                }
            }
        }
        Image {
            width,
            height,
            dpi: 300,
            format: Format::Rgb,
            data,
        }
    }

    /// How much the lines of the page around the photo still differ, as a
    /// fraction: the standard deviation of their means after a smoothing
    /// over 15 lines is taken away.
    fn stripes(image: &Image) -> f32 {
        let w = image.width;
        let means: Vec<f32> = (0..image.height)
            .map(|y| {
                let row = &image.data[y * w * 3..(y * w + w / 5) * 3];
                row.iter().map(|&v| f32::from(v)).sum::<f32>() / row.len() as f32
            })
            .collect();
        let k = 7;
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in k..means.len() - k {
            let smooth = means[y - k..=y + k].iter().sum::<f32>() / (2 * k + 1) as f32;
            sum += (means[y] / smooth - 1.0).powi(2);
            n += 1.0;
        }
        (sum / n).sqrt()
    }

    #[test]
    fn the_stripes_go_and_the_photo_stays() {
        // Every 10 lines exactly (300 dpi), and 5.02 (200 dpi, drifting).
        for (period, repeat) in [(10.0 / 3.0, 10), (5.02, 5)] {
            let clean = scan(400, 600, period, 0.0, 0.0);
            let mut image = scan(400, 600, period, 0.03, 0.0);
            assert!(stripes(&image) > 0.01, "{}", stripes(&image));
            assert!(remove(&mut image, None, repeat));
            assert!(stripes(&image) < 0.004, "{period}: {}", stripes(&image));
            // The photo and the page as they were, within a level or two.
            let worst = image
                .data
                .iter()
                .zip(&clean.data)
                .map(|(&a, &b)| (i32::from(a) - i32::from(b)).abs())
                .max()
                .unwrap();
            assert!(worst <= 3, "{period}: {worst}");
        }
    }

    #[test]
    fn even_and_odd_columns_have_their_own_stripes() {
        // 1200 dpi: the odd pixels see the pattern 2 lines later.
        let mut image = scan(400, 600, 5.0, 0.02, 2.0);
        let before = stripes(&image);
        assert!(remove(&mut image, None, 5));
        assert!(
            stripes(&image) < before / 5.0,
            "{before} {}",
            stripes(&image)
        );
    }

    #[test]
    fn the_16_bit_samples_lose_them_too() {
        let image = scan(400, 600, 10.0 / 3.0, 0.03, 0.0);
        let mut deep: Vec<u16> = image.data.iter().map(|&v| u16::from(v) * 257).collect();
        let mut eight = image.clone();
        assert!(remove(&mut eight, Some(&mut deep), 10));
        let as_eight = Image {
            data: deep.iter().map(|&v| (v >> 8) as u8).collect(),
            ..image.clone()
        };
        assert!(stripes(&as_eight) < 0.004, "{}", stripes(&as_eight));
        assert!(stripes(&eight) < 0.004, "{}", stripes(&eight));
    }

    /// A uniform white sheet with stripes, and a little noise of its own.
    fn white(period: f32, amplitude: f32, shift: f32) -> Image {
        let (width, height) = (300, 400);
        let mut image = scan(width, height, period, amplitude, shift);
        for (i, v) in image.data.iter_mut().enumerate() {
            // A sheet: the same everywhere but for some grain.
            let grain = ((i * 2654435761) % 7) as f32 - 3.0;
            let y = i / (width * 3);
            let x = (i / 3) % width;
            let t = y as f32 + if x % 2 == 1 { shift } else { 0.0 };
            let phase = std::f32::consts::TAU * t / period + (i % 3) as f32;
            let stripe = 1.0 + amplitude * phase.sin() + 0.4 * amplitude * (2.0 * phase).cos();
            *v = (200.0 * stripe + grain).round().clamp(0.0, 255.0) as u8;
        }
        image
    }

    #[test]
    fn measure_finds_how_often_the_stripes_repeat() {
        for (period, shift, repeat) in [
            (10.0 / 3.0, 0.0, 10),
            (5.0, 0.0, 5),
            (5.02, 0.0, 5),
            (2.5, 0.0, 5),
            (4.0, 0.0, 4),
            (5.0, 2.0, 5),
        ] {
            let m = measure(&white(period, 0.02, shift), None)
                .unwrap_or_else(|| panic!("{period}: nothing found"));
            assert_eq!(m.repeat, repeat, "{period}: {m:?}");
            assert!(m.strength > 0.008, "{period}: {m:?}");
            assert!(m.left < m.strength / 3.0, "{period}: {m:?}");
        }
    }

    #[test]
    fn measure_finds_nothing_on_a_clean_sheet() {
        assert_eq!(measure(&white(5.0, 0.0, 0.0), None), None);
    }

    #[test]
    fn a_small_image_is_left_alone() {
        let mut image = scan(400, 40, 10.0 / 3.0, 0.03, 0.0);
        let before = image.data.clone();
        assert!(!remove(&mut image, None, 10));
        assert_eq!(image.data, before);
    }
}
