// SPDX-License-Identifier: GPL-2.0-or-later
//! PDF documents of scanned pages.
//!
//! One image per page. Colour and grey pages are embedded as JPEG (the
//! PDF `DCTDecode` filter, which every reader decodes), black and white
//! pages as 1-bit images with `FlateDecode`, lossless and small. The page
//! is either the size of the scan (so it prints at its real size) or a
//! sheet of paper ([`Layout`]): A4, Letter, a photo size…, turned the way
//! the scan is, with the scan at its real size, fitted to the sheet or
//! filling it, inside a margin.

use crate::compress::{encode, prepare, Encoding};
use crate::Result;
use scan_imaging::Image;
use std::io::Write;

/// Size of the pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageSize {
    /// The size of each scan at its resolution.
    #[default]
    Fit,
    /// A3, 297 × 420 mm.
    A3,
    /// A4, 210 × 297 mm.
    A4,
    /// A5, 148 × 210 mm.
    A5,
    /// US Letter, 8.5 × 11 in.
    Letter,
    /// US Legal, 8.5 × 14 in.
    Legal,
    /// Photo paper, 10 × 15 cm.
    Photo10x15,
    /// Photo paper, 13 × 18 cm.
    Photo13x18,
}

impl PageSize {
    /// The sheet upright, width and height in millimetres (none: the size
    /// of the scan).
    pub fn millimetres(self) -> Option<(f64, f64)> {
        Some(match self {
            Self::Fit => return None,
            Self::A3 => (297.0, 420.0),
            Self::A4 => (210.0, 297.0),
            Self::A5 => (148.0, 210.0),
            Self::Letter => (215.9, 279.4),
            Self::Legal => (215.9, 355.6),
            Self::Photo10x15 => (100.0, 150.0),
            Self::Photo13x18 => (127.0, 178.0),
        })
    }
}

/// Where the scan goes on the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// At its real size, centred; shrunk only if it is larger than the
    /// sheet.
    #[default]
    Actual,
    /// As large as the sheet allows inside the margin, whole: enlarged or
    /// shrunk, proportions kept.
    FitPage,
    /// Covering the sheet inside the margin: what goes beyond it is cut
    /// (as photo prints are).
    Fill,
}

/// Which way the sheet is turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// Upright for scans taller than wide, sideways for the others.
    #[default]
    Auto,
    /// Upright, whatever the scan (a landscape scan across the top).
    Portrait,
    /// Sideways, whatever the scan.
    Landscape,
}

/// The sheet of the pages and where the scan goes on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub size: PageSize,
    pub orientation: Orientation,
    pub placement: Placement,
    /// White border kept on every side, millimetres, when the scan is
    /// fitted to the sheet or fills it (not at real size, nor with
    /// [`PageSize::Fit`]).
    pub margin_mm: f64,
    /// When the scan fills the sheet and overflows it: which part is kept,
    /// across and down, 0 to 1 (0: the left or top edge, 0.5: the middle).
    pub focus: [f64; 2],
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            size: PageSize::default(),
            orientation: Orientation::default(),
            placement: Placement::default(),
            margin_mm: 0.0,
            focus: [0.5, 0.5],
        }
    }
}

impl From<PageSize> for Layout {
    fn from(size: PageSize) -> Self {
        Self {
            size,
            ..Self::default()
        }
    }
}

/// A scan laid out on its page, in points (1/72 in, origin bottom left).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// Width and height of the page.
    pub page: (f64, f64),
    /// Where the image is drawn: left, bottom, width, height.
    pub image: [f64; 4],
    /// What is shown of it, when it overflows (Fill): left, bottom, width,
    /// height.
    pub clip: Option<[f64; 4]>,
}

const MM: f64 = 72.0 / 25.4;

impl Layout {
    /// Lays out an image of `w` × `h` inches.
    pub fn place(&self, w: f64, h: f64) -> Placed {
        let (iw, ih) = (w * 72.0, h * 72.0);
        let Some((sw, sh)) = self.size.millimetres() else {
            return Placed {
                page: (iw, ih),
                image: [0.0, 0.0, iw, ih],
                clip: None,
            };
        };
        // The sheet turned the way the scan is, or as asked.
        let landscape = match self.orientation {
            Orientation::Auto => w > h,
            Orientation::Portrait => false,
            Orientation::Landscape => true,
        };
        let (sw, sh) = if landscape {
            (sw.max(sh), sw.min(sh))
        } else {
            (sw.min(sh), sw.max(sh))
        };
        let (pw, ph) = (sw * MM, sh * MM);
        // The margin, at most a third of the shorter side; none at real size
        // (an A4 page on A4 stays as it is).
        let m = match self.placement {
            Placement::Actual => 0.0,
            _ => (self.margin_mm.max(0.0) * MM).min(pw.min(ph) / 3.0),
        };
        let (aw, ah) = (pw - 2.0 * m, ph - 2.0 * m);
        let fit = (aw / iw).min(ah / ih);
        let scale = match self.placement {
            Placement::Actual => fit.min(1.0),
            Placement::FitPage => fit,
            Placement::Fill => (aw / iw).max(ah / ih),
        };
        let (dw, dh) = (iw * scale, ih * scale);
        // Centred; overflowing (Fill), moved so that the part chosen shows
        // (y from the bottom: the top of the scan is at the top).
        let [fx, fy] = self.focus.map(|f| f.clamp(0.0, 1.0));
        let x = if dw > aw + 0.01 {
            m + (aw - dw) * fx
        } else {
            (pw - dw) / 2.0
        };
        let y = if dh > ah + 0.01 {
            m + (ah - dh) * (1.0 - fy)
        } else {
            (ph - dh) / 2.0
        };
        let overflows = dw > aw + 0.01 || dh > ah + 0.01;
        Placed {
            page: (pw, ph),
            image: [x, y, dw, dh],
            clip: overflows.then_some([m, m, aw, ah]),
        }
    }
}

/// Writes `pages` as a PDF document to `out`, all on `layout`'s sheet,
/// pictures stored as JPEG of `jpeg_quality` (black and white: lossless).
pub fn write<W: Write>(
    out: &mut W,
    pages: &[Image],
    layout: impl Into<Layout>,
    jpeg_quality: u8,
) -> Result<()> {
    let layout = layout.into();
    let encoding = Encoding {
        quality: jpeg_quality,
        ..Encoding::default()
    };
    let pages: Vec<PdfPage> = pages
        .iter()
        .map(|image| PdfPage {
            image,
            layout,
            encoding,
        })
        .collect();
    write_pages(out, &pages)
}

/// A page of a PDF: the finished scan, its sheet and how its picture is
/// stored.
#[derive(Debug, Clone, Copy)]
pub struct PdfPage<'a> {
    pub image: &'a Image,
    pub layout: Layout,
    pub encoding: Encoding,
}

/// Writes `pages` as a PDF document to `out`, each on its own sheet and
/// with its own encoding.
pub fn write_pages<W: Write>(out: &mut W, pages: &[PdfPage]) -> Result<()> {
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
    for (i, page) in pages.iter().enumerate() {
        let (image, layout) = (page.image, page.layout);
        let id = page_id(i);
        let dpi = f64::from(image.dpi.max(1));
        let (iw, ih) = (image.width as f64 / dpi, image.height as f64 / dpi);
        let Placed {
            page: (pw, ph),
            image: [x, y, dw, dh],
            clip,
        } = layout.place(iw, ih);
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
        // What overflows the sheet (Fill) is clipped to the margin.
        let clip = clip.map_or(String::new(), |[cx, cy, cw, ch]| {
            format!("{cx:.3} {cy:.3} {cw:.3} {ch:.3} re W n ")
        });
        let content = format!("q {clip}{dw:.3} 0 0 {dh:.3} {x:.3} {y:.3} cm /Im0 Do Q");
        object(
            &mut pdf,
            id + 1,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .as_bytes(),
        );
        // On the sheet at the size of the scan; the picture maybe smaller.
        let (dict, data) = encode(&prepare(image, &page.encoding), page.encoding.quality)?;
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
    use scan_imaging::Format;

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

    /// Close enough, in points.
    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.05
    }

    #[test]
    fn a_small_scan_stays_at_its_size_on_a4() {
        // 4 × 6 inches, real size, centred.
        let p = Layout::from(PageSize::A4).place(4.0, 6.0);
        assert!(near(p.page.0, 595.28) && near(p.page.1, 841.89));
        assert!(near(p.image[2], 288.0) && near(p.image[3], 432.0));
        assert!(near(p.image[0], (595.28 - 288.0) / 2.0));
        assert_eq!(p.clip, None);
    }

    #[test]
    fn fitted_to_the_page_a_crop_grows_to_the_margin() {
        // A 2 × 3 inch crop on A4 with 10 mm all round: as wide as the
        // sheet less the margins (the height is the limit: 3:2 is taller
        // than A4's 1.414).
        let layout = Layout {
            size: PageSize::A4,
            orientation: Orientation::Auto,
            placement: Placement::FitPage,
            margin_mm: 10.0,
            focus: [0.5, 0.5],
        };
        let p = layout.place(2.0, 3.0);
        let (aw, ah) = (595.276 - 2.0 * 10.0 * MM, 841.89 - 2.0 * 10.0 * MM);
        assert!(near(p.image[3], ah), "{p:?}");
        assert!(p.image[2] < aw);
        assert!(near(p.image[2] / p.image[3], 2.0 / 3.0));
        assert_eq!(p.clip, None);
    }

    #[test]
    fn a_landscape_scan_turns_the_sheet() {
        let p = Layout::from(PageSize::A4).place(9.0, 6.0);
        assert!(p.page.0 > p.page.1, "{p:?}");
    }

    #[test]
    fn at_real_size_the_margin_cuts_nothing() {
        // A full A4 page on A4 with a margin set: as it is, no clip.
        let layout = Layout {
            size: PageSize::A4,
            margin_mm: 10.0,
            ..Layout::default()
        };
        let p = layout.place(210.0 / 25.4, 297.0 / 25.4);
        assert_eq!(p.clip, None, "{p:?}");
        assert!(near(p.image[2], 595.276) && near(p.image[0], 0.0), "{p:?}");
    }

    #[test]
    fn the_part_kept_of_a_filled_sheet_is_chosen() {
        // A wide scan filling an upright 10 x 15: it overflows across.
        let layout = |fx| Layout {
            size: PageSize::Photo10x15,
            orientation: Orientation::Portrait,
            placement: Placement::Fill,
            margin_mm: 0.0,
            focus: [fx, 0.5],
        };
        let left = layout(0.0).place(6.0, 4.0);
        let middle = layout(0.5).place(6.0, 4.0);
        let right = layout(1.0).place(6.0, 4.0);
        // The left edge of the scan at the left edge of the sheet...
        assert!(near(left.image[0], 0.0), "{left:?}");
        // ...its right edge at the right edge of the sheet...
        assert!(
            near(right.image[0] + right.image[2], right.page.0),
            "{right:?}"
        );
        // ...or centred.
        assert!(
            near(middle.image[0] * 2.0 + middle.image[2], middle.page.0),
            "{middle:?}"
        );
        assert!(left.clip.is_some());
    }

    #[test]
    fn a_landscape_scan_on_a_sheet_kept_upright() {
        let layout = Layout {
            size: PageSize::A4,
            orientation: Orientation::Portrait,
            placement: Placement::FitPage,
            margin_mm: 10.0,
            focus: [0.5, 0.5],
        };
        let p = layout.place(9.0, 6.0);
        assert!(p.page.0 < p.page.1, "{p:?}");
        // As wide as the sheet less the margins, across the top half.
        assert!(near(p.image[2], 595.276 - 20.0 * MM), "{p:?}");
        let sideways = Layout {
            orientation: Orientation::Landscape,
            ..layout
        };
        assert!(sideways.place(6.0, 9.0).page.0 > sideways.place(6.0, 9.0).page.1);
    }

    #[test]
    fn each_page_on_its_own_sheet() {
        // The sample is wider than tall: kept upright on the first page.
        let upright = Layout {
            orientation: Orientation::Portrait,
            ..Layout::from(PageSize::A4)
        };
        let sideways = Layout {
            orientation: Orientation::Landscape,
            ..upright
        };
        let image = sample(Format::Grey);
        let mut out = Vec::new();
        let page = |layout| PdfPage {
            image: &image,
            layout,
            encoding: Encoding::default(),
        };
        write_pages(&mut out, &[page(upright), page(sideways)]).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("/MediaBox [0 0 595.28 841.89]"), "{text}");
        assert!(text.contains("/MediaBox [0 0 841.89 595.28]"), "{text}");
    }

    #[test]
    fn filling_a_photo_sheet_cuts_what_overflows() {
        // A square on 10 × 15 cm: covers the sheet, cut at the sides.
        let layout = Layout {
            size: PageSize::Photo10x15,
            orientation: Orientation::Auto,
            placement: Placement::Fill,
            margin_mm: 0.0,
            focus: [0.5, 0.5],
        };
        let p = layout.place(4.0, 4.0);
        assert!(near(p.image[3], p.image[2]));
        assert!(near(p.image[3], 150.0 * MM), "{p:?}");
        let clip = p.clip.expect("clipped");
        assert!(near(clip[2], 100.0 * MM) && near(clip[3], 150.0 * MM));
    }

    #[test]
    fn a_filled_page_clips_in_the_pdf() {
        let layout = Layout {
            size: PageSize::A4,
            orientation: Orientation::Auto,
            placement: Placement::Fill,
            margin_mm: 5.0,
            focus: [0.5, 0.5],
        };
        let mut out = Vec::new();
        write(&mut out, &[sample(Format::Grey)], layout, 85).unwrap();
        assert!(String::from_utf8_lossy(&out).contains(" re W n "));
    }

    #[test]
    fn fit_page_is_the_scan_at_its_size() {
        // 16 x 8 pixels at 300 dpi: 3.84 x 1.92 points.
        let mut out = Vec::new();
        write(&mut out, &[sample(Format::Grey)], PageSize::Fit, 85).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("/MediaBox [0 0 3.84 1.92]"));
    }
}
