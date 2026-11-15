// SPDX-License-Identifier: GPL-2.0-or-later
//! The scanner's own samples, for raw editors: 16-bit linear TIFF and DNG.
//!
//! Both hold the chip's samples untouched, as a raw file does: linear in
//! light, not balanced, not scaled, nothing clipped. The paper white the
//! calibration aims at sits below full scale (about 196/255 of it in red),
//! and everything the chip saw above it, highlights brighter than paper,
//! is kept. The colour space of the 3770's filters is close to sRGB's
//! primaries, so both files say so:
//!
//! * the DNG (Adobe's raw format, version 1.4) is a *linear raw*
//!   (photometric `LinearRaw`). As a camera's, it says what is neutral (the
//!   paper white of each channel, `AsShotNeutral`) and how much to brighten
//!   (`BaselineExposure`, so that paper opens as white), with the matrix
//!   from XYZ to these colours: Lightroom and Camera Raw develop it like a
//!   camera's raw file, white balanced, highlights recoverable;
//! * the TIFF, as the raw TIFF of scanning programs, embeds an ICC profile
//!   "sRGB lineare" (sRGB primaries, D65 adapted to D50 as ICC asks, tone
//!   curve gamma 1), which Photoshop and other colour-managed programs read.
//!   TIFF has no tags for white balance or exposure: it opens as the chip
//!   saw it, paper a little dark and warm, to be balanced in the editor.
//!
//! The writer is a small little-endian TIFF writer of one image (the tiff
//! crate cannot write DNG's own tags).

use crate::Result;
use scan_imaging::{Format, Image};
use std::io::Write;
use std::path::Path;

/// Linear RGB samples, 16 bits each, as the chip gave them.
#[derive(Debug, Clone)]
pub struct Linear16 {
    pub width: usize,
    pub height: usize,
    pub dpi: u16,
    /// Red, green, blue for each pixel, line after line.
    pub data: Vec<u16>,
    /// The level of the paper white in `data`, per channel (above 0, at
    /// most 65535): what the calibration aims at.
    pub white: [f64; 3],
    /// Who made the scanner ("HP"), for the file's metadata.
    pub make: String,
    /// The scanner's model ("Scanjet 3770").
    pub model: String,
    /// The program that wrote the file, with its version.
    pub software: String,
}

impl Linear16 {
    /// From the scanner's linear 8-bit samples, `white` their paper white
    /// per channel. Widened to 16 bits (255 to 65535), nothing else.
    pub fn from_linear8(image: &Image, white: [f64; 3]) -> Result<Self> {
        if image.format != Format::Rgb {
            return Err("RAW is colour only".into());
        }
        Ok(Self {
            width: image.width,
            height: image.height,
            dpi: image.dpi,
            data: image.data.iter().map(|&v| u16::from(v) * 257).collect(),
            white: white.map(|w| w * 257.0),
            ..Self::new(0, 0, 0, Vec::new())
        })
    }

    /// From the chip's 16-bit linear samples, `white` their paper white
    /// per channel (in 16-bit units). The samples are kept as they are.
    pub fn from_linear16(
        width: usize,
        height: usize,
        dpi: u16,
        samples: &[u16],
        white: [f64; 3],
    ) -> Self {
        Self {
            width,
            height,
            dpi,
            data: samples.to_vec(),
            white,
            ..Self::new(0, 0, 0, Vec::new())
        }
    }

    /// From 16-bit linear samples already balanced, paper white at full
    /// scale.
    pub fn new(width: usize, height: usize, dpi: u16, data: Vec<u16>) -> Self {
        Self {
            width,
            height,
            dpi,
            data,
            white: [65535.0; 3],
            make: String::new(),
            model: String::new(),
            software: String::new(),
        }
    }

    /// The same samples, said to come from `make`'s `model` and written by
    /// `software` (the file's metadata).
    pub fn with_source(self, make: &str, model: &str, software: &str) -> Self {
        Self {
            make: make.into(),
            model: model.into(),
            software: software.into(),
            ..self
        }
    }

    /// The scanner's whole name ("HP Scanjet 3770"), or "Scanner".
    fn device(&self) -> String {
        let name = format!("{} {}", self.make, self.model);
        match name.trim() {
            "" => "Scanner".into(),
            name => name.into(),
        }
    }

    /// The paper white of each channel, sane: in 1..=65535.
    fn paper(&self) -> [f64; 3] {
        self.white.map(|w| w.clamp(1.0, 65535.0))
    }
}

// ------------------------------------------------------------ TIFF writer

#[derive(Clone)]
enum Value {
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<(u32, u32)>),
    SRational(Vec<(i32, i32)>),
    Ascii(String),
    Byte(Vec<u8>),
    Undefined(Vec<u8>),
}

impl Value {
    fn kind(&self) -> u16 {
        match self {
            Value::Byte(_) => 1,
            Value::Ascii(_) => 2,
            Value::Short(_) => 3,
            Value::Long(_) => 4,
            Value::Rational(_) => 5,
            Value::Undefined(_) => 7,
            Value::SRational(_) => 10,
        }
    }

    fn count(&self) -> u32 {
        (match self {
            Value::Byte(v) | Value::Undefined(v) => v.len(),
            Value::Ascii(s) => s.len() + 1,
            Value::Short(v) => v.len(),
            Value::Long(v) => v.len(),
            Value::Rational(v) => v.len(),
            Value::SRational(v) => v.len(),
        }) as u32
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Value::Byte(v) | Value::Undefined(v) => out.extend_from_slice(v),
            Value::Ascii(s) => {
                out.extend_from_slice(s.as_bytes());
                out.push(0);
            }
            Value::Short(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Value::Long(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Value::Rational(v) => v.iter().for_each(|(a, b)| {
                out.extend_from_slice(&a.to_le_bytes());
                out.extend_from_slice(&b.to_le_bytes());
            }),
            Value::SRational(v) => v.iter().for_each(|(a, b)| {
                out.extend_from_slice(&a.to_le_bytes());
                out.extend_from_slice(&b.to_le_bytes());
            }),
        }
        out
    }
}

/// 16-bit samples of an image, `channels` (1 or 3) per pixel.
struct Deep<'a> {
    width: usize,
    height: usize,
    channels: usize,
    dpi: u16,
    data: &'a [u16],
}

impl<'a> From<&'a Linear16> for Deep<'a> {
    fn from(l: &'a Linear16) -> Self {
        Deep {
            width: l.width,
            height: l.height,
            channels: 3,
            dpi: l.dpi,
            data: &l.data,
        }
    }
}

/// Writes one uncompressed 16-bit image with `tags` (besides the layout
/// ones, which are added here).
fn write_tiff(
    path: &Path,
    image: Deep<'_>,
    photometric: u16,
    software: &str,
    mut tags: Vec<(u16, Value)>,
) -> Result<()> {
    let (w, h, ch) = (image.width, image.height, image.channels);
    if w == 0 || h == 0 || image.data.len() != w * h * ch {
        return Err("empty or inconsistent image".into());
    }
    let line_bytes = w * ch * 2;
    // Strips of about 256 KB.
    let rows = (256 * 1024 / line_bytes).clamp(1, h);
    let strips = h.div_ceil(rows);
    let strip_bytes: Vec<u32> = (0..strips)
        .map(|s| ((rows.min(h - s * rows)) * line_bytes) as u32)
        .collect();
    // Layout: header, pixel data, then the values, then the directory.
    let data_start = 8u32;
    let data_len = (h * line_bytes) as u64;
    if data_len > u64::from(u32::MAX) - (1 << 20) {
        return Err("image too large for a TIFF (over 4 GB)".into());
    }
    let mut offsets = Vec::with_capacity(strips);
    let mut at = data_start;
    for b in &strip_bytes {
        offsets.push(at);
        at += b;
    }
    tags.extend([
        (256, Value::Long(vec![w as u32])),
        (257, Value::Long(vec![h as u32])),
        (258, Value::Short(vec![16; ch])),
        (259, Value::Short(vec![1])),
        (262, Value::Short(vec![photometric])),
        (273, Value::Long(offsets)),
        (274, Value::Short(vec![1])),
        (277, Value::Short(vec![ch as u16])),
        (278, Value::Long(vec![rows as u32])),
        (279, Value::Long(strip_bytes)),
        (282, Value::Rational(vec![(u32::from(image.dpi), 1)])),
        (283, Value::Rational(vec![(u32::from(image.dpi), 1)])),
        (284, Value::Short(vec![1])),
        (296, Value::Short(vec![2])),
    ]);
    if !software.is_empty() {
        tags.push((305, Value::Ascii(software.into())));
    }
    tags.sort_by_key(|(t, _)| *t);

    // Values that do not fit in the entry go after the pixels.
    let mut extra: Vec<u8> = Vec::new();
    let extra_start = data_start + data_len as u32;
    let mut entries: Vec<[u8; 12]> = Vec::new();
    for (tag, value) in &tags {
        let bytes = value.bytes();
        let mut e = [0u8; 12];
        e[0..2].copy_from_slice(&tag.to_le_bytes());
        e[2..4].copy_from_slice(&value.kind().to_le_bytes());
        e[4..8].copy_from_slice(&value.count().to_le_bytes());
        if bytes.len() <= 4 {
            e[8..8 + bytes.len()].copy_from_slice(&bytes);
        } else {
            if extra.len() % 2 == 1 {
                extra.push(0);
            }
            let off = extra_start + extra.len() as u32;
            e[8..12].copy_from_slice(&off.to_le_bytes());
            extra.extend_from_slice(&bytes);
        }
        entries.push(e);
    }
    if extra.len() % 2 == 1 {
        extra.push(0);
    }
    let ifd = extra_start + extra.len() as u32;
    crate::write_file(path, |file| {
        file.write_all(b"II")?;
        file.write_all(&42u16.to_le_bytes())?;
        file.write_all(&ifd.to_le_bytes())?;
        let mut line = Vec::with_capacity(line_bytes);
        for row in image.data.chunks_exact(w * ch) {
            line.clear();
            for v in row {
                line.extend_from_slice(&v.to_le_bytes());
            }
            file.write_all(&line)?;
        }
        file.write_all(&extra)?;
        file.write_all(&(entries.len() as u16).to_le_bytes())?;
        for e in &entries {
            file.write_all(e)?;
        }
        file.write_all(&0u32.to_le_bytes())?;
        Ok(())
    })
}

// --------------------------------------------------------------- formats

/// XYZ (D65) to linear sRGB (IEC 61966-2-1).
const XYZ_TO_SRGB: [f64; 9] = [
    3.2406, -1.5372, -0.4986, //
    -0.9689, 1.8758, 0.0415, //
    0.0557, -0.2040, 1.0570,
];

/// Saves a 16-bit linear TIFF with the "sRGB lineare" profile: the
/// samples untouched, nothing clipped.
pub fn save_tiff16(path: &Path, image: &Linear16) -> Result<()> {
    write_tiff(
        path,
        image.into(),
        2,
        &image.software,
        vec![(34675, Value::Undefined(crate::icc::linear_srgb()))],
    )
}

/// What a DNG says about the paper white: the neutral of each channel
/// (the brightest 1) and the exposure, in stops, that brings the paper to
/// full scale.
fn dng_white(image: &Linear16) -> ([f64; 3], f64) {
    let paper = image.paper();
    let top = paper.iter().copied().fold(1.0, f64::max);
    (paper.map(|w| w / top), (65535.0 / top).log2())
}

/// Saves a DNG 1.4 linear raw: the samples untouched, nothing clipped.
pub fn save_dng(path: &Path, image: &Linear16) -> Result<()> {
    let srational = |v: f64| ((v * 10000.0).round() as i32, 10000);
    let rational = |v: f64| ((v * 1_000_000.0).round() as u32, 1_000_000);
    let (neutral, exposure) = dng_white(image);
    let tags = vec![
        (254, Value::Long(vec![0])),
        (271, Value::Ascii(image.make.clone())),
        (272, Value::Ascii(image.model.clone())),
        (50706, Value::Byte(vec![1, 4, 0, 0])),
        (50707, Value::Byte(vec![1, 1, 0, 0])),
        (50708, Value::Ascii(image.device())),
        (50717, Value::Long(vec![65535, 65535, 65535])),
        (
            50721,
            Value::SRational(XYZ_TO_SRGB.iter().map(|&v| srational(v)).collect()),
        ),
        (50728, Value::Rational(neutral.map(rational).to_vec())),
        (50730, Value::SRational(vec![srational(exposure)])),
        (50778, Value::Short(vec![21])),
    ];
    write_tiff(path, image.into(), 34892, &image.software, tags)
}

/// Saves a finished 16-bit image (`channels` 3 or 1, drawn through a tone
/// curve of `gamma`) as a TIFF (`.tif`/`.tiff`) or a PNG, with its profile
/// ([`crate::icc`]).
#[allow(clippy::too_many_arguments)]
pub fn save_deep(
    path: &Path,
    width: usize,
    height: usize,
    channels: usize,
    dpi: u16,
    data: &[u16],
    gamma: f64,
    software: &str,
) -> Result<()> {
    let profile = if channels == 1 {
        crate::icc::grey(gamma)
    } else {
        crate::icc::rgb(gamma)
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if ext == "tif" || ext == "tiff" {
        let image = Deep {
            width,
            height,
            channels,
            dpi,
            data,
        };
        let photometric = if channels == 1 { 1 } else { 2 };
        let tags = vec![(34675, Value::Undefined(profile))];
        return write_tiff(path, image, photometric, software, tags);
    }
    let color = if channels == 1 {
        png::ColorType::Grayscale
    } else {
        png::ColorType::Rgb
    };
    let info = crate::png_info(
        width,
        height,
        (color, png::BitDepth::Sixteen),
        dpi,
        Some(profile),
    )?;
    crate::write_file(path, |file| {
        let mut writer = png::Encoder::with_info(file, info)?.write_header()?;
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_be_bytes()).collect();
        writer.write_image_data(&bytes)?;
        writer.finish()?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Linear16 {
        let (w, h) = (37, 23);
        let data = (0..w * h * 3).map(|i| (i * 97 % 65536) as u16).collect();
        Linear16::new(w, h, 300, data)
    }

    #[test]
    fn tiff16_reads_back() {
        let dir = std::env::temp_dir().join("vetroscan-raw-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.tif");
        let image = sample();
        save_tiff16(&path, &image).unwrap();
        let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (37, 23));
        match decoder.read_image().unwrap() {
            tiff::decoder::DecodingResult::U16(v) => assert_eq!(v, image.data),
            _ => panic!("not 16 bits"),
        }
        let icc = decoder
            .get_tag_u8_vec(tiff::tags::Tag::Unknown(34675))
            .unwrap();
        assert_eq!(&icc[36..40], b"acsp");
        assert_eq!(
            u32::from_be_bytes(icc[0..4].try_into().unwrap()) as usize,
            icc.len()
        );
    }

    /// Tags of the first directory: tag, type, count and the 4 bytes of
    /// the value or offset.
    fn directory(file: &[u8]) -> Vec<(u16, u16, u32, [u8; 4])> {
        let u16_at = |i: usize| u16::from_le_bytes([file[i], file[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(file[i..i + 4].try_into().unwrap());
        assert_eq!(&file[0..4], b"II* ");
        let ifd = u32_at(4) as usize;
        (0..usize::from(u16_at(ifd)))
            .map(|k| {
                let e = ifd + 2 + 12 * k;
                (
                    u16_at(e),
                    u16_at(e + 2),
                    u32_at(e + 4),
                    file[e + 8..e + 12].try_into().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn dng_has_its_tags() {
        let dir = std::env::temp_dir().join("vetroscan-raw-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.dng");
        let image = sample().with_source("Test", "Flatbed A3", "VetroScan 9.9");
        save_dng(&path, &image).unwrap();
        let file = std::fs::read(&path).unwrap();
        // The scanner and the program are the ones given, not fixed ones.
        let has = |text: &str| file.windows(text.len()).any(|w| w == text.as_bytes());
        assert!(has("Test Flatbed A3") && has("VetroScan 9.9"));
        assert!(!has("Scanjet"));
        let tags = directory(&file);
        // Sorted, as TIFF requires.
        assert!(tags.windows(2).all(|w| w[0].0 < w[1].0));
        let find = |t: u16| tags.iter().find(|e| e.0 == t).copied().unwrap();
        assert_eq!(find(50706).3, [1, 4, 0, 0]);
        assert_eq!(u16::from_le_bytes([find(262).3[0], find(262).3[1]]), 34892);
        assert_eq!(find(50721).2, 9);
        // Its colours: XYZ to linear sRGB (ColorMatrix1), measured under
        // D65 (CalibrationIlluminant1 21), the white given as neutral.
        assert_eq!(u16::from_le_bytes([find(50778).3[0], find(50778).3[1]]), 21);
        assert_eq!(find(50728).2, 3);
        // The pixels: the first strip starts with the first samples.
        let offset = u32::from_le_bytes(find(273).3) as usize;
        let first = u16::from_le_bytes([file[offset + 2], file[offset + 3]]);
        assert_eq!(first, image.data[1]);
    }

    #[test]
    fn deep_png_and_tiff_read_back() {
        let dir = std::env::temp_dir().join("vetroscan-raw-test");
        std::fs::create_dir_all(&dir).unwrap();
        for channels in [1, 3] {
            let data: Vec<u16> = (0..9 * 5 * channels).map(|i| (i * 500) as u16).collect();
            for name in ["g.png", "g.tif"] {
                let path = dir.join(name);
                save_deep(&path, 9, 5, channels, 300, &data, 1.8, "test").unwrap();
                assert_eq!(read_deep(&path), data, "{name}");
                // The profile of the curve it was drawn with, and no sRGB
                // chunk saying otherwise.
                let profile = crate::icc::read(&deep_profile(&path)).unwrap();
                assert_eq!(profile.gamma, 1.80078125, "{name}");
                let space = if channels == 1 { b"GRAY" } else { b"RGB " };
                assert_eq!(&profile.space, space, "{name}");
            }
        }
    }

    fn deep_profile(path: &Path) -> Vec<u8> {
        if path.extension().unwrap() == "tif" {
            let mut dec = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
            dec.get_tag_u8_vec(tiff::tags::Tag::IccProfile).unwrap()
        } else {
            let dec =
                png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
            let r = dec.read_info().unwrap();
            assert!(r.info().srgb.is_none());
            r.info().icc_profile.as_ref().unwrap().to_vec()
        }
    }

    fn read_deep(path: &Path) -> Vec<u16> {
        if path.extension().unwrap() == "tif" {
            let mut dec = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
            match dec.read_image().unwrap() {
                tiff::decoder::DecodingResult::U16(v) => v,
                _ => panic!("not 16 bits"),
            }
        } else {
            let dec =
                png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
            let mut r = dec.read_info().unwrap();
            let mut buf = vec![0; r.output_buffer_size().unwrap()];
            let info = r.next_frame(&mut buf).unwrap();
            buf[..info.buffer_size()]
                .chunks_exact(2)
                .map(|w| u16::from_be_bytes([w[0], w[1]]))
                .collect()
        }
    }

    #[test]
    fn from_linear8_widens_only() {
        let image = Image {
            width: 2,
            height: 1,
            dpi: 300,
            format: Format::Rgb,
            data: vec![196, 186, 184, 255, 0, 92],
        };
        let raw = Linear16::from_linear8(&image, [196.0, 186.0, 184.0]).unwrap();
        assert_eq!(
            raw.data,
            vec![196 * 257, 186 * 257, 184 * 257, 65535, 0, 92 * 257]
        );
        assert_eq!(raw.white, [196.0 * 257.0, 186.0 * 257.0, 184.0 * 257.0]);
    }

    /// The chip's 16-bit samples, paper white as in the app.
    fn chip() -> Linear16 {
        let white = [196.0 * 256.0, 186.0 * 256.0, 184.0 * 256.0];
        // Paper, then a highlight brighter than paper, then the chip's top.
        let samples = vec![
            50176, 47616, 47104, 60000, 58000, 56000, 65535, 65535, 65535,
        ];
        Linear16::from_linear16(3, 1, 300, &samples, white)
    }

    #[test]
    fn dng_keeps_highlights_above_paper() {
        let dir = std::env::temp_dir().join("vetroscan-raw-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h.dng");
        let image = chip();
        save_dng(&path, &image).unwrap();
        let file = std::fs::read(&path).unwrap();
        let tags = directory(&file);
        let find = |t: u16| tags.iter().find(|e| e.0 == t).copied().unwrap();
        let u32_at = |i: usize| u32::from_le_bytes(file[i..i + 4].try_into().unwrap());
        // The samples, all nine, as the chip gave them.
        let offset = u32::from_le_bytes(find(273).3) as usize;
        let written: Vec<u16> = file[offset..offset + 18]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(written, image.data);
        // Neutral: the paper white of each channel, red the brightest.
        let at = u32::from_le_bytes(find(50728).3) as usize;
        let neutral: Vec<f64> = (0..3)
            .map(|k| f64::from(u32_at(at + 8 * k)) / f64::from(u32_at(at + 8 * k + 4)))
            .collect();
        assert!((neutral[0] - 1.0).abs() < 1e-6);
        assert!((neutral[1] - 186.0 / 196.0).abs() < 1e-5);
        assert!((neutral[2] - 184.0 / 196.0).abs() < 1e-5);
        // Exposure: from the red paper white to full scale.
        let at = u32::from_le_bytes(find(50730).3) as usize;
        let ev = f64::from(u32_at(at) as i32) / f64::from(u32_at(at + 4) as i32);
        assert!((ev - (65535.0f64 / 50176.0).log2()).abs() < 1e-4, "{ev}");
    }

    #[test]
    fn tiff16_keeps_highlights_above_paper() {
        let dir = std::env::temp_dir().join("vetroscan-raw-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h.tif");
        let image = chip();
        save_tiff16(&path, &image).unwrap();
        let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(&path).unwrap()).unwrap();
        match decoder.read_image().unwrap() {
            tiff::decoder::DecodingResult::U16(v) => assert_eq!(v, image.data),
            _ => panic!("not 16 bits"),
        }
    }
}
