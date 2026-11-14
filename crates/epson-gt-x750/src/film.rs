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
    e57[28] = (BLOCK_BYTES / line).min(MAX_LINES_PER_BLOCK) as u8;
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
const BLOCK_MAX: u32 = 65_527;

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
    /// the staggered columns' (4 lines at 2400 along, 12 at 5400).
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
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// The geometry of a film request with `1E 87`'s x correction `corr`. The start column is the
/// glass's left-edge form (`0x40fdd0`: divisor 6 at base 4800, 3 at base 2400) from the logical
/// start (`FILM_ORIGIN` + `corr`) x base / 4800 + x; at base 2400 only an even correction was
/// captured (-122), so how an odd one rounds is not known.
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
    let d = 6 * base / 4800;
    let logical =
        (FILM_ORIGIN + i32::from(corr.0)) * base as i32 / 4800 + (r.x * base / r.dpi) as i32;
    let Ok(logical) = u32::try_from(logical) else {
        return bad("a film's start column before the scanner's first");
    };
    let r9 = ((logical.saturating_sub(80) / d) >> 3) * 2;
    let r8 = logical - 4 * d * r9 - 80;
    let c0 = r8 + 4 * r9 + 80;
    let w1 = (((r8 + count + 160) >> 1) + 2 * r9) >> 1;
    let g = gcd(r.ydpi, 2400);
    let (den, num) = (r.ydpi / g, 2400 / g);
    let stagger = match r.ydpi {
        2400 => 4,
        5400 => 12,
        _ => 0,
    };
    let lines = r.height + 2 * (r.ydpi / 50) + stagger;
    let line_bytes = pixels * 3 * u32::from(r.bits) / 8;
    let ratio_code = match base / read_dpi {
        1 => 0x00,
        2 => 0x10,
        4 => 0x20,
        8 => 0x50,
        _ => return bad("a film's read resolution with no ratio code"),
    };
    let lead = if r.dpi == 600 { 128 } else { 32 };
    let y_steps = r.y * 2400 / r.ydpi;
    Ok(Geometry {
        base,
        read_dpi,
        pixels,
        columns: (c0, c0 + count),
        words: (w1 as u16, r9 as u16),
        lines,
        b8: ratio_code | u8::from(r.bits == 8),
        ratio: (den as u8, num as u8),
        lines_per_block: (BLOCK_MAX / line_bytes) as u16,
        lead,
        read_steps: lines * 2400 / r.ydpi + lead,
        start_steps: (y_steps + START_K - lead).saturating_sub(RAMP_512_STEPS),
        e11: 8 + u8::from(base == 2400),
    })
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
}

impl FilmPlan {
    /// The plan of `request` with `1E 87`'s x correction `corr`.
    pub fn new(request: FilmRequest, corr: XCorrection) -> Result<Self> {
        use crate::setup::block41;
        let g = geometry(&request, corr)?;
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
        let mut e21_origin = [0u8; 26];
        e21_origin[..3].copy_from_slice(&6144u32.to_le_bytes()[..3]);
        e21_origin[12] = 0x2A;
        let r8 = c0 - 4 * u32::from(g.words.1) - 80;
        Ok(Self {
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
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Every film request Epson Scan sent (its ICE pass, capture 18, aside), and the request it
    /// was made from.
    pub(crate) const CAPTURED: [(&str, FilmRequest); 11] = {
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
