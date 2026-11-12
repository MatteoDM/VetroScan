// SPDX-License-Identifier: GPL-2.0-or-later
//! Grey and black-and-white images from the chip's colour data.
//!
//! The chip always sends colour. For a grey scan the HP driver makes it
//! send each channel at a third of its level (through the white shading
//! table) and adds the three up (capture 07): the grey level is the mean of
//! the linear red, green and blue samples. The tone curve
//! of the scanner is applied after that.
//!
//! Black and white (capture 08) is that grey cut at a threshold: comparing
//! the HP driver's grey (07) and black-and-white (08) images of the same
//! sheet, half the pixels turn white at grey 196-200, and the WIA property
//! `Threshold` was 55: a pixel is white from `255 − threshold` up.

/// Default of the HP driver's `Threshold` property.
pub const DEFAULT_THRESHOLD: u8 = 55;

/// Sharpening that gives the edges of the HP driver's images (its WIA
/// property `Sharpen` = 3). Measured on the same 5 × 5 cm scanned by both
/// drivers: the steepest 1 % of the steps between neighbouring pixels,
/// over the range of the image, is 0.81 across and 0.73 down for the HP
/// driver, 0.53 and 0.48 for this one without sharpening, 0.81 and 0.72
/// with this amount.
pub const HP_SHARPEN: f32 = 1.5;

/// Sharpens an image with `channels` bytes per pixel (unsharp mask: each
/// sample moves away from the mean of its 3 × 3 neighbourhood by `amount`
/// times the difference). The outermost pixels are left as they are.
pub fn sharpen(data: &mut [u8], width: usize, channels: usize, amount: f32) {
    let height = data.len() / (width * channels).max(1);
    if amount == 0.0 || width < 3 || height < 3 {
        return;
    }
    let src = data.to_vec();
    let stride = width * channels;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            for c in 0..channels {
                let i = y * stride + x * channels + c;
                let mut sum = 0u32;
                for row in [i - stride, i, i + stride] {
                    for j in [row - channels, row, row + channels] {
                        sum += u32::from(src[j]);
                    }
                }
                let v = f32::from(src[i]);
                let out = v + amount * (v - sum as f32 / 9.0);
                data[i] = out.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Grey levels (one byte per pixel) from 8-bit linear RGB, through `tone`.
pub fn to_grey(rgb: &[u8], tone: &[u8; 256]) -> Vec<u8> {
    rgb.chunks_exact(3)
        .map(|p| {
            let sum = u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2]);
            tone[usize::from((sum + 1) / 3)]
        })
        .collect()
}

/// Black and white from grey levels, packed 8 pixels per byte, first pixel
/// in the high bit, each line padded to a whole byte; 1 is white (as the
/// PNG format has it for a 1-bit grey image).
pub fn to_lineart(grey: &[u8], width: usize, threshold: u8) -> Vec<u8> {
    if width == 0 {
        return Vec::new();
    }
    let cut = 255 - threshold;
    let stride = width.div_ceil(8);
    let mut out = Vec::with_capacity(stride * grey.len() / width.max(1));
    for line in grey.chunks_exact(width) {
        let mut row = vec![0u8; stride];
        for (x, &g) in line.iter().enumerate() {
            if g >= cut {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        out.extend_from_slice(&row);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_is_the_mean_of_the_linear_channels() {
        let identity: [u8; 256] = std::array::from_fn(|i| i as u8);
        assert_eq!(to_grey(&[30, 60, 90, 255, 255, 254], &identity), [60, 255]);
    }

    #[test]
    fn sharpen_steepens_edges_and_keeps_flat_areas() {
        // 4 x 3 grey image, a step between columns 1 and 2.
        let mut img = [10, 10, 100, 100, 10, 10, 100, 100, 10, 10, 100, 100];
        sharpen(&mut img, 4, 1, 1.5);
        // Middle line: 10 - 1.5 x 30 = -35 -> 0; 100 + 1.5 x 30 = 145.
        assert_eq!(&img[4..8], &[10, 0, 145, 100]);
        // Edges untouched.
        assert_eq!(&img[..4], &[10, 10, 100, 100]);
        let mut flat = [50u8; 27];
        sharpen(&mut flat, 3, 3, 1.5);
        assert!(flat.iter().all(|&v| v == 50));
    }

    #[test]
    fn lineart_cuts_at_255_minus_the_threshold() {
        // Threshold 55: white from 200 up. 10 pixels: 2 bytes per line.
        let grey = [199, 200, 0, 255, 0, 0, 0, 0, 255, 10];
        assert_eq!(
            to_lineart(&grey, 10, DEFAULT_THRESHOLD),
            [0b0101_0000, 0b1000_0000]
        );
    }
}
