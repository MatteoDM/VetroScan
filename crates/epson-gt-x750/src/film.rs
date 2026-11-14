// SPDX-License-Identifier: GPL-2.0-or-later
//! Films in the lid's transparency unit: the request, as Epson Scan sends it.
//!
//! Read from Epson Scan's captures of films (16-19, 72-81; `captures/epson4490/05-catture.md`,
//! outside the repository): the strips' and the slides' holders share one window,
//! [`ADAPTER_INCHES`] (Epson Scan's preview of either asks 810 x 2799 pixels at 300 dpi from
//! `(0, 0)`); a film's `1E 57` is Epson Scan's form ([`request_block`]), rebuilt byte for byte
//! by the tests from every film capture. What may be sent is [`crate::guard`]'s.

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
}
