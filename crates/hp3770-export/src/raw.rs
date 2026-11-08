// SPDX-License-Identifier: GPL-2.0-or-later
//! The scanner's own samples, for raw editors: 16-bit linear TIFF and DNG.
//!
//! Nothing of the adjustments is applied: the samples are linear in light,
//! scaled so that the white the calibration aims at is full scale. The
//! colour space of the 3770's filters is close to sRGB's primaries, so
//! both files say so:
//!
//! * the TIFF embeds an ICC profile "sRGB lineare" (sRGB primaries, D65
//!   adapted to D50 as ICC asks, tone curve gamma 1), which Photoshop and
//!   other colour-managed programs read;
//! * the DNG (Adobe's raw format, version 1.4) is a *linear raw*
//!   (photometric `LinearRaw`): already three colours a pixel, with the
//!   matrix from XYZ to these colours and a neutral white, so Lightroom
//!   and Camera Raw develop it like a camera's raw file.
//!
//! The writer is a small little-endian TIFF writer of one image (the tiff
//! crate cannot write DNG's own tags).

use crate::Result;
use hp3770_core::session::{Format, Image};
use std::io::Write;
use std::path::Path;

/// Linear RGB samples, 16 bits each.
#[derive(Debug, Clone)]
pub struct Linear16 {
    pub width: usize,
    pub height: usize,
    pub dpi: u16,
    /// Red, green, blue for each pixel, line after line.
    pub data: Vec<u16>,
}

impl Linear16 {
    /// From the scanner's linear 8-bit samples: `white` (per channel) to
    /// full scale. Brighter samples (specular highlights) are clipped.
    pub fn from_linear8(image: &Image, white: [f64; 3]) -> Result<Self> {
        if image.format != Format::Rgb {
            return Err("il RAW è solo a colori".into());
        }
        let scale = white.map(|w| 65535.0 / w.max(1.0));
        let data = image
            .data
            .iter()
            .enumerate()
            .map(|(i, &v)| (f64::from(v) * scale[i % 3]).round().min(65535.0) as u16)
            .collect();
        Ok(Self {
            width: image.width,
            height: image.height,
            dpi: image.dpi,
            data,
        })
    }

    /// From the chip's 16-bit linear samples: `white` (per channel, in
    /// 16-bit units) to full scale.
    pub fn from_linear16(
        width: usize,
        height: usize,
        dpi: u16,
        samples: &[u16],
        white: [f64; 3],
    ) -> Self {
        let scale = white.map(|w| 65535.0 / w.max(1.0));
        let data = samples
            .iter()
            .enumerate()
            .map(|(i, &v)| (f64::from(v) * scale[i % 3]).round().min(65535.0) as u16)
            .collect();
        Self {
            width,
            height,
            dpi,
            data,
        }
    }

    /// From 16-bit linear samples already scaled to full scale.
    pub fn new(width: usize, height: usize, dpi: u16, data: Vec<u16>) -> Self {
        Self {
            width,
            height,
            dpi,
            data,
        }
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
    mut tags: Vec<(u16, Value)>,
) -> Result<()> {
    let (w, h, ch) = (image.width, image.height, image.channels);
    if w == 0 || h == 0 || image.data.len() != w * h * ch {
        return Err("immagine vuota o incoerente".into());
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
        return Err("immagine troppo grande per un TIFF (oltre 4 GB)".into());
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
        (305, Value::Ascii("HP Scanjet 3770 (driver libero)".into())),
    ]);
    tags.sort_by_key(|(t, _)| *t);

    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
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
    file.flush()?;
    Ok(())
}

// --------------------------------------------------------------- formats

/// XYZ (D65) to linear sRGB (IEC 61966-2-1).
const XYZ_TO_SRGB: [f64; 9] = [
    3.2406, -1.5372, -0.4986, //
    -0.9689, 1.8758, 0.0415, //
    0.0557, -0.2040, 1.0570,
];

/// Linear sRGB to XYZ, adapted to D50 (Bradford), the primaries of an ICC
/// profile.
const SRGB_D50: [[f64; 3]; 3] = [
    [0.4361, 0.2225, 0.0139], // red: X, Y, Z
    [0.3851, 0.7169, 0.0971], // green
    [0.1431, 0.0606, 0.7141], // blue
];

/// An ICC v2 display profile: sRGB primaries, tone curve gamma 1.
fn linear_srgb_icc() -> Vec<u8> {
    fn s15(v: f64) -> [u8; 4] {
        ((v * 65536.0).round() as i32).to_be_bytes()
    }
    fn xyz(v: [f64; 3]) -> Vec<u8> {
        let mut out = b"XYZ \0\0\0\0".to_vec();
        for c in v {
            out.extend_from_slice(&s15(c));
        }
        out
    }
    fn text(s: &str) -> Vec<u8> {
        let mut out = b"text\0\0\0\0".to_vec();
        out.extend_from_slice(s.as_bytes());
        out.push(0);
        out
    }
    fn desc(s: &str) -> Vec<u8> {
        let mut out = b"desc\0\0\0\0".to_vec();
        out.extend_from_slice(&(s.len() as u32 + 1).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
        out.push(0);
        out.extend_from_slice(&[0; 4 + 4 + 2 + 1 + 67]);
        out
    }
    // Gamma 1.0 (u8Fixed8 256).
    let curve = b"curv\0\0\0\0\0\0\0\x01\x01\x00".to_vec();
    let elements: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", desc("sRGB lineare")),
        (b"cprt", text("Nessun copyright")),
        (b"wtpt", xyz([0.9642, 1.0, 0.8249])),
        (b"rXYZ", xyz(SRGB_D50[0])),
        (b"gXYZ", xyz(SRGB_D50[1])),
        (b"bXYZ", xyz(SRGB_D50[2])),
        (b"rTRC", curve.clone()),
        (b"gTRC", curve.clone()),
        (b"bTRC", curve),
    ];
    let table_len = 4 + 12 * elements.len();
    let mut offset = 128 + table_len;
    let mut table = (elements.len() as u32).to_be_bytes().to_vec();
    let mut body = Vec::new();
    for (sig, data) in &elements {
        while (offset + body.len()) % 4 != 0 {
            body.push(0);
        }
        let at = offset + body.len();
        table.extend_from_slice(*sig);
        table.extend_from_slice(&(at as u32).to_be_bytes());
        table.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
    }
    offset += body.len();
    let size = offset as u32;
    let mut header = vec![0u8; 128];
    header[0..4].copy_from_slice(&size.to_be_bytes());
    header[8..12].copy_from_slice(&[0x02, 0x10, 0, 0]); // version 2.1
    header[12..16].copy_from_slice(b"mntr");
    header[16..20].copy_from_slice(b"RGB ");
    header[20..24].copy_from_slice(b"XYZ ");
    header[36..40].copy_from_slice(b"acsp");
    // Rendering intent 0, illuminant D50.
    header[68..72].copy_from_slice(&s15(0.9642));
    header[72..76].copy_from_slice(&s15(1.0));
    header[76..80].copy_from_slice(&s15(0.8249));
    let mut out = header;
    out.extend_from_slice(&table);
    out.extend_from_slice(&body);
    out
}

/// Saves a 16-bit linear TIFF with the "sRGB lineare" profile.
pub fn save_tiff16(path: &Path, image: &Linear16) -> Result<()> {
    write_tiff(
        path,
        image.into(),
        2,
        vec![(34675, Value::Undefined(linear_srgb_icc()))],
    )
}

/// Saves a DNG 1.4 linear raw.
pub fn save_dng(path: &Path, image: &Linear16) -> Result<()> {
    let srational = |v: f64| ((v * 10000.0).round() as i32, 10000);
    let tags = vec![
        (254, Value::Long(vec![0])),
        (271, Value::Ascii("Hewlett-Packard".into())),
        (272, Value::Ascii("Scanjet 3770".into())),
        (50706, Value::Byte(vec![1, 4, 0, 0])),
        (50707, Value::Byte(vec![1, 1, 0, 0])),
        (50708, Value::Ascii("HP Scanjet 3770".into())),
        (50717, Value::Long(vec![65535, 65535, 65535])),
        (
            50721,
            Value::SRational(XYZ_TO_SRGB.iter().map(|&v| srational(v)).collect()),
        ),
        (50728, Value::Rational(vec![(1, 1), (1, 1), (1, 1)])),
        (50778, Value::Short(vec![21])),
    ];
    write_tiff(path, image.into(), 34892, tags)
}

/// Saves a finished 16-bit image (sRGB tones, `channels` 3 or 1) as a TIFF
/// (`.tif`/`.tiff`) or a PNG.
pub fn save_deep(
    path: &Path,
    width: usize,
    height: usize,
    channels: usize,
    dpi: u16,
    data: &[u16],
) -> Result<()> {
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
        return write_tiff(path, image, photometric, Vec::new());
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, u32::try_from(width)?, u32::try_from(height)?);
    encoder.set_color(if channels == 1 {
        png::ColorType::Grayscale
    } else {
        png::ColorType::Rgb
    });
    encoder.set_depth(png::BitDepth::Sixteen);
    let ppm = (f64::from(dpi) / 0.0254).round() as u32;
    encoder.set_pixel_dims(Some(png::PixelDimensions {
        xppu: ppm,
        yppu: ppm,
        unit: png::Unit::Meter,
    }));
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header()?;
    let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_be_bytes()).collect();
    writer.write_image_data(&bytes)?;
    writer.finish()?;
    Ok(())
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
        let dir = std::env::temp_dir().join("hp3770-raw-test");
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
        let dir = std::env::temp_dir().join("hp3770-raw-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.dng");
        let image = sample();
        save_dng(&path, &image).unwrap();
        let file = std::fs::read(&path).unwrap();
        let tags = directory(&file);
        // Sorted, as TIFF requires.
        assert!(tags.windows(2).all(|w| w[0].0 < w[1].0));
        let find = |t: u16| tags.iter().find(|e| e.0 == t).copied().unwrap();
        assert_eq!(find(50706).3, [1, 4, 0, 0]);
        assert_eq!(u16::from_le_bytes([find(262).3[0], find(262).3[1]]), 34892);
        assert_eq!(find(50721).2, 9);
        // The pixels: the first strip starts with the first samples.
        let offset = u32::from_le_bytes(find(273).3) as usize;
        let first = u16::from_le_bytes([file[offset + 2], file[offset + 3]]);
        assert_eq!(first, image.data[1]);
    }

    #[test]
    fn deep_png_and_tiff_read_back() {
        let dir = std::env::temp_dir().join("hp3770-raw-test");
        std::fs::create_dir_all(&dir).unwrap();
        let data: Vec<u16> = (0..9 * 5).map(|i| (i * 1000) as u16).collect();
        for name in ["g.png", "g.tif"] {
            let path = dir.join(name);
            save_deep(&path, 9, 5, 1, 300, &data).unwrap();
            assert_eq!(read_deep(&path), data, "{name}");
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
    fn from_linear8_scales_white_to_full() {
        let image = Image {
            width: 2,
            height: 1,
            dpi: 300,
            format: Format::Rgb,
            data: vec![196, 186, 184, 255, 0, 92],
        };
        let raw = Linear16::from_linear8(&image, [196.0, 186.0, 184.0]).unwrap();
        assert_eq!(raw.data, vec![65535, 65535, 65535, 65535, 0, 32768]);
    }
}
