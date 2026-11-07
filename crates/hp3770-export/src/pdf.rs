// SPDX-License-Identifier: GPL-2.0-or-later
//! PDF documents of scanned pages.
//!
//! One image per page. Colour and grey pages are embedded as JPEG (the
//! PDF `DCTDecode` filter, which every reader decodes), black and white
//! pages as 1-bit images with `FlateDecode`, lossless and small. The page
//! is either the size of the scan (so it prints at its real size) or A4 /
//! Letter with the scan centred on it, never enlarged.

use crate::Result;
use hp3770_core::session::{Format, Image};
use std::io::Write;

/// Size of the pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageSize {
    /// The size of each scan at its resolution.
    #[default]
    Fit,
    /// A4, 210 × 297 mm.
    A4,
    /// US Letter, 8.5 × 11 in.
    Letter,
}

impl PageSize {
    /// Width and height in points (1/72 in) for an image of `w` × `h`
    /// inches.
    fn points(self, w: f64, h: f64) -> (f64, f64) {
        match self {
            Self::Fit => (w * 72.0, h * 72.0),
            Self::A4 => (595.276, 841.89),
            Self::Letter => (612.0, 792.0),
        }
    }
}

/// An image as embedded in the PDF: its dictionary entries and data.
fn image_object(image: &Image, jpeg_quality: u8) -> Result<(String, Vec<u8>)> {
    use image::ImageEncoder;
    let (w, h) = (image.width, image.height);
    match image.format {
        Format::Rgb | Format::Grey => {
            let (space, color) = if image.format == Format::Rgb {
                ("/DeviceRGB", image::ExtendedColorType::Rgb8)
            } else {
                ("/DeviceGray", image::ExtendedColorType::L8)
            };
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, jpeg_quality)
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

/// Writes `pages` as a PDF document to `out`.
pub fn write<W: Write>(
    out: &mut W,
    pages: &[Image],
    size: PageSize,
    jpeg_quality: u8,
) -> Result<()> {
    if pages.is_empty() {
        return Err("no pages".into());
    }
    let mut pdf = Vec::new();
    let mut offsets = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");
    // Objects: 1 catalog, 2 page tree, then 3 per page (page, contents,
    // image).
    let page_id = |i: usize| 3 + 3 * i;
    let mut object = |pdf: &mut Vec<u8>, id: usize, body: &[u8]| {
        offsets.push((id, pdf.len()));
        pdf.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    };
    object(&mut pdf, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", page_id(i)))
        .collect();
    object(
        &mut pdf,
        2,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        )
        .as_bytes(),
    );
    for (i, image) in pages.iter().enumerate() {
        let id = page_id(i);
        let dpi = f64::from(image.dpi.max(1));
        let (iw, ih) = (image.width as f64 / dpi, image.height as f64 / dpi);
        let (pw, ph) = size.points(iw, ih);
        // The image at its real size, shrunk to fit the page if needed.
        let scale = (pw / (iw * 72.0)).min(ph / (ih * 72.0)).min(1.0);
        let (dw, dh) = (iw * 72.0 * scale, ih * 72.0 * scale);
        let (x, y) = ((pw - dw) / 2.0, (ph - dh) / 2.0);
        object(
            &mut pdf,
            id,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] \
                 /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
                id + 2,
                id + 1
            )
            .as_bytes(),
        );
        let content = format!("q {dw:.3} 0 0 {dh:.3} {x:.3} {y:.3} cm /Im0 Do Q");
        object(
            &mut pdf,
            id + 1,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .as_bytes(),
        );
        let (dict, data) = image_object(image, jpeg_quality)?;
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image {dict} /Length {} >>\nstream\n",
            data.len()
        )
        .into_bytes();
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\nendstream");
        object(&mut pdf, id + 2, &body);
    }
    let count = 3 + 3 * pages.len();
    offsets.sort_unstable();
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {count}\n0000000000 65535 f \n").as_bytes());
    for (_, offset) in &offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out.write_all(&pdf)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::sample;

    #[test]
    fn document_structure() {
        let pages = [
            sample(Format::Rgb),
            sample(Format::Lineart),
            sample(Format::Grey),
        ];
        let mut out = Vec::new();
        write(&mut out, &pages, PageSize::A4, 85).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 3"));
        assert_eq!(text.matches("/Type /Page ").count(), 3);
        assert_eq!(text.matches("/DCTDecode").count(), 2);
        assert_eq!(text.matches("/FlateDecode").count(), 1);
        // Every xref offset points at its object (on the bytes: the
        // file is binary).
        let find = |hay: &[u8], needle: &[u8]| {
            hay.windows(needle.len())
                .rposition(|w| w == needle)
                .unwrap()
        };
        let start = find(&out, b"startxref\n") + 10;
        let end = start + out[start..].iter().position(|&b| b == b'\n').unwrap();
        let xref_at: usize = std::str::from_utf8(&out[start..end])
            .unwrap()
            .parse()
            .unwrap();
        let table = std::str::from_utf8(&out[xref_at..start]).unwrap();
        for (id, line) in table.lines().skip(3).take(11).enumerate() {
            let offset: usize = line[..10].parse().unwrap();
            let expect = format!("{} 0 obj", id + 1);
            assert!(
                out[offset..].starts_with(expect.as_bytes()),
                "object {}",
                id + 1
            );
        }
    }

    #[test]
    fn fit_page_is_the_scan_at_its_size() {
        // 16 x 8 pixels at 300 dpi: 3.84 x 1.92 points.
        let mut out = Vec::new();
        write(&mut out, &[sample(Format::Grey)], PageSize::Fit, 85).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("/MediaBox [0 0 3.84 1.92]"));
    }
}
