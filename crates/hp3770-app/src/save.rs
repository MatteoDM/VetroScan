// SPDX-License-Identifier: GPL-2.0-or-later
//! Writing scanned images to files.

use hp3770_core::session::{Format, Image};
use std::path::Path;

/// Writes `image` as PNG (8-bit RGB, 8-bit grey or 1-bit black and white),
/// with its resolution.
pub fn png(path: &Path, image: &Image) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(
        file,
        u32::try_from(image.width)?,
        u32::try_from(image.height)?,
    );
    let (color, depth) = match image.format {
        Format::Rgb => (png::ColorType::Rgb, png::BitDepth::Eight),
        Format::Grey => (png::ColorType::Grayscale, png::BitDepth::Eight),
        Format::Lineart => (png::ColorType::Grayscale, png::BitDepth::One),
    };
    encoder.set_color(color);
    encoder.set_depth(depth);
    // Pixels per metre.
    let ppm = (f64::from(image.dpi) / 0.0254).round() as u32;
    encoder.set_pixel_dims(Some(png::PixelDimensions {
        xppu: ppm,
        yppu: ppm,
        unit: png::Unit::Meter,
    }));
    encoder.write_header()?.write_image_data(&image.data)?;
    Ok(())
}
