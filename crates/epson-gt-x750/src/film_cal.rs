// SPDX-License-Identifier: GPL-2.0-or-later
//! What Epson Scan computes from a film's calibration reads, rebuilt from its captures (16-19,
//! 72-81) and its interpreter (`esxuin54.dll` 1.27, under the EU interoperability exception;
//! the addresses in `docs/architecture-plan.md`, phase 12 step 14), and checked by the model
//! `captures/epson4490/tools/filmsetup.py` (`--check-offsets`, `--check-exposure`,
//! `--check-gains`, outside the repository) on every read of every film capture: the offsets
//! (`1E 31` bytes 0, 2, 4), the exposures and line time of the window's reads (`1E 21`), the
//! negatives' final exposures, and their gains (`1E 31` bytes 8-10). Pure functions: what reads
//! to make and what to send is the scan's, and the guard's.

/// The black pixels' mean (16 bits) the offsets aim at: a channel inside is left as it is.
pub const BLACK_WINDOW: (f64, f64) = (1024.0, 1536.0);

/// The offsets' search: the first offsets sent, and the first step.
pub const OFFSET_START: u8 = 0x80;
const OFFSET_FIRST_STEP: u8 = 0x40;

/// One round of the offsets' search: from the offsets `v` read with `step` and the black
/// pixels' means, the offsets for the next read and the next step; no step when the search is
/// over (no channel moved, or the step `01` spent): then one more read with the offsets given
/// (Epson Scan's). Every channel is judged again at every read (capture 72: one inside at `88`,
/// outside at the next read, moved by `02`).
pub fn offsets_round(v: [u8; 3], step: u8, means: [f64; 3]) -> ([u8; 3], Option<u8>) {
    let mut next = v;
    let mut moved = false;
    for c in 0..3 {
        if means[c] > BLACK_WINDOW.1 {
            next[c] = next[c].wrapping_sub(step);
            moved = true;
        } else if means[c] < BLACK_WINDOW.0 {
            next[c] = next[c].wrapping_add(step);
            moved = true;
        }
    }
    (next, (moved && step > 1).then_some(step >> 1))
}

/// The first offsets' step.
pub fn offsets_first_step() -> u8 {
    OFFSET_FIRST_STEP
}

/// The exposure every channel starts with, µs (`0x42d28c`), and the least it may get.
pub const EXPOSURE_START: u32 = 3840;

/// The level (8 bits) the exposures and gains aim at, and the windows they accept.
pub const EXPOSURE_TARGET: u32 = 230;
/// A channel whose level is in here keeps its exposure.
pub const EXPOSURE_WINDOW: (u32, u32) = (222, 236);
/// The gains aim lower, and keep in a narrower window.
pub const GAIN_TARGET: u32 = 228;
/// A channel whose level is in here keeps its gain.
pub const GAIN_WINDOW: (u32, u32) = (222, 230);

/// A window read's level per channel: the sum of the lines' maxima over four lines, divided by
/// four (`0x427ca0` per line), from the read's 8-bit samples, three a pixel.
pub fn level(lines: &[&[u8]]) -> [u32; 3] {
    let mut sum = [0u32; 3];
    for line in lines {
        for (c, s) in sum.iter_mut().enumerate() {
            *s += u32::from(line.iter().skip(c).step_by(3).copied().max().unwrap_or(0));
        }
    }
    sum.map(|s| s / 4)
}

/// One round of the exposures' loop (`0x426210`, `0x4254d0`): a channel whose level is outside
/// [`EXPOSURE_WINDOW`] and not `held` gets X x 230 / level, at least [`EXPOSURE_START`] and at
/// most `cap` ([`exposure_cap`]); `None` when no channel moved: every channel not held is
/// inside, or stays at its limit (the loop is over). For a negative the second and third
/// channels are held while the first is found; for a slide none.
pub fn exposure_round(x: [u32; 3], level: [u32; 3], held: [bool; 3], cap: u32) -> Option<[u32; 3]> {
    let mut next = x;
    let mut moved = false;
    for c in 0..3 {
        if held[c] || (EXPOSURE_WINDOW.0..=EXPOSURE_WINDOW.1).contains(&level[c]) {
            continue;
        }
        next[c] = (x[c] * EXPOSURE_TARGET / level[c].max(1))
            .clamp(EXPOSURE_START, cap.max(EXPOSURE_START));
        moved |= next[c] != x[c];
    }
    moved.then_some(next)
}

/// The longest exposure a window read may ask at `base`: its line time (the margin + the
/// longest exposure, [`window_block21`]) kept within the longest Epson Scan was captured
/// sending (`crate::guard::FILM_LINE_TIMES`: 27,264 µs, a slide at 4800 dpi, capture 101, its
/// red at 18,078 µs). Our first slide (capture 94) asked more than the 25,344 µs then captured
/// and was stopped by the guard; Epson Scan's own limit is in a table of its interpreter not read
/// (`0x40c820`): ours keeps the scanner inside what was captured, a channel at the limit a little
/// under its level.
pub fn exposure_cap(base: u32) -> u32 {
    crate::guard::FILM_LINE_TIMES.end() - margin(base)
}

/// The constant 110 µs the interpreter takes off an exposure before it rounds it, and adds back.
const EXPOSURE_OFFSET: u32 = 110;

/// A line time: rounded up to a multiple of 384 µs, as every one Epson Scan sent.
pub fn round_up_384(t: u32) -> u32 {
    t.div_ceil(384) * 384
}

/// A negative's final exposures from the first channel's (`0x423180`): less 110 µs, rounded up
/// to 384, then that, 10/15 and 10/23 of it, each plus 110 (captures 17, 74-77: 13,182 gives
/// 13,550, 9070, 5953). Also the image's line time: the rounded value (13,440 there).
pub fn negative_exposures(first: u32) -> ([u32; 3], u32) {
    let r = round_up_384(first.saturating_sub(EXPOSURE_OFFSET));
    (
        [r, r * 10 / 15, r * 10 / 23].map(|v| v + EXPOSURE_OFFSET),
        r,
    )
}

/// The margin M of the window's reads (`0x422dd0`): the line time is M + the longest exposure,
/// rounded up to 384 µs. From the interpreter's table (`0x40ca20`, per base: C 41,980 or 21,100)
/// and the window read's columns (12,960 or 6480 from its start r8 40 or 4): 8981 at base 4800,
/// 5731 at base 2400.
pub fn margin(base: u32) -> u32 {
    let (c, x, r) = if base == 4800 {
        (41_980, 40 + 12_960 + 160, 6)
    } else {
        (21_100, 4 + 6480 + 160, 3)
    };
    (((c - x) / r) >> 1) + (x >> 1)
}

/// The window's `1E 21` words at bytes 13-16 (the window from the film origin, no correction):
/// (2209, 548) at base 2400, (3840, 550) at base 4800.
pub fn window_words(base: u32) -> (u16, u16) {
    if base == 4800 {
        (3840, 550)
    } else {
        (2209, 548)
    }
}

/// The `1E 21` of a window read with exposures `x` (µs a channel): T = M + the longest,
/// rounded up to 384; each channel's value T - X; byte 12 `3f`; the window's words; then
/// (M + 254) / 2 in the loop, or the least value / 2 once a negative's final exposures are set
/// (`final_negative`), and the greatest value / 2; `last` the block's last byte.
pub fn window_block21(base: u32, x: [u32; 3], final_negative: bool, last: u8) -> [u8; 26] {
    exposure_block21(margin(base), window_words(base), x, final_negative, last)
}

/// [`window_block21`] for any read: its margin `m` and its words (the black pixels' reads:
/// [`crate::film::FilmPlan::black`]).
pub fn exposure_block21(
    m: u32,
    words: (u16, u16),
    x: [u32; 3],
    final_negative: bool,
    last: u8,
) -> [u8; 26] {
    let t = round_up_384(m + x.iter().copied().max().unwrap_or(0));
    let e = x.map(|v| t - v);
    let mut b = [0u8; 26];
    b[..3].copy_from_slice(&t.to_le_bytes()[..3]);
    for c in 0..3 {
        b[3 + 3 * c..6 + 3 * c].copy_from_slice(&e[c].to_le_bytes()[..3]);
    }
    b[12] = 0x3F;
    let (w1, r9) = words;
    let w3 = if final_negative {
        e.iter().copied().filter(|&v| v > 0).min().unwrap_or(0) >> 1
    } else {
        (m + 254) >> 1
    };
    let w4 = e.iter().copied().max().unwrap_or(0) >> 1;
    for (i, w) in [w1, r9, w3 as u16, w4 as u16].iter().enumerate() {
        b[13 + 2 * i..15 + 2 * i].copy_from_slice(&w.to_le_bytes());
    }
    b[25] = last;
    b
}

/// The gains' table (the interpreter's at `0x401180`): `1E 31` bytes 8-10 are indices into it.
pub const GAIN_TABLE: [u16; 64] = [
    966, 991, 1016, 1042, 1069, 1096, 1124, 1152, 1181, 1211, 1242, 1274, 1306, 1339, 1373, 1409,
    1445, 1482, 1520, 1559, 1599, 1641, 1684, 1728, 1774, 1821, 1869, 1919, 1971, 2025, 2080, 2138,
    2197, 2259, 2323, 2389, 2458, 2530, 2604, 2682, 2763, 2847, 2935, 3027, 3123, 3223, 3329, 3439,
    3556, 3678, 3806, 3942, 4085, 4237, 4397, 4567, 4748, 4941, 5147, 5367, 5603, 5856, 6129, 6424,
];

/// The negatives' gains before their search (`1E 31` bytes 8-10, captures 17, 72-78).
pub const NEGATIVE_GAINS: [u8; 3] = [0x22, 0x1B, 0x1B];

/// One round of the negatives' gains: a channel whose level is in [`GAIN_WINDOW`] keeps its
/// index, any other goes to the highest index whose value is at most value x 228 / level;
/// `None` when none moved (the search is over).
pub fn gains_round(g: [u8; 3], level: [u32; 3]) -> Option<[u8; 3]> {
    let mut next = g;
    for c in 0..3 {
        if (GAIN_WINDOW.0..=GAIN_WINDOW.1).contains(&level[c]) {
            continue;
        }
        let v = u32::from(GAIN_TABLE[usize::from(g[c]).min(63)]) * GAIN_TARGET / level[c].max(1);
        next[c] = GAIN_TABLE
            .iter()
            .rposition(|&t| u32::from(t) <= v)
            .unwrap_or(0) as u8;
    }
    (next != g).then_some(next)
}

/// The margin of a read of the image's columns (`0x422dd0` with them): as [`margin`], X the
/// read's start r8 + its columns + 160 (74: 4338 at base 2400; 77: 5509 at base 4800).
pub fn image_margin(base: u32, r8: u32, columns: u32) -> u32 {
    let (c, r) = if base == 4800 {
        (41_980, 6)
    } else {
        (21_100, 3)
    };
    let x = r8 + columns + 160;
    (((c - x) / r) >> 1) + (x >> 1)
}

/// A `1E 21` block from its parts.
fn block21(t: u32, e: [u32; 3], b12: u8, words: [u16; 4], last: u8) -> [u8; 26] {
    let mut b = [0u8; 26];
    b[..3].copy_from_slice(&t.to_le_bytes()[..3]);
    for c in 0..3 {
        b[3 + 3 * c..6 + 3 * c].copy_from_slice(&e[c].to_le_bytes()[..3]);
    }
    b[12] = b12;
    for (i, w) in words.iter().enumerate() {
        b[13 + 2 * i..15 + 2 * i].copy_from_slice(&w.to_le_bytes());
    }
    b[25] = last;
    b
}

/// The white read's `1E 21` (`0x425c20`), from the final exposures `x`, the image's words and
/// the image's margin ([`image_margin`]): when the longest exposure exceeds the next by at least
/// that margin, T is it rounded up to 384 and that channel integrates the whole line (its value
/// 0, byte 12 `3e`, `3b` or `2f` for the first, second or third), the others T - X; otherwise
/// the last window read's block (`last_window`) with the image's words. Then the least value
/// (not zero) / 2 and the greatest / 2. All eight film scans captured.
pub fn white_block21(
    x: [u32; 3],
    words: (u16, u16),
    image_margin: u32,
    last_window: &[u8; 26],
    last: u8,
) -> [u8; 26] {
    let mut order = [0usize, 1, 2];
    order.sort_by_key(|&c| x[c]);
    let (longest, second) = (order[2], x[order[1]]);
    if x[longest] >= second + image_margin {
        let t = round_up_384(x[longest]);
        let mut e = x.map(|v| t - v);
        e[longest] = 0;
        let b12 = [0x3E, 0x3B, 0x2F][longest];
        let least = e.iter().copied().filter(|&v| v > 0).min().unwrap_or(0);
        let most = e.iter().copied().max().unwrap_or(0);
        block21(
            t,
            e,
            b12,
            [words.0, words.1, (least >> 1) as u16, (most >> 1) as u16],
            last,
        )
    } else {
        let mut b = *last_window;
        b[13..15].copy_from_slice(&words.0.to_le_bytes());
        b[15..17].copy_from_slice(&words.1.to_le_bytes());
        b[25] = last;
        b
    }
}

/// A negative's dark and image reads' `1E 21`: every channel the whole line T (the image's,
/// [`negative_exposures`]), byte 12 `2a`, the image's words, then zeros.
pub fn negative_image_block21(t: u32, words: (u16, u16), last: u8) -> [u8; 26] {
    block21(t, [0; 3], 0x2A, [words.0, words.1, 0, 0], last)
}

/// A slide's dark and image reads' `1E 21`: the white's form ([`white_block21`]'s first) with
/// the exposures scaled by `factor` tenths around 110 µs, (X - 110) x factor / 10 + 110. The
/// factor is Epson Scan's own choice (15 for capture 80, 10 for 81; from its preview, it seems):
/// 10 keeps the white's.
pub fn slide_image_exposures(x: [u32; 3], factor: u32) -> [u32; 3] {
    x.map(|v| (v - EXPOSURE_OFFSET) * factor / 10 + EXPOSURE_OFFSET)
}

/// A negative's image gains from its calibration gains (`0x424320`): the first kept, the second
/// to the highest index whose value is at most value x 2000 / 1500, the third x 3000 / 2300
/// (74: `21 09 16` gives `21 14 1f`).
pub fn negative_image_gains(g: [u8; 3]) -> [u8; 3] {
    let up = |g: u8, num: u32, den: u32| {
        let v = u32::from(GAIN_TABLE[usize::from(g).min(63)]) * num / den;
        GAIN_TABLE
            .iter()
            .rposition(|&t| u32::from(t) <= v)
            .unwrap_or(0) as u8
    };
    [g[0], up(g[1], 2000, 1500), up(g[2], 3000, 2300)]
}

/// The slides' gains (`1E 31` bytes 8-10, captures 79-81): no search, these throughout.
pub const SLIDE_GAINS: [u8; 3] = [0x18, 0x1B, 0x18];

/// The lid unit's origin read: 150 lines of 384 pixels, 8 bits, three a pixel.
pub const ORIGIN_READ: (usize, usize) = (150, 384);

/// What `1E 8A` carries when the origin read finds no edge (`0x420fe0`): the default move.
pub const ORIGIN_DEFAULT: u32 = 0x436;

/// Each line's level in the lid unit's origin read ([`origin`]): its second channel's sum x 2/3
/// >> 8, the mean. `None` for a read too short.
pub fn origin_levels(read: &[u8]) -> Option<Vec<u32>> {
    let (lines, pixels) = ORIGIN_READ;
    if read.len() < lines * pixels * 3 {
        return None;
    }
    Some(
        read.chunks_exact(pixels * 3)
            .take(lines)
            .map(|l| {
                let sum: u64 = l.iter().skip(1).step_by(3).map(|&v| u64::from(v)).sum();
                (((sum * 0xAAAA_AAAB) >> 32) >> 8) as u32
            })
            .collect(),
    )
}

/// The lid unit's origin from its 150-line read (`0x420fe0`; all 14 film captures): each line's
/// level is its second channel's sum x 2/3 >> 8 (the mean); the threshold 5 x the highest level
/// / 10; after more than five lines above it, the first line below it is the edge; `1E 8A` is
/// (150 - edge + 35) x 12 + 256, and the move back that many steps less 1024 (a 512-value ramp's).
/// `None` when no edge is found (Epson Scan then moves by [`ORIGIN_DEFAULT`]).
pub fn origin(read: &[u8]) -> Option<u32> {
    origin_after(read, 35)
}

/// [`origin`] for Digital ICE's infrared pass: 31 lines in place of 35 (`0x42135d`, source
/// `03`; captures 108, 109 and 112: 228, 228 and 204 steps back).
pub fn infrared_origin(read: &[u8]) -> Option<u32> {
    origin_after(read, 31)
}

fn origin_after(read: &[u8], k: u32) -> Option<u32> {
    let lines = ORIGIN_READ.0;
    let levels = origin_levels(read)?;
    let threshold = 5 * levels.iter().map(|v| v / 10).max().unwrap_or(0);
    let mut above = 0;
    let mut edge = 0;
    for (i, &v) in levels.iter().enumerate() {
        if v > threshold {
            above += 1;
        }
        if above > 5 {
            edge = i;
            if v < threshold {
                break;
            }
        }
    }
    (edge > 0).then(|| (lines as u32 - edge as u32 + k) * 12 + 256)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Capture 72's black reads (the preview): the offsets sent and the means read, to the
    /// search's last read.
    #[test]
    fn the_offsets_search_follows_capture_72() {
        let reads: [([u8; 3], [f64; 3]); 7] = [
            ([0x80, 0x80, 0x80], [250.6, 310.5, 3.7]),
            ([0xC0, 0xC0, 0xC0], [6503.3, 5443.2, 5100.9]),
            ([0xA0, 0xA0, 0xA0], [3370.0, 2873.0, 2492.6]),
            ([0x90, 0x90, 0x90], [1808.8, 1615.2, 1233.6]),
            ([0x88, 0x88, 0x90], [1026.0, 956.3, 1236.7]),
            ([0x88, 0x8C, 0x90], [1021.5, 1274.7, 1238.5]),
            ([0x8A, 0x8C, 0x90], [1209.7, 1275.7, 1233.2]),
        ];
        let (mut v, mut step) = ([OFFSET_START; 3], offsets_first_step());
        for (k, (sent, means)) in reads.iter().enumerate() {
            assert_eq!(v, *sent, "read {k}");
            let (n, s) = offsets_round(v, step, *means);
            v = n;
            match s {
                Some(s) => step = s,
                None => {
                    assert_eq!(k, reads.len() - 1, "ended early");
                    // The read after the search: with what it found.
                    assert_eq!(v, [0x8A, 0x8C, 0x90]);
                    return;
                }
            }
        }
        panic!("the search did not end");
    }

    /// A channel that would need a line time longer than any captured stops at its limit, and
    /// the loop ends when nothing more moves.
    #[test]
    fn an_exposure_stops_at_the_longest_line_time_captured() {
        let cap = exposure_cap(4800);
        assert_eq!(cap, 18_283);
        assert_eq!(cap + margin(4800), 27_264);
        assert_eq!(exposure_cap(2400) + margin(2400), 27_264);
        let x = [18_000, 5927, 5661];
        assert_eq!(
            exposure_round(x, [220, 230, 228], [false; 3], cap),
            Some([18_283, 5927, 5661])
        );
        assert_eq!(
            exposure_round([18_283, 5927, 5661], [219, 230, 228], [false; 3], cap),
            None
        );
        // Its line time is the longest captured.
        let b = window_block21(4800, [18_283, 5927, 5661], false, 2);
        assert_eq!(u32::from_le_bytes([b[0], b[1], b[2], 0]), 27_264);
    }

    #[test]
    fn the_exposures_follow_the_captures() {
        let neg = [false, true, true];
        // Never reached in Epson Scan's captures: the same at any limit above them.
        let cap = exposure_cap(4800);
        // 74: 3840 -> 13,182 on the first channel, then in the window.
        assert_eq!(
            exposure_round([3840; 3], [67, 147, 165], neg, cap),
            Some([13_182, 3840, 3840])
        );
        assert_eq!(
            exposure_round([13_182, 3840, 3840], [223, 149, 168], neg, cap),
            None
        );
        // 77: three rounds.
        assert_eq!(
            exposure_round([12_988, 3840, 3840], [219, 147, 168], neg, cap),
            Some([13_640, 3840, 3840])
        );
        // 80, a slide: all three channels, then the first again.
        assert_eq!(
            exposure_round([3840; 3], [58, 160, 174], [false; 3], cap),
            Some([15_227, 5520, 5075])
        );
        assert_eq!(
            exposure_round([15_227, 5520, 5075], [216, 231, 230], [false; 3], cap),
            Some([16_213, 5520, 5075])
        );
        assert_eq!(
            exposure_round([16_213, 5520, 5075], [229, 230, 230], [false; 3], cap),
            None
        );
    }

    #[test]
    fn a_negatives_final_exposures_are_as_captured() {
        assert_eq!(negative_exposures(13_182), ([13_550, 9070, 5953], 13_440));
        assert_eq!(negative_exposures(13_442), ([13_550, 9070, 5953], 13_440));
        assert_eq!(negative_exposures(13_640), ([13_934, 9326, 6120], 13_824));
        assert_eq!(negative_exposures(13_123), ([13_166, 8814, 5786], 13_056));
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Window reads' `1E 21` as Epson Scan sent them (74; 76 at base 4800, its first).
    #[test]
    fn the_window_reads_block21_is_as_captured() {
        assert_eq!(margin(2400), 5731);
        assert_eq!(margin(4800), 8981);
        let cases: [(u32, [u32; 3], bool, u8, &str); 3] = [
            (
                2400,
                [3840; 3],
                false,
                2,
                "8025008016008016008016003fa1082402b00b400b0000000002",
            ),
            (
                2400,
                [13_182, 3840, 3840],
                false,
                2,
                "004b00821700003c00003c003fa1082402b00b001e0000000002",
            ),
            (
                2400,
                [13_550, 9070, 5953],
                true,
                2,
                "804c009217001229003f35003fa1082402c90b9f1a0000000002",
            ),
        ];
        for (base, x, fin, last, want) in cases {
            assert_eq!(
                window_block21(base, x, fin, last).to_vec(),
                hex(want),
                "{x:?}"
            );
        }
        // 76's first, at base 4800: T 13,056, values 9216, words 3840, 550, 4617, 4608.
        let got = window_block21(4800, [3840; 3], false, 2);
        assert_eq!(u32::from_le_bytes([got[0], got[1], got[2], 0]), 13_056);
        assert_eq!(u32::from_le_bytes([got[3], got[4], got[5], 0]), 9216);
        let w = |i: usize| u16::from_le_bytes([got[13 + 2 * i], got[14 + 2 * i]]);
        assert_eq!([w(0), w(1), w(2), w(3)], [3840, 550, 4617, 4608]);
    }

    #[test]
    fn the_gains_follow_capture_77() {
        let rounds: [([u8; 3], [u32; 3]); 5] = [
            ([34, 27, 27], [235, 255, 255]),
            ([32, 22, 22], [223, 255, 235]),
            ([32, 17, 20], [222, 255, 224]),
            ([32, 12, 20], [223, 245, 224]),
            ([32, 9, 20], [223, 227, 223]),
        ];
        for w in rounds.windows(2) {
            assert_eq!(gains_round(w[0].0, w[0].1), Some(w[1].0));
        }
        assert_eq!(gains_round(rounds[4].0, rounds[4].1), None);
        assert_eq!(NEGATIVE_GAINS, [34, 27, 27]);
    }

    #[test]
    fn the_level_is_the_mean_of_the_lines_maxima() {
        let a = [10u8, 200, 30, 12, 100, 40];
        let b = [11u8, 90, 33, 13, 95, 50];
        let lines: [&[u8]; 4] = [&a, &b, &a, &b];
        assert_eq!(
            level(&lines),
            [
                (12 + 13 + 12 + 13) / 4,
                (200 + 95 + 200 + 95) / 4,
                (40 + 50 + 40 + 50) / 4
            ]
        );
    }
    #[test]
    fn the_white_dark_and_image_blocks_are_as_captured() {
        // 74 (base 2400): the image's margin 4338; the white in the "3e" form.
        assert_eq!(image_margin(2400, 3, 2304), 4338);
        let w = white_block21([13_550, 9070, 5953], (1190, 574), 4338, &[0; 26], 6);
        assert_eq!(
            w.to_vec(),
            hex("003600000000921200bf1e003ea6043e0249095f0f0000000006")
        );
        assert_eq!(
            negative_image_block21(13_440, (1190, 574), 2).to_vec(),
            hex("8034000000000000000000002aa6043e02000000000000000002")
        );
        assert_eq!(negative_image_gains([0x21, 0x09, 0x16]), [0x21, 0x14, 0x1F]);
        // 77 (base 4800): the image's margin 5509 is too much: the last window block, the
        // image's words.
        assert_eq!(image_margin(4800, 42, 4624), 5509);
        let window = window_block21(4800, [13_934, 9326, 6120], true, 2);
        let w = white_block21([13_934, 9326, 6120], (1778, 572), 5509, &window, 6);
        assert_eq!(u32::from_le_bytes([w[0], w[1], w[2], 0]), 23_040);
        assert_eq!(w[12], 0x3F);
        assert_eq!(&w[13..17], &[0xF2, 0x06, 0x3C, 0x02]);
        // 80, a slide: the white, then the image with Epson Scan's factor 15.
        let x = [16_213, 5520, 5075];
        let w = white_block21(x, (2421, 672), image_margin(4800, 20, 6816), &[0; 26], 6);
        assert_eq!(u32::from_le_bytes([w[0], w[1], w[2], 0]), 16_512);
        let img = slide_image_exposures(x, 15);
        let i = white_block21(img, (2421, 672), image_margin(4800, 20, 6816), &[0; 26], 6);
        assert_eq!(
            i.to_vec(),
            hex("006000000000df3f007b42003e7509a002ef1f3d210000000006")
        );
    }
    /// A made-up origin read: the light from line 20 to line 100 (second channel 200, else 20):
    /// the edge at 100, so (150 - 100 + 35) x 12 + 256 = 1276, as capture 75's.
    #[test]
    fn the_lid_units_origin_is_the_lights_edge() {
        let (lines, pixels) = ORIGIN_READ;
        let mut read = Vec::with_capacity(lines * pixels * 3);
        for line in 0..lines {
            let g = if (20..100).contains(&line) { 200 } else { 20 };
            for _ in 0..pixels {
                read.extend([30, g, 40]);
            }
        }
        assert_eq!(origin(&read), Some(1276));
        assert_eq!(origin(&vec![0; lines * pixels * 3]), None);
    }
}
