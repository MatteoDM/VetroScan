// SPDX-License-Identifier: GPL-2.0-or-later
//! The table a 1200 dpi scan needs after every power-on (`1E 84` type 07).
//!
//! At 1200 dpi Epson's driver asks `1E 8B` first. `06`: the scanner holds the table (it
//! keeps it until switched off). `15`: the driver reads a raw line of the sensor from the
//! scanner's memory (`1E 83`, 244,800 bytes at `7c3d60`), asks `1E 87` again, and writes
//! back 600 bytes (`1E 84` type 07 at `7ff9a0`): per colour, a list of the sensor elements
//! that stand out from their neighbours.
//!
//! The rule, read in Epson's interpreter (`esxuin54.dll` 1.27, `0x40f4f0`, under the EU
//! interoperability exception; the code here is ours) and checked on the captures (05, 11
//! November 2026, and 42, 12 November after switching on again: different tables, every
//! byte the same as Epson's driver's):
//!
//! 1. the raw line is 16-bit big-endian samples, six interleaved: red, green and blue of the
//!    even elements, then of the odd ones (the sensor's two staggered rows);
//! 2. for each colour, element `2 + 2k + row` gets how much it rises above the higher of its
//!    two neighbours on the same row (0 if it does not), for `k` in 0..20,398;
//! 3. a colour's list starts with its length and three positions, 4000, 19900 and 35800
//!    less the x origin's correction of `1E 87`; then, in order, every element rising 1260
//!    or more, then the 41 highest of the rest (taken from the 45 highest), at most 100
//!    entries, the unused ones `ffff`;
//! 4. the three lists, 100 u16 each, little-endian.

use crate::setup::XCorrection;

/// `1E 83`'s header for the raw line: type 07, address `7c3d60`, 244,800 bytes.
pub const READ_RAW: [u8; 8] = [0x07, 0x60, 0x3D, 0x7C, 0x00, 0x40, 0xBC, 0x03];
/// The raw line's length.
pub const RAW_LEN: usize = 0x3_BC40;
/// `1E 84`'s header for the table: type 07, address `7ff9a0`, flag 02, 600 bytes.
pub const HEADER: [u8; 8] = [0x07, 0xA0, 0xF9, 0x7F, 0x02, 0x58, 0x02, 0x00];
/// The table's length.
pub const LEN: usize = 600;

/// Elements per colour (both rows).
const ELEMENTS: usize = 0x9F60;
/// The neighbour comparisons per row and colour.
const STEPS: usize = 0x4FAE;
/// Entries per colour's list.
const ENTRIES: usize = 100;
/// How far above its neighbours an element is listed first.
const THRESHOLD: u16 = 0x4EC;

/// The table from the raw line (`raw`, as `1E 83` read it) and `1E 87`'s correction.
pub fn compute(raw: &[u8], x_correction: XCorrection) -> Vec<u8> {
    let sample = |i: usize| -> u16 {
        raw.get(2 * i..2 * i + 2)
            .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
    };
    let mut rise = [
        vec![0u16; ELEMENTS],
        vec![0u16; ELEMENTS],
        vec![0u16; ELEMENTS],
    ];
    for channel in 0..6 {
        for k in 0..STEPS {
            let left = sample(channel + 6 * k);
            let mid = sample(channel + 6 * k + 6);
            let right = sample(channel + 6 * k + 12);
            let up = mid.saturating_sub(left).max(mid.saturating_sub(right));
            rise[channel % 3][2 + 2 * k + channel / 3] = up;
        }
    }
    let c = i32::from(x_correction.0);
    let header = [4000 - c, 19_900 - c, 35_800 - c].map(|v| v as u16);
    // The colours do not mix: Epson's loops go through them in turn, the same.
    let mut lists = [[0xFFFFu16; ENTRIES]; 3];
    for (list, rise) in lists.iter_mut().zip(rise.iter_mut()) {
        list[1..4].copy_from_slice(&header);
        let mut count = 4usize;
        for (e, r) in rise.iter_mut().enumerate().skip(1) {
            if *r >= THRESHOLD && count < ENTRIES {
                list[count] = e as u16;
                count += 1;
                *r = 0;
            }
        }
        // The 45 highest left (the first of equals; element 0 when none rises), then 41
        // of them, as room allows.
        let mut highest = [0u16; 45];
        for h in &mut highest {
            let (mut best, mut at) = (0u16, 0usize);
            for (e, &r) in rise.iter().enumerate().skip(1) {
                if r > best {
                    (best, at) = (r, e);
                }
            }
            *h = at as u16;
            rise[at] = 0;
        }
        for &e in &highest[..41] {
            if count < ENTRIES {
                list[count] = e;
                count += 1;
            }
        }
        list[0] = count as u16;
    }
    lists
        .iter()
        .flat_map(|l| l.iter().flat_map(|v| v.to_le_bytes()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Capture 05 (11 November 2026): the raw line `1E 83` read and the table Epson's driver
    /// wrote, with `1E 87`'s -93.
    #[test]
    fn the_table_is_rebuilt_from_the_raw_line() {
        let raw = include_bytes!("../testdata/05_1e83_7c3d60.bin");
        let want = include_bytes!("../testdata/05_1e84_07.bin");
        assert_eq!(raw.len(), RAW_LEN);
        let got = compute(raw, XCorrection(-93));
        assert_eq!(got.len(), LEN);
        assert_eq!(got, want.to_vec());
        // The header of each colour: its length, then 4000, 19900, 35800 less the correction.
        assert_eq!(got[..8], [100, 0, 0xFD, 0x0F, 0x19, 0x4E, 0x35, 0x8C]);
    }

    #[test]
    fn a_flat_line_lists_only_its_header_and_the_highest() {
        // No element above its neighbours: the 41 "highest" are element 0, as Epson's.
        let got = compute(&vec![0x10; RAW_LEN], XCorrection(0));
        let v: Vec<u16> = got
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(v[..5], [45, 4000, 19_900, 35_800, 0]);
        assert_eq!(v[45], 0xFFFF);
        assert_eq!(v[100], 45);
    }
}
