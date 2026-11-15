// SPDX-License-Identifier: GPL-2.0-or-later
//! How the pictures of a PDF are stored, and how to make a PDF fit a size.
//!
//! Each page has an [`Encoding`]: the JPEG quality of colour and grey
//! pages, the resolution above which it is resampled down, and whether a
//! colour page with no colour in it (a letter in black ink, a grey photo)
//! is stored as grey, a third of the data. Black and white pages are always
//! 1-bit and lossless.
//!
//! [`plan`] fits a document to a size: it measures each page on a
//! [`Sample`] (a smaller copy, a few million pixels) and lowers, one step
//! at a time, the page whose next step saves the most, until the whole
//! fits. Pages already small keep their quality: a document with one
//! large photo and nine pages of text gives up on the photo, not on the
//! text. The steps ([`ladder`]) go down in quality before resolution, and
//! never below what still reads well on paper.

use crate::Result;
use scan_imaging::{Format, Image};
use std::borrow::Cow;
use std::io::Write;

/// How a page's picture is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoding {
    /// JPEG quality of colour and grey pages, 1 to 100.
    pub quality: u8,
    /// Resampled down to this resolution when scanned finer (none: as
    /// scanned).
    pub max_dpi: Option<u16>,
    /// A colour page with no colour in it stored as grey.
    pub neutral_to_grey: bool,
}

impl Default for Encoding {
    /// Full quality: as scanned, JPEG 95.
    fn default() -> Self {
        Self {
            quality: 95,
            max_dpi: None,
            neutral_to_grey: false,
        }
    }
}

impl Encoding {
    /// The encoding of a quality from 30 to 100% (the choice in the app):
    /// 100% is the scan as it is with JPEG 95; lower, a lower JPEG quality
    /// and, under 90%, a cap on the resolution (300, 200, then 150 dpi).
    pub fn from_percent(percent: u8) -> Self {
        let p = f64::from(percent.clamp(10, 100));
        Self {
            quality: (35.0 + 0.6 * p).round() as u8,
            max_dpi: match percent {
                90.. => None,
                75..=89 => Some(300),
                55..=74 => Some(200),
                _ => Some(150),
            },
            neutral_to_grey: percent < 100,
        }
    }

    /// Whether `self` keeps at least as much as `other`.
    fn at_least(&self, other: &Self) -> bool {
        let dpi = |e: &Self| e.max_dpi.unwrap_or(u16::MAX);
        self.quality >= other.quality && dpi(self) >= dpi(other)
    }
}

/// The steps a page goes down, from `base`: first colour pages with no
/// colour to grey, then lower JPEG qualities, then lower resolutions,
/// down to JPEG 45 at 100 dpi (text still reads, photos still look like
/// photos).
pub fn ladder(base: Encoding) -> Vec<Encoding> {
    let mut steps = vec![base];
    let grey = Encoding {
        neutral_to_grey: true,
        ..base
    };
    if grey != base {
        steps.push(grey);
    }
    for (quality, dpi) in [
        (85, None),
        (80, Some(300)),
        (72, Some(300)),
        (72, Some(240)),
        (65, Some(200)),
        (60, Some(170)),
        (55, Some(150)),
        (50, Some(130)),
        (45, Some(100)),
    ] {
        let step = Encoding {
            quality,
            max_dpi: dpi,
            neutral_to_grey: true,
        };
        // Only below where the page starts.
        let capped = Encoding {
            quality: quality.min(base.quality),
            max_dpi: match (dpi, base.max_dpi) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
            ..step
        };
        if base.at_least(&capped) && steps.last() != Some(&capped) {
            steps.push(capped);
        }
    }
    steps
}

/// Side of the squares, pixels, whose mean colour [`is_neutral`] looks at.
const NEUTRAL_BLOCK: usize = 16;

/// How far, in levels, a square's mean colour may be from the page's tint
/// and still be no colour: the coloured fringes a scanner leaves along black
/// text average out within a square, a pen stroke or a stamp does not.
/// Measured on 13 November 2026 (the Epson 4490, an A4 at 300 dpi): a page of
/// black text came out at most 9 levels from its paper's tint, a page with a
/// blue signature up to 42, 75 of its squares beyond 15.
const NEUTRAL_TINT: i32 = 15;

/// Whether a colour image has no colour worth keeping: the mean colour of
/// every square of [`NEUTRAL_BLOCK`] pixels is the page's own tint (the
/// paper's), within [`NEUTRAL_TINT`] levels. A single square of colour (a
/// signature in blue ink, a red stamp) keeps the page in colour.
///
/// The paper's tint is the scanner's light on white: a darker square that
/// is still neutral has less of it, in proportion to its brightness (on the
/// Epson 4490 the paper is 10 levels bluer than green, black text none), so
/// each square is set against the tint scaled to its own brightness.
pub fn is_neutral(image: &Image) -> bool {
    let (w, h) = (image.width, image.height);
    if image.format != Format::Rgb || image.data.len() != w * h * 3 {
        return false;
    }
    // Each square's mean red and blue against its green, and its green.
    let mut tints: Vec<[i32; 2]> = Vec::new();
    let mut greens: Vec<i32> = Vec::new();
    for by in (0..h).step_by(NEUTRAL_BLOCK) {
        for bx in (0..w).step_by(NEUTRAL_BLOCK) {
            let (mut sum, mut n) = ([0i64; 3], 0i64);
            for y in by..(by + NEUTRAL_BLOCK).min(h) {
                let row = &image.data[(y * w + bx) * 3..(y * w + (bx + NEUTRAL_BLOCK).min(w)) * 3];
                for p in row.chunks_exact(3) {
                    for c in 0..3 {
                        sum[c] += i64::from(p[c]);
                    }
                    n += 1;
                }
            }
            let mean = sum.map(|v| (v / n.max(1)) as i32);
            tints.push([mean[0] - mean[1], mean[2] - mean[1]]);
            greens.push(mean[1]);
        }
    }
    if tints.is_empty() {
        return true;
    }
    // The paper: the median square's tint and brightness.
    let median = |mut v: Vec<i32>| {
        let mid = v.len() / 2;
        *v.select_nth_unstable(mid).1
    };
    let page = [
        median(tints.iter().map(|t| t[0]).collect()),
        median(tints.iter().map(|t| t[1]).collect()),
    ];
    let paper = median(greens.clone()).max(1);
    tints.iter().zip(&greens).all(|(t, &green)| {
        (0..2).all(|c| {
            let expected = page[c] * green / paper;
            (t[c] - expected).abs() <= NEUTRAL_TINT
        })
    })
}

/// The image as it goes into the PDF with `encoding`: grey if neutral and
/// so asked, resampled down if finer than its cap. An error for an image
/// whose data is not as long as its size says.
pub fn prepare<'a>(image: &'a Image, encoding: &Encoding) -> Result<Cow<'a, Image>> {
    let mut out = Cow::Borrowed(image);
    if image.format == Format::Lineart {
        return Ok(out);
    }
    if encoding.neutral_to_grey && is_neutral(image) {
        out = Cow::Owned(Image {
            width: image.width,
            height: image.height,
            dpi: image.dpi,
            format: Format::Grey,
            data: image
                .data
                .chunks_exact(3)
                .map(|p| {
                    ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000)
                        as u8
                })
                .collect(),
        });
    }
    if let Some(cap) = encoding.max_dpi.filter(|&cap| cap < out.dpi) {
        let ratio = f64::from(cap) / f64::from(out.dpi);
        let w = ((out.width as f64 * ratio).round() as usize).max(1);
        let h = ((out.height as f64 * ratio).round() as usize).max(1);
        out = Cow::Owned(resize(&out, w, h, cap)?);
    }
    Ok(out)
}

/// `image` resampled to `w` × `h` pixels at `dpi`; an error for an image
/// whose data is not as long as its size says.
fn resize(image: &Image, w: usize, h: usize, dpi: u16) -> Result<Image> {
    use image::imageops::{resize, FilterType};
    let (iw, ih) = (u32::try_from(image.width)?, u32::try_from(image.height)?);
    let (ow, oh) = (u32::try_from(w)?, u32::try_from(h)?);
    let wrong = || {
        format!(
            "an image of {} x {} with {} bytes of data",
            image.width,
            image.height,
            image.data.len()
        )
    };
    let data = match image.format {
        Format::Rgb => {
            let buffer = image::RgbImage::from_raw(iw, ih, image.data.clone()).ok_or_else(wrong)?;
            resize(&buffer, ow, oh, FilterType::CatmullRom).into_raw()
        }
        _ => {
            let buffer =
                image::GrayImage::from_raw(iw, ih, image.data.clone()).ok_or_else(wrong)?;
            resize(&buffer, ow, oh, FilterType::CatmullRom).into_raw()
        }
    };
    Ok(Image {
        width: w,
        height: h,
        dpi,
        format: image.format,
        data,
    })
}

/// The picture of a page as stored in the PDF: its dictionary entries and
/// data (after [`prepare`]); colour and grey in the colour space of the
/// `profile` object if given (`/ICCBased`).
pub(crate) fn encode(
    image: &Image,
    quality: u8,
    profile: Option<usize>,
) -> Result<(String, Vec<u8>)> {
    use image::ImageEncoder;
    let (w, h) = (image.width, image.height);
    match image.format {
        Format::Rgb | Format::Grey => {
            let (device, color) = if image.format == Format::Rgb {
                ("/DeviceRGB", image::ExtendedColorType::Rgb8)
            } else {
                ("/DeviceGray", image::ExtendedColorType::L8)
            };
            let space = profile.map_or_else(|| device.into(), |id| format!("[/ICCBased {id} 0 R]"));
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality.clamp(1, 100))
                .write_image(&image.data, u32::try_from(w)?, u32::try_from(h)?, color)?;
            Ok((
                format!(
                    "/Width {w} /Height {h} /ColorSpace {space} /BitsPerComponent 8 /Filter /DCTDecode"
                ),
                jpeg,
            ))
        }
        Format::Lineart => {
            // 1 bit, lines padded to a byte, 1 = white: PDF's own layout for
            // DeviceGray at 1 bit per component.
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
            z.write_all(&image.data)?;
            Ok((
                format!(
                    "/Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /FlateDecode"
                ),
                z.finish()?,
            ))
        }
    }
}

/// What a page costs in the PDF besides its picture (page, contents and
/// image dictionaries), and the document besides its pages (with a colour
/// and a grey profile, about 560 bytes each), in bytes.
pub const PAGE_OVERHEAD: u64 = 420;
pub const DOCUMENT_OVERHEAD: u64 = 1700;

/// Largest sample measured, pixels.
const SAMPLE_PIXELS: usize = 3_000_000;

/// A page measured for [`estimate`]: a smaller copy of its finished image
/// and the size of the real one.
#[derive(Debug, Clone)]
pub struct Sample {
    /// The copy (the page itself when small, or black and white).
    image: Image,
    /// Width, height and resolution of the real page.
    width: usize,
    height: usize,
    dpi: u16,
}

impl Sample {
    /// A sample of a finished page.
    pub fn new(page: &Image) -> Self {
        Self::of(page, page.width, page.height, page.dpi)
    }

    /// A sample from a smaller copy of a page whose real size is `width` ×
    /// `height` at `dpi` (the copy shown on the screen, say).
    pub fn of(copy: &Image, width: usize, height: usize, dpi: u16) -> Self {
        let pixels = copy.width * copy.height;
        let image = if copy.format == Format::Lineart || pixels <= SAMPLE_PIXELS {
            copy.clone()
        } else {
            let ratio = (SAMPLE_PIXELS as f64 / pixels as f64).sqrt();
            let w = ((copy.width as f64 * ratio) as usize).max(1);
            let h = ((copy.height as f64 * ratio) as usize).max(1);
            // A copy that cannot be resampled is measured as it is (its
            // encoding then says what is wrong with it).
            resize(copy, w, h, copy.dpi).unwrap_or_else(|_| copy.clone())
        };
        Self {
            image,
            width,
            height,
            dpi,
        }
    }
}

/// About how many bytes the picture of the sampled page takes with
/// `encoding`: measured on the sample brought to the resolution the page
/// will have, or, when that is finer than the sample, from the bytes per
/// pixel of the sample (which errs on the large side).
pub fn estimate(sample: &Sample, encoding: &Encoding) -> Result<u64> {
    let image = &sample.image;
    if image.format == Format::Lineart {
        let scale = (sample.width * sample.height) as f64 / (image.width * image.height) as f64;
        return Ok((encode(image, encoding.quality, None)?.1.len() as f64 * scale) as u64);
    }
    let dpi = encoding
        .max_dpi
        .map_or(sample.dpi, |cap| cap.min(sample.dpi));
    let ratio = f64::from(dpi) / f64::from(sample.dpi.max(1));
    let (w, h) = (
        ((sample.width as f64 * ratio).round() as usize).max(1),
        ((sample.height as f64 * ratio).round() as usize).max(1),
    );
    // Grey or not is decided on the sample, as on the page.
    let encoding = Encoding {
        max_dpi: None,
        ..*encoding
    };
    let prepared = prepare(image, &encoding)?;
    if w * h <= prepared.width * prepared.height {
        let small = resize(&prepared, w, h, dpi)?;
        return Ok(encode(&small, encoding.quality, None)?.1.len() as u64);
    }
    let bytes = encode(&prepared, encoding.quality, None)?.1.len() as f64;
    Ok((bytes * (w * h) as f64 / (prepared.width * prepared.height) as f64) as u64)
}

/// What [`plan`] chose.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    /// The encoding of each page.
    pub encodings: Vec<Encoding>,
    /// About how large the PDF will be, bytes.
    pub bytes: u64,
    /// Whether it fits the limit (always, without one).
    pub fits: bool,
}

/// The encodings of the pages so that the PDF is at most `limit` bytes
/// (none: each page with `base`), losing as little as possible: each step
/// lowers the page whose next step saves the most.
pub fn plan(samples: &[Sample], base: Encoding, limit: Option<u64>) -> Result<Planned> {
    let steps = ladder(base);
    let overhead = DOCUMENT_OVERHEAD + PAGE_OVERHEAD * samples.len() as u64;
    // sizes[page][step], measured when first needed.
    let mut sizes: Vec<Vec<Option<u64>>> = vec![vec![None; steps.len()]; samples.len()];
    let mut size = |page: usize, step: usize| -> Result<u64> {
        if let Some(bytes) = sizes[page][step] {
            return Ok(bytes);
        }
        let bytes = estimate(&samples[page], &steps[step])?;
        sizes[page][step] = Some(bytes);
        Ok(bytes)
    };
    let mut at = vec![0usize; samples.len()];
    let mut total = overhead;
    for page in 0..samples.len() {
        total += size(page, 0)?;
    }
    if let Some(limit) = limit {
        while total > limit {
            // The page whose next step that saves anything saves the most
            // (a step that saves nothing, as grey on a page already grey, is
            // passed over, never taken for nothing).
            let mut best: Option<(usize, usize, u64, u64)> = None;
            for page in 0..samples.len() {
                if samples[page].image.format == Format::Lineart {
                    continue;
                }
                let now = size(page, at[page])?;
                let mut step = at[page] + 1;
                while step < steps.len() && size(page, step)? >= now {
                    step += 1;
                }
                if step >= steps.len() {
                    continue;
                }
                let next = size(page, step)?;
                let saves = now - next;
                if best.is_none_or(|(_, _, s, _)| saves > s) {
                    best = Some((page, step, saves, next));
                }
            }
            let Some((page, step, _, next)) = best else {
                break;
            };
            total = total - size(page, at[page])? + next;
            at[page] = step;
        }
    }
    Ok(Planned {
        encodings: at.iter().map(|&i| steps[i]).collect(),
        bytes: total,
        fits: limit.is_none_or(|l| total <= l),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page of `w` × `h` pixels at `dpi`, with detail (so that JPEG has
    /// something to keep), in colour or with no colour.
    fn page(w: usize, h: usize, dpi: u16, coloured: bool) -> Image {
        let mut data = Vec::with_capacity(w * h * 3);
        let mut seed = 12345u32;
        for y in 0..h {
            for x in 0..w {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let noise = (seed >> 24) as u8 / 4;
                let v = ((x * 7 + y * 3) % 190) as u8 + noise;
                if coloured {
                    data.extend_from_slice(&[v, v / 2, 255 - v]);
                } else {
                    data.extend_from_slice(&[v, v, v.saturating_add(2)]);
                }
            }
        }
        Image {
            width: w,
            height: h,
            dpi,
            format: Format::Rgb,
            data,
        }
    }

    #[test]
    fn hundred_percent_is_the_scan_as_it_is() {
        let e = Encoding::from_percent(100);
        assert_eq!(e, Encoding::default());
        let e = Encoding::from_percent(60);
        assert_eq!(e.max_dpi, Some(200));
        assert!(e.quality < 95 && e.neutral_to_grey);
    }

    #[test]
    fn the_ladder_only_goes_down() {
        let base = Encoding::from_percent(80);
        let steps = ladder(base);
        assert_eq!(steps[0], base);
        for pair in steps.windows(2) {
            assert!(pair[0].at_least(&pair[1]), "{pair:?}");
            assert_ne!(pair[0], pair[1]);
        }
        assert!(steps.iter().all(|s| base.at_least(s)));
    }

    #[test]
    fn a_page_with_no_colour_goes_grey_and_a_coloured_one_does_not() {
        let grey = Encoding {
            neutral_to_grey: true,
            ..Encoding::default()
        };
        assert_eq!(
            prepare(&page(60, 40, 300, false), &grey).unwrap().format,
            Format::Grey
        );
        assert_eq!(
            prepare(&page(60, 40, 300, true), &grey).unwrap().format,
            Format::Rgb
        );
        // Not asked: left in colour.
        let as_is = Encoding::default();
        assert_eq!(
            prepare(&page(60, 40, 300, false), &as_is).unwrap().format,
            Format::Rgb
        );
    }

    #[test]
    fn a_cap_resamples_and_keeps_the_size_on_paper() {
        let image = page(600, 300, 600, true);
        let capped = Encoding {
            max_dpi: Some(200),
            ..Encoding::default()
        };
        let out = prepare(&image, &capped).unwrap();
        assert_eq!((out.width, out.height, out.dpi), (200, 100, 200));
    }

    #[test]
    fn the_plan_fits_the_limit_and_spares_the_small_pages() {
        let big = page(1200, 1600, 300, true);
        let small = page(300, 400, 300, true);
        let samples = [Sample::new(&big), Sample::new(&small), Sample::new(&small)];
        let free = plan(&samples, Encoding::default(), None).unwrap();
        assert!(free.fits);
        let limit = free.bytes / 3;
        let fitted = plan(&samples, Encoding::default(), Some(limit)).unwrap();
        assert!(fitted.fits, "{fitted:?}");
        assert!(fitted.bytes <= limit);
        // The big page gave up more than the small ones.
        let steps = ladder(Encoding::default());
        let rank = |e: &Encoding| steps.iter().position(|s| s == e).unwrap();
        assert!(rank(&fitted.encodings[0]) >= rank(&fitted.encodings[1]));
    }

    #[test]
    fn an_impossible_limit_says_so() {
        let samples = [Sample::new(&page(800, 800, 300, true))];
        let p = plan(&samples, Encoding::default(), Some(10)).unwrap();
        assert!(!p.fits);
        assert_eq!(p.encodings[0], *ladder(Encoding::default()).last().unwrap());
    }

    #[test]
    fn the_estimate_is_close_to_the_real_size() {
        let image = page(1000, 1400, 300, true);
        for encoding in ladder(Encoding::default()) {
            let real = encode(&prepare(&image, &encoding).unwrap(), encoding.quality, None)
                .unwrap()
                .1
                .len() as f64;
            let estimated = estimate(&Sample::new(&image), &encoding).unwrap() as f64;
            assert!(
                (estimated / real - 1.0).abs() < 0.25,
                "{encoding:?}: {estimated} for {real}"
            );
        }
    }

    #[test]
    fn a_blue_signature_keeps_the_page_in_colour() {
        // A grey page (with the slight tint of `page`) and, low down, a short
        // stroke of blue ink: a few hundred pixels of a 600 x 400 page.
        let mut image = page(600, 400, 300, false);
        assert!(is_neutral(&image));
        for y in 330..336 {
            for x in 400..480 {
                image.data[(y * 600 + x) * 3..][..3].copy_from_slice(&[30, 40, 140]);
            }
        }
        assert!(!is_neutral(&image));
        let grey = Encoding {
            neutral_to_grey: true,
            ..Encoding::default()
        };
        assert_eq!(prepare(&image, &grey).unwrap().format, Format::Rgb);
    }

    #[test]
    fn an_image_shorter_than_its_size_is_an_error_not_a_panic() {
        let mut image = page(600, 300, 600, true);
        image.data.truncate(image.data.len() - 3);
        let capped = Encoding {
            max_dpi: Some(200),
            ..Encoding::default()
        };
        assert!(prepare(&image, &capped).is_err());
    }

    #[test]
    fn a_step_that_saves_nothing_is_never_taken() {
        // A page already grey: the grey step saves nothing, so a plan that
        // must shrink it goes straight to a lower quality.
        let grey_page = Image {
            format: Format::Grey,
            data: page(400, 400, 300, false)
                .data
                .chunks_exact(3)
                .map(|p| p[0])
                .collect(),
            ..page(400, 400, 300, false)
        };
        let samples = [Sample::new(&grey_page)];
        let free = plan(&samples, Encoding::default(), None).unwrap();
        let p = plan(&samples, Encoding::default(), Some(free.bytes - 1)).unwrap();
        let steps = ladder(Encoding::default());
        assert!(steps[1].neutral_to_grey && steps[1].quality == steps[0].quality);
        assert_ne!(
            p.encodings[0], steps[1],
            "the grey step, which saves nothing"
        );
        assert!(p.encodings[0].quality < steps[0].quality, "{p:?}");
    }

    #[test]
    fn black_text_on_a_tinted_paper_is_neutral() {
        // As measured on the Epson 4490: paper 10 levels bluer than green,
        // black text neutral. Text is darker: less of the paper's tint.
        let (w, h) = (320, 320);
        let mut data = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let text = (y / 16) % 3 == 1 && (x / 16) % 2 == 0;
                data.extend_from_slice(if text {
                    &[58, 60, 58]
                } else {
                    &[225, 228, 238]
                });
            }
        }
        let image = Image {
            width: w,
            height: h,
            dpi: 300,
            format: Format::Rgb,
            data,
        };
        assert!(is_neutral(&image));
    }
}
