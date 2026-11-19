// SPDX-License-Identifier: GPL-2.0-or-later
//! Films in the lid's transparency unit: the request, as Epson Scan sends it.
//!
//! Read from Epson Scan's captures of films (16-19, 72-81; `captures/epson4490/05-catture.md`,
//! outside the repository): the strips' and the slides' holders share one window,
//! [`ADAPTER_INCHES`] (Epson Scan's preview of either asks 810 x 2799 pixels at 300 dpi from
//! `(0, 0)`); a film's `1E 57` is Epson Scan's form ([`request_block`]), rebuilt byte for byte
//! by the tests from every film capture; what the set-up takes from the request is
//! [`geometry`], checked on the eight film scans captured. What may be sent is
//! [`crate::guard`]'s.

use crate::setup::XCorrection;
use scanner_api::{Error, Result};

/// The transparency unit's window, inches across and along: Epson Scan's preview of the strips
/// and of the slides, 810 x 2799 pixels at 300 dpi (68.6 x 237.0 mm; the user guide's 4490
/// takes a strip of six frames or four slides).
pub const ADAPTER_INCHES: (f64, f64) = (2.7, 9.33);

/// The film window's top our scans leave out, steps of 1/2400 in (0.24 in, 6.1 mm): a read
/// starting higher would need a start move shorter than its ramps, never captured
/// ([`geometry`] refuses it; Epson Scan's own previews from the very top go another way, not
/// read). Only the holders' edge is there: the first frame of a strip starts at 8.8 mm (capture
/// 74), a slide's at 31.8 mm (capture 80).
pub const TOP_SKIP_STEPS: u32 = 576;

/// The window as the application sees it ([`for_area`]): [`ADAPTER_INCHES`] less
/// [`TOP_SKIP_STEPS`] at the top, inches across and along.
pub const AREA_INCHES: (f64, f64) = (
    ADAPTER_INCHES.0,
    ADAPTER_INCHES.1 - TOP_SKIP_STEPS as f64 / 2400.0,
);

/// The film request for an area of [`AREA_INCHES`] (pixels at `dpi` from its corner), read
/// along at [`READ_ALONG`]'s resolution: its top [`TOP_SKIP_STEPS`] lower in the window, its
/// lines enough for `height` once brought back to `dpi` (the image is then cut to `height`), and
/// never past the window's bottom.
pub fn for_area(
    dpi: u32,
    film: Film,
    (x, y, width, height): (u32, u32, u32, u32),
    bits: u8,
) -> FilmRequest {
    let ydpi = READ_ALONG
        .iter()
        .find(|(d, _)| *d == dpi)
        .map_or(dpi, |(_, along)| *along);
    let along = |v: u32| u64::from(v) * u64::from(ydpi) / u64::from(dpi);
    let top = TOP_SKIP_STEPS * ydpi / 2400;
    let y_read = along(y) as u32 + top;
    let max_h = (ADAPTER_INCHES.1 * f64::from(ydpi)).round() as u32;
    let lines = (u64::from(height) * u64::from(ydpi)).div_ceil(u64::from(dpi)) as u32;
    FilmRequest {
        dpi,
        ydpi,
        x,
        y: y_read,
        width,
        height: lines.min(max_h.saturating_sub(y_read)),
        bits,
        film,
    }
}

/// What is in the holder: Epson Scan's "colour negative" or "colour positive" (slides).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Film {
    /// A negative: byte 37 of `1E 57` is `01`.
    Negative,
    /// A positive (a slide): byte 37 is `00`.
    Positive,
}

/// The resolutions read along at another resolution on films: 4800 dpi at 5400 lines an inch
/// (capture 76, Epson Scan's 4800 dpi frame; its image then 6,997 lines for the 7,872 read).
/// 2400 dpi is read at 2400 on films (captures 17, 19, 77, 78, 80, 81), not at 3200 as on the
/// glass.
pub const READ_ALONG: [(u32, u32); 1] = [(4800, 5400)];

/// A film request in pixels at its resolution, from the adapter's window's corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilmRequest {
    /// Resolution across.
    pub dpi: u32,
    /// Resolution along ([`READ_ALONG`]).
    pub ydpi: u32,
    /// Left edge, pixels at `dpi`.
    pub x: u32,
    /// Top edge, pixels at `ydpi`.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels at `ydpi`.
    pub height: u32,
    /// Bits per channel: 8 or 16.
    pub bits: u8,
    /// Negative or positive.
    pub film: Film,
}

/// The most lines a block of Epson Scan's film requests asks (byte 28 of `1E 57`).
const MAX_LINES_PER_BLOCK: u32 = 32;

/// The block size byte 28 is counted from: the lines per block are this over the bytes of a
/// line, at most [`MAX_LINES_PER_BLOCK`] (true of the eleven film requests captured).
const BLOCK_BYTES: u32 = 262_144;

/// `1E 57`'s byte 28 for lines of `line` bytes: [`BLOCK_BYTES`] over them, at most
/// [`MAX_LINES_PER_BLOCK`].
pub fn lines_per_request_block(line: u32) -> u8 {
    (BLOCK_BYTES / line.max(1)).min(MAX_LINES_PER_BLOCK) as u8
}

/// Epson Scan's `1E 57` for a film (42 bytes): the request in pixels, colour (`13`), the bits,
/// the source `01` (the transparency unit), the lines per block, gamma `04` (Epson Scan's), the
/// film in byte 37; everything else zero (no colour correction in the scanner, as on the glass).
pub fn request_block(r: &FilmRequest) -> Result<[u8; 42]> {
    let line = r.width * 3 * u32::from(r.bits) / 8;
    if line == 0 {
        return Err(Error::InvalidRequest(
            "Epson 4490: a film request with no width".into(),
        ));
    }
    let mut e57 = [0u8; 42];
    for (i, v) in [r.dpi, r.ydpi, r.x, r.y, r.width, r.height]
        .iter()
        .enumerate()
    {
        e57[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    e57[24] = 0x13;
    e57[25] = r.bits;
    e57[26] = 0x01;
    e57[28] = lines_per_request_block(line);
    e57[29] = 0x04;
    e57[37] = u8::from(r.film == Film::Negative);
    Ok(e57)
}

/// Where Epson Scan parks the carriage for films, from home (`04-carriage.md`'s `0x420f60`): the
/// first move of every film scan (`1E 25 00`), by 1994 less where the carriage is (captures
/// 74-81: from 0; capture 72: already there, no move).
pub const PARK: u32 = 1994;

/// The steps a move with the 512-value ramp spends on its ramps, up and down: such a `1E 01`
/// carries the steps beyond them.
pub const RAMP_512_STEPS: u32 = 1024;

/// The film window's first column, in 4800ths of an inch at base 4800, less `1E 87`'s x
/// correction (captures 17 with -93 and 74-81 with -122: the same window).
pub const FILM_ORIGIN: i32 = 13_320;

/// The start move's constant: the image's first line is `y * 2400 / ydpi + START_K - lead`
/// steps ahead of the carriage after the white read (at 2058 in every capture).
const START_K: u32 = 583;

/// The most bytes of image a block carries (as on the glass): the lines per block are this over
/// the bytes of a line.
pub(crate) const BLOCK_MAX: u32 = 65_527;

/// What a film scan's set-up takes from its request, as Epson Scan computes it (read from
/// captures 17 and 74-81 and Epson's interpreter, `captures/epson4490/tools/filmsetup.py`, 0
/// differences on all eight): the columns, the lines, the motor's ratios and moves. What comes
/// from the calibration reads (the lid unit's origin, the offsets and gains of `1E 31`, the
/// line time T and the exposures of `1E 21`, the shading) is not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// The sensor's base: 2400 up to 1200 dpi, 4800 at 2400 and 4800 dpi.
    pub base: u32,
    /// The resolution read across: 4800 for a 2400 dpi film (two columns a pixel, halved on
    /// the computer), the request's otherwise.
    pub read_dpi: u32,
    /// Pixels a line across, as read: a multiple of 16.
    pub pixels: u32,
    /// The read's first column and the one after its last, at `base`.
    pub columns: (u32, u32),
    /// `1E 21`'s words at bytes 13-14 and 15-16 for the reads of the image's columns.
    pub words: (u16, u16),
    /// Lines read: the image's, the colours' offsets (2 x ydpi / 50) and, read at 4800 across,
    /// the staggered columns' (ydpi / 600: 4 lines at 2400 along, 9 at 5400), rounded up to a
    /// multiple of the motor ratio's denominator (9 at 5400).
    pub lines: u32,
    /// `1E 41`'s byte 8: the ratio of `read_dpi` to `base`, `| 1` at 8 bits.
    pub b8: u8,
    /// The motor's ratio, ydpi / 2400 reduced: `1E 41`'s bytes 9 and 10.
    pub ratio: (u8, u8),
    /// Lines a data block carries.
    pub lines_per_block: u16,
    /// The image read's lead (steps before the first line).
    pub lead: u32,
    /// The image read's `1E 01` steps: its lines along at 2400 steps an inch, and the lead.
    pub read_steps: u32,
    /// The start move's `1E 01` steps (with the 512-value ramp, both carriages): from the
    /// carriage after the white read to the image's first line.
    pub start_steps: u32,
    /// `1E 11` for the film's reads: 8 + 1 at base 2400.
    pub e11: u8,
}

impl Geometry {
    /// The image read's one-value ramp for a line time `t` (µs): (den x T / num) >> 4, as the
    /// glass's (captures: 210 at 600 dpi with T 13,440, 1890 at 4800 x 5400).
    pub fn read_ramp(&self, t: u32) -> u16 {
        let (den, num) = self.ratio;
        ((u32::from(den) * t / u32::from(num)) >> 4) as u16
    }

    /// The columns read, at `base`.
    pub fn column_count(&self) -> u32 {
        self.columns.1 - self.columns.0
    }

    /// The bytes of an image line as read: [`Geometry::pixels`] x 3 samples of `bits`.
    pub fn line_bytes(&self, bits: u8) -> u32 {
        self.pixels * 3 * u32::from(bits) / 8
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// The film window's first column at `base` with `1E 87`'s x correction `corr` (the logical
/// start of a read with no x): [`FILM_ORIGIN`] + `corr` at base 4800; at base 2400 both halved,
/// the correction toward zero (the interpreter halves its magnitude, `0x421e24`; capture 109's
/// colour read, -133, gives 6594 where a floor would give 6593; -122 halves either way).
pub fn logical_origin(corr: XCorrection, base: u32) -> i32 {
    let c = i32::from(corr.0);
    if base == 4800 {
        FILM_ORIGIN + c
    } else {
        FILM_ORIGIN / 2 + c / 2
    }
}

/// The glass's left-edge form (`0x40fdd0`, divisor 6 at base 4800, 3 at base 2400) of a read
/// starting at the logical column `logical` for `count` columns: its first `1E 41` column and
/// `1E 21`'s words at bytes 13-14 and 15-16 (w1, r9).
pub fn left_edge(logical: u32, count: u32, base: u32) -> (u32, (u16, u16)) {
    let d = 6 * base / 4800;
    let r9 = ((logical.saturating_sub(80) / d) >> 3) * 2;
    let r8 = logical - 4 * d * r9 - 80;
    let c0 = r8 + 4 * r9 + 80;
    let w1 = (((r8 + count + 160) >> 1) + 2 * r9) >> 1;
    (c0, (w1 as u16, r9 as u16))
}

/// The logical column of a read whose first `1E 41` column is `c0` and whose words are `words`
/// ([`left_edge`]'s inverse): `c0` + 4 (d - 1) r9.
pub fn logical_of(c0: u32, words: (u16, u16), base: u32) -> u32 {
    let d = 6 * base / 4800;
    c0 + 4 * (d - 1) * u32::from(words.1)
}

/// The geometry of a film request with `1E 87`'s x correction `corr`. The start column is the
/// glass's left-edge form ([`left_edge`]) from the logical start [`logical_origin`] + x x base /
/// dpi.
pub fn geometry(r: &FilmRequest, corr: XCorrection) -> Result<Geometry> {
    let bad = |why: &str| Err(Error::InvalidRequest(format!("Epson 4490: {why}")));
    if !matches!(r.dpi, 600 | 1200 | 2400 | 4800) || r.width == 0 || r.height == 0 {
        return bad("a film request outside the resolutions modelled");
    }
    let (base, read_dpi) = if r.dpi >= 2400 {
        (4800, 4800)
    } else {
        (2400, r.dpi)
    };
    let pixels = (r.width * read_dpi / r.dpi).div_ceil(16) * 16;
    let count = pixels * base / read_dpi;
    let logical = logical_origin(corr, base) + (r.x * base / r.dpi) as i32;
    let Ok(logical) = u32::try_from(logical) else {
        return bad("a film's start column before the scanner's first");
    };
    let (c0, words) = left_edge(logical, count, base);
    let g = gcd(r.ydpi, 2400);
    let (den, num) = (r.ydpi / g, 2400 / g);
    // The staggered columns' lines when read at 4800 across (4 at 2400 along, 9 at 5400), the
    // lines rounded up to a multiple of the ratio's denominator (9 at 5400: 8100 in capture 76,
    // 5328 in 101).
    let stagger = if read_dpi == 4800 { r.ydpi / 600 } else { 0 };
    let lines = (r.height + 2 * (r.ydpi / 50) + stagger).div_ceil(den) * den;
    let line_bytes = pixels * 3 * u32::from(r.bits) / 8;
    let ratio_code = match base / read_dpi {
        1 => 0x00,
        2 => 0x10,
        4 => 0x20,
        8 => 0x50,
        _ => return bad("a film's read resolution with no ratio code"),
    };
    let lead = planned_lead(r.dpi);
    let y_steps = r.y * 2400 / r.ydpi;
    if y_steps + START_K - lead <= RAMP_512_STEPS {
        // The start move would be shorter than its ramps: never captured (the least start
        // move seen is 267 steps beyond them, capture 74).
        return bad("a film area this near the window's top was never captured");
    }
    Ok(Geometry {
        base,
        read_dpi,
        pixels,
        columns: (c0, c0 + count),
        words,
        lines,
        b8: ratio_code | u8::from(r.bits == 8),
        ratio: (den as u8, num as u8),
        lines_per_block: (BLOCK_MAX / line_bytes) as u16,
        lead,
        read_steps: lines * 2400 / r.ydpi + lead,
        start_steps: y_steps + START_K - lead - RAMP_512_STEPS,
        e11: 8 + u8::from(base == 2400),
    })
}

/// The image read's lead a plan starts with, before the calibration has found its line time
/// ([`lead_for`] then sets it): 128 at 600 dpi, 32 otherwise, as every film captured but 127.
pub fn planned_lead(dpi: u32) -> u32 {
    if dpi == 600 {
        128
    } else {
        32
    }
}

/// The image read's lead (steps of run-up before its first line) for a read along at `ydpi` with
/// the line time `t` µs: Epson's interpreter's rule for every read (`0x41e760`, decoded for the
/// glass, `captures/epson4490/tools/setup.py`): the carriage's speed v = (2400 / ydpi) x 10^6 / t
/// steps a second; 32 below 151, 128 below 5500, 160 above. On films it gives every captured
/// lead: 128 at 600 dpi, 32 at 1200-4800, and 128 at 1200 dpi with t 11,904 (capture 127, Epson
/// Scan right after a power-on; v 168).
pub fn lead_for(ydpi: u32, t: u32) -> u32 {
    let v = 2400u64 * 1_000_000 / (u64::from(ydpi) * u64::from(t.max(1)));
    if v < 151 {
        32
    } else if v < 5500 {
        128
    } else {
        160
    }
}

impl Geometry {
    /// The same read with another `lead`: the image's first line where it was (the start move
    /// shorter by what the lead grows), the read move as long as its lines and the lead.
    pub fn with_lead(self, lead: u32) -> Result<Self> {
        let first = self.start_steps + self.lead;
        if first < lead {
            return Err(Error::Other(format!(
                "Epson 4490: a film read's lead of {lead} steps before a first line {first} steps on"
            )));
        }
        Ok(Self {
            start_steps: first - lead,
            read_steps: self.read_steps - self.lead + lead,
            lead,
            ..self
        })
    }
}

/// The `1E 43` of the film's reads after the lid unit's origin (gains 1.0 and the film's own
/// constants, `0040` each: every film capture), and of its image (`0040` throughout).
pub const E43_READS: [u8; 12] = [
    0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40,
];
/// The image read's `1E 43`.
pub const E43_IMAGE: [u8; 12] = [
    0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40,
];
/// A slide's `1E 43`, for its reads and its image alike: gain 1.0 (`8000`) throughout (captures
/// 79-81).
pub const E43_SLIDE: [u8; 12] = [
    0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80,
];

/// The `1E 43` of a film's calibration reads (`image` false) or of its image.
pub fn e43(film: Film, image: bool) -> [u8; 12] {
    match (film, image) {
        (Film::Positive, _) => E43_SLIDE,
        (Film::Negative, false) => E43_READS,
        (Film::Negative, true) => E43_IMAGE,
    }
}

/// The lid unit's origin read (every film capture): `1E 11` 03, `1E 21` T 6144 and nothing else
/// but byte 12 `2a`, `1E 41` columns 2800-3184, 150 lines, 8 bits, ratio 1/12, `1E 31` offsets
/// `80` and gains `1e`; its move 8 steps (one-value ramp 156) before, 1920 with a lead of 120
/// during.
pub const ORIGIN_E11: u8 = 0x03;
/// The origin read's `1E 31`.
pub const ORIGIN_E31: [u8; 12] = [0x80, 0, 0x80, 0, 0x80, 0, 0, 0, 0x1E, 0x1E, 0x1E, 0];

/// The lid unit's origin read's `1E 41`: columns 2800-3184, 150 lines, 8 bits, ratio 1/12.
pub fn origin_e41() -> [u8; 22] {
    crate::setup::block41(2800, 3184, 150, 0x01, 1, 12, 0, 0, 0x80, 0x80, 150)
}

/// Which scan a film plan is for: a film without Digital ICE, or one of Digital ICE's two passes
/// ([`crate::ice`]): the colour pass (`1E 57` source `04`) and the infrared one (source `03`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    /// A film scan (captures 17, 72-81, 98-101).
    Film,
    /// Digital ICE's colour pass (captures 18, 108-113).
    IceColour,
    /// Digital ICE's infrared pass.
    Infrared,
}

/// What a film scan sends that follows from its request and `1E 87`'s correction: the
/// [`Geometry`], and every `1E 41` and fixed move of its reads (captures 17, 74-81; the
/// calibration's values, from the reads, are `crate::film_cal`'s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilmPlan {
    /// What was asked.
    pub request: FilmRequest,
    /// Its geometry.
    pub geometry: Geometry,
    /// `1E 21` of the origin read.
    pub e21_origin: [u8; 26],
    /// `1E 41` of the origin read.
    pub e41_origin: [u8; 22],
    /// `1E 41` of the black pixels' reads (4 lines, 16 bits).
    pub e41_black: [u8; 22],
    /// `1E 41` of the window's exposure reads (4 lines, 8 bits).
    pub e41_window: [u8; 22],
    /// `1E 41` of the white read (16 lines over the image's columns, ratio 1/2).
    pub e41_white: [u8; 22],
    /// `1E 41` of the dark read (64 lines over the image's columns).
    pub e41_dark: [u8; 22],
    /// `1E 41` of the image read, byte 15 (the shading's scales) still zero.
    pub e41_image: [u8; 22],
    /// The black pixels' reads' `1E 21` words (bytes 13-16) and margin (the line time is it +
    /// the longest exposure, rounded up to 384, in the offsets' search).
    pub black: ((u16, u16), u32),
    /// The image columns' margin (`crate::film_cal::image_margin`).
    pub image_margin: u32,
    /// The scan it is for.
    pub pass: Pass,
}

impl FilmPlan {
    /// The plan of `request` with `1E 87`'s x correction `corr`.
    pub fn new(request: FilmRequest, corr: XCorrection) -> Result<Self> {
        Ok(Self::from_geometry(
            request,
            geometry(&request, corr)?,
            Pass::Film,
        ))
    }

    /// The plan of `request` read with the geometry `g`, for `pass`: the infrared pass's origin
    /// read has T 8064 (6144 on films and in the colour pass; captures 18, 108-113).
    pub fn from_geometry(request: FilmRequest, g: Geometry, pass: Pass) -> Self {
        use crate::setup::block41;
        let (den, num) = g.ratio;
        let (c0, c1) = g.columns;
        let lines = g.lines;
        let base4800 = g.base == 4800;
        let (b0, b1) = if base4800 { (70, 166) } else { (35, 83) };
        let (w0, w1) = if base4800 {
            (2320, 15_280)
        } else {
            (2276, 8756)
        };
        let origin_t: u32 = if pass == Pass::Infrared {
            crate::ice::IR_ORIGIN_T
        } else {
            6144
        };
        let mut e21_origin = [0u8; 26];
        e21_origin[..3].copy_from_slice(&origin_t.to_le_bytes()[..3]);
        e21_origin[12] = 0x2A;
        let r8 = c0 - 4 * u32::from(g.words.1) - 80;
        Self {
            request,
            e21_origin,
            e41_origin: origin_e41(),
            e41_black: block41(b0, b1, 4, 0x00, den, num, 0, 0, 0x80, 0, 4),
            e41_window: block41(w0, w1, 4, 0x01, den, num, 0, 0, 0x80, 0, 4),
            e41_white: block41(c0, c1, 16, 0x00, 1, 2, 0x40, 0, 0x80, 0x80, 16),
            e41_dark: block41(c0, c1, 64, 0x00, den, num, 0, 0, 0x80, 0, 64),
            e41_image: block41(
                c0,
                c1,
                lines,
                g.b8,
                den,
                num,
                0x80,
                0x80,
                0x00,
                0xA0,
                g.lines_per_block,
            ),
            black: if base4800 {
                ((81, 0), 3634)
            } else {
                ((60, 0), 3596)
            },
            image_margin: crate::film_cal::image_margin(g.base, r8, c1 - c0),
            geometry: g,
            pass,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Epson's lead rule gives every film capture's lead: 600 dpi 128 (74, 99), 1200-4800 32
    /// (75, 76, 17, 18, 80, 100, 101, 106-126), and 128 at 1200 dpi with capture 127's 11,904 µs;
    /// a plan led again keeps its first line.
    #[test]
    fn the_image_lead_follows_the_line_time() {
        for (ydpi, t, lead) in [
            (600, 13_440, 128),
            (600, 18_432, 128),
            (1200, 13_440, 32),
            (1200, 13_824, 32),
            (1200, 18_432, 32),
            (1200, 16_512, 32),
            (1200, 11_904, 128),
            (1200, 12_288, 128),
            (2400, 12_672, 32),
            (2400, 24_576, 32),
            (5400, 13_440, 32),
        ] {
            assert_eq!(lead_for(ydpi, t), lead, "{ydpi} {t}");
        }
        // Capture 127's colour read: start 269 and lead 128, read 1832 lines.
        let r = FilmRequest {
            dpi: 1200,
            ydpi: 1200,
            x: 151,
            y: 419,
            width: 1188,
            height: 1784,
            bits: 16,
            film: Film::Negative,
        };
        let g = geometry(&r, XCorrection(-104)).unwrap();
        assert_eq!((g.start_steps, g.lead), (269 + 96, 32));
        let led = g.with_lead(128).unwrap();
        assert_eq!(
            (led.start_steps, led.lead, led.read_steps),
            (269, 128, 1832 * 2 + 128)
        );
        assert_eq!(led.start_steps + led.lead, g.start_steps + g.lead);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Every film request Epson Scan sent (its ICE pass, capture 18, aside), and the request it
    /// was made from.
    pub(crate) const CAPTURED: [(&str, FilmRequest); 14] = {
        #[allow(clippy::too_many_arguments)]
        const fn r(
            dpi: u32,
            ydpi: u32,
            x: u32,
            y: u32,
            width: u32,
            height: u32,
            bits: u8,
            film: Film,
        ) -> FilmRequest {
            FilmRequest {
                dpi,
                ydpi,
                x,
                y,
                width,
                height,
                bits,
                film,
            }
        }
        use Film::{Negative as N, Positive as P};
        [
            // 72, 73: the preview of a negative strip (Thumbnail off).
            (
                "2c0100002c01000000000000000000002a030000ef0a0000130801002004000000000000000100000000",
                r(300, 300, 0, 0, 810, 2799, 8, N),
            ),
            // 79 (and 16, 19 with Thumbnail): the preview of slides.
            (
                "2c0100002c01000000000000000000002a030000ef0a0000130801002004000000000000000000000000",
                r(300, 300, 0, 0, 810, 2799, 8, P),
            ),
            // 74, 75, 76: frame 1 of a negative strip at 600, 1200, 4800 dpi, 16 bits.
            (
                "58020000580200005d000000d1000000400200006a030000131001002004000000000000000100000000",
                r(600, 600, 93, 209, 576, 874, 16, N),
            ),
            (
                "b0040000b0040000ba000000a301000081040000d5060000131001002004000000000000000100000000",
                r(1200, 1200, 186, 419, 1153, 1749, 16, N),
            ),
            (
                "c012000018150000e90200006107000007120000c01e0000131001000904000000000000000100000000",
                r(4800, 5400, 745, 1889, 4615, 7872, 16, N),
            ),
            // 17, 77, 78: a negative at 2400 dpi, 16 and 8 bits.
            (
                "6009000060090000a7010000b803000020080000f70c0000131001001504000000000000000100000000",
                r(2400, 2400, 423, 952, 2080, 3319, 16, N),
            ),
            (
                "6009000060090000460100001249000003090000aa0d0000131001001204000000000000000100000000",
                r(2400, 2400, 326, 18706, 2307, 3498, 16, N),
            ),
            (
                "6009000060090000460100001249000004090000aa0d0000130801002004000000000000000100000000",
                r(2400, 2400, 326, 18706, 2308, 3498, 8, N),
            ),
            // 19, 80, 81: a slide at 2400 dpi, 16 bits.
            (
                "60090000600900005f06000047200000500c0000ff070000131001000d04000000000000000000000000",
                r(2400, 2400, 1631, 8263, 3152, 2047, 16, P),
            ),
            (
                "6009000060090000eb050000c00b0000490d0000db080000131001000c04000000000000000000000000",
                r(2400, 2400, 1515, 3008, 3401, 2267, 16, P),
            ),
            (
                "60090000600900001906000040480000490d0000db080000131001000c04000000000000000000000000",
                r(2400, 2400, 1561, 18496, 3401, 2267, 16, P),
            ),
            // 99, 100, 101: slide 1 at 600, 1200 and 4800 dpi, 16 bits.
            (
                "58020000580200009d010000e40200005203000036020000131001002004000000000000000000000000",
                r(600, 600, 413, 740, 850, 566, 16, P),
            ),
            (
                "b0040000b00400003b030000c9050000a40600006d040000131001001904000000000000000000000000",
                r(1200, 1200, 827, 1481, 1700, 1133, 16, P),
            ),
            (
                "c012000018150000ed0c0000091a0000931a0000ee130000131001000604000000000000000000000000",
                r(4800, 5400, 3309, 6665, 6803, 5102, 16, P),
            ),
        ]
    };

    #[test]
    fn every_captured_film_request_is_rebuilt() {
        for (captured, request) in CAPTURED {
            assert_eq!(
                request_block(&request).unwrap().to_vec(),
                hex(captured),
                "{request:?}"
            );
        }
    }

    #[test]
    fn the_preview_is_the_whole_window() {
        let preview = CAPTURED[0].1;
        let px = |inches: f64| (inches * 300.0).round() as u32;
        assert_eq!(
            (preview.width, preview.height),
            (px(ADAPTER_INCHES.0), px(ADAPTER_INCHES.1))
        );
    }

    /// The geometry of every film scan captured (17 and 74-81), as Epson Scan sent it: request,
    /// `1E 87`'s correction, then columns, words, lines, b8, ratio, lines per block, read steps
    /// and lead, start steps, `1E 11`, the line time and its read ramp.
    #[allow(clippy::type_complexity)]
    const SCANS: [(
        FilmRequest,
        i16,
        (u32, u32),
        (u16, u16),
        u32,
        u8,
        (u8, u8),
        u16,
        (u32, u32),
        u32,
        u8,
        u32,
        u16,
    ); 8] = {
        const fn r(
            dpi: u32,
            ydpi: u32,
            x: u32,
            y: u32,
            width: u32,
            height: u32,
            bits: u8,
        ) -> FilmRequest {
            FilmRequest {
                dpi,
                ydpi,
                x,
                y,
                width,
                height,
                bits,
                film: Film::Negative,
            }
        }
        [
            (
                r(2400, 2400, 423, 952, 2080, 3319, 16),
                -93,
                (2433, 6593),
                (1668, 582),
                3419,
                0x00,
                (1, 1),
                2,
                (3451, 32),
                479,
                8,
                13_056,
                816,
            ),
            (
                r(600, 600, 93, 209, 576, 874, 16),
                -122,
                (2379, 4683),
                (1190, 574),
                898,
                0x20,
                (1, 4),
                18,
                (3720, 128),
                267,
                9,
                13_440,
                210,
            ),
            (
                r(1200, 1200, 186, 419, 1153, 1749, 16),
                -122,
                (2379, 4715),
                (1198, 574),
                1797,
                0x10,
                (1, 2),
                9,
                (3626, 32),
                365,
                9,
                13_440,
                420,
            ),
            (
                r(4800, 5400, 745, 1889, 4615, 7872, 16),
                -122,
                (2423, 7047),
                (1781, 576),
                8100,
                0x00,
                (9, 4),
                2,
                (3632, 32),
                366,
                8,
                13_440,
                1890,
            ),
            (
                r(2400, 2400, 326, 18706, 2307, 3498, 16),
                -122,
                (2410, 7034),
                (1778, 572),
                3598,
                0x00,
                (1, 1),
                2,
                (3630, 32),
                18_233,
                8,
                13_824,
                864,
            ),
            (
                r(2400, 2400, 326, 18706, 2308, 3498, 8),
                -122,
                (2410, 7034),
                (1778, 572),
                3598,
                0x01,
                (1, 1),
                4,
                (3630, 32),
                18_233,
                8,
                13_824,
                864,
            ),
            (
                r(2400, 2400, 1515, 3008, 3401, 2267, 16),
                -122,
                (2788, 9604),
                (2421, 672),
                2367,
                0x00,
                (1, 1),
                1,
                (2399, 32),
                2535,
                8,
                24_576,
                1536,
            ),
            (
                r(2400, 2400, 1561, 18496, 3401, 2267, 16),
                -122,
                (2800, 9616),
                (2424, 676),
                2367,
                0x00,
                (1, 1),
                1,
                (2399, 32),
                18_023,
                8,
                16_512,
                1032,
            ),
        ]
    };

    #[test]
    fn every_captured_film_scan_has_its_geometry() {
        for (r, corr, columns, words, lines, b8, ratio, lpb, read, start, e11, t, ramp) in SCANS {
            let g = geometry(&r, XCorrection(corr)).unwrap();
            assert_eq!(g.columns, columns, "{r:?}");
            assert_eq!(g.words, words, "{r:?}");
            assert_eq!(g.lines, lines, "{r:?}");
            assert_eq!(g.b8, b8, "{r:?}");
            assert_eq!(g.ratio, ratio, "{r:?}");
            assert_eq!(g.lines_per_block, lpb, "{r:?}");
            assert_eq!((g.read_steps, g.lead), read, "{r:?}");
            assert_eq!(g.start_steps, start, "{r:?}");
            assert_eq!(g.e11, e11, "{r:?}");
            assert_eq!(g.read_ramp(t), ramp, "{r:?}");
        }
    }

    /// An area of the application's window is the film request Epson Scan sent for it: capture
    /// 74's frame (93, 209 in the window at 600 dpi) is (93, 65) for the application; at 4800 dpi
    /// the lines are read at 5400, enough for the height asked; the window's whole is inside.
    #[test]
    fn an_area_of_the_window_is_a_film_request() {
        let r = for_area(600, Film::Negative, (93, 65, 576, 874), 16);
        assert_eq!(r, CAPTURED[2].1);
        assert_eq!(
            for_area(1200, Film::Negative, (186, 131, 1153, 1749), 16),
            CAPTURED[3].1
        );
        let r = for_area(4800, Film::Negative, (745, 528, 4615, 6997), 16);
        assert_eq!((r.ydpi, r.y, r.height), (5400, 1890, 7872));
        assert!(r.height * 4800 / 5400 >= 6997);
        for dpi in [600, 1200, 2400, 4800] {
            let (w, h) = (
                (AREA_INCHES.0 * f64::from(dpi)).round() as u32,
                (AREA_INCHES.1 * f64::from(dpi)).round() as u32,
            );
            for film in [Film::Negative, Film::Positive] {
                let r = for_area(dpi, film, (0, 0, w, h), 16);
                crate::guard::check_film_request(&r).unwrap_or_else(|e| panic!("{dpi}: {e}"));
                geometry(&r, XCorrection(-122)).unwrap_or_else(|e| panic!("{dpi}: {e}"));
            }
        }
    }

    /// A frame whose start move would be shorter than its ramps (never captured) is refused;
    /// the least start move captured (267 steps, capture 74) is not.
    #[test]
    fn a_frame_too_near_the_windows_top_is_refused() {
        let mut r = CAPTURED[2].1;
        let c = XCorrection(-122);
        assert_eq!(geometry(&r, c).unwrap().start_steps, 267);
        // 600 dpi, lead 128: y x 4 + 583 - 128 must pass 1024.
        r.y = 142;
        assert!(geometry(&r, c).is_err());
        r.y = 143;
        assert_eq!(geometry(&r, c).unwrap().start_steps, 3);
    }

    #[test]
    fn the_shading_tables_are_as_wide_as_captured() {
        let widths = [
            24_960, 13_824, 14_016, 27_744, 27_744, 27_744, 40_896, 40_896,
        ];
        for ((r, corr, ..), w) in SCANS.iter().zip(widths) {
            assert_eq!(
                geometry(r, XCorrection(*corr)).unwrap().column_count() * 6,
                w
            );
        }
    }
    fn hexb(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Every `1E 41` of captures 74 (600 dpi), 77 (2400), 76 (4800 x 5400) and 80 (a slide).
    #[test]
    fn a_film_plans_reads_are_as_captured() {
        let cases: [(usize, i16, [&str; 6]); 4] = [
            (
                2,
                -122,
                [
                    "f00a700c9600000001010c0000000000808096000000",
                    "23005300040000000001040000000000800004000000",
                    "e4083422040000000101040000000000800004000000",
                    "4b094b12100000000001024000000000808010000000",
                    "4b094b12400000000001040000000000800040000000",
                    "4b094b1282030000200104808000000000a012000000",
                ],
            ),
            (
                6,
                -122,
                [
                    "f00a700c9600000001010c0000000000808096000000",
                    "4600a600040000000001010000000000800004000000",
                    "1009b03b040000000101010000000000800004000000",
                    "6a097a1b100000000001024000000000808010000000",
                    "6a097a1b400000000001010000000000800040000000",
                    "6a097a1b0e0e0000000101808000000000a002000000",
                ],
            ),
            (
                4,
                -122,
                [
                    "f00a700c9600000001010c0000000000808096000000",
                    "4600a600040000000009040000000000800004000000",
                    "1009b03b040000000109040000000000800004000000",
                    "7709871b100000000001024000000000808010000000",
                    "7709871b400000000009040000000000800040000000",
                    "7709871ba41f0000000904808000000000a002000000",
                ],
            ),
            (
                9,
                -122,
                [
                    "f00a700c9600000001010c0000000000808096000000",
                    "4600a600040000000001010000000000800004000000",
                    "1009b03b040000000101010000000000800004000000",
                    "e40a8425100000000001024000000000808010000000",
                    "e40a8425400000000001010000000000800040000000",
                    "e40a84253f090000000101808000000000a001000000",
                ],
            ),
        ];
        let requests = crate::film::tests::CAPTURED;
        for (i, corr, want) in cases {
            let p = FilmPlan::new(requests[i].1, XCorrection(corr)).unwrap();
            let got = [
                p.e41_origin,
                p.e41_black,
                p.e41_window,
                p.e41_white,
                p.e41_dark,
                p.e41_image,
            ];
            for (k, (g, w)) in got.iter().zip(want).enumerate() {
                assert_eq!(g.to_vec(), hexb(w), "{:?}, read {k}", requests[i].1);
            }
        }
    }

    #[test]
    fn the_film_ramp_is_the_interpreters() {
        let r = crate::setup::film_ramp();
        assert_eq!(r.len(), 512);
        assert_eq!(&r[..6], &[2500, 2239, 2045, 1895, 1774, 1673]);
        assert_eq!(r[511], 222);
        // Every value, summed, as in the captures' tables.
        assert_eq!(r.iter().map(|&v| u32::from(v)).sum::<u32>(), 209_590);
    }
}
