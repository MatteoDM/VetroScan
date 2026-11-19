// SPDX-License-Identifier: GPL-2.0-or-later
//! Dust, hairs and scratches on a film, found in its infrared and filled from
//! their neighbours (what scanners call Digital ICE).
//!
//! A film's dyes let infrared through and dust does not, so a scanner that
//! reads an infrared channel laid on the colours pixel for pixel
//! (`scanner_api::ScanResult::infrared`) shows the dust as dark marks on a
//! nearly even grey. [`remove`] works only from what it measures on the scan
//! and its infrared (phase 12, step 17 of the plan, measured against a
//! scanner maker's own removal on nine scans of negatives and slides):
//!
//! 1. **the film**: where the infrared's local mean is near the film's (its
//!    90th percentile); a holder's edge is dark in infrared too, and it is no
//!    dust;
//! 2. **the infrared's contrast**: its logarithm less its local mean over a
//!    31 x 31 window (the dust found once is left out of the mean, so a big
//!    speck does not hide itself), and its noise on clean film;
//! 3. **the crosstalk**: a film's dyes still take a little infrared (a
//!    negative's cyan most), so the infrared carries a faint copy of the
//!    picture; its share of each colour's own contrast is fitted on the clean
//!    film (least squares) and taken out;
//! 4. **the mask**: the pixels [`DARK`] times the noise below their mean;
//! 5. **each colour's own mask**: the colours of a film scanner are not quite
//!    on each other, and the dust's shadow in each lies a little away from the
//!    infrared's (up to more than a line on a slide); how far is measured on
//!    the dust itself (the correlation of the infrared's darkness with each
//!    colour's), the mask moved by it and grown by a pixel;
//!    How deep each colour's shadow is against the infrared's is measured
//!    there too (the visible a little deeper than the infrared);
//! 6. **the fill**: where the dust still lets most of the light through
//!    ([`DEEP`] of the infrared or more) the colour is divided by what it took,
//!    which keeps the grain and the picture under it; under deeper dust, and a
//!    pixel round it, the colour is filled along the line through the pixel,
//!    of four, whose two ends outside the mask agree best (a hair is bridged
//!    across, a speck by an edge of the picture filled along the edge; a mark
//!    wider than the window stays as it is).

use crate::{Format, Image};

/// Half the side of the window the infrared and the colours are measured
/// against (31 x 31 pixels): wider than a speck or a hair, narrower than the
/// picture's own changes in the infrared.
const REACH: usize = 15;

/// How dark, in its own noise on clean film, the infrared's contrast must be
/// for a pixel to be dust.
pub const DARK: f32 = 4.0;

/// The film: where the infrared's local mean is at least this share of the
/// film's level (the infrared's 90th percentile).
const FILM: f32 = 0.8;

/// Lines and columns each colour's dust is looked for around the infrared's.
const SEARCH: i32 = 3;

/// The furthest a pixel is filled from.
const FILL_REACH: usize = 2 * REACH + 2;

/// Lines measured at a time (the measures are local: a band and its margin
/// hold what a line needs).
const BAND: usize = 512;

/// A band's margin: the film's erosion and the mean's window, and the search.
const MARGIN: usize = 2 * REACH + 2 + SEARCH as usize + 1;

/// How far, in pixels, each colour's mask is grown round the dust moved to
/// it, and the deep dust round itself: the colours' shadows are a little wider
/// than the infrared's.
const GROW: usize = 1;

/// Fewer dust pixels than this, and no colour's shift or depth is measured.
const MIN_DUST: usize = 20;

/// Under dust that lets through less than this share of the infrared the
/// colours are taken as lost and filled from around; under lighter dust they
/// are divided by what the dust took (as the infrared says), which keeps the
/// grain and the picture under it.
pub const DEEP: f32 = 0.9;

/// What [`remove`] measured and did.
#[derive(Debug, Clone, PartialEq)]
pub struct Removal {
    /// The infrared's noise on clean film: the robust standard deviation of
    /// its log contrast once the crosstalk is out.
    pub noise: f32,
    /// The contrast below which a pixel is dust ([`DARK`] x `noise`).
    pub threshold: f32,
    /// The crosstalk taken out: each colour's (red, green, blue) log contrast
    /// times this is the picture's share of the infrared's.
    pub crosstalk: [f32; 3],
    /// Where each colour's dust lies from the infrared's, lines and columns.
    pub shifts: [[f32; 2]; 3],
    /// How much deeper each colour's shadow is than the infrared's (the log of
    /// the light it lets through, over the infrared's).
    pub depth: [f32; 3],
    /// Pixels of film (where dust is looked for).
    pub film: usize,
    /// Pixels found dark in the infrared.
    pub found: usize,
    /// Pixels of each colour's mask (moved and grown) divided by what the
    /// dust took.
    pub divided: [usize; 3],
    /// Pixels of each colour's mask under dust deeper than [`DEEP`], filled
    /// from around.
    pub filled: [usize; 3],
    /// Pixels of the masks with nothing outside them within reach, left.
    pub left: usize,
}

/// Takes away from `image` (linear RGB, 8 bits) and from `samples16` (its
/// 16-bit samples, same size, measured instead when there are some) the dust
/// its `infrared` shows (one 16-bit sample a pixel, laid on the colours).
/// Nothing is done, and `None` returned, for an image of another layout or
/// size, or one too small to measure.
pub fn remove(
    image: &mut Image,
    mut samples16: Option<&mut [u16]>,
    infrared: &[u16],
) -> Option<Removal> {
    let (w, h) = (image.width, image.height);
    let n = w.checked_mul(h)?;
    if image.format != Format::Rgb
        || image.data.len() != n * 3
        || infrared.len() != n
        || w < 2 * REACH + 1
        || h < 2 * REACH + 1
        || samples16.as_ref().is_some_and(|s| s.len() != n * 3)
    {
        return None;
    }
    let (removal, masks, dark) = {
        let src = match &samples16 {
            Some(s) => Source::Deep(s),
            None => Source::Byte(&image.data),
        };
        find(&Ctx {
            w,
            h,
            ir: infrared,
            src,
            level: percentile(infrared, 0.9),
        })
    };
    let mut removal = removal;
    let deepest = DEEP.ln();
    for (c, mask) in masks.iter().enumerate() {
        let Some(mask) = mask else { continue };
        // The infrared's contrast where this colour's dust lies: how much the
        // dust took, and whether it took nearly all.
        let [sy, sx] = removal.shifts[c];
        let depth = removal.depth[c];
        let mut core = vec![false; n];
        let mut contrast = Vec::new();
        for (i, _) in mask.iter().enumerate().filter(|(_, &m)| m) {
            let (y, x) = ((i / w) as f32, (i % w) as f32);
            let r = darkest(&dark, w, h, y - sy, x - sx);
            core[i] = r < deepest;
            contrast.push((i, r));
        }
        // The deep dust and a pixel round it (its shadow in the colours is a
        // little wider than in the infrared), within the mask, filled; the
        // rest divided.
        let mut deep = vec![false; n];
        for &(i, _) in &contrast {
            let (y, x) = (i / w, i % w);
            deep[i] = (y.saturating_sub(GROW)..=(y + GROW).min(h - 1)).any(|yy| {
                (x.saturating_sub(GROW)..=(x + GROW).min(w - 1)).any(|xx| core[yy * w + xx])
            });
        }
        let gains: Vec<(usize, f32)> = contrast
            .iter()
            .filter(|&&(i, _)| !deep[i])
            .map(|&(i, r)| (i, (-depth * r).exp()))
            .collect();
        removal.divided[c] = gains.len();
        for &(i, g) in &gains {
            let v = &mut image.data[i * 3 + c];
            *v = (f32::from(*v) * g).round().clamp(0.0, 255.0) as u8;
        }
        let (filled, left) = fill(&mut image.data, w, h, c, &deep, f32::from, |v| {
            v.round().clamp(0.0, 255.0) as u8
        });
        if let Some(s) = samples16.as_deref_mut() {
            for &(i, g) in &gains {
                let v = &mut s[i * 3 + c];
                *v = (f32::from(*v) * g).round().clamp(0.0, 65535.0) as u16;
            }
            fill(s, w, h, c, &deep, f32::from, |v| {
                v.round().clamp(0.0, 65535.0) as u16
            });
        }
        removal.filled[c] = filled;
        removal.left += left;
    }
    Some(removal)
}

/// The lowest of `v` (`w` x `h`) at the whole places either side of a place
/// between pixels (as the masks are moved, [`moved`]): a shadow's edge that
/// falls between two lines is taken whole, not half.
fn darkest(v: &[f32], w: usize, h: usize, y: f32, x: f32) -> f32 {
    let y = y.clamp(0.0, (h - 1) as f32);
    let x = x.clamp(0.0, (w - 1) as f32);
    let (y0, x0) = (y.floor() as usize, x.floor() as usize);
    let (y1, x1) = (
        (y.ceil() as usize).min(h - 1),
        (x.ceil() as usize).min(w - 1),
    );
    v[y0 * w + x0]
        .min(v[y0 * w + x1])
        .min(v[y1 * w + x0])
        .min(v[y1 * w + x1])
}

/// Where the colours are read from.
enum Source<'a> {
    Deep(&'a [u16]),
    Byte(&'a [u8]),
}

impl Source<'_> {
    fn get(&self, pixel: usize, c: usize) -> f32 {
        match self {
            Source::Deep(s) => f32::from(s[pixel * 3 + c]),
            Source::Byte(s) => f32::from(s[pixel * 3 + c]) * 257.0,
        }
    }
}

struct Ctx<'a> {
    w: usize,
    h: usize,
    ir: &'a [u16],
    src: Source<'a>,
    /// The film's infrared (its 90th percentile).
    level: f32,
}

/// One band's measures, lines `a..b` of the image.
struct Band {
    a: usize,
    film: Vec<bool>,
    /// The infrared's log contrast (less the crosstalk when asked).
    r: Vec<f32>,
    /// Each colour's log contrast, when asked.
    colours: Option<[Vec<f32>; 3]>,
}

/// The measuring passes: the noise, the first mask, the crosstalk, the
/// corrected noise, the mask and the colours' shifts, the colours' depth; then
/// each colour's mask, and the infrared's contrast where it is dark (0 where
/// not, nor off the film).
fn find(ctx: &Ctx) -> (Removal, [Option<Vec<bool>>; 3], Vec<f32>) {
    let (w, h) = (ctx.w, ctx.h);
    let n = w * h;
    // 1. The plain contrast's noise, and the dust it shows, left out of the
    //    mean from now on.
    let mut film = 0usize;
    let mut sample = Vec::new();
    bands(ctx, None, None, |band, y0, y1| {
        for i in rows(band, y0, y1, w) {
            if band.film[i] {
                film += 1;
                if (band.a * w + i) % 7 == 0 {
                    sample.push(band.r[i]);
                }
            }
        }
    });
    let noise0 = spread(&mut sample);
    let mut first = vec![false; n];
    bands(ctx, None, None, |band, y0, y1| {
        for i in rows(band, y0, y1, w) {
            first[band.a * w + i] = band.film[i] && band.r[i] < -DARK * noise0;
        }
    });
    // 2. The crosstalk, fitted on the clean film.
    let mut ata = [[0f64; 3]; 3];
    let mut atb = [0f64; 3];
    bands(ctx, Some(&first), Some([0.0; 3]), |band, y0, y1| {
        let cs = band.colours.as_ref().expect("colours");
        for i in rows(band, y0, y1, w) {
            if !band.film[i] || band.r[i] < -3.0 * noise0 {
                continue;
            }
            let x = [cs[0][i], cs[1][i], cs[2][i]].map(f64::from);
            for (j, row) in ata.iter_mut().enumerate() {
                for (k, v) in row.iter_mut().enumerate() {
                    *v += x[j] * x[k];
                }
                atb[j] += x[j] * f64::from(band.r[i]);
            }
        }
    });
    let crosstalk = solve(ata, atb).map_or([0.0; 3], |a| a.map(|v| v as f32));
    // 3. The noise once the crosstalk is out, and the mask.
    let mut sample = Vec::new();
    bands(ctx, Some(&first), Some(crosstalk), |band, y0, y1| {
        for i in rows(band, y0, y1, w) {
            if band.film[i] && (band.a * w + i) % 7 == 0 {
                sample.push(band.r[i]);
            }
        }
    });
    let noise = spread(&mut sample);
    let threshold = DARK * noise;
    let mut mask = vec![false; n];
    let mut dark = vec![0f32; n];
    let mut found = 0usize;
    // Each colour's darkness against the infrared's, for each shift.
    let side = (2 * SEARCH + 1) as usize;
    let mut corr = [
        vec![0f64; side * side],
        vec![0f64; side * side],
        vec![0f64; side * side],
    ];
    bands(ctx, Some(&first), Some(crosstalk), |band, y0, y1| {
        let cs = band.colours.as_ref().expect("colours");
        let lines = band.r.len() / w;
        for i in rows(band, y0, y1, w) {
            if band.film[i] {
                dark[band.a * w + i] = band.r[i].min(0.0);
            }
            if !(band.film[i] && band.r[i] < -threshold) {
                continue;
            }
            mask[band.a * w + i] = true;
            found += 1;
            let dark = f64::from(-band.r[i]);
            let (y, x) = ((i / w) as i32, (i % w) as i32);
            for dy in -SEARCH..=SEARCH {
                let yy = y + dy;
                if yy < 0 || yy >= lines as i32 {
                    continue;
                }
                for dx in -SEARCH..=SEARCH {
                    let xx = x + dx;
                    if xx < 0 || xx >= w as i32 {
                        continue;
                    }
                    let j = yy as usize * w + xx as usize;
                    let at = ((dy + SEARCH) as usize) * side + (dx + SEARCH) as usize;
                    for (c, cc) in corr.iter_mut().enumerate() {
                        cc[at] += dark * f64::from((-cs[c][j]).max(0.0));
                    }
                }
            }
        }
    });
    let shifts = if found >= MIN_DUST {
        corr.map(|cc| peak(&cc, side))
    } else {
        [[0.0; 2]; 3]
    };
    // How deep each colour's shadow is against the infrared's: the colour's
    // log contrast where the dust lies over the infrared's, on the dust that
    // still lets light through.
    let deepest = DEEP.ln();
    let mut fit = [[0f64; 2]; 3];
    let mut count = 0usize;
    if found >= MIN_DUST {
        bands(ctx, Some(&first), Some(crosstalk), |band, y0, y1| {
            let cs = band.colours.as_ref().expect("colours");
            let lines = (band.r.len() / w) as isize;
            for i in rows(band, y0, y1, w) {
                let at = band.a * w + i;
                if !mask[at] || dark[at] < deepest {
                    continue;
                }
                count += 1;
                let (y, x) = ((i / w) as isize, (i % w) as isize);
                for (c, f) in fit.iter_mut().enumerate() {
                    let yy = y + shifts[c][0].round() as isize;
                    let xx = x + shifts[c][1].round() as isize;
                    if yy < 0 || xx < 0 || yy >= lines || xx >= w as isize {
                        continue;
                    }
                    let r = f64::from(dark[at]);
                    f[0] += f64::from(cs[c][yy as usize * w + xx as usize]) * r;
                    f[1] += r * r;
                }
            }
        });
    }
    let depth = fit.map(|[cr, rr]| {
        if count >= MIN_DUST && rr > 0.0 {
            ((cr / rr) as f32).clamp(0.5, 2.0)
        } else {
            1.0
        }
    });
    let masks: [Option<Vec<bool>>; 3] = if found == 0 {
        [None, None, None]
    } else {
        shifts.map(|s| Some(moved(&mask, w, h, s)))
    };
    (
        Removal {
            noise,
            threshold,
            crosstalk,
            shifts,
            depth,
            film,
            found,
            divided: [0; 3],
            filled: [0; 3],
            left: 0,
        },
        masks,
        dark,
    )
}

/// The indices, in a band's arrays, of the image's lines `y0..y1`.
fn rows(band: &Band, y0: usize, y1: usize, w: usize) -> std::ops::Range<usize> {
    (y0 - band.a) * w..(y1 - band.a) * w
}

/// Calls `each` with every band of [`BAND`] lines (`y0..y1`), measured on
/// them and their margin; the infrared's mean leaves out `exclude`; with
/// `crosstalk` the colours' contrasts are measured too and that much of
/// them taken out of the infrared's.
fn bands(
    ctx: &Ctx,
    exclude: Option<&[bool]>,
    crosstalk: Option<[f32; 3]>,
    mut each: impl FnMut(&Band, usize, usize),
) {
    let mut y0 = 0;
    while y0 < ctx.h {
        let y1 = (y0 + BAND).min(ctx.h);
        let a = y0.saturating_sub(MARGIN);
        let b = (y1 + MARGIN).min(ctx.h);
        let band = measure(ctx, a, b, exclude, crosstalk);
        each(&band, y0, y1);
        y0 = y1;
    }
}

/// A band's film, the infrared's contrast and the colours'.
fn measure(
    ctx: &Ctx,
    a: usize,
    b: usize,
    exclude: Option<&[bool]>,
    crosstalk: Option<[f32; 3]>,
) -> Band {
    let w = ctx.w;
    let lines = b - a;
    let ir: Vec<f32> = ctx.ir[a * w..b * w].iter().map(|&v| f32::from(v)).collect();
    // The film: the infrared's local mean near the film's level, and the
    // window around a pixel all such.
    let mean = boxed(&ir, None, w, lines, REACH);
    let off: Vec<f32> = mean
        .iter()
        .map(|&m| if m >= FILM * ctx.level { 0.0 } else { 1.0 })
        .collect();
    let near = boxed(&off, None, w, lines, REACH);
    let film: Vec<bool> = off
        .iter()
        .zip(&near)
        .map(|(&o, &k)| o == 0.0 && k < 1e-6)
        .collect();
    let log: Vec<f32> = ir.iter().map(|&v| v.max(1.0).ln()).collect();
    let keep: Vec<bool> = match exclude {
        Some(x) => film
            .iter()
            .zip(&x[a * w..b * w])
            .map(|(&f, &d)| f && !d)
            .collect(),
        None => film.clone(),
    };
    let local = boxed(&log, Some(&keep), w, lines, REACH);
    let mut r: Vec<f32> = log
        .iter()
        .zip(&local)
        .map(|(&l, &m)| if m.is_finite() { l - m } else { 0.0 })
        .collect();
    let colours = crosstalk.map(|k| {
        let cs = [0, 1, 2].map(|c| {
            let log: Vec<f32> = (a * w..b * w)
                .map(|p| ctx.src.get(p, c).max(1.0).ln())
                .collect();
            let mean = boxed(&log, None, w, lines, REACH);
            log.iter()
                .zip(&mean)
                .map(|(&l, &m)| l - m)
                .collect::<Vec<f32>>()
        });
        if k != [0.0; 3] {
            for (i, v) in r.iter_mut().enumerate() {
                *v -= k[0] * cs[0][i] + k[1] * cs[1][i] + k[2] * cs[2][i];
            }
        }
        cs
    });
    Band {
        a,
        film,
        r,
        colours,
    }
}

/// The mean of `v` (`w` x `lines`) over the (2 `r` + 1)² window around each
/// pixel, the window cut at the edges and, with `keep`, only its kept pixels
/// (NaN where none is).
fn boxed(v: &[f32], keep: Option<&[bool]>, w: usize, lines: usize, r: usize) -> Vec<f32> {
    let kept = |i: usize| keep.is_none_or(|k| k[i]);
    let mut sums = vec![0f64; w * lines];
    let mut counts = vec![0f64; w * lines];
    for y in 0..lines {
        let row = y * w;
        let (mut s, mut c) = (0f64, 0f64);
        for x in 0..=r.min(w - 1) {
            if kept(row + x) {
                s += f64::from(v[row + x]);
                c += 1.0;
            }
        }
        for x in 0..w {
            sums[row + x] = s;
            counts[row + x] = c;
            if x + r + 1 < w && kept(row + x + r + 1) {
                s += f64::from(v[row + x + r + 1]);
                c += 1.0;
            }
            if x >= r && kept(row + x - r) {
                s -= f64::from(v[row + x - r]);
                c -= 1.0;
            }
        }
    }
    let mut out = vec![0f32; w * lines];
    let (mut s, mut c) = (vec![0f64; w], vec![0f64; w]);
    let add = |s: &mut [f64], c: &mut [f64], y: usize, sign: f64| {
        for x in 0..w {
            s[x] += sign * sums[y * w + x];
            c[x] += sign * counts[y * w + x];
        }
    };
    for y in 0..=r.min(lines - 1) {
        add(&mut s, &mut c, y, 1.0);
    }
    for y in 0..lines {
        for x in 0..w {
            out[y * w + x] = if c[x] > 0.5 {
                (s[x] / c[x]) as f32
            } else {
                f32::NAN
            };
        }
        if y + r + 1 < lines {
            add(&mut s, &mut c, y + r + 1, 1.0);
        }
        if y >= r {
            add(&mut s, &mut c, y - r, -1.0);
        }
    }
    out
}

/// The robust standard deviation (1.4826 x the median absolute deviation);
/// a small floor keeps a made-up noiseless infrared from finding everything.
fn spread(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 1.0;
    }
    let mid = v.len() / 2;
    let median = *v.select_nth_unstable_by(mid, f32::total_cmp).1;
    for x in v.iter_mut() {
        *x = (*x - median).abs();
    }
    let mad = *v.select_nth_unstable_by(mid, f32::total_cmp).1;
    (1.4826 * mad).max(1e-3)
}

/// The value below which a share `q` of `v` lies.
fn percentile(v: &[u16], q: f64) -> f32 {
    let mut counts = vec![0usize; 65536];
    for &x in v {
        counts[usize::from(x)] += 1;
    }
    let target = (v.len() as f64 * q) as usize;
    let mut seen = 0;
    for (x, &k) in counts.iter().enumerate() {
        seen += k;
        if seen > target {
            return x as f32;
        }
    }
    65535.0
}

/// Solves the 3 x 3 normal equations (Gauss with pivoting); `None` when they
/// say nothing (no clean film, or colours that do not vary).
fn solve(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    let scale = a[0][0].abs().max(a[1][1].abs()).max(a[2][2].abs());
    if scale <= 0.0 {
        return None;
    }
    for col in 0..3 {
        let p = (col..3).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[p][col].abs() < 1e-9 * scale {
            return None;
        }
        a.swap(col, p);
        b.swap(col, p);
        for row in col + 1..3 {
            let f = a[row][col] / a[col][col];
            let pivot = a[col];
            for (v, p) in a[row].iter_mut().zip(pivot).skip(col) {
                *v -= f * p;
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0f64; 3];
    for row in (0..3).rev() {
        let s: f64 = (row + 1..3).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// The correlation's peak (lines, columns from the middle): the centre of the
/// shifts within a tenth of its height (from its median) of its highest,
/// weighted by how far above that they are. A speck gives a peak, and two
/// whole places equally high when it lies between them; a long hair gives a
/// ridge along itself, whose centre this takes (its highest point is
/// anywhere along it).
fn peak(corr: &[f64], side: usize) -> [f32; 2] {
    let top = corr.iter().copied().fold(f64::MIN, f64::max);
    let mut sorted = corr.to_vec();
    sorted.sort_by(f64::total_cmp);
    let floor = top - 0.1 * (top - sorted[sorted.len() / 2]);
    let (mut sy, mut sx, mut sw) = (0f64, 0f64, 0f64);
    for (i, &v) in corr.iter().enumerate() {
        let weight = v - floor;
        if weight > 0.0 {
            sy += weight * (i / side) as f64;
            sx += weight * (i % side) as f64;
            sw += weight;
        }
    }
    if sw <= 0.0 {
        return [0.0; 2];
    }
    let half = SEARCH as f64;
    [(sy / sw - half) as f32, (sx / sw - half) as f32]
}

/// `mask` moved by `shift` (lines, columns; a fraction takes both whole
/// places either side) and grown by a pixel all round.
fn moved(mask: &[bool], w: usize, h: usize, shift: [f32; 2]) -> Vec<bool> {
    let places = |s: f32| {
        let (f, c) = (s.floor() as isize, s.ceil() as isize);
        if f == c {
            vec![f]
        } else {
            vec![f, c]
        }
    };
    let (ys, xs) = (places(shift[0]), places(shift[1]));
    let mut out = vec![false; w * h];
    for (i, _) in mask.iter().enumerate().filter(|(_, &m)| m) {
        let (y, x) = ((i / w) as isize, (i % w) as isize);
        for sy in &ys {
            for sx in &xs {
                for gy in -(GROW as isize)..=GROW as isize {
                    for gx in -(GROW as isize)..=GROW as isize {
                        let (yy, xx) = (y + sy + gy, x + sx + gx);
                        if yy >= 0 && xx >= 0 && (yy as usize) < h && (xx as usize) < w {
                            out[yy as usize * w + xx as usize] = true;
                        }
                    }
                }
            }
        }
    }
    out
}

/// Fills colour `c` of `data` (three samples a pixel) under `mask`: along
/// each of four lines through the pixel (across, along, both diagonals), the
/// nearest pixel outside the mask on either side, interpolated by distance;
/// the lines weighted by how well their two ends agree and how short they
/// are, so that a hair is bridged across and a speck by an edge of the
/// picture is filled along the edge, not from both sides of it. Returns the
/// pixels filled and those left (nothing outside within [`FILL_REACH`]).
fn fill<T: Copy>(
    data: &mut [T],
    w: usize,
    h: usize,
    c: usize,
    mask: &[bool],
    get: impl Fn(T) -> f32,
    put: impl Fn(f32) -> T,
) -> (usize, usize) {
    const AXES: [(isize, isize); 4] = [(0, 1), (1, 0), (1, 1), (1, -1)];
    let (mut filled, mut left) = (0, 0);
    // The nearest value outside the mask from `i` in direction (dy, dx), and how far.
    let end = |data: &[T], y: isize, x: isize, dy: isize, dx: isize| -> Option<(f32, f32)> {
        let step = if dy != 0 && dx != 0 {
            std::f32::consts::SQRT_2
        } else {
            1.0
        };
        for k in 1..=FILL_REACH as isize {
            let (yy, xx) = (y + k * dy, x + k * dx);
            if yy < 0 || xx < 0 || yy as usize >= h || xx as usize >= w {
                return None;
            }
            let j = yy as usize * w + xx as usize;
            if !mask[j] {
                return Some((get(data[j * 3 + c]), k as f32 * step));
            }
        }
        None
    };
    for i in 0..w * h {
        if !mask[i] {
            continue;
        }
        let (y, x) = ((i / w) as isize, (i % w) as isize);
        let (mut sum, mut weights) = (0f32, 0f32);
        for (dy, dx) in AXES {
            let (value, spread, length) = match (end(data, y, x, dy, dx), end(data, y, x, -dy, -dx))
            {
                (Some((a, da)), Some((b, db))) => {
                    ((a * db + b * da) / (da + db), (a - b).abs(), da + db)
                }
                // One side only (an edge of the image): trusted less.
                (Some((a, da)), None) | (None, Some((a, da))) => (a, 0.1 * a, 2.0 * da),
                (None, None) => continue,
            };
            // A level, or a hundredth of the value: what two ends that agree still differ by.
            let agree = spread + 1.0 + 0.01 * value.abs();
            let weight = 1.0 / (agree * agree * length);
            sum += weight * value;
            weights += weight;
        }
        if weights > 0.0 {
            data[i * 3 + c] = put(sum / weights);
            filled += 1;
        } else {
            left += 1;
        }
    }
    (filled, left)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up generator of grain (no randomness in the tests: the same
    /// numbers every time).
    struct Grain(u64);

    impl Grain {
        /// Roughly normal, mean 0, sd 1 (the sum of four uniforms).
        fn next(&mut self) -> f32 {
            let mut s = 0.0;
            for _ in 0..4 {
                self.0 = self
                    .0
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                s += (self.0 >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
            }
            s * 1.732
        }
    }

    /// A film `w` x `h`: a picture (`scene(y, x)`, light 0-1 per colour),
    /// grain, and dust that lets through `dust(y, x)` (1 = none), in the
    /// colours moved by `shifts` (lines, columns) from the infrared's; the
    /// infrared carries `crosstalk` of the red's log, and its own grain.
    struct Film {
        w: usize,
        h: usize,
        image: Image,
        samples: Vec<u16>,
        infrared: Vec<u16>,
        /// The colours as they were without the dust.
        clean: Vec<u16>,
    }

    fn film(
        w: usize,
        h: usize,
        scene: impl Fn(f32, f32) -> [f32; 3],
        dust: impl Fn(f32, f32) -> f32,
        shifts: [[f32; 2]; 3],
        crosstalk: f32,
    ) -> Film {
        let mut g = Grain(7);
        let mut samples = Vec::with_capacity(w * h * 3);
        let mut clean = Vec::with_capacity(w * h * 3);
        let mut infrared = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let (fy, fx) = (y as f32, x as f32);
                let s = scene(fy, fx);
                for c in 0..3 {
                    let v = s[c] * (1.0 + 0.01 * g.next());
                    let d = dust(fy - shifts[c][0], fx - shifts[c][1]);
                    clean.push((v * 50_000.0).round() as u16);
                    samples.push((v * d * 50_000.0).round() as u16);
                }
                let ir = 0.85 * s[0].powf(crosstalk) * dust(fy, fx) * (1.0 + 0.006 * g.next());
                infrared.push((ir * 65_535.0).round().min(65_535.0) as u16);
            }
        }
        let data = samples.iter().map(|&v| (v >> 8) as u8).collect();
        Film {
            w,
            h,
            image: Image {
                width: w,
                height: h,
                dpi: 1200,
                format: Format::Rgb,
                data,
            },
            samples,
            infrared,
            clean,
        }
    }

    /// A speck (radius 3) and a hair (2 pixels wide, across).
    fn specks(y: f32, x: f32) -> f32 {
        let speck = (y - 60.0).hypot(x - 70.0) <= 3.0;
        let hair = (y - (100.0 + 0.3 * x)).abs() <= 1.0 && (20.0..180.0).contains(&x);
        if speck || hair {
            0.25
        } else {
            1.0
        }
    }

    fn grey(_: f32, _: f32) -> [f32; 3] {
        [0.6, 0.5, 0.4]
    }

    /// The largest difference from the clean colours over the pixels where
    /// `dust` (in that colour's place) is.
    fn worst(f: &Film, samples: &[u16], shifts: [[f32; 2]; 3]) -> f32 {
        let mut worst = 0f32;
        for y in 0..f.h {
            for x in 0..f.w {
                for (c, [sy, sx]) in shifts.iter().enumerate() {
                    if specks(y as f32 - sy, x as f32 - sx) < 1.0 {
                        let i = (y * f.w + x) * 3 + c;
                        let d = (f32::from(samples[i]) - f32::from(f.clean[i])).abs()
                            / f32::from(f.clean[i]);
                        worst = worst.max(d);
                    }
                }
            }
        }
        worst
    }

    #[test]
    fn a_speck_and_a_hair_are_found_and_filled() {
        let shifts = [[1.2, 0.0], [0.5, 0.0], [0.0, 0.0]];
        let mut f = film(220, 200, grey, specks, shifts, 0.0);
        assert!(worst(&f, &f.samples, shifts) > 0.7);
        let mut samples = f.samples.clone();
        let r = remove(&mut f.image, Some(&mut samples), &f.infrared).unwrap();
        // The dust is found where it is, and nothing else.
        let dusty = (0..200 * 220)
            .filter(|&i| specks((i / 220) as f32, (i % 220) as f32) < 1.0)
            .count();
        assert!(
            r.found >= dusty * 9 / 10 && r.found <= dusty * 11 / 10,
            "{r:?} {dusty}"
        );
        assert!((r.noise - 0.006).abs() < 0.002, "{r:?}");
        // Red's shadow found more than a line further along, blue's on the infrared's.
        assert!((r.shifts[0][0] - 1.2).abs() < 0.4, "{r:?}");
        assert!(r.shifts[2][0].abs() < 0.4, "{r:?}");
        assert_eq!(r.left, 0);
        // Filled within a few % of the clean film (its grain 1 %).
        let left = worst(&f, &samples, shifts);
        assert!(left < 0.05, "{left}");
        // The 8 bits too.
        for (i, &v) in f.image.data.iter().enumerate() {
            assert!(
                (i32::from(v) - i32::from(samples[i] >> 8)).abs() <= 2,
                "{i}"
            );
        }
        // Away from the dust, nothing moved.
        for y in [10, 150, 190] {
            for x in [10, 120, 210] {
                let i = (y * 220 + x) * 3;
                assert_eq!(samples[i..i + 3], f.samples[i..i + 3], "{y} {x}");
            }
        }
    }

    #[test]
    fn the_infrared_s_copy_of_the_picture_is_taken_out() {
        // A picture of sharp bars (the red from 0.2 to 0.8), the infrared
        // carrying a tenth of the red's log: its bars' edges are 5 % or more
        // below their local mean, many times its noise.
        let bars = |_: f32, x: f32| {
            let r = if (x as usize / 9) % 2 == 0 { 0.8 } else { 0.2 };
            [r, 0.5, 0.4]
        };
        let none = |_: f32, _: f32| 1.0;
        let mut f = film(200, 150, bars, none, [[0.0; 2]; 3], 0.1);
        let mut samples = f.samples.clone();
        let r = remove(&mut f.image, Some(&mut samples), &f.infrared).unwrap();
        assert!((r.crosstalk[0] - 0.1).abs() < 0.02, "{r:?}");
        assert!(r.found < 20, "{r:?}");
        // Without the correction the bars would have been dust.
        let plain = r.noise * DARK;
        assert!(0.1 * (0.8f32 / 0.2).ln() / 2.0 > plain, "{r:?}");
    }

    #[test]
    fn a_holder_dark_in_infrared_is_no_dust() {
        // The top 40 lines are the holder: dark in every light.
        let holder = |y: f32, _: f32| if y < 40.0 { 0.02 } else { 1.0 };
        let mut f = film(120, 140, grey, holder, [[0.0; 2]; 3], 0.0);
        let mut samples = f.samples.clone();
        let r = remove(&mut f.image, Some(&mut samples), &f.infrared).unwrap();
        assert_eq!(r.found, 0, "{r:?}");
        assert_eq!(samples, f.samples);
        assert!(r.film < 120 * 100, "{r:?}");
    }

    #[test]
    fn eight_bits_alone() {
        let mut f = film(220, 200, grey, specks, [[0.0; 2]; 3], 0.0);
        let before = f.image.data.clone();
        let r = remove(&mut f.image, None, &f.infrared).unwrap();
        assert!(r.found > 100, "{r:?}");
        // The speck's middle is back to the film's grey.
        let i = (60 * 220 + 70) * 3;
        assert!(before[i] < 50);
        assert!((i32::from(f.image.data[i]) - (0.6 * 50_000.0 / 256.0) as i32).abs() < 6);
    }

    /// The removal on real scans: the frames a driver assembled and the infrared it laid on them
    /// (`ICE_REG/<k>_frame.bin`, three u16 a pixel, `<k>_laid.bin`, one, `<k>_size.txt` the
    /// width and height; written by the Epson 4490 driver's `registration_on_the_captures`); it
    /// writes the frame cleaned (`<k>_clean.bin`) for `captures/epson4490/tools/ice_dust.py
    /// --ours`, which sets it beside the scanner maker's own removal. Run with `ICE_REG=… cargo
    /// test --release -p scan-imaging --lib dust_on_the_captures -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the captures' frames, outside the repository"]
    fn dust_on_the_captures() {
        let dir = std::env::var("ICE_REG").expect("ICE_REG");
        let u16s = |b: Vec<u8>| -> Vec<u16> {
            b.chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect()
        };
        for k in ["18", "108", "109", "110", "112", "113", "126", "127", "130"] {
            let Ok(size) = std::fs::read_to_string(format!("{dir}/{k}_size.txt")) else {
                println!("{k}: no frame, skipped");
                continue;
            };
            let wh: Vec<usize> = size
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            let mut samples = u16s(std::fs::read(format!("{dir}/{k}_frame.bin")).unwrap());
            let ir = u16s(std::fs::read(format!("{dir}/{k}_laid.bin")).unwrap());
            let mut image = Image {
                width: wh[0],
                height: wh[1],
                dpi: 2400,
                format: Format::Rgb,
                data: samples.iter().map(|&v| (v >> 8) as u8).collect(),
            };
            let t = std::time::Instant::now();
            let r = remove(&mut image, Some(&mut samples), &ir).unwrap();
            println!("{k}: {:.2} s, {r:?}", t.elapsed().as_secs_f32());
            std::fs::write(
                format!("{dir}/{k}_clean.bin"),
                samples
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<u8>>(),
            )
            .unwrap();
        }
    }

    #[test]
    fn nothing_for_what_does_not_fit() {
        let mut f = film(40, 40, grey, specks, [[0.0; 2]; 3], 0.0);
        let ir = f.infrared.clone();
        assert!(remove(&mut f.image, None, &ir[1..]).is_none());
        let mut short = f.samples[3..].to_vec();
        assert!(remove(&mut f.image, Some(&mut short), &ir).is_none());
        let mut small = film(20, 40, grey, specks, [[0.0; 2]; 3], 0.0);
        let ir = small.infrared.clone();
        assert!(remove(&mut small.image, None, &ir).is_none());
    }
}
