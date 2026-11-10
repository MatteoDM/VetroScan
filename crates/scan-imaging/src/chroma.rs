// SPDX-License-Identifier: GPL-2.0-or-later
//! Colour noise: the noise of the colour, taken away while the brightness,
//! where the detail is, stays as it is.
//!
//! Measured on five strips of film with the HP 3770 on 10 November 2026
//! (phase 13 of `docs/architecture-plan.md`): in the light parts of a
//! negative's positive the noise is the scanner's, and much more in blue
//! than in red (a light blue T-shirt: 3.9 / 5.8 / 12.1 levels of 255).
//! Nothing here is tuned to a scanner: [`estimate`] measures, on the
//! picture itself, how noisy its colour is against its brightness, and
//! [`radius_for`] smooths the colour only as much as that needs, so a
//! quieter scanner gets less of it, or none.
//!
//! The colour is the ratios red / brightness and blue / brightness (green
//! follows from the brightness): an edge between a dark and a light part
//! of the same colour has the same ratios on both sides, so smoothing them
//! leaves no coloured fringe there.

/// Brightness of linear RGB (Rec. 709).
const KR: f32 = 0.2126;
const KG: f32 = 0.7152;
const KB: f32 = 0.0722;

/// Added to every value before a ratio, so the darkest pixels do not make
/// it explode (0.1 % of the full scale: more would tint the dark side of
/// an edge with the light side's colour).
const FLOOR: f32 = 65.0;

/// The colour's noise left after the smoothing, against the brightness's:
/// well below, where it is not seen.
const TARGET: f32 = 0.3;

/// The widest smoothing, pixels: wider would blur small coloured details.
pub const MAX_RADIUS: f32 = 3.0;

/// Below this the smoothing is not worth doing.
const MIN_RADIUS: f32 = 0.5;

/// How noisy a picture is, relative to its brightness (dimensionless).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Noise {
    /// Of the brightness.
    pub brightness: f32,
    /// Of the colour (the noisier of the two ratios).
    pub colour: f32,
}

/// The noise of 16-bit linear RGB `samples`, `width` pixels a line: each
/// pixel against its left and right neighbours (not the lines above and
/// below, which a scanner's stripes would add to), the median absolute
/// difference over the mid-tones (the clipped parts have no noise to
/// see; edges and texture are a minority the median looks past).
pub fn estimate(samples: &[u16], width: usize) -> Option<Noise> {
    if width < 3 {
        return None;
    }
    let height = samples.len() / (width * 3);
    let mut luma = Vec::new();
    let mut red = Vec::new();
    let mut blue = Vec::new();
    // Every few lines and pixels: plenty for a median.
    let step = (height * width / 200_000).max(1);
    let px = |i: usize| -> [f32; 3] {
        let s = &samples[i * 3..i * 3 + 3];
        [f32::from(s[0]), f32::from(s[1]), f32::from(s[2])]
    };
    let ratios = |p: [f32; 3]| {
        let y = KR * p[0] + KG * p[1] + KB * p[2];
        (
            y,
            (p[0] + FLOOR) / (y + FLOOR),
            (p[2] + FLOOR) / (y + FLOOR),
        )
    };
    let mut i = 0;
    while i < width * height {
        let x = i % width;
        if x > 0 && x + 1 < width {
            let (y, r, b) = ratios(px(i));
            let (yl, rl, bl) = ratios(px(i - 1));
            let (yr, rr, br) = ratios(px(i + 1));
            let (lo, hi) = (0.02 * 65535.0, 0.9 * 65535.0);
            if [y, yl, yr].iter().all(|v| (lo..hi).contains(v)) {
                luma.push(((y - (yl + yr) / 2.0) / y).abs());
                red.push((r - (rl + rr) / 2.0).abs() / r);
                blue.push((b - (bl + br) / 2.0).abs() / b);
            }
        }
        i += step;
    }
    if luma.len() < 1000 {
        return None;
    }
    // MAD to a standard deviation, and the neighbours' average taken out
    // (the difference has 1.5 times the variance of one pixel).
    let sigma = |v: &mut Vec<f32>| median(v) * 1.4826 / 1.5f32.sqrt();
    let brightness = sigma(&mut luma);
    let colour = sigma(&mut red).max(sigma(&mut blue));
    (brightness > 0.0).then_some(Noise { brightness, colour })
}

fn median(v: &mut [f32]) -> f32 {
    let mid = v.len() / 2;
    *v.select_nth_unstable_by(mid, f32::total_cmp).1
}

/// How wide a smoothing of the colour brings its noise to [`TARGET`] of
/// the brightness's (a Gaussian of radius s divides white noise by about
/// 2·√π·s), at most [`MAX_RADIUS`]; `None` when it is already low enough.
pub fn radius_for(noise: Noise) -> Option<f32> {
    let needed = noise.colour / (TARGET * noise.brightness);
    let radius = (needed / (2.0 * std::f32::consts::PI.sqrt())).min(MAX_RADIUS);
    (radius >= MIN_RADIUS).then_some(radius)
}

/// Smooths the colour of 16-bit linear RGB `samples` with a Gaussian of
/// `radius` pixels, the brightness of every pixel kept.
pub fn reduce(samples: &mut [u16], width: usize, radius: f32) {
    if width == 0 || radius <= 0.0 {
        return;
    }
    let n = samples.len() / 3;
    let height = n / width;
    let mut luma = vec![0f32; n];
    let mut red = vec![0f32; n];
    let mut blue = vec![0f32; n];
    for i in 0..n {
        let s = &samples[i * 3..i * 3 + 3];
        let (r, g, b) = (f32::from(s[0]), f32::from(s[1]), f32::from(s[2]));
        let y = KR * r + KG * g + KB * b;
        luma[i] = y;
        red[i] = (r + FLOOR) / (y + FLOOR);
        blue[i] = (b + FLOOR) / (y + FLOOR);
    }
    let kernel = gaussian(radius);
    blur(&mut red, width, height, &kernel);
    blur(&mut blue, width, height, &kernel);
    for i in 0..n {
        let y = luma[i];
        let r = (red[i] * (y + FLOOR) - FLOOR).max(0.0);
        let b = (blue[i] * (y + FLOOR) - FLOOR).max(0.0);
        let g = ((y - KR * r - KB * b) / KG).max(0.0);
        let out = &mut samples[i * 3..i * 3 + 3];
        for (o, v) in out.iter_mut().zip([r, g, b]) {
            *o = v.round().min(65535.0) as u16;
        }
    }
}

/// [`estimate`], [`radius_for`] and [`reduce`] in one: the radius used,
/// if any.
pub fn reduce_auto(samples: &mut [u16], width: usize) -> Option<f32> {
    let radius = radius_for(estimate(samples, width)?)?;
    reduce(samples, width, radius);
    Some(radius)
}

/// A normalised Gaussian, out to three radii.
fn gaussian(radius: f32) -> Vec<f32> {
    let reach = (radius * 3.0).ceil() as i32;
    let k: Vec<f32> = (-reach..=reach)
        .map(|x| (-(x * x) as f32 / (2.0 * radius * radius)).exp())
        .collect();
    let sum: f32 = k.iter().sum();
    k.into_iter().map(|v| v / sum).collect()
}

/// A separable blur of one plane, the edges repeated.
fn blur(plane: &mut [f32], width: usize, height: usize, kernel: &[f32]) {
    let reach = kernel.len() / 2;
    let mut line = Vec::new();
    for row in plane.chunks_exact_mut(width) {
        line.clear();
        line.extend_from_slice(row);
        for (x, out) in row.iter_mut().enumerate() {
            *out = kernel
                .iter()
                .enumerate()
                .map(|(k, w)| w * line[(x + k).saturating_sub(reach).min(width - 1)])
                .sum();
        }
    }
    let mut column = vec![0f32; height];
    for x in 0..width {
        for (y, c) in column.iter_mut().enumerate() {
            *c = plane[y * width + x];
        }
        for y in 0..height {
            plane[y * width + x] = kernel
                .iter()
                .enumerate()
                .map(|(k, w)| w * column[(y + k).saturating_sub(reach).min(height - 1)])
                .sum();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up noise, the same every run.
    struct Rng(u64);

    impl Rng {
        fn normal(&mut self) -> f32 {
            // Sum of uniforms: near enough a Gaussian of deviation 1.
            (0..12)
                .map(|_| {
                    self.0 = self
                        .0
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    (self.0 >> 40) as f32 / (1u64 << 24) as f32
                })
                .sum::<f32>()
                - 6.0
        }
    }

    /// `w` × `h` of one colour with noise `sigma` per channel (fractions of
    /// the value).
    fn flat(w: usize, h: usize, colour: [f32; 3], sigma: [f32; 3]) -> Vec<u16> {
        let mut rng = Rng(7);
        (0..w * h)
            .flat_map(|_| {
                let n = [rng.normal(), rng.normal(), rng.normal()];
                (0..3).map(move |c| (colour[c] * (1.0 + sigma[c] * n[c])).round() as u16)
            })
            .collect()
    }

    fn spread(samples: &[u16], channel: usize) -> f32 {
        let v: Vec<f32> = samples
            .iter()
            .skip(channel)
            .step_by(3)
            .map(|&s| f32::from(s))
            .collect();
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt() / mean
    }

    #[test]
    fn a_noisy_blue_is_brought_near_the_others_and_the_colour_kept() {
        // As the T-shirt: blue three times as noisy as red.
        let (w, h) = (300, 200);
        let mut s = flat(w, h, [12000.0, 20000.0, 30000.0], [0.01, 0.015, 0.03]);
        let mean = |s: &[u16], c: usize| {
            s.iter()
                .skip(c)
                .step_by(3)
                .map(|&v| f64::from(v))
                .sum::<f64>()
                / (w * h) as f64
        };
        let before: Vec<f64> = (0..3).map(|c| mean(&s, c)).collect();
        let radius = reduce_auto(&mut s, w).expect("noisy colour");
        assert!(radius > 1.0 && radius <= MAX_RADIUS, "{radius}");
        assert!(spread(&s, 2) < 0.015, "blue {}", spread(&s, 2));
        for (c, before) in before.iter().enumerate() {
            let after = mean(&s, c);
            assert!(
                (after / before - 1.0).abs() < 0.005,
                "channel {c}: {before} {after}"
            );
        }
    }

    #[test]
    fn a_picture_whose_colour_is_as_quiet_as_its_brightness_is_left_alone() {
        // Noise in the brightness only: the same in the three channels.
        let (w, h) = (300, 200);
        let mut rng = Rng(3);
        let mut s: Vec<u16> = (0..w * h)
            .flat_map(|_| {
                let k = 1.0 + 0.02 * rng.normal();
                [20000.0 * k, 22000.0 * k, 18000.0 * k].map(|v| v.round() as u16)
            })
            .collect();
        let noise = estimate(&s, w).unwrap();
        assert!(noise.colour < noise.brightness * 0.5, "{noise:?}");
        let copy = s.clone();
        assert_eq!(reduce_auto(&mut s, w), None);
        assert_eq!(s, copy);
    }

    #[test]
    fn an_edge_of_brightness_stays_sharp() {
        // The left half dark, the right light, the same colour: the
        // smoothing must not soften the step.
        let (w, h) = (64, 32);
        let mut s = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                let k = if x < w / 2 { 0.2 } else { 1.0 };
                s.extend([30000.0 * k, 40000.0 * k, 50000.0 * k].map(|v: f32| v as u16));
            }
        }
        let copy = s.clone();
        reduce(&mut s, w, MAX_RADIUS);
        // Within half a percent (FLOOR makes the dark side's ratios a
        // little different from the light side's).
        for (a, b) in s.iter().zip(&copy) {
            let (a, b) = (f32::from(*a), f32::from(*b));
            assert!((a - b).abs() <= 0.005 * b, "{a} {b}");
        }
    }
}
