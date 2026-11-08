// SPDX-License-Identifier: GPL-2.0-or-later
//! Images: what a scan holds and how its pixels are laid out.

use crate::convert;

/// What the image holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 8-bit RGB.
    Colour,
    /// 8-bit grey.
    Grey,
    /// Black and white, white from `255 − threshold` up.
    Lineart {
        /// The HP driver's `Threshold` (default
        /// [`convert::DEFAULT_THRESHOLD`]).
        threshold: u8,
    },
}

/// Pixel layout of an [`Image`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 3 bytes per pixel, red, green, blue.
    Rgb,
    /// 1 byte per pixel.
    Grey,
    /// 1 bit per pixel, first pixel in the high bit, lines padded to a
    /// byte, 1 = white.
    Lineart,
}

/// A finished image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Pixels per line.
    pub width: usize,
    /// Lines.
    pub height: usize,
    /// Resolution in dpi, both ways.
    pub dpi: u16,
    /// Pixel layout of `data`.
    pub format: Format,
    /// The pixels, line after line.
    pub data: Vec<u8>,
}

impl Image {
    /// The image turned clockwise by `quarter_turns` × 90°.
    pub fn rotated(&self, quarter_turns: u8) -> Image {
        let turns = quarter_turns % 4;
        if turns == 0 {
            return self.clone();
        }
        // Black and white is turned as 0/255 grey, then packed again.
        let (channels, data) = match self.format {
            Format::Rgb => (3, self.data.clone()),
            Format::Grey => (1, self.data.clone()),
            Format::Lineart => (1, lineart_to_grey(self)),
        };
        let (w, h) = (self.width, self.height);
        let (nw, nh) = if turns == 2 { (w, h) } else { (h, w) };
        let mut out = vec![0u8; data.len()];
        for y in 0..h {
            for x in 0..w {
                let (nx, ny) = match turns {
                    1 => (h - 1 - y, x),
                    2 => (w - 1 - x, h - 1 - y),
                    _ => (y, w - 1 - x),
                };
                let src = (y * w + x) * channels;
                let dst = (ny * nw + nx) * channels;
                out[dst..dst + channels].copy_from_slice(&data[src..src + channels]);
            }
        }
        let data = if self.format == Format::Lineart {
            convert::to_lineart(&out, nw, 127)
        } else {
            out
        };
        Image {
            width: nw,
            height: nh,
            data,
            ..*self
        }
    }

    /// The image as 8-bit RGB (grey and black and white expanded), for
    /// display.
    pub fn to_rgb(&self) -> Vec<u8> {
        match self.format {
            Format::Rgb => self.data.clone(),
            Format::Grey => self.data.iter().flat_map(|&g| [g, g, g]).collect(),
            Format::Lineart => {
                let stride = self.width.div_ceil(8);
                let mut out = Vec::with_capacity(self.width * self.height * 3);
                for line in self.data.chunks_exact(stride) {
                    for x in 0..self.width {
                        let v = if line[x / 8] & (0x80 >> (x % 8)) != 0 {
                            255
                        } else {
                            0
                        };
                        out.extend_from_slice(&[v, v, v]);
                    }
                }
                out
            }
        }
    }
}

/// Black and white as 8-bit grey, 0 or 255.
fn lineart_to_grey(image: &Image) -> Vec<u8> {
    let stride = image.width.div_ceil(8);
    image
        .data
        .chunks_exact(stride)
        .flat_map(|line| {
            (0..image.width).map(move |x| {
                if line[x / 8] & (0x80 >> (x % 8)) != 0 {
                    255
                } else {
                    0
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation() {
        // 3 x 2 grey: 1 2 3 / 4 5 6.
        let img = Image {
            width: 3,
            height: 2,
            dpi: 300,
            format: Format::Grey,
            data: vec![1, 2, 3, 4, 5, 6],
        };
        let r = img.rotated(1);
        assert_eq!(
            (r.width, r.height, r.data.clone()),
            (2, 3, vec![4, 1, 5, 2, 6, 3])
        );
        assert_eq!(img.rotated(2).data, [6, 5, 4, 3, 2, 1]);
        assert_eq!(img.rotated(3).data, [3, 6, 2, 5, 1, 4]);
        assert_eq!(r.rotated(3), img);
        let bw = Image {
            width: 3,
            height: 1,
            dpi: 300,
            format: Format::Lineart,
            data: vec![0b1000_0000],
        };
        assert_eq!(bw.rotated(1).data, [0b1000_0000, 0, 0]);
    }

    #[test]
    fn lineart_expands_for_display() {
        let img = Image {
            width: 3,
            height: 1,
            dpi: 300,
            format: Format::Lineart,
            data: vec![0b1010_0000],
        };
        assert_eq!(img.to_rgb(), [255, 255, 255, 0, 0, 0, 255, 255, 255]);
    }
}
