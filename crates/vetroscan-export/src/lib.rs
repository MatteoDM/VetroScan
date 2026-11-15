// SPDX-License-Identifier: GPL-2.0-or-later
//! Writing scans to files: single images in the common formats, and
//! documents of several pages (PDF, multi-page TIFF, a folder or a ZIP of
//! images).
//!
//! Every encoder is pure Rust. Each file records the resolution of the
//! scan where the format allows it, so that a 300 dpi page prints at its
//! real size, and says what its numbers are with an ICC profile ([`icc`]:
//! sRGB primaries, the gamma of the tone curve the page was drawn with)
//! where the format and its encoder take one: PNG, TIFF, JPEG, WebP and the
//! PDF's pictures; not AVIF nor BMP (their encoders have no place for it),
//! nor black and white.
//!
//! JPEG XL is not offered: the Rust JPEG XL encoders are not mature yet.

pub mod compress;
pub mod icc;
mod pdf;
mod raw;

use scan_imaging::{Format, Image};
use std::fmt;
use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};

pub use compress::{Encoding, Planned, Sample};
pub use pdf::{Layout, Orientation, PageSize, PdfPage, Placed, Placement};
pub use raw::{save_deep, save_dng, save_tiff16, Linear16};

/// A finished image and the gamma of the tone curve its numbers were drawn
/// with (the page's tone: what its file's profile says).
#[derive(Debug, Clone, Copy)]
pub struct Tagged<'a> {
    pub image: &'a Image,
    pub gamma: f64,
}

impl Tagged<'_> {
    /// The profile of its file: RGB or grey, none for black and white.
    fn profile(&self) -> Option<Vec<u8>> {
        icc::of(self.image.format, self.gamma)
    }
}

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

/// Writes the file `path` with `write`, into a temporary file next to it that
/// replaces `path` only once it is complete: an export that fails half-way
/// leaves the file that was there as it was.
pub(crate) fn write_file(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<()>,
) -> Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{}: not a file name", path.display()))?;
    let mut temporary = std::ffi::OsString::from(".");
    temporary.push(name);
    temporary.push(".part");
    let temporary = path.with_file_name(temporary);
    let written = (|| -> Result<()> {
        let mut file = BufWriter::new(File::create(&temporary)?);
        write(&mut file)?;
        file.flush()?;
        file.get_ref().sync_all()?;
        Ok(())
    })();
    let done = written.and_then(|()| Ok(std::fs::rename(&temporary, path)?));
    if done.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    done
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

/// Writes one image to `out` in `format`, with its profile where the
/// format takes one.
pub fn write_image<W: Write + Seek>(
    out: &mut W,
    page: Tagged<'_>,
    format: ImageFormat,
) -> Result<()> {
    use image::ImageEncoder;
    let image = page.image;
    let (w, h) = (u32::try_from(image.width)?, u32::try_from(image.height)?);
    match format {
        ImageFormat::Png => write_png(out, page),
        ImageFormat::Tiff => write_tiff(out, std::slice::from_ref(&page)),
        ImageFormat::Jpeg { quality } => {
            let (data, color) = pixels(image);
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(out, quality);
            encoder.set_pixel_density(image::codecs::jpeg::PixelDensity::dpi(image.dpi));
            if let Some(profile) = page.profile() {
                encoder.set_icc_profile(profile)?;
            }
            encoder.write_image(&data, w, h, color)?;
            Ok(())
        }
        ImageFormat::WebP => {
            let (data, color) = pixels(image);
            // The WebP encoder takes RGB(A) or L8(A) only.
            let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(out);
            // WebP holds colour only (grey is stored as RGB): an RGB profile
            // for grey too, which leaves neutral greys neutral.
            if image.format != Format::Lineart {
                encoder.set_icc_profile(icc::rgb(page.gamma))?;
            }
            encoder.write_image(&data, w, h, color)?;
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
pub fn save(path: &Path, page: Tagged<'_>) -> Result<()> {
    let format = ImageFormat::from_path(path).unwrap_or(ImageFormat::Png);
    save_as(path, page, format)
}

/// Writes one image to a file in `format`.
pub fn save_as(path: &Path, page: Tagged<'_>, format: ImageFormat) -> Result<()> {
    write_file(path, |file| write_image(file, page, format))
}

/// The PNG header of an image: size, colour, depth, resolution and the
/// profile (`iCCP`) if any.
pub(crate) fn png_info(
    width: usize,
    height: usize,
    color: (png::ColorType, png::BitDepth),
    dpi: u16,
    profile: Option<Vec<u8>>,
) -> Result<png::Info<'static>> {
    let mut info = png::Info::with_size(u32::try_from(width)?, u32::try_from(height)?);
    (info.color_type, info.bit_depth) = color;
    let ppm = (f64::from(dpi) / 0.0254).round() as u32;
    info.pixel_dims = Some(png::PixelDimensions {
        xppu: ppm,
        yppu: ppm,
        unit: png::Unit::Meter,
    });
    info.icc_profile = profile.map(Into::into);
    Ok(info)
}

fn write_png<W: Write>(out: &mut W, page: Tagged<'_>) -> Result<()> {
    let image = page.image;
    let color = match image.format {
        Format::Rgb => (png::ColorType::Rgb, png::BitDepth::Eight),
        Format::Grey => (png::ColorType::Grayscale, png::BitDepth::Eight),
        Format::Lineart => (png::ColorType::Grayscale, png::BitDepth::One),
    };
    let info = png_info(image.width, image.height, color, image.dpi, page.profile())?;
    png::Encoder::with_info(out, info)?
        .write_header()?
        .write_image_data(&image.data)?;
    Ok(())
}

/// A TIFF value of type UNDEFINED, as the ICC profile's tag (34675) is
/// written (the tiff crate writes bytes as BYTE).
struct Undefined<'a>(&'a [u8]);

impl tiff::encoder::TiffValue for Undefined<'_> {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: tiff::tags::Type = tiff::tags::Type::UNDEFINED;

    fn count(&self) -> usize {
        self.0.len()
    }

    fn data(&self) -> std::borrow::Cow<'_, [u8]> {
        self.0.into()
    }
}

/// Writes `pages` as one TIFF file, one page each (Deflate compression,
/// resolution recorded, each page's profile in its directory). Black and
/// white is stored as 8-bit grey 0/255, which Deflate shrinks almost as
/// much as 1 bit. No pages is an error: a TIFF needs one.
pub fn write_tiff<W: Write + Seek>(out: &mut W, pages: &[Tagged<'_>]) -> Result<()> {
    use tiff::encoder::{colortype, compression::DeflateLevel, Compression, Rational, TiffEncoder};
    use tiff::tags::{ResolutionUnit, Tag};
    if pages.is_empty() {
        return Err("no pages".into());
    }
    let mut encoder =
        TiffEncoder::new(out)?.with_compression(Compression::Deflate(DeflateLevel::Balanced));
    for tagged in pages {
        let page = tagged.image;
        let (w, h) = (u32::try_from(page.width)?, u32::try_from(page.height)?);
        let dpi = Rational {
            n: u32::from(page.dpi),
            d: 1,
        };
        let profile = tagged.profile();
        match page.format {
            Format::Rgb => {
                let mut img = encoder.new_image::<colortype::RGB8>(w, h)?;
                img.resolution(ResolutionUnit::Inch, dpi);
                if let Some(p) = &profile {
                    img.encoder().write_tag(Tag::IccProfile, Undefined(p))?;
                }
                img.write_data(&page.data)?;
            }
            Format::Grey => {
                let mut img = encoder.new_image::<colortype::Gray8>(w, h)?;
                img.resolution(ResolutionUnit::Inch, dpi);
                if let Some(p) = &profile {
                    img.encoder().write_tag(Tag::IccProfile, Undefined(p))?;
                }
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
pub fn save_tiff(path: &Path, pages: &[Tagged<'_>]) -> Result<()> {
    write_file(path, |file| write_tiff(file, pages))
}

/// Writes `pages` as a PDF document (see [`pdf`]), laid out on `layout`'s
/// sheet (a bare [`PageSize`] keeps each scan at its real size).
pub fn save_pdf(
    path: &Path,
    pages: &[Tagged<'_>],
    layout: impl Into<Layout>,
    jpeg_quality: u8,
) -> Result<()> {
    write_file(path, |file| pdf::write(file, pages, layout, jpeg_quality))
}

/// Writes a PDF document whose pages each have their own sheet and
/// encoding.
pub fn save_pdf_pages(path: &Path, pages: &[PdfPage]) -> Result<()> {
    write_file(path, |file| pdf::write_pages(file, pages))
}

/// File name of page `n` (from 1) of `count`: `name-01.png` and so on.
fn page_name(name: &str, n: usize, count: usize, format: ImageFormat) -> String {
    numbered(name, n, count, format.extension())
}

/// File name of page `n` (from 1) of `count` with `extension`: `name-01.tif`
/// and so on, with as many digits as `count` needs, at least two (the names
/// sort in page order). Every file a document is written to one per page
/// is named so.
pub fn numbered(name: &str, n: usize, count: usize, extension: &str) -> String {
    let digits = count.to_string().len().max(2);
    format!("{name}-{n:0digits$}.{extension}")
}

/// Writes each page as an image in `folder` (created if needed), named
/// `name-01.png` and so on. Returns the files written.
pub fn save_folder(
    folder: &Path,
    name: &str,
    pages: &[Tagged<'_>],
    format: ImageFormat,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(folder)?;
    pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let path = folder.join(page_name(name, i + 1, pages.len(), format));
            save_as(&path, *page, format)?;
            Ok(path)
        })
        .collect()
}

/// Writes each page as an image into a ZIP file (stored as they are: the
/// images are already compressed).
pub fn save_zip(path: &Path, name: &str, pages: &[Tagged<'_>], format: ImageFormat) -> Result<()> {
    write_file(path, |file| {
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (i, page) in pages.iter().enumerate() {
            let mut buffer = std::io::Cursor::new(Vec::new());
            write_image(&mut buffer, *page, format)?;
            zip.start_file(page_name(name, i + 1, pages.len(), format), options)?;
            zip.write_all(buffer.get_ref())?;
        }
        zip.finish()?;
        Ok(())
    })
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

    fn tagged(image: &Image) -> Tagged<'_> {
        Tagged { image, gamma: 2.2 }
    }

    #[test]
    fn every_format_round_trips_through_the_image_crate() {
        for format in [Format::Rgb, Format::Grey, Format::Lineart] {
            let img = sample(format);
            for f in ImageFormat::ALL {
                let mut out = std::io::Cursor::new(Vec::new());
                write_image(&mut out, tagged(&img), f).unwrap_or_else(|e| panic!("{f}: {e}"));
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

    /// The profile of a file, read by its format's decoder (the image
    /// crate's generic reader loses a TIFF's on the way).
    fn profile_of(bytes: &[u8], format: ImageFormat) -> Option<Vec<u8>> {
        use image::codecs;
        use image::ImageDecoder;
        let r = std::io::Cursor::new(bytes);
        match format {
            ImageFormat::Png => codecs::png::PngDecoder::new(r).unwrap().icc_profile(),
            ImageFormat::Jpeg { .. } => codecs::jpeg::JpegDecoder::new(r).unwrap().icc_profile(),
            ImageFormat::Tiff => codecs::tiff::TiffDecoder::new(r).unwrap().icc_profile(),
            ImageFormat::WebP => codecs::webp::WebPDecoder::new(r).unwrap().icc_profile(),
            _ => unreachable!("no profile in {format}"),
        }
        .unwrap()
    }

    #[test]
    fn every_format_says_what_its_numbers_are() {
        // A scanner drawing at 1.8 (as the Epson 4490's glass), one at 2.2.
        for gamma in [1.8, 2.2] {
            for format in [Format::Rgb, Format::Grey, Format::Lineart] {
                let img = sample(format);
                let page = Tagged { image: &img, gamma };
                for f in ImageFormat::ALL {
                    let mut out = std::io::Cursor::new(Vec::new());
                    write_image(&mut out, page, f).unwrap();
                    let bytes = out.into_inner();
                    if matches!(f, ImageFormat::Avif { .. } | ImageFormat::Bmp) {
                        // No place for a profile in their encoders.
                        continue;
                    }
                    let profile = profile_of(&bytes, f);
                    let space = match format {
                        // Nothing to describe in two levels.
                        Format::Lineart => {
                            assert_eq!(profile, None, "{f}");
                            continue;
                        }
                        // WebP stores grey as colour.
                        Format::Grey if f != ImageFormat::WebP => *b"GRAY",
                        _ => *b"RGB ",
                    };
                    let profile = profile.unwrap_or_else(|| panic!("{f} {format:?}: no profile"));
                    let d = icc::read(&profile)
                        .unwrap_or_else(|| panic!("{f} {format:?}: {} bytes", profile.len()));
                    assert_eq!(d.space, space, "{f} {format:?}");
                    assert!(
                        (d.gamma - gamma).abs() < 0.002,
                        "{f} {format:?}: {}",
                        d.gamma
                    );
                    assert!((d.white[2] - 1.0891).abs() < 1e-4, "{f}: D65");
                }
            }
        }
    }

    #[test]
    fn multi_page_tiff_has_every_page_with_its_profile() {
        let pages = [
            sample(Format::Rgb),
            sample(Format::Grey),
            sample(Format::Lineart),
        ];
        let tagged: Vec<Tagged> = pages
            .iter()
            .zip([1.8, 2.2, 2.2])
            .map(|(image, gamma)| Tagged { image, gamma })
            .collect();
        let mut out = std::io::Cursor::new(Vec::new());
        write_tiff(&mut out, &tagged).unwrap();
        out.set_position(0);
        let mut decoder = tiff::decoder::Decoder::new(out).unwrap();
        let mut profiles = Vec::new();
        loop {
            let profile = decoder
                .get_tag_u8_vec(tiff::tags::Tag::IccProfile)
                .ok()
                .map(|p| icc::read(&p).unwrap());
            profiles.push(profile.map(|d| (d.space, d.gamma)));
            if !decoder.more_images() {
                break;
            }
            decoder.next_image().unwrap();
        }
        assert_eq!(
            profiles,
            [
                Some((*b"RGB ", 1.80078125)),
                Some((*b"GRAY", 2.19921875)),
                None
            ]
        );
    }

    #[test]
    fn the_numbers_do_not_change() {
        // The same pixels with or without a profile, at any gamma.
        let img = sample(Format::Rgb);
        for f in [ImageFormat::Png, ImageFormat::Tiff, ImageFormat::WebP] {
            let decoded = [1.8, 2.2].map(|gamma| {
                let mut out = std::io::Cursor::new(Vec::new());
                write_image(&mut out, Tagged { image: &img, gamma }, f).unwrap();
                image::load_from_memory(out.get_ref())
                    .unwrap()
                    .to_rgb8()
                    .into_raw()
            });
            assert_eq!(decoded[0], img.data, "{f}");
            assert_eq!(decoded[1], img.data, "{f}");
        }
    }

    #[test]
    fn page_names_sort() {
        assert_eq!(page_name("doc", 3, 12, ImageFormat::Png), "doc-03.png");
        assert_eq!(page_name("doc", 7, 120, ImageFormat::Tiff), "doc-007.tif");
    }

    #[test]
    fn a_failed_save_leaves_the_old_file() {
        let dir = std::env::temp_dir().join(format!("vetroscan-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.tif");
        std::fs::write(&path, b"the old file").unwrap();
        // No pages: an error, and the old file as it was, nothing left beside it.
        assert!(save_tiff(&path, &[]).is_err());
        let grey = sample(Format::Grey);
        assert_eq!(std::fs::read(&path).unwrap(), b"the old file");
        assert!(!dir.join(".doc.tif.part").exists());
        // A good save replaces it.
        save_tiff(&path, &[tagged(&grey)]).unwrap();
        assert_eq!(&std::fs::read(&path).unwrap()[..2], b"II");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
