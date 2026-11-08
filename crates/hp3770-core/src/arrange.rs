// SPDX-License-Identifier: GPL-2.0-or-later
//! Lines arranged by the host ([`ArrangeLine::Software`]), and the doubled
//! width of a 2400 dpi scan.
//!
//! The three colour rows of the CCD (and, from 1200 dpi up, its even and
//! odd pixels) see different lines of the document at the same time. Below
//! 2400 dpi the chip delays them itself; at 2400 dpi the distances do not
//! fit in its registers, so it sends the lines as read, with as many more
//! as the colours are apart, and the host puts each sample back on its
//! line (`Arrange_Colour` and `Triplet_Colour_HRes` in hp3900).
//!
//! The sensor resolution is 1200 dpi: a 2400 dpi scan is 1200 pixels per
//! inch across, 2400 lines per inch down, and the HP driver doubles the
//! width on the host.
//!
//! [`ArrangeLine::Software`]: crate::setup::ArrangeLine::Software

use crate::profile::Profile;
use crate::setup::ScanParams;

/// How far apart, in lines of the scan, the samples of one output line
/// are in the chip's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Distances {
    /// Lines between two colour rows of the sensor.
    pub colour: usize,
    /// Lines between even and odd pixels (0 when the chip does not
    /// separate them).
    pub even_odd: usize,
    /// Which row each colour is on (red, green, blue).
    pub order: [u8; 3],
}

impl Distances {
    /// The distances of a scan with `params`, as hp3900 computes them.
    pub fn of(profile: &Profile, params: &ScanParams, high_resolution: bool) -> Self {
        let p = &profile.platform;
        let y = usize::from(params.resolution_y);
        let sensor = usize::from(p.sensor_resolution);
        Self {
            colour: usize::from(p.line_distance) * y / sensor,
            even_odd: if high_resolution {
                usize::from(p.even_odd_distance) * y / sensor
            } else {
                0
            },
            order: p.rgb_order,
        }
    }

    /// Lines the chip sends beyond the image.
    pub fn extra_lines(&self) -> usize {
        let last = usize::from(*self.order.iter().max().unwrap_or(&0));
        last * self.colour + self.even_odd
    }
}

/// Puts the samples of `raw` (8-bit RGB, `width` pixels per line, as the
/// chip sends them) back on their lines and returns `lines` lines.
///
/// The sample of colour `c` at pixel `p` of output line `n` is taken from
/// line `n + order[c] × colour (+ even_odd if p is odd)` of the chip's
/// data. Lines the data does not reach are left black.
pub fn arrange<T: Copy + Default>(raw: &[T], width: usize, lines: usize, d: &Distances) -> Vec<T> {
    let stride = width * 3;
    let mut out = vec![T::default(); stride * lines];
    for n in 0..lines {
        for p in 0..width {
            for c in 0..3 {
                let line = n
                    + usize::from(d.order[c]) * d.colour
                    + if p % 2 == 1 { d.even_odd } else { 0 };
                let src = line * stride + p * 3 + c;
                if let Some(&v) = raw.get(src) {
                    out[n * stride + p * 3 + c] = v;
                }
            }
        }
    }
    out
}

/// Doubles the width of an RGB image (8 or 16 bits): each pixel is
/// followed by the average of it and the next one (the last is repeated).
pub fn double_width<T: Copy + Into<u32> + TryFrom<u32>>(rgb: &[T], width: usize) -> Vec<T> {
    let stride = width * 3;
    let mut out = Vec::with_capacity(rgb.len() * 2);
    for line in rgb.chunks_exact(stride) {
        for p in 0..width {
            let here = &line[p * 3..p * 3 + 3];
            let next = if p + 1 < width {
                &line[p * 3 + 3..p * 3 + 6]
            } else {
                here
            };
            out.extend_from_slice(here);
            out.extend((0..3).map(|c| {
                let mean = (here[c].into() + next[c].into()).div_ceil(2);
                T::try_from(mean).unwrap_or(here[c])
            }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::HP3770;

    #[test]
    fn distances_of_capture_06b() {
        // 2400 dpi down, 1200 across, even and odd pixels apart: the chip
        // sent 105 lines more than the image (setup's extra lines, one more
        // than the samples need).
        let params = ScanParams {
            negative: false,
            depth: 8,
            resolution_x: 1200,
            resolution_y: 2400,
            left: 259,
            top: 746,
            width: 944,
            height: 1896,
        };
        let d = Distances::of(&HP3770, &params, true);
        assert_eq!((d.colour, d.even_odd), (48, 8));
        assert_eq!(d.extra_lines(), 104);
    }

    #[test]
    fn samples_go_back_on_their_line() {
        // 2 pixels, colours 1 line apart, odd pixels 1 line later: the
        // value of each sample is the output line it belongs to.
        let d = Distances {
            colour: 1,
            even_odd: 1,
            order: [0, 1, 2],
        };
        let lines = 3;
        let raw_lines = lines + d.extra_lines();
        let mut raw = vec![0u8; raw_lines * 6];
        for n in 0..lines {
            for p in 0..2 {
                for c in 0..3 {
                    let line = n + c + p;
                    raw[line * 6 + p * 3 + c] = n as u8 + 10;
                }
            }
        }
        let out = arrange(&raw, 2, lines, &d);
        for (n, line) in out.chunks(6).enumerate() {
            assert!(
                line.iter().all(|&v| v == n as u8 + 10),
                "line {n}: {line:?}"
            );
        }
    }

    #[test]
    fn width_doubles_with_averages() {
        let out = double_width(&[0u8, 10, 20, 100, 110, 121], 2);
        assert_eq!(out, [0, 10, 20, 50, 60, 71, 100, 110, 121, 100, 110, 121]);
    }
}
