// SPDX-License-Identifier: GPL-2.0-or-later
//! `hp3770-app`: desktop application for the HP Scanjet 3770.
//!
//! Two ways of working, chosen first: **Documento** (one or more pages,
//! exported as PDF, multi-page TIFF or one image per page) and **Foto**
//! (one image per scan). The viewer on the left shows the preview of the
//! glass (where the area is drawn) or the scanned page; the panel on the
//! right holds the scan settings, the adjustments (applied live, never
//! changing the scanner's data) and the output.
//!
//! The scanner's linear samples are kept: adjustments are redrawn on the
//! preview and on a screen-sized copy of each page as the sliders move, and
//! applied at full resolution when saving. Every scanner request goes
//! through [`worker`].

#![windows_subsystem = "windows"]

mod access;
mod curve;
#[cfg(test)]
mod e2e;
mod editor;
mod histogram;
mod prefs;
mod sheet;
mod testing;
mod viewer;
mod wia;
mod worker;

use editor::{Edit, EditMsg, Editor, Section};
use histogram::Histogram;
use hp3770_export::{ImageFormat, PageSize};
use iced::widget::{
    button, canvas, center, checkbox, column, container, mouse_area, opaque, pick_list,
    progress_bar, row, rule, scrollable, slider, stack, text, Space,
};
use iced::{Alignment, Element, Length, Point, Size, Subscription, Task, Theme};
use scan_imaging::adjust::{self, Adjustments};
use scan_imaging::convert;
use scan_imaging::detect::{self, Found};
use scan_imaging::transform::Transform;
use scan_imaging::{Format, Image, Mode};
use scanner_api::{
    Area, Capabilities, DeviceInfo, DeviceStatus, LampState, Progress, ScanRequest,
    SuggestedAction, Tone,
};
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use viewer::{Picture, Selection, View};
use worker::{Command, Event, Handle, Purpose};

/// Resolution of the preview of the whole glass (the lowest the scanner
/// has).
const PREVIEW_DPI: u16 = 200;

/// Above this size (bytes in memory) the interface warns before scanning.
const LARGE_SCAN: f64 = 400e6;

/// Longest side of the copy of a page on which adjustments are shown live.
const PROXY: usize = 2400;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("HP Scanjet 3770")
        .subscription(App::subscription)
        .theme(App::theme)
        .window(window_settings())
        .run()
}

/// The main window. On Linux its application id is the name of the
/// `.desktop` file in `packaging/linux`, which is how Wayland compositors
/// give the window its name and icon.
fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: Size::new(1360.0, 900.0),
        // The title bar's and the taskbar's icon (the .exe has its own,
        // from build.rs, for Explorer).
        icon: iced::window::icon::from_file_data(
            include_bytes!("../../../packaging/icons/hp3770-app-256.png"),
            None,
        )
        .ok(),
        // Closing asks first when scans are not saved (Message::CloseRequested).
        exit_on_close_request: false,
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: "hp3770-app".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------- choices

/// What is being scanned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// One or more pages, one document.
    Document,
    /// One image per scan.
    Photo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Colour {
    Rgb,
    Grey,
    Lineart,
}

const ALL_COLOURS: [Colour; 3] = [Colour::Rgb, Colour::Grey, Colour::Lineart];

impl std::fmt::Display for Colour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Rgb => "Colore",
            Self::Grey => "Grigio",
            Self::Lineart => "Bianco e nero",
        })
    }
}

/// Output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Output {
    Pdf,
    TiffPages,
    Image(ImageFormat),
    /// The scanner's linear samples, without adjustments.
    Raw(RawFormat),
    /// The finished image at 16 bits per channel (scans made at 16 bits).
    Deep(DeepFormat),
}

/// Image files with 16 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeepFormat {
    Png,
    Tiff,
}

/// Files of the scanner's own samples, for raw editors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawFormat {
    /// 16-bit linear TIFF with a linear sRGB profile.
    Tiff16,
    /// Adobe DNG, linear raw.
    Dng,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pdf => f.write_str("PDF"),
            Self::TiffPages => f.write_str("TIFF multipagina"),
            Self::Image(format) => write!(f, "{format}"),
            Self::Raw(RawFormat::Tiff16) => f.write_str("RAW · TIFF 16 bit lineare"),
            Self::Deep(DeepFormat::Png) => f.write_str("PNG 16 bit"),
            Self::Deep(DeepFormat::Tiff) => f.write_str("TIFF 16 bit"),
            Self::Raw(RawFormat::Dng) => f.write_str("RAW · DNG (Lightroom, Camera Raw)"),
        }
    }
}

impl Output {
    fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::TiffPages => "tif",
            Self::Image(format) => format.extension(),
            Self::Raw(RawFormat::Tiff16) => "tif",
            Self::Raw(RawFormat::Dng) => "dng",
            Self::Deep(DeepFormat::Png) => "png",
            Self::Deep(DeepFormat::Tiff) => "tif",
        }
    }

    /// Lossy formats would ruin black and white.
    fn lossy(self) -> bool {
        matches!(
            self,
            Self::Image(ImageFormat::Jpeg { .. } | ImageFormat::Avif { .. })
        )
    }
}

/// The outputs that make sense for `kind` and `colour`, in order of
/// preference.
fn outputs(kind: Kind, colour: Colour, depth: u8) -> Vec<Output> {
    let mut all: Vec<Output> = match kind {
        Kind::Document => vec![
            Output::Pdf,
            Output::TiffPages,
            Output::Image(ImageFormat::Png),
            Output::Image(ImageFormat::Jpeg { quality: 90 }),
            Output::Image(ImageFormat::Tiff),
        ],
        Kind::Photo => vec![
            Output::Image(ImageFormat::Jpeg { quality: 92 }),
            Output::Image(ImageFormat::Png),
            Output::Image(ImageFormat::Tiff),
            Output::Image(ImageFormat::WebP),
            Output::Image(ImageFormat::Avif { quality: 80 }),
            // To print: the photo on a sheet (A4, 10 × 15…).
            Output::Pdf,
            Output::Raw(RawFormat::Tiff16),
            Output::Raw(RawFormat::Dng),
        ],
    };
    if depth == 16 && colour != Colour::Lineart {
        // After the 8-bit formats, before RAW.
        let at = all
            .iter()
            .position(|o| matches!(o, Output::Raw(_)))
            .unwrap_or(all.len());
        all.splice(
            at..at,
            [
                Output::Deep(DeepFormat::Png),
                Output::Deep(DeepFormat::Tiff),
            ],
        );
    }
    all.into_iter()
        .filter(|o| colour != Colour::Lineart || !o.lossy())
        .filter(|o| colour == Colour::Rgb || !matches!(o, Output::Raw(_)))
        .collect()
}

/// How the finished pages (or photos) go to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Writer {
    Pdf,
    /// One multi-page TIFF.
    Tiff,
    Zip,
    /// One image per page, next to the chosen name.
    Folder,
    /// The only page, as one image.
    One,
}

/// How `n` pages are written to `path` with `output` chosen: the name's
/// extension decides PDF and ZIP; several pages make a multi-page TIFF
/// only when that is the format chosen, else one image each ("TIFF" is
/// one image per page, as PNG and JPEG are).
fn writer(output: Output, path: &std::path::Path, n: usize) -> Writer {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => Writer::Pdf,
        "zip" => Writer::Zip,
        "tif" | "tiff" if n > 1 && output == Output::TiffPages => Writer::Tiff,
        _ if n > 1 => Writer::Folder,
        _ => Writer::One,
    }
}

/// The image format of `path`: the one chosen (with its quality) when the
/// name has its extension, else the extension's, else PNG.
fn image_format(path: &std::path::Path, output: Output) -> ImageFormat {
    match (ImageFormat::from_path(path), output) {
        (Some(by_name), Output::Image(chosen))
            if std::mem::discriminant(&by_name) == std::mem::discriminant(&chosen) =>
        {
            chosen
        }
        (Some(by_name), _) => by_name,
        (None, _) => ImageFormat::Png,
    }
}

/// The sheet of the PDF pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PdfSize {
    Fit,
    A4,
    A3,
    A5,
    Letter,
    Legal,
    Photo10x15,
    Photo13x18,
}

const PDF_SIZES: [PdfSize; 8] = [
    PdfSize::Fit,
    PdfSize::A4,
    PdfSize::A3,
    PdfSize::A5,
    PdfSize::Letter,
    PdfSize::Legal,
    PdfSize::Photo10x15,
    PdfSize::Photo13x18,
];

impl PdfSize {
    /// The name in the preferences file.
    fn key(self) -> &'static str {
        match self {
            Self::Fit => "scansione",
            Self::A4 => "a4",
            Self::A3 => "a3",
            Self::A5 => "a5",
            Self::Letter => "letter",
            Self::Legal => "legal",
            Self::Photo10x15 => "10x15",
            Self::Photo13x18 => "13x18",
        }
    }
}

impl std::fmt::Display for PdfSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Fit => "Come la scansione",
            Self::A4 => "A4 (21 × 29,7 cm)",
            Self::A3 => "A3 (29,7 × 42 cm)",
            Self::A5 => "A5 (14,8 × 21 cm)",
            Self::Letter => "Letter (8,5 × 11\")",
            Self::Legal => "Legal (8,5 × 14\")",
            Self::Photo10x15 => "Foto 10 × 15 cm",
            Self::Photo13x18 => "Foto 13 × 18 cm",
        })
    }
}

impl From<PdfSize> for PageSize {
    fn from(s: PdfSize) -> Self {
        match s {
            PdfSize::Fit => PageSize::Fit,
            PdfSize::A4 => PageSize::A4,
            PdfSize::A3 => PageSize::A3,
            PdfSize::A5 => PageSize::A5,
            PdfSize::Letter => PageSize::Letter,
            PdfSize::Legal => PageSize::Legal,
            PdfSize::Photo10x15 => PageSize::Photo10x15,
            PdfSize::Photo13x18 => PageSize::Photo13x18,
        }
    }
}

/// Where the scan goes on the sheet of a PDF page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PdfFit {
    Actual,
    Page,
    Fill,
}

const PDF_FITS: [PdfFit; 3] = [PdfFit::Actual, PdfFit::Page, PdfFit::Fill];

impl PdfFit {
    fn key(self) -> &'static str {
        match self {
            Self::Actual => "reale",
            Self::Page => "adatta",
            Self::Fill => "riempi",
        }
    }
}

impl std::fmt::Display for PdfFit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Actual => "Dimensione reale",
            Self::Page => "Adatta al foglio",
            Self::Fill => "Riempi il foglio (taglia i bordi)",
        })
    }
}

/// Which way the sheet of a PDF page is turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sheet {
    Auto,
    Portrait,
    Landscape,
}

const SHEETS: [Sheet; 3] = [Sheet::Auto, Sheet::Portrait, Sheet::Landscape];

impl Sheet {
    fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Portrait => "verticale",
            Self::Landscape => "orizzontale",
        }
    }
}

impl std::fmt::Display for Sheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "Automatico (come la scansione)",
            Self::Portrait => "Verticale",
            Self::Landscape => "Orizzontale",
        })
    }
}

impl From<Sheet> for hp3770_export::Orientation {
    fn from(s: Sheet) -> Self {
        match s {
            Sheet::Auto => Self::Auto,
            Sheet::Portrait => Self::Portrait,
            Sheet::Landscape => Self::Landscape,
        }
    }
}

/// How a page sits on its sheet of the PDF: which way the sheet is turned,
/// where the page goes on it, the margin. The document has one; a page may
/// have its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PageLayout {
    sheet: Sheet,
    fit: PdfFit,
    margin: u8,
}

/// The chip switches the lamp off by itself after this many minutes (the
/// HP driver's timer, set at every start-up): the longest choice.
const LAMP_SCANNER: u8 = 13;

/// When the lamp goes off: minutes without scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LampAfter(u8);

const LAMP_CHOICES: [LampAfter; 6] = [
    LampAfter(0),
    LampAfter(1),
    LampAfter(2),
    LampAfter(5),
    LampAfter(10),
    LampAfter(LAMP_SCANNER),
];

impl std::fmt::Display for LampAfter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0 => f.write_str("Appena finita la scansione"),
            1 => f.write_str("Dopo 1 minuto"),
            LAMP_SCANNER => write!(f, "Dopo {LAMP_SCANNER} minuti (come il driver HP)"),
            m => write!(f, "Dopo {m} minuti"),
        }
    }
}

/// Lowest quality of the PDF's pictures offered, percent.
const PDF_QUALITY_MIN: u8 = 30;

/// What the estimate of a PDF was made for: the pages as they were
/// (their revision), the way of working, the quality and the limit.
type EstimateKey = (u64, Kind, u8, Option<u32>);

/// The estimate of a PDF, made off the interface thread.
#[derive(Debug)]
struct Estimated {
    key: EstimateKey,
    /// The pages measured (kept while the pages do not change).
    samples: Arc<Vec<hp3770_export::Sample>>,
    result: Result<hp3770_export::Planned, String>,
}

/// A size in bytes as people read it: "850 KB", "4,7 MB".
fn human_size(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    if kb < 1000.0 {
        format!("{} KB", kb.round().max(1.0))
    } else {
        let mb = kb / 1024.0;
        let text = if mb < 10.0 {
            format!("{mb:.1}")
        } else {
            format!("{mb:.0}")
        };
        format!("{} MB", text.replace('.', ","))
    }
}

/// The encoding of a quality percentage.
fn e_base(percent: u8) -> hp3770_export::Encoding {
    hp3770_export::Encoding::from_percent(percent)
}

/// What the limit cost: "3 pagine su 10 alleggerite, fino a 200 dpi e
/// JPEG 65" (none when no page was).
fn lightened(
    encodings: &[hp3770_export::Encoding],
    base: hp3770_export::Encoding,
) -> Option<String> {
    let changed: Vec<&hp3770_export::Encoding> = encodings
        .iter()
        .filter(|e| e.quality != base.quality || e.max_dpi != base.max_dpi)
        .collect();
    if changed.is_empty() {
        return None;
    }
    let quality = changed
        .iter()
        .map(|e| e.quality)
        .min()
        .unwrap_or(base.quality);
    let dpi = changed.iter().filter_map(|e| e.max_dpi).min();
    let n = changed.len();
    let of = if n == encodings.len() {
        if n == 1 {
            "la pagina alleggerita".to_string()
        } else {
            format!("tutte le {n} pagine alleggerite")
        }
    } else if n == 1 {
        format!("1 pagina su {} alleggerita", encodings.len())
    } else {
        format!("{n} pagine su {} alleggerite", encodings.len())
    };
    Some(match dpi {
        Some(dpi) => format!("{of}, fino a {dpi} dpi e JPEG {quality}"),
        None => format!("{of}, fino a JPEG {quality}"),
    })
}

/// Megabytes typed ("5", "2,5", "0.8") in kilobytes; none for nothing.
fn parse_megabytes(text: &str) -> Option<Option<u32>> {
    let text = text.trim().trim_end_matches(['M', 'B', 'm', 'b']).trim();
    if text.is_empty() {
        return Some(None);
    }
    let mb: f64 = text.replace(',', ".").parse().ok()?;
    (mb > 0.0 && mb <= 10_000.0).then(|| Some((mb * 1024.0).round().max(1.0) as u32))
}

/// Margins offered for a scan fitted to the sheet, millimetres.
const PDF_MARGINS: [u8; 5] = [0, 5, 10, 15, 20];

/// A margin in the list of choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Margin(u8);

impl std::fmt::Display for Margin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0 => f.write_str("Nessuno (al vivo)"),
            mm => write!(f, "{mm} mm"),
        }
    }
}

/// Light or dark: as the system, or chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl std::fmt::Display for ThemeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::System => "Sistema",
            Self::Light => "Chiaro",
            Self::Dark => "Scuro",
        })
    }
}

/// The settings of one way of working, remembered separately.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Settings {
    dpi: u16,
    colour: Colour,
    threshold: u8,
    adjust: Adjustments,
    output: Output,
    pdf_size: PdfSize,
    /// Which way the sheets are turned (pages may turn theirs).
    pdf_sheet: Sheet,
    pdf_fit: PdfFit,
    /// Margin of a scan fitted to the sheet (or filling it), millimetres.
    pdf_margin: u8,
    /// Quality of the PDF's pictures, 30 to 100% (100: as scanned).
    pdf_quality: u8,
    /// Largest PDF, kilobytes (none: no limit): pages are lightened one
    /// by one until it fits.
    pdf_limit_kb: Option<u32>,
    auto_save: bool,
    /// Find the originals on the glass (photos, cards, receipts) and scan
    /// each one straightened, as its own image.
    auto_crop: bool,
    /// Bits per sample asked of the scanner: 8 or 16.
    depth: u8,
}

impl Settings {
    fn document() -> Self {
        Self {
            dpi: 300,
            colour: Colour::Rgb,
            threshold: convert::DEFAULT_THRESHOLD,
            adjust: Adjustments::default(),
            output: Output::Pdf,
            pdf_size: PdfSize::Fit,
            pdf_sheet: Sheet::Auto,
            pdf_fit: PdfFit::Actual,
            pdf_margin: 10,
            pdf_quality: 100,
            pdf_limit_kb: None,
            auto_save: false,
            auto_crop: false,
            depth: 8,
        }
    }

    /// The layout of the document's pages (those without their own).
    fn page_layout(&self) -> PageLayout {
        PageLayout {
            sheet: self.pdf_sheet,
            fit: self.pdf_fit,
            margin: self.pdf_margin,
        }
    }

    /// How a PDF page is laid out: on the document's sheet, with its own
    /// layout if it has one, the part `focus` kept when it overflows.
    fn layout(&self, own: Option<PageLayout>, focus: [f32; 2]) -> hp3770_export::Layout {
        let l = own.unwrap_or(self.page_layout());
        hp3770_export::Layout {
            size: self.pdf_size.into(),
            orientation: l.sheet.into(),
            placement: match l.fit {
                PdfFit::Actual => hp3770_export::Placement::Actual,
                PdfFit::Page => hp3770_export::Placement::FitPage,
                PdfFit::Fill => hp3770_export::Placement::Fill,
            },
            margin_mm: f64::from(l.margin),
            focus: focus.map(f64::from),
        }
    }

    fn photo() -> Self {
        Self {
            dpi: 600,
            output: Output::Image(ImageFormat::Jpeg { quality: 92 }),
            auto_crop: true,
            depth: 16,
            ..Self::document()
        }
    }

    fn mode(&self) -> Mode {
        match self.colour {
            Colour::Rgb => Mode::Colour,
            Colour::Grey => Mode::Grey,
            Colour::Lineart => Mode::Lineart {
                threshold: self.threshold,
            },
        }
    }
}

// ------------------------------------------------------------ preferences

impl Settings {
    fn to_stored(self) -> prefs::StoredMode {
        let a = self.adjust;
        prefs::StoredMode {
            dpi: self.dpi,
            colour: match self.colour {
                Colour::Rgb => "colore",
                Colour::Grey => "grigio",
                Colour::Lineart => "bn",
            }
            .into(),
            threshold: self.threshold,
            output: match self.output {
                Output::Pdf => "pdf",
                Output::TiffPages => "tiff-pagine",
                Output::Image(ImageFormat::Png) => "png",
                Output::Image(ImageFormat::Jpeg { .. }) => "jpeg",
                Output::Image(ImageFormat::Tiff) => "tiff",
                Output::Image(ImageFormat::WebP) => "webp",
                Output::Image(ImageFormat::Avif { .. }) => "avif",
                Output::Raw(RawFormat::Tiff16) => "raw-tiff16",
                Output::Raw(RawFormat::Dng) => "raw-dng",
                Output::Deep(DeepFormat::Png) => "png16",
                Output::Deep(DeepFormat::Tiff) => "tiff16",
                Output::Image(ImageFormat::Bmp) => "bmp",
            }
            .into(),
            pdf_size: self.pdf_size.key().into(),
            pdf_sheet: self.pdf_sheet.key().into(),
            pdf_fit: self.pdf_fit.key().into(),
            pdf_margin: Some(self.pdf_margin),
            pdf_quality: Some(self.pdf_quality),
            pdf_limit_kb: self.pdf_limit_kb,
            auto_save: self.auto_save,
            auto_crop: Some(self.auto_crop),
            depth: Some(self.depth),
            adjust: prefs::StoredAdjust {
                temperature: a.temperature,
                tint: a.tint,
                exposure: a.exposure,
                highlights: a.highlights,
                shadows: a.shadows,
                whites: a.whites,
                blacks: a.blacks,
                vibrance: a.vibrance,
                curve_points: a.curve.iter().map(|c| c.points().to_vec()).collect(),
                noise: a.noise,
                descreen: a.descreen,
                adaptive: a.adaptive,
                black: a.black,
                white: a.white,
                gamma: a.gamma,
                brightness: a.brightness,
                contrast: a.contrast,
                saturation: a.saturation,
                sharpen: a.sharpen,
            },
        }
    }

    /// The same settings with the look of a new session: the adjustments
    /// and the black-and-white threshold back to their defaults.
    fn fresh_look(self) -> Self {
        let defaults = Settings::document();
        Self {
            adjust: defaults.adjust,
            threshold: defaults.threshold,
            ..self
        }
    }

    /// Settings from the stored form; what does not make sense for `kind`
    /// falls back to the defaults.
    fn from_stored(kind: Kind, m: &prefs::StoredMode) -> Self {
        let base = match kind {
            Kind::Document => Settings::document(),
            Kind::Photo => Settings::photo(),
        };
        let colour = match m.colour.as_str() {
            "grigio" => Colour::Grey,
            "bn" => Colour::Lineart,
            _ => Colour::Rgb,
        };
        let output = match m.output.as_str() {
            "pdf" => Output::Pdf,
            "tiff-pagine" => Output::TiffPages,
            "png" => Output::Image(ImageFormat::Png),
            "jpeg" => Output::Image(ImageFormat::Jpeg { quality: 92 }),
            "tiff" => Output::Image(ImageFormat::Tiff),
            "webp" => Output::Image(ImageFormat::WebP),
            "avif" => Output::Image(ImageFormat::Avif { quality: 80 }),
            "raw-tiff16" => Output::Raw(RawFormat::Tiff16),
            "raw-dng" => Output::Raw(RawFormat::Dng),
            "png16" => Output::Deep(DeepFormat::Png),
            "tiff16" => Output::Deep(DeepFormat::Tiff),
            _ => base.output,
        };
        let depth = match m.depth {
            Some(d @ (8 | 16)) => d,
            _ => base.depth,
        };
        let allowed = outputs(kind, colour, depth);
        let output = allowed
            .iter()
            .copied()
            .find(|o| {
                std::mem::discriminant(o) == std::mem::discriminant(&output)
                    && o.extension() == output.extension()
            })
            .unwrap_or(allowed[0]);
        let a = &m.adjust;
        Self {
            // Checked against the scanner when it is open (fit_to_scanner).
            dpi: if (50..=19_200).contains(&m.dpi) {
                m.dpi
            } else {
                base.dpi
            },
            colour,
            threshold: m.threshold,
            adjust: Adjustments {
                temperature: a.temperature,
                tint: a.tint,
                exposure: a.exposure,
                highlights: a.highlights,
                shadows: a.shadows,
                whites: a.whites,
                blacks: a.blacks,
                vibrance: a.vibrance,
                curve: std::array::from_fn(|i| {
                    a.curve_points
                        .get(i)
                        .map(|p| scan_imaging::adjust::Curve::new(p))
                        .unwrap_or(scan_imaging::adjust::Curve::IDENTITY)
                }),
                noise: a.noise,
                descreen: a.descreen,
                adaptive: a.adaptive,
                black: a.black,
                white: a.white,
                gamma: a.gamma,
                brightness: a.brightness,
                contrast: a.contrast,
                saturation: a.saturation,
                sharpen: a.sharpen,
            },
            output,
            pdf_size: PDF_SIZES
                .into_iter()
                .find(|s| s.key() == m.pdf_size)
                .unwrap_or(base.pdf_size),
            pdf_sheet: SHEETS
                .into_iter()
                .find(|s| s.key() == m.pdf_sheet)
                .unwrap_or(base.pdf_sheet),
            pdf_fit: PDF_FITS
                .into_iter()
                .find(|f| f.key() == m.pdf_fit)
                .unwrap_or(base.pdf_fit),
            pdf_margin: m
                .pdf_margin
                .filter(|mm| PDF_MARGINS.contains(mm))
                .unwrap_or(base.pdf_margin),
            pdf_quality: m
                .pdf_quality
                .filter(|q| (PDF_QUALITY_MIN..=100).contains(q))
                .unwrap_or(base.pdf_quality),
            pdf_limit_kb: m.pdf_limit_kb.filter(|&kb| kb > 0),
            auto_save: m.auto_save,
            auto_crop: m.auto_crop.unwrap_or(base.auto_crop),
            depth,
        }
    }
}

/// The factory presets of a way of working: read-only, ahead of the
/// three of the user. Chosen after what Epson Scan, VueScan, NAPS2 and
/// ScanSnap offer: text for OCR in black and white with an adaptive
/// threshold, clean colour documents, receipts, printed matter with
/// descreening, lossless archives; photos for everyday use, for archiving,
/// faded prints, black and white, photos printed in magazines.
fn factory_presets(kind: Kind) -> Vec<(&'static str, Settings)> {
    let d = Settings::document();
    let f = Settings::photo();
    let a = Adjustments::default();
    match kind {
        Kind::Document => vec![
            (
                "Testo per OCR (bianco e nero)",
                Settings {
                    colour: Colour::Lineart,
                    adjust: Adjustments {
                        adaptive: true,
                        sharpen: 1.0,
                        ..a
                    },
                    ..d
                },
            ),
            (
                "Documento a colori",
                Settings {
                    adjust: Adjustments {
                        whites: 25.0,
                        blacks: -10.0,
                        ..a
                    },
                    ..d
                },
            ),
            (
                "Ricevuta / scontrino",
                Settings {
                    colour: Colour::Grey,
                    adjust: Adjustments {
                        contrast: 30.0,
                        whites: 30.0,
                        blacks: -25.0,
                        ..a
                    },
                    ..d
                },
            ),
            (
                "Rivista / giornale",
                Settings {
                    adjust: Adjustments {
                        descreen: 133.0,
                        sharpen: 1.0,
                        ..a
                    },
                    ..d
                },
            ),
            (
                "Archivio (600 dpi, senza perdita)",
                Settings {
                    dpi: 600,
                    adjust: Adjustments { sharpen: 0.0, ..a },
                    output: Output::TiffPages,
                    ..d
                },
            ),
        ],
        Kind::Photo => vec![
            (
                "Foto standard",
                Settings {
                    adjust: Adjustments {
                        vibrance: 10.0,
                        sharpen: 1.0,
                        ..a
                    },
                    ..f
                },
            ),
            (
                "Archivio foto (1200 dpi, TIFF)",
                Settings {
                    dpi: 1200,
                    adjust: Adjustments { sharpen: 0.0, ..a },
                    output: Output::Image(ImageFormat::Tiff),
                    ..f
                },
            ),
            (
                "Foto sbiadita (recupero colori)",
                Settings {
                    adjust: Adjustments {
                        contrast: 15.0,
                        vibrance: 35.0,
                        saturation: 10.0,
                        blacks: -15.0,
                        ..a
                    },
                    ..f
                },
            ),
            (
                "Foto in bianco e nero",
                Settings {
                    colour: Colour::Grey,
                    adjust: Adjustments {
                        contrast: 10.0,
                        ..a
                    },
                    ..f
                },
            ),
            (
                "Foto stampata su rivista",
                Settings {
                    adjust: Adjustments {
                        descreen: 150.0,
                        sharpen: 1.0,
                        ..a
                    },
                    ..f
                },
            ),
        ],
    }
}

/// What a front-panel button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonAction {
    AddPage,
    RedoLast,
    SaveDocument,
    ScanPhoto,
    Photocopy,
    OpenFolder,
    Nothing,
}

impl ButtonAction {
    const DOCUMENT: [ButtonAction; 5] = [
        Self::AddPage,
        Self::RedoLast,
        Self::SaveDocument,
        Self::OpenFolder,
        Self::Nothing,
    ];
    const PHOTO: [ButtonAction; 4] = [
        Self::ScanPhoto,
        Self::Photocopy,
        Self::OpenFolder,
        Self::Nothing,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::AddPage => "aggiungi-pagina",
            Self::RedoLast => "rifai-ultima",
            Self::SaveDocument => "salva-documento",
            Self::ScanPhoto => "scansiona-foto",
            Self::Photocopy => "fotocopia",
            Self::OpenFolder => "apri-cartella",
            Self::Nothing => "niente",
        }
    }

    /// The actions a button can have in `kind`.
    fn allowed(kind: Kind) -> &'static [ButtonAction] {
        match kind {
            Kind::Document => &Self::DOCUMENT,
            Kind::Photo => &Self::PHOTO,
        }
    }

    /// The action saved as `key`, if `kind` has it.
    fn from_key(kind: Kind, key: &str) -> Option<ButtonAction> {
        Self::allowed(kind).iter().copied().find(|a| a.key() == key)
    }

    /// What a button meant for `action` does in `kind` until the user
    /// chooses: Scan adds a page (scans a photo), Copy redoes the last page
    /// (photocopies), Share saves the document (opens the folder).
    fn default_for(kind: Kind, action: SuggestedAction) -> ButtonAction {
        match (kind, action) {
            (Kind::Document, SuggestedAction::Scan) => Self::AddPage,
            (Kind::Document, SuggestedAction::Copy) => Self::RedoLast,
            (Kind::Document, SuggestedAction::Share) => Self::SaveDocument,
            (Kind::Photo, SuggestedAction::Scan) => Self::ScanPhoto,
            (Kind::Photo, SuggestedAction::Copy) => Self::Photocopy,
            (Kind::Photo, SuggestedAction::Share) => Self::OpenFolder,
            (_, SuggestedAction::None) => Self::Nothing,
        }
    }
}

impl std::fmt::Display for ButtonAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AddPage => "Aggiungi una pagina",
            Self::RedoLast => "Rifai l'ultima pagina",
            Self::SaveDocument => "Salva il documento e apri la cartella",
            Self::ScanPhoto => "Scansiona la foto",
            Self::Photocopy => "Fotocopia (scansiona e stampa)",
            Self::OpenFolder => "Apri la cartella",
            Self::Nothing => "Niente",
        })
    }
}

/// Bits per sample of the scan, for the choice in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Depth(u8);

impl std::fmt::Display for Depth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bit per canale", self.0)
    }
}

/// Which way of working the application opens with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartMode {
    Last,
    Document,
    Photo,
}

impl std::fmt::Display for StartMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Last => "L'ultimo usato",
            Self::Document => "Documento",
            Self::Photo => "Foto",
        })
    }
}

// ------------------------------------------------------------------ pages

/// How the interface names item `i` of `count`: "pagina 2", "foto 3", or
/// just "foto" when it is the only one.
fn item_name(kind: Kind, i: usize, count: usize) -> String {
    match kind {
        Kind::Document => format!("pagina {}", i + 1),
        Kind::Photo if count > 1 => format!("foto {}", i + 1),
        Kind::Photo => "foto".into(),
    }
}

/// The scanner's linear colour samples of a scan, and a smaller copy for
/// live adjustments.
#[derive(Debug, Clone)]
struct Raw {
    full: Arc<Image>,
    proxy: Arc<Image>,
    /// The tone curve of the scanner that made the samples.
    tone: Tone,
}

impl Raw {
    fn new(image: Image, tone: Tone) -> Self {
        let proxy = shrink(&image, PROXY);
        Self {
            full: Arc::new(image),
            proxy: Arc::new(proxy),
            tone,
        }
    }

    /// The linear sample at a point of the proxy, averaged over 5 × 5
    /// pixels.
    fn sample(&self, p: Point) -> [u8; 3] {
        let img = &self.proxy;
        let (cx, cy) = (p.x as i64, p.y as i64);
        let mut sum = [0u32; 3];
        let mut n = 0;
        for y in cy - 2..=cy + 2 {
            for x in cx - 2..=cx + 2 {
                if x < 0 || y < 0 || x >= img.width as i64 || y >= img.height as i64 {
                    continue;
                }
                let i = (y as usize * img.width + x as usize) * 3;
                for (c, s) in sum.iter_mut().enumerate() {
                    *s += u32::from(img.data[i + c]);
                }
                n += 1;
            }
        }
        sum.map(|s| (s / n.max(1)) as u8)
    }
}

/// An RGB image shrunk (block averages) to at most `max` pixels a side.
fn shrink(image: &Image, max: usize) -> Image {
    let factor = image.width.max(image.height).div_ceil(max).max(1);
    if factor == 1 {
        return image.clone();
    }
    let (w, h) = (image.width / factor, image.height / factor);
    let mut data = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 3];
            for yy in 0..factor {
                let row = (y * factor + yy) * image.width;
                for xx in 0..factor {
                    let i = (row + x * factor + xx) * 3;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += u32::from(image.data[i + c]);
                    }
                }
            }
            let n = (factor * factor) as u32;
            data.extend(sum.map(|s| (s / n) as u8));
        }
    }
    Image {
        width: w,
        height: h,
        dpi: (image.dpi as usize / factor) as u16,
        format: Format::Rgb,
        data,
    }
}

/// The adjusted image of a raw scan (on the proxy, or at full size),
/// through the tone curve of the scanner that made it.
fn render(raw: &Image, settings: &Settings, adjust: &Adjustments, tone: &Tone) -> Image {
    adjust::render(
        &raw.data,
        raw.width,
        raw.height,
        raw.dpi,
        settings.mode(),
        adjust,
        false,
        tone,
    )
}

/// What comes in from outside when straightening: the paper white of the
/// linear samples (the white levels of the scanner's tone curve).
fn fill8(tone: &Tone) -> [u8; 3] {
    tone.white.map(|w| w as u8)
}

fn fill16(tone: &Tone) -> [u16; 3] {
    tone.white.map(|w| (w * 256.0) as u16)
}

/// The linear samples of a scan with the geometry edits.
fn edited(raw: &Image, t: &Transform, tone: &Tone) -> Image {
    let (data, width, height) = t.apply(&raw.data, raw.width, raw.height, 3, &fill8(tone));
    Image {
        width,
        height,
        dpi: t.output_dpi(raw.width, raw.height, raw.dpi),
        format: Format::Rgb,
        data,
    }
}

/// What the viewer shows of a page: the adjusted image of the small copy
/// with `t`, its picture (`size`: the full size it stands for) and a
/// thumbnail if asked.
fn draw_page(
    proxy: &Image,
    tone: &Tone,
    settings: &Settings,
    adjust: &Adjustments,
    t: &Transform,
    size: Size,
    thumb: bool,
) -> (Image, Picture, Option<Picture>) {
    let raw = if t.is_identity() {
        Cow::Borrowed(proxy)
    } else {
        Cow::Owned(edited(proxy, t, tone))
    };
    let shown = render(&raw, settings, adjust, tone);
    let rgb = shown.to_rgb();
    let mut picture = Picture::from_rgb(shown.width, shown.height, &rgb);
    picture.size = size;
    let thumb = thumb.then(|| Picture::from_rgb_max(shown.width, shown.height, &rgb, 240));
    (shown, picture, thumb)
}

/// How a scan looks: the adjustments and the black-and-white threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Look {
    adjust: Adjustments,
    threshold: u8,
}

impl Look {
    fn of(s: &Settings) -> Self {
        Self {
            adjust: s.adjust,
            threshold: s.threshold,
        }
    }

    fn put(self, s: &mut Settings) {
        s.adjust = self.adjust;
        s.threshold = self.threshold;
    }
}

/// A page of the document (or the photo): its raw scan, the settings it
/// was scanned with, and what is shown. It looks like the shared settings
/// of its way of working, unless it has a look of its own. A copy shares
/// the scan (the samples are behind `Arc`s).
#[derive(Clone)]
struct Page {
    raw: Raw,
    settings: Settings,
    /// "Solo questa pagina": adjustments of its own.
    own: Option<Look>,
    shown: Image,
    picture: Picture,
    thumb: Picture,
    /// Where it was found on the glass (pixels of the preview), when it
    /// was cut out by the automatic crop: "Rifai" scans just that again.
    origin: Option<Found>,
    /// The chip's 16-bit samples (same size as `raw.full`), when the scan
    /// was made for a RAW file.
    raw16: Option<Arc<Vec<u16>>>,
    /// Turns, straightening, crop and size, worked out from the samples
    /// each time (the samples stay as scanned).
    transform: Transform,
    /// The colour mode it was scanned in.
    scanned_colour: Colour,
    /// Its own layout in a PDF (none: the document's).
    layout: Option<PageLayout>,
    /// In a PDF, when it fills the sheet and overflows it: the part kept,
    /// across and down (0 to 1, 0.5 the middle).
    focus: [f32; 2],
}

impl Page {
    fn new(raw: Raw, settings: Settings) -> Self {
        let size = Size::new(raw.full.width as f32, raw.full.height as f32);
        let (shown, picture, thumb) = draw_page(
            &raw.proxy,
            &raw.tone,
            &settings,
            &settings.adjust,
            &Transform::default(),
            size,
            true,
        );
        let thumb = thumb.unwrap_or_else(|| picture.clone());
        Self {
            picture,
            thumb,
            raw,
            settings,
            own: None,
            shown,
            origin: None,
            raw16: None,
            transform: Transform::default(),
            scanned_colour: settings.colour,
            layout: None,
            focus: [0.5, 0.5],
        }
    }

    /// Its size in inches as finished (crop, turns and size applied).
    fn inches(&self) -> (f64, f64) {
        let full = &self.raw.full;
        let (w, h) = self.transform.output_size(full.width, full.height);
        let dpi = f64::from(
            self.transform
                .output_dpi(full.width, full.height, full.dpi)
                .max(1),
        );
        (w as f64 / dpi, h as f64 / dpi)
    }

    /// What the editor changes.
    fn edit(&self) -> Edit {
        Edit {
            look: self.own,
            colour: self.settings.colour,
            transform: self.transform,
        }
    }

    fn set_edit(&mut self, e: Edit) {
        self.own = e.look;
        self.settings.colour = e.colour;
        self.transform = e.transform;
    }

    /// The settings of an edit of this page.
    fn settings_with(&self, e: &Edit, shared: &Settings) -> Settings {
        let mut s = self.settings;
        s.colour = e.colour;
        e.look.unwrap_or(Look::of(shared)).put(&mut s);
        s
    }

    /// The transform for the small copy and the size the viewer shows.
    /// `frame`: the whole turned frame, for choosing the crop. When the
    /// result is smaller than the copy it is drawn at its real size, so
    /// the softness of a reduction shows.
    fn preview_geometry(&self, t: &Transform, frame: bool) -> (Transform, Size) {
        let full = &self.raw.full;
        if frame {
            let (w, h) = t.oriented_size(full.width, full.height);
            return (t.frame_only(), Size::new(w as f32, h as f32));
        }
        let (ow, oh) = t.output_size(full.width, full.height);
        let mut p = Transform { dpi: None, ..*t };
        if let Some([sx, sy]) = t.scale {
            let f = self.raw.proxy.width as f32 / full.width.max(1) as f32;
            let s = [(sx / f).min(1.0), (sy / f).min(1.0)];
            p.scale = (s != [1.0, 1.0]).then_some(s);
        }
        (p, Size::new(ow as f32, oh as f32))
    }

    /// The scanner's samples at full size with the geometry edits.
    fn full_samples(&self) -> (Arc<Image>, Option<Arc<Vec<u16>>>) {
        let t = &self.transform;
        if t.is_identity() {
            return (self.raw.full.clone(), self.raw16.clone());
        }
        let full = &self.raw.full;
        let deep = self.raw16.as_ref().map(|samples| {
            let fill = fill16(&self.raw.tone);
            Arc::new(t.apply(samples, full.width, full.height, 3, &fill).0)
        });
        (Arc::new(edited(full, t, &self.raw.tone)), deep)
    }

    /// A point of the viewer (`frame`: the whole turned frame) on the
    /// small copy of the samples.
    fn proxy_point(&self, p: Point, frame: bool) -> Point {
        let full = &self.raw.full;
        let t = if frame {
            self.transform.frame_only()
        } else {
            self.transform
        };
        let (x, y) = t.source_point(full.width, full.height, p.x, p.y);
        let k = self.raw.proxy.width as f32 / full.width.max(1) as f32;
        Point::new(x * k, y * k)
    }

    /// The settings it is drawn with: its own look, or the shared one.
    fn effective(&self, shared: &Settings) -> Settings {
        let mut s = self.settings;
        self.own.unwrap_or(Look::of(shared)).put(&mut s);
        s
    }

    fn redraw(&mut self, before: bool, shared: &Settings) {
        let settings = self.effective(shared);
        let adjust = if before {
            Adjustments::default()
        } else {
            settings.adjust
        };
        let (t, size) = self.preview_geometry(&self.transform, false);
        let (shown, picture, thumb) = draw_page(
            &self.raw.proxy,
            &self.raw.tone,
            &settings,
            &adjust,
            &t,
            size,
            true,
        );
        self.shown = shown;
        self.picture = picture.replacing(&self.picture);
        if let Some(thumb) = thumb {
            self.thumb = thumb;
        }
    }

    /// The finished image at full resolution.
    /// The page at full size with its adjustments: from the 16-bit
    /// samples when the scan has them (smooth shadows after the curves).
    fn finished(&self, shared: &Settings) -> Image {
        let settings = self.effective(shared);
        let (full, deep) = self.full_samples();
        match &deep {
            Some(samples) => adjust::render_samples(
                adjust::Samples::Sixteen(samples),
                full.width,
                full.height,
                full.dpi,
                settings.mode(),
                &settings.adjust,
                false,
                &self.raw.tone,
            ),
            None => render(&full, &settings, &settings.adjust, &self.raw.tone),
        }
    }

    /// A small copy of the finished page (its geometry and look drawn on
    /// the small copy of the scan) and the real page's size: what the
    /// estimate of a PDF measures.
    fn pdf_sample(&self, shared: &Settings) -> hp3770_export::Sample {
        let settings = self.effective(shared);
        let full = &self.raw.full;
        // The crop and turns, at the small copy's size.
        let t = Transform {
            scale: None,
            dpi: None,
            ..self.transform
        };
        let small = if t.is_identity() {
            render(&self.raw.proxy, &settings, &settings.adjust, &self.raw.tone)
        } else {
            let tone = &self.raw.tone;
            render(
                &edited(&self.raw.proxy, &t, tone),
                &settings,
                &settings.adjust,
                tone,
            )
        };
        let (w, h) = self.transform.output_size(full.width, full.height);
        let dpi = self.transform.output_dpi(full.width, full.height, full.dpi);
        hp3770_export::Sample::of(&small, w, h, dpi)
    }

    /// A quarter turn, part of the page's geometry edits.
    fn rotate(&mut self, clockwise: bool, shared: &Settings) {
        self.transform.rotate(clockwise);
        self.redraw(false, shared);
    }
}

// ---------------------------------------------------------------- messages

/// Groups of the adjustments panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Group {
    Tone,
    Curve,
    Colour,
    Detail,
    Lineart,
}

/// The channel the curve editor shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurveChannel {
    All,
    Red,
    Green,
    Blue,
}

impl CurveChannel {
    fn index(self) -> usize {
        match self {
            Self::All => 0,
            Self::Red => 1,
            Self::Green => 2,
            Self::Blue => 3,
        }
    }
}

/// Screens of printed matter, as offered.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Screen(f32);

impl Eq for Screen {}

impl std::fmt::Display for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 as u32 {
            0 => f.write_str("Spento"),
            85 => f.write_str("Giornale (85 lpi)"),
            133 => f.write_str("Rivista (133 lpi)"),
            150 => f.write_str("Rivista patinata (150 lpi)"),
            175 => f.write_str("Stampa d'arte (175 lpi)"),
            n => write!(f, "{n} lpi"),
        }
    }
}

const SCREENS: [Screen; 5] = [
    Screen(0.0),
    Screen(85.0),
    Screen(133.0),
    Screen(150.0),
    Screen(175.0),
];

/// An adjustment with a slider and a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Noise,
    Temperature,
    Tint,
    Exposure,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Vibrance,
    Black,
    White,
    Gamma,
    Brightness,
    Contrast,
    Saturation,
    Sharpen,
    Threshold,
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Self::Temperature => "Temperatura (freddo ↔ caldo)",
            Self::Noise => "Riduzione rumore",
            Self::Tint => "Tinta (verde ↔ magenta)",
            Self::Exposure => "Esposizione (stop)",
            Self::Highlights => "Luci",
            Self::Shadows => "Ombre",
            Self::Whites => "Bianchi",
            Self::Blacks => "Neri",
            Self::Vibrance => "Vividezza",
            Self::Black => "Punto del nero",
            Self::White => "Punto del bianco",
            Self::Gamma => "Gamma (mezzitoni)",
            Self::Brightness => "Luminosità",
            Self::Contrast => "Contrasto",
            Self::Saturation => "Saturazione",
            Self::Sharpen => "Nitidezza (1,5 = come HP)",
            Self::Threshold => "Soglia (più alta = più nero)",
        }
    }

    fn range(self) -> std::ops::RangeInclusive<f32> {
        match self {
            Self::Black => 0.0..=254.0,
            Self::White => 1.0..=255.0,
            Self::Gamma => 0.2..=5.0,
            Self::Sharpen => 0.0..=4.0,
            Self::Threshold => 0.0..=255.0,
            Self::Exposure => -3.0..=3.0,
            Self::Noise => 0.0..=100.0,
            _ => -100.0..=100.0,
        }
    }

    fn step(self) -> f32 {
        match self {
            Self::Gamma | Self::Exposure => 0.05,
            Self::Sharpen => 0.1,
            _ => 1.0,
        }
    }

    fn get(self, s: &Settings) -> f32 {
        let a = &s.adjust;
        match self {
            Self::Temperature => a.temperature,
            Self::Noise => a.noise,
            Self::Tint => a.tint,
            Self::Exposure => a.exposure,
            Self::Highlights => a.highlights,
            Self::Shadows => a.shadows,
            Self::Whites => a.whites,
            Self::Blacks => a.blacks,
            Self::Vibrance => a.vibrance,
            Self::Black => a.black,
            Self::White => a.white,
            Self::Gamma => a.gamma,
            Self::Brightness => a.brightness,
            Self::Contrast => a.contrast,
            Self::Saturation => a.saturation,
            Self::Sharpen => a.sharpen,
            Self::Threshold => f32::from(s.threshold),
        }
    }

    /// The value of a fresh setup (double click on the slider).
    fn default(self) -> f32 {
        let s = Settings::document();
        self.get(&s)
    }

    fn set(self, s: &mut Settings, v: f32) {
        let v = v.clamp(*self.range().start(), *self.range().end());
        let a = &mut s.adjust;
        match self {
            Self::Temperature => a.temperature = v,
            Self::Noise => a.noise = v,
            Self::Tint => a.tint = v,
            Self::Exposure => a.exposure = v,
            Self::Highlights => a.highlights = v,
            Self::Shadows => a.shadows = v,
            Self::Whites => a.whites = v,
            Self::Blacks => a.blacks = v,
            Self::Vibrance => a.vibrance = v,
            Self::Black => a.black = v.min(a.white - 1.0),
            Self::White => a.white = v.max(a.black + 1.0),
            Self::Gamma => a.gamma = v,
            Self::Brightness => a.brightness = v,
            Self::Contrast => a.contrast = v,
            Self::Saturation => a.saturation = v,
            Self::Sharpen => a.sharpen = v,
            Self::Threshold => s.threshold = v.round() as u8,
        }
    }

    fn format(self, v: f32) -> String {
        if self.step() < 1.0 {
            format!("{v:.2}").replace('.', ",")
        } else {
            format!("{v:.0}")
        }
    }
}

/// What a background render redraws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Preview,
    /// The page shown in the result tab (a rescan waiting, or the current
    /// page).
    Page,
    /// Another page of the document whose shared look changed (its
    /// thumbnail and picture).
    Index(usize),
}

/// An adjusted image computed off the interface thread.
#[derive(Debug)]
struct Rendered {
    target: Target,
    /// For [`Target::Page`]: which page was shown (see `App::shown_key`).
    page: Option<usize>,
    /// `App::epoch` when it started.
    epoch: u64,
    shown: Image,
    picture: Picture,
    thumb: Option<Picture>,
}

/// The pages of the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Scan,
    Adjust,
    Output,
    Prefs,
}

/// Which image the viewer shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Preview,
    Result,
}

#[derive(Debug, Clone)]
enum Message {
    Worker(Event),
    Rendered(Arc<Rendered>),
    SystemTheme(iced::theme::Mode),
    Theme(ThemeChoice),
    Kind(Kind),
    Tab(Tab),
    Pane(Pane),
    Undo,
    Redo,
    CurveChannel(CurveChannel),
    /// New points of the curve of the channel shown.
    CurveSet(Vec<[f32; 2]>),
    Descreen(Screen),
    Adaptive(bool),
    /// "Solo questa pagina" on or off for the page shown.
    OwnLook(bool),
    /// The look of the page shown to every page, the preview and the next
    /// scans.
    LookApplyToAll,
    PresetName(String),
    PresetSave,
    PresetApply(String),
    PresetDelete(String),
    Key(iced::keyboard::Event),
    StartMode(StartMode),
    /// A button's action in the current way of working: which button and
    /// what it does.
    ButtonAction(usize, ButtonAction),
    /// Use this scanner (when there are several).
    ChooseScanner(DeviceInfo),
    /// Make Windows leave the scanner buttons alone (or not).
    WiaQuiet(bool),
    WiaDone(Result<(), String>),
    /// Install the udev rule (Linux, scanner there but not openable).
    GrantAccess,
    GrantDone(Result<(), String>),
    ChooseFolder,
    Folder(Option<PathBuf>),
    DefaultFolder,
    DocumentName(String),
    PhotoName(String),
    FactoryReset,
    View(View),
    Fit,
    ActualSize,
    Select(Option<Selection>),
    Dpi(u16),
    Depth(Depth),
    Colour(Colour),
    Output(Output),
    PdfSize(PdfSize),
    PdfFit(PdfFit),
    PdfMargin(Margin),
    PdfSheet(Sheet),
    PdfQuality(u8),
    /// The largest PDF being typed, in megabytes.
    PdfLimitText(String),
    /// The estimate of the PDF is ready.
    Estimated(Arc<Estimated>),
    /// The page shown gets a layout of its own in the PDF (from the
    /// document's), or goes back to the document's.
    PdfOwnLayout(bool),
    /// The layout of the page shown becomes the document's, for every page.
    PdfApplyToAll,
    /// The part kept of the page shown, when it overflows its sheet.
    PageFocus([f32; 2]),
    AutoSave(bool),
    AutoCrop(bool),
    /// Find the originals on the preview again.
    Detect,
    /// Leave out one of the originals found.
    DropFound(usize),
    Toggle(Group),
    Adjust(Field, f32),
    /// Text typed in the number of a field.
    Draft(Field, String),
    /// Enter in the number of a field.
    Commit(Field),
    AutoLevels,
    AutoThreshold,
    Eyedropper,
    Picked(Point),
    ResetGroup(Group),
    Before(bool),
    RefreshStatus,
    Preview,
    Scan,
    Cancel,
    Save,
    SaveTo(Option<PathBuf>),
    PageShow(usize),
    PageRotate(usize, bool),
    PageMove(usize, bool),
    PageDelete(usize),
    /// A copy of the page right after it: the same scan, cut, turned and
    /// adjusted on its own (several photos from one scan).
    PageDuplicate(usize),
    /// The mouse went down on a page of the strip: shown, and maybe
    /// dragged to another place.
    PagePress(usize),
    /// A page dragged is over this one.
    PageHover(usize),
    /// The mouse went up: a page dragged goes where it was let go.
    PageDrop,
    PageRedo(usize),
    AcceptRedo,
    RejectRedo,
    NewDocument,
    /// The editor of a scan already made.
    Edit(EditMsg),
    /// The answer to the question asked before throwing scans away: go
    /// on (true) or leave everything as it is (false).
    Confirm(bool),
    /// "Salva…" in that question: save, then go on if the save went well.
    ConfirmSave,
    /// The window's close button.
    CloseRequested(iced::window::Id),
    /// The lamp is off (or the scanner did not answer in time): close.
    Close(iced::window::Id),
    LampAfter(LampAfter),
}

/// A question asked before an action that throws away scans not saved:
/// the action is done again, confirmed, when the answer is yes.
#[derive(Debug, Clone)]
struct Confirm {
    title: String,
    body: String,
    /// The label of the button that goes on.
    yes: String,
    then: Box<Message>,
}

/// The value of `offered` nearest to `wanted` (the larger on a tie);
/// `wanted` itself when nothing is offered.
fn nearest(offered: &[u16], wanted: u16) -> u16 {
    offered
        .iter()
        .copied()
        .min_by_key(|&d| (d.abs_diff(wanted), std::cmp::Reverse(d)))
        .unwrap_or(wanted)
}

/// An image brought to `dpi` (the preview of a scanner whose resolutions
/// do not include [`PREVIEW_DPI`]): each pixel the one under its centre.
fn at_dpi(image: Image, dpi: u16) -> Image {
    if image.dpi == dpi || image.dpi == 0 || image.format != Format::Rgb {
        return image;
    }
    let k = f64::from(image.dpi) / f64::from(dpi);
    let width = ((image.width as f64) / k).round().max(1.0) as usize;
    let height = ((image.height as f64) / k).round().max(1.0) as usize;
    let mut data = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        let sy = (((y as f64 + 0.5) * k) as usize).min(image.height - 1);
        for x in 0..width {
            let sx = (((x as f64 + 0.5) * k) as usize).min(image.width - 1);
            let i = (sy * image.width + sx) * 3;
            data.extend_from_slice(&image.data[i..i + 3]);
        }
    }
    Image {
        width,
        height,
        dpi,
        format: Format::Rgb,
        data,
    }
}

/// The area of a rectangle given in millimetres, in pixels at `dpi`.
fn area_mm(dpi: u16, left: f64, top: f64, width: f64, height: f64) -> Area {
    let px = |mm: f64| (mm * f64::from(dpi) / 25.4).round().max(0.0) as u32;
    Area {
        x: px(left),
        y: px(top),
        width: px(width),
        height: px(height),
    }
}

/// `area` (pixels at `dpi`) cut to the glass: the margin around an
/// original near the edge, or the rounding of position and size apart,
/// would ask the scanner for a pixel beyond it, and it refuses the scan.
fn within_glass(area: Area, dpi: u16, caps: &Capabilities) -> Area {
    let (max_w, max_h) = caps.glass_pixels(dpi);
    let x = area.x.min(max_w.saturating_sub(1));
    let y = area.y.min(max_h.saturating_sub(1));
    Area {
        x,
        y,
        width: area.width.min(max_w - x),
        height: area.height.min(max_h - y),
    }
}

/// One pass of the scanner: an area of the glass (pixels of the
/// preview) and the originals in it, each cut out of the scan.
#[derive(Debug, Clone)]
struct Job {
    bounds: [f32; 4],
    crops: Vec<Found>,
    /// The area asked of the scanner (pixels at the resolution).
    area: Option<Area>,
}

/// Largest single pass for several originals; beyond it each is scanned
/// on its own.
const ONE_PASS: f64 = 700e6;

/// Cut inside the edge of a found original (millimetres): its edge is
/// blurred and a sliver of the lid would show.
const INSET_MM: f32 = 0.4;

// --------------------------------------------------------------------- app

struct App {
    worker: Option<Handle>,
    status: Option<Result<DeviceStatus, String>>,
    /// What the scanner in use can do (before one answers: the first model
    /// the backends know). Every part of the app tied to a function of the
    /// scanner asks here, and only here (see `offers`).
    caps: Capabilities,
    /// The scanner in use, once opened.
    device: Option<DeviceInfo>,
    /// Every scanner found, to choose from when there is more than one.
    devices: Vec<DeviceInfo>,
    /// The scanner is plugged in but this user may not open it (Linux,
    /// udev rule missing).
    access_denied: bool,
    system_dark: bool,
    theme: ThemeChoice,
    kind: Kind,
    document: Settings,
    photo: Settings,
    tab: Tab,
    pane: Pane,
    start_mode: StartMode,
    /// Where scans are saved (None: Pictures\Scansioni).
    custom_folder: Option<PathBuf>,
    document_name: String,
    photo_name: String,
    /// What Scan, Copy and Instant Share do, per way of working.
    /// What the buttons do, by scanner model and way of working (button
    /// id → action key); a button not there does its default.
    button_actions: std::collections::BTreeMap<String, prefs::StoredButtons>,
    /// Print the next photo when it arrives (photocopy).
    print_next: bool,
    /// Whether Windows leaves the buttons alone (None: nothing to set).
    wia_quiet: Option<bool>,
    /// What was last written to the preferences file.
    saved: prefs::Prefs,
    /// The raw preview of the glass, its adjusted image and picture.
    preview: Option<(Raw, Image, Picture)>,
    preview_view: View,
    result_view: View,
    selection: Option<Selection>,
    /// Originals found on the preview (pixels of the preview).
    found: Vec<Found>,
    /// The pass running and those still to do.
    job: Option<Job>,
    jobs: std::collections::VecDeque<Job>,
    /// Originals of this scan acquired so far, and how many in all.
    batch: (usize, usize),
    open: Vec<Group>,
    eyedropper: bool,
    before: bool,
    /// The scan running, and where it is.
    busy: Option<(Purpose, Option<Progress>)>,
    /// Pages of the document; in Foto mode, the last photo only.
    pages: Vec<Page>,
    current: Option<usize>,
    redo: Option<usize>,
    redo_auto: bool,
    redone: Option<(usize, Page)>,
    note: String,
    curve_channel: CurveChannel,
    /// When the curve was last dragged (one step of Ctrl+Z per drag).
    curve_drag: Option<std::time::Instant>,
    /// Named setups (at most three), and the name being typed.
    presets: Vec<prefs::Preset>,
    preset_name: String,
    /// Earlier settings of what the panel edits, for Ctrl+Z.
    undo: Vec<(Target, Settings)>,
    redo_stack: Vec<(Target, Settings)>,
    /// The field last changed and when: a slider dragged is one step.
    last_edit: Option<(Field, std::time::Instant)>,
    /// A number being typed, not applied yet.
    draft: Option<(Field, String)>,
    /// A render is running off the interface thread.
    rendering: bool,
    /// What changed and is waiting to be redrawn.
    dirty_preview: bool,
    dirty_page: bool,
    /// Pages following the shared look whose picture is out of date.
    stale: Vec<usize>,
    /// Changes each time pages move, go or are replaced: a render
    /// started before is of a page that may not be at its place any more.
    epoch: u64,
    /// The editor of a scan, when open.
    editor: Option<Editor>,
    /// The part of the editor last used.
    edit_section: Section,
    /// The pages changed since they were last saved (or none was).
    unsaved: bool,
    /// The question shown before throwing scans away, if any.
    confirm: Option<Confirm>,
    /// What to do once "Salva…" in that question has saved.
    after_save: Option<Message>,
    /// Set while a confirmed action runs: it does not ask again.
    confirmed: bool,
    /// A page of the strip held down: where it was, and the page it is
    /// over now (where it goes if let go).
    drag: Option<(usize, Option<usize>)>,
    /// Changes each time the pages, or how they look, change: the estimate
    /// of the PDF follows it.
    pdf_rev: u64,
    /// The last estimate of the PDF (what it was for, the pages measured).
    pdf_estimate: Option<Arc<Estimated>>,
    /// An estimate is being made.
    estimating: bool,
    /// The largest PDF as typed (none: shown from the settings).
    limit_draft: Option<String>,
    /// Minutes without scanning before the lamp goes off (0: as soon as a
    /// scan is over).
    lamp_off: u8,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let stored = prefs::Prefs::load();
        let start_mode = match stored.start_mode.as_str() {
            "documento" => StartMode::Document,
            "foto" => StartMode::Photo,
            _ => StartMode::Last,
        };
        let kind = match (start_mode, stored.last_mode.as_str()) {
            (StartMode::Document, _) => Kind::Document,
            (StartMode::Photo, _) => Kind::Photo,
            (StartMode::Last, "foto") => Kind::Photo,
            (StartMode::Last, _) => Kind::Document,
        };
        let theme = match stored.theme.as_str() {
            "chiaro" => ThemeChoice::Light,
            "scuro" => ThemeChoice::Dark,
            _ => ThemeChoice::System,
        };
        (
            Self {
                worker: None,
                status: None,
                caps: worker::registry()
                    .models()
                    .into_iter()
                    .next()
                    .unwrap_or_else(Capabilities::placeholder),
                device: None,
                devices: Vec::new(),
                access_denied: false,
                system_dark: false,
                theme,
                kind,
                // How the scan is made and saved comes back; the look (the
                // adjustments, the black-and-white threshold) starts clean
                // at every opening: a look to keep is a preset.
                document: Settings::from_stored(Kind::Document, &stored.document).fresh_look(),
                photo: Settings::from_stored(Kind::Photo, &stored.photo).fresh_look(),
                tab: Tab::Preview,
                pane: Pane::Scan,
                start_mode,
                custom_folder: (!stored.folder.is_empty()).then(|| PathBuf::from(&stored.folder)),
                document_name: stored.document_name.clone(),
                photo_name: stored.photo_name.clone(),
                button_actions: stored.buttons.clone(),
                print_next: false,
                wia_quiet: wia::quiet(),
                saved: stored.clone(),
                preview: None,
                preview_view: View::default(),
                result_view: View::default(),
                selection: None,
                found: Vec::new(),
                job: None,
                jobs: std::collections::VecDeque::new(),
                batch: (0, 0),
                open: vec![Group::Tone, Group::Colour],
                eyedropper: false,
                before: false,
                busy: None,
                pages: Vec::new(),
                current: None,
                redo: None,
                redo_auto: false,
                redone: None,
                note: "Pronto: fai un'anteprima, scegli l'area e scansiona".into(),
                draft: None,
                curve_channel: CurveChannel::All,
                curve_drag: None,
                presets: prefs::load_presets(),
                preset_name: String::new(),
                undo: Vec::new(),
                redo_stack: Vec::new(),
                last_edit: None,
                rendering: false,
                dirty_preview: false,
                dirty_page: false,
                stale: Vec::new(),
                epoch: 0,
                editor: None,
                edit_section: Section::Colour,
                unsaved: false,
                confirm: None,
                after_save: None,
                confirmed: false,
                drag: None,
                pdf_rev: 0,
                pdf_estimate: None,
                estimating: false,
                limit_draft: None,
                lamp_off: stored.lamp_off_minutes.min(LAMP_SCANNER),
            },
            iced::system::theme().map(Message::SystemTheme),
        )
    }

    fn theme(&self) -> Theme {
        let dark = match self.theme {
            ThemeChoice::System => self.system_dark,
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
        };
        if dark {
            Theme::Dark
        } else {
            Theme::Light
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(worker::events).map(Message::Worker),
            iced::system::theme_changes().map(Message::SystemTheme),
            iced::keyboard::listen().map(Message::Key),
            iced::window::close_requests().map(Message::CloseRequested),
            // A page dragged in the strip is let go wherever the mouse goes
            // up (no-op when nothing is dragged).
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                    iced::mouse::Button::Left,
                )) => Some(Message::PageDrop),
                _ => None,
            }),
        ])
    }

    /// What the estimate of the PDF is for now.
    fn estimate_key(&self) -> EstimateKey {
        let s = self.settings();
        (self.pdf_rev, self.kind, s.pdf_quality, s.pdf_limit_kb)
    }

    /// Starts the estimate of the PDF when the output is a PDF and the
    /// last one is out of date (one at a time; the pages are measured
    /// again only when they changed).
    fn schedule_estimate(&mut self) -> Task<Message> {
        let s = *self.settings();
        if s.output != Output::Pdf || self.pages.is_empty() || self.estimating {
            return Task::none();
        }
        let key = self.estimate_key();
        if self.pdf_estimate.as_ref().is_some_and(|e| e.key == key) {
            return Task::none();
        }
        let measured = self
            .pdf_estimate
            .as_ref()
            .filter(|e| (e.key.0, e.key.1) == (key.0, key.1))
            .map(|e| e.samples.clone());
        let pages = if measured.is_none() {
            self.pages.clone()
        } else {
            Vec::new()
        };
        self.estimating = true;
        let base = hp3770_export::Encoding::from_percent(s.pdf_quality);
        let limit = s.pdf_limit_kb.map(|kb| u64::from(kb) * 1024);
        Task::perform(
            async move {
                let samples = measured
                    .unwrap_or_else(|| Arc::new(pages.iter().map(|p| p.pdf_sample(&s)).collect()));
                let result =
                    hp3770_export::compress::plan(&samples, base, limit).map_err(|e| e.to_string());
                Arc::new(Estimated {
                    key,
                    samples,
                    result,
                })
            },
            Message::Estimated,
        )
    }

    /// What the question says is lost: "Le 3 pagine acquisite non sono
    /// salvate e andranno perse."
    fn lost_sentence(&self, n: usize) -> String {
        let what = match self.kind {
            Kind::Document => ("La pagina acquisita", "pagine"),
            Kind::Photo => ("La foto acquisita", "foto"),
        };
        if n == 1 {
            format!("{} non è salvata e andrà persa.", what.0)
        } else {
            format!(
                "Le {n} {} acquisite non sono salvate e andranno perse.",
                what.1
            )
        }
    }

    /// Before an action that throws scans away: shows the question, and
    /// `then` is done again once confirmed. True when it asked (the
    /// action stops here); false when it was confirmed already.
    fn ask(&mut self, then: Message, title: String, body: String, yes: impl Into<String>) -> bool {
        if self.confirmed {
            return false;
        }
        self.confirm = Some(Confirm {
            title,
            body,
            yes: yes.into(),
            then: Box::new(then),
        });
        true
    }

    /// Does an action the question was answered yes to.
    fn run_confirmed(&mut self, then: Message) -> Task<Message> {
        self.confirmed = true;
        let task = self.handle(then);
        self.confirmed = false;
        task
    }

    /// Whether these messages change how the pages come out (their look
    /// follows the shared settings unless they have their own).
    fn changes_pages(message: &Message) -> bool {
        if let Message::Edit(m) = message {
            return !matches!(
                m,
                EditMsg::Open(_) | EditMsg::Discard | EditMsg::Section(_) | EditMsg::Unit(_)
            );
        }
        matches!(
            message,
            Message::Undo
                | Message::Redo
                | Message::CurveSet(_)
                | Message::Descreen(_)
                | Message::Adaptive(_)
                | Message::OwnLook(_)
                | Message::LookApplyToAll
                | Message::PresetApply(_)
                | Message::Adjust(..)
                | Message::Commit(_)
                | Message::AutoLevels
                | Message::AutoThreshold
                | Message::Picked(_)
                | Message::ResetGroup(_)
                | Message::Colour(_)
                | Message::PageRotate(..)
                | Message::PageMove(..)
                | Message::PageDelete(_)
                | Message::PageDuplicate(_)
                | Message::PdfOwnLayout(_)
                | Message::PdfApplyToAll
                | Message::PageFocus(_)
                | Message::AcceptRedo
        )
    }

    fn settings(&self) -> &Settings {
        match self.kind {
            Kind::Document => &self.document,
            Kind::Photo => &self.photo,
        }
    }

    fn settings_mut(&mut self) -> &mut Settings {
        match self.kind {
            Kind::Document => &mut self.document,
            Kind::Photo => &mut self.photo,
        }
    }

    /// The page the result tab shows: a rescan waiting for a choice, else
    /// the current page.
    fn shown(&self) -> Option<&Page> {
        self.redone
            .as_ref()
            .map(|(_, p)| p)
            .or_else(|| self.current.and_then(|i| self.pages.get(i)))
    }

    fn shown_mut(&mut self) -> Option<&mut Page> {
        match &mut self.redone {
            Some((_, p)) => Some(p),
            None => self.current.and_then(|i| self.pages.get_mut(i)),
        }
    }

    /// Which page the result tab shows: the current one, or `None` for a
    /// rescan waiting for a choice.
    fn shown_key(&self) -> Option<usize> {
        if self.redone.is_some() {
            None
        } else {
            self.current
        }
    }

    /// Whether the panel edits a scanned page (else the settings of the
    /// next scan, shown on the preview).
    fn editing_page(&self) -> bool {
        self.tab == Tab::Result && self.shown().is_some()
    }

    /// The settings the panel shows and edits.
    fn active(&self) -> Settings {
        let shared = *self.settings();
        if self.editing_page() {
            self.shown().map(|p| p.effective(&shared)).unwrap_or(shared)
        } else {
            shared
        }
    }

    /// Whether the panel edits a page with a look of its own.
    fn editing_own(&self) -> bool {
        self.editing_page() && self.shown().is_some_and(|p| p.own.is_some())
    }

    /// The shared look changed: redraw the preview, the page shown if it
    /// follows it, and (in the background) the other pages that do.
    fn shared_changed(&mut self) {
        self.dirty_preview = true;
        if self.shown().is_some_and(|p| p.own.is_none()) {
            self.dirty_page = true;
        }
        self.restale();
    }

    /// Every page following the shared look, but the one shown, is to be
    /// drawn again in the background.
    fn restale(&mut self) {
        let shown = self.current;
        self.stale = self
            .pages
            .iter()
            .enumerate()
            .filter(|(i, p)| p.own.is_none() && Some(*i) != shown)
            .map(|(i, _)| i)
            .collect();
    }

    /// Pages moved, went or were replaced: what knew them by their place
    /// (renders running, pages waiting to be drawn, Ctrl+Z of a page's
    /// own look) no longer holds.
    fn pages_changed(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.pdf_rev += 1;
        if !self.stale.is_empty() {
            self.restale();
        }
        self.dirty_page = true;
        self.forget_page_undo();
    }

    /// Drops the Ctrl+Z steps of pages' own looks.
    fn forget_page_undo(&mut self) {
        self.undo.retain(|(t, _)| *t == Target::Preview);
        self.redo_stack.retain(|(t, _)| *t == Target::Preview);
    }

    /// Applies a change to the active settings and redraws what shows it.
    /// New scans start from the last settings used.
    fn change(&mut self, f: impl Fn(&mut Settings)) {
        self.remember(None);
        self.apply(f);
    }

    /// Changes one field: moving the same slider again within a second is
    /// the same step of Ctrl+Z.
    fn change_field(&mut self, field: Field, f: impl Fn(&mut Settings)) {
        self.remember(Some(field));
        self.apply(f);
    }

    /// Saves the settings the panel edits for Ctrl+Z: the page's own look,
    /// or the shared settings.
    fn remember(&mut self, field: Option<Field>) {
        if self.editor().is_some() {
            self.edit_remember(field.map(editor::Key::Field));
            return;
        }
        let now = std::time::Instant::now();
        let same = matches!(
            (field, self.last_edit),
            (Some(f), Some((g, at))) if f == g && now - at < std::time::Duration::from_secs(1)
        );
        self.last_edit = field.map(|f| (f, now));
        if same {
            return;
        }
        // A page's own look is kept with the page it belongs to:
        // `Index(i)` for page i, `Page` for a new scan waiting for a choice.
        let entry = if self.editing_own() {
            let page = match self.shown_key() {
                Some(i) => Target::Index(i),
                None => Target::Page,
            };
            (page, self.active())
        } else {
            (Target::Preview, *self.settings())
        };
        self.undo.push(entry);
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Puts back settings saved for Ctrl+Z / Ctrl+Y; returns the ones they
    /// replace.
    fn restore(&mut self, target: Target, settings: Settings) -> Option<(Target, Settings)> {
        self.last_edit = None;
        match target {
            Target::Page | Target::Index(_) => {
                let shared = *self.settings();
                // Whether the result tab shows that page now.
                let shown = self.tab == Tab::Result
                    && match target {
                        Target::Index(i) => self.shown_key() == Some(i),
                        _ => self.redone.is_some(),
                    };
                let page = match target {
                    Target::Index(i) => self.pages.get_mut(i),
                    _ => self.redone.as_mut().map(|(_, p)| p),
                }?;
                let current = page.effective(&shared);
                page.own = Some(Look::of(&settings));
                page.settings.colour = settings.colour;
                if shown {
                    self.dirty_page = true;
                } else {
                    // Out of sight: its thumbnail now.
                    page.redraw(false, &shared);
                }
                Some((target, current))
            }
            Target::Preview => {
                let current = *self.settings();
                *self.settings_mut() = settings;
                self.shared_changed();
                Some((Target::Preview, current))
            }
        }
    }

    /// A change to what the panel edits. On a page with its own look, that
    /// look; otherwise the shared settings, which every page following
    /// them shows. The colour mode of a page shown stays its own.
    fn apply(&mut self, f: impl Fn(&mut Settings)) {
        if self.editor().is_some() {
            // The editor gives the page a look of its own.
            let shared = *self.settings();
            if let Some(page) = self.shown_mut() {
                let mut s = page.effective(&shared);
                f(&mut s);
                page.settings.colour = s.colour;
                page.own = Some(Look::of(&s));
            }
            self.dirty_page = true;
            return;
        }
        if self.editing_page() {
            let shared = *self.settings();
            let Some(page) = self.shown_mut() else {
                return;
            };
            let mut s = page.effective(&shared);
            f(&mut s);
            page.settings.colour = s.colour;
            if page.own.is_some() {
                page.own = Some(Look::of(&s));
                self.dirty_page = true;
            } else {
                Look::of(&s).put(self.settings_mut());
                self.shared_changed();
            }
        } else {
            f(self.settings_mut());
            self.shared_changed();
        }
    }

    /// Starts the render of what changed, off the interface thread, unless
    /// one is running: then the latest values are rendered when it ends
    /// (a slider dragged fast skips the values in between).
    fn schedule(&mut self) -> Task<Message> {
        if self.rendering {
            return Task::none();
        }
        let before = self.before;
        let shared = *self.settings();
        let target = if std::mem::take(&mut self.dirty_page) {
            Target::Page
        } else if std::mem::take(&mut self.dirty_preview) {
            Target::Preview
        } else if let Some(i) = self.stale.pop() {
            Target::Index(i)
        } else {
            return Task::none();
        };
        let job = match target {
            Target::Preview => self.preview.as_ref().map(|(raw, ..)| {
                let adjust = if before {
                    Adjustments::default()
                } else {
                    shared.adjust
                };
                (
                    raw.clone(),
                    shared,
                    adjust,
                    Transform::default(),
                    None,
                    false,
                )
            }),
            Target::Page => self.shown().map(|p| self.page_job(p, true)),
            Target::Index(i) => self.pages.get(i).map(|p| self.page_job(p, false)),
        };
        let Some((raw, settings, adjust, transform, size, thumb)) = job else {
            return Task::none();
        };
        let page = self.shown_key();
        let epoch = self.epoch;
        self.rendering = true;
        Task::perform(
            async move {
                let (shown, mut picture, thumb) = draw_page(
                    &raw.proxy,
                    &raw.tone,
                    &settings,
                    &adjust,
                    &transform,
                    size.unwrap_or_default(),
                    thumb,
                );
                if size.is_none() {
                    picture.size = Size::new(shown.width as f32, shown.height as f32);
                }
                Arc::new(Rendered {
                    target,
                    page,
                    epoch,
                    shown,
                    picture,
                    thumb,
                })
            },
            Message::Rendered,
        )
    }

    /// What to render of a page: the small copy, settings, adjustments,
    /// geometry, size shown and whether the thumbnail follows. With the
    /// editor open "Prima" is the page as it was when it opened.
    #[allow(clippy::type_complexity)]
    fn page_job(
        &self,
        page: &Page,
        shown: bool,
    ) -> (Raw, Settings, Adjustments, Transform, Option<Size>, bool) {
        let shared = *self.settings();
        let editor = self.editor().filter(|_| shown);
        let edit = match editor {
            Some(e) if self.before => e.start,
            _ => page.edit(),
        };
        let settings = page.settings_with(&edit, &shared);
        let adjust = if self.before && editor.is_none() && shown {
            Adjustments::default()
        } else {
            settings.adjust
        };
        let frame = shown && self.cropping();
        let (t, size) = page.preview_geometry(&edit.transform, frame);
        let thumb = !frame && !(self.before && shown);
        (page.raw.clone(), settings, adjust, t, Some(size), thumb)
    }

    fn redraw_preview(&mut self) {
        let settings = *self.settings();
        let adjust = if self.before {
            Adjustments::default()
        } else {
            settings.adjust
        };
        if let Some((raw, shown, picture)) = &mut self.preview {
            *shown = render(&raw.proxy, &settings, &adjust, &raw.tone);
            *picture =
                Picture::from_rgb(shown.width, shown.height, &shown.to_rgb()).replacing(picture);
        }
    }

    /// Size of the glass in preview pixels.
    fn glass(&self) -> Size {
        let (w, h) = self.caps.glass_pixels(PREVIEW_DPI);
        Size::new(w as f32, h as f32)
    }

    /// The area to scan in millimetres: the selection, or an A4 page.
    fn area_mm(&self) -> (f64, f64, f64, f64) {
        let mm = |px: f32| f64::from(px) * 25.4 / f64::from(PREVIEW_DPI);
        match self.selection {
            Some(s) if s.width >= 4.0 && s.height >= 4.0 => {
                (mm(s.x), mm(s.y), mm(s.width), mm(s.height))
            }
            _ => (0.0, 0.0, 210.0, 297.0),
        }
    }

    /// The scan: the chip's linear colour data, every adjustment is done
    /// afterwards.
    fn request(&self) -> ScanRequest {
        self.request_mm(self.area_mm())
    }

    /// The resolution chosen, or the nearest the scanner in use offers
    /// (the choice stays saved for a scanner that has it).
    fn dpi(&self) -> u16 {
        nearest(&self.caps.resolutions, self.settings().dpi)
    }

    /// The resolution of the preview: the scanner's nearest to
    /// [`PREVIEW_DPI`] (the preview is brought to that afterwards).
    fn preview_dpi(&self) -> u16 {
        nearest(&self.caps.resolutions, PREVIEW_DPI)
    }

    /// Whether the scanner in use gives 16-bit samples.
    fn offers_16bit(&self) -> bool {
        self.caps.depths.contains(&16)
    }

    /// The bits per sample chosen, if the scanner in use gives them (8
    /// otherwise; the choice stays saved).
    fn depth(&self) -> u8 {
        if self.offers_16bit() {
            self.settings().depth
        } else {
            8
        }
    }

    /// The scan of an area given in millimetres.
    fn request_mm(&self, (x, y, w, h): (f64, f64, f64, f64)) -> ScanRequest {
        let dpi = self.dpi();
        // A RAW file always gets the 16-bit samples, when there are some.
        let s = self.settings();
        let raw = matches!(s.output, Output::Raw(_));
        let depth = if self.depth() == 16 || (raw && self.offers_16bit()) {
            16
        } else {
            8
        };
        let area = within_glass(area_mm(dpi, x, y, w, h), dpi, &self.caps);
        ScanRequest { dpi, area, depth }
    }

    /// The passes for the originals found, when the automatic crop applies
    /// (it on, something found, no selection): all in one pass, or each
    /// on its own when one pass would take too much memory.
    fn plan(&self) -> Vec<Job> {
        if !self.settings().auto_crop || self.selection.is_some() || self.found.is_empty() {
            return Vec::new();
        }
        let glass = self.glass();
        let margin = f32::from(PREVIEW_DPI) / 25.4;
        let expand = |b: [f32; 4]| {
            let (l, t) = ((b[0] - margin).max(0.0), (b[1] - margin).max(0.0));
            let r = (b[0] + b[2] + margin).min(glass.width);
            let bottom = (b[1] + b[3] + margin).min(glass.height);
            [l, t, r - l, bottom - t]
        };
        let union = self
            .found
            .iter()
            .map(Found::bounds)
            .reduce(|a, b| {
                let (l, t) = (a[0].min(b[0]), a[1].min(b[1]));
                let r = (a[0] + a[2]).max(b[0] + b[2]);
                let bottom = (a[1] + a[3]).max(b[1] + b[3]);
                [l, t, r - l, bottom - t]
            })
            .map(expand)
            .unwrap_or_default();
        let one = Job {
            bounds: union,
            crops: self.found.clone(),
            area: None,
        };
        if self.found.len() == 1 || self.job_memory(&one) <= ONE_PASS {
            vec![one]
        } else {
            self.found
                .iter()
                .map(|f| Job {
                    bounds: expand(f.bounds()),
                    crops: vec![*f],
                    area: None,
                })
                .collect()
        }
    }

    /// The scan of a pass.
    fn job_request(&self, job: &Job) -> ScanRequest {
        let mm = |px: f32| f64::from(px) * 25.4 / f64::from(PREVIEW_DPI);
        let [x, y, w, h] = job.bounds;
        self.request_mm((mm(x), mm(y), mm(w), mm(h)))
    }

    fn job_memory(&self, job: &Job) -> f64 {
        Self::bytes(&self.job_request(job), self.dpi())
    }

    fn bytes(request: &ScanRequest, dpi: u16) -> f64 {
        let a = request.area;
        let raw = 3.0 * f64::from(a.width) * f64::from(a.height);
        // 2400 dpi: the chip's half-width data and the doubled image; 16
        // bits: the samples kept besides the 8-bit image.
        let resolution = if dpi > 1200 { 2.0 } else { 1.0 };
        let depth = if request.depth == 16 { 3.0 } else { 1.0 };
        raw * resolution * depth
    }

    /// Bytes the scan takes in memory.
    fn memory(&self) -> f64 {
        let plan = self.plan();
        if !plan.is_empty() {
            return plan.iter().map(|j| self.job_memory(j)).fold(0.0, f64::max);
        }
        Self::bytes(&self.request(), self.dpi())
    }

    fn start(&mut self, purpose: Purpose) {
        if !self.idle() {
            // Nothing is sent: a "Rifai" asked for does not wait for the
            // next scan.
            self.redo = None;
            self.redo_auto = false;
            return;
        }
        // In Foto a new scan takes the place of the photos there.
        let n = self.pages.len();
        if purpose == Purpose::Page
            && self.kind == Kind::Photo
            && self.redo.is_none()
            && self.jobs.is_empty()
            && n > 0
            && self.unsaved
            && self.ask(
                Message::Scan,
                "Fare una nuova scansione?".into(),
                format!(
                    "{} La nuova scansione {} sostituisce.",
                    self.lost_sentence(n),
                    if n == 1 { "la" } else { "le" }
                ),
                "Scansiona",
            )
        {
            return;
        }
        let request = if purpose == Purpose::Preview {
            let dpi = self.preview_dpi();
            let (width, height) = self.caps.glass_pixels(dpi);
            ScanRequest {
                area: Area {
                    x: 0,
                    y: 0,
                    width,
                    height,
                },
                dpi,
                ..self.request()
            }
        } else if let Some(job) = self.next_job() {
            let mut request = self.job_request(&job);
            self.job = Some(Job {
                area: Some(request.area),
                ..job
            });
            request.dpi = self.dpi();
            request
        } else {
            self.batch = (0, 1);
            self.request()
        };
        if let (Some(worker), None) = (&self.worker, &self.busy) {
            worker.send(Command::Scan(purpose, request));
            self.busy = Some((purpose, None));
            self.eyedropper = false;
        }
    }

    /// The pass to scan now: the next of the running scan, or the first
    /// of a new plan (also for "Rifai" on an original cut out).
    fn next_job(&mut self) -> Option<Job> {
        if self.busy.is_some() || self.worker.is_none() {
            return None;
        }
        if let Some(job) = self.jobs.pop_front() {
            return Some(job);
        }
        if let Some(i) = self.redo {
            let origin = self.pages.get(i).and_then(|p| p.origin)?;
            let margin = f32::from(PREVIEW_DPI) / 25.4;
            let b = origin.bounds();
            self.batch = (0, 1);
            return Some(Job {
                bounds: [
                    (b[0] - margin).max(0.0),
                    (b[1] - margin).max(0.0),
                    b[2] + 2.0 * margin,
                    b[3] + 2.0 * margin,
                ],
                crops: vec![origin],
                area: None,
            });
        }
        let mut plan: std::collections::VecDeque<Job> = self.plan().into();
        let first = plan.pop_front()?;
        self.batch = (0, self.found.len());
        self.jobs = plan;
        Some(first)
    }

    /// The images of a scan: the originals cut out and straightened, or
    /// the whole scan.
    fn cut(
        image: Image,
        samples16: Option<Vec<u16>>,
        job: Option<&Job>,
    ) -> Vec<(Image, Option<Found>, Option<Vec<u16>>)> {
        let Some((job, area)) = job.and_then(|j| Some((j, j.area?))) else {
            return vec![(image, None, samples16)];
        };
        let factor = f32::from(image.dpi) / f32::from(PREVIEW_DPI);
        let inset = INSET_MM / 25.4 * f32::from(image.dpi);
        job.crops
            .iter()
            .map(|c| {
                let mut f = c.scaled(factor, -(area.x as f32), -(area.y as f32));
                f.width = (f.width - 2.0 * inset).max(1.0);
                f.height = (f.height - 2.0 * inset).max(1.0);
                let samples = samples16
                    .as_ref()
                    .map(|s| detect::straightened16(s, image.width, image.height, &f).2);
                (image.straightened(&f), Some(*c), samples)
            })
            .collect()
    }

    /// The usual folder of saved scans: Pictures\Scansioni.
    fn default_folder() -> PathBuf {
        #[cfg(test)]
        return testing::dir().join("Scansioni");
        #[cfg(not(test))]
        dirs::picture_dir()
            .or_else(dirs::document_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Scansioni")
    }

    /// Where scans are saved.
    fn folder(&self) -> PathBuf {
        self.custom_folder
            .clone()
            .unwrap_or_else(Self::default_folder)
    }

    /// The start of file names in the current way of working.
    fn name(&self) -> &str {
        let name = match self.kind {
            Kind::Document => &self.document_name,
            Kind::Photo => &self.photo_name,
        };
        if name.trim().is_empty() {
            "scansione"
        } else {
            name.trim()
        }
    }

    /// The preferences as they are now.
    fn prefs(&self) -> prefs::Prefs {
        prefs::Prefs {
            start_mode: match self.start_mode {
                StartMode::Last => "ultimo",
                StartMode::Document => "documento",
                StartMode::Photo => "foto",
            }
            .into(),
            last_mode: match self.kind {
                Kind::Document => "documento",
                Kind::Photo => "foto",
            }
            .into(),
            theme: match self.theme {
                ThemeChoice::System => "sistema",
                ThemeChoice::Light => "chiaro",
                ThemeChoice::Dark => "scuro",
            }
            .into(),
            folder: self
                .custom_folder
                .as_ref()
                .map(|f| f.display().to_string())
                .unwrap_or_default(),
            document_name: self.document_name.clone(),
            photo_name: self.photo_name.clone(),
            // For older versions: the HP 3770's buttons by position.
            document_buttons: self.legacy_button_keys(Kind::Document),
            photo_buttons: self.legacy_button_keys(Kind::Photo),
            buttons: self.button_actions.clone(),
            document: self.document.to_stored(),
            photo: self.photo.to_stored(),
            lamp_off_minutes: self.lamp_off,
        }
    }

    /// The own layout of the page shown, if it has one.
    fn own_layout_mut(&mut self) -> Option<&mut PageLayout> {
        self.current
            .and_then(|i| self.pages.get_mut(i))
            .and_then(|p| p.layout.as_mut())
    }

    /// The layout of the page shown in the PDF, and whether it is its own.
    fn shown_layout(&self) -> (PageLayout, bool) {
        let own = self
            .current
            .and_then(|i| self.pages.get(i))
            .and_then(|p| p.layout);
        (own.unwrap_or(self.settings().page_layout()), own.is_some())
    }

    /// Whether the viewer shows the page on its sheet: the output is a PDF
    /// on real paper, its panel open, a page there.
    fn print_preview(&self) -> bool {
        let s = self.settings();
        self.pane == Pane::Output
            && s.output == Output::Pdf
            && s.pdf_size != PdfSize::Fit
            && self.redone.is_none()
            && self.editor.is_none()
            && self.current.is_some_and(|i| i < self.pages.len())
    }

    /// What button `index` of the scanner in use does in `kind`: the
    /// action chosen for that model, or the default of what it is meant for.
    fn button_action(&self, kind: Kind, index: usize) -> ButtonAction {
        let Some(button) = self.caps.buttons.get(index) else {
            return ButtonAction::Nothing;
        };
        self.button_actions
            .get(&self.caps.name())
            .and_then(|m| match kind {
                Kind::Document => m.document.get(&button.id),
                Kind::Photo => m.photo.get(&button.id),
            })
            .and_then(|key| ButtonAction::from_key(kind, key))
            .unwrap_or_else(|| ButtonAction::default_for(kind, button.action))
    }

    /// Chooses what button `index` of the scanner in use does in the
    /// current way of working, for that model.
    fn set_button_action(&mut self, index: usize, action: ButtonAction) {
        let Some(button) = self.caps.buttons.get(index) else {
            return;
        };
        let model = self.button_actions.entry(self.caps.name()).or_default();
        let map = match self.kind {
            Kind::Document => &mut model.document,
            Kind::Photo => &mut model.photo,
        };
        map.insert(button.id.clone(), action.key().into());
    }

    /// The actions of the HP Scanjet 3770's buttons by position, as the
    /// preferences before v0.2.0 kept them.
    fn legacy_button_keys(&self, kind: Kind) -> Vec<String> {
        let suggested = [
            SuggestedAction::Scan,
            SuggestedAction::Copy,
            SuggestedAction::Share,
        ];
        let model = self.button_actions.get(prefs::LEGACY_MODEL);
        prefs::LEGACY_BUTTONS
            .iter()
            .zip(suggested)
            .map(|(id, action)| {
                model
                    .and_then(|m| match kind {
                        Kind::Document => m.document.get(*id),
                        Kind::Photo => m.photo.get(*id),
                    })
                    .and_then(|key| ButtonAction::from_key(kind, key))
                    .unwrap_or_else(|| ButtonAction::default_for(kind, action))
                    .key()
                    .to_string()
            })
            .collect()
    }

    /// Tells the scanner thread when to switch the lamp off.
    fn send_lamp_timer(&self) {
        if self.caps.lamp.is_none() {
            return;
        }
        if let Some(worker) = &self.worker {
            let after = std::time::Duration::from_secs(60 * u64::from(self.lamp_off));
            worker.send(Command::LampTimer(after));
        }
    }

    /// A file name with the date and time.
    fn stamped(prefix: &str, extension: &str) -> String {
        format!(
            "{prefix}-{}.{extension}",
            chrono::Local::now().format("%Y-%m-%d-%H%M%S")
        )
    }

    /// A name made of the date and time that no file in `folder` has yet,
    /// nor the files of each page next to it (`NAME-001.ext`): two saves
    /// in the same second would otherwise write over the first without a
    /// word. "NAME (2).ext" when taken.
    fn free_name(folder: &std::path::Path, prefix: &str, extension: &str) -> String {
        let stamped = Self::stamped(prefix, extension);
        let stem = stamped
            .strip_suffix(&format!(".{extension}"))
            .unwrap_or(&stamped)
            .to_string();
        let names: Vec<String> = std::fs::read_dir(folder)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        let taken = |stem: &str| {
            names.iter().any(|n| {
                n.ends_with(&format!(".{extension}"))
                    && (n == &format!("{stem}.{extension}") || n.starts_with(&format!("{stem}-")))
            })
        };
        let mut candidate = stem.clone();
        let mut n = 2;
        while taken(&candidate) {
            candidate = format!("{stem} ({n})");
            n += 1;
        }
        format!("{candidate}.{extension}")
    }

    /// Writes the document (all pages) or the shown photo to `path`.
    fn write(&self, path: &std::path::Path) -> Result<String, String> {
        let settings = self.settings();
        if let Output::Raw(format) = settings.output {
            return self.write_raw(path, format);
        }
        if let Output::Deep(_) = settings.output {
            return self.write_deep(path);
        }
        let pages: Vec<Image> = self.pages.iter().map(|p| p.finished(settings)).collect();
        if pages.is_empty() {
            return Err("niente da salvare".into());
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("pagina")
            .to_string();
        let n = pages.len();
        let folder = path.parent().unwrap_or(std::path::Path::new("."));
        let format = image_format(path, settings.output);
        match writer(settings.output, path, n) {
            Writer::Pdf => {
                let summary = self.write_pdf(path, &pages)?;
                return Ok(format!("{} · {summary}", path.display()));
            }
            Writer::Tiff => hp3770_export::save_tiff(path, &pages),
            Writer::Zip => hp3770_export::save_zip(path, &name, &pages, ImageFormat::Png),
            // One image per page, next to the chosen name.
            Writer::Folder => hp3770_export::save_folder(folder, &name, &pages, format).map(|_| ()),
            Writer::One => hp3770_export::save_as(path, &pages[0], format),
        }
        .map_err(|e| e.to_string())?;
        let noun = match self.kind {
            Kind::Document => "pagine",
            Kind::Photo => "foto",
        };
        Ok(match writer(settings.output, path, n) {
            Writer::One => path.display().to_string(),
            _ if n == 1 => path.display().to_string(),
            Writer::Folder => format!("{n} file in {}", folder.display()),
            _ => format!("{n} {noun} in {}", path.display()),
        })
    }

    /// Writes the finished `pages` as a PDF: each on its sheet, its
    /// pictures at the quality chosen and, with a limit, lightened page by
    /// page until the file fits (measured on the file written, and done
    /// again tighter if the estimate fell short). Says how large it is and
    /// what was lightened.
    fn write_pdf(&self, path: &std::path::Path, pages: &[Image]) -> Result<String, String> {
        use hp3770_export::{compress, Encoding, PdfPage, Sample};
        let settings = self.settings();
        let base = Encoding::from_percent(settings.pdf_quality);
        let limit = settings.pdf_limit_kb.map(|kb| u64::from(kb) * 1024);
        let samples: Vec<Sample> = match limit {
            Some(_) => pages.iter().map(Sample::new).collect(),
            None => Vec::new(),
        };
        let mut budget = limit;
        let mut planned = None;
        for _ in 0..4 {
            let encodings = match budget {
                Some(b) => {
                    let p = compress::plan(&samples, base, Some(b)).map_err(|e| e.to_string())?;
                    let encodings = p.encodings.clone();
                    planned = Some(p);
                    encodings
                }
                None => vec![base; pages.len()],
            };
            let laid: Vec<PdfPage> = pages
                .iter()
                .zip(&self.pages)
                .zip(&encodings)
                .map(|((image, page), &encoding)| PdfPage {
                    image,
                    layout: settings.layout(page.layout, page.focus),
                    encoding,
                })
                .collect();
            hp3770_export::save_pdf_pages(path, &laid).map_err(|e| e.to_string())?;
            let written = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
            match (limit, budget) {
                // Over the limit by the estimate's error: plan again for
                // less, by as much as it fell short.
                (Some(l), Some(b)) if written > l && planned.as_ref().is_some_and(|p| p.fits) => {
                    budget = Some((b as f64 * l as f64 / written as f64 * 0.97) as u64);
                }
                _ => return Ok(self.pdf_summary(written, limit, &encodings, base)),
            }
        }
        let written = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        let encodings = planned.map_or_else(Vec::new, |p| p.encodings);
        Ok(self.pdf_summary(written, limit, &encodings, base))
    }

    /// "PDF di 4,7 MB (limite 5 MB) · 3 pagine alleggerite, fino a 200 dpi
    /// e JPEG 65".
    fn pdf_summary(
        &self,
        bytes: u64,
        limit: Option<u64>,
        encodings: &[hp3770_export::Encoding],
        base: hp3770_export::Encoding,
    ) -> String {
        let mut out = format!("PDF di {}", human_size(bytes));
        if let Some(l) = limit {
            if bytes <= l {
                out.push_str(&format!(" (limite {})", human_size(l)));
            } else {
                out.push_str(&format!(
                    ", oltre il limite di {}: le pagine sono già alleggerite al massimo",
                    human_size(l)
                ));
            }
        }
        if let Some(lightened) = lightened(encodings, base) {
            out.push_str(&format!(" · {lightened}"));
        }
        out
    }

    /// Writes every page (one file each) finished at 16 bits per channel:
    /// from the 16-bit samples, or the 8-bit image widened for pages
    /// scanned at 8 bits.
    fn write_deep(&self, path: &std::path::Path) -> Result<String, String> {
        if self.pages.is_empty() {
            return Err("niente da salvare".into());
        }
        let shared = self.settings();
        let n = self.pages.len();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("foto")
            .to_string();
        let ext = shared.output.extension();
        for (i, page) in self.pages.iter().enumerate() {
            let target = if n == 1 {
                path.to_path_buf()
            } else {
                path.with_file_name(format!("{stem}-{:03}.{ext}", i + 1))
            };
            let settings = page.effective(shared);
            let (full, raw16) = page.full_samples();
            let deep = raw16.as_ref().and_then(|samples| {
                adjust::render16(
                    samples,
                    full.width,
                    full.height,
                    full.dpi,
                    settings.mode(),
                    &settings.adjust,
                    false,
                    &page.raw.tone,
                )
            });
            let (data, channels, w, h, dpi) = match deep {
                Some(data) => {
                    let channels = if settings.colour == Colour::Grey {
                        1
                    } else {
                        3
                    };
                    (data, channels, full.width, full.height, full.dpi)
                }
                None => {
                    let image = page.finished(shared);
                    let channels = match image.format {
                        Format::Rgb => 3,
                        _ => 1,
                    };
                    let data = match image.format {
                        Format::Lineart => {
                            return Err("bianco e nero: niente 16 bit".into());
                        }
                        _ => image.data.iter().map(|&v| u16::from(v) * 257).collect(),
                    };
                    (data, channels, image.width, image.height, image.dpi)
                }
            };
            hp3770_export::save_deep(&target, w, h, channels, dpi, &data)
                .map_err(|e| e.to_string())?;
        }
        Ok(if n == 1 {
            path.display().to_string()
        } else {
            format!(
                "{n} file a 16 bit in {}",
                path.parent().unwrap_or(path).display()
            )
        })
    }

    /// Writes the linear samples of every page (one file each), without
    /// adjustments: the white the calibration aims at is full scale.
    fn write_raw(&self, path: &std::path::Path, format: RawFormat) -> Result<String, String> {
        if self.pages.is_empty() {
            return Err("niente da salvare".into());
        }
        let n = self.pages.len();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("foto")
            .to_string();
        let ext = Output::Raw(format).extension();
        let mut written = Vec::new();
        for (i, page) in self.pages.iter().enumerate() {
            let target = if n == 1 {
                path.to_path_buf()
            } else {
                path.with_file_name(format!("{stem}-{:03}.{ext}", i + 1))
            };
            let (full, raw16) = page.full_samples();
            let raw = match &raw16 {
                Some(samples) => hp3770_export::Linear16::from_linear16(
                    full.width,
                    full.height,
                    full.dpi,
                    samples,
                    page.raw.tone.white.map(|w| w * 256.0),
                ),
                None => hp3770_export::Linear16::from_linear8(&full, page.raw.tone.white)
                    .map_err(|e| e.to_string())?,
            };
            match format {
                RawFormat::Tiff16 => hp3770_export::save_tiff16(&target, &raw),
                RawFormat::Dng => hp3770_export::save_dng(&target, &raw),
            }
            .map_err(|e| e.to_string())?;
            written.push(target);
        }
        Ok(match written.as_slice() {
            [one] => one.display().to_string(),
            _ => format!(
                "{n} file RAW in {}",
                path.parent().unwrap_or(path).display()
            ),
        })
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let touches_pages = !self.pages.is_empty() && Self::changes_pages(&message);
        let task = self.handle(message);
        if touches_pages {
            self.unsaved = true;
            self.pdf_rev += 1;
        }
        if self.pages.is_empty() {
            self.unsaved = false;
        }
        let task = Task::batch([task, self.schedule_estimate()]);
        let now = self.prefs();
        if now != self.saved {
            if let Err(e) = now.save() {
                self.note = format!("Impostazioni non salvate: {e}");
            }
            self.saved = now;
        }
        Task::batch([task, self.schedule()])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        // Leaving the page edited keeps what was done to it.
        if self.editor.is_some()
            && matches!(
                message,
                Message::Kind(_)
                    | Message::Tab(Tab::Preview)
                    | Message::PageMove(..)
                    | Message::PageDelete(_)
                    | Message::PageDuplicate(_)
                    | Message::PageRedo(_)
                    | Message::NewDocument
                    | Message::Scan
                    | Message::Preview
            )
        {
            self.close_editor(true);
        }
        match message {
            Message::Edit(m) => self.on_edit(m),
            Message::Worker(event) => self.on_worker(event),
            Message::Rendered(done) => {
                self.rendering = false;
                let Ok(Rendered {
                    target,
                    page: rendered_page,
                    epoch,
                    shown,
                    picture,
                    thumb,
                }) = Arc::try_unwrap(done)
                else {
                    return Task::none();
                };
                match target {
                    // Pages moved, went or were replaced meanwhile: the
                    // picture may be of another page. Draw again.
                    Target::Page if epoch != self.epoch => self.dirty_page = true,
                    Target::Index(_) if epoch != self.epoch => self.restale(),
                    Target::Preview => {
                        if let Some((_, old_shown, old)) = &mut self.preview {
                            *old = picture.replacing(old);
                            *old_shown = shown;
                        }
                    }
                    Target::Page if rendered_page != self.shown_key() => {
                        // Another page is shown now: draw that one.
                        self.dirty_page = true;
                    }
                    Target::Page | Target::Index(_) => {
                        let page = match target {
                            Target::Index(i) => self.pages.get_mut(i),
                            _ => self.shown_mut(),
                        };
                        if let Some(page) = page {
                            page.picture = picture.replacing(&page.picture);
                            if let Some(thumb) = thumb {
                                page.thumb = thumb;
                            }
                            page.shown = shown;
                        }
                    }
                }
            }
            Message::SystemTheme(mode) => self.system_dark = mode == iced::theme::Mode::Dark,
            Message::Theme(choice) => self.theme = choice,
            Message::Kind(kind) => {
                // Not during a scan: its pages go where the mode says.
                if self.kind != kind && self.busy.is_none() {
                    // The limit typed belongs to the other way of working.
                    self.limit_draft = None;
                    let dropped = self.pages.len().saturating_sub(1);
                    if kind == Kind::Photo
                        && dropped > 0
                        && self.unsaved
                        && self.ask(
                            Message::Kind(kind),
                            "Passare a Foto?".into(),
                            format!(
                                "In Foto resta solo l'ultima scansione. {}",
                                if dropped == 1 {
                                    "L'altra pagina del documento non è salvata e andrà persa."
                                        .to_string()
                                } else {
                                    format!(
                                        "Le altre {dropped} pagine del documento non sono \
                                         salvate e andranno perse."
                                    )
                                },
                            ),
                            "Passa a Foto",
                        )
                    {
                        return Task::none();
                    }
                    self.kind = kind;
                    self.redone = None;
                    if kind == Kind::Photo {
                        // A photo at a time: keep the last scan only.
                        let last = self.pages.pop();
                        self.pages.clear();
                        self.pages.extend(last);
                        self.current = (!self.pages.is_empty()).then_some(0);
                    }
                    self.pages_changed();
                    // Ctrl+Z restores the settings of the mode it was in.
                    self.undo.clear();
                    self.redo_stack.clear();
                    self.last_edit = None;
                    // The pages that follow the shared look take this
                    // mode's one.
                    self.shared_changed();
                }
            }
            Message::Tab(tab) => self.tab = tab,
            Message::Pane(pane) => {
                self.pane = pane;
                // The page on its sheet, where the output is chosen.
                if self.print_preview() {
                    self.tab = Tab::Result;
                }
            }
            Message::StartMode(mode) => self.start_mode = mode,
            Message::WiaQuiet(on) => {
                self.note = "Conferma la richiesta di Windows (amministratore)…".into();
                return Task::perform(async move { wia::set_quiet(on) }, Message::WiaDone);
            }
            Message::WiaDone(result) => {
                self.wia_quiet = wia::quiet();
                self.note = match result {
                    Ok(()) if self.wia_quiet == Some(true) => {
                        "Windows non apre più la sua app ai tasti dello scanner".into()
                    }
                    Ok(()) => "Windows torna ad aprire la sua app ai tasti dello scanner".into(),
                    Err(e) => format!("Non cambiato: {e}"),
                };
            }
            Message::GrantAccess => {
                self.note = "Conferma con la password per dare lo scanner a questo utente…".into();
                return Task::perform(async { access::grant() }, Message::GrantDone);
            }
            Message::GrantDone(result) => {
                self.note = match result {
                    Ok(()) => "Accesso allo scanner concesso".into(),
                    Err(e) => format!("Accesso non concesso: {e}"),
                };
                if let Some(w) = &self.worker {
                    w.send(Command::Status);
                }
            }
            Message::ButtonAction(i, action) => self.set_button_action(i, action),
            Message::ChooseScanner(device) => {
                if let Some(w) = &self.worker {
                    w.send(Command::Choose(device));
                }
            }
            Message::ChooseFolder => {
                let start = self.folder();
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_directory(start)
                            .pick_folder()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::Folder,
                );
            }
            Message::Folder(Some(folder)) => self.custom_folder = Some(folder),
            Message::Folder(None) => {}
            Message::DefaultFolder => self.custom_folder = None,
            Message::DocumentName(name) => self.document_name = name,
            Message::PhotoName(name) => self.photo_name = name,
            Message::FactoryReset => {
                let d = prefs::Prefs::default();
                self.document = Settings::document();
                self.photo = Settings::photo();
                self.start_mode = StartMode::Last;
                self.custom_folder = None;
                self.document_name = d.document_name;
                self.photo_name = d.photo_name;
                self.button_actions.clear();
                self.theme = ThemeChoice::System;
                self.lamp_off = d.lamp_off_minutes;
                self.send_lamp_timer();
                self.undo.clear();
                self.redo_stack.clear();
                self.shared_changed();
                self.note = "Impostazioni di fabbrica ripristinate".into();
            }
            Message::View(view) => *self.current_view() = view,
            Message::Fit => *self.current_view() = View::default(),
            Message::ActualSize => {
                let v = self.current_view();
                *v = v.actual_size();
            }
            Message::Select(selection) => self.selection = selection,
            Message::Dpi(dpi) => self.settings_mut().dpi = dpi,
            Message::Depth(d) => {
                let kind = self.kind;
                let s = self.settings_mut();
                s.depth = d.0;
                if !outputs(kind, s.colour, s.depth).contains(&s.output) {
                    s.output = outputs(kind, s.colour, s.depth)[0];
                }
            }
            Message::Colour(colour) => {
                let kind = self.kind;
                self.change(|s| {
                    s.colour = colour;
                    if !outputs(kind, colour, s.depth).contains(&s.output) {
                        s.output = outputs(kind, colour, s.depth)[0];
                    }
                });
            }
            Message::Output(output) => self.settings_mut().output = output,
            Message::PdfSize(size) => self.settings_mut().pdf_size = size,
            // The page shown when it has its own layout, else the document.
            Message::PdfFit(fit) => match self.own_layout_mut() {
                Some(l) => l.fit = fit,
                None => self.settings_mut().pdf_fit = fit,
            },
            Message::PdfMargin(Margin(mm)) => match self.own_layout_mut() {
                Some(l) => l.margin = mm,
                None => self.settings_mut().pdf_margin = mm,
            },
            Message::PdfSheet(sheet) => match self.own_layout_mut() {
                Some(l) => l.sheet = sheet,
                None => self.settings_mut().pdf_sheet = sheet,
            },
            Message::PdfQuality(q) => {
                self.settings_mut().pdf_quality = q.clamp(PDF_QUALITY_MIN, 100);
            }
            Message::PdfLimitText(text) => {
                if let Some(kb) = parse_megabytes(&text) {
                    self.settings_mut().pdf_limit_kb = kb;
                }
                self.limit_draft = Some(text);
            }
            Message::Estimated(done) => {
                self.estimating = false;
                self.pdf_estimate = Some(done);
            }
            Message::PdfOwnLayout(on) => {
                let document = self.settings().page_layout();
                if let Some(page) = self.current.and_then(|i| self.pages.get_mut(i)) {
                    page.layout = on.then_some(page.layout.unwrap_or(document));
                }
            }
            Message::PdfApplyToAll => {
                let own = self
                    .current
                    .and_then(|i| self.pages.get(i))
                    .and_then(|p| p.layout);
                if let Some(l) = own {
                    let s = self.settings_mut();
                    s.pdf_sheet = l.sheet;
                    s.pdf_fit = l.fit;
                    s.pdf_margin = l.margin;
                    for page in &mut self.pages {
                        page.layout = None;
                    }
                    self.note = "Impaginazione applicata a tutte le pagine".into();
                }
            }
            Message::PageFocus(focus) => {
                if let Some(page) = self.current.and_then(|i| self.pages.get_mut(i)) {
                    page.focus = focus.map(|f| f.clamp(0.0, 1.0));
                }
            }
            Message::AutoSave(on) => self.settings_mut().auto_save = on,
            Message::AutoCrop(on) => self.settings_mut().auto_crop = on,
            Message::Detect => {
                if let Some((raw, _, _)) = &self.preview {
                    self.found = detect::objects(&raw.full);
                    self.note = Self::found_note(self.found.len());
                }
            }
            Message::DropFound(i) => {
                if i < self.found.len() {
                    self.found.remove(i);
                }
            }
            Message::Toggle(group) => {
                if let Some(i) = self.open.iter().position(|g| *g == group) {
                    self.open.remove(i);
                } else {
                    self.open.push(group);
                }
            }
            Message::Adjust(field, v) => {
                self.draft = None;
                self.change_field(field, |s| field.set(s, v));
            }
            Message::PresetName(name) => self.preset_name = name,
            Message::PresetSave => {
                let name = self.preset_name.trim().to_string();
                if name.is_empty() {
                    self.note = "Dai un nome al preset".into();
                    return Task::none();
                }
                // "Applica" looks among the factory presets first: one of
                // the user's with the same name could never be applied.
                if [Kind::Document, Kind::Photo]
                    .into_iter()
                    .flat_map(factory_presets)
                    .any(|(n, _)| n == name)
                {
                    self.note = format!("«{name}» è un preset di fabbrica: scegli un altro nome");
                    return Task::none();
                }
                let setup = self.active().to_stored();
                let existing = self.presets.iter().position(|p| p.name == name);
                match existing {
                    Some(i) => self.presets[i].setup = setup,
                    None if self.presets.len() < prefs::MAX_PRESETS => {
                        self.presets.push(prefs::Preset {
                            name: name.clone(),
                            setup,
                        })
                    }
                    None => {
                        self.note = format!(
                            "Al massimo {} preset: eliminane uno o salva sopra a uno esistente",
                            prefs::MAX_PRESETS
                        );
                        return Task::none();
                    }
                }
                self.note = match prefs::save_presets(&self.presets) {
                    Ok(()) => format!("Preset «{name}» salvato"),
                    Err(e) => format!("Preset non salvato: {e}"),
                };
                self.preset_name.clear();
            }
            Message::PresetApply(name) => {
                let factory = factory_presets(self.kind)
                    .into_iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, s)| s);
                let user = self
                    .presets
                    .iter()
                    .find(|p| p.name == name)
                    .map(|p| Settings::from_stored(self.kind, &p.setup));
                if let Some(mut new) = factory.or(user) {
                    // The output folder behaviour stays the user's.
                    new.auto_save = self.settings().auto_save;
                    new.auto_crop = self.settings().auto_crop;
                    self.change(|s| *s = new);
                    self.note = format!("Preset «{name}» applicato (Ctrl+Z per tornare indietro)");
                }
            }
            Message::PresetDelete(name) => {
                self.presets.retain(|p| p.name != name);
                self.note = match prefs::save_presets(&self.presets) {
                    Ok(()) => format!("Preset «{name}» eliminato"),
                    Err(e) => format!("Preset non salvato: {e}"),
                };
            }
            Message::OwnLook(on) => {
                let shared = *self.settings();
                if self.kind == Kind::Document && self.editing_page() {
                    if let Some(page) = self.shown_mut() {
                        page.own = on.then(|| page.own.unwrap_or(Look::of(&shared)));
                        self.dirty_page = true;
                    }
                    self.note = if on {
                        "Questa pagina ha ora regolazioni sue".into()
                    } else {
                        "Questa pagina segue di nuovo le regolazioni comuni".into()
                    };
                }
            }
            Message::LookApplyToAll => {
                let shared = *self.settings();
                let look = self
                    .shown()
                    .filter(|_| self.kind == Kind::Document && self.editing_page())
                    .map(|p| Look::of(&p.effective(&shared)));
                if let Some(look) = look {
                    look.put(self.settings_mut());
                    for page in &mut self.pages {
                        page.own = Some(look);
                    }
                    if let Some((_, page)) = &mut self.redone {
                        page.own = Some(look);
                    }
                    let shown = self.current;
                    self.stale = (0..self.pages.len())
                        .filter(|&i| Some(i) != shown)
                        .collect();
                    self.dirty_preview = true;
                    self.dirty_page = true;
                    self.note = "Regolazioni applicate a tutte le pagine".into();
                }
            }
            Message::CurveChannel(c) => self.curve_channel = c,
            Message::CurveSet(points) => {
                let c = self.curve_channel.index();
                // A drag is one step of Ctrl+Z.
                let now = std::time::Instant::now();
                if self
                    .curve_drag
                    .is_none_or(|at| now - at > std::time::Duration::from_secs(1))
                {
                    self.remember(None);
                }
                self.curve_drag = Some(now);
                let curve = scan_imaging::adjust::Curve::new(&points);
                self.apply(|s| s.adjust.curve[c] = curve);
            }
            Message::Descreen(screen) => self.change(|s| s.adjust.descreen = screen.0),
            Message::Adaptive(on) => self.change(|s| s.adjust.adaptive = on),
            Message::Undo if self.editor().is_some() => self.on_edit(EditMsg::Undo),
            Message::Redo if self.editor().is_some() => self.on_edit(EditMsg::Redo),
            Message::Undo => {
                if let Some((target, settings)) = self.undo.pop() {
                    match self.restore(target, settings) {
                        Some(current) => {
                            self.redo_stack.push(current);
                            self.note = "Annullato".into();
                        }
                        None => self.note = "Niente da annullare qui".into(),
                    }
                }
            }
            Message::Redo => {
                if let Some((target, settings)) = self.redo_stack.pop() {
                    if let Some(current) = self.restore(target, settings) {
                        self.undo.push(current);
                        self.note = "Ripristinato".into();
                    }
                }
            }
            Message::Key(event) => return self.on_key(event),
            Message::Draft(field, text) => self.draft = Some((field, text)),
            Message::Commit(field) => {
                if let Some((f, text)) = self.draft.take() {
                    if f == field {
                        match text.trim().replace(',', ".").parse::<f32>() {
                            Ok(v) => self.change(|s| field.set(s, v)),
                            Err(_) => self.note = format!("«{text}» non è un numero"),
                        }
                    }
                }
            }
            Message::AutoLevels => {
                if let Some((proxy, tone)) = self.active_proxy() {
                    let settings = self.active();
                    let base = Adjustments {
                        black: 0.0,
                        white: 255.0,
                        sharpen: 0.0,
                        ..settings.adjust
                    };
                    let shown = render(&proxy, &settings, &base, &tone);
                    let (black, white) = adjust::auto_levels(&shown);
                    self.change(|s| {
                        s.adjust.black = black;
                        s.adjust.white = white;
                    });
                }
            }
            Message::AutoThreshold => {
                if let Some((proxy, tone)) = self.active_proxy() {
                    let settings = Settings {
                        colour: Colour::Grey,
                        ..self.active()
                    };
                    let grey = render(&proxy, &settings, &settings.adjust, &tone);
                    let t = adjust::auto_threshold(&grey.data);
                    self.change(|s| s.threshold = t);
                }
            }
            Message::Eyedropper => self.eyedropper = !self.eyedropper,
            Message::Picked(point) => {
                self.eyedropper = false;
                let picked = if self.editing_page() {
                    // The viewer shows the page with its geometry edits.
                    let frame = self.cropping();
                    self.shown()
                        .map(|p| p.raw.sample(p.proxy_point(point, frame)))
                } else {
                    self.preview.as_ref().map(|(raw, ..)| {
                        let scale = raw.proxy.width as f32 / raw.full.width.max(1) as f32;
                        raw.sample(Point::new(point.x * scale, point.y * scale))
                    })
                };
                if let Some(sample) = picked {
                    self.change(|s| s.adjust = s.adjust.neutral_at(sample));
                    self.note = format!(
                        "Bilanciamento dal punto scelto (R {} G {} B {})",
                        sample[0], sample[1], sample[2]
                    );
                }
            }
            Message::ResetGroup(group) => self.change(|s| {
                let d = Adjustments::default();
                let j = &mut s.adjust;
                match group {
                    Group::Tone => {
                        j.temperature = d.temperature;
                        j.tint = d.tint;
                        j.exposure = d.exposure;
                        j.highlights = d.highlights;
                        j.shadows = d.shadows;
                        j.whites = d.whites;
                        j.blacks = d.blacks;
                        j.black = d.black;
                        j.white = d.white;
                        j.gamma = d.gamma;
                        j.brightness = d.brightness;
                        j.contrast = d.contrast;
                    }
                    Group::Colour => {
                        j.saturation = d.saturation;
                        j.vibrance = d.vibrance;
                    }
                    Group::Curve => j.curve = d.curve,
                    Group::Detail => {
                        j.sharpen = d.sharpen;
                        j.noise = d.noise;
                        j.descreen = d.descreen;
                    }
                    Group::Lineart => {
                        s.threshold = convert::DEFAULT_THRESHOLD;
                        j.adaptive = d.adaptive;
                    }
                }
            }),
            Message::Before(on) => {
                self.before = on;
                self.redraw_preview();
                self.dirty_page = true;
            }
            Message::RefreshStatus => {
                if let Some(w) = &self.worker {
                    w.send(Command::Status);
                }
            }
            Message::Preview => self.start(Purpose::Preview),
            Message::Scan => {
                self.redone = None;
                self.start(Purpose::Page);
            }
            Message::Cancel => {
                if let Some(w) = &self.worker {
                    w.cancel();
                    self.note = "Annullamento…".into();
                }
            }
            Message::Save => {
                let output = self.settings().output;
                let ext = output.extension();
                let (prefix, filter) = match (self.kind, output) {
                    (Kind::Document, Output::Image(_)) if self.pages.len() > 1 => {
                        (self.name(), "Un file per pagina")
                    }
                    (Kind::Document, _) => (self.name(), "Documento"),
                    (Kind::Photo, _) => (self.name(), "Immagine"),
                };
                let folder = self.folder();
                let _ = std::fs::create_dir_all(&folder);
                let file = Self::free_name(&folder, prefix, ext);
                // Tests answer the dialog themselves.
                if cfg!(test) {
                    return Task::done(Message::SaveTo(testing::save_answer(&folder, &file)));
                }
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_directory(folder)
                            .add_filter(filter, &[ext])
                            .set_file_name(file)
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::SaveTo,
                );
            }
            Message::SaveTo(Some(path)) => {
                let then = self.after_save.take();
                match self.write(&path) {
                    Ok(what) => {
                        self.note = format!("Salvato: {what}");
                        self.unsaved = false;
                        // "Salva…" in the question: now what was asked.
                        if let Some(then) = then {
                            return self.run_confirmed(then);
                        }
                    }
                    Err(e) => self.note = format!("Salvataggio non riuscito: {e}"),
                }
            }
            Message::SaveTo(None) => self.after_save = None,
            Message::Confirm(yes) => {
                if let Some(confirm) = self.confirm.take() {
                    if yes {
                        return self.run_confirmed(*confirm.then);
                    }
                    self.print_next = false;
                    self.note = "Nulla è cambiato".into();
                }
            }
            Message::ConfirmSave => {
                if let Some(confirm) = self.confirm.take() {
                    self.after_save = Some(*confirm.then);
                    return self.handle(Message::Save);
                }
            }
            Message::CloseRequested(id) => {
                let n = self.pages.len();
                let lost = (n > 0 && self.unsaved).then(|| self.lost_sentence(n));
                let asked = if self.busy.is_some() {
                    let body = match &lost {
                        Some(lost) => format!("La scansione in corso si interrompe. {lost}"),
                        None => "La scansione in corso si interrompe e va persa.".into(),
                    };
                    self.ask(
                        Message::CloseRequested(id),
                        "Chiudere durante la scansione?".into(),
                        body,
                        "Chiudi",
                    )
                } else if let Some(lost) = lost {
                    self.ask(
                        Message::CloseRequested(id),
                        "Chiudere senza salvare?".into(),
                        lost,
                        "Chiudi senza salvare",
                    )
                } else {
                    false
                };
                if !asked {
                    // The lamp off first, if there is one: nobody is going
                    // to scan.
                    let Some(worker) = self.worker.clone().filter(|_| self.caps.lamp.is_some())
                    else {
                        return iced::window::close(id);
                    };
                    self.note = "Spegnimento della lampada…".into();
                    return Task::perform(
                        async move {
                            let (done, wait) = std::sync::mpsc::channel();
                            worker.send(Command::LampOff(Some(done)));
                            // Not longer: during a scan the thread answers
                            // only at its end.
                            let _ = wait.recv_timeout(std::time::Duration::from_secs(2));
                        },
                        move |()| Message::Close(id),
                    );
                }
            }
            Message::Close(id) => return iced::window::close(id),
            Message::LampAfter(LampAfter(minutes)) => {
                self.lamp_off = minutes;
                self.send_lamp_timer();
            }
            Message::PageShow(i) if self.editor.is_some() => self.open_editor(i),
            Message::PageShow(i) => {
                self.current = Some(i);
                self.redone = None;
                self.result_view = View::default();
                self.tab = Tab::Result;
            }
            Message::PageRotate(i, clockwise) if self.editor().is_some_and(|e| e.index == i) => {
                self.on_edit(EditMsg::Rotate(clockwise));
            }
            Message::PageRotate(i, clockwise) => {
                let shared = *self.settings();
                if let Some(page) = self.pages.get_mut(i) {
                    page.rotate(clockwise, &shared);
                    self.current = Some(i);
                    self.result_view = View::default();
                }
            }
            Message::PageMove(i, forward) => {
                let j = if forward { i + 1 } else { i.wrapping_sub(1) };
                // Not while a page is scanned again: "Rifai" knows it by
                // its place.
                if self.busy.is_none() && i < self.pages.len() && j < self.pages.len() {
                    self.pages.swap(i, j);
                    self.current = Some(j);
                    // A new scan waiting for a choice follows its page.
                    if let Some((index, _)) = &mut self.redone {
                        if *index == i {
                            *index = j;
                        } else if *index == j {
                            *index = i;
                        }
                    }
                    self.pages_changed();
                }
            }
            Message::PageDuplicate(i) => {
                // Not while a new scan waits for a choice: "Rifai" knows
                // its page by its place.
                if self.busy.is_none() && self.redone.is_none() && i < self.pages.len() {
                    let copy = self.pages[i].clone();
                    self.pages.insert(i + 1, copy);
                    self.current = Some(i + 1);
                    self.tab = Tab::Result;
                    self.result_view = View::default();
                    self.pages_changed();
                    self.note = format!(
                        "{} duplicata: la copia è la {}, da ritagliare e regolare a parte",
                        self.page_title(i),
                        self.page_name(i + 1)
                    );
                }
            }
            Message::PagePress(i) => {
                if i < self.pages.len() {
                    if self.busy.is_none() && self.redone.is_none() {
                        self.drag = Some((i, None));
                    }
                    return self.handle(Message::PageShow(i));
                }
            }
            Message::PageHover(j) => {
                if let Some((from, over)) = &mut self.drag {
                    *over = (j != *from).then_some(j);
                }
            }
            Message::PageDrop => {
                if let Some((from, Some(to))) = self.drag.take() {
                    let n = self.pages.len();
                    if self.busy.is_none() && self.redone.is_none() && from < n && to < n {
                        self.close_editor(true);
                        let page = self.pages.remove(from);
                        self.pages.insert(to, page);
                        self.current = Some(to);
                        self.pages_changed();
                        self.unsaved = true;
                        let name = item_name(self.kind, to, n);
                        self.note = format!("Spostata: ora è la {name}");
                    }
                }
            }
            Message::PageDelete(i) => {
                if self.busy.is_none() && i < self.pages.len() {
                    let title = self.page_title(i);
                    if self.unsaved
                        && self.ask(
                            Message::PageDelete(i),
                            format!("Eliminare la {}?", self.page_name(i)),
                            "Non è salvata e non si può recuperare.".into(),
                            format!("Elimina la {}", self.page_name(i)),
                        )
                    {
                        return Task::none();
                    }
                    self.pages.remove(i);
                    self.redone = None;
                    self.current = (!self.pages.is_empty()).then(|| i.min(self.pages.len() - 1));
                    self.pages_changed();
                    self.note = format!("{title} eliminata");
                }
            }
            Message::PageRedo(i) => {
                if self.busy.is_none() && i < self.pages.len() {
                    self.redo = Some(i);
                    self.start(Purpose::Page);
                }
            }
            Message::AcceptRedo => {
                if let Some((index, page)) = self.redone.take() {
                    if index < self.pages.len() {
                        self.pages[index] = page;
                        self.current = Some(index);
                        self.pages_changed();
                        self.note = format!("{} sostituita", self.page_title(index));
                    }
                }
            }
            Message::RejectRedo => {
                if let Some((index, _)) = self.redone.take() {
                    self.current = Some(index);
                    self.forget_page_undo();
                    self.note = format!("{} lasciata com'era", self.page_title(index));
                }
            }
            Message::NewDocument => {
                let n = self.pages.len();
                if n > 0
                    && self.unsaved
                    && self.ask(
                        Message::NewDocument,
                        "Iniziare un nuovo documento?".into(),
                        self.lost_sentence(n),
                        "Scarta e ricomincia",
                    )
                {
                    return Task::none();
                }
                self.pages.clear();
                self.current = None;
                self.redone = None;
                self.pages_changed();
                self.note = "Nuovo documento".into();
            }
        }
        Task::none()
    }

    /// Keyboard shortcuts (only when no text field has the key).
    fn on_key(&mut self, event: iced::keyboard::Event) -> Task<Message> {
        use iced::keyboard::{key::Named, Event as K, Key};
        let K::KeyPressed { key, modifiers, .. } = event else {
            return Task::none();
        };
        // A question on the screen: Esc leaves everything as it is, Enter
        // does not go on (it would throw scans away by a slip).
        if self.confirm.is_some() {
            return match key.as_ref() {
                Key::Named(Named::Escape) => self.handle(Message::Confirm(false)),
                _ => Task::none(),
            };
        }
        let editing = self.editor().is_some();
        let message = match (key.as_ref(), modifiers.command(), modifiers.shift()) {
            (Key::Named(Named::Enter), false, _) if editing => Some(EditMsg::Apply.into()),
            (Key::Named(Named::Escape), _, _) if editing && !self.eyedropper => {
                Some(EditMsg::Discard.into())
            }
            (Key::Character("r"), false, false) if editing => Some(EditMsg::Rotate(true).into()),
            (Key::Character("r" | "R"), false, true) if editing => {
                Some(EditMsg::Rotate(false).into())
            }
            (Key::Character("e"), false, _) if !editing && self.tab == Tab::Result => {
                self.current.map(|i| EditMsg::Open(i).into())
            }
            (Key::Character("z"), true, false) => Some(Message::Undo),
            (Key::Character("z"), true, true) | (Key::Character("y"), true, _) => {
                Some(Message::Redo)
            }
            (Key::Character("p"), true, _) => Some(Message::Preview),
            (Key::Named(Named::Enter), true, _) => Some(Message::Scan),
            (Key::Character("s"), true, _) => Some(Message::Save),
            (Key::Character("d"), true, _) if !editing && self.tab == Tab::Result => {
                self.current.map(Message::PageDuplicate)
            }
            (Key::Character("f"), false, _) => Some(Message::Fit),
            (Key::Character("1"), false, _) => Some(Message::ActualSize),
            (Key::Character("\\"), false, _) => Some(Message::Before(!self.before)),
            (Key::Character("i"), false, _) => Some(Message::Eyedropper),
            (Key::Named(Named::Escape), _, _) => {
                if self.eyedropper {
                    Some(Message::Eyedropper)
                } else {
                    Some(Message::Select(None))
                }
            }
            _ => None,
        };
        match message {
            // Actions that need the scanner or a page check it themselves.
            Some(Message::Preview | Message::Scan) if !self.idle() => Task::none(),
            Some(m) => self.handle(m),
            None => Task::none(),
        }
    }

    /// The small copy of the samples the panel acts on: the page with its
    /// geometry edits, or the preview.
    fn active_proxy(&self) -> Option<(Arc<Image>, Tone)> {
        if self.editing_page() {
            self.shown().map(|p| {
                let (t, _) = p.preview_geometry(&p.transform, false);
                let proxy = if t.is_identity() {
                    p.raw.proxy.clone()
                } else {
                    Arc::new(edited(&p.raw.proxy, &t, &p.raw.tone))
                };
                (proxy, p.raw.tone)
            })
        } else {
            self.preview
                .as_ref()
                .map(|(raw, ..)| (raw.proxy.clone(), raw.tone))
        }
    }

    fn current_view(&mut self) -> &mut View {
        match self.tab {
            Tab::Preview => &mut self.preview_view,
            Tab::Result => &mut self.result_view,
        }
    }

    fn on_worker(&mut self, event: Event) {
        match event {
            Event::Ready(handle) => {
                handle.send(Command::Status);
                self.worker = Some(handle);
                self.send_lamp_timer();
            }
            Event::Opened(device, caps, devices) => {
                self.caps = *caps;
                self.device = Some(device);
                self.devices = devices;
                self.send_lamp_timer();
            }
            Event::Status(status) => {
                self.access_denied = status.is_err() && access::denied();
                self.status = Some(status);
            }
            Event::Progress(stage) => {
                if let Some((_, s)) = &mut self.busy {
                    *s = Some(stage);
                }
            }
            Event::Done(purpose, result) => {
                self.busy = None;
                match result {
                    Ok(done) => {
                        self.on_scan(purpose, done.image.clone(), done.samples16.clone());
                    }
                    Err(e) => {
                        self.redo = None;
                        self.redo_auto = false;
                        self.print_next = false;
                        self.job = None;
                        self.jobs.clear();
                        if e == worker::CANCELLED {
                            // Stopped with "Annulla": nothing went wrong.
                            self.note = match purpose {
                                Purpose::Preview => "Anteprima annullata".into(),
                                Purpose::Page => "Scansione annullata".into(),
                            };
                        } else {
                            self.note = format!("Errore: {e}");
                            if purpose == Purpose::Page {
                                notify("Scansione non riuscita", &e, true);
                            }
                        }
                    }
                }
                if let Some(w) = &self.worker {
                    w.send(Command::Status);
                }
            }
            Event::Button(button) => self.on_button(button),
        }
    }

    fn found_note(n: usize) -> String {
        match n {
            0 => "Anteprima pronta: trascina per scegliere l'area, regola a destra".into(),
            1 => {
                "Anteprima pronta: un originale riconosciuto, sarà ritagliato e raddrizzato".into()
            }
            n => format!(
                "Anteprima pronta: {n} originali riconosciuti, ognuno diventa un'immagine \
                 raddrizzata"
            ),
        }
    }

    fn on_scan(&mut self, purpose: Purpose, image: Image, samples16: Option<Vec<u16>>) {
        match purpose {
            Purpose::Preview => {
                self.close_editor(true);
                // The preview is worked in pixels of PREVIEW_DPI.
                let raw = Raw::new(at_dpi(image, PREVIEW_DPI), self.caps.tone);
                let settings = *self.settings();
                let shown = render(&raw.proxy, &settings, &settings.adjust, &raw.tone);
                let picture = Picture::from_rgb(shown.width, shown.height, &shown.to_rgb());
                self.found = detect::objects(&raw.full);
                self.preview = Some((raw, shown, picture));
                self.preview_view = View::default();
                self.tab = Tab::Preview;
                self.note = if self.settings().auto_crop {
                    Self::found_note(self.found.len())
                } else {
                    Self::found_note(0)
                };
            }
            Purpose::Page => {
                let job = self.job.take();
                for (image, origin, samples) in Self::cut(image, samples16, job.as_ref()) {
                    self.add_page(image, origin, samples);
                }
                self.result_view = View::default();
                self.tab = Tab::Result;
                if self.jobs.is_empty() {
                    self.finish_batch();
                } else {
                    self.start(Purpose::Page);
                }
            }
        }
    }

    /// A page (or photo) scanned: added, or put in place of the one being
    /// redone.
    fn add_page(&mut self, image: Image, origin: Option<Found>, samples16: Option<Vec<u16>>) {
        self.close_editor(true);
        self.unsaved = true;
        self.pdf_rev += 1;
        let (w, h, dpi) = (image.width, image.height, image.dpi);
        let mut page = Page::new(Raw::new(image, self.caps.tone), *self.settings());
        // "Solo questa pagina" from the start: every page keeps the look it
        // was scanned with, and changing one does not change the others.
        if self.kind == Kind::Document {
            page.own = Some(Look::of(self.settings()));
        }
        page.origin = origin;
        page.raw16 = samples16.filter(|s| s.len() == w * h * 3).map(Arc::new);
        let first = self.batch.0 == 0;
        self.batch.0 += 1;
        let of = if self.batch.1 > 1 {
            format!(" ({} di {})", self.batch.0, self.batch.1)
        } else {
            String::new()
        };
        match (self.redo, self.kind) {
            (Some(index), _) if self.redo_auto => {
                self.redo = None;
                self.redo_auto = false;
                if index < self.pages.len() {
                    self.pages[index] = page;
                    self.current = Some(index);
                    self.pages_changed();
                }
                self.note = format!("{} rifatta", self.page_title(index));
                notify(&self.note, "", false);
            }
            (Some(index), _) => {
                self.redo = None;
                self.redone = Some((index, page));
                // The panel and the strip show the page done again.
                self.current = Some(index);
                self.note = format!(
                    "Nuova scansione della {}: tienila o lascia la vecchia",
                    self.page_name(index)
                );
            }
            (None, Kind::Document) => {
                self.pages.push(page);
                self.current = Some(self.pages.len() - 1);
                self.note = format!(
                    "Pagina {} acquisita{of} · {w} × {h} pixel a {dpi} dpi",
                    self.pages.len()
                );
                notify(&format!("Pagina {} acquisita", self.pages.len()), "", false);
            }
            (None, Kind::Photo) => {
                if first {
                    self.pages.clear();
                    self.pages_changed();
                }
                self.pages.push(page);
                self.current = Some(if first { 0 } else { self.current.unwrap_or(0) });
                self.note = format!("Foto acquisita{of} · {w} × {h} pixel a {dpi} dpi");
            }
        }
    }

    /// After the last pass: in Foto mode, print (photocopy) and save if
    /// asked.
    fn finish_batch(&mut self) {
        if self.kind != Kind::Photo || self.redone.is_some() {
            return;
        }
        let n = self.pages.len();
        if std::mem::take(&mut self.print_next) {
            self.print_shown();
        }
        if self.photo.auto_save && n > 0 {
            let folder = self.folder();
            let _ = std::fs::create_dir_all(&folder);
            let path = folder.join(Self::free_name(
                &folder,
                self.name(),
                self.photo.output.extension(),
            ));
            let written = self.write(&path);
            if written.is_ok() {
                self.unsaved = false;
            }
            self.note = match written {
                Ok(_) if n > 1 => {
                    notify(
                        &format!("{n} foto salvate"),
                        &folder.display().to_string(),
                        false,
                    );
                    format!("{n} foto salvate in {}", folder.display())
                }
                Ok(_) => {
                    notify("Foto salvata", &path.display().to_string(), false);
                    format!("Foto salvata in {}", path.display())
                }
                Err(e) => format!("Salvataggio non riuscito: {e}"),
            };
        } else if n > 1 {
            self.note = format!("{n} foto acquisite: scegli a sinistra, salva con «Salva»");
            notify(&format!("{n} foto acquisite"), "", false);
        }
    }

    /// The front-panel buttons, by way of working. Ignored while a scan
    /// runs. Documento: Scan adds a page, Copy redoes the last one,
    /// Instant Share saves the document. Foto: Scan scans (and saves if
    /// automatic saving is on), Instant Share opens the folder.
    fn on_button(&mut self, index: usize) {
        let Some(name) = self.caps.buttons.get(index).map(|b| b.label.clone()) else {
            return;
        };
        if self.busy.is_some() {
            self.note = format!("Tasto {name} ignorato: scansione in corso");
            return;
        }
        if self.confirm.is_some() {
            self.note = format!("Tasto {name} ignorato: rispondi prima alla domanda");
            notify("Rispondi alla domanda nell'app", "", false);
            return;
        }
        let action = self.button_action(self.kind, index);
        match action {
            ButtonAction::AddPage | ButtonAction::ScanPhoto => {
                self.redone = None;
                self.start(Purpose::Page);
                self.notify_question();
            }
            ButtonAction::RedoLast => match self.pages.len() {
                0 => {
                    self.note = "Nessuna pagina da rifare: Scan acquisisce la prima".into();
                    notify(
                        "Nessuna pagina da rifare",
                        "Premi Scan per la prima pagina",
                        false,
                    );
                }
                n => {
                    self.redone = None;
                    self.redo = Some(n - 1);
                    self.redo_auto = true;
                    self.start(Purpose::Page);
                }
            },
            ButtonAction::Photocopy => {
                self.redone = None;
                self.print_next = true;
                self.start(Purpose::Page);
                if self.confirm.is_some() {
                    self.notify_question();
                } else {
                    notify("Fotocopia", "Scansione in corso, poi la stampa", false);
                }
            }
            ButtonAction::SaveDocument => {
                if self.pages.is_empty() {
                    self.note = "Documento vuoto: premi Scan per acquisire una pagina".into();
                    notify(
                        "Documento vuoto",
                        "Premi Scan per acquisire una pagina",
                        false,
                    );
                    return;
                }
                let folder = self.folder();
                let _ = std::fs::create_dir_all(&folder);
                let path = folder.join(Self::free_name(
                    &folder,
                    self.name(),
                    self.settings().output.extension(),
                ));
                match self.write(&path) {
                    Ok(what) => {
                        self.note = format!("Salvato: {what}");
                        self.unsaved = false;
                        notify("Documento salvato", &path.display().to_string(), false);
                        open_folder(&folder);
                    }
                    Err(e) => {
                        self.note = format!("Salvataggio non riuscito: {e}");
                        notify("Salvataggio non riuscito", &e, true);
                    }
                }
            }
            ButtonAction::OpenFolder => open_folder(&self.folder()),
            ButtonAction::Nothing => {}
        }
    }

    /// A front-panel button asked something the app first asks about: the
    /// person is at the scanner, the question is on the screen.
    fn notify_question(&self) {
        if let Some(confirm) = &self.confirm {
            notify(&confirm.title, "Rispondi nell'app", false);
        }
    }

    /// Prints the photo shown on the default printer (the photocopy).
    fn print_shown(&mut self) {
        let shared = *self.settings();
        let Some(image) = self.shown().map(|p| p.finished(&shared)) else {
            return;
        };
        let path = std::env::temp_dir().join(Self::stamped("fotocopia", "png"));
        match hp3770_export::save(&path, &image) {
            Ok(()) => match print_file(&path) {
                Ok(()) => {
                    self.note = "Fotocopia inviata alla stampante predefinita".into();
                    notify("Fotocopia inviata alla stampante", "", false);
                }
                Err(e) => {
                    self.note = format!("Stampa non riuscita: {e}");
                    notify("Stampa non riuscita", &e, true);
                }
            },
            Err(e) => self.note = format!("Stampa non riuscita: {e}"),
        }
    }

    // ------------------------------------------------------------- views

    fn view(&self) -> Element<'_, Message> {
        let mut body = row![].height(Length::Fill);
        if self.kind == Kind::Document || self.pages.len() > 1 {
            body = body.push(self.strip());
        }
        body = body.push(self.stage()).push(self.inspector());
        let page = column![self.top_bar(), body, self.status_bar()];
        match &self.confirm {
            Some(confirm) => stack![page, self.question(confirm)].into(),
            None => page.into(),
        }
    }

    /// The question before throwing scans away, over the darkened window:
    /// a click outside it is "Annulla".
    fn question<'a>(&self, confirm: &'a Confirm) -> Element<'a, Message> {
        let card = container(
            column![
                text(&confirm.title).size(17),
                text(&confirm.body).size(13),
                row![
                    Space::new().width(Length::Fill),
                    button(text("Annulla").size(13))
                        .padding([6, 14])
                        .style(button::secondary)
                        .on_press(Message::Confirm(false)),
                    button(text("Salva…").size(13))
                        .padding([6, 14])
                        .style(button::primary)
                        .on_press(Message::ConfirmSave),
                    button(text(&confirm.yes).size(13))
                        .padding([6, 14])
                        .style(button::danger)
                        .on_press(Message::Confirm(true)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(14),
        )
        .padding(22)
        .max_width(460)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    radius: 10.0.into(),
                    width: 1.0,
                    color: palette.background.strong.color,
                },
                shadow: iced::Shadow {
                    color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                    offset: iced::Vector::new(0.0, 4.0),
                    blur_radius: 16.0,
                },
                ..Default::default()
            }
        });
        opaque(
            mouse_area(center(opaque(card)).style(|_: &Theme| container::Style {
                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
                ..Default::default()
            }))
            .on_press(Message::Confirm(false)),
        )
    }

    fn idle(&self) -> bool {
        self.busy.is_none() && self.worker.is_some()
    }

    /// Mode switch, scanner state, preview and the main action.
    fn top_bar(&self) -> Element<'_, Message> {
        let segment = |label: &'static str, kind: Option<Kind>| {
            let on = kind == Some(self.kind);
            button(text(label).size(13))
                .padding([6, 14])
                .style(move |theme: &Theme, status| {
                    let palette = theme.extended_palette();
                    let mut style = button::text(theme, status);
                    if on {
                        style.background = Some(palette.background.base.color.into());
                        style.text_color = palette.background.base.text;
                        style.border.radius = 6.0.into();
                        style.shadow = iced::Shadow {
                            color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.18),
                            offset: iced::Vector::new(0.0, 1.0),
                            blur_radius: 3.0,
                        };
                    } else if kind.is_none() {
                        style.text_color = palette.background.strong.color;
                    }
                    style
                })
                .on_press_maybe(kind.filter(|_| self.busy.is_none()).map(Message::Kind))
        };
        let modes = container(
            row![
                segment("Documento", Some(Kind::Document)),
                segment("Foto", Some(Kind::Photo)),
                segment("Pellicola", None),
            ]
            .spacing(2),
        )
        .padding(3)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.extended_palette().background.weak.color.into()),
            border: iced::Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let (dot, state) = match (&self.status, &self.busy) {
            (_, Some(_)) => (
                iced::Color::from_rgb(0.95, 0.65, 0.2),
                "Al lavoro".to_string(),
            ),
            (Some(Ok(s)), None) => (
                iced::Color::from_rgb(0.24, 0.77, 0.48),
                match (s.ready, s.lamp) {
                    (false, _) => "Occupato".to_string(),
                    (true, Some(LampState::Off)) => "Pronto · lampada spenta".to_string(),
                    (true, Some(_)) => "Pronto · lampada accesa".to_string(),
                    (true, None) => "Pronto".to_string(),
                },
            ),
            (Some(Err(_)), None) => (
                iced::Color::from_rgb(0.94, 0.36, 0.33),
                "Scanner non trovato".to_string(),
            ),
            (None, None) => (
                iced::Color::from_rgb(0.6, 0.6, 0.6),
                "Collegamento…".to_string(),
            ),
        };
        let chip = container(
            row![
                container(Space::new().width(8).height(8)).style(move |_: &Theme| {
                    container::Style {
                        background: Some(dot.into()),
                        border: iced::Border {
                            radius: 4.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                }),
                text(state).size(12),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([5, 10])
        .style(|theme: &Theme| container::Style {
            background: Some(theme.extended_palette().background.weak.color.into()),
            border: iced::Border {
                radius: 12.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let planned = self.plan().iter().map(|j| j.crops.len()).sum::<usize>();
        let next = match (self.kind, planned) {
            (Kind::Document, 0 | 1) => format!("Scansiona pagina {}", self.pages.len() + 1),
            (Kind::Document, n) => format!("Scansiona {n} pagine"),
            (Kind::Photo, 0 | 1) => "Scansiona foto".into(),
            (Kind::Photo, n) => format!("Scansiona {n} foto"),
        };
        // The name, and at its top right the version, small.
        let name = row![
            text("Scanjet 3770").size(15),
            text(concat!("v", env!("CARGO_PKG_VERSION")))
                .size(10)
                .style(text::secondary),
        ]
        .spacing(3)
        .align_y(Alignment::Start);
        let bar = row![
            name,
            modes,
            Space::new().width(Length::Fill),
            button(chip)
                .padding(0)
                .style(button::text)
                .on_press_maybe(self.idle().then_some(Message::RefreshStatus)),
            button(text("Anteprima").size(13))
                .style(button::secondary)
                .padding([7, 12])
                .on_press_maybe(self.idle().then_some(Message::Preview)),
            button(text(next).size(13))
                .padding([7, 16])
                .on_press_maybe(self.idle().then_some(Message::Scan)),
            pick_list(
                [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark],
                Some(self.theme),
                Message::Theme
            )
            .text_size(12),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        container(bar)
            .padding([10, 16])
            .width(Length::Fill)
            .style(surface)
            .into()
    }

    /// The pages of the document, top to bottom, each with its tools.
    fn strip(&self) -> Element<'_, Message> {
        let idle = self.busy.is_none();
        let tool = |label: &'static str, message: Message, enabled: bool| {
            button(text(label).size(12))
                .padding([1, 5])
                .style(button::text)
                .on_press_maybe(enabled.then_some(message))
        };
        let n = self.pages.len();
        let header = match self.kind {
            Kind::Document => "PAGINE",
            Kind::Photo => "FOTO",
        };
        let mut list = column![text(format!("{header} · {n}")).size(11)].spacing(10);
        // Where a page dragged would go: a bar before or after the page it
        // is over.
        let drop_at = match self.drag {
            Some((from, Some(to))) => Some((to, from > to)),
            _ => None,
        };
        let bar = || {
            container(Space::new().height(3).width(Length::Fill)).style(|theme: &Theme| {
                container::Style {
                    background: Some(theme.palette().primary.into()),
                    border: iced::Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
        };
        let dragging = self.drag.is_some_and(|(_, over)| over.is_some());
        for (i, page) in self.pages.iter().enumerate() {
            let selected = self.current == Some(i) && self.tab == Tab::Result;
            if drop_at == Some((i, true)) {
                list = list.push(bar());
            }
            // Click to show, double click to edit, drag to move.
            let thumb = mouse_area(
                container(
                    iced::widget::image(page.thumb.handle.clone())
                        .width(Length::Fill)
                        .height(140),
                )
                .padding(2)
                .style(move |theme: &Theme| container::Style {
                    border: iced::Border {
                        radius: 6.0.into(),
                        width: 2.0,
                        color: if selected {
                            theme.palette().primary
                        } else {
                            iced::Color::TRANSPARENT
                        },
                    },
                    ..Default::default()
                }),
            )
            .on_press(Message::PagePress(i))
            .on_enter(Message::PageHover(i))
            .on_release(Message::PageDrop)
            .on_double_click(EditMsg::Open(i).into())
            .interaction(if dragging {
                iced::mouse::Interaction::Grabbing
            } else {
                iced::mouse::Interaction::Pointer
            });
            let mut label = format!("{}", i + 1);
            if page.own.is_some() {
                label.push_str(" · propria");
            }
            if !page.transform.is_identity() {
                label.push_str(" · ✎");
            }
            if page.layout.is_some() {
                label.push_str(" · foglio");
            }
            let edited_here = self.editor().is_some_and(|e| e.index == i);
            list = list.push(
                column![
                    thumb,
                    row![
                        text(label).size(12).width(Length::Fill),
                        tool("↑", Message::PageMove(i, false), idle && i > 0),
                        tool("↓", Message::PageMove(i, true), idle && i + 1 < n),
                        tool("↻", Message::PageRotate(i, true), idle),
                    ]
                    .align_y(Alignment::Center),
                    row![
                        tool(
                            "Modifica",
                            EditMsg::Open(i).into(),
                            !edited_here && self.redone.is_none()
                        ),
                        tool("Rifai", Message::PageRedo(i), idle),
                    ],
                    row![
                        tool(
                            "Duplica",
                            Message::PageDuplicate(i),
                            idle && self.redone.is_none()
                        ),
                        Space::new().width(Length::Fill),
                        tool("Elimina", Message::PageDelete(i), idle),
                    ],
                ]
                .spacing(2),
            );
            if drop_at == Some((i, false)) {
                list = list.push(bar());
            }
        }
        // Which front-panel button adds a page (or scans a photo), as set
        // in the preferences.
        let (adds, what) = match self.kind {
            Kind::Document => (ButtonAction::AddPage, "aggiunge una pagina"),
            Kind::Photo => (ButtonAction::ScanPhoto, "scansiona una foto"),
        };
        // A page dragged out of the strip goes back where it was.
        let strip = |list| {
            let area = mouse_area(
                container(scrollable(list).height(Length::Fill))
                    .width(156)
                    .height(Length::Fill)
                    .style(surface),
            );
            match self.drag {
                Some((from, _)) => area.on_exit(Message::PageHover(from)).into(),
                None => area.into(),
            }
        };
        let Some(button) = (0..self.caps.buttons.len())
            .find(|&i| self.button_action(self.kind, i) == adds)
            .map(|i| self.caps.buttons[i].label.clone())
        else {
            return strip(list.padding([12, 10]));
        };
        list = list.push(
            container(
                text(format!("Il tasto {button} dello scanner {what}"))
                    .size(11)
                    .center(),
            )
            .padding(10)
            .width(Length::Fill)
            .style(|theme: &Theme| container::Style {
                border: iced::Border {
                    radius: 6.0.into(),
                    width: 1.0,
                    color: theme.extended_palette().background.strong.color,
                },
                text_color: Some(theme.extended_palette().background.strong.color),
                ..Default::default()
            }),
        );
        strip(list.padding([12, 10]))
    }

    /// The size of the PDF: the quality of its pictures, a largest size,
    /// and how large it will be.
    fn pdf_compression(&self) -> Element<'_, Message> {
        let s = self.settings();
        let e = hp3770_export::Encoding::from_percent(s.pdf_quality);
        let what = match (s.pdf_quality, e.max_dpi) {
            (100, _) => "come la scansione, JPEG 95".to_string(),
            (_, None) => format!("JPEG {}, pagine senza colore in grigio", e.quality),
            (_, Some(dpi)) => format!("JPEG {}, al massimo {dpi} dpi", e.quality),
        };
        let quality = column![
            labelled(
                "Qualità",
                row![
                    slider(
                        f32::from(PDF_QUALITY_MIN)..=100.0,
                        f32::from(s.pdf_quality),
                        |v| Message::PdfQuality(((v / 5.0).round() * 5.0) as u8),
                    )
                    .step(5.0_f32),
                    text(format!("{}%", s.pdf_quality)).size(13).width(42),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            ),
            text(what).size(11),
        ]
        .spacing(4);
        let typed = self.limit_draft.clone().unwrap_or_else(|| {
            s.pdf_limit_kb
                .map(|kb| {
                    let mb = f64::from(kb) / 1024.0;
                    format!("{mb:.2}")
                        .trim_end_matches('0')
                        .trim_end_matches('.')
                        .replace('.', ",")
                })
                .unwrap_or_default()
        });
        let invalid = parse_megabytes(&typed).is_none();
        let limit = labelled(
            "Al massimo",
            row![
                iced::widget::text_input("nessun limite", &typed)
                    .on_input(Message::PdfLimitText)
                    .size(13)
                    .width(Length::Fill),
                text("MB").size(13),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        );
        let key = self.estimate_key();
        let current = self.pdf_estimate.as_ref().filter(|e| e.key == key);
        let estimate = match (current, &self.pdf_estimate) {
            (Some(e), _) => match &e.result {
                Ok(p) => match s.pdf_limit_kb {
                    None => format!("Dimensione stimata: circa {}", human_size(p.bytes)),
                    Some(kb) if p.fits => {
                        let limit = human_size(u64::from(kb) * 1024);
                        match lightened(&p.encodings, e_base(s.pdf_quality)) {
                            Some(l) => format!("Circa {}, nel limite di {limit}: {l}", human_size(p.bytes)),
                            None => format!(
                                "Circa {}, già nel limite di {limit}: nessuna pagina da alleggerire",
                                human_size(p.bytes)
                            ),
                        }
                    }
                    Some(kb) => format!(
                        "Anche alleggerendo ogni pagina resta circa {}, oltre {}: prova \
                         Grigio o Bianco e nero, o una risoluzione di scansione più bassa",
                        human_size(p.bytes),
                        human_size(u64::from(kb) * 1024)
                    ),
                },
                Err(err) => format!("Stima non riuscita: {err}"),
            },
            (None, Some(old)) => match &old.result {
                Ok(p) => format!("Circa {} · aggiornamento…", human_size(p.bytes)),
                Err(_) => "Calcolo della dimensione…".into(),
            },
            (None, None) if self.pages.is_empty() => {
                "La dimensione si vede dopo la prima scansione".into()
            }
            (None, None) => "Calcolo della dimensione…".into(),
        };
        let mut out = column![heading("Dimensione del PDF"), quality, limit].spacing(8);
        if invalid {
            out = out.push(text("Scrivi un numero di megabyte, per esempio 5 o 2,5").size(11));
        }
        out.push(text(estimate).size(12)).into()
    }

    /// The canvas with its floating controls.
    fn stage(&self) -> Element<'_, Message> {
        let pick = self
            .eyedropper
            .then_some(Message::Picked as fn(Point) -> Message);
        let (view, zoom) = match self.tab {
            Tab::Preview => (
                viewer::viewer(
                    self.preview.as_ref().map(|(_, _, p)| p),
                    self.glass(),
                    "Premi «Anteprima» per vedere il vetro",
                    self.preview_view,
                    self.selection,
                    if self.settings().auto_crop {
                        self.found.iter().map(|f| f.corners()).collect()
                    } else {
                        Vec::new()
                    },
                    Message::View,
                    Some(Message::Select),
                    pick,
                    25.4 / f32::from(PREVIEW_DPI),
                    false,
                ),
                self.preview_view.scale,
            ),
            Tab::Result if self.print_preview() => {
                let page = &self.pages[self.current.unwrap_or(0)];
                let s = self.settings();
                let (l, own) = self.shown_layout();
                let (w, h) = page.inches();
                let placed = s.layout(page.layout, page.focus).place(w, h);
                let margin = match l.fit {
                    PdfFit::Actual => 0.0,
                    _ => f64::from(l.margin) * 72.0 / 25.4,
                };
                let caption = format!(
                    "{} · {} · {}{}",
                    s.pdf_size,
                    match l.sheet {
                        Sheet::Auto => "orientamento automatico",
                        Sheet::Portrait => "verticale",
                        Sheet::Landscape => "orizzontale",
                    },
                    l.fit.to_string().to_lowercase(),
                    if own { " · solo questa pagina" } else { "" }
                );
                (
                    sheet::sheet(
                        &page.picture,
                        placed,
                        margin,
                        page.focus,
                        Message::PageFocus,
                        caption,
                    ),
                    None,
                )
            }
            Tab::Result => (
                viewer::viewer(
                    self.shown().map(|p| &p.picture),
                    self.glass(),
                    "Nessuna scansione: premi «Scansiona»",
                    self.result_view,
                    self.crop_selection(),
                    Vec::new(),
                    Message::View,
                    self.cropping()
                        .then_some(editor::crop_message as fn(Option<Selection>) -> Message),
                    pick,
                    if self.cropping() {
                        self.shown()
                            .map_or(0.0, |p| 25.4 / f32::from(p.raw.full.dpi.max(1)))
                    } else {
                        0.0
                    },
                    self.cropping(),
                ),
                self.result_view.scale,
            ),
        };
        let small = |label: &'static str, message: Option<Message>, on: bool| {
            button(text(label).size(12))
                .padding([5, 9])
                .style(move |theme: &Theme, status| {
                    let mut style = button::text(theme, status);
                    if on {
                        style.background = Some(theme.extended_palette().primary.weak.color.into());
                        style.text_color = theme.extended_palette().primary.weak.text;
                        style.border.radius = 5.0.into();
                    }
                    style
                })
                .on_press_maybe(message)
        };
        let result_label = match self.kind {
            Kind::Document => "Pagina",
            Kind::Photo => "Foto",
        };
        let views = card(
            row![
                small(
                    "Anteprima",
                    Some(Message::Tab(Tab::Preview)),
                    self.tab == Tab::Preview
                ),
                small(
                    result_label,
                    Some(Message::Tab(Tab::Result)),
                    self.tab == Tab::Result
                ),
            ]
            .spacing(2),
        );
        let edit = (self.tab == Tab::Result
            && self.editor().is_none()
            && self.redone.is_none()
            && self.shown().is_some())
        .then(|| self.current.map(|i| EditMsg::Open(i).into()))
        .flatten();
        let tools = card(
            column![
                small("Modifica…", edit, false),
                small("Contagocce", Some(Message::Eyedropper), self.eyedropper),
                small(
                    "Togli selezione",
                    (self.tab == Tab::Preview && self.selection.is_some())
                        .then_some(Message::Select(None)),
                    false
                ),
            ]
            .spacing(2),
        );
        let compare = card(
            row![
                small("Prima", Some(Message::Before(true)), self.before),
                small("Dopo", Some(Message::Before(false)), !self.before),
            ]
            .spacing(2),
        );
        let zoom_label = match zoom {
            None => "Vista intera".to_string(),
            Some(s) => format!("{:.0}%", s * 100.0),
        };
        let zoom = card(
            row![
                text(zoom_label).size(12),
                small("Adatta", Some(Message::Fit), false),
                small("1:1", Some(Message::ActualSize), false),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
        let mut layers = iced::widget::Stack::new()
            .push(view)
            .push(
                container(column![views, tools].spacing(8))
                    .padding(14)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .push(
                container(compare)
                    .padding(14)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Right),
            )
            .push(
                container(zoom)
                    .padding(14)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Right)
                    .align_y(iced::alignment::Vertical::Bottom),
            );
        if let Some(bar) = self.editor_bar() {
            layers = layers.push(
                container(bar)
                    .padding(14)
                    .width(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Center),
            );
        }
        if let Some(progress) = self.progress_card() {
            layers = layers.push(
                container(progress)
                    .padding(18)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Center)
                    .align_y(iced::alignment::Vertical::Bottom),
            );
        }
        if let Some((index, _)) = &self.redone {
            if self.tab == Tab::Result {
                layers = layers.push(
                    container(card(
                        row![
                            text(format!("Nuova scansione della {}", self.page_name(*index)))
                                .size(13),
                            button(text("Tieni la nuova").size(13)).on_press(Message::AcceptRedo),
                            button(text("Lascia la vecchia").size(13))
                                .style(button::secondary)
                                .on_press(Message::RejectRedo),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    ))
                    .padding(14)
                    .width(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Center),
                );
            }
        }
        container(layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(
                        if palette.is_dark {
                            iced::Color::from_rgb8(16, 18, 22)
                        } else {
                            iced::Color::from_rgb8(227, 229, 234)
                        }
                        .into(),
                    ),
                    ..Default::default()
                }
            })
            .into()
    }

    /// The progress of the running scan, shown on the canvas.
    fn progress_card(&self) -> Option<Element<'_, Message>> {
        let (purpose, stage) = self.busy.as_ref()?;
        let what = match purpose {
            Purpose::Preview => "Anteprima".to_string(),
            Purpose::Page => match self.kind {
                // "Rifai": the page done again, not the next one.
                _ if self.redo.is_some() => format!(
                    "Nuova scansione della {}",
                    self.page_name(self.redo.unwrap_or_default())
                ),
                _ if self.job.as_ref().is_some_and(|j| j.crops.len() > 1) => format!(
                    "Scansione di {} originali in una passata",
                    self.job.as_ref().map_or(0, |j| j.crops.len())
                ),
                _ if self.batch.1 > 1 => {
                    format!("Scansione {} di {}", self.batch.0 + 1, self.batch.1)
                }
                Kind::Document => format!("Scansione pagina {}", self.pages.len() + 1),
                Kind::Photo => "Scansione foto".to_string(),
            },
        };
        let (fraction, detail) = match stage {
            None | Some(Progress::Starting) => (0.02, "Avvio dello scanner"),
            Some(Progress::WarmingUp) => (0.05, "La lampada era spenta: riscaldamento"),
            Some(Progress::Calibrating) => (0.1, "Calibrazione"),
            Some(Progress::Scanning { .. }) => (0.0, "Lettura"),
            Some(Progress::Finishing) => (0.97, "Fine della scansione ed elaborazione"),
        };
        let fraction = match stage {
            Some(Progress::Scanning { done, total }) => {
                0.1 + 0.85 * *done as f32 / (*total).max(1) as f32
            }
            _ => fraction,
        };
        Some(card(
            column![
                row![
                    text(format!("{what} · {:.0}%", fraction * 100.0))
                        .size(13)
                        .width(Length::Fill),
                    button(text("Annulla").size(12))
                        .style(button::danger)
                        .padding([3, 10])
                        .on_press(Message::Cancel),
                ]
                .align_y(Alignment::Center),
                progress_bar(0.0..=1.0, fraction).girth(6),
                text(detail).size(11),
            ]
            .spacing(8)
            .width(420),
        ))
    }

    /// The inspector: scan settings, adjustments, output.
    fn inspector(&self) -> Element<'_, Message> {
        let tab = |label: &'static str, pane: Pane| {
            let on = self.pane == pane;
            button(text(label).size(13))
                .padding([8, 12])
                .style(move |theme: &Theme, status| {
                    let mut style = button::text(theme, status);
                    if on {
                        style.text_color = theme.extended_palette().background.base.text;
                        style.border = iced::Border {
                            color: theme.palette().primary,
                            width: 0.0,
                            radius: 0.0.into(),
                        };
                        style.background =
                            Some(theme.extended_palette().background.weak.color.into());
                    }
                    style
                })
                .on_press(Message::Pane(pane))
        };
        let tabs = row![
            tab("Scansione", Pane::Scan),
            tab("Regola", Pane::Adjust),
            tab("Uscita", Pane::Output),
            tab("Preferenze", Pane::Prefs),
        ]
        .spacing(2);
        if self.editor().is_some() {
            return container(
                column![
                    scrollable(container(self.editor_pane()).padding([12, 16]))
                        .height(Length::Fill),
                    rule::horizontal(1),
                    container(
                        row![
                            button(text("Scarta").size(14).center().width(Length::Fill))
                                .width(Length::Fill)
                                .padding([9, 12])
                                .style(button::secondary)
                                .on_press(EditMsg::Discard.into()),
                            button(text("Applica").size(14).center().width(Length::Fill))
                                .width(Length::Fill)
                                .padding([9, 12])
                                .on_press(EditMsg::Apply.into()),
                        ]
                        .spacing(8)
                    )
                    .padding(12),
                ]
                .height(Length::Fill),
            )
            .width(352)
            .height(Length::Fill)
            .style(surface)
            .into();
        }
        let content = match self.pane {
            Pane::Scan => self.scan_pane(),
            Pane::Adjust => self.adjust_pane(),
            Pane::Output => self.output_pane(),
            Pane::Prefs => self.prefs_pane(),
        };
        let footer = self.footer();
        container(
            column![
                container(tabs).padding([8, 12]),
                rule::horizontal(1),
                scrollable(container(content).padding([12, 16])).height(Length::Fill),
                rule::horizontal(1),
                container(footer).padding(12),
            ]
            .height(Length::Fill),
        )
        .width(352)
        .height(Length::Fill)
        .style(surface)
        .into()
    }

    fn scan_pane(&self) -> Element<'_, Message> {
        let s = self.active();
        let request = self.request();
        let (w, h) = (request.area.width, request.area.height);
        let (x, y, wmm, hmm) = self.area_mm();
        let memory = self.memory();
        let mut pane = column![
            self.access_card(),
            heading("Originale"),
            labelled(
                "Risoluzione",
                pick_list(
                    self.caps.resolutions.clone(),
                    Some(self.dpi()),
                    Message::Dpi
                )
                .width(Length::Fill)
                .into()
            ),
            labelled(
                "Colore",
                pick_list(ALL_COLOURS, Some(s.colour), Message::Colour)
                    .width(Length::Fill)
                    .into()
            ),
        ]
        .spacing(10);
        // The depth only for a scanner that gives 16 bits.
        if self.offers_16bit() {
            pane = pane
                .push(labelled(
                    "Profondità",
                    pick_list(
                        [Depth(8), Depth(16)],
                        Some(Depth(self.depth())),
                        Message::Depth,
                    )
                    .width(Length::Fill)
                    .into(),
                ))
                .push(
                    text(if self.depth() == 16 {
                        "16 bit: ombre e sfumature senza scalini anche dopo curve forti; \
                         più memoria."
                    } else {
                        "8 bit: come il driver HP. Per foto e RAW meglio 16 bit."
                    })
                    .size(11),
                );
        }
        pane = pane.push(heading("Area")).push(
            checkbox(self.settings().auto_crop)
                .label("Ritaglio automatico")
                .on_toggle(Message::AutoCrop)
                .text_size(13),
        );
        let plan = self.plan();
        if !plan.is_empty() {
            let n = self.found.len();
            pane = pane.push(
                text(match n {
                    1 => "Un originale riconosciuto sul vetro: sarà ritagliato e raddrizzato."
                        .to_string(),
                    n => format!(
                        "{n} originali riconosciuti sul vetro: ognuno diventa un'immagine, \
                         ritagliata e raddrizzata{}.",
                        if plan.len() == 1 {
                            ", in una passata sola"
                        } else {
                            ", uno alla volta"
                        }
                    ),
                })
                .size(12),
            );
            let mm = 25.4 / f32::from(PREVIEW_DPI);
            for (i, f) in self.found.iter().enumerate() {
                let tilt = if f.angle == 0.0 {
                    "dritto".to_string()
                } else {
                    format!("storto di {:.1}°", f.angle.abs()).replace('.', ",")
                };
                pane = pane.push(
                    row![
                        text(format!(
                            "{} · {:.0} × {:.0} mm · {tilt}",
                            i + 1,
                            f.width * mm,
                            f.height * mm
                        ))
                        .size(12)
                        .width(Length::Fill),
                        button(text("Escludi").size(11))
                            .padding([1, 6])
                            .style(button::text)
                            .on_press(Message::DropFound(i)),
                    ]
                    .align_y(Alignment::Center),
                );
            }
            pane = pane.push(text(format!("in memoria fino a {:.0} MB", memory / 1e6)).size(12));
        } else {
            if self.settings().auto_crop && self.preview.is_some() {
                pane = pane.push(
                    text(if self.selection.is_some() {
                        "La selezione ha la precedenza sul ritaglio automatico."
                    } else {
                        "Nessun originale riconosciuto (un foglio bianco sul coperchio \
                         bianco può sfuggire): trascina sull'anteprima per scegliere l'area."
                    })
                    .size(11),
                );
            }
            pane = pane
                .push(
                    text(if self.selection.is_none() {
                        "Tutto il formato A4. Trascina sull'anteprima per scegliere un'area."
                            .to_string()
                    } else {
                        format!("Da {x:.0}, {y:.0} mm · {wmm:.0} × {hmm:.0} mm")
                    })
                    .size(12),
                )
                .push(
                    text(format!(
                        "{w} × {h} pixel · in memoria {:.0} MB",
                        memory / 1e6
                    ))
                    .size(12),
                );
        }
        if self.settings().auto_crop && self.preview.is_some() {
            pane = pane.push(
                button(text("Riconosci di nuovo").size(12))
                    .padding([3, 8])
                    .style(button::secondary)
                    .on_press(Message::Detect),
            );
        }
        if memory > LARGE_SCAN {
            pane = pane.push(
                text(format!(
                    "Scansione molto grande: circa {:.1} GB e alcuni minuti. \
                     Seleziona un'area o abbassa la risoluzione se non serve tutta.",
                    memory / 1e9
                ))
                .size(12)
                .style(text::danger),
            );
        }
        if self.dpi() > 1200 && self.selection.is_none() && plan.is_empty() {
            pane = pane.push(
                text("2400 dpi servono per piccoli originali: seleziona l'area sull'anteprima.")
                    .size(12)
                    .style(text::warning),
            );
        }
        // Only for a scanner with buttons.
        if !self.caps.buttons.is_empty() {
            let what: Vec<String> = self
                .caps
                .buttons
                .iter()
                .enumerate()
                .map(|(i, b)| format!("{}: {}", b.label, self.button_action(self.kind, i)))
                .collect();
            pane = pane.push(heading("Tasti dello scanner")).push(
                text(format!(
                    "{} (si cambiano nelle Preferenze)",
                    what.join(" · ")
                ))
                .size(12),
            );
        }
        pane.into()
    }

    fn adjust_pane(&self) -> Element<'_, Message> {
        let editing = if self.editing_page() {
            match (&self.redone, self.current) {
                (Some((i, _)), _) => format!("Nuova scansione della {}", self.page_name(*i)),
                (None, Some(i)) => self.page_title(i),
                (None, None) => String::new(),
            }
        } else {
            "Prossima scansione".to_string()
        };
        let scope: Element<'_, Message> = match (self.kind, self.editing_page()) {
            (Kind::Document, true) => {
                let own = self.shown().is_some_and(|p| p.own.is_some());
                let mut scope = column![
                    checkbox(own)
                        .label("Solo questa pagina")
                        .on_toggle(Message::OwnLook)
                        .text_size(13),
                    text(if own {
                        "Le modifiche valgono solo per questa pagina."
                    } else {
                        "Segue le regolazioni comuni: le modifiche valgono anche per \
                         l'anteprima e per le pagine senza «Solo questa pagina»."
                    })
                    .size(11),
                ]
                .spacing(4);
                if self.pages.len() > 1 {
                    scope = scope.push(
                        button(text("Applica a tutte le pagine").size(12))
                            .style(button::text)
                            .padding([2, 6])
                            .on_press(Message::LookApplyToAll),
                    );
                }
                scope.into()
            }
            (Kind::Document, false) => text(
                "Le modifiche valgono per l'anteprima, per le prossime scansioni \
                 e per le pagine senza «Solo questa pagina».",
            )
            .size(11)
            .into(),
            (Kind::Photo, _) => text("Anteprima e foto hanno sempre le stesse regolazioni.")
                .size(11)
                .into(),
        };
        let top = column![text(editing.to_uppercase()).size(11), scope].spacing(10);
        self.adjust_controls(top.into())
    }

    /// The adjustments (presets, histogram, groups) under `top`: the
    /// "Regola" pane, and the colours of the editor.
    fn adjust_controls<'a>(&'a self, top: Element<'a, Message>) -> Element<'a, Message> {
        let s = self.active();
        let lineart = s.colour == Colour::Lineart;
        let histogram: Element<'_, Message> = {
            let shown = if self.editing_page() {
                self.shown().map(|p| &p.shown)
            } else {
                self.preview.as_ref().map(|(_, s, _)| s)
            };
            match shown.map(Histogram::of) {
                Some(hist) if !hist.is_empty() => {
                    let (black, white) = hist.clipped();
                    column![
                        canvas(hist).width(Length::Fill).height(84),
                        row![
                            text(format!("Neri tagliati {black:.1}%"))
                                .size(11)
                                .width(Length::Fill),
                            text(format!("Bianchi tagliati {white:.1}%")).size(11),
                        ],
                    ]
                    .spacing(4)
                    .into()
                }
                _ => text("L'istogramma compare dopo l'anteprima")
                    .size(11)
                    .into(),
            }
        };
        let base = self.group(
            Group::Tone,
            "Base",
            !lineart,
            "In bianco e nero conta la soglia, qui sotto",
            column![
                row![
                    button(text("Contagocce neutro").size(12))
                        .style(if self.eyedropper {
                            button::primary
                        } else {
                            button::secondary
                        })
                        .on_press(Message::Eyedropper),
                    button(text("Livelli auto").size(12))
                        .style(button::secondary)
                        .on_press(Message::AutoLevels),
                ]
                .spacing(6),
                self.adjuster(Field::Temperature, &s),
                self.adjuster(Field::Tint, &s),
                self.adjuster(Field::Exposure, &s),
                self.adjuster(Field::Contrast, &s),
                self.adjuster(Field::Highlights, &s),
                self.adjuster(Field::Shadows, &s),
                self.adjuster(Field::Whites, &s),
                self.adjuster(Field::Blacks, &s),
                text("Livelli").size(12),
                self.adjuster(Field::Black, &s),
                self.adjuster(Field::White, &s),
                self.adjuster(Field::Gamma, &s),
                self.adjuster(Field::Brightness, &s),
            ]
            .spacing(8)
            .into(),
        );
        let colour = self.group(
            Group::Colour,
            "Colore",
            s.colour == Colour::Rgb,
            "Solo per le scansioni a colori",
            column![
                self.adjuster(Field::Vibrance, &s),
                self.adjuster(Field::Saturation, &s),
            ]
            .spacing(8)
            .into(),
        );
        let detail = self.group(
            Group::Detail,
            "Dettaglio",
            true,
            "",
            column![
                self.adjuster(Field::Sharpen, &s),
                self.adjuster(Field::Noise, &s),
                labelled(
                    "Descreening",
                    pick_list(SCREENS, Some(Screen(s.adjust.descreen)), Message::Descreen)
                        .width(Length::Fill)
                        .text_size(13)
                        .into()
                ),
                text("Rimuove il retino di stampa di riviste e giornali (effetto moiré).").size(11),
            ]
            .spacing(8)
            .into(),
        );
        let channel_tab = |label: &'static str, c: CurveChannel| {
            button(text(label).size(12))
                .padding([3, 10])
                .style(if self.curve_channel == c {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::CurveChannel(c))
        };
        let curve_colour = match self.curve_channel {
            CurveChannel::All => iced::Color::from_rgb(0.85, 0.85, 0.85),
            CurveChannel::Red => iced::Color::from_rgb(0.95, 0.3, 0.3),
            CurveChannel::Green => iced::Color::from_rgb(0.3, 0.8, 0.35),
            CurveChannel::Blue => iced::Color::from_rgb(0.35, 0.5, 1.0),
        };
        let mut tabs = row![channel_tab("Tutti", CurveChannel::All)].spacing(4);
        if s.colour == Colour::Rgb {
            tabs = tabs
                .push(channel_tab("R", CurveChannel::Red))
                .push(channel_tab("G", CurveChannel::Green))
                .push(channel_tab("B", CurveChannel::Blue));
        }
        let channel = if s.colour == Colour::Rgb {
            self.curve_channel.index()
        } else {
            0
        };
        let curve_preset = |label: &'static str, c: scan_imaging::adjust::Curve| {
            button(text(label).size(11))
                .padding([3, 8])
                .style(button::secondary)
                .on_press(Message::CurveSet(c.points().to_vec()))
        };
        let curve_histogram = if self.editing_page() {
            self.shown().map(|p| &p.shown)
        } else {
            self.preview.as_ref().map(|(_, s, _)| s)
        }
        .and_then(|img| Histogram::of(img).levels());
        let curve_group = self.group(
            Group::Curve,
            "Curva",
            !lineart,
            "Non serve in bianco e nero",
            column![
                tabs,
                canvas(curve::Editor {
                    curve: s.adjust.curve[channel],
                    colour: curve_colour,
                    histogram: curve_histogram,
                    on_change: Message::CurveSet,
                })
                .width(Length::Fill)
                .height(220),
                row![
                    curve_preset("Lineare", scan_imaging::adjust::Curve::IDENTITY),
                    curve_preset(
                        "Contrasto medio",
                        scan_imaging::adjust::Curve::MEDIUM_CONTRAST
                    ),
                    curve_preset(
                        "Contrasto forte",
                        scan_imaging::adjust::Curve::STRONG_CONTRAST
                    ),
                ]
                .spacing(4),
                text(
                    "Clic per aggiungere un punto (fino a 16), trascina per spostarlo, \
                     doppio clic o trascina fuori per toglierlo. Gli estremi sono il punto \
                     del nero e del bianco."
                )
                .size(11),
            ]
            .spacing(6)
            .into(),
        );
        let bw = self.group(
            Group::Lineart,
            "Bianco e nero",
            lineart,
            "Solo con il colore «Bianco e nero»",
            column![
                button(text("Soglia automatica").size(12))
                    .style(button::secondary)
                    .on_press(Message::AutoThreshold),
                self.adjuster(Field::Threshold, &s),
                checkbox(s.adjust.adaptive)
                    .label("Soglia adattiva (pagine illuminate in modo non uniforme)")
                    .on_toggle(Message::Adaptive)
                    .text_size(13),
            ]
            .spacing(8)
            .into(),
        );
        let names: Vec<String> = factory_presets(self.kind)
            .into_iter()
            .map(|(n, _)| n.to_string())
            .chain(self.presets.iter().map(|p| p.name.clone()))
            .collect();
        let mut presets = column![row![pick_list(names, None::<String>, Message::PresetApply)
            .placeholder("Applica un preset…")
            .text_size(13)
            .width(Length::Fill),]]
        .spacing(6);
        let full = self.presets.len() >= prefs::MAX_PRESETS
            && !self
                .presets
                .iter()
                .any(|p| p.name == self.preset_name.trim());
        presets = presets.push(
            row![
                iced::widget::text_input("Nome del preset", &self.preset_name)
                    .on_input(Message::PresetName)
                    .on_submit(Message::PresetSave)
                    .size(13),
                button(text("Salva").size(13))
                    .style(button::secondary)
                    .on_press_maybe((!full).then_some(Message::PresetSave)),
            ]
            .spacing(6),
        );
        if full {
            presets = presets.push(
                text("Tre preset salvati: scrivi il nome di uno per sostituirlo, o eliminane uno.")
                    .size(11),
            );
        }
        for p in &self.presets {
            presets = presets.push(
                row![
                    text(&p.name).size(12).width(Length::Fill),
                    button(text("Elimina").size(11))
                        .style(button::text)
                        .padding([1, 6])
                        .on_press(Message::PresetDelete(p.name.clone())),
                ]
                .align_y(Alignment::Center),
            );
        }
        column![
            top,
            presets,
            histogram,
            base,
            curve_group,
            colour,
            detail,
            bw
        ]
        .spacing(10)
        .into()
    }

    /// When the scanner is plugged in but may not be opened (Linux, udev
    /// rule missing): what is wrong and the button that fixes it.
    fn access_card(&self) -> Element<'_, Message> {
        if !self.access_denied {
            return Space::new().into();
        }
        container(
            column![
                text("Lo scanner è collegato, ma Linux non lo lascia usare a questo utente.")
                    .size(13),
                text(format!(
                    "Basta una volta: installa la regola {} (chiede la password).",
                    access::RULE_PATH_SHOWN
                ))
                .size(11),
                button(text("Permetti l'accesso allo scanner").size(13))
                    .padding([6, 12])
                    .on_press_maybe(self.idle().then_some(Message::GrantAccess)),
            ]
            .spacing(6),
        )
        .padding(10)
        .width(Length::Fill)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.warning.weak.color.into()),
                text_color: Some(palette.warning.weak.text),
                border: iced::Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into()
    }

    /// Whether Windows opens its own scan app on a button press, and the
    /// switch (Windows only, with the HP driver).
    fn wia_row(&self) -> Element<'_, Message> {
        let hp_usb = self
            .device
            .as_ref()
            .is_some_and(|d| d.backend == hp3770_core::rts8822::BACKEND);
        match self.wia_quiet.filter(|_| hp_usb) {
            None => Space::new().into(),
            Some(quiet) => column![
                text(if quiet {
                    "Windows lascia i tasti a questa app."
                } else {
                    "Windows apre anche la sua app di scansione a ogni tasto, \
                     e toglie lo scanner a questa."
                })
                .size(11),
                button(
                    text(if quiet {
                        "Ridai i tasti a Windows"
                    } else {
                        "Usa i tasti dello scanner solo con questa app"
                    })
                    .size(12)
                )
                .padding([4, 10])
                .style(if quiet {
                    button::secondary
                } else {
                    button::primary
                })
                .on_press(Message::WiaQuiet(!quiet)),
                text("Chiede una volta la conferma di amministratore.").size(11),
            ]
            .spacing(4)
            .into(),
        }
    }

    /// The preferences tied to functions of the scanner in use: which
    /// scanner (when there are several), its buttons, its lamp. Only what
    /// it has is shown.
    fn scanner_prefs(&self) -> Element<'_, Message> {
        let mut col = column![].spacing(10);
        if self.devices.len() > 1 {
            col = col.push(heading("Scanner")).push(labelled(
                "Usa",
                pick_list(
                    self.devices.clone(),
                    self.device.clone(),
                    Message::ChooseScanner,
                )
                .width(Length::Fill)
                .text_size(13)
                .into(),
            ));
        }
        if !self.caps.buttons.is_empty() {
            col = col.push(heading(match self.kind {
                Kind::Document => "Tasti dello scanner in Documento",
                Kind::Photo => "Tasti dello scanner in Foto",
            }));
            for (i, b) in self.caps.buttons.iter().enumerate() {
                col = col.push(self.button_choice(i, b.label.clone()));
            }
            col = col
                .push(
                    text(
                        "Valgono anche con l'app in secondo piano; ignorati durante una \
                         scansione.",
                    )
                    .size(11),
                )
                .push(self.wia_row());
        }
        if self.caps.lamp.is_some() {
            col = col
                .push(heading("Lampada"))
                .push(labelled(
                    "Spegnila",
                    pick_list(
                        LAMP_CHOICES,
                        Some(LampAfter(self.lamp_off)),
                        Message::LampAfter,
                    )
                    .width(Length::Fill)
                    .into(),
                ))
                .push(
                    text(
                        "Senza scansioni per questo tempo l'app spegne la lampada, e la \
                         spegne sempre chiudendosi. Riaccenderla costa circa 10 secondi di \
                         riscaldamento alla scansione dopo. Lo scanner la spegne comunque da \
                         solo dopo 13 minuti.",
                    )
                    .size(11),
                );
        }
        col.into()
    }

    /// The choice of what a front-panel button does.
    fn button_choice(&self, index: usize, label: String) -> Element<'_, Message> {
        let options = ButtonAction::allowed(self.kind);
        let current = self.button_action(self.kind, index);
        labelled(
            label,
            pick_list(options, Some(current), move |a| {
                Message::ButtonAction(index, a)
            })
            .width(Length::Fill)
            .text_size(13)
            .into(),
        )
    }

    fn prefs_pane(&self) -> Element<'_, Message> {
        let path = prefs::Prefs::path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        column![
            heading("All'avvio"),
            labelled(
                "Modo",
                pick_list(
                    [StartMode::Last, StartMode::Document, StartMode::Photo],
                    Some(self.start_mode),
                    Message::StartMode
                )
                .width(Length::Fill)
                .into()
            ),
            text(
                "Risoluzione, colore, formato e pagina PDF di ogni modo restano come li \
                 lasci. Le regolazioni ripartono da zero a ogni apertura: per ritrovarle, \
                 salvale in un preset."
            )
            .size(12),
            heading("Cartella dei file"),
            text(self.folder().display().to_string()).size(12),
            row![
                button(text("Scegli…").size(13))
                    .style(button::secondary)
                    .on_press(Message::ChooseFolder),
                button(text(format!("Immagini{}Scansioni", std::path::MAIN_SEPARATOR)).size(13))
                    .style(button::text)
                    .on_press_maybe(
                        self.custom_folder
                            .is_some()
                            .then_some(Message::DefaultFolder)
                    ),
            ]
            .spacing(8),
            heading("Nomi dei file"),
            labelled(
                "Documenti",
                iced::widget::text_input("documento", &self.document_name)
                    .on_input(Message::DocumentName)
                    .size(13)
                    .into()
            ),
            labelled(
                "Foto",
                iced::widget::text_input("foto", &self.photo_name)
                    .on_input(Message::PhotoName)
                    .size(13)
                    .into()
            ),
            text(format!(
                "Esempio: {}",
                Self::stamped(self.name(), self.settings().output.extension())
            ))
            .size(12),
            self.scanner_prefs(),
            heading("Impostazioni"),
            text(format!("Salvate in {path}")).size(11),
            button(text("Ripristina impostazioni di fabbrica").size(13))
                .style(button::danger)
                .on_press(Message::FactoryReset),
        ]
        .spacing(10)
        .into()
    }

    fn output_pane(&self) -> Element<'_, Message> {
        let s = self.active();
        let options = outputs(self.kind, s.colour, self.depth());
        let mut pane = column![
            heading("Formato"),
            pick_list(options, Some(self.settings().output), Message::Output).width(Length::Fill),
        ]
        .spacing(10);
        if s.colour == Colour::Lineart {
            pane =
                pane.push(text("Bianco e nero: niente JPEG/AVIF, rovinerebbero i bordi.").size(11));
        }
        if let Output::Raw(_) = self.settings().output {
            pane = pane.push(
                text(
                    "I campioni dello scanner, lineari e senza regolazioni (ritaglio e \
                     raddrizzatura sì): si sviluppano in Lightroom, Camera Raw o Photoshop. \
                     Le regolazioni a destra valgono solo per l'anteprima.",
                )
                .size(11),
            );
        } else if self.kind == Kind::Photo && s.colour == Colour::Rgb {
            pane = pane.push(
                text("Per Lightroom o Photoshop scegli un formato RAW (DNG o TIFF 16 bit).")
                    .size(11),
            );
        }
        if self.settings().output == Output::Pdf {
            let s = *self.settings();
            pane = pane.push(labelled(
                "Foglio",
                pick_list(PDF_SIZES, Some(s.pdf_size), Message::PdfSize)
                    .width(Length::Fill)
                    .into(),
            ));
            if s.pdf_size != PdfSize::Fit {
                // The document's layout, or the own one of the page shown.
                let (l, own) = self.shown_layout();
                if let Some(i) = self.current.filter(|&i| i < self.pages.len()) {
                    pane = pane.push(
                        checkbox(own)
                            .label(format!(
                                "Impaginazione propria per la {}",
                                self.page_name(i)
                            ))
                            .on_toggle(Message::PdfOwnLayout)
                            .text_size(13),
                    );
                }
                pane = pane.push(
                    text(if own {
                        "Le scelte qui sotto valgono solo per questa pagina."
                    } else {
                        "Le scelte qui sotto valgono per tutte le pagine senza impaginazione propria."
                    })
                    .size(11),
                );
                pane = pane.push(labelled(
                    "Orientamento",
                    pick_list(SHEETS, Some(l.sheet), Message::PdfSheet)
                        .width(Length::Fill)
                        .into(),
                ));
                pane = pane.push(labelled(
                    "Sul foglio",
                    pick_list(PDF_FITS, Some(l.fit), Message::PdfFit)
                        .width(Length::Fill)
                        .into(),
                ));
                if l.fit != PdfFit::Actual {
                    pane = pane.push(labelled(
                        "Margine",
                        pick_list(
                            PDF_MARGINS.map(Margin),
                            Some(Margin(l.margin)),
                            Message::PdfMargin,
                        )
                        .width(Length::Fill)
                        .into(),
                    ));
                }
                if own {
                    pane = pane.push(
                        row![
                            button(text("Applica a tutte le pagine").size(13))
                                .style(button::secondary)
                                .on_press(Message::PdfApplyToAll),
                            button(text("Come le altre").size(13))
                                .style(button::text)
                                .on_press(Message::PdfOwnLayout(false)),
                        ]
                        .spacing(8),
                    );
                }
                pane = pane.push(
                    text(match l.fit {
                        PdfFit::Actual => {
                            "Alla sua misura vera, al centro del foglio (rimpicciolita solo se non ci sta): stampata al 100% misura come l'originale."
                        }
                        PdfFit::Page => {
                            "Grande quanto il foglio permette, intera, dentro il margine: un ritaglio piccolo riempie l'A4."
                        }
                        PdfFit::Fill => {
                            "Copre il foglio dentro il margine; quello che avanza si taglia: trascina la pagina sul foglio, a sinistra, per scegliere cosa tenere."
                        }
                    })
                    .size(11),
                );
                pane = pane.push(
                    text(
                        "Il foglio con la pagina sopra è nel visore: in rosso il bordo di quello che finisce sulla carta.",
                    )
                    .size(11),
                );
            }
        }
        if self.settings().output == Output::Pdf {
            pane = pane.push(self.pdf_compression());
        }
        pane = pane
            .push(heading("Cartella"))
            .push(text(self.folder().display().to_string()).size(12));
        if self.kind == Kind::Photo {
            pane = pane.push(
                checkbox(self.settings().auto_save)
                    .label("Salva da sola ogni foto nella cartella")
                    .on_toggle(Message::AutoSave)
                    .text_size(13),
            );
        }
        if self.kind == Kind::Document {
            pane = pane.push(
                button(text("Nuovo documento").size(13))
                    .style(button::secondary)
                    .on_press_maybe(
                        (!self.pages.is_empty() && self.busy.is_none())
                            .then_some(Message::NewDocument),
                    ),
            );
        }
        pane.into()
    }

    /// The main action of the inspector: saving.
    fn footer(&self) -> Element<'_, Message> {
        let output = self.settings().output;
        let (label, enabled) = match self.kind {
            Kind::Document => (
                format!("Salva {output}…"),
                !self.pages.is_empty() && self.busy.is_none(),
            ),
            Kind::Photo => (
                format!("Salva {output}…"),
                self.shown().is_some() && self.busy.is_none(),
            ),
        };
        button(text(label).size(14).center().width(Length::Fill))
            .width(Length::Fill)
            .padding([9, 12])
            .on_press_maybe(enabled.then_some(Message::Save))
            .into()
    }

    /// Facts at the bottom: scanner, area, resolution, last note.
    fn status_bar(&self) -> Element<'_, Message> {
        let s = self.settings();
        let request = self.request();
        let link = match &self.status {
            // What the scanner says first about itself ("USB 2.0").
            Some(Ok(st)) => st
                .details
                .first()
                .map_or_else(|| "collegato".to_string(), |(l, v)| format!("{l} {v}")),
            Some(Err(_)) => "non collegato".to_string(),
            None => "…".to_string(),
        };
        let fact = |label: &'static str, value: String| {
            row![text(label).size(11), text(value).size(11)].spacing(4)
        };
        container(
            row![
                fact("Scanner", link),
                fact("Risoluzione", format!("{} dpi · {}", self.dpi(), s.colour)),
                fact(
                    "Immagine",
                    format!("{} × {}", request.area.width, request.area.height)
                ),
                text(self.note.clone()).size(11).width(Length::Fill),
                text("Rotella zoom · tasto destro sposta · doppio clic su un cursore lo azzera")
                    .size(11),
            ]
            .spacing(18)
            .align_y(Alignment::Center),
        )
        .padding([6, 16])
        .width(Length::Fill)
        .style(surface)
        .into()
    }

    /// A labelled slider with an editable number: double click on the
    /// slider gives back the default value, a number typed and confirmed
    /// with Enter is applied.
    fn adjuster(&self, field: Field, s: &Settings) -> Element<'_, Message> {
        let value = field.get(s);
        let number = match &self.draft {
            Some((f, text)) if *f == field => text.clone(),
            _ => field.format(value),
        };
        column![
            row![
                text(field.label()).size(12).width(Length::Fill),
                iced::widget::text_input("", &number)
                    .on_input(move |t| Message::Draft(field, t))
                    .on_submit(Message::Commit(field))
                    .size(12)
                    .padding([2, 6])
                    .width(64)
                    .align_x(iced::alignment::Horizontal::Right),
            ]
            .align_y(Alignment::Center),
            iced::widget::mouse_area(
                slider(field.range(), value, move |v| Message::Adjust(field, v)).step(field.step())
            )
            .on_double_click(Message::Adjust(field, field.default())),
        ]
        .spacing(2)
        .into()
    }

    /// A collapsible group of adjustments, disabled (with the reason) when
    /// it does not apply.
    fn group<'a>(
        &self,
        group: Group,
        title: &'a str,
        enabled: bool,
        why: &'a str,
        content: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let open = self.open.contains(&group);
        let header = row![
            button(text(format!("{} {title}", if open { "▾" } else { "▸" })).size(14))
                .style(button::text)
                .on_press(Message::Toggle(group))
                .width(Length::Fill),
            button(text("Ripristina").size(12))
                .style(button::text)
                .on_press_maybe(enabled.then_some(Message::ResetGroup(group))),
        ]
        .align_y(Alignment::Center);
        let body: Element<'a, Message> = match (open, enabled) {
            (false, _) => Space::new().into(),
            (true, true) => content,
            (true, false) => text(why).size(12).style(text::secondary).into(),
        };
        container(column![header, body].spacing(4))
            .padding(8)
            .style(container::rounded_box)
            .into()
    }
}

// ----------------------------------------------------------------- widgets

fn heading(label: &str) -> iced::widget::Text<'_> {
    text(label).size(16)
}

fn labelled<'a>(
    label: impl iced::widget::text::IntoFragment<'a>,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    row![text(label).size(13).width(100), control]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

/// A desktop notification (with the system sound), so that whoever is at
/// the scanner hears that a page is done. Shown from a thread: it may take
/// a moment and must not hold up the interface.
fn notify(summary: &str, body: &str, error: bool) {
    let summary = if error {
        format!("⚠ {summary}")
    } else {
        summary.to_string()
    };
    if cfg!(test) {
        testing::record(format!("notifica: {summary}"));
        return;
    }
    let body = body.to_string();
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .appname("HP Scanjet 3770")
            .summary(&summary)
            .body(&body)
            .show();
    });
}

/// Opens a folder in the file manager.
fn open_folder(folder: &std::path::Path) {
    if cfg!(test) {
        testing::record(format!("cartella aperta: {}", folder.display()));
        return;
    }
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(folder).spawn();
}

/// Panels: the base background with a hairline border.
fn surface(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.base.color.into()),
        border: iced::Border {
            width: 1.0,
            color: palette.background.weak.color,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// A floating card on the canvas.
fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(6)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    width: 1.0,
                    color: palette.background.weak.color,
                    radius: 8.0.into(),
                },
                shadow: iced::Shadow {
                    color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.28),
                    offset: iced::Vector::new(0.0, 4.0),
                    blur_radius: 14.0,
                },
                ..Default::default()
            }
        })
        .into()
}

/// Sends an image to the default printer: Paint on Windows (`/pt` prints
/// without opening a window), `lp` elsewhere.
fn print_file(path: &std::path::Path) -> Result<(), String> {
    if cfg!(test) {
        testing::record(format!("stampa: {}", path.display()));
        return Ok(());
    }
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("mspaint");
        c.arg("/pt").arg(path);
        c
    } else {
        let mut c = std::process::Command::new("lp");
        c.arg(path);
        c
    };
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn pages_and_photos_are_named_for_what_they_are() {
        assert_eq!(item_name(Kind::Document, 2, 5), "pagina 3");
        assert_eq!(item_name(Kind::Photo, 1, 3), "foto 2");
        assert_eq!(item_name(Kind::Photo, 0, 1), "foto");
    }

    #[test]
    fn several_tiff_pages_are_one_file_only_as_multipage_tiff() {
        let tif = Path::new("scansione.tif");
        assert_eq!(writer(Output::TiffPages, tif, 3), Writer::Tiff);
        // "TIFF" is one image per page, like PNG.
        assert_eq!(
            writer(Output::Image(ImageFormat::Tiff), tif, 3),
            Writer::Folder
        );
        assert_eq!(
            writer(Output::Image(ImageFormat::Tiff), tif, 1),
            Writer::One
        );
        assert_eq!(writer(Output::TiffPages, tif, 1), Writer::One);
        assert_eq!(writer(Output::Pdf, Path::new("d.pdf"), 1), Writer::Pdf);
        assert_eq!(
            writer(Output::Image(ImageFormat::Png), Path::new("d.png"), 2),
            Writer::Folder
        );
    }

    #[test]
    fn the_quality_chosen_is_kept() {
        let jpeg = Output::Image(ImageFormat::Jpeg { quality: 92 });
        assert_eq!(
            image_format(Path::new("f.jpg"), jpeg),
            ImageFormat::Jpeg { quality: 92 }
        );
        // Another extension typed: its format.
        assert_eq!(image_format(Path::new("f.png"), jpeg), ImageFormat::Png);
        assert_eq!(image_format(Path::new("f"), jpeg), ImageFormat::Png);
    }

    #[test]
    fn areas_stay_on_the_glass() {
        use scanner_api::Scanner as _;
        let caps = scanner_api::fake::FakeScanner::flatbed()
            .capabilities()
            .clone();
        for &dpi in &caps.resolutions {
            let (max_w, max_h) = caps.glass_pixels(dpi);
            // An original at the bottom right corner, with the margin of
            // "Rifai" around it (1 mm past both edges).
            let area = within_glass(area_mm(dpi, 150.0, 240.0, 66.9, 58.2), dpi, &caps);
            assert!(area.x + area.width <= max_w, "{dpi} dpi: {area:?}");
            assert!(area.y + area.height <= max_h, "{dpi} dpi: {area:?}");
            assert!(area.width > 0 && area.height > 0);
            // An area inside is left alone.
            let inside = area_mm(dpi, 10.0, 10.0, 100.0, 100.0);
            assert_eq!(within_glass(inside, dpi, &caps), inside);
        }
    }

    #[test]
    fn the_nearest_resolution_offered() {
        let offered = [150, 300, 600, 1200];
        assert_eq!(nearest(&offered, 300), 300);
        assert_eq!(nearest(&offered, 2400), 1200);
        assert_eq!(nearest(&offered, 200), 150);
        assert_eq!(nearest(&offered, 225), 300, "a tie takes the larger");
        assert_eq!(nearest(&[], 200), 200);
    }

    #[test]
    fn a_preview_is_brought_to_its_resolution() {
        let image = Image {
            width: 6,
            height: 3,
            dpi: 300,
            format: Format::Rgb,
            data: (0..6 * 3 * 3).map(|i| i as u8).collect(),
        };
        let small = at_dpi(image.clone(), 200);
        assert_eq!((small.width, small.height, small.dpi), (4, 2, 200));
        assert_eq!(small.data.len(), 4 * 2 * 3);
        assert_eq!(at_dpi(image.clone(), 300), image);
    }
}
