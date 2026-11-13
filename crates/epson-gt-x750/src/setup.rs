// SPDX-License-Identifier: GPL-2.0-or-later
//! The set-up blocks of a glass scan, computed from the request.
//!
//! The rules were read in Epson's interpreter (`esxuin54.dll` 1.27, under the
//! EU interoperability exception; addresses and the derivation in
//! `captures/epson4490/06-formato-dati.md`, "Il set-up calcolato dalla
//! richiesta", outside the repository) and checked on every glass capture:
//! the tests rebuild, from each capture's request, every block Epson's driver
//! wrote. The code is ours, as is the Python model it was ported from
//! (`captures/epson4490/tools/setup.py`); no table of Epson's is copied: the
//! motor ramps are computed ([`ramp`], [`fixed_ramp`]).
//!
//! Units: the motor makes [`STEPS_PER_INCH`] steps per inch; T, the line time,
//! is in microseconds; columns are at the sensor's base resolution.
//!
//! Only colour on the glass (8 or 16 bits per channel). What the scanner
//! may actually be sent is decided by [`crate::guard`], not here: this module
//! also computes requests no capture has shown (it is a model of Epson's
//! rules, tested on the captures).

use scanner_api::{Error, Result};

/// The motor's steps per inch.
pub const STEPS_PER_INCH: u32 = 2400;
/// Steps from home to the glass's first line (`0x593` in the interpreter).
pub const Y_ORIGIN: u32 = 1427;
/// The bytes after every block of image data: `02`, the block's length (u24),
/// `00 00 80 00`.
pub const TRAILER_LEN: usize = 8;

/// A request in pixels at its resolution, as WIA's `1E 57` carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// Resolution across.
    pub dpi: u32,
    /// Resolution along (WIA: the same; Epson Scan asked 3200 for 2400).
    pub ydpi: u32,
    /// Left edge, pixels at `dpi` from the glass's origin.
    pub x: u32,
    /// Top edge, pixels at `ydpi`.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bits per channel: 8 or 16.
    pub bits: u8,
}

/// The resolutions read along at another resolution, as Epson Scan reads them: 2400 dpi at
/// 3200 lines an inch (captures 13, 57 and 58, never 2400 along), the image then reduced on the
/// computer ([`crate::scan::reduce_along`]).
pub const READ_ALONG: [(u32, u32); 1] = [(2400, 3200)];

impl Request {
    /// A request with the same resolution both ways, as WIA makes them.
    pub fn new(dpi: u32, x: u32, y: u32, width: u32, height: u32, bits: u8) -> Self {
        Self {
            dpi,
            ydpi: dpi,
            x,
            y,
            width,
            height,
            bits,
        }
    }

    /// The request that reads an image of `width` x `height` pixels at `dpi` both ways, at
    /// `(x, y)`: [`Request::new`], but read along at [`READ_ALONG`]'s resolution where it has
    /// one, `y` rounded and `height` rounded up there (so that [`Request::image_height`] gives
    /// `height` back).
    pub fn glass(dpi: u32, x: u32, y: u32, width: u32, height: u32, bits: u8) -> Self {
        let Some(&(_, ydpi)) = READ_ALONG.iter().find(|(d, _)| *d == dpi) else {
            return Self::new(dpi, x, y, width, height, bits);
        };
        let (num, den) = (u64::from(ydpi), u64::from(dpi));
        let along = |v: u32, up: bool| {
            let v = u64::from(v) * num;
            let v = if up {
                v.div_ceil(den)
            } else {
                (2 * v + den) / (2 * den)
            };
            u32::try_from(v).unwrap_or(u32::MAX)
        };
        Self {
            ydpi,
            y: along(y, false),
            height: along(height, true),
            ..Self::new(dpi, x, y, width, height, bits)
        }
    }

    /// The lines of the image this request gives, at `dpi`: its `height`, or fewer when it is
    /// read along at another resolution.
    pub fn image_height(&self) -> u32 {
        let h = u64::from(self.height) * u64::from(self.dpi) / u64::from(self.ydpi);
        u32::try_from(h).unwrap_or(u32::MAX)
    }
}

/// One row of the interpreter's resolution table (`0x40cbb0`, the colour
/// group, not Epson Scan's draft mode) with its starting line time (`0x40dab0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    /// The sensor's base resolution the columns are counted at.
    pub base: u32,
    /// dpi / base x 100,000.
    pub ratio: u32,
    /// The starting line time, µs.
    pub t0: u32,
}

/// The resolutions of the table: any other one is not a glass scan the
/// interpreter knows (it would become 4800, another path).
pub fn resolution(dpi: u32) -> Option<Resolution> {
    let (base, ratio, t0) = match dpi {
        100 => (800, 12_500, 4224),
        200 => (800, 25_000, 3840),
        300 => (800, 37_500, 3840),
        400 => (800, 50_000, 3840),
        600 => (800, 75_000, 3840),
        800 => (800, 100_000, 3840),
        900 => (1200, 75_000, 5568),
        1200 => (2400, 50_000, 10_752),
        2400 => (2400, 100_000, 10_752),
        _ => return None,
    };
    Some(Resolution { base, ratio, t0 })
}

/// `1E 11` and the first byte of `1E 46`: the base resolution's index.
fn base_index(base: u32) -> Option<u8> {
    Some(match base {
        4800 => 0,
        2400 => 1,
        1200 => 2,
        800 => 3,
        600 => 4,
        _ => return None,
    })
}

/// The x origin at a base (`0x40ca26`, columns) and the 4800ths of an inch
/// per column.
fn origin(base: u32) -> Option<(u32, i32)> {
    Some(match base {
        2400 => (276, 2),
        1200 => (139, 4),
        800 => (93, 6),
        600 => (70, 8),
        _ => return None,
    })
}

/// The columns of the sensor's black (shielded) pixels, from the captures:
/// known at 800 and 2400 only.
fn black_columns(base: u32) -> Option<(u16, u16)> {
    match base {
        800 => Some((12, 28)),
        2400 => Some((35, 83)),
        _ => None,
    }
}

/// The ratio's code in `1E 41` (`0x41f7ae`).
fn ratio_code(ratio: u32) -> Option<u8> {
    Some(match ratio {
        3125 => 0x50,
        6250 => 0x40,
        12_500 => 0x30,
        25_000 => 0x20,
        37_500 => 0xB0,
        50_000 => 0x10,
        75_000 => 0xA0,
        100_000 => 0x00,
        _ => return None,
    })
}

/// The floor of `1E 21`'s fourth u16 in the data's set-up.
fn floor_21(base: u32) -> Option<u32> {
    Some(match base {
        600 | 800 => 0x6CC,
        1200 => 0xA33,
        2400 => 0x1466,
        _ => return None,
    })
}

/// `1E 43` for the calibration reads: gain 1.0 (`8000`) per channel, then the
/// glass's constants (`0x420626`).
pub const GAIN_CAL: [u8; 12] = [
    0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x70, 0x75, 0xCE, 0x78, 0x5A, 0x7C,
];
/// `1E 43` for the image read.
pub const GAIN_DATA: [u8; 12] = [
    0x70, 0x75, 0xCE, 0x78, 0x5A, 0x7C, 0x70, 0x75, 0xCE, 0x78, 0x5A, 0x7C,
];

/// The x origin's correction stored in the scanner: bytes 2-3 of the `1E 87`
/// answer, in 4800ths of an inch (`a3 ff` = -93 on Matteo's 4490; another
/// scanner may have another).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XCorrection(pub i16);

impl XCorrection {
    /// From the 10 bytes `1E 87` answers.
    pub fn from_1e87(answer: &[u8]) -> Result<Self> {
        match answer {
            [_, _, lo, hi, ..] if answer.len() == 10 => Ok(Self(i16::from_le_bytes([*lo, *hi]))),
            _ => Err(Error::Other(format!(
                "1E 87 answered {} bytes, not 10",
                answer.len()
            ))),
        }
    }

    /// The origin at `base` corrected (`0x427240`).
    fn origin(self, base: u32) -> Option<u32> {
        let (origin, div) = origin(base)?;
        let c = i32::from(self.0);
        Some(if c < 0 {
            origin.saturating_sub(((-c) / div) as u32)
        } else {
            origin + (c / div) as u32
        })
    }
}

/// How the motor runs during a read (`0x429240`), from its speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 2000 steps/s or more: a 512-value ramp.
    Fast,
    /// 700-1999 steps/s: 512 values halved.
    Medium,
    /// Under 700: one value, no ramp.
    Slow,
}

impl Mode {
    fn of(v: u32) -> Self {
        if v >= 2000 {
            Self::Fast
        } else if v < 700 {
            Self::Slow
        } else {
            Self::Medium
        }
    }

    /// The ramp's steps (R).
    fn ramp(self) -> u32 {
        match self {
            Self::Fast => 512,
            Self::Medium => 256,
            Self::Slow => 0,
        }
    }
}

/// A move without reading (`0x413cd0`): the ramp for `1E 84` and the `1E 01`
/// block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    /// The steps.
    pub steps: u32,
    /// Which of the three kinds of move.
    pub kind: MoveKind,
    /// The `1E 84` type 04 table.
    pub ramp: Vec<u16>,
    /// The `1E 01` block.
    pub block: [u8; 12],
}

/// The three kinds of move without reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    /// Under 514 steps: one speed, no ramp.
    Short,
    /// 514-1025: the 256-value ramp.
    Medium,
    /// 1026 or more: the 512-value ramp.
    Long,
}

/// A `1E 01` block: the steps in all, the lead, the ramp's last index twice,
/// the flags.
pub fn move_block(total: u32, lead: u16, ramp: u16, flags: u16) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[..4].copy_from_slice(&total.to_le_bytes());
    b[4..6].copy_from_slice(&lead.to_le_bytes());
    b[6..8].copy_from_slice(&ramp.to_le_bytes());
    b[8..10].copy_from_slice(&ramp.to_le_bytes());
    b[10..12].copy_from_slice(&flags.to_le_bytes());
    b
}

/// The acceleration from 400 steps/s to 10⁶/`period` steps/s in `n` steps,
/// as the interpreter computes it (`0x429dc0`): the periods in µs, divided by
/// `prescale`. `k` is 2n − 3 (1021 for 512 values).
fn accelerate(period: u32, n: usize, k: f64, prescale: f64) -> Vec<u16> {
    let fe = 1e6 / f64::from(period);
    let mut a = 2.0 * (fe * fe - 160_000.0);
    a /= ((fe / 400.0).powi(2) + k * k - 1.0).sqrt() + k;
    let v0 = 400.0 - a / 800.0;
    let mut prev = 0.0;
    (0..n)
        .map(|i| {
            let t = ((i as f64 + 1.0) * 2.0 * a + v0 * v0).sqrt() - v0;
            let t = t / a;
            let p = ((t - prev) * 1e6 / prescale) as u16;
            prev = t;
            p
        })
        .collect()
}

/// The ramp of a read (`1E 84` type 04): `period` µs a step at full speed.
pub fn ramp(period: u32, mode: Mode) -> Vec<u16> {
    let prescale = match mode {
        Mode::Slow => return vec![(period >> 4) as u16],
        Mode::Fast => 1,
        Mode::Medium => 2,
    };
    let mut out = accelerate(period, 512, 1021.0, f64::from(prescale));
    out[511] = (period / prescale) as u16;
    out
}

/// The two fixed ramps of the moves without reading (`0x401260`, 256
/// values, and `0x401860`, 512): the same acceleration as [`ramp`], from
/// 400 steps/s to 10⁶/250 and 10⁶/160 steps/s, with the first period
/// 2500 µs exactly and the last one less than the full speed's. Computed,
/// not copied: checked against the values the scanner received in the
/// captures (the 512 one) and against the interpreter (both).
pub fn fixed_ramp(n: usize) -> Vec<u16> {
    let period = if n == 256 { 250 } else { 160 };
    let mut out = accelerate(period, n, (2 * n - 3) as f64, 1.0);
    out[0] = 2500;
    out[n - 1] = (period - 1) as u16;
    out
}

/// A move of `steps` without reading.
pub fn start_move(steps: u32) -> Move {
    let (kind, ramp, block) = if steps < 0x202 {
        (MoveKind::Short, vec![156], move_block(steps, 0, 0, 0x18))
    } else if steps < 0x402 {
        (
            MoveKind::Medium,
            fixed_ramp(256),
            move_block(steps - 512, 0, 0xFF, 0x10),
        )
    } else {
        (
            MoveKind::Long,
            fixed_ramp(512),
            move_block(steps - 1024, 0, 0x1FF, 0x10),
        )
    };
    Move {
        steps,
        kind,
        ramp,
        block,
    }
}

/// The header of a `1E 84` table: type, address (u24), flag, length in bytes
/// (u24).
pub fn table_header(kind: u8, address: u32, flag: u8, len: usize) -> [u8; 8] {
    let a = address.to_le_bytes();
    let l = (len as u32).to_le_bytes();
    [kind, a[0], a[1], a[2], flag, l[0], l[1], l[2]]
}

/// The header of a motor ramp.
pub fn ramp_header(values: usize) -> [u8; 8] {
    table_header(0x04, 0x01_0000, 0x02, values * 2)
}

/// The header of the shading table for `columns` columns.
pub fn shading_header(columns: u32) -> [u8; 8] {
    table_header(0x05, 0x01_0400, 0x02, columns as usize * 6)
}

/// A `1E 41` block (`0x41f730`, 22 bytes).
#[allow(clippy::too_many_arguments)]
fn block41(
    c0: u32,
    c1: u32,
    lines: u32,
    b8: u8,
    den: u8,
    num: u8,
    b11: u8,
    b12: u8,
    b16: u8,
    b17: u8,
    lpb: u16,
) -> [u8; 22] {
    let mut b = [0u8; 22];
    b[0..2].copy_from_slice(&(c0 as u16).to_le_bytes());
    b[2..4].copy_from_slice(&(c1 as u16).to_le_bytes());
    b[4..8].copy_from_slice(&lines.to_le_bytes());
    b[8] = b8;
    b[9] = den;
    b[10] = num;
    b[11] = b11;
    b[12] = b12;
    // 13, 14: zero; 15: the shading's two scales, set once known.
    b[16] = b16;
    b[17] = b17;
    b[18..20].copy_from_slice(&lpb.to_le_bytes());
    b
}

/// A `1E 21` block (26 bytes). The last two u16 are zero in every capture but Epson Scan's 15
/// (600 dpi, 16 bits, 2 and 1002; its 43, the same resolution and depth, zero): not known.
fn block21(t: u32, w3fe: u32, w400: u32, last: u8) -> [u8; 26] {
    let mut b = [0u8; 26];
    b[..3].copy_from_slice(&t.to_le_bytes()[..3]);
    b[12] = 0x2A;
    b[17..19].copy_from_slice(&(w3fe as u16).to_le_bytes());
    b[19..21].copy_from_slice(&(w400 as u16).to_le_bytes());
    b[25] = last;
    b
}

/// WIA's `1E 57` for `r` (42 bytes): the request in pixels, colour (`13`), the
/// bits, the lines per block asked, gamma `03`; the colour correction (byte
/// 33) zero. Byte 28 is a guess: the even floor of 65,536 / bytes per
/// requested line fits the WIA captures (the scanner reads with `1E 41`'s
/// lines per block anyway).
pub fn request_block(r: &Request) -> Result<[u8; 42]> {
    let line = r.width * 3 * u32::from(r.bits) / 8;
    let asked = u8::try_from((65_536 / line.max(1)) & !1).map_err(|_| {
        Error::InvalidRequest(format!(
            "{} pixels: narrower than any scan seen (the request block cannot carry it)",
            r.width
        ))
    })?;
    let mut e57 = [0u8; 42];
    for (i, v) in [r.dpi, r.ydpi, r.x, r.y, r.width, r.height]
        .iter()
        .enumerate()
    {
        e57[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    e57[24] = 0x13;
    e57[25] = r.bits;
    e57[28] = asked;
    e57[29] = 0x03;
    Ok(e57)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Everything a glass scan sends that depends on the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// What was asked.
    pub request: Request,
    /// The correction it was computed with.
    pub x_correction: XCorrection,
    /// The sensor's base resolution.
    pub base: u32,
    /// The line time, µs.
    pub t: u32,
    /// Lines (den) per `num` motor steps.
    pub den: u32,
    /// Motor steps per `den` lines.
    pub num: u32,
    /// Lines read, the colours' offset and the rounding to `den` included.
    pub lines: u32,
    /// Pixels per line read (wider than asked).
    pub pixels: u32,
    /// First and last column read, at the base resolution.
    pub columns: (u32, u32),
    /// The shading table's non-zero width, in columns.
    pub shading_columns: u32,
    /// Where the image's first line is, motor steps from home.
    pub start: u32,
    /// The read's speed, steps/s.
    pub speed: u32,
    /// How the motor runs during the read.
    pub mode: Mode,
    /// Steps of lead before the read.
    pub lead: u32,
    /// Whether the read would start lower than asked (too close to home to
    /// get up to speed: never seen on the glass).
    pub starts_late: bool,
    /// Bytes per line of image data.
    pub bytes_per_line: u32,
    /// Lines per block of image data.
    pub lines_per_block: u32,
    /// The carriage's position after the calibration, steps.
    pub after_calibration: u32,
    /// The furthest step the read reaches.
    pub read_end: u32,
    /// `1E 57` (42 bytes).
    pub e57: [u8; 42],
    /// `1E 11`.
    pub e11: u8,
    /// The two `1E 46` (before and after the wait).
    pub e46: [[u8; 8]; 2],
    /// `1E 21` of the calibration reads.
    pub e21_cal: [u8; 26],
    /// `1E 21` of the image read.
    pub e21_data: [u8; 26],
    /// `1E 41` of the black pixels.
    pub e41_black: [u8; 22],
    /// `1E 41` of the white.
    pub e41_white: [u8; 22],
    /// `1E 41` of the dark.
    pub e41_dark: [u8; 22],
    /// `1E 41` of the image; byte 15 (the shading's scales) still zero.
    pub e41_data: [u8; 22],
    /// The white calibration's ramp: one value.
    pub ramp_cal: Vec<u16>,
    /// The white calibration's `1E 01`.
    pub move_cal: [u8; 12],
    /// The move from the calibration to the lead, if any.
    pub start_move: Option<Move>,
    /// The image read's ramp.
    pub ramp_data: Vec<u16>,
    /// The image read's `1E 01`.
    pub move_data: [u8; 12],
}

impl Plan {
    /// Computes the plan of `request` for a scanner whose `1E 87` gave
    /// `x_correction`. Errors for what the interpreter's rules do not cover
    /// (a resolution out of its table, a base without known black pixels,
    /// a width the request block cannot carry).
    pub fn new(request: Request, x_correction: XCorrection) -> Result<Self> {
        let r = request;
        let bad = |what: String| Error::InvalidRequest(what);
        if r.bits != 8 && r.bits != 16 {
            return Err(bad(format!("{} bits per channel", r.bits)));
        }
        if r.width == 0 || r.height == 0 || r.ydpi == 0 {
            return Err(bad("an empty area".into()));
        }
        let res =
            resolution(r.dpi).ok_or_else(|| bad(format!("{} dpi is not in the table", r.dpi)))?;
        let unknown = || bad(format!("{} dpi: base {} not modelled", r.dpi, res.base));
        let opt = base_index(res.base).ok_or_else(unknown)?;
        let origin = x_correction.origin(res.base).ok_or_else(unknown)?;
        let (k0, k1) = black_columns(res.base).ok_or_else(unknown)?;
        let code = ratio_code(res.ratio).ok_or_else(unknown)?;
        let floor21 = floor_21(res.base).ok_or_else(unknown)?;
        let ratio = u64::from(res.ratio);

        // Columns (0x427240 ... 0x427688).
        let w = u64::from(r.width);
        let step = if matches!(res.ratio, 37_500 | 75_000) {
            384
        } else {
            16
        };
        let pixels = w.div_ceil(step) * step;
        let cols = pixels * 100_000 / ratio;
        let x = u64::from(r.x) * 100_000;
        let xc = x / ratio + if x % ratio != 0 { 4 } else { 0 };
        let wc = w * 100_000 / ratio + if (w * 100_000) % ratio != 0 { 4 } else { 0 };
        let shading_columns = wc.min(cols) as u32;
        let c0 = origin + xc as u32;
        let c1 = c0 + cols as u32;
        let pixels = pixels as u32;

        // The line time (0x4276b4): longer for a wide read.
        let mut t = res.t0;
        while c1 / 2 >= t - 254 {
            t += 384;
        }

        // Vertical (0x4291e0, 0x427710).
        let g = gcd(r.ydpi, STEPS_PER_INCH);
        let (den, num) = (r.ydpi / g, STEPS_PER_INCH / g);
        let extra = 2 * (r.ydpi / 50);
        let lines = (r.height + extra).div_ceil(den) * den;
        let start = Y_ORIGIN + (u64::from(r.y) * u64::from(num) / u64::from(den)) as u32;
        let bytes_per_line = pixels * 3 * u32::from(r.bits) / 8;
        let lines_per_block = (65_527 / bytes_per_line).min(lines);
        if lines_per_block == 0 {
            // Only at 2400 dpi and 16 bits inside the glass: a line over a block was never
            // captured (Epson Scan's widest at 16 bits, 1200 dpi on the whole glass, is 61,200
            // bytes a line).
            let most = 65_527 / (3 * u32::from(r.bits) / 8) / 16 * 16;
            return Err(bad(format!(
                "{pixels} pixels a line at {} dpi and {} bits, more than a block holds: at most \
                 {most} pixels ({:.0} mm) across",
                r.dpi,
                r.bits,
                f64::from(most) / f64::from(r.dpi) * 25.4
            )));
        }

        // The speed and the ramp's mode (0x429240), the minimum lead (0x41e760).
        let speed = (u64::from(num) * 1_000_000 / (u64::from(den) * u64::from(t))) as u32;
        let mode = Mode::of(speed);
        let m = if speed < 151 {
            32
        } else if speed < 5500 {
            128
        } else {
            160
        };
        // The calibration: 16 lines at 1200 lines per inch, no ramp.
        let vcal = 2 * 1_000_000 / t;
        let mcal = if vcal < 151 { 32 } else { 128 };
        let move_cal = move_block(mcal + 32, mcal as u16, 0, 0x18);
        let after_calibration = mcal + 32;

        // The start move and the lead (0x41e760).
        let l0 = i64::from(start) - i64::from(after_calibration);
        let rr = i64::from(mode.ramp());
        let m = i64::from(m);
        let far = match mode {
            Mode::Fast => 3 * 512 + 2 * m,
            Mode::Medium => 2 * m + 1280,
            Mode::Slow => m,
        };
        let (mut n, mut lead, mut starts_late) = (0i64, m, false);
        if l0 > far {
            n = l0 - rr - m;
        } else if l0 < m + rr {
            starts_late = true;
        } else {
            lead = l0 - rr;
        }
        let num64 = i64::from(num);
        n += lead % num64;
        lead -= lead % num64;
        let (n, lead) = (n as u32, lead as u32);
        let read = lines * num / den;
        let (flags, ramp_end) = match mode {
            Mode::Fast => (0, 0x1FF),
            Mode::Medium => (2, 0xFF),
            Mode::Slow => (8, 0),
        };
        let move_data = move_block(lead + read, lead as u16, ramp_end, flags);
        let b8 = code | u8::from(r.bits == 8);

        let mut w3fe = floor21.max((c1 + 10) / 4);
        let mut w400 = (t - 259) / 2;
        if (c1 + 10) / 2 >= t - 259 || w3fe >= w400 {
            (w3fe, w400) = (0, 0);
        }
        let (den8, num8) = (den as u8, num as u8);
        let e57 = request_block(&r)?;
        let start_move = (n > 0).then(|| start_move(n));
        Ok(Self {
            request: r,
            x_correction,
            base: res.base,
            t,
            den,
            num,
            lines,
            pixels,
            columns: (c0, c1),
            shading_columns,
            start,
            speed,
            mode,
            lead,
            starts_late,
            bytes_per_line,
            lines_per_block,
            after_calibration,
            // The read's ramp adds its steps to its block's (S = 160 + N + R + L).
            read_end: after_calibration + n + mode.ramp() + lead + read,
            e57,
            e11: opt,
            e46: [
                [opt, 1, t as u8, (t >> 8) as u8, 0, 0, 0, 0],
                [opt, 0, t as u8, (t >> 8) as u8, 0, 0, 0, 0],
            ],
            e21_cal: block21(t, 0, 0, 0),
            e21_data: block21(t, w3fe, w400, 4),
            e41_black: block41(
                u32::from(k0),
                u32::from(k1),
                4,
                0,
                den8,
                num8,
                0,
                0,
                0x80,
                0,
                4,
            ),
            e41_white: block41(c0, c1, 16, 0, 1, 2, 0x40, 0, 0x80, 0x80, 16),
            e41_dark: block41(c0, c1, 16, 0, den8, num8, 0, 0, 0x80, 0, 16),
            e41_data: block41(
                c0,
                c1,
                lines,
                b8,
                den8,
                num8,
                0x80,
                0x80,
                0,
                0xA0,
                lines_per_block as u16,
            ),
            ramp_cal: vec![((t / 2) >> 4) as u16],
            move_cal,
            start_move,
            ramp_data: ramp(den * t / num, mode),
            move_data,
        })
    }

    /// Columns read: as many in the calibration as in the image.
    pub fn column_count(&self) -> u32 {
        self.columns.1 - self.columns.0
    }

    /// The black pixels' columns.
    pub fn black_column_count(&self) -> u32 {
        let (a, b) = black_columns(self.base).expect("checked by new");
        u32::from(b - a)
    }

    /// The colours' offset in lines: green is this many lines below blue,
    /// red twice as many.
    pub fn colour_offset(&self) -> u32 {
        self.request.ydpi / 50
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One step of a stripped capture (`testdata/README.md`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum Line {
        /// A write: its bytes, or a placeholder (`=gamma`, `=tone`, `=shading N`,
        /// `=42 HEX`, `=other N`).
        W(Vec<u8>, Option<String>),
        /// An answer that must be these bytes.
        R(Vec<u8>),
        /// An answer that may differ (1E 53, 86, 87, 92).
        A(Vec<u8>),
        /// A state poll and its last answer.
        P(Vec<u8>, Vec<u8>),
        /// Image data, so many bytes.
        D(usize),
        /// An answer left out (`R =other N`).
        Skipped(usize),
    }

    pub(crate) fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    pub(crate) fn parse(text: &str) -> Vec<Line> {
        text.lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .map(|l| {
                let f: Vec<&str> = l.split(' ').collect();
                match f.as_slice() {
                    ["W", p, rest @ ..] if p.starts_with('=') => {
                        let tag = std::iter::once(&p[1..])
                            .chain(rest.iter().copied())
                            .collect::<Vec<_>>()
                            .join(" ");
                        Line::W(Vec::new(), Some(tag))
                    }
                    ["W", h] => Line::W(hex(h), None),
                    ["R" | "A", "=other", n] => Line::Skipped(n.parse().unwrap()),
                    ["R", h] => Line::R(hex(h)),
                    ["A", h] => Line::A(hex(h)),
                    ["P", c, h] => Line::P(hex(c), hex(h)),
                    ["D", n] => Line::D(n.parse().unwrap()),
                    _ => panic!("not a line: {l}"),
                }
            })
            .collect()
    }

    pub(crate) const CAPTURES: [(&str, &str); 26] = [
        ("03", include_str!("../testdata/03_colore_300dpi_5x5cm.txt")),
        ("04", include_str!("../testdata/04_colore_600dpi_5x5cm.txt")),
        (
            "05",
            include_str!("../testdata/05_colore_1200dpi_2x2cm.txt"),
        ),
        ("06", include_str!("../testdata/06_colore_150dpi_A4.txt")),
        ("07", include_str!("../testdata/07_colore_300dpi_A4.txt")),
        (
            "10",
            include_str!("../testdata/10_colore_300dpi_area_centro.txt"),
        ),
        (
            "13",
            include_str!("../testdata/13_epsonscan_2400dpi_48bit.txt"),
        ),
        (
            "15",
            include_str!("../testdata/15_epsonscan_600dpi_48bit.txt"),
        ),
        (
            "25",
            include_str!("../testdata/25_wia_vetro_dopo_pellicole.txt"),
        ),
        (
            "29",
            include_str!("../testdata/29_wia_vetro_riferimento.txt"),
        ),
        (
            "30",
            include_str!("../testdata/30_colore_300dpi_x40_y60.txt"),
        ),
        (
            "31",
            include_str!("../testdata/31_colore_200dpi_x40_y60.txt"),
        ),
        (
            "32",
            include_str!("../testdata/32_colore_600dpi_x40_y60.txt"),
        ),
        (
            "33",
            include_str!("../testdata/33_colore_1200dpi_x40_y60.txt"),
        ),
        ("34", include_str!("../testdata/34_colore_400dpi_2x2cm.txt")),
        ("35", include_str!("../testdata/35_colore_100dpi_5x5cm.txt")),
        ("36", include_str!("../testdata/36_colore_300dpi_8x5cm.txt")),
        ("37", include_str!("../testdata/37_colore_800dpi_2x2cm.txt")),
        (
            "38",
            include_str!("../testdata/38_epsonscan_100dpi_48bit.txt"),
        ),
        (
            "39",
            include_str!("../testdata/39_epsonscan_200dpi_48bit.txt"),
        ),
        (
            "40",
            include_str!("../testdata/40_epsonscan_300dpi_48bit.txt"),
        ),
        (
            "41",
            include_str!("../testdata/41_epsonscan_400dpi_48bit.txt"),
        ),
        (
            "42",
            include_str!("../testdata/42_epsonscan_1200dpi_48bit.txt"),
        ),
        (
            "43",
            include_str!("../testdata/43_epsonscan_600dpi_48bit_vetro.txt"),
        ),
        (
            "57",
            include_str!("../testdata/57_epsonscan_2400dpi_48bit_20x30.txt"),
        ),
        (
            "58",
            include_str!("../testdata/58_epsonscan_2400dpi_24bit.txt"),
        ),
    ];

    /// The blocks a capture wrote after each command, in order.
    pub(crate) fn blocks(lines: &[Line]) -> Vec<(u8, Vec<Vec<u8>>)> {
        let mut out: Vec<(u8, Vec<Vec<u8>>)> = Vec::new();
        for l in lines {
            match l {
                Line::W(b, None) if b.len() == 2 && (b[0] == 0x1E || b[0] == 0x1B) => {
                    out.push((if b[0] == 0x1E { b[1] } else { 0 }, Vec::new()))
                }
                Line::W(b, tag) => {
                    let b = match tag {
                        Some(t) => format!("={t}").into_bytes(),
                        None => b.clone(),
                    };
                    out.last_mut().expect("a command first").1.push(b);
                }
                _ => {}
            }
        }
        out
    }

    pub(crate) fn of(blocks: &[(u8, Vec<Vec<u8>>)], cmd: u8) -> Vec<&Vec<Vec<u8>>> {
        blocks
            .iter()
            .filter(|(c, w)| *c == cmd && !w.is_empty())
            .map(|(_, w)| w)
            .collect()
    }

    /// The request and the 1E 87 answer of a capture.
    pub(crate) fn request_of(lines: &[Line]) -> (Request, XCorrection, u8) {
        let b = blocks(lines);
        let q = &of(&b, 0x57)[0][0];
        let u = |i: usize| u32::from_le_bytes(q[4 * i..4 * i + 4].try_into().unwrap());
        let request = Request {
            dpi: u(0),
            ydpi: u(1),
            x: u(2),
            y: u(3),
            width: u(4),
            height: u(5),
            bits: q[25],
        };
        let i87 = lines
            .iter()
            .position(|l| matches!(l, Line::W(b, None) if b == &[0x1E, 0x87]))
            .unwrap();
        let answer = lines[i87 + 1..]
            .iter()
            .find_map(|l| match l {
                Line::A(a) if a.len() == 10 => Some(a.clone()),
                _ => None,
            })
            .unwrap();
        (request, XCorrection::from_1e87(&answer).unwrap(), q[33])
    }

    fn ramp_bytes(r: &[u16]) -> Vec<u8> {
        r.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// Every set-up block of every glass capture, rebuilt from its request:
    /// the model of `setup.py --check`, in Rust.
    #[test]
    fn every_captured_block_is_rebuilt_from_the_request() {
        for (name, text) in CAPTURES {
            let lines = parse(text);
            let b = blocks(&lines);
            let (request, xc, cc) = request_of(&lines);
            let p = Plan::new(request, xc).unwrap_or_else(|e| panic!("{name}: {e}"));
            let ctx = |what: &str| format!("capture {name}, {what}");
            let mut e57 = p.e57;
            e57[33] = cc;
            // WIA's own only: Epson Scan's 1E 57 has gamma 04 (byte 29) and its own lines per
            // block (byte 28), at 8 bits too (capture 58).
            if of(&b, 0x57)[0][0][29] == 0x03 {
                assert_eq!(of(&b, 0x57)[0][0], e57.to_vec(), "{}", ctx("1E 57"));
            }
            for w in of(&b, 0x11) {
                assert_eq!(w[0], [p.e11], "{}", ctx("1E 11"));
            }
            let e46 = of(&b, 0x46);
            assert_eq!(e46[0][0], p.e46[0], "{}", ctx("1E 46"));
            assert_eq!(e46[1][0], p.e46[1], "{}", ctx("1E 46 (2)"));
            // Capture 15's last two u16 (2, 1002) are not modelled: left out.
            let e21: Vec<Vec<u8>> = of(&b, 0x21)
                .iter()
                .map(|w| {
                    let mut b = w[0].clone();
                    if name == "15" {
                        b[21..25].fill(0);
                    }
                    b
                })
                .collect();
            for w in &e21[..3] {
                assert_eq!(*w, p.e21_cal, "{}", ctx("1E 21 cal"));
            }
            assert_eq!(e21[3], p.e21_data, "{}", ctx("1E 21 data"));
            let e41 = of(&b, 0x41);
            assert_eq!(e41[0][0], p.e41_black, "{}", ctx("1E 41 black"));
            assert_eq!(e41[1][0], p.e41_white, "{}", ctx("1E 41 white"));
            assert_eq!(e41[2][0], p.e41_dark, "{}", ctx("1E 41 dark"));
            let mut data = e41[3][0].clone();
            data[15] = 0; // the shading's scales
            let mut want = p.e41_data;
            want[16] = cc;
            assert_eq!(data, want, "{}", ctx("1E 41 data"));
            let e43 = of(&b, 0x43);
            assert_eq!(e43[0][0], GAIN_CAL, "{}", ctx("1E 43 cal"));
            assert_eq!(e43.last().unwrap()[0], GAIN_DATA, "{}", ctx("1E 43 data"));
            let ramps: Vec<_> = of(&b, 0x84).into_iter().filter(|w| w[0][0] == 4).collect();
            let moves = of(&b, 0x01);
            let check_ramp = |w: &Vec<Vec<u8>>, r: &[u16], what: &str| {
                assert_eq!(w[0], ramp_header(r.len()), "{}", ctx(what));
                assert_eq!(w[1], ramp_bytes(r), "{}", ctx(what));
            };
            check_ramp(ramps[0], &p.ramp_cal, "ramp cal");
            assert_eq!(moves[0][0], p.move_cal, "{}", ctx("1E 01 cal"));
            let k = match &p.start_move {
                Some(m) => {
                    check_ramp(ramps[1], &m.ramp, "ramp start");
                    assert_eq!(moves[1][0], m.block, "{}", ctx("1E 01 start"));
                    2
                }
                None => {
                    assert_eq!(moves.len(), 2, "{}", ctx("a start move not modelled"));
                    1
                }
            };
            check_ramp(ramps[k], &p.ramp_data, "ramp data");
            assert_eq!(moves[k][0], p.move_data, "{}", ctx("1E 01 data"));
            let shading = of(&b, 0x84).into_iter().find(|w| w[0][0] == 5).unwrap();
            assert_eq!(
                shading[0],
                shading_header(p.column_count()),
                "{}",
                ctx("shading")
            );
            assert!(!p.starts_late, "{}", ctx("starts late"));
        }
    }

    #[test]
    fn the_x_correction_is_read_from_1e_87() {
        let c = XCorrection::from_1e87(&hex("0100a3ff1b003201cc00")).unwrap();
        assert_eq!(c, XCorrection(-93));
        assert_eq!(c.origin(800), Some(78));
        assert_eq!(c.origin(2400), Some(230));
        assert_eq!(XCorrection(12).origin(800), Some(95));
        assert!(XCorrection::from_1e87(&[1, 0, 0xA3]).is_err());
        // Another correction moves every column.
        let r = Request::new(300, 0, 0, 591, 591, 8);
        let a = Plan::new(r, XCorrection(-93)).unwrap();
        let b = Plan::new(r, XCorrection(0)).unwrap();
        assert_eq!(b.columns.0 - a.columns.0, 15);
    }

    /// The 512-value fixed ramp, as the scanner received it (capture 10's start
    /// move), and both fixed ramps as the interpreter holds them (first and last
    /// values, read in `esxuin54.dll`).
    #[test]
    fn the_fixed_ramps_are_computed() {
        let r = fixed_ramp(512);
        assert_eq!(r.len(), 512);
        assert_eq!(r[..6], [2500, 2054, 1785, 1601, 1464, 1357]);
        assert_eq!(r[506..], [160, 160, 160, 160, 160, 159]);
        let r = fixed_ramp(256);
        assert_eq!(r.len(), 256);
        assert_eq!(r[..6], [2500, 2119, 1872, 1696, 1562, 1455]);
        assert_eq!(r[250..], [252, 251, 251, 250, 250, 249]);
        assert!(r.windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn the_start_move_kinds_follow_the_steps() {
        assert_eq!(start_move(3).kind, MoveKind::Short);
        assert_eq!(start_move(3).ramp, [156]);
        assert_eq!(start_move(513).kind, MoveKind::Short);
        assert_eq!(start_move(514).kind, MoveKind::Medium);
        assert_eq!(start_move(1025).kind, MoveKind::Medium);
        assert_eq!(start_move(1026).kind, MoveKind::Long);
        assert_eq!(
            start_move(6299).block,
            [0x9B, 0x14, 0, 0, 0, 0, 0xFF, 0x01, 0xFF, 0x01, 0x10, 0]
        );
    }

    /// The first generated script's request (300 dpi, 5x5 cm at 40/60 mm): the
    /// values of `06-formato-dati.md`'s table and of the replay on 12 November.
    #[test]
    fn the_first_generated_request() {
        let p = Plan::new(Request::new(300, 472, 709, 591, 591, 8), XCorrection(-93)).unwrap();
        assert_eq!(p.columns, (1340, 3388));
        assert_eq!(p.start_move.as_ref().map(|m| m.steps), Some(6299));
        assert_eq!(p.lead, 128);
        assert_eq!(p.lines, 603);
        assert_eq!(p.pixels, 768);
        assert_eq!(p.colour_offset(), 6);
        assert_eq!(
            p.move_data,
            [0x58, 0x13, 0, 0, 0x80, 0, 0xFF, 0x01, 0xFF, 0x01, 0, 0]
        );
    }

    #[test]
    fn requests_out_of_the_table_are_errors() {
        let x = XCorrection(-93);
        assert!(Plan::new(Request::new(150, 0, 0, 100, 100, 8), x).is_err());
        assert!(Plan::new(Request::new(4800, 0, 0, 100, 100, 8), x).is_err());
        // 900 dpi: base 1200, black pixels not known.
        assert!(Plan::new(Request::new(900, 0, 0, 100, 100, 8), x).is_err());
        assert!(Plan::new(Request::new(300, 0, 0, 100, 100, 12), x).is_err());
        assert!(Plan::new(Request::new(300, 0, 0, 0, 100, 8), x).is_err());
        // Too narrow for 1E 57's byte 28.
        assert!(Plan::new(Request::new(300, 0, 0, 20, 100, 8), x).is_err());
    }
}
