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
/// [`EXPOSURE_WINDOW`] and not `held` gets X x 230 / level, at least [`EXPOSURE_START`]; `None`
/// when every channel not held is inside (the loop is over). For a negative the second and
/// third channels are held while the first is found; for a slide none.
pub fn exposure_round(x: [u32; 3], level: [u32; 3], held: [bool; 3]) -> Option<[u32; 3]> {
    let mut next = x;
    let mut moved = false;
    for c in 0..3 {
        if held[c] || (EXPOSURE_WINDOW.0..=EXPOSURE_WINDOW.1).contains(&level[c]) {
            continue;
        }
        next[c] = (x[c] * EXPOSURE_TARGET / level[c].max(1)).max(EXPOSURE_START);
        moved = true;
    }
    moved.then_some(next)
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
    let m = margin(base);
    let t = round_up_384(m + x.iter().copied().max().unwrap_or(0));
    let e = x.map(|v| t - v);
    let mut b = [0u8; 26];
    b[..3].copy_from_slice(&t.to_le_bytes()[..3]);
    for c in 0..3 {
        b[3 + 3 * c..6 + 3 * c].copy_from_slice(&e[c].to_le_bytes()[..3]);
    }
    b[12] = 0x3F;
    let (w1, r9) = window_words(base);
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

    #[test]
    fn the_exposures_follow_the_captures() {
        let neg = [false, true, true];
        // 74: 3840 -> 13,182 on the first channel, then in the window.
        assert_eq!(
            exposure_round([3840; 3], [67, 147, 165], neg),
            Some([13_182, 3840, 3840])
        );
        assert_eq!(
            exposure_round([13_182, 3840, 3840], [223, 149, 168], neg),
            None
        );
        // 77: three rounds.
        assert_eq!(
            exposure_round([12_988, 3840, 3840], [219, 147, 168], neg),
            Some([13_640, 3840, 3840])
        );
        // 80, a slide: all three channels, then the first again.
        assert_eq!(
            exposure_round([3840; 3], [58, 160, 174], [false; 3]),
            Some([15_227, 5520, 5075])
        );
        assert_eq!(
            exposure_round([15_227, 5520, 5075], [216, 231, 230], [false; 3]),
            Some([16_213, 5520, 5075])
        );
        assert_eq!(
            exposure_round([16_213, 5520, 5075], [229, 230, 230], [false; 3]),
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
}
