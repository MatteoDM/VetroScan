// SPDX-License-Identifier: GPL-2.0-or-later
//! The shading the scanner applies, computed from our own calibration reads.
//!
//! Epson's driver reads the sensor's black pixels, a white strip with the lamp on and the same
//! with the lamp off, then loads the scanner with a table of two bytes per column and channel
//! (white and dark) and the bases they are relative to; the scanner then corrects every sample
//! itself. The formulas were found on six captures (`captures/epson4490/06-formato-dati.md`,
//! "Lo shading", outside the repository): the dark byte, the minimum dark and the scales exact,
//! the white byte about one unit off (the driver leaves out dusty lines by a rule not known).
//! Tried on the scanner with `epson4490 replay` on 12 November 2026: the same image as Epson's
//! driver's.

/// The `1E 42` block before the white calibration: zero, then each channel's mean of the
/// black pixels twice (`raw`: 16-bit RGB, possibly ending with the 8-byte block trailer).
pub fn black_block(raw: &[u8]) -> Vec<u8> {
    let n = raw.len();
    let raw = if n >= 8 && raw[n - 2..] == [0x80, 0x00] && raw[n - 8] == 0x02 {
        &raw[..n - 8]
    } else {
        raw
    };
    let mut sum = [0f64; 3];
    let mut count = 0usize;
    for p in raw.chunks_exact(6) {
        for (ch, s) in sum.iter_mut().enumerate() {
            *s += f64::from(u16::from_le_bytes([p[2 * ch], p[2 * ch + 1]]));
        }
        count += 1;
    }
    let mean: [u16; 3] = std::array::from_fn(|ch| (sum[ch] / count.max(1) as f64).floor() as u16);
    [
        0, 0, 0, mean[0], mean[0], mean[1], mean[1], mean[2], mean[2],
    ]
    .iter()
    .flat_map(|v: &u16| v.to_le_bytes())
    .collect()
}

/// The shading computed from a replay's own calibration (`captures/epson4490/06-formato-dati.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shading {
    /// The white's base per channel (`1E 42`).
    pub k: [u16; 3],
    /// The minimum dark per channel (`1E 42`); of the even columns where they differ.
    pub dark_min: [u16; 3],
    /// The odd columns' minimum dark: the same as `dark_min` but on a sensor whose two rows of
    /// columns have darks of their own ([`Shading::compute_by_parity`]).
    pub dark_min_odd: [u16; 3],
    /// The white's scale, 2^`white_shift` (`1E 41` byte 15, high nibble).
    pub white_shift: u8,
    /// The dark's scale, 2^`dark_shift` (low nibble).
    pub dark_shift: u8,
    /// The table: per column, per channel, `white byte << 8 | dark byte`.
    pub table: Vec<u16>,
}

/// The per-column statistics of a calibration read: 16-bit RGB, `lines` x `cols`.
fn columns(raw: &[u8], cols: usize) -> Vec<Vec<[u16; 3]>> {
    let line = cols * 6;
    raw.chunks_exact(line)
        .map(|l| {
            l.chunks_exact(6)
                .map(|p| {
                    [
                        u16::from_le_bytes([p[0], p[1]]),
                        u16::from_le_bytes([p[2], p[3]]),
                        u16::from_le_bytes([p[4], p[5]]),
                    ]
                })
                .collect()
        })
        .collect()
}

/// The smallest shift that brings `span` to 255 or less.
fn shift_for(span: f64) -> u8 {
    (0u8..16)
        .find(|&s| span / f64::from(1u32 << s) <= 255.0)
        .unwrap_or(15)
}

impl Shading {
    /// From the white and dark reads (16-bit RGB over `cols` columns), for a table of `len`
    /// values covering the first `width` columns.
    pub fn compute(white: &[u8], dark: &[u8], cols: usize, width: usize, len: usize) -> Self {
        Self::compute_with(white, dark, cols, width, len, false)
    }

    /// [`Shading::compute`] with the minimum dark of the even and the odd columns apart, each
    /// column's dark byte from its own: Epson Scan's films at base 4800 (captures 76-81: its
    /// `1E 42` carries two minimum darks a channel, 687 and 1650 in 77, and with them every dark
    /// byte but a few comes out as its table's). At base 2400 the two are the same.
    pub fn compute_by_parity(
        white: &[u8],
        dark: &[u8],
        cols: usize,
        width: usize,
        len: usize,
    ) -> Self {
        Self::compute_with(white, dark, cols, width, len, true)
    }

    fn compute_with(
        white: &[u8],
        dark: &[u8],
        cols: usize,
        width: usize,
        len: usize,
        by_parity: bool,
    ) -> Self {
        let w = columns(white, cols);
        let d = columns(dark, cols);
        let width = width.min(cols);
        // The white per column: the mean of its brightest lines (the driver leaves out the
        // lines darkened by dust on the strip; its exact rule is not known).
        let keep = w.len() - w.len() * 5 / 16;
        let mut white_col = vec![[0f64; 3]; width];
        let mut dark_col = vec![[0f64; 3]; width];
        for col in 0..width {
            for ch in 0..3 {
                let mut v: Vec<u16> = w.iter().map(|l| l[col][ch]).collect();
                v.sort_unstable();
                let top = &v[v.len() - keep..];
                white_col[col][ch] = top.iter().map(|&x| f64::from(x)).sum::<f64>() / keep as f64;
                dark_col[col][ch] =
                    d.iter().map(|l| f64::from(l[col][ch])).sum::<f64>() / d.len() as f64;
            }
        }
        let min = |v: &[[f64; 3]], ch: usize| v.iter().map(|c| c[ch]).fold(f64::MAX, f64::min);
        let max = |v: &[[f64; 3]], ch: usize| v.iter().map(|c| c[ch]).fold(f64::MIN, f64::max);
        let k: [u16; 3] = std::array::from_fn(|ch| min(&white_col, ch).floor() as u16);
        let min_of = |parity: usize, ch: usize| {
            dark_col
                .iter()
                .enumerate()
                .filter(|(i, _)| !by_parity || i % 2 == parity)
                .map(|(_, c)| c[ch])
                .fold(f64::MAX, f64::min)
                .floor() as u16
        };
        let dark_min: [u16; 3] = std::array::from_fn(|ch| min_of(0, ch));
        let dark_min_odd: [u16; 3] = std::array::from_fn(|ch| min_of(1, ch));
        let dark_of = |col: usize, ch: usize| {
            if col % 2 == 1 {
                dark_min_odd[ch]
            } else {
                dark_min[ch]
            }
        };
        let white_shift = (0..3)
            .map(|ch| shift_for(max(&white_col, ch) - f64::from(k[ch])))
            .max()
            .unwrap_or(0);
        let dark_shift = (0..width)
            .flat_map(|col| (0..3).map(move |ch| (col, ch)))
            .map(|(col, ch)| shift_for(dark_col[col][ch] - f64::from(dark_of(col, ch))))
            .max()
            .unwrap_or(0);
        let mut table = vec![0u16; len];
        for col in 0..width {
            for ch in 0..3 {
                let i = col * 3 + ch;
                if i >= len {
                    break;
                }
                let hi = ((white_col[col][ch] - f64::from(k[ch])) / f64::from(1u32 << white_shift))
                    .floor()
                    .clamp(0.0, 255.0) as u16;
                let lo = ((dark_col[col][ch] - f64::from(dark_of(col, ch)))
                    / f64::from(1u32 << dark_shift))
                .floor()
                .clamp(0.0, 255.0) as u16;
                table[i] = hi << 8 | lo;
            }
        }
        Self {
            k,
            dark_min,
            dark_min_odd,
            white_shift,
            dark_shift,
            table,
        }
    }

    /// The `1E 42` block: K per channel, then each channel's minimum dark of the even columns
    /// and of the odd (the same twice but for [`Shading::compute_by_parity`]).
    pub fn block42(&self) -> Vec<u8> {
        let [d0, d1, d2] = self.dark_min;
        let [o0, o1, o2] = self.dark_min_odd;
        [self.k[0], self.k[1], self.k[2], d0, o0, d1, o1, d2, o2]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }

    pub fn table_bytes(&self) -> Vec<u8> {
        self.table.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb16(pixels: &[[u16; 3]]) -> Vec<u8> {
        pixels
            .iter()
            .flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes()))
            .collect()
    }

    #[test]
    fn the_black_block_is_the_floor_of_each_channels_mean() {
        // Two pixels, then the 8-byte block trailer, left out.
        let mut raw = rgb16(&[[1136, 1238, 1330], [1139, 1239, 1331]]);
        raw.extend([0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x80, 0x00]);
        let want: Vec<u8> = [0u16, 0, 0, 1137, 1137, 1238, 1238, 1330, 1330]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(black_block(&raw), want);
    }

    #[test]
    fn the_shading_packs_white_and_dark_per_column() {
        // Two columns, 16 lines: white 1000 / 1000+255*128 (one line darkened by dust in the
        // second, left out), dark 100 / 100+4*10.
        let cols = 2;
        let mut white = Vec::new();
        let mut dark = Vec::new();
        for line in 0..16 {
            let dust = if line == 3 { 5000 } else { 0 };
            white.extend(rgb16(&[[1000; 3], [1000 + 255 * 128 - dust; 3]]));
            dark.extend(rgb16(&[[100; 3], [140; 3]]));
        }
        let s = Shading::compute(&white, &dark, cols, 2, 8);
        assert_eq!(s.k, [1000; 3]);
        assert_eq!(s.dark_min, [100; 3]);
        assert_eq!((s.white_shift, s.dark_shift), (7, 0));
        // Column 0: white 0, dark 0; column 1: white 255, dark 40; then zeros.
        assert_eq!(
            s.table,
            [0, 0, 0, 255 << 8 | 40, 255 << 8 | 40, 255 << 8 | 40, 0, 0]
        );
        assert_eq!(s.block42()[..6], [0xE8, 0x03, 0xE8, 0x03, 0xE8, 0x03]);
    }

    #[test]
    fn the_dark_by_parity_has_two_minimums() {
        // Two columns, the odd one darker by 960: by parity each is its own minimum.
        let mut white = Vec::new();
        let mut dark = Vec::new();
        for _ in 0..16 {
            white.extend(rgb16(&[[1000; 3], [1000; 3]]));
            dark.extend(rgb16(&[[700; 3], [1660; 3]]));
        }
        let s = Shading::compute_by_parity(&white, &dark, 2, 2, 6);
        assert_eq!((s.dark_min, s.dark_min_odd), ([700; 3], [1660; 3]));
        assert_eq!(s.dark_shift, 0);
        assert!(s.table.iter().all(|v| v & 0xFF == 0));
        let k = s.block42();
        assert_eq!(&k[6..10], &[0xBC, 0x02, 0x7C, 0x06]);
        // The glass's: one minimum, twice.
        let g = Shading::compute(&white, &dark, 2, 2, 6);
        assert_eq!((g.dark_min, g.dark_min_odd), ([700; 3], [700; 3]));
    }
}
