// SPDX-License-Identifier: GPL-2.0-or-later
//! Writing scans to files: single images in the common formats, and
//! documents of several pages (PDF, multi-page TIFF, a folder or a ZIP of
//! images).
//!
//! Every encoder is pure Rust. Each file records the resolution of the
//! scan where the format allows it, so that a 300 dpi page prints at its
//! real size.
//!
//! JPEG XL is not offered: the Rust JPEG XL encoders are not mature yet.

mod pdf;

use hp3770_core::session::{Format, Image};
use std::fmt;
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};

pub use pdf::PageSize;

/// Error of an export.
pub type Error = Box<dyn std::error::Error + Send + Sync>;
/// Result of an export.
pub type Result<T> = std::result::Result<T, Error>;

/// Image file formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// PNG: lossless, 1-bit for black and white.
    Png,
    /// JPEG with a quality from 1 to 100 (not for black and white, which is
    /// saved as grey).
    Jpeg {
        /// Quality, 1-100.
        quality: u8,
    },
    /// TIFF, Deflate compression, 1-bit for black and white.
    Tiff,
    /// WebP, lossless.
    WebP,
    /// AVIF with a quality from 1 to 100 (slow to encode).
    Avif {
        /// Quality, 1-100.
        quality: u8,
    },
    /// BMP, uncompressed.
    Bmp,
}

impl ImageFormat {
    /// Every format, with default qualities.
    pub const ALL: [ImageFormat; 6] = [
        ImageFormat::Png,
        ImageFormat::Jpeg { quality: 90 },
        ImageFormat::Tiff,
        ImageFormat::WebP,
        ImageFormat::Avif { quality: 80 },
        ImageFormat::Bmp,
    ];

    /// The usual file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg { .. } => "jpg",
            Self::Tiff => "tif",
            Self::WebP => "webp",
            Self::Avif { .. } => "avif",
            Self::Bmp => "bmp",
        }
    }

    /// The format of a file name, from its extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "png" => Self::Png,
            "jpg" | "jpeg" => Self::Jpeg { quality: 90 },
            "tif" | "tiff" => Self::Tiff,
            "webp" => Self::WebP,
            "avif" => Self::Avif { quality: 80 },
            "bmp" => Self::Bmp,
            _ => return None,
        })
    }
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Png => "PNG",
            Self::Jpeg { .. } => "JPEG",
            Self::Tiff => "TIFF",
            Self::WebP => "WebP",
            Self::Avif { .. } => "AVIF",
            Self::Bmp => "BMP",
        })
    }
}

/// Black and white expanded to 8-bit grey (0 or 255).
fn lineart_to_grey(image: &Image) -> Vec<u8> {
    let stride = image.width.div_ceil(8);
    let mut out = Vec::with_capacity(image.width * image.height);
    for line in image.data.chunks_exact(stride) {
        out.extend((0..image.width).map(|x| {
            if line[x / 8] & (0x80 >> (x % 8)) != 0 {
                255
            } else {
                0
            }
        }));
    }
    out
}

/// The pixels as the `image` crate takes them, black and white as grey.
fn pixels(image: &Image) -> (std::borrow::Cow<'_, [u8]>, image::ExtendedColorType) {
    match image.format {
        Format::Rgb => (image.data.as_slice().into(), image::ExtendedColorType::Rgb8),
        Format::Grey => (image.data.as_slice().into(), image::ExtendedColorType::L8),
        Format::Lineart => (lineart_to_grey(image).into(), image::ExtendedColorType::L8),
    }
}

/// Writes one image to `out` in `format`.
pub fn write_image<W: Write + Seek>(out: &mut W, image: &Image, format: ImageFormat) -> Result<()> {
    use image::ImageEncoder;
    let (w, h) = (u32::try_from(image.width)?, u32::try_from(image.height)?);
    match format {
        ImageFormat::Png => write_png(out, image),
        ImageFormat::Tiff => write_tiff(out, std::slice::from_ref(image)),
        ImageFormat::Jpeg { quality } => {
            let (data, color) = pixels(image);
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(out, quality);
            encoder.set_pixel_density(image::codecs::jpeg::PixelDensity::dpi(image.dpi));
            encoder.write_image(&data, w, h, color)?;
            Ok(())
        }
        ImageFormat::WebP => {
            let (data, color) = pixels(image);
            // The WebP encoder takes RGB(A) or L8(A) only.
            image::codecs::webp::WebPEncoder::new_lossless(out).write_image(&data, w, h, color)?;
            Ok(())
        }
        ImageFormat::Avif { quality } => {
            let (data, color) = pixels(image);
            let rgb;
            let (data, color) = if color == image::ExtendedColorType::L8 {
                // The AVIF encoder wants colour.
                rgb = data.iter().flat_map(|&g| [g, g, g]).collect::<Vec<u8>>();
                (&rgb[..], image::ExtendedColorType::Rgb8)
            } else {
                (&data[..], color)
            };
            image::codecs::avif::AvifEncoder::new_with_speed_quality(out, 6, quality)
                .write_image(data, w, h, color)?;
            Ok(())
        }
        ImageFormat::Bmp => {
            let (data, color) = pixels(image);
            image::codecs::bmp::BmpEncoder::new(out).write_image(&data, w, h, color)?;
            Ok(())
        }
    }
}

/// Writes one image to a file, in the format of its extension (PNG if the
/// extension is not known).
pub fn save(path: &Path, image: &Image) -> Result<()> {
    let format = ImageFormat::from_path(path).unwrap_or(ImageFormat::Png);
    save_as(path, image, format)
}

/// Writes one image to a file in `format`.
pub fn save_as(path: &Path, image: &Image, format: ImageFormat) -> Result<()> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    write_image(&mut file, image, format)?;
    file.flush()?;
    Ok(())
}

fn write_png<W: Write>(out: &mut W, image: &Image) -> Result<()> {
    let mut encoder = png::Encoder::new(
        out,
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
    let ppm = (f64::from(image.dpi) / 0.0254).round() as u32;
    encoder.set_pixel_dims(Some(png::PixelDimensions {
        xppu: ppm,
        yppu: ppm,
        unit: png::Unit::Meter,
    }));
    encoder.write_header()?.write_image_data(&image.data)?;
    Ok(())
}

/// Writes `pages` as one TIFF file, one page each (Deflate compression,
/// resolution recorded). Black and white is stored as 8-bit grey 0/255,
/// which Deflate shrinks almost as much as 1 bit.
pub fn write_tiff<W: Write + Seek>(out: &mut W, pages: &[Image]) -> Result<()> {
    use tiff::encoder::{colortype, compression::DeflateLevel, Compression, Rational, TiffEncoder};
    use tiff::tags::ResolutionUnit;
    let mut encoder =
        TiffEncoder::new(out)?.with_compression(Compression::Deflate(DeflateLevel::Balanced));
    for page in pages {
        let (w, h) = (u32::try_from(page.width)?, u32::try_from(page.height)?);
        let dpi = Rational {
            n: u32::from(page.dpi),
            d: 1,
        };
        match page.format {
            Format::Rgb => {
                let mut img = encoder.new_image::<colortype::RGB8>(w, h)?;
                img.resolution(ResolutionUnit::Inch, dpi);
                img.write_data(&page.data)?;
            }
            Format::Grey => {
                let mut img = encoder.new_image::<colortype::Gray8>(w, h)?;
                img.resolution(ResolutionUnit::Inch, dpi);
                img.write_data(&page.data)?;
            }
            Format::Lineart => {
                let grey = lineart_to_grey(page);
                let mut img = encoder.new_image::<colortype::Gray8>(w, h)?;
                img.resolution(ResolutionUnit::Inch, dpi);
                img.write_data(&grey)?;
            }
        }
    }
    Ok(())
}

/// Writes `pages` as a multi-page TIFF file.
pub fn save_tiff(path: &Path, pages: &[Image]) -> Result<()> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    write_tiff(&mut file, pages)?;
    file.flush()?;
    Ok(())
}

/// Writes `pages` as a PDF document (see [`pdf`]).
pub fn save_pdf(path: &Path, pages: &[Image], size: PageSize, jpeg_quality: u8) -> Result<()> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    pdf::write(&mut file, pages, size, jpeg_quality)?;
    file.flush()?;
    Ok(())
}

/// File name of page `n` (from 1) of `count`: `name-01.png` and so on.
fn page_name(name: &str, n: usize, count: usize, format: ImageFormat) -> String {
    let digits = count.to_string().len().max(2);
    format!("{name}-{n:0digits$}.{}", format.extension())
}

/// Writes each page as an image in `folder` (created if needed), named
/// `name-01.png` and so on. Returns the files written.
pub fn save_folder(
    folder: &Path,
    name: &str,
    pages: &[Image],
    format: ImageFormat,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(folder)?;
    pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let path = folder.join(page_name(name, i + 1, pages.len(), format));
            save_as(&path, page, format)?;
            Ok(path)
        })
        .collect()
}

/// Writes each page as an image into a ZIP file (stored as they are: the
/// images are already compressed).
pub fn save_zip(path: &Path, name: &str, pages: &[Image], format: ImageFormat) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (i, page) in pages.iter().enumerate() {
        let mut buffer = std::io::Cursor::new(Vec::new());
        write_image(&mut buffer, page, format)?;
        zip.start_file(page_name(name, i + 1, pages.len(), format), options)?;
        zip.write_all(buffer.get_ref())?;
    }
    zip.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample(format: Format) -> Image {
        let (width, height) = (16, 8);
        let data = match format {
            Format::Rgb => (0..width * height)
                .flat_map(|i| [(i * 2) as u8, 100, 200])
                .collect(),
            Format::Grey => (0..width * height).map(|i| i as u8).collect(),
            Format::Lineart => vec![0b1010_1010; width / 8 * height],
        };
        Image {
            width,
            height,
            dpi: 300,
            format,
            data,
        }
    }

    #[test]
    fn every_format_round_trips_through_the_image_crate() {
        for format in [Format::Rgb, Format::Grey, Format::Lineart] {
            let img = sample(format);
            for f in ImageFormat::ALL {
                let mut out = std::io::Cursor::new(Vec::new());
                write_image(&mut out, &img, f).unwrap_or_else(|e| panic!("{f}: {e}"));
                let bytes = out.into_inner();
                if f == (ImageFormat::Avif { quality: 80 }) {
                    // No AVIF decoder in pure Rust here: check the signature.
                    assert_eq!(&bytes[4..8], b"ftyp");
                    continue;
                }
                let back = image::load_from_memory(&bytes)
                    .unwrap_or_else(|e| panic!("{f} {format:?}: {e}"));
                assert_eq!((back.width(), back.height()), (16, 8), "{f}");
                if !matches!(f, ImageFormat::Jpeg { .. }) {
                    // Lossless: the same pixels.
                    let expect = match format {
                        Format::Rgb => img.data.clone(),
                        _ => pixels(&img).0.into_owned(),
                    };
                    let got = match format {
                        Format::Rgb => back.to_rgb8().into_raw(),
                        _ => back.to_luma8().into_raw(),
                    };
                    assert_eq!(got, expect, "{f} {format:?}");
                }
            }
        }
    }

    #[test]
    fn multi_page_tiff_has_every_page() {
        let pages = [
            sample(Format::Rgb),
            sample(Format::Grey),
            sample(Format::Lineart),
        ];
        let mut out = std::io::Cursor::new(Vec::new());
        write_tiff(&mut out, &pages).unwrap();
        out.set_position(0);
        let mut decoder = tiff::decoder::Decoder::new(out).unwrap();
        let mut count = 1;
        while decoder.more_images() {
            decoder.next_image().unwrap();
            count += 1;
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn page_names_sort() {
        assert_eq!(page_name("doc", 3, 12, ImageFormat::Png), "doc-03.png");
        assert_eq!(page_name("doc", 7, 120, ImageFormat::Tiff), "doc-007.tif");
    }
}
