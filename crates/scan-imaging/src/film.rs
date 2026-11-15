// SPDX-License-Identifier: GPL-2.0-or-later
//! Films in transmitted light: slides and negatives.
//!
//! With a transparency adapter the scanner measures how much light the film
//! lets through, the transmission `T`: linear samples where full scale is
//! the lamp seen through no film (the adapter's calibration). What comes out
//! of here are linear 16-bit samples again, now of the picture, for
//! [`adjust::render_samples`](crate::adjust::render_samples) and
//! [`adjust::render16`](crate::adjust::render16) with the tone curve
//! [`TONE`]; all the usual adjustments follow.
//!
//! **Negatives** are worked on in density, `D = log10(1 / T)`. The film that
//! was never exposed (the base: its edges, the gaps between the frames) is
//! the least dense, orange in colour films (their mask). Every density is
//! taken from the base's, channel by channel: the mask goes away and the
//! base becomes a neutral black. A negative's density grows with the
//! logarithm of the light that reached it, with a slope (the film's gamma,
//! about [`NEGATIVE_GAMMA`]) a little different for each layer; so each
//! channel is stretched on its own, from the base to its brightest
//! highlights: those come out neutral too, and the slope of each layer is
//! made the same. What is left is the logarithm of the scene's light, turned
//! back into linear light over the range the film recorded.
//!
//! **Slides** are positives: their transmission is the picture already. It
//! is only stretched to the full 16 bits, with one black and one white for
//! the three channels, so that its colours stay those of the film.
//!
//! The measures ([`Negative::measure`], [`Slide::measure`]) are taken once,
//! on the preview or the frame, and applied as they are to the scan at full
//! resolution: the same picture on the screen and in the file.
//!
//! [`frames`] finds the slides and the frames of negative strips in a
//! preview of the holder.

use crate::adjust::Tone;
use crate::detect::{self, Found};
use crate::{Format, Image};

/// What is in the holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Film {
    /// Positives: slides, mounted or not.
    Slide,
    /// Negatives, colour or black and white.
    Negative,
}

/// The tone curve of the positives made here: plain display gamma, white
/// at full scale.
pub const TONE: Tone = Tone {
    white: [255.0; 3],
    grey_white: 255.0,
    gamma: 2.2,
};

/// Gamma of negative films, the slope of their density against the
/// logarithm of the light: about 0.6 for colour, 0.55-0.7 for black and
/// white.
pub const NEGATIVE_GAMMA: f32 = 0.6;

/// The widest range of light a positive spans, in decades: a photographic
/// print's, about 100:1.
const MAX_RANGE: f32 = 2.0;

/// A 35 mm frame, along the strip and across it, in millimetres.
pub const FRAME_MM: [f32; 2] = [36.0, 24.0];

/// From one 35 mm frame to the next along the strip, in millimetres.
pub const PITCH_MM: f32 = 38.0;

/// Full scale of the samples: the lamp through no film.
const FULL: f32 = 65535.0;

/// Below this transmission in any channel: the holder or a slide's mount,
/// not film (density 3, about as far as the scanners for the glass see).
const OPAQUE: f32 = 0.001;

/// Above this transmission in every channel: the lamp through no film.
const CLEAR: f32 = 0.95;

/// Density from the base below which, in every channel, a sample is the
/// base itself (the gaps between the frames), left out of the levels.
const NEAR_BASE: f32 = 0.04;

/// Share of the samples left beyond the black and white of the levels.
const CLIP: f64 = 0.001;

/// Share of a negative's samples, the densest, its highlights are measured
/// on (see [`Negative::measure`]).
const HIGH_SHARE: f64 = 0.005;

/// Fewest samples of film to measure anything on.
const FEWEST: usize = 64;

/// Steps of the density histograms.
const DENSITY_STEP: f32 = 0.001;
const DENSITY_BINS: usize = 4000;

fn transmission(v: u16) -> f32 {
    f32::from(v).max(1.0) / FULL
}

/// A sample that is film: not the holder, not the lamp beside the film.
fn is_film(p: &[u16]) -> bool {
    let opaque = (FULL * OPAQUE) as u16;
    let clear = (FULL * CLEAR) as u16;
    p.iter().all(|&v| v >= opaque) && !p.iter().all(|&v| v > clear)
}

/// The film base as transmission per channel: the median of the samples in
/// a rectangle (`x`, `y`, `width`, `height` in pixels) of `samples`, linear
/// RGB `image_width` pixels wide. `None` if the rectangle is empty.
pub fn base_in(samples: &[u16], image_width: usize, rect: [usize; 4]) -> Option<[f32; 3]> {
    let [x, y, w, h] = rect;
    let height = samples.len() / 3 / image_width.max(1);
    let (x1, y1) = ((x + w).min(image_width), (y + h).min(height));
    let mut channels: [Vec<u16>; 3] = Default::default();
    for row in y.min(y1)..y1 {
        for col in x.min(x1)..x1 {
            let i = (row * image_width + col) * 3;
            for (c, values) in channels.iter_mut().enumerate() {
                values.push(samples[i + c]);
            }
        }
    }
    if channels[0].is_empty() {
        return None;
    }
    Some(channels.map(|mut v| {
        let mid = v.len() / 2;
        transmission(*v.select_nth_unstable(mid).1)
    }))
}

/// The film base in the gaps between frames of a strip: the median of the
/// samples between each two `frames` next to each other on the same strip
/// (in pixels of the linear RGB `samples`, `image_width` pixels wide, as
/// [`frames`] gives them), over the middle half of the gap along the strip
/// and of the frames' breadth across it. The gaps are bare film, never
/// exposed: unlike [`base_auto`], light leaking past the film's edges
/// cannot pass for it. Two frames are neighbours when one lies along the
/// other's length, less than a frame away: with strips side by side the
/// reading order goes from one strip to the other (the Epson 4490's holder,
/// two strips), and what lies between them is the holder. A gap that is
/// mostly not film (the holder's bar over it) is left out too. `None` with
/// no gap between neighbours.
pub fn base_between(samples: &[u16], image_width: usize, frames: &[Found]) -> Option<[f32; 3]> {
    let height = samples.len() / 3 / image_width.max(1);
    let mut channels: [Vec<u16>; 3] = Default::default();
    for (i, a) in frames.iter().enumerate() {
        // The frame's length, as a unit vector, and its breadth.
        let (s, c) = a.angle.to_radians().sin_cos();
        let (u, length, breadth) = if a.width >= a.height {
            ([c, s], a.width, a.height)
        } else {
            ([-s, c], a.height, a.width)
        };
        for b in &frames[i + 1..] {
            let (dx, dy) = (b.cx - a.cx, b.cy - a.cy);
            let along_d = dx * u[0] + dy * u[1];
            let across_d = -dx * u[1] + dy * u[0];
            // On the same strip, the next frame along it.
            if across_d.abs() > breadth / 2.0 || along_d.abs() >= 2.0 * length {
                continue;
            }
            let gap = along_d.abs() - length;
            if gap < 2.0 {
                continue;
            }
            let (ux, uy) = if along_d >= 0.0 {
                (u[0], u[1])
            } else {
                (-u[0], -u[1])
            };
            let (mx, my) = ((a.cx + b.cx) / 2.0, (a.cy + b.cy) / 2.0);
            let (half_along, half_across) = (gap / 4.0, breadth / 4.0);
            let reach = (half_along.max(half_across) + 1.0) as i64;
            let mut here: Vec<[u16; 3]> = Vec::new();
            for y in (my as i64 - reach).max(0)..=(my as i64 + reach).min(height as i64 - 1) {
                for x in
                    (mx as i64 - reach).max(0)..=(mx as i64 + reach).min(image_width as i64 - 1)
                {
                    let (px, py) = (x as f32 + 0.5 - mx, y as f32 + 0.5 - my);
                    let along = px * ux + py * uy;
                    let across = -px * uy + py * ux;
                    if along.abs() <= half_along && across.abs() <= half_across {
                        let k = (y as usize * image_width + x as usize) * 3;
                        here.push([samples[k], samples[k + 1], samples[k + 2]]);
                    }
                }
            }
            let film = here.iter().filter(|p| is_film(&p[..])).count();
            if film * 2 < here.len() {
                continue;
            }
            for p in here {
                for (c, values) in channels.iter_mut().enumerate() {
                    values.push(p[c]);
                }
            }
        }
    }
    if channels[0].is_empty() {
        return None;
    }
    Some(channels.map(|mut v| {
        let mid = v.len() / 2;
        transmission(*v.select_nth_unstable(mid).1)
    }))
}

/// The film base found by itself: the least dense film among `samples`
/// (linear RGB). With the edge of the strip or a gap between frames in
/// view it is the base; with only a frame, the darkest shadows of the
/// scene, a little denser, which do as well. `None` with too little film.
pub fn base_auto(samples: &[u16]) -> Option<[f32; 3]> {
    let mut film: Vec<[u16; 3]> = samples
        .chunks_exact(3)
        .filter(|p| is_film(p))
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    if film.len() < FEWEST {
        return None;
    }
    // The least dense: the largest product of the transmissions. The
    // densest 0.2 % of those, so that a few bright specks (dust holes,
    // scratches) do not count.
    let light = |p: &[u16; 3]| p.iter().map(|&v| f64::from(v).max(1.0).ln()).sum::<f64>();
    let n = (film.len() / 500).max(FEWEST / 2);
    film.select_nth_unstable_by(n, |a, b| light(b).total_cmp(&light(a)));
    let top = &film[..n];
    Some(std::array::from_fn(|c| {
        let mut v: Vec<u16> = top.iter().map(|p| p[c]).collect();
        let mid = v.len() / 2;
        transmission(*v.select_nth_unstable(mid).1)
    }))
}

/// The measures of a negative: how to turn it into a positive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Negative {
    /// The film base, transmission per channel (orange in colour films).
    pub base: [f32; 3],
    /// Density above the base of the brightest highlights, per channel:
    /// the mean over the densest pixels of the three channels together.
    pub high: [f32; 3],
    /// The black, as a share of `high` (the same for the three channels:
    /// the base is neutral, the shadows keep its balance).
    pub black: f32,
}

impl Negative {
    /// Measures a negative from its linear RGB `samples` (a frame, or the
    /// preview of the holder: the holder and the lamp are left out), with
    /// the film `base` if known (else [`base_auto`]). `None` with too
    /// little film.
    pub fn measure(samples: &[u16], base: Option<[f32; 3]>) -> Option<Negative> {
        let base = match base {
            Some(b) => b,
            None => base_auto(samples)?,
        };
        // Density above the base of each sample value, per channel, worked
        // out once.
        let densities_of: [Vec<f32>; 3] = std::array::from_fn(|c| {
            let base_log = base[c].clamp(1.0 / FULL, 1.0).log10();
            (0..=u16::MAX)
                .map(|v| base_log - transmission(v).log10())
                .collect()
        });
        let density_of =
            |p: &[u16]| -> [f32; 3] { std::array::from_fn(|c| densities_of[c][usize::from(p[c])]) };
        let mut histograms = vec![[0u64; 3]; DENSITY_BINS];
        let mut counted = 0u64;
        for p in samples.chunks_exact(3).filter(|p| is_film(p)) {
            let d = density_of(p);
            if d.iter().all(|&d| d < NEAR_BASE) {
                continue;
            }
            for (c, &d) in d.iter().enumerate() {
                let bin = ((d / DENSITY_STEP).max(0.0) as usize).min(DENSITY_BINS - 1);
                histograms[bin][c] += 1;
            }
            counted += 1;
        }
        if counted < FEWEST as u64 {
            return None;
        }
        let cut = ((counted as f64 * CLIP) as u64).max(1);
        let density = |c: usize, from_top: bool| -> f32 {
            let mut sum = 0;
            let bins: Box<dyn Iterator<Item = usize>> = if from_top {
                Box::new((0..DENSITY_BINS).rev())
            } else {
                Box::new(0..DENSITY_BINS)
            };
            for bin in bins {
                sum += histograms[bin][c];
                if sum >= cut {
                    return bin as f32 * DENSITY_STEP;
                }
            }
            0.0
        };
        // The highlights: the same pixels for the three channels, the densest
        // by their total density, each channel's density averaged over
        // them. The densest of each channel on its own are different pixels
        // (a saturated colour, a speck), and a first scan of real negatives
        // came out with a cast that way; the brightest parts of a scene are
        // nearly always nearly neutral.
        let total_bins = 3 * DENSITY_BINS;
        let mut totals = vec![0u64; total_bins];
        let mut densities = Vec::with_capacity(counted as usize);
        for p in samples.chunks_exact(3).filter(|p| is_film(p)) {
            let d = density_of(p);
            if d.iter().all(|&d| d < NEAR_BASE) {
                continue;
            }
            let t = d.iter().map(|&d| d.max(0.0)).sum::<f32>();
            totals[((t / DENSITY_STEP) as usize).min(total_bins - 1)] += 1;
            densities.push((t, d));
        }
        let top = ((counted as f64 * HIGH_SHARE) as u64).max(1);
        let mut sum = 0;
        let mut threshold = 0.0;
        for bin in (0..total_bins).rev() {
            sum += totals[bin];
            if sum >= top {
                threshold = bin as f32 * DENSITY_STEP;
                break;
            }
        }
        let mut high = [0f32; 3];
        let mut n = 0f32;
        for (t, d) in &densities {
            if *t >= threshold {
                for c in 0..3 {
                    high[c] += d[c].max(0.0);
                }
                n += 1.0;
            }
        }
        let high: [f32; 3] = high.map(|h| (h / n.max(1.0)).max(0.05));
        let black = (0..3)
            .map(|c| density(c, false) / high[c])
            .fold(1.0f32, f32::min)
            .clamp(0.0, 0.9);
        Some(Negative { base, high, black })
    }

    /// The range of light the positive spans, in decades (log10 units; 2 is
    /// 100:1, about 6.6 stops): the scene's range the negative holds, at
    /// most 2, the range of a photographic print (a first scan of real
    /// negatives with up to 3 came out with the midtones in the shadows).
    pub fn scene_range(&self) -> f32 {
        let high = self.high.iter().sum::<f32>() / 3.0;
        (high * (1.0 - self.black) / NEGATIVE_GAMMA).clamp(1.0, MAX_RANGE)
    }

    /// The positive of linear RGB `samples`: linear 16-bit samples of the
    /// scene's light, black at 0, white at full scale.
    pub fn positive(&self, samples: &[u16]) -> Vec<u16> {
        let tables = self.tables();
        samples
            .iter()
            .enumerate()
            .map(|(i, &v)| tables[i % 3][usize::from(v)])
            .collect()
    }

    /// For each channel, the positive's sample of every sample value.
    fn tables(&self) -> [Vec<u16>; 3] {
        let range = self.scene_range();
        let floor = 10f32.powf(-range);
        std::array::from_fn(|c| {
            let base_log = self.base[c].clamp(1.0 / FULL, 1.0).log10();
            (0..=u16::MAX)
                .map(|v| {
                    let d = base_log - transmission(v).log10();
                    let n = ((d / self.high[c] - self.black) / (1.0 - self.black)).clamp(0.0, 1.0);
                    let light = (10f32.powf(range * (n - 1.0)) - floor) / (1.0 - floor);
                    (light * FULL).round().clamp(0.0, FULL) as u16
                })
                .collect()
        })
    }
}

/// The measures of a slide: its black and white, as samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slide {
    /// Where the darkest channel of the darkest film is.
    pub black: u16,
    /// Where the brightest channel of the brightest film is.
    pub white: u16,
}

impl Slide {
    /// Measures a slide from its linear RGB `samples` (the mount, the
    /// holder and the lamp beside the slide are left out). `None` with too
    /// little film.
    pub fn measure(samples: &[u16]) -> Option<Slide> {
        let mut darkest = vec![0u64; 1 << 16];
        let mut brightest = vec![0u64; 1 << 16];
        let mut counted = 0u64;
        for p in samples.chunks_exact(3).filter(|p| is_film(p)) {
            darkest[usize::from(*p.iter().min().unwrap_or(&0))] += 1;
            brightest[usize::from(*p.iter().max().unwrap_or(&0))] += 1;
            counted += 1;
        }
        if counted < FEWEST as u64 {
            return None;
        }
        let cut = ((counted as f64 * CLIP) as u64).max(1);
        let level = |h: &[u64], from_top: bool| -> u16 {
            let mut sum = 0;
            let mut at = |v: usize| {
                sum += h[v];
                sum >= cut
            };
            let found = if from_top {
                (0..h.len()).rev().find(|&v| at(v))
            } else {
                (0..h.len()).find(|&v| at(v))
            };
            found.unwrap_or(0) as u16
        };
        let black = level(&darkest, false);
        let white = level(&brightest, true).max(black.saturating_add(256));
        Some(Slide { black, white })
    }

    /// The slide stretched to the full 16 bits, linear.
    pub fn positive(&self, samples: &[u16]) -> Vec<u16> {
        let (black, span) = (f32::from(self.black), f32::from(self.white - self.black));
        samples
            .iter()
            .map(|&v| {
                ((f32::from(v) - black) / span * FULL)
                    .round()
                    .clamp(0.0, FULL) as u16
            })
            .collect()
    }
}

/// The positive of a film's linear RGB `samples`, measured on themselves
/// (with the base of a negative if known). `None` with too little film.
pub fn positive(film: Film, samples: &[u16], base: Option<[f32; 3]>) -> Option<Vec<u16>> {
    match film {
        Film::Negative => Negative::measure(samples, base).map(|n| n.positive(samples)),
        Film::Slide => Slide::measure(samples).map(|s| s.positive(samples)),
    }
}

/// The film inside `frames` of `samples` (linear RGB, `image_width` pixels
/// wide), each frame a tenth smaller, away from the holder's edges: what a
/// film's levels are measured on. All of `samples` with no frames.
pub fn inside(samples: &[u16], image_width: usize, frames: &[Found]) -> Vec<u16> {
    if frames.is_empty() || image_width == 0 {
        return samples.to_vec();
    }
    let inner: Vec<Found> = frames
        .iter()
        .map(|f| Found {
            width: f.width * 0.9,
            height: f.height * 0.9,
            ..*f
        })
        .collect();
    let mut v = Vec::new();
    for (i, p) in samples.chunks_exact(3).enumerate() {
        let (x, y) = (i % image_width, i / image_width);
        let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
        if inner.iter().any(|f| f.contains(cx, cy)) {
            v.extend_from_slice(p);
        }
    }
    v
}

/// A strip of negatives scanned in the adapter's holder (16-bit linear RGB
/// `samples`, `width` x `height` pixels at `dpi`): its frames
/// ([`frames`]) and its measures, the base in the gaps between the frames
/// ([`base_between`]), the levels inside them ([`inside`]). `None` with too
/// little film.
pub fn measure_strip(
    samples: &[u16],
    width: usize,
    height: usize,
    dpi: u16,
) -> Option<(Negative, Vec<Found>)> {
    measure_strip_with(samples, width, height, dpi, None).map(|s| (s.negative, s.frames))
}

/// The measures of a strip of negatives ([`measure_strip_with`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Strip {
    /// How the strip becomes its picture.
    pub negative: Negative,
    /// Its frames, in reading order.
    pub frames: Vec<Found>,
    /// The base measured in the gaps between the frames ([`base_between`]):
    /// bare film, what a roll's base is taken from. `None` with a base
    /// given, or with no gap in view (the base then found on the film
    /// itself, [`base_auto`], less sure).
    pub gaps: Option<[f32; 3]>,
}

/// A strip of negatives as [`measure_strip`], with the film's `base` if
/// known: the base of the roll it belongs to, measured once, so that the
/// frames of one roll come out alike. The base given is used as it is; the
/// highlights and the black are the strip's own, measured inside its
/// frames (they follow the scenes, the base does not). Without a base, the
/// one in the gaps between the frames.
pub fn measure_strip_with(
    samples: &[u16],
    width: usize,
    height: usize,
    dpi: u16,
    base: Option<[f32; 3]>,
) -> Option<Strip> {
    if samples.len() != width * height * 3 {
        return None;
    }
    let image = Image {
        width,
        height,
        dpi,
        format: Format::Rgb,
        data: samples.iter().map(|&v| (v >> 8) as u8).collect(),
    };
    let found = frames(&image, Film::Negative);
    let gaps = match base {
        Some(_) => None,
        None => base_between(samples, width, &found),
    };
    let negative = Negative::measure(&inside(samples, width, &found), base.or(gaps))?;
    Some(Strip {
        negative,
        frames: found,
        gaps,
    })
}

/// The strips among the `frames` of negatives found in a holder: the
/// frames lying along one line, one along another's length (any number of
/// frames apart: a frame left blank does not split a strip). Two strips
/// side by side (the Epson 4490's holder) are two groups; the HP 3770's
/// holder, one strip, is one. In the order of their first frame.
pub fn strips_of(frames: &[Found]) -> Vec<Vec<Found>> {
    let mut group: Vec<usize> = (0..frames.len()).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        while group[i] != i {
            group[i] = group[group[i]];
            i = group[i];
        }
        i
    }
    for (i, a) in frames.iter().enumerate() {
        let (s, c) = a.angle.to_radians().sin_cos();
        let (u, breadth) = if a.width >= a.height {
            ([c, s], a.height)
        } else {
            ([-s, c], a.width)
        };
        for (j, b) in frames.iter().enumerate().skip(i + 1) {
            let (dx, dy) = (b.cx - a.cx, b.cy - a.cy);
            let across = -dx * u[1] + dy * u[0];
            if across.abs() <= breadth / 2.0 {
                let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                group[rj] = ri;
            }
        }
    }
    let mut strips: Vec<(usize, Vec<Found>)> = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let r = root(&mut group, i);
        match strips.iter_mut().find(|(k, _)| *k == r) {
            Some((_, v)) => v.push(*f),
            None => strips.push((r, vec![*f])),
        }
    }
    strips.into_iter().map(|(_, v)| v).collect()
}

/// A strip of negatives measured on its own ([`measure_strips_with`]).
#[derive(Debug, Clone, PartialEq)]
pub struct StripPart {
    /// Where it lies: the rectangle around its frames (left, top, width,
    /// height, pixels of the image measured).
    pub bounds: [f32; 4],
    /// Its frames and measures.
    pub strip: Strip,
}

/// Each strip of negatives in a scan of the holder measured on its own
/// ([`strips_of`]): its base in its own gaps (or `base`, a roll's, when
/// given), its highlights and black inside its own frames. Strips side by
/// side may be of different rolls, with bases and exposures of their own:
/// measured together the brightest of one strip would set the white of
/// both (on the Epson 4490, 15 November 2026: two rolls, bases 0.245 /
/// 0.191 / 0.166 and 0.214 / 0.168 / 0.153, the indoor strip came out
/// orange). Strips with too little film are left out.
pub fn measure_strips_with(
    samples: &[u16],
    width: usize,
    height: usize,
    dpi: u16,
    base: Option<[f32; 3]>,
) -> Vec<StripPart> {
    if samples.len() != width * height * 3 {
        return Vec::new();
    }
    let image = Image {
        width,
        height,
        dpi,
        format: Format::Rgb,
        data: samples.iter().map(|&v| (v >> 8) as u8).collect(),
    };
    strips_of(&frames(&image, Film::Negative))
        .into_iter()
        .filter_map(|found| {
            let gaps = match base {
                Some(_) => None,
                None => base_between(samples, width, &found),
            };
            let negative = Negative::measure(&inside(samples, width, &found), base.or(gaps))?;
            let bounds = found
                .iter()
                .map(Found::bounds)
                .reduce(|a, b| {
                    let (l, t) = (a[0].min(b[0]), a[1].min(b[1]));
                    let r = (a[0] + a[2]).max(b[0] + b[2]);
                    let bottom = (a[1] + a[3]).max(b[1] + b[3]);
                    [l, t, r - l, bottom - t]
                })
                .unwrap_or_default();
            Some(StripPart {
                bounds,
                strip: Strip {
                    negative,
                    frames: found,
                    gaps,
                },
            })
        })
        .collect()
}

/// Each frame of the strips measured ([`measure_strips_with`]) with its
/// own highlights and black on its strip's base: the base is the roll's,
/// one for the strip, but the white follows each picture, as a lab's
/// scanner balances each frame (chosen with Matteo on 15 November 2026 on
/// the Epson 4490's strips: one white for a strip made the frames under
/// its brightest sky dark and flat). A frame with too little film takes
/// its strip's measures. Where each lies (left, top, width, height in
/// pixels of `samples`, `width` pixels wide) and its measures.
pub fn measure_frames(
    parts: &[StripPart],
    samples: &[u16],
    width: usize,
) -> Vec<([f32; 4], Negative)> {
    parts
        .iter()
        .flat_map(|part| {
            part.strip.frames.iter().map(move |f| {
                let own = Negative::measure(
                    &inside(samples, width, std::slice::from_ref(f)),
                    Some(part.strip.negative.base),
                );
                (f.bounds(), own.unwrap_or(part.strip.negative))
            })
        })
        .collect()
}

/// The positive of linear RGB `samples` (`width` pixels wide), each pixel
/// turned with the measures of the place it lies in, or the nearest
/// (`strips`: frames or strips, where each lies, left, top, width and
/// height in pixels of these samples, and its measures). With none,
/// `whole`'s.
pub fn positive_by_place(
    strips: &[([f32; 4], Negative)],
    whole: &Negative,
    samples: &[u16],
    width: usize,
) -> Vec<u16> {
    if strips.len() < 2 || width == 0 {
        let m = strips.first().map_or(whole, |(_, m)| m);
        return m.positive(samples);
    }
    let distance = |b: &[f32; 4], x: f32, y: f32| {
        let dx = (b[0] - x).max(x - (b[0] + b[2])).max(0.0);
        let dy = (b[1] - y).max(y - (b[1] + b[3])).max(0.0);
        dx * dx + dy * dy
    };
    // One table per strip, each pixel through its strip's: no more memory
    // than one strip's positive (a frame at 4800 dpi is 35 million pixels).
    let tables: Vec<[Vec<u16>; 3]> = strips.iter().map(|(_, m)| m.tables()).collect();
    samples
        .chunks_exact(3)
        .enumerate()
        .flat_map(|(i, p)| {
            let (x, y) = ((i % width) as f32 + 0.5, (i / width) as f32 + 0.5);
            let mut k = 0;
            let mut best = f32::MAX;
            for (j, (b, _)) in strips.iter().enumerate() {
                let d = distance(b, x, y);
                if d < best {
                    best = d;
                    k = j;
                }
            }
            let t = &tables[k];
            [
                t[0][usize::from(p[0])],
                t[1][usize::from(p[1])],
                t[2][usize::from(p[2])],
            ]
        })
        .collect()
}

/// How a film becomes its picture, whichever it is: taken once (on the
/// preview, or on the scan) and applied as they are to every scan of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Measures {
    /// A negative's: its base, highlights and black.
    Negative(Negative),
    /// A slide's: its black and white.
    Slide(Slide),
}

impl Measures {
    /// The film's linear RGB `samples` turned into its picture, linear.
    pub fn positive(&self, samples: &[u16]) -> Vec<u16> {
        match self {
            Measures::Negative(n) => n.positive(samples),
            Measures::Slide(s) => s.positive(samples),
        }
    }
}

/// Slides scanned in the adapter's holder (16-bit linear RGB `samples`,
/// `width` x `height` pixels at `dpi`): the slides found ([`frames`]) and
/// their measures, taken inside them ([`inside`], away from the edges of
/// the mounts). `None` with too little film.
pub fn measure_slides(
    samples: &[u16],
    width: usize,
    height: usize,
    dpi: u16,
) -> Option<(Slide, Vec<Found>)> {
    if samples.len() != width * height * 3 {
        return None;
    }
    let image = Image {
        width,
        height,
        dpi,
        format: Format::Rgb,
        data: samples.iter().map(|&v| (v >> 8) as u8).collect(),
    };
    let found = frames(&image, Film::Slide);
    let slide = Slide::measure(&inside(samples, width, &found))?;
    Some((slide, found))
}

/// A slide's piece wider than this both ways, millimetres, is its mount
/// lit round by light leaking past it ([`frames`]): wider than a 35 mm
/// picture is high (24 mm) by a quarter.
const MOUNT_MM: f32 = 30.0;

/// The rim of such a mount left out when its picture is sought inside it,
/// millimetres: the leak (up to about 1 mm on the Epson 4490) and the blocks
/// it lit around it.
const RIM_MM: f32 = 3.0;

/// The pictures in a preview of the holder (linear RGB, as the scanner
/// gives it), in reading order: each slide, or each frame of the strips of
/// 35 mm negatives. A slot left empty, or the leader of a strip never
/// exposed, gives nothing.
pub fn frames(image: &Image, film: Film) -> Vec<Found> {
    if image.format != Format::Rgb || image.data.len() != image.width * image.height * 3 {
        return Vec::new();
    }
    let mm = f32::from(image.dpi) / 25.4;
    // Blocks of about 0.5 mm, as in `detect::objects`.
    let block = (usize::from(image.dpi) / 50).max(1);
    let (bw, bh) = (image.width / block, image.height / block);
    if bw < 8 || bh < 8 {
        return Vec::new();
    }
    let mut lit = vec![false; bw * bh];
    let mut clear = vec![false; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let mut sum = [0u32; 3];
            for y in by * block..(by + 1) * block {
                let row = (y * image.width + bx * block) * 3;
                for p in image.data[row..row + block * 3].chunks_exact(3) {
                    for c in 0..3 {
                        sum[c] += u32::from(p[c]);
                    }
                }
            }
            let mean = sum.map(|s| s as f32 / (block * block) as f32 / 255.0);
            let i = by * bw + bx;
            // A little above the opaque level: the holder is never quite
            // black on a preview.
            lit[i] = mean.iter().any(|&m| m > 0.02);
            clear[i] = mean.iter().all(|&m| m > CLEAR);
        }
    }
    // Close gaps of about 1 mm (a dark corner of the picture), fill holes
    // (sprocket holes, dark parts of the picture inside its outline): the
    // mask and the fits of its pieces, in blocks.
    let close = 2;
    // Each fit with a block of its piece.
    let shapes = |lit: &[bool]| -> (Vec<bool>, Vec<(detect::Found, usize)>) {
        let mut mask = detect::dilate(lit, bw, bh, close);
        detect::fill_holes(&mut mask, bw, bh);
        let mask = detect::erode(&mask, bw, bh, close);
        let fits = detect::components(&mask, bw, bh)
            .into_iter()
            .filter_map(|outline| {
                let [x, y] = *outline.first()?;
                Some((detect::fit(&outline)?, y as usize * bw + x as usize))
            })
            .collect();
        (mask, fits)
    };
    let min_side = detect::MIN_SIDE_MM * mm / block as f32;
    // Slides: light leaking past a mount's edges, between it and the holder,
    // draws thin lines around the mount that enclose it whole once the holes
    // are filled (the Epson 4490's holder: lines up to about 1 mm wide around
    // opaque mounts of 51 mm, capture 102). A slide's piece wider than
    // MOUNT_MM both ways is such a mount: its picture is sought inside it,
    // its rim of RIM_MM left out (there: the four pictures, 35 x 23.5 mm, at
    // 200 and at 600 dpi).
    let mount = MOUNT_MM * mm / block as f32;
    let rim = (RIM_MM * mm / block as f32).round() as usize;
    let (mask, fits) = shapes(&lit);
    let mut masks = vec![mask];
    let mut kept: Vec<(detect::Found, usize)> = Vec::new();
    for (f, seed) in fits {
        if f.width < min_side || f.height < min_side {
            continue;
        }
        if film != Film::Slide || f.width.min(f.height) <= mount {
            kept.push((f, 0));
            continue;
        }
        // The piece's own blocks: a ring left open is not filled, and a
        // picture inside it found on its own is not found twice.
        let piece = piece_at(&masks[0], bw, bh, seed);
        let inside = detect::erode(&piece, bw, bh, rim);
        let inner: Vec<bool> = lit.iter().zip(&inside).map(|(&l, &a)| l && a).collect();
        let (inner_mask, inner_fits) = shapes(&inner);
        masks.push(inner_mask);
        let k = masks.len() - 1;
        kept.extend(
            inner_fits
                .into_iter()
                .filter(|(g, _)| g.width >= min_side && g.height >= min_side)
                .map(|(g, _)| (g, k)),
        );
    }
    let scale = block as f32;
    let pieces: Vec<Found> = kept
        .into_iter()
        .filter_map(|(f, k)| {
            let mask = &masks[k];
            // A slot with nothing in it: the lamp, all clear.
            let (mut n, mut bright) = (0usize, 0usize);
            for by in 0..bh {
                for bx in 0..bw {
                    if mask[by * bw + bx] && f.contains(bx as f32, by as f32) {
                        n += 1;
                        bright += usize::from(clear[by * bw + bx]);
                    }
                }
            }
            if bright * 10 > n * 9 {
                return None;
            }
            Some(Found {
                cx: (f.cx + 0.5) * scale,
                cy: (f.cy + 0.5) * scale,
                width: (f.width + 1.0) * scale,
                height: (f.height + 1.0) * scale,
                angle: f.angle,
            })
        })
        .collect();
    let mut found: Vec<Found> = match film {
        Film::Slide => pieces,
        Film::Negative => pieces
            .iter()
            .flat_map(|piece| split_strips(image, piece, mm))
            .flat_map(|strip| strip_frames(image, &strip, mm))
            .collect(),
    };
    detect::reading_order(&mut found);
    found
}

/// The width of a strip of 35 mm film, millimetres.
const STRIP_MM: f32 = 35.0;

/// A piece of the holder's light far wider than a strip of film (`piece`,
/// in pixels; `mm` pixels per millimetre) split into the strips lying side
/// by side in it: light crossing the holder between them (a slot lit at
/// the top of the holder, a hole, a leak at an end) joins them into one
/// piece, whose frames would be sought across both. Across the piece, over
/// the middle of its length, the holder between the strips is mostly dark:
/// each run of mostly lit lines at least most of a strip wide is a strip.
/// A piece no wider than a strip and a half, or with one such run only,
/// stays as it is.
fn split_strips(image: &Image, piece: &Found, mm: f32) -> Vec<Found> {
    let lengthwise = piece.width >= piece.height;
    let (length, breadth) = if lengthwise {
        (piece.width, piece.height)
    } else {
        (piece.height, piece.width)
    };
    if breadth <= 1.5 * STRIP_MM * mm {
        return vec![*piece];
    }
    let (s, c) = piece.angle.to_radians().sin_cos();
    let (along, across) = if lengthwise {
        ([c, s], [-s, c])
    } else {
        ([-s, c], [c, s])
    };
    let lit = |t: f32, k: f32| -> Option<bool> {
        let x = piece.cx + t * along[0] + k * across[0];
        let y = piece.cy + t * along[1] + k * across[1];
        if x < 0.0 || y < 0.0 || x >= image.width as f32 || y >= image.height as f32 {
            return None;
        }
        let i = (y as usize * image.width + x as usize) * 3;
        Some(
            image.data[i..i + 3]
                .iter()
                .any(|&v| f32::from(v) / 255.0 > 0.02),
        )
    };
    // Lines across every half millimetre, each sampled every half
    // millimetre over the middle 80 % of the length (away from a slot or a
    // leak at the ends).
    let step = (0.5 * mm).max(1.0);
    let lines = (breadth / step) as usize;
    let samples = ((0.8 * length) / step).max(1.0) as usize;
    let share: Vec<f32> = (0..lines)
        .map(|j| {
            let k = -breadth / 2.0 + (j as f32 + 0.5) * step;
            let (mut on, mut n) = (0usize, 0usize);
            for i in 0..samples {
                let t = -0.4 * length + (i as f32 + 0.5) * step;
                if let Some(l) = lit(t, k) {
                    n += 1;
                    on += usize::from(l);
                }
            }
            on as f32 / n.max(1) as f32
        })
        .collect();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (j, &v) in share.iter().chain([&0.0]).enumerate() {
        match (v >= 0.5, start) {
            (true, None) => start = Some(j),
            (false, Some(a)) => {
                runs.push((a, j));
                start = None;
            }
            _ => {}
        }
    }
    let strips: Vec<Found> = runs
        .into_iter()
        .filter(|&(a, b)| (b - a) as f32 * step >= 0.6 * STRIP_MM * mm)
        .map(|(a, b)| {
            let mid = -breadth / 2.0 + (a + b) as f32 / 2.0 * step;
            let wide = (b - a) as f32 * step;
            let (width, height) = if lengthwise {
                (length, wide)
            } else {
                (wide, length)
            };
            Found {
                cx: piece.cx + mid * across[0],
                cy: piece.cy + mid * across[1],
                width,
                height,
                angle: piece.angle,
            }
        })
        .collect();
    if strips.len() < 2 {
        vec![*piece]
    } else {
        strips
    }
}

/// The blocks of `mask` joined to block `seed` (4-connected).
fn piece_at(mask: &[bool], w: usize, h: usize, seed: usize) -> Vec<bool> {
    let mut piece = vec![false; mask.len()];
    if !mask[seed] {
        return piece;
    }
    let mut stack = vec![seed];
    piece[seed] = true;
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let mut push = |j: usize| {
            if mask[j] && !piece[j] {
                piece[j] = true;
                stack.push(j);
            }
        };
        if x > 0 {
            push(i - 1);
        }
        if x + 1 < w {
            push(i + 1);
        }
        if y > 0 {
            push(i - w);
        }
        if y + 1 < h {
            push(i + w);
        }
    }
    piece
}

/// The frames along a strip of negatives (`strip`, in pixels; `mm`
/// pixels per millimetre). The gaps between frames are bare base: the
/// samples across the middle of the strip are searched for them every
/// [`PITCH_MM`], each gap then sought a little to either side (cameras
/// differ); a frame lies between two gaps, or a pitch from the one it has.
fn strip_frames(image: &Image, strip: &Found, mm: f32) -> Vec<Found> {
    let (s, c) = strip.angle.to_radians().sin_cos();
    // Along the strip and across it, as unit vectors on the image.
    let lengthwise = strip.width >= strip.height;
    let (along, across, length, breadth) = if lengthwise {
        ([c, s], [-s, c], strip.width, strip.height)
    } else {
        ([-s, c], [c, s], strip.height, strip.width)
    };
    if breadth < 0.8 * FRAME_MM[1] * mm || length < 0.8 * FRAME_MM[0] * mm {
        return Vec::new();
    }
    let pixel = |t: f32, k: f32| -> Option<[u16; 3]> {
        let x = strip.cx + t * along[0] + k * across[0];
        let y = strip.cy + t * along[1] + k * across[1];
        if x < 0.0 || y < 0.0 || x >= image.width as f32 || y >= image.height as f32 {
            return None;
        }
        let i = (y as usize * image.width + x as usize) * 3;
        Some(std::array::from_fn(|c| u16::from(image.data[i + c]) * 257))
    };
    // Lines across the middle 20 mm of the strip, every 0.25 mm along it.
    let step = 0.25 * mm;
    let lines = (length / step) as usize;
    let half_band = 10.0 * mm;
    let mut samples: Vec<Vec<[u16; 3]>> = Vec::with_capacity(lines);
    for j in 0..lines {
        let t = -length / 2.0 + (j as f32 + 0.5) * step;
        let mut line = Vec::new();
        let mut k = -half_band;
        while k <= half_band {
            if let Some(p) = pixel(t, k) {
                line.push(p);
            }
            k += step;
        }
        samples.push(line);
    }
    // How much each line looks like a gap: bare base is even across the
    // strip, and among the least dense film nearby. Judged against the
    // lines within 20 mm, not against one colour of the base: the
    // adapter's light is not even along the strip (on the HP 3770 the
    // middle of the holder gets about 1.3 times the light of its top), and
    // light leaking past the film's edges is less dense than any base.
    let median_log: Vec<Option<[f32; 3]>> = samples
        .iter()
        .map(|line| {
            if line.is_empty() {
                return None;
            }
            Some(std::array::from_fn(|c| {
                let mut v: Vec<u16> = line.iter().map(|p| p[c]).collect();
                let mid = v.len() / 2;
                transmission(*v.select_nth_unstable(mid).1).log10()
            }))
        })
        .collect();
    let even: Vec<f32> = samples
        .iter()
        .zip(&median_log)
        .map(|(line, m)| {
            let Some(m) = m else { return 0.0 };
            let on = line
                .iter()
                .filter(|p| (0..3).all(|c| (transmission(p[c]).log10() - m[c]).abs() < 0.05))
                .count();
            on as f32 / line.len().max(1) as f32
        })
        .collect();
    let light: Vec<f32> = median_log
        .iter()
        .map(|m| m.map_or(f32::MIN, |m| m.iter().sum::<f32>()))
        .collect();
    // The least dense film of each line: the light of its brightest tenth
    // of pixels (a few leaking pixels do not reach it).
    let line_top: Vec<f32> = samples
        .iter()
        .map(|line| {
            let mut v: Vec<f32> = line
                .iter()
                .map(|p| (0..3).map(|c| transmission(p[c]).log10()).sum::<f32>())
                .collect();
            if v.is_empty() {
                return f32::MIN;
            }
            let k = (v.len() * 9 / 10).min(v.len() - 1);
            *v.select_nth_unstable_by(k, f32::total_cmp).1
        })
        .collect();
    let near = (20.0 * mm / step) as usize;
    let bare: Vec<f32> = (0..lines)
        .map(|j| {
            let around = &line_top[j.saturating_sub(near)..(j + near + 1).min(lines)];
            let brightest = around.iter().copied().fold(f32::MIN, f32::max);
            // Within 0.1 of density per channel of the least dense film.
            if light[j] >= brightest - 0.3 {
                even[j]
            } else {
                0.0
            }
        })
        .collect();
    let bare_at = |t: f32| -> f32 {
        let j = ((t + length / 2.0) / step).floor();
        if j < 0.0 || j as usize >= bare.len() {
            0.0
        } else {
            bare[j as usize]
        }
    };
    // Mean bareness over a gap of about 1.5 mm centred at `t`.
    let gap_width = 1.5 * mm;
    let gap_score = |t: f32| -> f32 {
        let n = (gap_width / step).max(1.0) as usize;
        (0..n)
            .map(|i| bare_at(t - gap_width / 2.0 + (i as f32 + 0.5) * step))
            .sum::<f32>()
            / n as f32
    };
    let pitch = PITCH_MM * mm;
    let starts = -length / 2.0;
    // The phase of the gaps that shows the most bare base.
    let mut phase = 0.0;
    let mut best = -1.0;
    let mut p = 0.0;
    while p < pitch {
        let mut t = starts + p;
        let (mut sum, mut n) = (0.0, 0);
        while t < length / 2.0 {
            sum += gap_score(t);
            n += 1;
            t += pitch;
        }
        let score = if n > 0 { sum / n as f32 } else { 0.0 };
        if score > best {
            best = score;
            phase = p;
        }
        p += step;
    }
    // Each gap where it really is, near where the phase puts it, as its two
    // edges: the run of bare base around the best place. The base at the
    // ends of the strip (a leader, the cut) is a gap of any width.
    let frame = [FRAME_MM[0] * mm, FRAME_MM[1] * mm];
    let reach = 3.0 * mm;
    let mut gaps: Vec<Option<[f32; 2]>> = Vec::new();
    let mut t = starts + phase - pitch;
    while t < length / 2.0 + pitch {
        let mut at = None;
        let mut top = 0.5;
        let mut u = t - reach;
        while u <= t + reach {
            let score = gap_score(u);
            if score > top {
                top = score;
                at = Some(u);
            }
            u += step;
        }
        gaps.push(at.map(|u| {
            let (mut left, mut right) = (u, u);
            while left > starts && bare_at(left - step) > 0.5 {
                left -= step;
            }
            while right < length / 2.0 && bare_at(right + step) > 0.5 {
                right += step;
            }
            [left, right]
        }));
        t += pitch;
    }
    // No gap seen (a strip too dark, or all leader): frames centred.
    if best < 0.5 {
        let n = ((length + pitch - frame[0]) / pitch).floor().max(1.0);
        phase = -(n - 1.0) * pitch / 2.0 - pitch / 2.0 - starts + pitch;
        gaps = vec![None; n as usize + 1];
    }
    let expected = |i: usize| starts + phase + (i as f32 - 1.0) * pitch + pitch / 2.0;
    // A gap of the camera, not the end of the strip.
    let narrow = |g: &[f32; 2]| g[1] - g[0] < 5.0 * mm;
    let mut frames = Vec::new();
    for i in 0..gaps.len().saturating_sub(1) {
        let after = gaps[i].filter(narrow).map(|g| g[1] + frame[0] / 2.0);
        let before = gaps[i + 1].filter(narrow).map(|g| g[0] - frame[0] / 2.0);
        let centre = match (after, before, gaps[i], gaps[i + 1]) {
            (Some(a), Some(b), ..) => (a + b) / 2.0,
            (Some(a), None, ..) => a,
            (None, Some(b), ..) => b,
            // Base on both sides, wider than a gap: a frame alone.
            (None, None, Some(a), Some(b)) => (a[1] + b[0]) / 2.0,
            (None, None, Some(a), None) => a[1] + frame[0] / 2.0,
            (None, None, None, Some(b)) => b[0] - frame[0] / 2.0,
            (None, None, None, None) => expected(i),
        };
        // At least most of the frame on the strip.
        let (from, to) = (centre - frame[0] / 2.0, centre + frame[0] / 2.0);
        let inside = to.min(length / 2.0) - from.max(-length / 2.0);
        if inside < 0.85 * frame[0] {
            continue;
        }
        // A frame all bare base was never exposed.
        let lines = (frame[0] / step) as usize;
        let blank = (0..lines)
            .map(|j| bare_at(from + (j as f32 + 0.5) * step))
            .sum::<f32>()
            / lines.max(1) as f32;
        if blank > 0.8 {
            continue;
        }
        let (width, height) = if lengthwise {
            (frame[0].min(length), frame[1].min(breadth))
        } else {
            (frame[1].min(breadth), frame[0].min(length))
        };
        frames.push(Found {
            cx: strip.cx + centre * along[0],
            cy: strip.cy + centre * along[1],
            width,
            height,
            angle: strip.angle,
        });
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The orange mask of a colour negative, transmission per channel.
    const MASK: [f32; 3] = [0.62, 0.38, 0.21];
    /// Slopes of the three layers.
    const SLOPES: [f32; 3] = [0.55, 0.6, 0.66];

    /// The negative of scene light `light` (0-1, linear): densities above
    /// the base along each layer's slope, over a scene of 2.2 decades, with
    /// a little fog over the base.
    fn exposed(light: [f32; 3]) -> [u16; 3] {
        std::array::from_fn(|c| {
            let log = light[c].max(1e-4).log10() + 2.2;
            let d = 0.08 + SLOPES[c] * log.max(0.0);
            (MASK[c] * 10f32.powf(-d) * FULL).round() as u16
        })
    }

    /// A scene: a grey ramp from black to white, and a red, green and blue
    /// patch, 64 pixels each.
    fn scene() -> Vec<[f32; 3]> {
        let mut s: Vec<[f32; 3]> = (0..=255)
            .map(|i| {
                let v = 10f32.powf(-2.2 * (1.0 - i as f32 / 255.0));
                [v; 3]
            })
            .collect();
        for colour in [[0.5, 0.05, 0.05], [0.05, 0.5, 0.05], [0.05, 0.05, 0.5]] {
            s.extend(std::iter::repeat_n(colour, 64));
        }
        s
    }

    fn developed(base: Option<[f32; 3]>) -> Vec<u16> {
        let samples: Vec<u16> = scene().into_iter().flat_map(exposed).collect();
        positive(Film::Negative, &samples, base).expect("film")
    }

    #[test]
    fn a_negative_comes_out_neutral_and_full_range() {
        for base in [Some(MASK), None] {
            let out = developed(base);
            // The grey ramp: neutral, from black to white, never darker
            // going up.
            let ramp: Vec<&[u16]> = out.chunks_exact(3).take(256).collect();
            for p in &ramp {
                let (lo, hi) = (p.iter().min().unwrap(), p.iter().max().unwrap());
                assert!(
                    f32::from(hi - lo) < 0.03 * FULL,
                    "not neutral: {p:?} (base {base:?})"
                );
            }
            assert!(ramp[0].iter().all(|&v| v < 600), "black {:?}", ramp[0]);
            assert!(
                ramp[255].iter().all(|&v| v > 64000),
                "white {:?}",
                ramp[255]
            );
            for w in ramp.windows(2) {
                assert!(
                    u32::from(w[1][1]) + 64 >= u32::from(w[0][1]),
                    "{:?} then {:?}",
                    w[0],
                    w[1]
                );
            }
            // The middle grey of the scene stays near the middle in light.
            let mid = f32::from(ramp[128][1]) / FULL;
            let expected = 10f32.powf(-2.2 * (1.0 - 128.0 / 255.0));
            assert!((mid - expected).abs() < 0.04, "{mid} for {expected}");
        }
    }

    #[test]
    fn colours_keep_their_hue() {
        let out = developed(Some(MASK));
        let patch = |k: usize| -> [u16; 3] {
            let i = (256 + k * 64 + 32) * 3;
            [out[i], out[i + 1], out[i + 2]]
        };
        for k in 0..3 {
            let p = patch(k);
            let others = (0..3).filter(|&c| c != k).map(|c| p[c]).max().unwrap();
            assert!(p[k] > others * 4, "patch {k}: {p:?}");
        }
    }

    #[test]
    fn the_base_and_the_holder_are_left_out() {
        let mut samples: Vec<u16> = scene().into_iter().flat_map(exposed).collect();
        let measured = Negative::measure(&samples, Some(MASK)).unwrap();
        // Gaps of bare base, the black holder and the lamp beside the strip.
        let bare = MASK.map(|m| (m * FULL) as u16);
        for _ in 0..200 {
            samples.extend_from_slice(&bare);
            samples.extend_from_slice(&[3, 2, 1]);
            samples.extend_from_slice(&[65535, 65535, 65535]);
        }
        assert_eq!(Negative::measure(&samples, Some(MASK)), Some(measured));
        // Found by itself among the gaps: the base.
        let base = base_auto(&samples).unwrap();
        for c in 0..3 {
            assert!((base[c] - MASK[c]).abs() < 0.005, "{base:?}");
        }
    }

    #[test]
    fn the_base_between_frames() {
        // 40 x 10 pixels: two frames of picture, the base between them, and
        // a bright leak inside the first frame that must not count.
        let (w, h) = (40, 10);
        let base = [30000u16, 10000, 8000];
        let mut samples = vec![0u16; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                let p = if (17..23).contains(&x) {
                    base
                } else if x == 3 {
                    [60000, 60000, 60000]
                } else {
                    [5000, 3000, 2000]
                };
                samples[i..i + 3].copy_from_slice(&p);
            }
        }
        let frame = |cx: f32| Found {
            cx,
            cy: 5.0,
            width: 16.0,
            height: 10.0,
            angle: 0.0,
        };
        let got = base_between(&samples, w, &[frame(8.0), frame(32.0)]).unwrap();
        assert_eq!(got.map(|t| (t * FULL).round() as u16), base);
        assert_eq!(base_between(&samples, w, &[frame(8.0)]), None);
        // Strips standing up: the same along y.
        let frame_v = |cy: f32| Found {
            cx: 5.0,
            cy,
            width: 10.0,
            height: 16.0,
            angle: 0.0,
        };
        let turned: Vec<u16> = (0..w)
            .flat_map(|x| (0..h).flat_map(move |y| [y, x]))
            .collect::<Vec<usize>>()
            .chunks(2)
            .flat_map(|yx| {
                let i = (yx[0] * w + yx[1]) * 3;
                [samples[i], samples[i + 1], samples[i + 2]]
            })
            .collect();
        let got = base_between(&turned, h, &[frame_v(8.0), frame_v(32.0)]).unwrap();
        assert_eq!(got.map(|t| (t * FULL).round() as u16), base);
    }

    #[test]
    fn the_base_between_frames_of_strips_side_by_side() {
        // Two strips standing up, 12 pixels apart across the opaque holder:
        // in reading order the frames go from one strip to the other, and
        // the base is still the one between the frames of each strip.
        let (w, h) = (40, 40);
        let base = [30000u16, 10000, 8000];
        let mut samples = vec![10u16; w * h * 3];
        for y in 0..h {
            for x in (0..10).chain(22..32) {
                let i = (y * w + x) * 3;
                let p = if (17..23).contains(&y) {
                    base
                } else {
                    [5000, 3000, 2000]
                };
                samples[i..i + 3].copy_from_slice(&p);
            }
        }
        let frame = |cx: f32, cy: f32| Found {
            cx,
            cy,
            width: 10.0,
            height: 16.0,
            angle: 0.0,
        };
        let frames = [
            frame(5.0, 8.0),
            frame(27.0, 8.0),
            frame(5.0, 32.0),
            frame(27.0, 32.0),
        ];
        let got = base_between(&samples, w, &frames).unwrap();
        assert_eq!(got.map(|t| (t * FULL).round() as u16), base);
        // Without a gap between frames of one strip, nothing: the holder
        // between the strips is not the base.
        assert_eq!(base_between(&samples, w, &frames[..2]), None);
    }

    #[test]
    fn a_gap_under_the_holder_is_not_the_base() {
        // Two frames whose gap is covered by an opaque bar.
        let (w, h) = (40, 10);
        let mut samples = vec![[5000u16, 3000, 2000]; w * h];
        for y in 0..h {
            for x in 17..23 {
                samples[y * w + x] = [10, 10, 10];
            }
        }
        let samples: Vec<u16> = samples.into_iter().flatten().collect();
        let frame = |cx: f32| Found {
            cx,
            cy: 5.0,
            width: 16.0,
            height: 10.0,
            angle: 0.0,
        };
        assert_eq!(base_between(&samples, w, &[frame(8.0), frame(32.0)]), None);
    }

    #[test]
    fn strips_side_by_side_are_two_and_a_strip_with_a_blank_frame_one() {
        let frame = |cx: f32, cy: f32| Found {
            cx,
            cy,
            width: 24.0,
            height: 36.0,
            angle: 0.0,
        };
        // In reading order, from one strip to the other; the left strip's
        // second frame left blank (none found there).
        let frames = [
            frame(15.0, 20.0),
            frame(52.0, 21.0),
            frame(52.0, 59.0),
            frame(15.0, 96.0),
            frame(52.0, 97.0),
        ];
        let strips = strips_of(&frames);
        assert_eq!(strips.len(), 2);
        assert_eq!(strips[0].len(), 2);
        assert_eq!(strips[1].len(), 3);
        assert!(strips[0].iter().all(|f| f.cx == 15.0));
        assert_eq!(strips_of(&frames[..1]).len(), 1);
        assert!(strips_of(&[]).is_empty());
    }

    #[test]
    fn each_pixel_is_turned_with_its_own_strips_measures() {
        // Two columns, each of its own film: the same light comes out as each
        // strip's own negative says.
        let a = Negative {
            base: [0.6, 0.4, 0.2],
            high: [1.0; 3],
            black: 0.0,
        };
        let b = Negative {
            base: [0.3, 0.2, 0.1],
            high: [1.0; 3],
            black: 0.0,
        };
        let (w, h) = (10, 2);
        let samples = vec![9000u16; w * h * 3];
        let strips = [([0.0, 0.0, 4.0, 2.0], a), ([6.0, 0.0, 4.0, 2.0], b)];
        let got = positive_by_place(&strips, &a, &samples, w);
        let at = |x: usize| &got[x * 3..x * 3 + 3];
        let pa = a.positive(&samples[..3]);
        let pb = b.positive(&samples[..3]);
        assert_eq!(at(1), &pa[..]);
        assert_eq!(at(8), &pb[..]);
        // Between them, the nearer one.
        assert_eq!(at(4), &pa[..]);
        assert_eq!(at(5), &pb[..]);
        assert_ne!(pa, pb);
    }

    #[test]
    fn each_frame_has_its_own_white_on_its_strips_base() {
        let (image, expected) = strip(0.0, 3, 0.0);
        let samples: Vec<u16> = image.data.iter().map(|&v| u16::from(v) * 257).collect();
        let (w, h, dpi) = (image.width, image.height, image.dpi);
        let parts = measure_strips_with(&samples, w, h, dpi, None);
        assert_eq!(parts.len(), 1);
        let frames = measure_frames(&parts, &samples, w);
        assert_eq!(frames.len(), expected.len());
        for (b, m) in &frames {
            // The strip's base, the frame's own levels.
            assert_eq!(m.base, parts[0].strip.negative.base);
            let f = parts[0]
                .strip
                .frames
                .iter()
                .find(|f| f.bounds() == *b)
                .expect("a frame's place");
            let own =
                Negative::measure(&inside(&samples, w, std::slice::from_ref(f)), Some(m.base))
                    .unwrap();
            assert_eq!(*m, own);
        }
    }

    #[test]
    fn the_base_of_a_rectangle() {
        // 4 x 2 pixels, the base in the right half.
        let mut samples = vec![1000u16; 4 * 2 * 3];
        for y in 0..2 {
            for x in 2..4 {
                let i = (y * 4 + x) * 3;
                samples[i..i + 3].copy_from_slice(&[40000, 25000, 14000]);
            }
        }
        let base = base_in(&samples, 4, [2, 0, 5, 5]).unwrap();
        assert_eq!(
            base.map(|b| (b * FULL).round() as u16),
            [40000, 25000, 14000]
        );
        assert_eq!(base_in(&samples, 4, [9, 9, 2, 2]), None);
    }

    #[test]
    fn too_little_film() {
        assert_eq!(positive(Film::Negative, &[0; 300], None), None);
        assert_eq!(positive(Film::Slide, &[65535; 300], None), None);
    }

    #[test]
    fn a_slide_fills_the_sixteen_bits() {
        // A slide between 2 % and 70 % of the lamp, a little warm.
        let samples: Vec<u16> = (0..1000)
            .flat_map(|i| {
                let t = 0.02 + 0.68 * i as f32 / 999.0;
                [t * 1.0, t * 0.95, t * 0.85].map(|v| (v * FULL) as u16)
            })
            .collect();
        let out = positive(Film::Slide, &samples, None).unwrap();
        let min = *out.iter().min().unwrap();
        let max = *out.iter().max().unwrap();
        assert!(min < 300 && max > 65200, "{min} {max}");
        // The colour of the film stays: red over blue as before.
        let i = 600 * 3;
        let before = f32::from(samples[i]) / f32::from(samples[i + 2]);
        let after = f32::from(out[i]) / f32::from(out[i + 2]);
        assert!(
            after > 1.1 && (after - before).abs() < 0.15,
            "{before} {after}"
        );
    }

    /// A preview of the holder at 150 dpi, black; `pictures` drawn in it.
    fn holder(width: usize, height: usize) -> Image {
        Image {
            width,
            height,
            dpi: 150,
            format: Format::Rgb,
            data: vec![2; width * height * 3],
        }
    }

    fn paint(image: &mut Image, area: &Found, colour: impl Fn(f32, f32) -> [u8; 3]) {
        for y in 0..image.height {
            for x in 0..image.width {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                if area.contains(px, py) {
                    let i = (y * image.width + x) * 3;
                    image.data[i..i + 3].copy_from_slice(&colour(px, py));
                }
            }
        }
    }

    /// Something to look at: dense and thin patches, never bare base.
    fn picture(x: f32, y: f32) -> [u8; 3] {
        let v = ((x / 9.0).sin() * (y / 7.0).cos() * 0.5 + 0.5) * 0.6 + 0.15;
        [
            (v * 0.62 * 200.0) as u8,
            (v * 0.38 * 200.0) as u8,
            (v * 0.21 * 200.0) as u8,
        ]
    }

    fn near(a: &Found, b: &Found, mm: f32) {
        let tol = 1.5 * mm;
        assert!(
            (a.cx - b.cx).abs() < tol
                && (a.cy - b.cy).abs() < tol
                && (a.width - b.width).abs() < tol
                && (a.height - b.height).abs() < tol
                && (a.angle - b.angle).abs() < 0.5,
            "{a:?} != {b:?}"
        );
    }

    fn strip(angle: f32, frames: usize, leader_mm: f32) -> (Image, Vec<Found>) {
        let mm = 150.0 / 25.4;
        let (w, h) = (1100, 420);
        let mut image = holder(w, h);
        let length = (frames as f32 * PITCH_MM + leader_mm + 4.0) * mm;
        let strip = Found {
            cx: w as f32 / 2.0,
            cy: h as f32 / 2.0,
            width: length,
            height: 35.0 * mm,
            angle,
        };
        // The bare base, orange.
        let base = [
            (0.62 * 255.0) as u8,
            (0.38 * 255.0) as u8,
            (0.21 * 255.0) as u8,
        ];
        paint(&mut image, &strip, |_, _| base);
        let (s, c) = angle.to_radians().sin_cos();
        let first = -length / 2.0 + (leader_mm + 3.0 + FRAME_MM[0] / 2.0) * mm;
        let expected: Vec<Found> = (0..frames)
            .map(|i| {
                let t = first + i as f32 * PITCH_MM * mm;
                Found {
                    cx: strip.cx + t * c,
                    cy: strip.cy + t * s,
                    width: FRAME_MM[0] * mm,
                    height: FRAME_MM[1] * mm,
                    angle,
                }
            })
            .collect();
        for f in &expected {
            paint(&mut image, f, picture);
        }
        (image, expected)
    }

    /// A strip as the adapter scans it, at 16 bits: its frames found, its
    /// base the orange between them, a positive of every frame.
    #[test]
    fn a_strip_measured_as_it_comes_from_the_adapter() {
        let (image, expected) = strip(0.0, 3, 0.0);
        let samples: Vec<u16> = image.data.iter().map(|&v| u16::from(v) * 257).collect();
        let (n, found) =
            measure_strip(&samples, image.width, image.height, image.dpi).expect("film");
        assert_eq!(found.len(), expected.len());
        let orange = [0.62, 0.38, 0.21].map(|t: f32| (t * 255.0).floor() / 255.0);
        for (b, o) in n.base.iter().zip(orange) {
            assert!((b - o).abs() < 0.01, "{:?}", n.base);
        }
        // The levels from inside the frames only: the holder and the base
        // are not in them.
        let film = inside(&samples, image.width, &found);
        assert!(!film.is_empty() && film.len() < samples.len() / 4);
        assert!(film.chunks_exact(3).all(|p| p[0] > 600));
        assert_eq!(inside(&samples, image.width, &[]), samples);
        assert!(measure_strip(&samples, image.width + 1, image.height, 150).is_none());
    }

    /// A roll's base, given: used as it is, not the strip's own; the
    /// highlights and the black still measured on the strip.
    #[test]
    fn a_strip_measured_with_the_base_of_its_roll() {
        let (image, _) = strip(0.0, 3, 0.0);
        let samples: Vec<u16> = image.data.iter().map(|&v| u16::from(v) * 257).collect();
        let (w, h, dpi) = (image.width, image.height, image.dpi);
        let own = measure_strip_with(&samples, w, h, dpi, None).expect("film");
        let gaps = own.gaps.expect("the base between the frames");
        assert_eq!(own.negative.base, gaps);
        // The same base given: the same measures.
        let same = measure_strip_with(&samples, w, h, dpi, Some(gaps)).expect("film");
        assert_eq!(same.negative, own.negative);
        assert_eq!(same.gaps, None, "not measured with a base given");
        // A roll's base a little denser than this strip's gaps (another
        // frame of the roll, another day): it is the one used, and the
        // levels are measured from it, on this strip.
        let roll = gaps.map(|b| b * 0.9);
        let locked = measure_strip_with(&samples, w, h, dpi, Some(roll)).expect("film");
        assert_eq!(locked.negative.base, roll);
        assert_eq!(locked.frames, own.frames);
        let expected = Negative::measure(&inside(&samples, w, &own.frames), Some(roll)).unwrap();
        assert_eq!(locked.negative, expected);
        // From a denser base every density is lower: the highlights too.
        for c in 0..3 {
            assert!(locked.negative.high[c] < own.negative.high[c], "{c}");
        }
        // A single frame has no gap: no base of its own to lock.
        let (one, _) = strip(0.0, 1, 0.0);
        let one16: Vec<u16> = one.data.iter().map(|&v| u16::from(v) * 257).collect();
        if let Some(s) = measure_strip_with(&one16, one.width, one.height, one.dpi, None) {
            assert_eq!(s.gaps, None);
        }
    }

    #[test]
    fn frames_along_a_strip_of_negatives() {
        let mm = 150.0 / 25.4;
        for (angle, leader) in [(0.0, 0.0), (1.5, 0.0), (0.0, 30.0)] {
            let (image, expected) = strip(angle, 4, leader);
            let found = frames(&image, Film::Negative);
            assert_eq!(found.len(), expected.len(), "{found:?} (angle {angle})");
            for (f, e) in found.iter().zip(&expected) {
                near(f, e, mm);
            }
        }
    }

    #[test]
    fn a_strip_with_no_gap_to_see() {
        // The picture runs over the gaps too: the frames are centred.
        let mm = 150.0 / 25.4;
        let (mut image, _) = strip(0.0, 3, 0.0);
        let length = (3.0 * PITCH_MM + 4.0) * mm;
        let all = Found {
            cx: 550.0,
            cy: 210.0,
            width: length,
            height: 35.0 * mm,
            angle: 0.0,
        };
        paint(&mut image, &all, picture);
        let found = frames(&image, Film::Negative);
        assert_eq!(found.len(), 3, "{found:?}");
        for (i, f) in found.iter().enumerate() {
            let e = Found {
                cx: 550.0 + (i as f32 - 1.0) * PITCH_MM * mm,
                cy: 210.0,
                width: FRAME_MM[0] * mm,
                height: FRAME_MM[1] * mm,
                angle: 0.0,
            };
            near(f, &e, mm);
        }
    }

    #[test]
    fn two_strips_joined_by_light_are_split() {
        // Two strips one above the other, 5 mm of holder between them,
        // joined at one end by a band of light (a slot of the holder lit
        // across both): each strip's frames, not frames across both.
        let mm = 150.0 / 25.4;
        let (a, expected_a) = strip(0.0, 3, 0.0);
        let (w, h) = (a.width, a.height);
        let gap = (35.0 + 5.0) * mm;
        let mut image = holder(w, h + gap as usize + 10);
        let shift = |f: &Found, dy: f32| Found {
            cy: f.cy + dy,
            ..*f
        };
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                let p = [a.data[i], a.data[i + 1], a.data[i + 2]];
                if p.iter().any(|&v| v > 10) {
                    for dy in [0, gap as usize] {
                        let j = ((y + dy) * w + x) * 3;
                        image.data[j..j + 3].copy_from_slice(&p);
                    }
                }
            }
        }
        // The band of light across both, at the left end.
        let length = (3.0 * PITCH_MM + 4.0) * mm;
        let band = Found {
            cx: w as f32 / 2.0 - length / 2.0 + 2.0,
            cy: h as f32 / 2.0 + gap / 2.0,
            width: 8.0,
            height: gap + 35.0 * mm,
            angle: 0.0,
        };
        paint(&mut image, &band, |_, _| [250, 250, 250]);
        let found = frames(&image, Film::Negative);
        assert_eq!(found.len(), 6, "{found:?}");
        let expected: Vec<Found> = expected_a
            .iter()
            .flat_map(|f| [*f, shift(f, gap)])
            .collect();
        for e in &expected {
            assert!(
                found
                    .iter()
                    .any(|f| (f.cx - e.cx).abs() < 1.5 * mm && (f.cy - e.cy).abs() < 1.5 * mm),
                "{e:?} not in {found:?}"
            );
        }
        assert_eq!(strips_of(&found).len(), 2);
    }

    #[test]
    fn a_strip_standing_up() {
        let mm = 150.0 / 25.4;
        let (image, expected) = strip(0.0, 2, 0.0);
        let turned = image.rotated(1);
        let found = frames(&turned, Film::Negative);
        assert_eq!(found.len(), 2, "{found:?}");
        for (f, e) in found.iter().zip(&expected) {
            // Turned clockwise: (x, y) goes to (h - y, x).
            let e = Found {
                cx: image.height as f32 - e.cy,
                cy: e.cx,
                width: e.height,
                height: e.width,
                angle: 0.0,
            };
            near(f, &e, mm);
        }
    }

    #[test]
    fn slides_in_their_mounts_and_an_empty_slot() {
        let mm = 150.0 / 25.4;
        let mut image = holder(900, 400);
        let window = |cx: f32| Found {
            cx,
            cy: 200.0,
            width: 34.0 * mm,
            height: 23.0 * mm,
            angle: 0.0,
        };
        let slides = [window(200.0), window(450.0)];
        for s in &slides {
            paint(&mut image, s, picture);
        }
        // The third slot is empty: the lamp.
        paint(&mut image, &window(700.0), |_, _| [255, 255, 255]);
        let found = frames(&image, Film::Slide);
        assert_eq!(found.len(), 2, "{found:?}");
        for (f, e) in found.iter().zip(&slides) {
            near(f, e, mm);
        }
    }

    /// Light leaking past an opaque mount's edges (the Epson 4490's holder,
    /// capture 102) draws a thin ring around it: the picture inside is found,
    /// not the whole mount; with the ring left open on a side, the picture once;
    /// a dark band across a picture does not cut it.
    #[test]
    fn slides_with_light_leaking_round_their_mounts() {
        let mm = 150.0 / 25.4;
        let mut image = holder(1300, 450);
        let mut slides = Vec::new();
        for (i, cx) in [220.0, 600.0, 980.0].into_iter().enumerate() {
            let mount = |inset: f32| Found {
                cx,
                cy: 225.0,
                width: (51.0 - inset) * mm,
                height: (51.0 - inset) * mm,
                angle: 0.0,
            };
            // The leak: a line about 0.7 mm wide all round, the lamp's full
            // light; inside it the mount, opaque.
            paint(&mut image, &mount(0.0), |_, _| [255, 255, 255]);
            paint(&mut image, &mount(1.4), |_, _| [1, 1, 1]);
            if i == 1 {
                // Open on its left side.
                let side = Found {
                    cx: cx - 25.5 * mm,
                    cy: 225.0,
                    width: 2.0 * mm,
                    height: 52.0 * mm,
                    angle: 0.0,
                };
                paint(&mut image, &side, |_, _| [1, 1, 1]);
            }
            let window = Found {
                cx,
                cy: 225.0,
                width: 35.0 * mm,
                height: 23.5 * mm,
                angle: 0.0,
            };
            paint(&mut image, &window, picture);
            if i == 2 {
                // A dark band 2.5 mm high across the picture (a hedge in shade).
                let band = Found {
                    cx,
                    cy: 225.0 + 4.0 * mm,
                    width: 35.0 * mm,
                    height: 2.5 * mm,
                    angle: 0.0,
                };
                paint(&mut image, &band, |_, _| [2, 2, 2]);
            }
            slides.push(window);
        }
        let found = frames(&image, Film::Slide);
        assert_eq!(found.len(), 3, "{found:?}");
        for (f, e) in found.iter().zip(&slides) {
            near(f, e, mm);
        }
    }

    /// Slides are measured inside the slides found: the dark bevel of a
    /// mount around the picture is not the picture's black.
    #[test]
    fn slides_measured_inside_their_mounts() {
        let mm = 150.0 / 25.4;
        let mut image = holder(700, 400);
        let window = |cx: f32, inset: f32| Found {
            cx,
            cy: 200.0,
            width: (34.0 - inset) * mm,
            height: (23.0 - inset) * mm,
            angle: 0.0,
        };
        for cx in [200.0, 450.0] {
            // A rim 1 mm wide, darker than the picture, lighter than the
            // holder: film to `is_film`, not lit to `frames`.
            paint(&mut image, &window(cx, 0.0), |_, _| [3, 3, 3]);
            paint(&mut image, &window(cx, 2.0), picture);
        }
        let samples: Vec<u16> = image.data.iter().map(|&v| u16::from(v) * 257).collect();
        let (w, h) = (image.width, image.height);
        let (slide, found) = measure_slides(&samples, w, h, image.dpi).expect("film");
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(Some(slide), Slide::measure(&inside(&samples, w, &found)));
        // All of it: the rim, or the holder, would be the black.
        let whole = Slide::measure(&samples).unwrap();
        assert!(whole.black <= 3 * 257, "{whole:?}");
        assert!(slide.black > 3 * 257, "{slide:?}");
        let measures = Measures::Slide(slide);
        assert_eq!(measures.positive(&samples), slide.positive(&samples));
        // Nothing but an opaque holder: nothing to measure.
        let empty = vec![50; w * h * 3];
        assert_eq!(measure_slides(&empty, w, h, image.dpi), None);
        assert_eq!(measure_slides(&samples, w, h + 1, image.dpi), None);
    }
}
