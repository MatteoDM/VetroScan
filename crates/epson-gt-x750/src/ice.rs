// SPDX-License-Identifier: GPL-2.0-or-later
//! Digital ICE: a film read twice, in colour and in infrared, the infrared laid on the colours.
//!
//! Read from Epson Scan's captures 18 and 108-113 (`captures/epson4490/05-catture.md`,
//! `06-formato-dati.md`, "Digital ICE", outside the repository) and from Epson's interpreter
//! (`esxuin54.dll` 1.27, under the EU interoperability exception); every rule here is
//! `captures/epson4490/tools/icesetup.py`'s, which reproduces every captured value with no
//! difference on the six ICE scans (`--check`). An ICE scan is two whole film scans
//! ([`crate::scan::scan_ice`]):
//!
//! * the colour pass, a film scan with `1E 57` source `04` and gamma `03`, which starts with a
//!   **hole pre-read** ([`pre_e41`]): two round holes of the film holder, either side of the
//!   window, read under the lid unit; their places ([`holes`]) go to the scanner with `1E 88`;
//! * the infrared pass (`1E 57` mode `20`, source `03`, the lid unit's infrared light `1E A2 04`),
//!   which reads the holes again and starts its read and chooses its columns from both
//!   ([`infrared_plan`]): the infrared image is 0.73 % larger and a few lines off.
//!
//! The registration ([`register`]) is the driver's (Matteo's decision, 17 November 2026): the
//! infrared's green CCD row, scaled by the lens's fixed ratio ([`colour_per_infrared`]) and
//! shifted by what a correlation of the two images measures (`ice.py --register`'s).

use crate::film::{self, FilmPlan, FilmRequest, Geometry, Pass};
use crate::scan::Image;
use crate::setup::{self, XCorrection};
use scanner_api::{Error, Result};

/// The hole pre-read's first column with no x correction ([`pre_c0`]).
pub const PRE_C0: u32 = 6398;
/// The hole pre-read's pixels a line (one a column) and lines, 8 bits.
pub const PRE_READ: (u32, u32) = (7040, 480);
/// The pre-read's line time, µs (`1E 21` `002a…`, byte 12 `2a`).
pub const PRE_T: u32 = 10_752;
/// The carriage's move before the pre-read (with the 256-value ramp, flags `10`: 131 + 512).
pub const PRE_MOVE: u32 = 131;
/// The pre-read's own move: 512 steps, lead 32.
pub const PRE_READ_STEPS: u32 = 512;
/// Where the pre-read leaves the carriage (`1E 86`): 131 + 512 + 512.
pub const PRE_AT: u32 = 1155;
/// The park after the pre-read: 327 + 512 = [`film::PARK`] - [`PRE_AT`].
pub const PRE_PARK: u32 = 327;
/// The steps a move with the 256-value ramp spends on its ramps.
pub const RAMP_256_STEPS: u32 = 512;
/// The pre-read's gains (`1E 31` bytes 8-10): `32` in the colour pass, `3f` in the infrared.
pub const PRE_GAINS: (u8, u8) = (0x32, 0x3F);
/// The lid unit's move at the infrared pass's start (`1E 25 02`, the film ramp): 705 + 1024.
pub const IR_UNIT_MOVE: u32 = 705;
/// Where that move leaves the lid unit.
pub const IR_UNIT_AT: u32 = IR_UNIT_MOVE + film::RAMP_512_STEPS;
/// The lid unit's infrared light (`1E A2 04`), and its dark read's (`05`; `02`/`03` for colour).
pub const LAMP_INFRARED: u8 = 0x04;
/// The infrared pass's dark read's light.
pub const LAMP_INFRARED_DARK: u8 = 0x05;
/// The infrared pass's lid-unit origin read: line time (6144 on films) and its `1E 31` gains.
pub const IR_ORIGIN_T: u32 = 8064;
/// The infrared origin read's `1E 31`: offsets `80`, gains `32` (`1e` on films).
pub const IR_ORIGIN_E31: [u8; 12] = [0x80, 0, 0x80, 0, 0x80, 0, 0, 0, 0x32, 0x32, 0x32, 0];
/// The infrared pass's gains (`1E 31` bytes 8-10), no search: every capture.
pub const IR_GAINS: [u8; 3] = [0x19, 0x24, 0x22];
/// The longest line time an infrared window read was captured with (110, 112, 113; the films'
/// own limit is `crate::guard::FILM_LINE_TIMES`).
pub const IR_MAX_LINE_TIME: u32 = 28_800;
/// The infrared start: the colour pass's start move + this - floor(D / 4) ([`start_delta`]).
pub const START_K: i32 = 48;
/// The holes' images: each [`HOLE_W`] columns of the pre-read's green samples, the right one
/// from this column.
pub const RIGHT_HOLE: u32 = 6235;
/// The holes' images' width.
pub const HOLE_W: usize = 692;
/// How much larger the infrared image is than the colour one, as Epson Scan's choice of the
/// infrared read's start and columns takes it: 1 / 0.99279 (the image's scale measured on
/// captures 18, 108, 110 with `ice.py --register`; any ratio from 1.00699 to 1.00731 gives every
/// capture's columns).
pub const IR_SCALE: (u64, u64) = (100_000, 99_279);

/// A correction halved toward zero, as the interpreter halves `1E 87`'s (`0x421e24`).
fn half(corr: XCorrection) -> i32 {
    i32::from(corr.0) / 2
}

/// The hole pre-read's first column: [`PRE_C0`] + `corr` / 2 toward zero (18: -93 gives 6352,
/// 108-113: -133 gives 6332).
pub fn pre_c0(corr: XCorrection) -> u32 {
    (PRE_C0 as i32 + half(corr)) as u32
}

/// The hole pre-read's `1E 41`: 7040 columns from [`pre_c0`], 480 lines, 8 bits, ratio 1/1.
pub fn pre_e41(corr: XCorrection) -> [u8; 22] {
    pre_e41_at(pre_c0(corr))
}

/// The hole pre-read's `1E 41` from its first column `c0`.
pub fn pre_e41_at(c0: u32) -> [u8; 22] {
    let (px, lines) = PRE_READ;
    setup::block41(
        c0,
        c0 + px,
        lines,
        0x01,
        1,
        1,
        0,
        0,
        0x80,
        0x80,
        lines as u16,
    )
}

/// The hole pre-read's `1E 21`: T [`PRE_T`], byte 12 `2a`, nothing else.
pub fn pre_e21() -> [u8; 26] {
    let mut b = [0u8; 26];
    b[..3].copy_from_slice(&PRE_T.to_le_bytes()[..3]);
    b[12] = 0x2A;
    b
}

/// The hole pre-read's `1E 31`: offsets `80`, the pass's gains ([`PRE_GAINS`]).
pub fn pre_e31(pass: Pass) -> [u8; 12] {
    let g = if pass == Pass::Infrared {
        PRE_GAINS.1
    } else {
        PRE_GAINS.0
    };
    [0x80, 0, 0x80, 0, 0x80, 0, 0, 0, g, g, g, 0]
}

/// The two holes of the pre-read, in 4800ths of an inch (half pixels): for the left one, then
/// the right (its columns from [`RIGHT_HOLE`]): the column, the line, the width, the height.
/// `found` when both were.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Holes {
    /// Both holes found.
    pub found: bool,
    /// Left x, y, width, height; right x, y, width, height.
    pub values: [u16; 8],
}

impl Holes {
    /// `1E 88`'s block: `01` when both were found (else `00`), `00`, the eight values.
    pub fn block88(&self) -> [u8; 18] {
        let mut b = [0u8; 18];
        b[0] = u8::from(self.found);
        for (i, v) in self.values.iter().enumerate() {
            b[2 + 2 * i..4 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// The holes `1E 87 01` gives back (`1E 88`'s block).
    pub fn from_block88(b: &[u8]) -> Option<Self> {
        if b.len() != 18 || b[0] > 1 || b[1] != 0 {
            return None;
        }
        Some(Self {
            found: b[0] == 1,
            values: std::array::from_fn(|i| u16::from_le_bytes([b[2 + 2 * i], b[3 + 2 * i]])),
        })
    }
}

/// The threshold of the holes (`0x421d30`): a histogram of every second sample of the left
/// hole's image in 26 bins of 10; the background's bin is the last with the most counts, the
/// hole's the last with the most from three bins above it (the background's when it is 22 or
/// more); the threshold 5 x (the two bins' sum), half-way between their starts.
fn threshold(left: &[u8]) -> u8 {
    let mut hist = [0u32; 26];
    for &v in left.iter().step_by(2) {
        if let Some(h) = hist.get_mut(usize::from(v / 10)) {
            *h += 1;
        }
    }
    let top = hist.iter().copied().max().unwrap_or(0);
    let bg = hist.iter().rposition(|&h| h == top).unwrap_or(0);
    let mut hole = bg;
    if bg < 22 {
        let mut best = 0;
        for (i, &h) in hist.iter().enumerate().skip(bg + 3) {
            if h >= best {
                best = h;
                hole = i;
            }
        }
    }
    (5 * (bg + hole)) as u8
}

/// One hole in its image (`0x4218d0`): in each line the samples above `thr` are counted (their
/// first and last column kept) from the line's start or the last reset; at each sample not above
/// it a count of 28-122 makes the line's centre (first + last) / 2 and the widest (last - first)
/// so far, any other count resets. Over the lines: a line with a centre is kept (the first and
/// the last such line; the columns' average (average + centre) / 2, the first alone); a line
/// without one, after 28-122 lines with, makes the hole (the line (first + last) / 2, the height
/// last - first); any other resets. (found, column, line, width, height) in pixels.
fn hole(img: &[u8], lines: usize, thr: u8) -> (bool, u32, u32, u32, u32) {
    let (mut widest, mut first, mut last, mut n, mut avg) = (0u32, None::<u32>, 0u32, 0u32, 0u32);
    let (mut found, mut cy, mut h) = (false, 0u32, 0u32);
    for (y, row) in img.chunks_exact(HOLE_W).take(lines).enumerate() {
        let y = y as u32;
        let (mut lo, mut hi, mut cnt, mut c) = (u32::MAX, 0u32, 0u32, 0u32);
        for (x, &v) in row.iter().enumerate() {
            let x = x as u32;
            if v > thr {
                lo = lo.min(x);
                hi = hi.max(x);
                cnt += 1;
            } else if (28..=122).contains(&cnt) {
                widest = widest.max(hi - lo);
                c = (lo + hi) >> 1;
            } else {
                (lo, hi, cnt, c) = (u32::MAX, 0, 0, 0);
            }
        }
        if c != 0 {
            first = Some(first.map_or(y, |f| f.min(y)));
            last = last.max(y);
            avg = if n > 0 { (avg + c) >> 1 } else { c };
            n += 1;
        } else if (28..=122).contains(&n) {
            let f = first.unwrap_or(0);
            (cy, h, found) = ((f + last) >> 1, last - f, true);
        } else {
            (found, last, cy, n, first) = (false, 0, 0, 0, None);
        }
    }
    (found, avg, cy, widest, h)
}

/// The holes of a hole pre-read ([`PRE_READ`]: 480 lines of 7040 pixels, three 8-bit samples a
/// pixel), as Epson's interpreter finds them on its green samples (captures 18, 108-113: every
/// `1E 88`). `None` for a read too short.
pub fn holes(read: &[u8]) -> Option<Holes> {
    let (px, lines) = (PRE_READ.0 as usize, PRE_READ.1 as usize);
    if read.len() < px * lines * 3 {
        return None;
    }
    let green = |x0: usize| -> Vec<u8> {
        read.chunks_exact(px * 3)
            .take(lines)
            .flat_map(|l| (x0..x0 + HOLE_W).map(move |x| l[3 * x + 1]))
            .collect()
    };
    let (left, right) = (green(0), green(RIGHT_HOLE as usize));
    let t = threshold(&left);
    let (fl, lx, ly, lw, lh) = hole(&left, lines, t);
    let (fr, rx, ry, rw, rh) = hole(&right, lines, t);
    // x 4800 / 2400 (the pre-read's 1E 57 asks 2400 dpi both ways).
    let v = [lx, ly, lw, lh, rx, ry, rw, rh].map(|v| (2 * v) as u16);
    Some(Holes {
        found: fl && fr,
        values: v,
    })
}

/// The infrared start move less the colour one: [`START_K`] - floor(D / 4), D the colour holes'
/// lines less the infrared holes', left plus right, in 4800ths (39-44 in captures 18 and
/// 108-126, 45 right after a power-on in 127).
pub fn start_delta(colour: &Holes, infrared: &Holes) -> i32 {
    let d = (i32::from(colour.values[1]) - i32::from(infrared.values[1]))
        + (i32::from(colour.values[5]) - i32::from(infrared.values[5]));
    START_K - d.div_euclid(4)
}

/// The `1E 57` of a film request for a pass: a film's ([`film::request_block`]); the colour
/// pass's with source `04` and gamma `03`; the infrared pass's mode `20` (one 16-bit sample a
/// pixel asked), source `03`, its lines per block (32 for every capture's width), gamma `03`, no
/// film type.
pub fn request_block(r: &FilmRequest, pass: Pass) -> Result<[u8; 42]> {
    let mut e57 = film::request_block(r)?;
    match pass {
        Pass::Film => {}
        Pass::IceColour => {
            e57[26] = 0x04;
            e57[29] = 0x03;
        }
        Pass::Infrared => {
            e57[24] = 0x20;
            e57[25] = 16;
            e57[26] = 0x03;
            e57[28] = film::lines_per_request_block(r.width * 2);
            e57[29] = 0x03;
            e57[37] = 0;
        }
    }
    Ok(e57)
}

/// Rounds `num / den` half away from zero.
/// The infrared pass's plan from the colour pass's (`colour`), `1E 87`'s correction and the two
/// passes' holes (Epson Scan's choice, `icesetup.py`): its read starts at the colour read's
/// logical column L moved by the left hole's shift and by the scale (L + (iL - cL) +
/// round((L - P - cL) x (1 / 0.99279 - 1)) in 4800ths, P the pre-read's first column x 2); its
/// pixels the colour read's x 1 / 0.99279 rounded up to 128; its lines the image's and the
/// staggered columns' (no colour offsets: h + 4 at 2400 dpi, h at 1200); its start move the
/// colour one's + [`start_delta`].
pub fn infrared_plan(
    colour: &FilmPlan,
    corr: XCorrection,
    colour_holes: &Holes,
    infrared_holes: &Holes,
) -> Result<FilmPlan> {
    let bad = |why: String| Err(Error::Other(format!("Epson 4490, Digital ICE: {why}")));
    if !colour_holes.found || !infrared_holes.found {
        return bad(format!(
            "the film holder's holes were not found (colour {:?}, infrared {:?})",
            colour_holes.values, infrared_holes.values
        ));
    }
    let r = colour.request;
    let g = colour.geometry;
    let u = 4800 / g.base;
    let logical = film::logical_of(g.columns.0, g.words, g.base);
    let p = 2 * pre_c0(corr) as i64;
    let (cl, il) = (
        i64::from(colour_holes.values[0]),
        i64::from(infrared_holes.values[0]),
    );
    let x = i64::from(logical * u) - p - cl;
    let (sn, sd) = (IR_SCALE.0 as i64, IR_SCALE.1 as i64);
    // Truncated toward zero, as integers divide (capture 126, the one frame on the right
    // strip: 53.55 -> 53; every other capture's fraction is below a half).
    let shift = x * (sn - sd) / sd;
    let at = i64::from(logical * u) + (il - cl) + shift;
    let Ok(at) = u32::try_from(at) else {
        return bad(format!(
            "an infrared start before the scanner's first column ({at})"
        ));
    };
    let ir_logical = at / u;
    let pixels = (u64::from(g.pixels) * IR_SCALE.0).div_ceil(IR_SCALE.1 * 128) as u32 * 128;
    let count = pixels * g.base / g.read_dpi;
    let (c0, words) = film::left_edge(ir_logical, count, g.base);
    let (den, _) = g.ratio;
    let stagger = if g.read_dpi == 4800 { r.ydpi / 600 } else { 0 };
    let lines = (r.height + stagger).div_ceil(u32::from(den)) * u32::from(den);
    // Its first line `start_delta` steps after the colour read's (start move and lead: capture 127,
    // the colour read's lead 128 and the infrared's 32); its lead set again from its own line
    // time after its calibration (`film::lead_for`).
    let lead = film::planned_lead(r.dpi);
    let start = i64::from(g.start_steps)
        + i64::from(g.lead)
        + i64::from(start_delta(colour_holes, infrared_holes))
        - i64::from(lead);
    let Ok(start_steps) = u32::try_from(start) else {
        return bad(format!("an infrared start move of {start} steps"));
    };
    let ir = Geometry {
        pixels,
        columns: (c0, c0 + count),
        words,
        lines,
        lines_per_block: (film::BLOCK_MAX / (pixels * 6)) as u16,
        read_steps: lines * 2400 / r.ydpi + lead,
        start_steps,
        lead,
        ..g
    };
    Ok(FilmPlan::from_geometry(r, ir, Pass::Infrared))
}

/// The infrared exposures' last `1E 21` (`0x424320`): every channel integrates the whole line T,
/// the green exposure `x_green` rounded up to 384 µs, byte 12 `2a`; the words `words`, then
/// (`end` + 508) / 4 and (T - 259) / 2, `end` the infrared image read's last logical column (base
/// units), both 0 unless 2T - 254 > `end` + 10; `last` the block's last byte.
pub fn final_block(x_green: u32, end: u32, words: (u16, u16), last: u8) -> [u8; 26] {
    let t = crate::film_cal::round_up_384(x_green);
    let (w3, w4) = if 2 * t > end + 264 {
        ((end + 508) >> 2, (t - 259) >> 1)
    } else {
        (0, 0)
    };
    let mut b = [0u8; 26];
    b[..3].copy_from_slice(&t.to_le_bytes()[..3]);
    b[12] = 0x2A;
    for (i, w) in [words.0, words.1, w3 as u16, w4 as u16].iter().enumerate() {
        b[13 + 2 * i..15 + 2 * i].copy_from_slice(&w.to_le_bytes());
    }
    b[25] = last;
    b
}

/// The infrared image read's last logical column, base units ([`final_block`]'s `end`).
pub fn image_end(plan: &FilmPlan) -> u32 {
    let g = &plan.geometry;
    film::logical_of(g.columns.0, g.words, g.base) + g.column_count()
}

/// The colour image's columns per infrared column across the frame whose middle is `middle`
/// (4800ths of an inch, the film window's logical columns): the lens magnifies the infrared a
/// little more, and a little less further right. Measured with `ice.py --register` (17 November
/// 2026): 0.99279 around 16,150 (negatives at 2400 dpi, captures 18, 108, 110; 0.99282 at 1200,
/// 109), 0.99250 around 19,850 (a slide, 112, 113): one straight line through both (the curve of
/// one lens, as far as two places can tell; a frame's own quadratic fit is too noisy to say
/// more). Between them and a little beyond, within 0.3 pixel at 2400 dpi of either fit.
pub fn colour_per_infrared(middle: f64) -> f64 {
    0.99279 - (middle - 16_150.0) * (0.99279 - 0.99250) / (19_850.0 - 16_150.0)
}

/// Where an infrared read lies on its colour read, as [`register`] found it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Registration {
    /// Colour columns per infrared column ([`colour_per_infrared`]).
    pub scale: f64,
    /// The infrared read's line (its green CCD row, the late columns put back) of the frame's
    /// first row.
    pub line: f64,
    /// The infrared read's pixel of the colour read's first pixel.
    pub column: f64,
    /// How many tiles the shift was measured on.
    pub tiles: f64,
    /// The shift was measured.
    pub measured: bool,
}

/// The infrared's green CCD row as lines of the read's pixels, the late (odd sensor) columns
/// moved back by their lag, as f32.
fn green_plane(raw: &[u8], plan: &FilmPlan) -> (Vec<f32>, usize, usize) {
    let g = &plan.geometry;
    let (px, lines) = (g.pixels as usize, g.lines as usize);
    let stagger = crate::scan::film_stagger(plan) as usize;
    let c0 = g.columns.0 as usize;
    let line = px * 6;
    let mut out = vec![0f32; px * lines];
    for l in 0..lines {
        for j in 0..px {
            let late = if (c0 + j) % 2 == 1 { stagger } else { 0 };
            let at = (l + late).min(lines - 1) * line + j * 6 + 2;
            out[l * px + j] = f32::from(u16::from_le_bytes([raw[at], raw[at + 1]]));
        }
    }
    (out, px, lines)
}

/// Bilinear sample of a plane, the edges repeated.
fn sample(p: &[f32], w: usize, h: usize, y: f64, x: f64) -> f32 {
    let y = y.clamp(0.0, (h - 1) as f64);
    let x = x.clamp(0.0, (w - 1) as f64);
    let (y0, x0) = (y.floor() as usize, x.floor() as usize);
    let (y1, x1) = ((y0 + 1).min(h - 1), (x0 + 1).min(w - 1));
    let (fy, fx) = ((y - y0 as f64) as f32, (x - x0 as f64) as f32);
    let a = p[y0 * w + x0] * (1.0 - fx) + p[y0 * w + x1] * fx;
    let b = p[y1 * w + x0] * (1.0 - fx) + p[y1 * w + x1] * fx;
    a * (1.0 - fy) + b * fy
}

/// The infrared laid on a frame `w` x `h` with `reg`: frame pixel x is the colour read's pixel x
/// (2x + 1, its neighbours weighted 1/4 when the read was twice as fine across, as
/// `crate::scan::assemble_film` puts the colours together).
fn lay(
    plane: &[f32],
    pw: usize,
    ph: usize,
    w: usize,
    h: usize,
    halved: bool,
    reg: &Registration,
) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        let l = reg.line + y as f64;
        for x in 0..w {
            let at = |i: f64| sample(plane, pw, ph, l, reg.column + i / reg.scale);
            out[y * w + x] = if halved {
                let i = 2.0 * x as f64 + 1.0;
                (at(i - 1.0) + 2.0 * at(i) + at(i + 1.0)) / 4.0
            } else {
                at(x as f64)
            };
        }
    }
    out
}

/// Each sample over its `k` x `k` neighbourhood's mean, less 1 (`ice.py`'s `rel`): dust and
/// edges stand out, the light's slow changes do not.
fn relative(p: &[f32], w: usize, h: usize, k: usize) -> Vec<f32> {
    let r = k / 2;
    // An integral image with the edges repeated.
    let (pw, ph) = (w + 2 * r + 1, h + 2 * r + 1);
    let mut s = vec![0f64; pw * ph];
    for y in 1..ph {
        let sy = (y - 1).saturating_sub(r).min(h - 1);
        let mut row = 0f64;
        for x in 1..pw {
            let sx = (x - 1).saturating_sub(r).min(w - 1);
            row += f64::from(p[sy * w + sx]);
            s[y * pw + x] = s[(y - 1) * pw + x] + row;
        }
    }
    let n = (k * k) as f64;
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let sum =
                s[(y + k) * pw + x + k] - s[y * pw + x + k] - s[(y + k) * pw + x] + s[y * pw + x];
            let m = sum / n;
            out[y * w + x] = if m > 0.0 {
                (f64::from(p[y * w + x]) / m - 1.0) as f32
            } else {
                0.0
            };
        }
    }
    out
}

/// The tiles [`shift`] measures on, frame pixels across and along (`ice.py --register`'s: 256
/// columns of a read at 4800 across, 400 lines).
const TILE: (usize, usize) = (128, 200);

/// The fewest tiles with a clear peak for a shift to count as measured.
const MIN_TILES: usize = 4;

/// The shift of `b` on `a` (both `w` x `h`), as `ice.py --register` measures it: on each tile of
/// a grid over the frame the (dy, dx) within `r` that makes a(y, x) most like b(y + dy, x + dx),
/// to a tenth of a pixel by a parabola through the peak and its neighbours, kept when the peak
/// lies inside the search; the medians of the tiles kept (a few tiles on the film's edge or on
/// the picture's strong edges do not move them), and how many there were. `None` with fewer
/// than [`MIN_TILES`].
fn shift(a: &[f32], b: &[f32], w: usize, h: usize, r: usize) -> Option<(f64, f64, f64)> {
    let n = 2 * r + 1;
    let margin = r + 16;
    let starts = |len: usize, tile: usize| -> Vec<usize> {
        if len < tile + 2 * margin {
            return Vec::new();
        }
        let k = (len - 2 * margin) / tile;
        (0..k).map(|i| margin + i * tile).collect()
    };
    let para = |m: f64, p0: f64, p1: f64| {
        let den = m - 2.0 * p0 + p1;
        if den == 0.0 {
            0.0
        } else {
            0.5 * (m - p1) / den
        }
    };
    let mut found: Vec<(f64, f64)> = Vec::new();
    let mut c = vec![0f64; n * n];
    for &ty in &starts(h, TILE.1) {
        for &tx in &starts(w, TILE.0) {
            for (si, sum) in c.iter_mut().enumerate() {
                let dy = (si / n) as isize - r as isize;
                let dx = (si % n) as isize - r as isize;
                let mut acc = 0f64;
                for y in ty..ty + TILE.1 {
                    let yb = (y as isize + dy) as usize;
                    let xb = (tx as isize + dx) as usize;
                    let ra = &a[y * w + tx..y * w + tx + TILE.0];
                    let rb = &b[yb * w + xb..yb * w + xb + TILE.0];
                    for (&va, &vb) in ra.iter().zip(rb) {
                        acc += f64::from(va) * f64::from(vb);
                    }
                }
                *sum = acc;
            }
            let Some(best) = (0..n * n).max_by(|&i, &j| c[i].total_cmp(&c[j])) else {
                continue;
            };
            let (by, bx) = (best / n, best % n);
            if by == 0 || bx == 0 || by == n - 1 || bx == n - 1 || c[best] <= 0.0 {
                continue;
            }
            let fy = para(c[(by - 1) * n + bx], c[best], c[(by + 1) * n + bx]);
            let fx = para(c[by * n + bx - 1], c[best], c[by * n + bx + 1]);
            found.push((by as f64 - r as f64 + fy, bx as f64 - r as f64 + fx));
        }
    }
    if found.len() < MIN_TILES {
        return None;
    }
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        let m = v.len() / 2;
        if v.len() % 2 == 1 {
            v[m]
        } else {
            (v[m - 1] + v[m]) / 2.0
        }
    };
    let k = found.len() as f64;
    Some((
        median(found.iter().map(|f| f.0).collect()),
        median(found.iter().map(|f| f.1).collect()),
        k,
    ))
}

/// The search of [`register`]: so many frame pixels either way from the prediction (the
/// captures' shifts are within 1.2 lines and 1 column of it).
const SEARCH: usize = 5;

/// The infrared channel laid on a film frame (`colour`, put together by
/// `crate::scan::assemble_film` from `colour_plan`'s read) from the infrared read `ir_raw`
/// (`ir_plan`'s, three 16-bit samples a pixel): its green CCD row (the one that covers the
/// frame when the pass reads h + 4 lines), the late columns put back; across, the colour read's
/// pixel i is the infrared read's column + i x [`colour_per_infrared`]'s inverse; along, frame
/// row y is its line + y. The prediction (column 0, line 1 at 2400 dpi and 0 at 1200, from
/// Epson Scan's start and columns) is then moved by the shift a correlation of the infrared and
/// the colours' blue row finds (dust, hairs and the film's edges are in both; `ice.py
/// --register`'s, there 0.5-1.1 lines and 0.6 columns from scan to scan). One 16-bit sample a
/// frame pixel, row by row.
pub fn register(
    colour: &Image,
    colour_plan: &FilmPlan,
    ir_raw: &[u8],
    ir_plan: &FilmPlan,
) -> (Vec<u16>, Registration) {
    let (w, h) = (colour.width as usize, colour.height as usize);
    let g = &colour_plan.geometry;
    let u = f64::from(4800 / g.base);
    let middle = f64::from(film::logical_of(g.columns.0, g.words, g.base)) * u
        + f64::from(g.column_count()) * u / 2.0;
    let halved = g.read_dpi != colour_plan.request.dpi;
    let stagger = crate::scan::film_stagger(ir_plan);
    let mut reg = Registration {
        scale: colour_per_infrared(middle),
        line: f64::from(stagger / 4),
        column: 0.0,
        tiles: 0.0,
        measured: false,
    };
    let (plane, pw, ph) = green_plane(ir_raw, ir_plan);
    let first = lay(&plane, pw, ph, w, h, halved, &reg);
    // The colours' blue row (assemble_film's third sample), as f32.
    let s = usize::from(colour.bits / 8);
    let blue: Vec<f32> = colour
        .data
        .chunks_exact(3 * s)
        .map(|p| {
            if s == 2 {
                f32::from(u16::from_le_bytes([p[4], p[5]]))
            } else {
                f32::from(p[2])
            }
        })
        .collect();
    if let Some((dy, dx, tiles)) = shift(
        &relative(&blue, w, h, 31),
        &relative(&first, w, h, 31),
        w,
        h,
        SEARCH,
    ) {
        reg.line += dy;
        reg.column += dx * if halved { 2.0 } else { 1.0 } / reg.scale;
        reg.tiles = tiles;
        reg.measured = true;
    }
    let out = lay(&plane, pw, ph, w, h, halved, &reg)
        .into_iter()
        .map(|v| v.round().clamp(0.0, 65_535.0) as u16)
        .collect();
    (out, reg)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::film::Film;

    /// A made-up pre-read: two bright discs on a dark ground, as the holder's holes look.
    fn pre_read(holes: [(f64, f64); 2], radius: f64) -> Vec<u8> {
        let (px, lines) = (PRE_READ.0 as usize, PRE_READ.1 as usize);
        let mut d = vec![2u8; px * lines * 3];
        for y in 0..lines {
            for x in 0..px {
                for &(cx, cy) in &holes {
                    if (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2) <= radius * radius {
                        d[(y * px + x) * 3 + 1] = 255;
                    }
                }
            }
        }
        d
    }

    #[test]
    fn the_holes_are_found_as_epsons_interpreter_finds_them() {
        // Discs of radius 36 at (351, 269) and (6550, 256): 72 wide, 72 tall.
        let h = holes(&pre_read([(351.0, 269.0), (6550.0, 256.0)], 36.0)).unwrap();
        assert!(h.found);
        assert_eq!(h.values[0], 702);
        assert_eq!(h.values[1], 538);
        assert_eq!(h.values[2], 144);
        // Lines with 28 or more pixels lit: the height a little short of the diameter.
        assert!((120..=144).contains(&h.values[3]), "{:?}", h.values);
        assert_eq!(h.values[4], 2 * (6550 - RIGHT_HOLE as u16));
        assert_eq!(h.values[5], 512);
        let b = h.block88();
        assert_eq!(&b[..4], &[0x01, 0x00, 0xBE, 0x02]);
        assert_eq!(Holes::from_block88(&b), Some(h));
        // No holes: not found, and 1E 88 says so.
        let none = holes(&vec![2u8; (PRE_READ.0 * PRE_READ.1 * 3) as usize]).unwrap();
        assert!(!none.found);
        assert_eq!(none.block88()[0], 0);
    }

    #[test]
    fn the_pre_reads_column_halves_the_correction_toward_zero() {
        assert_eq!(pre_c0(XCorrection(-93)), 6352);
        assert_eq!(pre_c0(XCorrection(-133)), 6332);
        assert_eq!(pre_c0(XCorrection(-122)), 6337);
        // Captures 108-113's pre-read 1E 41, and 1E 21.
        assert_eq!(
            pre_e41(XCorrection(-133)).to_vec(),
            crate::setup::tests::hex("bc183c34e001000001010100000000008080e0010000")
        );
        assert_eq!(
            pre_e21().to_vec(),
            crate::setup::tests::hex("002a000000000000000000002a00000000000000000000000000")
        );
    }

    /// The six captures' holes (1E 88's and the infrared pre-read's, `icesetup.py`), their
    /// colour reads and what Epson Scan sent for the infrared: start move, columns, words, lines.
    #[allow(clippy::type_complexity)]
    pub(crate) const ICE: [(
        &str,
        FilmRequest,
        i16,
        [u16; 8],
        [u16; 8],
        u32,
        u32,
        (u32, u32),
        (u16, u16),
        u32,
    ); 8] = {
        const fn r(dpi: u32, x: u32, y: u32, width: u32, height: u32, film: Film) -> FilmRequest {
            FilmRequest {
                dpi,
                ydpi: dpi,
                x,
                y,
                width,
                height,
                bits: 16,
                film,
            }
        }
        use Film::{Negative as N, Positive as P};
        [
            (
                "18",
                r(2400, 423, 952, 2080, 3319, N),
                -93,
                [652, 558, 142, 140, 592, 534, 150, 142],
                [598, 538, 138, 138, 624, 516, 146, 134],
                479,
                518,
                (2424, 6648),
                (0, 0),
                3323,
            ),
            (
                "108",
                r(2400, 303, 1096, 2362, 3401, N),
                -133,
                [702, 538, 144, 140, 630, 512, 150, 140],
                [646, 528, 138, 138, 662, 504, 146, 132],
                623,
                667,
                (2380, 7244),
                (1831, 568),
                3405,
            ),
            (
                "109",
                r(1200, 151, 548, 1181, 1700, N),
                -133,
                [702, 540, 144, 140, 630, 514, 150, 138],
                [648, 528, 136, 136, 664, 502, 146, 134],
                623,
                665,
                (2358, 4918),
                (0, 0),
                1700,
            ),
            (
                "110",
                r(2400, 303, 1096, 2362, 3401, N),
                -133,
                [702, 538, 142, 142, 630, 514, 150, 140],
                [648, 528, 138, 136, 664, 502, 146, 134],
                623,
                666,
                (2382, 7246),
                (0, 0),
                3405,
            ),
            (
                "112",
                r(2400, 1631, 2915, 3401, 2362, P),
                -133,
                [706, 536, 144, 142, 638, 514, 148, 140],
                [652, 522, 138, 136, 670, 498, 146, 134],
                2442,
                2483,
                (2817, 9729),
                (0, 0),
                2366,
            ),
            (
                "113",
                r(2400, 1631, 2915, 3401, 2362, P),
                -133,
                [708, 532, 142, 140, 638, 508, 150, 140],
                [652, 516, 138, 138, 670, 494, 146, 134],
                2442,
                2483,
                (2815, 9727),
                (0, 0),
                2366,
            ),
            (
                "126",
                r(1200, 1876, 9446, 1212, 1691, N),
                -82,
                [654, 558, 142, 142, 582, 538, 150, 142],
                [600, 546, 138, 138, 616, 526, 146, 134],
                18_419,
                18_461,
                (3522, 6082),
                (0, 0),
                1691,
            ),
            (
                "133",
                r(600, 361, 728, 879, 600, P),
                -151,
                [718, 554, 144, 142, 650, 522, 150, 142],
                [664, 536, 138, 138, 682, 506, 146, 134],
                2343,
                2383,
                (2731, 6315),
                (0, 0),
                600,
            ),
        ]
    };

    /// Captures 132 (a slide at 1200 dpi), 134 (one at 2400) and 137 (a negative at 600), all
    /// with the correction -151: the infrared start one column before Epson Scan's (its hole
    /// reads behind it differ from ours by a half pixel now and then: 132 and 133, one slide and
    /// session, our holes 2 apart, Epson's start the same), the rest as it planned.
    #[test]
    fn the_infrared_columns_are_epson_scans_within_one() {
        let slide = |dpi, x, y, w, h| FilmRequest {
            dpi,
            ydpi: dpi,
            x,
            y,
            width: w,
            height: h,
            bits: 16,
            film: Film::Positive,
        };
        let neg = FilmRequest {
            film: Film::Negative,
            ..slide(600, 81, 204, 582, 892)
        };
        for (req, ch, ih, cstart, istart, c0, lines) in [
            (
                slide(1200, 722, 1457, 1759, 1201),
                [706, 542, 144, 142, 654, 508, 150, 140],
                [650, 532, 138, 138, 686, 498, 146, 134],
                2441,
                2484,
                2731,
                1201,
            ),
            (
                slide(2400, 1445, 2915, 3519, 2402),
                [720, 556, 144, 144, 650, 526, 152, 142],
                [664, 540, 136, 138, 682, 514, 146, 132],
                2442,
                2483,
                2743,
                2406,
            ),
            (
                neg,
                [720, 532, 144, 142, 652, 508, 150, 140],
                [666, 522, 138, 136, 684, 496, 146, 134],
                247,
                290,
                2356,
                892,
            ),
        ] {
            let corr = XCorrection(-151);
            let colour = FilmPlan::new(req, corr).unwrap();
            assert_eq!(colour.geometry.start_steps, cstart, "{req:?}");
            let h = |v| Holes {
                found: true,
                values: v,
            };
            let g = infrared_plan(&colour, corr, &h(ch), &h(ih))
                .unwrap()
                .geometry;
            assert_eq!((g.start_steps, g.lines), (istart, lines), "{req:?}");
            assert_eq!(g.columns.0 + 1, c0, "{req:?}");
        }
    }

    /// Captures 127 and 130 (Epson Scan right after a power-on): the colour read's line time
    /// asks a lead of 128, the infrared's 32; the infrared's first line 45 steps after the
    /// colour's (start move and lead: 269 + 128, 410 + 32).
    #[test]
    fn the_infrared_start_counts_each_reads_lead() {
        let req = FilmRequest {
            dpi: 1200,
            ydpi: 1200,
            x: 151,
            y: 419,
            width: 1188,
            height: 1784,
            bits: 16,
            film: Film::Negative,
        };
        for (corr, ch, ih, cols) in [
            (
                -104,
                [672, 556, 144, 142, 606, 530, 150, 142],
                [616, 550, 138, 138, 636, 524, 146, 134],
                (2355, 4915),
            ),
            (
                -87,
                [656, 564, 144, 142, 584, 538, 152, 142],
                [602, 556, 138, 138, 616, 532, 146, 132],
                (2365, 4925),
            ),
        ] {
            let corr = XCorrection(corr);
            let mut colour = FilmPlan::new(req, corr).unwrap();
            colour.geometry = colour.geometry.with_lead(128).unwrap();
            assert_eq!(colour.geometry.start_steps, 269);
            let h = |v| Holes {
                found: true,
                values: v,
            };
            let ir = infrared_plan(&colour, corr, &h(ch), &h(ih)).unwrap();
            let g = ir.geometry;
            assert_eq!((g.start_steps, g.lead, g.columns), (410, 32, cols));
            assert_eq!(g.read_steps, 1784 * 2 + 32);
            crate::guard::check_infrared_plan(&colour, &ir).unwrap();
        }
    }

    #[test]
    fn the_infrared_read_is_planned_as_epson_scan_planned_it() {
        for (name, req, corr, ch, ih, cstart, istart, cols, words, lines) in ICE {
            let corr = XCorrection(corr);
            let colour = FilmPlan::new(req, corr).unwrap();
            assert_eq!(colour.geometry.start_steps, cstart, "{name}");
            let (c, i) = (
                Holes {
                    found: true,
                    values: ch,
                },
                Holes {
                    found: true,
                    values: ih,
                },
            );
            let ir = infrared_plan(&colour, corr, &c, &i).unwrap();
            let g = ir.geometry;
            assert_eq!(g.start_steps, istart, "{name}");
            assert_eq!(g.columns, cols, "{name}");
            if words != (0, 0) {
                assert_eq!(g.words, words, "{name}");
            }
            assert_eq!(g.lines, lines, "{name}");
            let lead = crate::film::planned_lead(req.dpi);
            assert_eq!(g.read_steps, lines * 2400 / req.ydpi + lead, "{name}");
            assert_eq!(ir.pass, Pass::Infrared);
            assert_eq!(&ir.e21_origin[..3], &IR_ORIGIN_T.to_le_bytes()[..3]);
        }
    }

    #[test]
    fn the_infrared_request_is_epson_scans() {
        // 108's colour and infrared 1E 57.
        let r = ICE[1].1;
        let hex = crate::setup::tests::hex;
        assert_eq!(
            request_block(&r, Pass::IceColour).unwrap().to_vec(),
            hex("60090000600900002f010000480400003a090000490d0000131004001203000000000000000100000000")
        );
        assert_eq!(
            request_block(&r, Pass::Infrared).unwrap().to_vec(),
            hex("60090000600900002f010000480400003a090000490d0000201003002003000000000000000000000000")
        );
    }

    #[test]
    fn the_infrared_exposures_end_as_epson_scans() {
        let hex = crate::setup::tests::hex;
        // 108: the green 16,498 µs, the image read ending at 13,740 + 4864.
        assert_eq!(
            final_block(16_498, 13_740 + 4864, (3840, 550), 2).to_vec(),
            hex("8040000000000000000000002a000f2602aa12be1f0000000002")
        );
        assert_eq!(
            final_block(16_498, 18_604, (1831, 568), 6).to_vec(),
            hex("8040000000000000000000002a27073802aa12be1f0000000006")
        );
        // 109, base 2400: 16,563 µs, the read ending at 6870 + 2560.
        assert_eq!(
            final_block(16_563, 9430, (2209, 548), 2).to_vec(),
            hex("0042000000000000000000002aa1082402b4097e200000000002")
        );
    }

    #[test]
    fn the_start_moves_with_the_holes() {
        for (name, _, _, ch, ih, cstart, istart, ..) in ICE {
            let d = start_delta(
                &Holes {
                    found: true,
                    values: ch,
                },
                &Holes {
                    found: true,
                    values: ih,
                },
            );
            assert_eq!(cstart as i32 + d, istart as i32, "{name}");
        }
    }

    #[test]
    fn the_lens_ratio_is_one_line_through_both_places_measured() {
        assert!((colour_per_infrared(16_150.0) - 0.99279).abs() < 1e-9);
        assert!((colour_per_infrared(19_850.0) - 0.99250).abs() < 1e-9);
        // 109 (1200 dpi), its middle near 16,160: 0.99282 measured.
        assert!((colour_per_infrared(16_160.0) - 0.99282).abs() < 5e-5);
    }

    /// The registration on Epson Scan's own reads of every ICE capture (dumped by `ice.py`'s
    /// decoder to `ICE_REG/<n>_col.bin` and `<n>_ir.bin`, three u16 a pixel), to set beside
    /// `ice.py --register`; it writes the frame as the driver assembles it (`<n>_frame.bin`,
    /// three u16 a pixel; `<n>_size.txt` its width and height) and the infrared it lays on it
    /// (`<n>_laid.bin`, one u16 a pixel) for `ice_dust.py`, and the scan-imaging crate's
    /// `dust_on_the_captures`. Run with `ICE_REG=… cargo test --release -p epson-gt-x750 --lib
    /// registration_on_the_captures -- --ignored --nocapture`; a capture whose reads are not
    /// there is skipped.
    #[test]
    #[ignore = "needs the captures' reads, outside the repository"]
    fn registration_on_the_captures() {
        let dir = std::env::var("ICE_REG").expect("ICE_REG");
        // 127 and 130: Epson Scan right after a power-on, the colour read's lead 128.
        let after_power_on = |corr: i16, ch, ih| {
            let req = FilmRequest {
                dpi: 1200,
                ydpi: 1200,
                x: 151,
                y: 419,
                width: 1188,
                height: 1784,
                bits: 16,
                film: Film::Negative,
            };
            (req, corr, ch, ih, Some(128))
        };
        // Name, request, x correction, holes in colour and in infrared, the colour read's lead.
        type Case = (FilmRequest, i16, [u16; 8], [u16; 8], Option<u32>);
        let mut cases: Vec<(&str, Case)> = ICE
            .iter()
            .map(|&(k, req, corr, ch, ih, ..)| (k, (req, corr, ch, ih, None)))
            .collect();
        cases.push((
            "127",
            after_power_on(
                -104,
                [672, 556, 144, 142, 606, 530, 150, 142],
                [616, 550, 138, 138, 636, 524, 146, 134],
            ),
        ));
        cases.push((
            "130",
            after_power_on(
                -87,
                [656, 564, 144, 142, 584, 538, 152, 142],
                [602, 556, 138, 138, 616, 532, 146, 132],
            ),
        ));
        // 134: the slide of the dust pair (20 November 2026), as it was before it was blown clean.
        cases.push((
            "134",
            (
                FilmRequest {
                    dpi: 2400,
                    ydpi: 2400,
                    x: 1445,
                    y: 2915,
                    width: 3519,
                    height: 2402,
                    bits: 16,
                    film: Film::Positive,
                },
                -151,
                [720, 556, 144, 144, 650, 526, 152, 142],
                [664, 540, 136, 138, 682, 514, 146, 132],
                None,
            ),
        ));
        // 141: a Kodak BW400CN (chromogenic black and white, C-41) as a negative at 1200 dpi
        // (21 November 2026): how much of the infrared its picture explains.
        cases.push((
            "141",
            (
                FilmRequest {
                    dpi: 1200,
                    ydpi: 1200,
                    x: 163,
                    y: 8082,
                    width: 1165,
                    height: 1784,
                    bits: 16,
                    film: Film::Negative,
                },
                -151,
                [710, 542, 144, 142, 650, 512, 150, 142],
                [654, 526, 138, 138, 684, 496, 146, 134],
                None,
            ),
        ));
        // Where our infrared start is one column before Epson Scan's (its reads are laid by the
        // columns they were read at).
        let read_at = [("134", 2743u32), ("141", 2357)];
        for (k, (req, corr, ch, ih, lead)) in cases {
            if !std::path::Path::new(&format!("{dir}/{k}_col.bin")).exists() {
                println!("{k}: no reads, skipped");
                continue;
            }
            let corr = XCorrection(corr);
            let mut geometry = crate::film::geometry(&req, corr).unwrap();
            if let Some(lead) = lead {
                geometry = geometry.with_lead(lead).unwrap();
            }
            let colour = FilmPlan::from_geometry(req, geometry, Pass::IceColour);
            let h = |v| Holes {
                found: true,
                values: v,
            };
            let mut ir = infrared_plan(&colour, corr, &h(ch), &h(ih)).unwrap();
            if let Some(&(_, c0)) = read_at.iter().find(|(n, _)| *n == k) {
                let (a, b) = ir.geometry.columns;
                ir.geometry.columns = (c0, c0 + b - a);
            }
            let col = std::fs::read(format!("{dir}/{k}_col.bin")).unwrap();
            let raw_ir = std::fs::read(format!("{dir}/{k}_ir.bin")).unwrap();
            let image = crate::scan::assemble_film(&col, &colour);
            let (out, reg) = register(&image, &colour, &raw_ir, &ir);
            let mean = out.iter().map(|&v| f64::from(v)).sum::<f64>() / out.len() as f64;
            println!(
                "{k}: {reg:?}, infrared mean {mean:.0}, frame {} x {}",
                image.width, image.height
            );
            std::fs::write(format!("{dir}/{k}_frame.bin"), &image.data).unwrap();
            std::fs::write(
                format!("{dir}/{k}_size.txt"),
                format!("{} {}", image.width, image.height),
            )
            .unwrap();
            std::fs::write(
                format!("{dir}/{k}_laid.bin"),
                out.iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<u8>>(),
            )
            .unwrap();
        }
    }

    /// A made-up frame and its infrared read, the infrared 0.7 % larger and a line and a half
    /// lower, the dust in both: the registration finds the shift and lays the dust on it.
    #[test]
    fn the_infrared_is_laid_on_the_colours() {
        let req = FilmRequest {
            dpi: 1200,
            ydpi: 1200,
            x: 151,
            y: 548,
            width: 600,
            height: 600,
            bits: 16,
            film: Film::Negative,
        };
        let corr = XCorrection(-133);
        let colour_plan = FilmPlan::new(req, corr).unwrap();
        let holes = Holes {
            found: true,
            values: [702, 540, 144, 140, 630, 514, 150, 138],
        };
        let ir_plan = infrared_plan(&colour_plan, corr, &holes, &holes).unwrap();
        let g = colour_plan.geometry;
        let gi = ir_plan.geometry;
        let middle = f64::from(film::logical_of(g.columns.0, g.words, g.base)) * 2.0
            + f64::from(g.column_count());
        let scale = colour_per_infrared(middle);
        // Dust: dark specks at made-up places, each a few pixels.
        let specks: Vec<(f64, f64)> = (0..60)
            .map(|k| {
                let k = k as f64;
                (40.0 + (k * 97.0) % 520.0, 40.0 + (k * 61.0) % 520.0)
            })
            .collect();
        let dust = |y: f64, x: f64| -> f64 {
            specks
                .iter()
                .map(|&(sy, sx)| (-((y - sy).powi(2) + (x - sx).powi(2)) / 4.0).exp())
                .sum::<f64>()
                .min(1.0)
        };
        let (w, h) = (req.width as usize, req.height as usize);
        let mut data = Vec::with_capacity(w * h * 6);
        for y in 0..h {
            for x in 0..w {
                let v = (30_000.0 * (1.0 - 0.8 * dust(y as f64, x as f64))) as u16;
                for _ in 0..3 {
                    data.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        let colour = Image {
            width: req.width,
            height: req.height,
            bits: 16,
            data,
        };
        // The infrared: frame (y, x) at line y + 1.5 + 0 (no stagger at 1200), column 0.8 +
        // x / scale.
        let (dy, dx) = (1.5, 0.8);
        let (px, lines) = (gi.pixels as usize, gi.lines as usize);
        let mut raw = vec![0u8; px * lines * 6];
        for l in 0..lines {
            for j in 0..px {
                let y = l as f64 - dy;
                let x = (j as f64 - dx) * scale;
                let v = (50_000.0 * (1.0 - 0.6 * dust(y, x))) as u16;
                let at = (l * px + j) * 6 + 2;
                raw[at..at + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
        let (ir, reg) = register(&colour, &colour_plan, &raw, &ir_plan);
        assert!(reg.measured, "{reg:?}");
        assert!((reg.line - dy).abs() < 0.2, "{reg:?}");
        assert!((reg.column - dx).abs() < 0.3, "{reg:?}");
        // Under the first speck the infrared is dark.
        let (sy, sx) = specks[0];
        let at = sy as usize * w + sx as usize;
        assert!(ir[at] < 25_000, "{}", ir[at]);
        assert!(ir[300 * w + 5] > 45_000 || dust(300.0, 5.0) > 0.1);
    }
}
