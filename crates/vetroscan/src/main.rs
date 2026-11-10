// SPDX-License-Identifier: GPL-2.0-or-later
//! `vetroscan`: desktop scanning application (the HP Scanjet 3770, and the
//! scanners the backends of `scanner-api` reach).
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
mod i18n;
mod keys;
mod prefs;
mod separa;
mod sheet;
mod testing;
mod viewer;
mod wia;
mod worker;

use editor::{Edit, EditMsg, Editor, Section};
use histogram::Histogram;
use i18n::{tr, trf};
use iced::widget::{
    button, canvas, center, checkbox, column, container, mouse_area, opaque, pick_list,
    progress_bar, row, rule, scrollable, slider, stack, text, Space,
};
use iced::{Alignment, Element, Length, Point, Size, Subscription, Task, Theme};
use scan_imaging::adjust::{self, Adjustments};
use scan_imaging::convert;
use scan_imaging::detect::{self, Found};
use scan_imaging::film::{self, Film, Measures};
use scan_imaging::stripes;
use scan_imaging::transform::{self, Transform};
use scan_imaging::{Format, Image, Mode};
use scanner_api::{
    Area, Capabilities, ColourMatrix, DeviceInfo, DeviceStatus, LampState, Progress, ScanRequest,
    Source, SuggestedAction, Tone,
};
use separa::{Separating, SplitMsg};
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use vetroscan_export::{ImageFormat, PageSize};
use viewer::{Picture, Selection, View};
use worker::{Command, Event, Handle, Purpose};

/// Resolution of the preview of the whole glass (the lowest the scanner
/// has).
const PREVIEW_DPI: u16 = 200;

/// The app's name and version, as written in the files it saves.
fn software() -> String {
    format!("{APP_NAME} {}", env!("CARGO_PKG_VERSION"))
}

/// The app's name, as shown (title, bar, notifications). Until v0.2 it
/// was "HP Scanjet 3770", the name of the one scanner it knew.
const APP_NAME: &str = "VetroScan"; // not translated: a name

/// Above this size (bytes in memory) the interface warns before scanning.
const LARGE_SCAN: f64 = 400e6;

/// Longest side of the copy of a page on which adjustments are shown live.
const PROXY: usize = 2400;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title(APP_NAME)
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
            include_bytes!("../../../packaging/icons/vetroscan-256.png"),
            None,
        )
        .ok(),
        // Closing asks first when scans are not saved (Message::CloseRequested).
        exit_on_close_request: false,
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: "vetroscan".into(),
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

/// A way of working as the button at the top chooses it, each with its
/// own work apart (see [`Bench`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Work {
    Document,
    Photo,
    /// Foto with films in the adapter.
    Film,
}

const WORKS: [Work; 3] = [Work::Document, Work::Photo, Work::Film];

impl Work {
    fn of(kind: Kind, film: bool) -> Self {
        match (kind, film) {
            (Kind::Document, _) => Self::Document,
            (Kind::Photo, false) => Self::Photo,
            (Kind::Photo, true) => Self::Film,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Document => 0,
            Self::Photo => 1,
            Self::Film => 2,
        }
    }

    /// What it holds, and where: "3 pagine in Documento".
    fn holding(self, n: usize) -> String {
        match (self, n) {
            (Self::Document, 1) => tr("1 pagina in Documento").into(),
            (Self::Document, n) => trf("{n} pagine in Documento", &[("n", &n)]),
            (Self::Photo, 1) => tr("1 foto in Foto").into(),
            (Self::Photo, n) => trf("{n} foto in Foto", &[("n", &n)]),
            (Self::Film, 1) => tr("1 foto in Pellicola").into(),
            (Self::Film, n) => trf("{n} foto in Pellicola", &[("n", &n)]),
        }
    }
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
            Self::Rgb => tr("Colore"),
            Self::Grey => tr("Grigio"),
            Self::Lineart => tr("Bianco e nero"),
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
            Self::Fit => tr("Come la scansione"),
            Self::A4 => tr("A4 (21 × 29,7 cm)"),
            Self::A3 => tr("A3 (29,7 × 42 cm)"),
            Self::A5 => tr("A5 (14,8 × 21 cm)"),
            Self::Letter => tr("Letter (8,5 × 11\")"),
            Self::Legal => tr("Legal (8,5 × 14\")"),
            Self::Photo10x15 => tr("Foto 10 × 15 cm"),
            Self::Photo13x18 => tr("Foto 13 × 18 cm"),
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
            Self::Actual => tr("Dimensione reale"),
            Self::Page => tr("Adatta al foglio"),
            Self::Fill => tr("Riempi il foglio (taglia i bordi)"),
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
            Self::Auto => tr("Automatico"),
            Self::Portrait => tr("Verticale"),
            Self::Landscape => tr("Orizzontale"),
        })
    }
}

impl From<Sheet> for vetroscan_export::Orientation {
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
            LAMP_SCANNER => f.write_str(&trf(
                "Dopo {m} minuti (come il driver HP)",
                &[("m", &LAMP_SCANNER)],
            )),
            m => f.write_str(&trf("Dopo {m} minuti", &[("m", &m)])),
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
    samples: Arc<Vec<vetroscan_export::Sample>>,
    result: Result<vetroscan_export::Planned, String>,
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
        format!("{} MB", i18n::decimal(text))
    }
}

/// The encoding of a quality percentage.
fn e_base(percent: u8) -> vetroscan_export::Encoding {
    vetroscan_export::Encoding::from_percent(percent)
}

/// What the limit cost: "3 pagine su 10 alleggerite, fino a 200 dpi e
/// JPEG 65" (none when no page was).
fn lightened(
    encodings: &[vetroscan_export::Encoding],
    base: vetroscan_export::Encoding,
) -> Option<String> {
    let changed: Vec<&vetroscan_export::Encoding> = encodings
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
            tr("la pagina alleggerita").to_string()
        } else {
            trf("tutte le {n} pagine alleggerite", &[("n", &n)])
        }
    } else if n == 1 {
        trf(
            "1 pagina su {total} alleggerita",
            &[("total", &encodings.len())],
        )
    } else {
        trf(
            "{n} pagine su {total} alleggerite",
            &[("n", &n), ("total", &encodings.len())],
        )
    };
    Some(match dpi {
        Some(dpi) => trf(
            "{of}, fino a {dpi} dpi e JPEG {quality}",
            &[("of", &of), ("dpi", &dpi), ("quality", &quality)],
        ),
        None => trf(
            "{of}, fino a JPEG {quality}",
            &[("of", &of), ("quality", &quality)],
        ),
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
            Self::System => tr("Sistema"),
            Self::Light => tr("Chiaro"),
            Self::Dark => tr("Scuro"),
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
    /// Take away the stripes the scanner leaves across its scans of the
    /// glass, where it declares them (`Capabilities::stripes`).
    destripe: bool,
    /// Scan the glass at twice the resolution chosen and bring it back to
    /// it: finer, less noise and fainter stripes, in about twice the time.
    double: bool,
    /// Films: smooth the noise of the colour of their picture, as much as
    /// measured on it (`scan_imaging::chroma`).
    colour_noise: bool,
    /// Films: correct the picture's colours with the scanner's matrix for
    /// the film, where it declares one (`Capabilities::colour`).
    scanner_colour: bool,
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
            destripe: false,
            double: false,
            colour_noise: false,
            scanner_colour: false,
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
    fn layout(&self, own: Option<PageLayout>, focus: [f32; 2]) -> vetroscan_export::Layout {
        let l = own.unwrap_or(self.page_layout());
        vetroscan_export::Layout {
            size: self.pdf_size.into(),
            orientation: l.sheet.into(),
            placement: match l.fit {
                PdfFit::Actual => vetroscan_export::Placement::Actual,
                PdfFit::Page => vetroscan_export::Placement::FitPage,
                PdfFit::Fill => vetroscan_export::Placement::Fill,
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

    /// Films in the adapter: a 35 mm frame is small, 1200 dpi gives it
    /// about 2 megapixels; the 16-bit samples for turning a negative into
    /// its picture; each frame found its own image.
    fn film() -> Self {
        Self {
            dpi: 1200,
            colour_noise: true,
            scanner_colour: true,
            ..Self::photo()
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
            destripe: Some(self.destripe),
            double: Some(self.double),
            colour_noise: Some(self.colour_noise),
            scanner_colour: Some(self.scanner_colour),
            roll_base: None,
            slides: false,
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
        Self::from_stored_on(
            kind,
            match kind {
                Kind::Document => Settings::document(),
                Kind::Photo => Settings::photo(),
            },
            m,
        )
    }

    /// Settings from the stored form for `kind`, `base` for what does not
    /// make sense.
    fn from_stored_on(kind: Kind, base: Settings, m: &prefs::StoredMode) -> Self {
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
            destripe: m.destripe.unwrap_or(base.destripe),
            double: m.double.unwrap_or(base.double),
            colour_noise: m.colour_noise.unwrap_or(base.colour_noise),
            scanner_colour: m.scanner_colour.unwrap_or(base.scanner_colour),
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
                tr("Testo per OCR (bianco e nero)"),
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
                tr("Documento a colori"),
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
                tr("Ricevuta / scontrino"),
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
                tr("Rivista / giornale"),
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
                tr("Archivio (600 dpi, senza perdita)"),
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
                tr("Foto standard"),
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
                tr("Archivio foto (1200 dpi, TIFF)"),
                Settings {
                    dpi: 1200,
                    adjust: Adjustments { sharpen: 0.0, ..a },
                    output: Output::Image(ImageFormat::Tiff),
                    ..f
                },
            ),
            (
                tr("Foto sbiadita (recupero colori)"),
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
                tr("Foto in bianco e nero"),
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
                tr("Foto stampata su rivista"),
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
            Self::AddPage => tr("Aggiungi una pagina"),
            Self::RedoLast => tr("Rifai l'ultima pagina"),
            Self::SaveDocument => tr("Salva il documento e apri la cartella"),
            Self::ScanPhoto => tr("Scansiona la foto"),
            Self::Photocopy => tr("Fotocopia (scansiona e stampa)"),
            Self::OpenFolder => tr("Apri la cartella"),
            Self::Nothing => tr("Niente"),
        })
    }
}

/// Bits per sample of the scan, for the choice in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Depth(u8);

impl std::fmt::Display for Depth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&trf("{bits} bit per canale", &[("bits", &self.0)]))
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
            Self::Last => tr("L'ultimo usato"),
            Self::Document => tr("Documento"),
            Self::Photo => tr("Foto"),
        })
    }
}

/// What lies in the film adapter's holder, as Pellicola asks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holder {
    Negatives,
    Slides,
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Negatives => tr("Negativi"),
            Self::Slides => tr("Diapositive"),
        })
    }
}

// ------------------------------------------------------------------ pages

/// How the interface names item `i` of `count`: "pagina 2", "foto 3", or
/// just "foto" when it is the only one.
fn item_name(kind: Kind, i: usize, count: usize) -> String {
    match kind {
        Kind::Document => trf("pagina {n}", &[("n", &(i + 1))]),
        Kind::Photo if count > 1 => trf("foto {n}", &[("n", &(i + 1))]),
        Kind::Photo => tr("foto").into(),
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
    /// Who made that scanner and its model, for the metadata of RAW files.
    scanner: Arc<(String, String)>,
}

impl Raw {
    fn new(image: Image, caps: &Capabilities) -> Self {
        let proxy = shrink(&image, PROXY);
        Self {
            full: Arc::new(image),
            proxy: Arc::new(proxy),
            tone: caps.tone,
            scanner: Arc::new((caps.manufacturer.clone(), caps.model.clone())),
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
    /// A scan kept whole: what "Separa" needs to find its originals in it,
    /// without scanning again.
    whole: Option<Whole>,
    /// The chip's 16-bit samples (same size as `raw.full`), when the scan
    /// was made for a RAW file.
    raw16: Option<Arc<Vec<u16>>>,
    /// Turns, straightening, crop and size, worked out from the samples
    /// each time (the samples stay as scanned).
    transform: Transform,
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
            whole: None,
            raw16: None,
            transform: Transform::default(),
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
            transform: self.transform,
        }
    }

    fn set_edit(&mut self, e: Edit) {
        self.transform = e.transform;
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
    fn pdf_sample(&self, shared: &Settings) -> vetroscan_export::Sample {
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
        vetroscan_export::Sample::of(&small, w, h, dpi)
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
    Colour,
    Tone,
    Levels,
    Curve,
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
            Self::Temperature => tr("Temperatura (freddo ↔ caldo)"),
            Self::Noise => tr("Riduzione rumore"),
            Self::Tint => tr("Tinta (verde ↔ magenta)"),
            Self::Exposure => tr("Esposizione (stop)"),
            Self::Highlights => tr("Luci"),
            Self::Shadows => tr("Ombre"),
            Self::Whites => tr("Bianchi"),
            Self::Blacks => tr("Neri"),
            Self::Vibrance => tr("Vividezza"),
            Self::Black => tr("Punto del nero"),
            Self::White => tr("Punto del bianco"),
            Self::Gamma => tr("Gamma (mezzitoni)"),
            Self::Brightness => tr("Luminosità"),
            Self::Contrast => tr("Contrasto"),
            Self::Saturation => tr("Saturazione"),
            Self::Sharpen => tr("Nitidezza (1,5 = come HP)"),
            Self::Threshold => tr("Soglia (più alta = più nero)"),
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
            i18n::decimal(format!("{v:.2}"))
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

/// A field of millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MmField {
    /// The area to scan: left, top, width, height on the glass.
    Area(usize),
    /// The page on its sheet: from the left edge, from the top edge.
    Offset(usize),
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
    /// The language of the interface.
    Language(i18n::LangChoice),
    Kind(Kind),
    /// Foto with films in the adapter ("Pellicola").
    Film,
    Tab(Tab),
    Pane(Pane),
    Undo,
    Redo,
    CurveChannel(CurveChannel),
    /// New points of the curve of the channel shown, and the point chosen
    /// after (moved by the arrows).
    CurveSet(Vec<[f32; 2]>, Option<usize>),
    /// A point of the curve chosen, or none.
    CurvePoint(Option<usize>),
    /// A millimetre field typed: the area to scan, or where the page is on
    /// its sheet.
    MmInput(MmField, String),
    /// Enter in a millimetre field: the number typed is done.
    MmSubmit,
    /// The position of the page shown on its sheet given to every page.
    PageFocusToAll,
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
    /// Open the folder of the scans in the file manager (made if missing).
    ShowFolder,
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
    /// The colour mode of the page shown (in "Regola"; the next scans
    /// keep theirs).
    PageColour(Colour),
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
    /// "Togli le righe dello scanner".
    Destripe(bool),
    /// "Scansiona a ... dpi e riduci".
    Double(bool),
    /// "Riduci il rumore di colore" (films).
    ColourNoise(bool),
    /// "Correggi i colori dello scanner" (films it declares a matrix for).
    ScannerColour(bool),
    /// "Blocca la base": the base of the last strip becomes the roll's.
    LockBase,
    /// "Nuovo rullino": each strip measures its own base again.
    NewRoll,
    /// Pellicola: slides in the holder (true) or negatives.
    Slides(bool),
    /// "Misura le righe": a strip of a white sheet at each resolution.
    MeasureStripes,
    /// Back to the stripes the scanner's model declares.
    ForgetStripes,
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
    /// "Separa": the originals found in a page scanned whole shown on it,
    /// to correct before each is cut out as a page of its own, without
    /// scanning again.
    PageSplit(usize),
    /// "Separa" open: its areas, cutting them out or leaving the scan whole.
    Split(SplitMsg),
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

/// `area` cut to a bed of `(max_w, max_h)` pixels at its resolution (the
/// glass, or the film adapter's area): the margin around an original near
/// the edge, or the rounding of position and size apart, would ask the
/// scanner for a pixel beyond it, and it refuses the scan.
fn within(area: Area, (max_w, max_h): (u32, u32)) -> Area {
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

/// The highest resolution "Misura le righe" scans at: above it a strip
/// of the glass is large and slow, and no stripes were measured there.
const MEASURE_MAX_DPI: u16 = 1200;

/// The height of the strip "Misura le righe" scans, millimetres.
const MEASURE_STRIP_MM: f64 = 25.0;

/// "Misura le righe" running: a strip at each resolution in turn.
#[derive(Debug, Clone)]
struct Measuring {
    /// The resolutions still to scan, the next last.
    left: Vec<u16>,
    /// The one being scanned.
    current: u16,
    /// How many in all.
    total: usize,
    /// What was found so far.
    done: Vec<prefs::StoredStripes>,
}

/// A scan kept whole, as "Separa" needs it to cut its originals out
/// afterwards.
#[derive(Debug, Clone)]
struct Whole {
    /// The area it covered (pixels at its resolution): where the
    /// originals were, for "Rifai" on each.
    area: Area,
    /// Films: the scan as the scanner saw it, before it became the
    /// positive, at [`PREVIEW_DPI`]; the frames are found on that.
    transmission: Option<Arc<Image>>,
    /// Which film it was, for finding its frames again.
    film: Film,
}

/// Largest single pass for several originals; beyond it each is scanned
/// on its own.
const ONE_PASS: f64 = 700e6;

/// Cut inside the edge of a found original (millimetres): its edge is
/// blurred and a sliver of the lid would show.
const INSET_MM: f32 = 0.4;

/// The work of a way of working put aside while another is shown: its
/// pages and what goes with them, found again as they were when it comes
/// back. The way of working shown has its work in [`App`]'s own fields.
struct Bench {
    pages: Vec<Page>,
    current: Option<usize>,
    /// A rescan waiting for a choice.
    redone: Option<(usize, Page)>,
    unsaved: bool,
    /// Ctrl+Z and Ctrl+Y of its settings and pages.
    undo: Vec<(Target, Settings)>,
    redo_stack: Vec<(Target, Settings)>,
    tab: Tab,
    result_view: View,
}

impl Bench {
    /// Nothing done yet: the glass's preview first, a film's scans (they
    /// have no preview).
    fn new(work: Work) -> Self {
        Self {
            pages: Vec::new(),
            current: None,
            redone: None,
            unsaved: false,
            undo: Vec::new(),
            redo_stack: Vec::new(),
            tab: if work == Work::Film {
                Tab::Result
            } else {
                Tab::Preview
            },
            result_view: View::default(),
        }
    }
}

/// A scan of films developed ([`App::develop`]): the picture at 8 bits
/// and at 16, its measures and, for negatives, the base found between its
/// frames.
type Developed = (Image, Option<Vec<u16>>, Option<Measures>, Option<[f32; 3]>);

/// A film base read back from the preferences: a transmission per channel,
/// above nothing and at most the lamp's.
fn valid_base(base: [f32; 3]) -> bool {
    base.iter().all(|b| b.is_finite() && *b > 0.0 && *b <= 1.0)
}

/// What belongs to where the scans come from, the glass or the adapter's
/// area: its preview, the area chosen and the originals found on it.
/// Documento and Foto share the glass's.
#[derive(Default)]
struct Bed {
    preview: Option<(Raw, Image, Picture)>,
    preview_view: View,
    selection: Option<Selection>,
    found: Vec<Found>,
    film_preview: Option<Image>,
    measures: Option<Measures>,
}

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
    /// In Foto, films in the adapter instead of the glass ("Pellicola").
    film: bool,
    /// The work of the ways of working not shown, by [`Work::index`] (the
    /// slot of the one shown is empty: its work is in the fields below).
    benches: [Bench; 3],
    /// The preview, area and originals of the source not in use (the
    /// adapter's while on the glass, and the other way round).
    other_bed: Bed,
    /// The settings for films, apart from the photos' ones.
    film_settings: Settings,
    /// The preview of films as the scanner gave it (their transmission,
    /// pixels of the preview), where the frames are found.
    film_preview: Option<Image>,
    /// The scan running was asked at twice the resolution chosen
    /// (`Settings::double`): it is halved when it comes.
    doubled: bool,
    /// How the films of the preview become their picture: the full scans
    /// take the same measures, for the same picture.
    measures: Option<Measures>,
    /// "Diapositive" chosen in Pellicola: slides in the holder rather than
    /// negatives (asked only of an adapter that takes both, see
    /// `film_kind`).
    slides: bool,
    /// The base of the roll in the adapter, locked: every strip scanned is
    /// measured from it, so the frames of one roll come out alike (their
    /// highlights and black stay their own). None: each strip its own.
    roll_base: Option<[f32; 3]>,
    /// The base measured in the gaps between the frames of the last strip
    /// developed, what "Blocca la base" locks.
    strip_base: Option<[f32; 3]>,
    /// "Pellicola" was chosen with the adapter not plugged in, as last
    /// heard: switch when the scanner's state says it is.
    film_wanted: bool,
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
    /// The stripes measured on each scanner ("Misura le righe").
    measured: std::collections::BTreeMap<String, Vec<prefs::StoredStripes>>,
    /// "Misura le righe" running.
    measuring: Option<Measuring>,
    /// The Epson 4490's firmware chosen in the preferences file (not in the
    /// interface yet: kept as it is when the preferences are saved).
    epson_firmware: String,
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
    /// The point of the curve clicked last: the arrows move it.
    curve_selected: Option<usize>,
    /// A millimetre field being typed, and what is in it.
    mm_draft: Option<(MmField, String)>,
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
    /// "Separa" open on a page scanned whole.
    separating: Option<Separating>,
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
    /// The language chosen (automatic: the system's).
    language: i18n::LangChoice,
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
                film: false,
                benches: WORKS.map(Bench::new),
                other_bed: Bed::default(),
                film_settings: Settings::from_stored_on(
                    Kind::Photo,
                    Settings::film(),
                    &stored.film,
                )
                .fresh_look(),
                film_preview: None,
                doubled: false,
                measures: None,
                slides: stored.film.slides,
                roll_base: stored.film.roll_base.filter(|b| valid_base(*b)),
                strip_base: None,
                film_wanted: false,
                tab: Tab::Preview,
                pane: Pane::Scan,
                start_mode,
                custom_folder: (!stored.folder.is_empty()).then(|| PathBuf::from(&stored.folder)),
                document_name: stored.document_name.clone(),
                photo_name: stored.photo_name.clone(),
                button_actions: stored.buttons.clone(),
                measured: stored.stripes.clone(),
                measuring: None,
                epson_firmware: stored.epson_firmware.clone(),
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
                open: vec![Group::Colour, Group::Tone, Group::Levels],
                eyedropper: false,
                before: false,
                busy: None,
                pages: Vec::new(),
                current: None,
                redo: None,
                redo_auto: false,
                redone: None,
                note: tr("Pronto: fai un'anteprima, scegli l'area e scansiona").into(),
                draft: None,
                curve_channel: CurveChannel::All,
                curve_drag: None,
                curve_selected: None,
                mm_draft: None,
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
                separating: None,
                editor: None,
                edit_section: Section::Frame,
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
                language: {
                    let choice = i18n::LangChoice::from_key(&stored.language);
                    i18n::set(choice.resolve());
                    choice
                },
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
        let base = vetroscan_export::Encoding::from_percent(s.pdf_quality);
        let limit = s.pdf_limit_kb.map(|kb| u64::from(kb) * 1024);
        Task::perform(
            async move {
                let samples = measured
                    .unwrap_or_else(|| Arc::new(pages.iter().map(|p| p.pdf_sample(&s)).collect()));
                let result = vetroscan_export::compress::plan(&samples, base, limit)
                    .map_err(|e| e.to_string());
                Arc::new(Estimated {
                    key,
                    samples,
                    result,
                })
            },
            Message::Estimated,
        )
    }

    /// The save dialog's file type and title: whether the pages (or
    /// photos) go into one file or one file each, so that a dialog asking
    /// for one name does not look like it saves one scan only.
    fn save_labels(&self) -> (&'static str, String) {
        let n = self.pages.len();
        let document = self.kind == Kind::Document;
        let one_file = matches!(self.settings().output, Output::Pdf | Output::TiffPages);
        match (n, one_file, document) {
            (0 | 1, _, true) => (tr("Documento"), tr("Salva la pagina").into()),
            (0 | 1, _, false) => (tr("Immagine"), tr("Salva la foto").into()),
            (n, true, true) => (
                tr("Documento"),
                trf("Salva le {n} pagine in un solo file", &[("n", &n)]),
            ),
            (n, true, false) => (
                tr("Documento"),
                trf("Salva le {n} foto in un solo file", &[("n", &n)]),
            ),
            (n, false, true) => (
                tr("Un file per pagina"),
                trf(
                    "Salva le {n} pagine: un file per pagina, con il numero in fondo al nome",
                    &[("n", &n)],
                ),
            ),
            (n, false, false) => (
                tr("Un file per foto"),
                trf(
                    "Salva le {n} foto: un file per foto, con il numero in fondo al nome",
                    &[("n", &n)],
                ),
            ),
        }
    }

    /// What the question says is lost: "Le 3 pagine acquisite non sono
    /// salvate e andranno perse."
    fn lost_sentence(&self, n: usize) -> String {
        match (self.kind, n) {
            (Kind::Document, 1) => tr("La pagina acquisita non è salvata e andrà persa.").into(),
            (Kind::Photo, 1) => tr("La foto acquisita non è salvata e andrà persa.").into(),
            (Kind::Document, n) => trf(
                "Le {n} pagine acquisite non sono salvate e andranno perse.",
                &[("n", &n)],
            ),
            (Kind::Photo, n) => trf(
                "Le {n} foto acquisite non sono salvate e andranno perse.",
                &[("n", &n)],
            ),
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
                | Message::CurveSet(..)
                | Message::Descreen(_)
                | Message::Adaptive(_)
                | Message::PageColour(_)
                | Message::OwnLook(_)
                | Message::LookApplyToAll
                | Message::PresetApply(_)
                | Message::Adjust(..)
                | Message::Commit(_)
                | Message::AutoLevels
                | Message::AutoThreshold
                | Message::Picked(_)
                | Message::ResetGroup(_)
                | Message::PageRotate(..)
                | Message::PageMove(..)
                | Message::PageDelete(_)
                | Message::PageDuplicate(_)
                | Message::PdfOwnLayout(_)
                | Message::PdfApplyToAll
                | Message::PageFocus(_)
                | Message::PageFocusToAll
                | Message::MmInput(MmField::Offset(_), _)
                | Message::AcceptRedo
                | Message::Split(SplitMsg::Apply)
        )
    }

    fn settings(&self) -> &Settings {
        if self.filming() {
            return &self.film_settings;
        }
        match self.kind {
            Kind::Document => &self.document,
            Kind::Photo => &self.photo,
        }
    }

    fn settings_mut(&mut self) -> &mut Settings {
        if self.filming() {
            return &mut self.film_settings;
        }
        match self.kind {
            Kind::Document => &mut self.document,
            Kind::Photo => &mut self.photo,
        }
    }

    /// Whether the scans are of films in the adapter: Foto with
    /// "Pellicola" chosen. (Its work stays there to be saved with a
    /// scanner without the adapter; such a scanner does not scan in it,
    /// see `start`.)
    fn filming(&self) -> bool {
        self.kind == Kind::Photo && self.film
    }

    /// The way of working shown.
    fn work(&self) -> Work {
        Work::of(self.kind, self.film)
    }

    /// Whether a way of working put aside has pages.
    fn has_work(&self, work: Work) -> bool {
        work != self.work() && !self.benches[work.index()].pages.is_empty()
    }

    /// The work not saved, in every way of working: which, and how many
    /// pages or photos.
    fn unsaved_work(&self) -> Vec<(Work, usize)> {
        WORKS
            .into_iter()
            .filter_map(|w| {
                let (n, unsaved) = if w == self.work() {
                    (self.pages.len(), self.unsaved)
                } else {
                    let b = &self.benches[w.index()];
                    (b.pages.len(), b.unsaved)
                };
                (n > 0 && unsaved).then_some((w, n))
            })
            .collect()
    }

    /// What is lost closing now, if anything: the sentence of the question.
    fn lost_closing(&self) -> Option<String> {
        match self.unsaved_work().as_slice() {
            [] => None,
            [(w, n)] if *w == self.work() => Some(self.lost_sentence(*n)),
            all => {
                let list: Vec<String> = all.iter().map(|(w, n)| w.holding(*n)).collect();
                Some(trf(
                    "Ancora da salvare: {list}. Chiudendo senza salvare, il lavoro va perso.",
                    &[("list", &list.join(", "))],
                ))
            }
        }
    }

    /// Puts the work shown aside, in its slot.
    fn park(&mut self) {
        let bench = Bench {
            pages: std::mem::take(&mut self.pages),
            current: self.current.take(),
            redone: self.redone.take(),
            unsaved: std::mem::take(&mut self.unsaved),
            undo: std::mem::take(&mut self.undo),
            redo_stack: std::mem::take(&mut self.redo_stack),
            tab: self.tab,
            result_view: self.result_view,
        };
        self.benches[self.work().index()] = bench;
    }

    /// Takes up the work of the way of working shown, as it was left.
    fn take_up(&mut self) {
        let work = self.work();
        let b = std::mem::replace(&mut self.benches[work.index()], Bench::new(work));
        self.pages = b.pages;
        self.current = b.current;
        self.redone = b.redone;
        self.unsaved = b.unsaved;
        self.undo = b.undo;
        self.redo_stack = b.redo_stack;
        self.tab = b.tab;
        self.result_view = b.result_view;
    }

    /// The glass and the adapter's area change places.
    fn swap_bed(&mut self) {
        let other = std::mem::take(&mut self.other_bed);
        self.other_bed = Bed {
            preview: std::mem::replace(&mut self.preview, other.preview),
            preview_view: std::mem::replace(&mut self.preview_view, other.preview_view),
            selection: std::mem::replace(&mut self.selection, other.selection),
            found: std::mem::replace(&mut self.found, other.found),
            film_preview: std::mem::replace(&mut self.film_preview, other.film_preview),
            measures: std::mem::replace(&mut self.measures, other.measures),
        };
        self.mm_draft = None;
    }

    /// The scanner's correction of the colours of the film in the
    /// adapter, where it declares one and the user leaves it on.
    fn colour_correction(&self) -> Option<ColourMatrix> {
        let source = self.source();
        self.caps
            .colour_at(source)
            .filter(|_| source != Source::Glass && self.settings().scanner_colour)
    }

    /// Where the scans come from.
    fn source(&self) -> Source {
        if !self.filming() {
            return Source::Glass;
        }
        match self.film_kind() {
            Film::Slide => Source::Slide,
            Film::Negative => Source::Negative,
        }
    }

    /// The film in the adapter: slides if chosen and the adapter takes
    /// them, or if it takes nothing else; else negatives.
    fn film_kind(&self) -> Film {
        let slides = self.caps.offers(Source::Slide);
        if slides && (self.slides || !self.caps.offers(Source::Negative)) {
            Film::Slide
        } else {
            Film::Negative
        }
    }

    /// Whether the scanner's adapter takes any film.
    fn offers_films(&self) -> bool {
        self.caps.offers(Source::Negative) || self.caps.offers(Source::Slide)
    }

    /// "Negativi" or "Diapositive" chosen. Measures and frames found were
    /// of the other film: they go (the photos scanned stay). Not during a
    /// scan.
    fn set_slides(&mut self, slides: bool) {
        if self.busy.is_some() || self.slides == slides {
            return;
        }
        self.slides = slides;
        if !self.filming() {
            return;
        }
        self.measures = None;
        self.found.clear();
        self.film_preview = None;
        self.note = match self.film_kind() {
            Film::Slide => tr("Diapositive nel supporto: ognuna diventa un'immagine"),
            Film::Negative => tr("Negativi nel supporto: ogni fotogramma diventa un'immagine"),
        }
        .into();
    }

    /// "Negativi" or "Diapositive", for an adapter that takes both.
    fn holder_view(&self) -> Option<Element<'_, Message>> {
        if !(self.caps.offers(Source::Slide) && self.caps.offers(Source::Negative)) {
            return None;
        }
        let chosen = match self.film_kind() {
            Film::Slide => Holder::Slides,
            Film::Negative => Holder::Negatives,
        };
        // During a scan the choice stays (see `set_slides`).
        Some(labelled(
            tr("Nel supporto"),
            pick_list(
                [Holder::Negatives, Holder::Slides],
                Some(chosen),
                |h: Holder| Message::Slides(h == Holder::Slides),
            )
            .width(Length::Fill)
            .into(),
        ))
    }

    /// Whether the scanner says its film adapter is plugged in.
    fn adapter_plugged(&self) -> bool {
        matches!(&self.status, Some(Ok(s)) if s.adapter == Some(true))
    }

    /// The samples of a scan as the app keeps them, with the tone curve
    /// they are drawn with: the scanner's, or for films the positives'.
    fn raw(&self, image: Image) -> Raw {
        let mut raw = Raw::new(image, &self.caps);
        if self.filming() {
            raw.tone = film::TONE;
        }
        raw
    }

    /// "Rullino", films only: the base of the roll locked and shown, or
    /// the base of the last strip to lock.
    fn roll_view(&self) -> Element<'_, Message> {
        let base = |b: [f32; 3]| {
            let [r, g, b] = b.map(|v| i18n::decimal(format!("{v:.3}")));
            trf(
                "Base: R {r} · G {g} · B {b}",
                &[("r", &r), ("g", &g), ("b", &b)],
            )
        };
        let idle = self.busy.is_none();
        let mut block = column![text(tr("Rullino")).size(13)].spacing(4);
        match self.roll_base {
            Some(b) => {
                block = block
                    .push(text(base(b)).size(12))
                    .push(
                        text(tr(
                            "Bloccata: ogni striscia usa questa base e misura da sé solo \
                             luci e neri, così i fotogrammi dello stesso rullino vengono uguali.",
                        ))
                        .size(11),
                    )
                    .push(
                        button(text(tr("Nuovo rullino")).size(13))
                            .style(button::secondary)
                            .on_press_maybe(idle.then_some(Message::NewRoll)),
                    );
            }
            None => {
                block = block.push(
                    text(tr(
                        "Ogni striscia misura la sua base negli spazi tra i fotogrammi. \
                         Bloccala per usarla in tutte le strisce dello stesso rullino.",
                    ))
                    .size(11),
                );
                if let Some(b) = self.strip_base {
                    block = block.push(text(base(b)).size(12));
                }
                block = block.push(
                    button(text(tr("Blocca la base dell'ultima striscia")).size(13))
                        .style(button::secondary)
                        .on_press_maybe(
                            (idle && self.strip_base.is_some()).then_some(Message::LockBase),
                        ),
                );
            }
        }
        block.into()
    }

    /// A scan of `film` turned into its picture, with `measured` (the
    /// preview's) or measures of its own (a negative's from the `roll`'s
    /// base if locked), its colours corrected with `correction` if given
    /// (the scanner's, for this film): the picture at 8 bits and at 16, the measures and,
    /// for negatives, the base found in the gaps between its frames (none
    /// with a base given). Without film to measure, the scan as it is.
    fn develop(
        film: Film,
        image: Image,
        samples16: Option<Vec<u16>>,
        measured: Option<Measures>,
        roll: Option<[f32; 3]>,
        correction: Option<ColourMatrix>,
        colour_noise: bool,
    ) -> Developed {
        let (w, h) = (image.width, image.height);
        let deep: Cow<[u16]> = match &samples16 {
            Some(s) if s.len() == w * h * 3 => Cow::Borrowed(s),
            _ => Cow::Owned(image.data.iter().map(|&v| u16::from(v) * 257).collect()),
        };
        let (measures, gaps) = match (measured, film) {
            (Some(m), _) => (Some(m), None),
            (None, Film::Negative) => {
                match film::measure_strip_with(&deep, w, h, image.dpi, roll) {
                    Some(s) => (Some(Measures::Negative(s.negative)), s.gaps),
                    None => (None, None),
                }
            }
            (None, Film::Slide) => (
                film::measure_slides(&deep, w, h, image.dpi).map(|(s, _)| Measures::Slide(s)),
                None,
            ),
        };
        let Some(m) = measures else {
            return (image, samples16, None, None);
        };
        let mut positive = m.positive(&deep);
        // Before the noise: what the matrix spreads is smoothed with it.
        if let Some(c) = correction {
            c.apply(&mut positive);
        }
        // Measured on the picture itself: as much as it needs, or none.
        if colour_noise {
            scan_imaging::chroma::reduce_auto(&mut positive, w);
        }
        let picture = Image {
            data: positive.iter().map(|&v| (v >> 8) as u8).collect(),
            ..image
        };
        (picture, Some(positive), Some(m), gaps)
    }

    /// Switches to `kind`, with films in the adapter or not (only Foto has
    /// them). Each way of working keeps its own work: the one left is put
    /// aside as it is, nothing thrown away and nothing made to look
    /// otherwise, and the other's taken up as it was left.
    fn set_mode(&mut self, kind: Kind, film: bool) -> Task<Message> {
        let film = film && kind == Kind::Photo;
        // Not during a scan: its pages go where the mode says.
        if (self.kind == kind && self.film == film) || self.busy.is_some() {
            return Task::none();
        }
        self.close_editor(true);
        // The limit typed belongs to the other way of working.
        self.limit_draft = None;
        self.park();
        if self.film != film {
            // The glass and the adapter's area: each its own preview, area
            // chosen and originals found.
            self.swap_bed();
        }
        self.kind = kind;
        self.film = film;
        self.take_up();
        self.redo = None;
        self.redo_auto = false;
        self.drag = None;
        self.draft = None;
        self.last_edit = None;
        // Renders running and pages waiting to be drawn were of the other
        // work (its Ctrl+Z went with it).
        self.epoch = self.epoch.wrapping_add(1);
        self.pdf_rev += 1;
        self.dirty_page = true;
        // The preview and the pages that follow the shared look are drawn
        // with this mode's.
        self.shared_changed();
        Task::none()
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

    /// Whether the panel's changes go to the page shown alone: a page
    /// with a look of its own, or any photo (in Foto and Pellicola each
    /// photo is an original of its own, and takes a look of its own at
    /// its first change).
    fn editing_own(&self) -> bool {
        self.editing_page()
            && (self.kind == Kind::Photo || self.shown().is_some_and(|p| p.own.is_some()))
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
            (self.page_target(), self.active())
        } else {
            (Target::Preview, *self.settings())
        };
        self.undo.push(entry);
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo_stack.clear();
    }

    /// The Ctrl+Z target of the page shown: `Index(i)` for page i, `Page`
    /// for a new scan waiting for a choice.
    fn page_target(&self) -> Target {
        match self.shown_key() {
            Some(i) => Target::Index(i),
            None => Target::Page,
        }
    }

    /// Saves the page shown for Ctrl+Z, whether or not it has a look of
    /// its own (its colour mode is always its own).
    fn remember_page(&mut self) {
        self.last_edit = None;
        self.undo.push((self.page_target(), self.active()));
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Saves the shared settings for Ctrl+Z, whatever the panel shows.
    fn remember_shared(&mut self) {
        self.last_edit = None;
        self.undo.push((Target::Preview, *self.settings()));
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
                let photo = self.kind == Kind::Photo;
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
                // Put back as the shared look is: a page following it goes
                // on following it, and a photo follows it again (its own
                // look came with a change).
                let look = Look::of(&settings);
                if look != Look::of(&shared) {
                    page.own = Some(look);
                } else if photo {
                    page.own = None;
                } else if page.own.is_some() {
                    page.own = Some(look);
                }
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

    /// A change to what the panel edits. On a page with its own look (or
    /// a photo, which takes one), that look; otherwise the shared
    /// settings, which every page following them shows. The colour mode
    /// of a page shown stays its own.
    fn apply(&mut self, f: impl Fn(&mut Settings)) {
        if self.editing_page() {
            let shared = *self.settings();
            let own = self.editing_own();
            let Some(page) = self.shown_mut() else {
                return;
            };
            let mut s = page.effective(&shared);
            f(&mut s);
            let look = Look::of(&s);
            let same = look == Look::of(&shared);
            page.settings.colour = s.colour;
            if page.own.is_some() || (own && !same) {
                page.own = Some(look);
                self.dirty_page = true;
            } else if same {
                // Only its colour mode changed.
                self.dirty_page = true;
            } else {
                look.put(self.settings_mut());
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
        let settings = page.effective(&shared);
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

    /// Size of the glass in preview pixels (of the adapter's area, for
    /// films).
    fn glass(&self) -> Size {
        let (w, h) = self.bed_pixels(PREVIEW_DPI);
        Size::new(w as f32, h as f32)
    }

    /// The glass, or the adapter's area for films, in pixels at `dpi`.
    fn bed_pixels(&self, dpi: u16) -> (u32, u32) {
        self.caps
            .bed_pixels(self.source(), dpi)
            .unwrap_or_else(|| self.caps.glass_pixels(dpi))
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
        nearest(
            self.caps.resolutions_for(self.source()),
            self.settings().dpi,
        )
    }

    /// The resolution of the preview: the scanner's nearest to
    /// [`PREVIEW_DPI`] (the preview is brought to that afterwards).
    fn preview_dpi(&self) -> u16 {
        nearest(self.caps.resolutions_for(self.source()), PREVIEW_DPI)
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
        // A film becomes its picture from the 16-bit samples.
        let film = self.filming();
        let depth = if self.depth() == 16 || ((raw || film) && self.offers_16bit()) {
            16
        } else {
            8
        };
        let area = within(area_mm(dpi, x, y, w, h), self.bed_pixels(dpi));
        ScanRequest {
            dpi,
            source: self.source(),
            area,
            depth,
        }
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
        Self::bytes(&self.job_request(job), self.dpi()) * self.doubling_factor()
    }

    /// Lines after which the stripes of a scan from `source` at `dpi`
    /// repeat: on the glass as measured on this scanner ("Misura le
    /// righe"), else as its model declares them for that source.
    fn stripes_at(&self, source: Source, dpi: u16) -> Option<usize> {
        let measured = self
            .measured
            .get(&self.caps.name())
            .and_then(|m| m.iter().find(|s| s.dpi == dpi))
            .filter(|_| source == Source::Glass);
        match measured {
            Some(s) => (s.repeat > 0).then_some(usize::from(s.repeat)),
            None => self.caps.stripes_at(source, dpi),
        }
    }

    /// "Misura le righe": the resolutions it scans at, smallest first.
    fn measure_resolutions(&self) -> Vec<u16> {
        self.caps
            .resolutions_for(Source::Glass)
            .iter()
            .copied()
            .filter(|&d| d <= MEASURE_MAX_DPI)
            .collect()
    }

    /// Scans the next strip of "Misura le righe", or ends it: what was
    /// found is kept for this scanner and said.
    fn next_measure(&mut self) {
        let Some(measuring) = &mut self.measuring else {
            return;
        };
        let Some(dpi) = measuring.left.pop() else {
            let done = self.measuring.take().map(|m| m.done).unwrap_or_default();
            let said: Vec<String> = done
                .iter()
                .map(|s| {
                    if s.repeat == 0 {
                        trf("{dpi} dpi nessuna", &[("dpi", &s.dpi)])
                    } else {
                        trf(
                            "{dpi} dpi ogni {n} righe ({pct} %)",
                            &[
                                ("dpi", &s.dpi),
                                ("n", &s.repeat),
                                ("pct", &i18n::decimal(format!("{:.1}", s.percent))),
                            ],
                        )
                    }
                })
                .collect();
            self.measured.insert(self.caps.name(), done);
            self.note = trf("Righe misurate: {list}", &[("list", &said.join(", "))]);
            return;
        };
        measuring.current = dpi;
        let (gw, gh) = self.caps.glass_inches;
        let (width, height) = self
            .caps
            .bed_pixels(Source::Glass, dpi)
            .unwrap_or_else(|| self.caps.glass_pixels(dpi));
        // Low on the glass: inside an A4 sheet laid at the top, away from
        // anything else left on the glass.
        let top = gh * 25.4 * 0.8;
        let area = within(
            area_mm(dpi, 0.0, top, gw * 25.4, MEASURE_STRIP_MM),
            (width, height),
        );
        let request = ScanRequest {
            dpi,
            source: Source::Glass,
            area,
            depth: if self.offers_16bit() { 16 } else { 8 },
        };
        if let (Some(worker), None) = (&self.worker, &self.busy) {
            worker.send(Command::Scan(Purpose::Measure, request));
            self.busy = Some((Purpose::Measure, None));
        }
    }

    /// Whether the scans of pages are asked at twice the resolution chosen
    /// and halved: "Scansiona a ... dpi e riduci", on the glass, where the
    /// scanner offers twice it.
    fn doubles(&self) -> bool {
        self.settings().double && !self.filming() && self.double_dpi().is_some()
    }

    /// Twice the resolution chosen, if the scanner offers it on the glass.
    fn double_dpi(&self) -> Option<u16> {
        let double = self.dpi().checked_mul(2)?;
        self.caps
            .resolutions_for(Source::Glass)
            .contains(&double)
            .then_some(double)
    }

    /// How much more memory a scan at twice the resolution takes.
    fn doubling_factor(&self) -> f64 {
        if self.doubles() {
            4.0
        } else {
            1.0
        }
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
        Self::bytes(&self.request(), self.dpi()) * self.doubling_factor()
    }

    fn start(&mut self, purpose: Purpose) {
        if !self.idle() {
            // Nothing is sent: a "Rifai" asked for does not wait for the
            // next scan.
            self.redo = None;
            self.redo_auto = false;
            return;
        }
        // The frames of a film kept from a scanner with the adapter, shown
        // with one without it: there to save, not to scan.
        if self.filming() && !self.caps.offers(self.source()) {
            self.redo = None;
            self.redo_auto = false;
            self.note = tr(
                "Questo scanner non ha l'adattatore per pellicole: le foto della \
                            pellicola restano qui da salvare",
            )
            .into();
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
                tr("Fare una nuova scansione?").into(),
                if n == 1 {
                    trf(
                        "{lost} La nuova scansione la sostituisce.",
                        &[("lost", &self.lost_sentence(n))],
                    )
                } else {
                    trf(
                        "{lost} La nuova scansione le sostituisce.",
                        &[("lost", &self.lost_sentence(n))],
                    )
                },
                tr("Scansiona"),
            )
        {
            return;
        }
        let request = if purpose == Purpose::Preview {
            let dpi = self.preview_dpi();
            let (width, height) = self.bed_pixels(dpi);
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
            let request = self.request();
            // A whole scan: nothing to cut yet, but its area is kept (the
            // frames of a film may be found in it, and "Separa" later).
            self.job = Some(Job {
                bounds: [0.0; 4],
                crops: Vec::new(),
                area: Some(request.area),
            });
            request
        };
        // Twice the resolution: the same area, twice the pixels each way;
        // halved when it comes (the job keeps the area at the resolution
        // chosen).
        let mut request = request;
        self.doubled = false;
        if purpose == Purpose::Page && self.doubles() {
            if let Some(dpi) = self.double_dpi() {
                let a = request.area;
                request.area = within(
                    Area {
                        x: a.x * 2,
                        y: a.y * 2,
                        width: a.width * 2,
                        height: a.height * 2,
                    },
                    self.bed_pixels(dpi),
                );
                request.dpi = dpi;
                self.doubled = true;
            }
        }
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

    /// Originals found in a scan (pixels at [`PREVIEW_DPI`] of the scan)
    /// put on the bed (pixels of the preview), from the area the scan
    /// covered at `dpi`.
    fn on_bed(found: Vec<Found>, area: Option<Area>, dpi: u16) -> Vec<Found> {
        let k = f32::from(PREVIEW_DPI) / f32::from(dpi.max(1));
        let (dx, dy) = area.map_or((0.0, 0.0), |a| (a.x as f32 * k, a.y as f32 * k));
        found.into_iter().map(|f| f.scaled(1.0, dx, dy)).collect()
    }

    /// Whether "Separa" can look for originals in a page: a scan kept
    /// whole, in colour, in Foto.
    fn splittable(&self, page: &Page) -> bool {
        self.kind == Kind::Photo && page.whole.is_some() && page.raw.full.format == Format::Rgb
    }

    /// The originals of a page scanned whole (the frames of a film, the
    /// photos on the glass), in pixels of the scan at [`PREVIEW_DPI`].
    fn originals(page: &Page) -> Vec<Found> {
        let whole = page.whole.as_ref();
        match whole.and_then(|w| w.transmission.as_ref().map(|t| (t, w.film))) {
            Some((t, kind)) => film::frames(t, kind),
            None => detect::objects(&at_dpi((*page.raw.full).clone(), PREVIEW_DPI)),
        }
    }

    /// "Separa", once the originals are as wanted: each of `found` (pixels
    /// of the scan at [`PREVIEW_DPI`]) cut out of page `i`, a page of its
    /// own in its place, without scanning again.
    fn split_into(&mut self, i: usize, found: Vec<Found>) {
        let Some(page) = self.pages.get(i).filter(|p| self.splittable(p)) else {
            return;
        };
        let Some(whole) = page.whole.clone() else {
            return;
        };
        if found.is_empty() {
            return;
        }
        let full = Arc::clone(&page.raw.full);
        let job = Job {
            bounds: [0.0; 4],
            crops: Self::on_bed(found, Some(whole.area), full.dpi),
            area: Some(whole.area),
        };
        let samples = page.raw16.as_ref().map(|s| s.to_vec());
        let (tone, scanner, settings, own) = (
            page.raw.tone,
            Arc::clone(&page.raw.scanner),
            page.settings,
            page.own,
        );
        let title = self.page_title(i);
        let parts: Vec<Page> = Self::cut((*full).clone(), samples, Some(&job))
            .into_iter()
            .map(|(image, origin, samples)| {
                let (w, h) = (image.width, image.height);
                let mut raw = Raw::new(image, &self.caps);
                raw.tone = tone;
                raw.scanner = Arc::clone(&scanner);
                let mut part = Page::new(raw, settings);
                part.own = own;
                part.origin = origin;
                part.raw16 = samples.filter(|s| s.len() == w * h * 3).map(Arc::new);
                part
            })
            .collect();
        let n = parts.len();
        self.close_editor(true);
        self.pages.splice(i..=i, parts);
        self.redone = None;
        self.current = Some(i);
        self.unsaved = true;
        self.pdf_rev += 1;
        self.tab = Tab::Result;
        self.result_view = View::default();
        self.pages_changed();
        self.note = if n == 1 {
            trf("{title} ritagliata sull'originale", &[("title", &title)])
        } else {
            trf(
                "{title} separata in {n} immagini",
                &[("title", &title), ("n", &n)],
            )
        };
    }

    /// The images of a scan: the originals cut out and straightened, or
    /// the whole scan.
    fn cut(
        image: Image,
        samples16: Option<Vec<u16>>,
        job: Option<&Job>,
    ) -> Vec<(Image, Option<Found>, Option<Vec<u16>>)> {
        let Some((job, area)) = job
            .filter(|j| !j.crops.is_empty())
            .and_then(|j| Some((j, j.area?)))
        else {
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
        return testing::dir().join("Scansioni"); // not translated: a folder
        #[cfg(not(test))]
        dirs::picture_dir()
            .or_else(dirs::document_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            // not translated: the folder stays the same in every language
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
            "scansione" // not translated: a file name
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
            stripes: self.measured.clone(),
            document: self.document.to_stored(),
            photo: self.photo.to_stored(),
            film: prefs::StoredMode {
                roll_base: self.roll_base,
                slides: self.slides,
                ..self.film_settings.to_stored()
            },
            lamp_off_minutes: self.lamp_off,
            language: self.language.key().into(),
            epson_firmware: self.epson_firmware.clone(),
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
            return Err(tr("niente da salvare").into());
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
            Writer::Tiff => vetroscan_export::save_tiff(path, &pages),
            Writer::Zip => vetroscan_export::save_zip(path, &name, &pages, ImageFormat::Png),
            // One image per page, next to the chosen name.
            Writer::Folder => {
                vetroscan_export::save_folder(folder, &name, &pages, format).map(|_| ())
            }
            Writer::One => vetroscan_export::save_as(path, &pages[0], format),
        }
        .map_err(|e| e.to_string())?;
        Ok(match writer(settings.output, path, n) {
            Writer::One => path.display().to_string(),
            _ if n == 1 => path.display().to_string(),
            Writer::Folder => trf(
                "{n} file in {dir}",
                &[("n", &n), ("dir", &folder.display())],
            ),
            _ => match self.kind {
                Kind::Document => trf(
                    "{n} pagine in {file}",
                    &[("n", &n), ("file", &path.display())],
                ),
                Kind::Photo => trf(
                    "{n} foto in {file}",
                    &[("n", &n), ("file", &path.display())],
                ),
            },
        })
    }

    /// Writes the finished `pages` as a PDF: each on its sheet, its
    /// pictures at the quality chosen and, with a limit, lightened page by
    /// page until the file fits (measured on the file written, and done
    /// again tighter if the estimate fell short). Says how large it is and
    /// what was lightened.
    fn write_pdf(&self, path: &std::path::Path, pages: &[Image]) -> Result<String, String> {
        use vetroscan_export::{compress, Encoding, PdfPage, Sample};
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
            vetroscan_export::save_pdf_pages(path, &laid).map_err(|e| e.to_string())?;
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
        encodings: &[vetroscan_export::Encoding],
        base: vetroscan_export::Encoding,
    ) -> String {
        let mut out = trf("PDF di {size}", &[("size", &human_size(bytes))]);
        if let Some(l) = limit {
            if bytes <= l {
                out.push_str(&trf(" (limite {size})", &[("size", &human_size(l))]));
            } else {
                out.push_str(&trf(
                    ", oltre il limite di {size}: le pagine sono già alleggerite al massimo",
                    &[("size", &human_size(l))],
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
            return Err(tr("niente da salvare").into());
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
                            return Err(tr("bianco e nero: niente 16 bit").into());
                        }
                        _ => image.data.iter().map(|&v| u16::from(v) * 257).collect(),
                    };
                    (data, channels, image.width, image.height, image.dpi)
                }
            };
            vetroscan_export::save_deep(&target, w, h, channels, dpi, &data, &software())
                .map_err(|e| e.to_string())?;
        }
        Ok(if n == 1 {
            path.display().to_string()
        } else {
            trf(
                "{n} file a 16 bit in {dir}",
                &[("n", &n), ("dir", &path.parent().unwrap_or(path).display())],
            )
        })
    }

    /// Writes the linear samples of every page (one file each), without
    /// adjustments: the white the calibration aims at is full scale.
    fn write_raw(&self, path: &std::path::Path, format: RawFormat) -> Result<String, String> {
        if self.pages.is_empty() {
            return Err(tr("niente da salvare").into());
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
                Some(samples) => vetroscan_export::Linear16::from_linear16(
                    full.width,
                    full.height,
                    full.dpi,
                    samples,
                    page.raw.tone.white.map(|w| w * 256.0),
                ),
                None => vetroscan_export::Linear16::from_linear8(&full, page.raw.tone.white)
                    .map_err(|e| e.to_string())?,
            };
            let (make, model) = &*page.raw.scanner;
            let raw = raw.with_source(make, model, &software());
            match format {
                RawFormat::Tiff16 => vetroscan_export::save_tiff16(&target, &raw),
                RawFormat::Dng => vetroscan_export::save_dng(&target, &raw),
            }
            .map_err(|e| e.to_string())?;
            written.push(target);
        }
        Ok(match written.as_slice() {
            [one] => one.display().to_string(),
            _ => trf(
                "{n} file RAW in {dir}",
                &[("n", &n), ("dir", &path.parent().unwrap_or(path).display())],
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
                self.note = trf("Impostazioni non salvate: {e}", &[("e", &e)]);
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
                    | Message::Film
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
        // Doing something else with the pages leaves the scan whole.
        if self.separating.is_some()
            && matches!(
                message,
                Message::Kind(_)
                    | Message::Film
                    | Message::Tab(Tab::Preview)
                    | Message::PageShow(_)
                    | Message::PagePress(_)
                    | Message::PageRotate(..)
                    | Message::PageMove(..)
                    | Message::PageDelete(_)
                    | Message::PageDuplicate(_)
                    | Message::PageRedo(_)
                    | Message::NewDocument
                    | Message::Scan
                    | Message::Preview
                    | Message::Edit(EditMsg::Open(_))
            )
        {
            self.cancel_split();
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
                    // The glass and the adapter changed places meanwhile:
                    // the picture may be of the other's preview.
                    Target::Preview if epoch != self.epoch => self.dirty_preview = true,
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
            Message::Language(choice) => {
                self.language = choice;
                let before = i18n::current();
                i18n::set(choice.resolve());
                // The last note was written in the old language.
                if i18n::current() != before {
                    self.note = tr("Lingua cambiata").into();
                }
            }
            Message::Kind(kind) => return self.set_mode(kind, false),
            Message::Film if self.adapter_plugged() => return self.set_mode(Kind::Photo, true),
            // Frames scanned already: they are there to see and save, with
            // the adapter unplugged too.
            Message::Film if self.has_work(Work::Film) => {
                let task = self.set_mode(Kind::Photo, true);
                if matches!(&self.status, Some(Ok(s)) if s.adapter == Some(false)) {
                    self.note = tr("L'adattatore per pellicole non è collegato: collega \
                                    il suo cavo sul retro dello scanner")
                    .into();
                }
                return task;
            }
            Message::Film => {
                // Plugged in since the state was read? Ask the scanner.
                if let Some(w) = self.worker.as_ref().filter(|_| self.busy.is_none()) {
                    w.send(Command::Status);
                    self.film_wanted = true;
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
                self.note = tr("Conferma la richiesta di Windows (amministratore)…").into();
                return Task::perform(async move { wia::set_quiet(on) }, Message::WiaDone);
            }
            Message::WiaDone(result) => {
                self.wia_quiet = wia::quiet();
                self.note = match result {
                    Ok(()) if self.wia_quiet == Some(true) => {
                        tr("Windows non apre più la sua app ai tasti dello scanner").into()
                    }
                    Ok(()) => {
                        tr("Windows torna ad aprire la sua app ai tasti dello scanner").into()
                    }
                    Err(e) => trf("Non cambiato: {e}", &[("e", &e)]),
                };
            }
            Message::GrantAccess => {
                self.note =
                    tr("Conferma con la password per dare lo scanner a questo utente…").into();
                return Task::perform(async { access::grant() }, Message::GrantDone);
            }
            Message::GrantDone(result) => {
                self.note = match result {
                    Ok(()) => tr("Accesso allo scanner concesso").into(),
                    Err(e) => trf("Accesso non concesso: {e}", &[("e", &e)]),
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
            Message::ShowFolder => {
                let folder = self.folder();
                if let Err(e) = std::fs::create_dir_all(&folder) {
                    self.note = trf("Cartella non creata: {e}", &[("e", &e)]);
                } else {
                    open_folder(&folder);
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
                self.film_settings = Settings::film();
                self.roll_base = None;
                self.slides = false;
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
                self.note = tr("Impostazioni di fabbrica ripristinate").into();
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
            // The colour of the next scans, as the resolution beside it,
            // whatever the viewer shows (a page's own is changed in
            // "Regola": Message::PageColour). The pages keep theirs.
            Message::Colour(colour) => {
                self.remember_shared();
                let kind = self.kind;
                let s = self.settings_mut();
                s.colour = colour;
                if !outputs(kind, colour, s.depth).contains(&s.output) {
                    s.output = outputs(kind, colour, s.depth)[0];
                }
                self.dirty_preview = true;
            }
            Message::PageColour(colour) => {
                if self.editing_page() && self.active().colour != colour {
                    // Kept with the page, whether or not it has a look of
                    // its own: the mode is always the page's.
                    self.remember_page();
                    self.apply(|s| s.colour = colour);
                }
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
                    // The position on the sheet too.
                    let focus = self.pages[self.current.unwrap_or(0)].focus;
                    for page in &mut self.pages {
                        page.layout = None;
                        page.focus = focus;
                    }
                    self.note = tr("Impaginazione applicata a tutte le pagine").into();
                }
            }
            Message::PageFocus(focus) => {
                if let Some(page) = self.current.and_then(|i| self.pages.get_mut(i)) {
                    page.focus = focus.map(|f| f.clamp(0.0, 1.0));
                }
            }
            Message::PageFocusToAll => {
                if let Some(focus) = self
                    .current
                    .and_then(|i| self.pages.get(i))
                    .map(|p| p.focus)
                {
                    for page in &mut self.pages {
                        page.focus = focus;
                    }
                    self.note = tr("Posizione sul foglio data a tutte le pagine").into();
                }
            }
            Message::MmInput(field, typed) => self.mm_input(field, typed),
            Message::MmSubmit => self.mm_draft = None,
            Message::AutoSave(on) => self.settings_mut().auto_save = on,
            Message::AutoCrop(on) => self.settings_mut().auto_crop = on,
            Message::Destripe(on) => self.settings_mut().destripe = on,
            Message::ColourNoise(on) => self.settings_mut().colour_noise = on,
            Message::ScannerColour(on) => self.settings_mut().scanner_colour = on,
            Message::LockBase => {
                let negatives = self.filming() && self.film_kind() == Film::Negative;
                if let Some(base) = self.strip_base.filter(|_| negatives) {
                    self.roll_base = Some(base);
                    // The preview's measures were from another base.
                    self.measures = None;
                    self.note =
                        tr("Base del rullino bloccata: le prossime strisce useranno questa").into();
                }
            }
            Message::Slides(slides) => self.set_slides(slides),
            Message::NewRoll => {
                // The last strip belongs to the roll taken out.
                self.strip_base = None;
                if self.roll_base.take().is_some() {
                    self.measures = None;
                    self.note =
                        tr("Nuovo rullino: ogni striscia misura di nuovo la sua base").into();
                }
            }
            Message::Double(on) => self.settings_mut().double = on,
            Message::MeasureStripes => {
                if self.busy.is_none()
                    && self.worker.is_some()
                    && !self.ask(
                        Message::MeasureStripes,
                        tr("Misurare le righe dello scanner?").into(),
                        trf(
                            "Metti un foglio bianco liscio sul vetro, o lascialo vuoto, e chiudi \
                             il coperchio. Lo scanner fa una striscia per ogni risoluzione fino \
                             a {max} dpi: qualche minuto.",
                            &[("max", &MEASURE_MAX_DPI)],
                        ),
                        tr("Misura"),
                    )
                {
                    let mut left = self.measure_resolutions();
                    left.reverse();
                    self.measuring = Some(Measuring {
                        total: left.len(),
                        left,
                        current: 0,
                        done: Vec::new(),
                    });
                    self.next_measure();
                }
            }
            Message::ForgetStripes => {
                self.measured.remove(&self.caps.name());
                self.note = tr("Righe dello scanner: di nuovo quelle del modello").into();
            }
            Message::Detect => {
                if let Some(transmission) = &self.film_preview {
                    self.found = film::frames(transmission, self.film_kind());
                    self.note = Self::frames_note(self.found.len());
                } else if let Some((raw, _, _)) = &self.preview {
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
                    self.note = tr("Dai un nome al preset").into();
                    return Task::none();
                }
                // "Applica" looks among the factory presets first: one of
                // the user's with the same name could never be applied.
                if [Kind::Document, Kind::Photo]
                    .into_iter()
                    .flat_map(factory_presets)
                    .any(|(n, _)| n == name)
                {
                    self.note = trf(
                        "«{name}» è un preset di fabbrica: scegli un altro nome",
                        &[("name", &name)],
                    );
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
                        self.note = trf(
                            "Al massimo {n} preset: eliminane uno o salva sopra a uno esistente",
                            &[("n", &prefs::MAX_PRESETS)],
                        );
                        return Task::none();
                    }
                }
                self.note = match prefs::save_presets(&self.presets) {
                    Ok(()) => trf("Preset «{name}» salvato", &[("name", &name)]),
                    Err(e) => trf("Preset non salvato: {e}", &[("e", &e)]),
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
                    // How the scanner is driven, not a look.
                    new.destripe = self.settings().destripe;
                    new.double = self.settings().double;
                    new.colour_noise = self.settings().colour_noise;
                    new.scanner_colour = self.settings().scanner_colour;
                    self.change(|s| *s = new);
                    self.note = trf(
                        "Preset «{name}» applicato (Ctrl+Z per tornare indietro)",
                        &[("name", &name)],
                    );
                }
            }
            Message::PresetDelete(name) => {
                self.presets.retain(|p| p.name != name);
                self.note = match prefs::save_presets(&self.presets) {
                    Ok(()) => trf("Preset «{name}» eliminato", &[("name", &name)]),
                    Err(e) => trf("Preset non salvato: {e}", &[("e", &e)]),
                };
            }
            Message::OwnLook(on) => {
                let shared = *self.settings();
                if self.editing_page() && (self.kind == Kind::Document || !on) {
                    let photo = self.kind == Kind::Photo;
                    if photo {
                        self.remember_page();
                    }
                    if let Some(page) = self.shown_mut() {
                        page.own = on.then(|| page.own.unwrap_or(Look::of(&shared)));
                        self.dirty_page = true;
                    }
                    self.note = match (photo, on) {
                        (false, true) => tr("Questa pagina ha ora regolazioni sue").into(),
                        (false, false) => {
                            tr("Questa pagina segue di nuovo le regolazioni comuni").into()
                        }
                        (true, _) => tr(
                            "Questa foto segue di nuovo le regolazioni comuni (Ctrl+Z per tornare \
                             indietro)",
                        )
                        .into(),
                    };
                }
            }
            // Foto and Pellicola: the look of the photo shown becomes the
            // shared one, and every photo follows it again.
            Message::LookApplyToAll if self.kind == Kind::Photo => {
                let shared = *self.settings();
                let look = self
                    .shown()
                    .filter(|_| self.editing_page())
                    .map(|p| Look::of(&p.effective(&shared)));
                if let Some(look) = look {
                    self.remember_shared();
                    look.put(self.settings_mut());
                    for page in &mut self.pages {
                        page.own = None;
                    }
                    if let Some((_, page)) = &mut self.redone {
                        page.own = None;
                    }
                    let shown = self.current;
                    self.stale = (0..self.pages.len())
                        .filter(|&i| Some(i) != shown)
                        .collect();
                    self.dirty_preview = true;
                    self.dirty_page = true;
                    self.note = tr("Regolazioni applicate a tutte le foto").into();
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
                    self.note = tr("Regolazioni applicate a tutte le pagine").into();
                }
            }
            Message::CurveChannel(c) => {
                self.curve_channel = c;
                self.curve_selected = None;
            }
            Message::CurvePoint(point) => self.curve_selected = point,
            Message::CurveSet(points, point) => {
                self.curve_selected = point;
                let c = self.curve_index();
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
                            self.note = tr("Annullato").into();
                        }
                        None => self.note = tr("Niente da annullare qui").into(),
                    }
                }
            }
            Message::Redo => {
                if let Some((target, settings)) = self.redo_stack.pop() {
                    if let Some(current) = self.restore(target, settings) {
                        self.undo.push(current);
                        self.note = tr("Ripristinato").into();
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
                            Err(_) => {
                                self.note = trf("«{text}» non è un numero", &[("text", &text)])
                            }
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
                    self.note = trf(
                        "Bilanciamento dal punto scelto (R {r} G {g} B {b})",
                        &[("r", &sample[0]), ("g", &sample[1]), ("b", &sample[2])],
                    );
                }
            }
            Message::ResetGroup(group) => self.change(|s| {
                let d = Adjustments::default();
                let j = &mut s.adjust;
                match group {
                    Group::Colour => {
                        j.temperature = d.temperature;
                        j.tint = d.tint;
                        j.saturation = d.saturation;
                        j.vibrance = d.vibrance;
                    }
                    Group::Tone => {
                        j.exposure = d.exposure;
                        j.contrast = d.contrast;
                        j.highlights = d.highlights;
                        j.shadows = d.shadows;
                        j.whites = d.whites;
                        j.blacks = d.blacks;
                    }
                    Group::Levels => {
                        j.black = d.black;
                        j.white = d.white;
                        j.gamma = d.gamma;
                        j.brightness = d.brightness;
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
            Message::Preview => {
                // Films: the adapter's area is small and full of frames, and
                // a scan finds them itself; a preview would only add a lamp
                // warm-up (9 November 2026, with Matteo).
                if self.filming() {
                    self.note =
                        tr("Per le pellicole l'anteprima non serve: Scansiona trova i fotogrammi")
                            .into();
                } else {
                    self.start(Purpose::Preview);
                }
            }
            Message::Scan => {
                self.redone = None;
                self.start(Purpose::Page);
            }
            Message::Cancel => {
                if let Some(w) = &self.worker {
                    w.cancel();
                    self.note = tr("Annullamento…").into();
                }
            }
            Message::Save => {
                let output = self.settings().output;
                let ext = output.extension();
                let (filter, title) = self.save_labels();
                let prefix = self.name();
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
                            .set_title(title)
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
                        self.note = trf("Salvato: {what}", &[("what", &what)]);
                        self.unsaved = false;
                        // "Salva…" in the question: now what was asked. Closing
                        // asks again while another way of working has work
                        // not saved.
                        match then {
                            Some(then @ Message::CloseRequested(_)) => return self.handle(then),
                            Some(then) => return self.run_confirmed(then),
                            None => {}
                        }
                    }
                    Err(e) => self.note = trf("Salvataggio non riuscito: {e}", &[("e", &e)]),
                }
            }
            Message::SaveTo(None) => self.after_save = None,
            Message::Confirm(yes) => {
                if let Some(confirm) = self.confirm.take() {
                    if yes {
                        return self.run_confirmed(*confirm.then);
                    }
                    self.print_next = false;
                    self.note = tr("Nulla è cambiato").into();
                }
            }
            Message::ConfirmSave => {
                if let Some(confirm) = self.confirm.take() {
                    // Closing, with nothing to save where the app is: the
                    // first way of working with work not saved is shown and
                    // saved.
                    let here = self.unsaved && !self.pages.is_empty();
                    if matches!(*confirm.then, Message::CloseRequested(_)) && !here {
                        if let Some(&(work, _)) = self.unsaved_work().first() {
                            let _ = self.set_mode(
                                match work {
                                    Work::Document => Kind::Document,
                                    Work::Photo | Work::Film => Kind::Photo,
                                },
                                work == Work::Film,
                            );
                        }
                    }
                    self.after_save = Some(*confirm.then);
                    return self.handle(Message::Save);
                }
            }
            Message::CloseRequested(id) => {
                // The work of every way of working, not only the one shown.
                let lost = self.lost_closing();
                let asked = if self.busy.is_some() {
                    let body = match &lost {
                        Some(lost) => trf(
                            "La scansione in corso si interrompe. {lost}",
                            &[("lost", &lost)],
                        ),
                        None => tr("La scansione in corso si interrompe e va persa.").into(),
                    };
                    self.ask(
                        Message::CloseRequested(id),
                        tr("Chiudere durante la scansione?").into(),
                        body,
                        tr("Chiudi"),
                    )
                } else if let Some(lost) = lost {
                    self.ask(
                        Message::CloseRequested(id),
                        tr("Chiudere senza salvare?").into(),
                        lost,
                        tr("Chiudi senza salvare"),
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
                    self.note = tr("Spegnimento della lampada…").into();
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
                    self.note = trf(
                        "{title} duplicata: la copia è la {copy}, da ritagliare e regolare a parte",
                        &[
                            ("title", &self.page_title(i)),
                            ("copy", &self.page_name(i + 1)),
                        ],
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
                        self.note = trf("Spostata: ora è la {name}", &[("name", &name)]);
                    }
                }
            }
            Message::PageDelete(i) => {
                if self.busy.is_none() && i < self.pages.len() {
                    let title = self.page_title(i);
                    if self.unsaved
                        && self.ask(
                            Message::PageDelete(i),
                            trf("Eliminare la {name}?", &[("name", &self.page_name(i))]),
                            tr("Non è salvata e non si può recuperare.").into(),
                            trf("Elimina la {name}", &[("name", &self.page_name(i))]),
                        )
                    {
                        return Task::none();
                    }
                    self.pages.remove(i);
                    self.redone = None;
                    self.current = (!self.pages.is_empty()).then(|| i.min(self.pages.len() - 1));
                    self.pages_changed();
                    self.note = trf("{title} eliminata", &[("title", &title)]);
                }
            }
            Message::PageSplit(i) => {
                if self.busy.is_none() && self.redone.is_none() {
                    self.begin_split(i);
                }
            }
            Message::Split(m) => self.on_split(m),
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
                        self.note =
                            trf("{title} sostituita", &[("title", &self.page_title(index))]);
                    }
                }
            }
            Message::RejectRedo => {
                if let Some((index, _)) = self.redone.take() {
                    self.current = Some(index);
                    self.forget_page_undo();
                    self.note = trf(
                        "{title} lasciata com'era",
                        &[("title", &self.page_title(index))],
                    );
                }
            }
            Message::NewDocument => {
                let n = self.pages.len();
                if n > 0
                    && self.unsaved
                    && self.ask(
                        Message::NewDocument,
                        tr("Iniziare un nuovo documento?").into(),
                        self.lost_sentence(n),
                        tr("Scarta e ricomincia"),
                    )
                {
                    return Task::none();
                }
                self.pages.clear();
                self.current = None;
                self.redone = None;
                self.pages_changed();
                self.note = tr("Nuovo documento").into();
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
        // "Separa": Enter cuts, Esc leaves the scan whole, Delete leaves
        // out the area chosen.
        if let Some(sep) = &self.separating {
            let message = match key.as_ref() {
                Key::Named(Named::Enter) => Some(SplitMsg::Apply),
                Key::Named(Named::Escape) => Some(SplitMsg::Cancel),
                Key::Named(Named::Delete | Named::Backspace) => sep.chosen.map(SplitMsg::Drop),
                _ => None,
            };
            if let Some(m) = message {
                return self.update(m.into());
            }
        }
        // Arrows and Home move what is active ([`keys`]).
        if let Some(m) = keys::read(&key, modifiers) {
            let message = self.on_move(m);
            return self.keyed(message);
        }
        let curve = self.curve_selected.is_some() && self.curve_shown();
        let straightening = self
            .editor()
            .is_some_and(|e| e.section == editor::Section::Frame);
        match key.as_ref() {
            // The point of the curve: the next one (Shift: the one before),
            // away, or no longer chosen.
            Key::Named(Named::Tab) if curve => {
                let message = self.curve_key(Some(!modifiers.shift()));
                return self.keyed(message);
            }
            Key::Named(Named::Delete | Named::Backspace) if curve => {
                let message = self.curve_key(None);
                return self.keyed(message);
            }
            Key::Named(Named::Escape) if curve => {
                return self.update(Message::CurvePoint(None));
            }
            // The strip of pages: the page before or after; with Alt, the
            // page shown moves there.
            Key::Named(named @ (Named::PageUp | Named::PageDown)) => {
                let forward = named == Named::PageDown;
                let Some(i) = self.current.filter(|&i| i < self.pages.len()) else {
                    return Task::none();
                };
                let message = if modifiers.alt() {
                    Some(Message::PageMove(i, forward))
                } else if forward {
                    (i + 1 < self.pages.len()).then_some(Message::PageShow(i + 1))
                } else {
                    i.checked_sub(1).map(Message::PageShow)
                };
                return self.keyed(message);
            }
            // Straightening: [ and ] turn by 0.1°, with Shift by 1° (on
            // some keyboards Shift makes them { and }; on others they need
            // AltGr, which comes as Ctrl + Alt).
            Key::Character(c @ ("[" | "]" | "{" | "}")) if straightening => {
                let degrees = if modifiers.shift() || c == "{" || c == "}" {
                    1.0
                } else {
                    0.1
                };
                let sign = if c == "[" || c == "{" { -1.0 } else { 1.0 };
                return self.update(EditMsg::AngleBy(sign * degrees).into());
            }
            _ => {}
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
            (Key::Character("o"), true, _) => Some(Message::ShowFolder),
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
            message => self.keyed(message),
        }
    }

    /// Does what a key asked for as if it came by itself (so that a page
    /// changed from the keyboard is unsaved, as one changed with the
    /// mouse).
    fn keyed(&mut self, message: Option<Message>) -> Task<Message> {
        match message {
            Some(m) => self.update(m),
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
                if std::mem::take(&mut self.film_wanted) {
                    if self.adapter_plugged() {
                        let _ = self.set_mode(Kind::Photo, true);
                    } else {
                        self.note = tr("L'adattatore per pellicole non è collegato: collega \
                                        il suo cavo sul retro dello scanner")
                        .into();
                    }
                }
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
                        self.measuring = None;
                        if e == worker::CANCELLED {
                            // Stopped with "Annulla": nothing went wrong.
                            self.note = match purpose {
                                Purpose::Preview => tr("Anteprima annullata").into(),
                                Purpose::Page => tr("Scansione annullata").into(),
                                Purpose::Measure => tr("Misura delle righe annullata").into(),
                            };
                        } else {
                            self.note = trf("Errore: {e}", &[("e", &e)]);
                            if purpose == Purpose::Page {
                                notify(tr("Scansione non riuscita"), &e, true);
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
            0 => tr("Anteprima pronta: trascina per scegliere l'area, regola a destra").into(),
            1 => tr("Anteprima pronta: un originale riconosciuto, sarà ritagliato e raddrizzato")
                .into(),
            n => trf(
                "Anteprima pronta: {n} originali riconosciuti, ognuno diventa un'immagine \
                 raddrizzata",
                &[("n", &n)],
            ),
        }
    }

    /// A scan as the settings want it before anything else: the scanner's
    /// stripes taken away (at the resolution scanned, where they were
    /// measured; for films on the negative's linear samples, before it is
    /// measured and turned into its picture), then, on the glass, a scan at
    /// twice the resolution brought back to the one chosen.
    fn cleaned(
        &mut self,
        purpose: Purpose,
        mut image: Image,
        mut samples16: Option<Vec<u16>>,
    ) -> (Image, Option<Vec<u16>>) {
        let doubled = std::mem::take(&mut self.doubled) && purpose == Purpose::Page;
        if purpose == Purpose::Measure {
            return (image, samples16);
        }
        if self.settings().destripe {
            if let Some(repeat) = self.stripes_at(self.source(), image.dpi) {
                stripes::remove(&mut image, samples16.as_deref_mut(), repeat);
            }
        }
        if self.filming() {
            return (image, samples16);
        }
        if doubled && image.format == Format::Rgb {
            let (w, h) = (image.width, image.height);
            let (data, nw, nh) = transform::halved(&image.data, w, h, 3);
            let deep = samples16
                .filter(|s| s.len() == w * h * 3)
                .map(|s| transform::halved(&s, w, h, 3).0);
            return (
                Image {
                    width: nw,
                    height: nh,
                    dpi: image.dpi / 2,
                    format: Format::Rgb,
                    data,
                },
                deep,
            );
        }
        (image, samples16)
    }

    /// The note on the preview of films: the frames found, or no film.
    fn frames_note(n: usize) -> String {
        match n {
            0 => tr(
                "Anteprima pronta: nessun fotogramma riconosciuto, trascina per scegliere \
                     l'area",
            )
            .into(),
            1 => tr("Anteprima pronta: un fotogramma riconosciuto, diventa un'immagine").into(),
            n => trf(
                "Anteprima pronta: {n} fotogrammi riconosciuti, ognuno diventa un'immagine",
                &[("n", &n)],
            ),
        }
    }

    fn on_scan(&mut self, purpose: Purpose, image: Image, samples16: Option<Vec<u16>>) {
        let (image, samples16) = self.cleaned(purpose, image, samples16);
        let colour_noise = self.settings().colour_noise;
        let correction = self.colour_correction();
        match purpose {
            Purpose::Preview => {
                self.close_editor(true);
                // Films: their picture shown, their frames found on the
                // film as the scanner saw it.
                let film = self.filming();
                let kind = self.film_kind();
                let transmission = film.then(|| at_dpi(image.clone(), PREVIEW_DPI));
                let image = if film {
                    let (picture, _, measures, gaps) = Self::develop(
                        kind,
                        image,
                        samples16,
                        None,
                        self.roll_base,
                        correction,
                        colour_noise,
                    );
                    self.measures = measures;
                    if gaps.is_some() {
                        self.strip_base = gaps;
                    }
                    picture
                } else {
                    image
                };
                // The preview is worked in pixels of PREVIEW_DPI.
                let raw = self.raw(at_dpi(image, PREVIEW_DPI));
                let settings = *self.settings();
                let shown = render(&raw.proxy, &settings, &settings.adjust, &raw.tone);
                let picture = Picture::from_rgb(shown.width, shown.height, &shown.to_rgb());
                self.found = match &transmission {
                    Some(t) => film::frames(t, kind),
                    None => detect::objects(&raw.full),
                };
                self.film_preview = transmission;
                self.preview = Some((raw, shown, picture));
                self.preview_view = View::default();
                self.tab = Tab::Preview;
                let n = if self.settings().auto_crop {
                    self.found.len()
                } else {
                    0
                };
                self.note = match (film, self.measures) {
                    (true, None) => {
                        tr("Anteprima pronta: nessuna pellicola riconosciuta nel supporto").into()
                    }
                    (true, Some(_)) => Self::frames_note(n),
                    (false, _) => Self::found_note(n),
                };
            }
            Purpose::Measure => {
                let found = stripes::measure(&image, samples16.as_deref());
                if let Some(measuring) = &mut self.measuring {
                    measuring.done.push(prefs::StoredStripes {
                        dpi: measuring.current,
                        repeat: found.map_or(0, |f| f.repeat as u16),
                        percent: found.map_or(0.0, |f| f.strength * 100.0),
                    });
                }
                self.next_measure();
            }
            Purpose::Page => {
                let mut job = self.job.take();
                let film = self.filming();
                let kind = self.film_kind();
                let whole = job.as_ref().is_some_and(|j| j.crops.is_empty());
                // A film scanned whole: the frames are found on it as the
                // scanner saw it, now (without a preview) or with "Separa".
                let transmission =
                    (film && whole).then(|| Arc::new(at_dpi(image.clone(), PREVIEW_DPI)));
                if let (Some(j), Some(t)) = (job.as_mut(), &transmission) {
                    if self.settings().auto_crop && self.selection.is_none() && self.redo.is_none()
                    {
                        j.crops = Self::on_bed(film::frames(t, kind), j.area, image.dpi);
                        self.batch.1 = j.crops.len().max(1);
                    }
                }
                let kept = job
                    .as_ref()
                    .filter(|j| j.crops.is_empty())
                    .and_then(|j| j.area)
                    .map(|area| Whole {
                        area,
                        transmission: transmission.clone(),
                        film: kind,
                    });
                let (image, samples16) = if film {
                    let (picture, deep, _, gaps) = Self::develop(
                        kind,
                        image,
                        samples16,
                        self.measures,
                        self.roll_base,
                        correction,
                        colour_noise,
                    );
                    if gaps.is_some() {
                        self.strip_base = gaps;
                    }
                    (picture, deep)
                } else {
                    (image, samples16)
                };
                for (image, origin, samples) in Self::cut(image, samples16, job.as_ref()) {
                    self.add_page(image, origin, samples, kept.clone());
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
    fn add_page(
        &mut self,
        image: Image,
        origin: Option<Found>,
        samples16: Option<Vec<u16>>,
        whole: Option<Whole>,
    ) {
        self.close_editor(true);
        self.unsaved = true;
        self.pdf_rev += 1;
        let (w, h, dpi) = (image.width, image.height, image.dpi);
        let mut page = Page::new(self.raw(image), *self.settings());
        page.whole = whole.filter(|_| origin.is_none());
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
            trf(
                " ({i} di {n})",
                &[("i", &self.batch.0), ("n", &self.batch.1)],
            )
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
                self.note = trf("{title} rifatta", &[("title", &self.page_title(index))]);
                notify(&self.note, "", false);
            }
            (Some(index), _) => {
                self.redo = None;
                self.redone = Some((index, page));
                // The panel and the strip show the page done again.
                self.current = Some(index);
                self.note = trf(
                    "Nuova scansione della {name}: tienila o lascia la vecchia",
                    &[("name", &self.page_name(index))],
                );
            }
            (None, Kind::Document) => {
                self.pages.push(page);
                self.current = Some(self.pages.len() - 1);
                self.note = trf(
                    "Pagina {n} acquisita{of} · {w} × {h} pixel a {dpi} dpi",
                    &[
                        ("n", &self.pages.len()),
                        ("of", &of),
                        ("w", &w),
                        ("h", &h),
                        ("dpi", &dpi),
                    ],
                );
                notify(
                    &trf("Pagina {n} acquisita", &[("n", &self.pages.len())]),
                    "",
                    false,
                );
            }
            (None, Kind::Photo) => {
                if first {
                    self.pages.clear();
                    self.pages_changed();
                }
                self.pages.push(page);
                self.current = Some(if first { 0 } else { self.current.unwrap_or(0) });
                self.note = trf(
                    "Foto acquisita{of} · {w} × {h} pixel a {dpi} dpi",
                    &[("of", &of), ("w", &w), ("h", &h), ("dpi", &dpi)],
                );
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
        if self.settings().auto_save && n > 0 {
            let folder = self.folder();
            let _ = std::fs::create_dir_all(&folder);
            let path = folder.join(Self::free_name(
                &folder,
                self.name(),
                self.settings().output.extension(),
            ));
            let written = self.write(&path);
            if written.is_ok() {
                self.unsaved = false;
            }
            self.note = match written {
                Ok(_) if n > 1 => {
                    notify(
                        &trf("{n} foto salvate", &[("n", &n)]),
                        &folder.display().to_string(),
                        false,
                    );
                    trf(
                        "{n} foto salvate in {dir}",
                        &[("n", &n), ("dir", &folder.display())],
                    )
                }
                Ok(_) => {
                    notify(tr("Foto salvata"), &path.display().to_string(), false);
                    trf("Foto salvata in {file}", &[("file", &path.display())])
                }
                Err(e) => trf("Salvataggio non riuscito: {e}", &[("e", &e)]),
            };
        } else if n > 1 {
            self.note = trf(
                "{n} foto acquisite: scegli a sinistra, salva con «Salva»",
                &[("n", &n)],
            );
            notify(&trf("{n} foto acquisite", &[("n", &n)]), "", false);
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
            self.note = trf(
                "Tasto {name} ignorato: scansione in corso",
                &[("name", &name)],
            );
            return;
        }
        if self.confirm.is_some() {
            self.note = trf(
                "Tasto {name} ignorato: rispondi prima alla domanda",
                &[("name", &name)],
            );
            notify(tr("Rispondi alla domanda nell'app"), "", false);
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
                    self.note = tr("Nessuna pagina da rifare: Scan acquisisce la prima").into();
                    notify(
                        tr("Nessuna pagina da rifare"),
                        tr("Premi Scan per la prima pagina"),
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
                    notify(
                        tr("Fotocopia"),
                        tr("Scansione in corso, poi la stampa"),
                        false,
                    );
                }
            }
            ButtonAction::SaveDocument => {
                if self.pages.is_empty() {
                    self.note = tr("Documento vuoto: premi Scan per acquisire una pagina").into();
                    notify(
                        tr("Documento vuoto"),
                        tr("Premi Scan per acquisire una pagina"),
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
                        self.note = trf("Salvato: {what}", &[("what", &what)]);
                        self.unsaved = false;
                        notify(tr("Documento salvato"), &path.display().to_string(), false);
                        open_folder(&folder);
                    }
                    Err(e) => {
                        self.note = trf("Salvataggio non riuscito: {e}", &[("e", &e)]);
                        notify(tr("Salvataggio non riuscito"), &e, true);
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
            notify(&confirm.title, tr("Rispondi nell'app"), false);
        }
    }

    /// Prints the photo shown on the default printer (the photocopy).
    fn print_shown(&mut self) {
        let shared = *self.settings();
        let Some(image) = self.shown().map(|p| p.finished(&shared)) else {
            return;
        };
        let path = std::env::temp_dir().join(Self::stamped("fotocopia", "png"));
        match vetroscan_export::save(&path, &image) {
            Ok(()) => match print_file(&path) {
                Ok(()) => {
                    self.note = tr("Fotocopia inviata alla stampante predefinita").into();
                    notify(tr("Fotocopia inviata alla stampante"), "", false);
                }
                Err(e) => {
                    self.note = trf("Stampa non riuscita: {e}", &[("e", &e)]);
                    notify(tr("Stampa non riuscita"), &e, true);
                }
            },
            Err(e) => self.note = trf("Stampa non riuscita: {e}", &[("e", &e)]),
        }
    }

    // ------------------------------------------------------------- views

    fn view(&self) -> Element<'_, Message> {
        let mut body = row![].height(Length::Fill);
        // In Foto from the first photo: its tools (Duplica, Rifai, Elimina)
        // are there.
        if self.kind == Kind::Document || !self.pages.is_empty() {
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
                    button(text(tr("Annulla")).size(13))
                        .padding([6, 14])
                        .style(button::secondary)
                        .on_press(Message::Confirm(false)),
                    button(text(tr("Salva…")).size(13))
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
        let segment = |label: &'static str, message: Option<Message>, on: bool| {
            let off = message.is_none();
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
                    } else if off {
                        style.text_color = palette.background.strong.color;
                    }
                    style
                })
                .on_press_maybe(message.filter(|_| self.busy.is_none()))
        };
        let mut segments = row![
            segment(
                tr("Documento"),
                Some(Message::Kind(Kind::Document)),
                self.kind == Kind::Document
            ),
            segment(
                tr("Foto"),
                Some(Message::Kind(Kind::Photo)),
                self.kind == Kind::Photo && !self.filming()
            ),
        ]
        .spacing(2);
        // Films only for a scanner with an adapter for them (with its cable
        // unplugged a press says so), or while frames scanned are there.
        if self.offers_films() || self.filming() || self.has_work(Work::Film) {
            segments = segments.push(segment(
                tr("Pellicola"),
                Some(Message::Film),
                self.filming(),
            ));
        }
        let modes = container(segments)
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
                tr("Al lavoro").to_string(),
            ),
            (Some(Ok(s)), None) => (
                iced::Color::from_rgb(0.24, 0.77, 0.48),
                match (s.ready, s.lamp) {
                    (false, _) => tr("Occupato").to_string(),
                    (true, Some(LampState::Off)) => tr("Pronto · lampada spenta").to_string(),
                    (true, Some(_)) => tr("Pronto · lampada accesa").to_string(),
                    (true, None) => tr("Pronto").to_string(),
                },
            ),
            (Some(Err(_)), None) => (
                iced::Color::from_rgb(0.94, 0.36, 0.33),
                tr("Scanner non trovato").to_string(),
            ),
            (None, None) => (
                iced::Color::from_rgb(0.6, 0.6, 0.6),
                tr("Collegamento…").to_string(),
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
            (Kind::Document, 0 | 1) => {
                trf("Scansiona pagina {n}", &[("n", &(self.pages.len() + 1))])
            }
            (Kind::Document, n) => trf("Scansiona {n} pagine", &[("n", &n)]),
            (Kind::Photo, 0 | 1) => tr("Scansiona foto").into(),
            (Kind::Photo, n) => trf("Scansiona {n} foto", &[("n", &n)]),
        };
        // The name, and at its top right the version, small.
        let name = row![
            text(APP_NAME).size(15),
            text(concat!("v", env!("CARGO_PKG_VERSION")))
                .size(10)
                .style(text::secondary),
        ]
        .spacing(3)
        .align_y(Alignment::Start);
        // Films have no preview (Message::Preview).
        let preview = (!self.filming()).then(|| {
            button(text(tr("Anteprima")).size(13))
                .style(button::secondary)
                .padding([7, 12])
                .on_press_maybe(self.idle().then_some(Message::Preview))
        });
        let bar = row![
            name,
            modes,
            Space::new().width(Length::Fill),
            button(chip)
                .padding(0)
                .style(button::text)
                .on_press_maybe(self.idle().then_some(Message::RefreshStatus)),
        ]
        .push(preview)
        .push(
            row![
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
            .align_y(Alignment::Center),
        )
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
            Kind::Document => tr("PAGINE"),
            Kind::Photo => tr("FOTO"),
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
                label.push_str(tr(" · propria"));
            }
            if !page.transform.is_identity() {
                label.push_str(" · ✎");
            }
            if page.layout.is_some() {
                label.push_str(tr(" · foglio"));
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
                            tr("Modifica"),
                            EditMsg::Open(i).into(),
                            !edited_here && self.redone.is_none()
                        ),
                        tool(tr("Rifai"), Message::PageRedo(i), idle),
                    ],
                    row![
                        tool(
                            tr("Duplica"),
                            Message::PageDuplicate(i),
                            idle && self.redone.is_none()
                        ),
                        Space::new().width(Length::Fill),
                        tool(tr("Elimina"), Message::PageDelete(i), idle),
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
            Kind::Document => (ButtonAction::AddPage, tr("aggiunge una pagina")),
            Kind::Photo => (ButtonAction::ScanPhoto, tr("scansiona una foto")),
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
                text(trf(
                    "Il tasto {button} dello scanner {what}",
                    &[("button", &button), ("what", &what)],
                ))
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
        let e = vetroscan_export::Encoding::from_percent(s.pdf_quality);
        let what = match (s.pdf_quality, e.max_dpi) {
            (100, _) => tr("come la scansione, JPEG 95").to_string(),
            (_, None) => trf(
                "JPEG {quality}, pagine senza colore in grigio",
                &[("quality", &e.quality)],
            ),
            (_, Some(dpi)) => trf(
                "JPEG {quality}, al massimo {dpi} dpi",
                &[("quality", &e.quality), ("dpi", &dpi)],
            ),
        };
        let quality = column![
            labelled(
                tr("Qualità"),
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
                    i18n::decimal(
                        format!("{mb:.2}")
                            .trim_end_matches('0')
                            .trim_end_matches('.')
                            .to_string(),
                    )
                })
                .unwrap_or_default()
        });
        let invalid = parse_megabytes(&typed).is_none();
        let limit = labelled(
            tr("Al massimo"),
            row![
                iced::widget::text_input(tr("nessun limite"), &typed)
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
                    None => trf(
                        "Dimensione stimata: circa {size}",
                        &[("size", &human_size(p.bytes))],
                    ),
                    Some(kb) if p.fits => {
                        let limit = human_size(u64::from(kb) * 1024);
                        match lightened(&p.encodings, e_base(s.pdf_quality)) {
                            Some(l) => trf(
                                "Circa {size}, nel limite di {limit}: {l}",
                                &[("size", &human_size(p.bytes)), ("limit", &limit), ("l", &l)],
                            ),
                            None => trf(
                                "Circa {size}, già nel limite di {limit}: nessuna pagina da \
                                 alleggerire",
                                &[("size", &human_size(p.bytes)), ("limit", &limit)],
                            ),
                        }
                    }
                    Some(kb) => trf(
                        "Anche alleggerendo ogni pagina resta circa {size}, oltre {limit}: prova \
                         Grigio o Bianco e nero, o una risoluzione di scansione più bassa",
                        &[
                            ("size", &human_size(p.bytes)),
                            ("limit", &human_size(u64::from(kb) * 1024)),
                        ],
                    ),
                },
                Err(err) => trf("Stima non riuscita: {err}", &[("err", &err)]),
            },
            (None, Some(old)) => match &old.result {
                Ok(p) => trf(
                    "Circa {size} · aggiornamento…",
                    &[("size", &human_size(p.bytes))],
                ),
                Err(_) => tr("Calcolo della dimensione…").into(),
            },
            (None, None) if self.pages.is_empty() => {
                tr("La dimensione si vede dopo la prima scansione").into()
            }
            (None, None) => tr("Calcolo della dimensione…").into(),
        };
        let mut out = column![heading(tr("Dimensione del PDF")), quality, limit].spacing(8);
        if invalid {
            out = out.push(text(tr("Scrivi un numero di megabyte, per esempio 5 o 2,5")).size(11));
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
                    tr("Premi «Anteprima» per vedere il vetro"),
                    self.preview_view,
                    self.selection,
                    // The selection has the precedence over the originals.
                    if self.settings().auto_crop && self.selection.is_none() {
                        self.found.iter().map(|f| f.corners()).collect()
                    } else {
                        Vec::new()
                    },
                    Message::View,
                    Some(Message::Select),
                    pick,
                    None,
                    25.4 / f32::from(PREVIEW_DPI),
                    false,
                ),
                self.preview_view.scale,
            ),
            Tab::Result if self.separating.is_some() => {
                (self.split_viewer(), self.result_view.scale)
            }
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
                        Sheet::Auto => tr("orientamento automatico"),
                        Sheet::Portrait => "verticale",
                        Sheet::Landscape => "orizzontale",
                    },
                    l.fit.to_string().to_lowercase(),
                    if own {
                        tr(" · solo questa pagina")
                    } else {
                        ""
                    }
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
                    tr("Nessuna scansione: premi «Scansiona»"),
                    self.result_view,
                    self.crop_selection(),
                    Vec::new(),
                    Message::View,
                    self.cropping()
                        .then_some(editor::crop_message as fn(Option<Selection>) -> Message),
                    pick,
                    None,
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
            Kind::Document => tr("Pagina"),
            Kind::Photo => tr("Foto"),
        };
        let views = card(
            row![]
                .push((!self.filming()).then(|| {
                    small(
                        tr("Anteprima"),
                        Some(Message::Tab(Tab::Preview)),
                        self.tab == Tab::Preview,
                    )
                }))
                .push(small(
                    result_label,
                    Some(Message::Tab(Tab::Result)),
                    self.tab == Tab::Result,
                ))
                .spacing(2),
        );
        let edit = (self.tab == Tab::Result
            && self.editor().is_none()
            && self.redone.is_none()
            && self.shown().is_some())
        .then(|| self.current.map(|i| EditMsg::Open(i).into()))
        .flatten();
        // The photo shown, scanned whole: its originals cut out of it
        // without scanning again.
        let split = (self.tab == Tab::Result
            && self.busy.is_none()
            && self.redone.is_none()
            && self.separating.is_none())
        .then_some(self.current)
        .flatten()
        .and_then(|i| self.pages.get(i).map(|p| (i, p)))
        .filter(|(_, p)| self.splittable(p));
        let mut tools = column![small(tr("Modifica…"), edit, false)].spacing(2);
        if let Some((i, page)) = split {
            let label = if page
                .whole
                .as_ref()
                .is_some_and(|w| w.transmission.is_some())
            {
                tr("Separa i fotogrammi")
            } else {
                tr("Separa le foto")
            };
            tools = tools.push(small(label, Some(Message::PageSplit(i)), false));
        }
        let tools = card(
            tools
                .push(small(
                    tr("Contagocce"),
                    Some(Message::Eyedropper),
                    self.eyedropper,
                ))
                .push(small(
                    tr("Togli selezione"),
                    (self.tab == Tab::Preview && self.selection.is_some())
                        .then_some(Message::Select(None)),
                    false,
                )),
        );
        let compare = card(
            row![
                small(tr("Prima"), Some(Message::Before(true)), self.before),
                small(tr("Dopo"), Some(Message::Before(false)), !self.before),
            ]
            .spacing(2),
        );
        let zoom_label = match zoom {
            None => tr("Vista intera").to_string(),
            Some(s) => format!("{:.0}%", s * 100.0),
        };
        let zoom = card(
            row![
                text(zoom_label).size(12),
                small(tr("Adatta"), Some(Message::Fit), false),
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
                            text(trf(
                                "Nuova scansione della {name}",
                                &[("name", &self.page_name(*index))],
                            ))
                            .size(13),
                            button(text(tr("Tieni la nuova")).size(13))
                                .on_press(Message::AcceptRedo),
                            button(text(tr("Lascia la vecchia")).size(13))
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
            Purpose::Preview => tr("Anteprima").to_string(),
            Purpose::Measure => {
                let (i, n, dpi) = self
                    .measuring
                    .as_ref()
                    .map_or((0, 0, 0), |m| (m.total - m.left.len(), m.total, m.current));
                trf(
                    "Misura delle righe a {dpi} dpi ({i} di {n})",
                    &[("dpi", &dpi), ("i", &i), ("n", &n)],
                )
            }
            Purpose::Page => match self.kind {
                // "Rifai": the page done again, not the next one.
                _ if self.redo.is_some() => trf(
                    "Nuova scansione della {name}",
                    &[("name", &self.page_name(self.redo.unwrap_or_default()))],
                ),
                _ if self.job.as_ref().is_some_and(|j| j.crops.len() > 1) => trf(
                    "Scansione di {n} originali in una passata",
                    &[("n", &self.job.as_ref().map_or(0, |j| j.crops.len()))],
                ),
                _ if self.batch.1 > 1 => trf(
                    "Scansione {i} di {n}",
                    &[("i", &(self.batch.0 + 1)), ("n", &self.batch.1)],
                ),
                Kind::Document => trf("Scansione pagina {n}", &[("n", &(self.pages.len() + 1))]),
                Kind::Photo => tr("Scansione foto").to_string(),
            },
        };
        let (fraction, detail) = match stage {
            None | Some(Progress::Starting) => (0.02, tr("Avvio dello scanner")),
            Some(Progress::WarmingUp) => (0.05, tr("La lampada era spenta: riscaldamento")),
            Some(Progress::Calibrating) => (0.1, tr("Calibrazione")),
            Some(Progress::Scanning { .. }) => (0.0, tr("Lettura")),
            Some(Progress::Finishing) => (0.97, tr("Fine della scansione ed elaborazione")),
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
                    button(text(tr("Annulla")).size(12))
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
            tab(tr("Scansione"), Pane::Scan),
            tab(tr("Regola"), Pane::Adjust),
            tab(tr("Uscita"), Pane::Output),
            tab(tr("Preferenze"), Pane::Prefs),
        ]
        .spacing(2);
        if self.separating.is_some() {
            return container(
                column![
                    scrollable(container(self.split_pane()).padding([12, 16])).height(Length::Fill),
                    rule::horizontal(1),
                    container(self.split_footer()).padding(12),
                ]
                .height(Length::Fill),
            )
            .width(352)
            .height(Length::Fill)
            .style(surface)
            .into();
        }
        if self.editor().is_some() {
            return container(
                column![
                    scrollable(container(self.editor_pane()).padding([12, 16]))
                        .height(Length::Fill),
                    rule::horizontal(1),
                    container(
                        row![
                            button(text(tr("Scarta")).size(14).center().width(Length::Fill))
                                .width(Length::Fill)
                                .padding([9, 12])
                                .style(button::secondary)
                                .on_press(EditMsg::Discard.into()),
                            button(text(tr("Applica")).size(14).center().width(Length::Fill))
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
        // How the next scans are made, whatever the viewer shows.
        let s = *self.settings();
        let request = self.request();
        let (w, h) = (request.area.width, request.area.height);
        let (x, y, wmm, hmm) = self.area_mm();
        let memory = self.memory();
        let mut pane = column![
            self.access_card(),
            heading(tr("Originale")),
            labelled(
                tr("Risoluzione"),
                pick_list(
                    self.caps.resolutions_for(self.source()).to_vec(),
                    Some(self.dpi()),
                    Message::Dpi
                )
                .width(Length::Fill)
                .into()
            ),
            labelled(
                tr("Colore"),
                pick_list(ALL_COLOURS, Some(s.colour), Message::Colour)
                    .width(Length::Fill)
                    .into()
            ),
        ]
        .spacing(10);
        if let Some(holder) = self.holder_view().filter(|_| self.filming()) {
            pane = pane.push(holder);
        }
        // A page shown: what it is in is changed in its editor.
        if let Some(i) = self.shown_key().filter(|_| self.editing_page()) {
            pane = pane.push(
                text(trf(
                    "Valgono per le prossime scansioni: il colore della {name} si cambia in \
                     «Regola».",
                    &[("name", &self.page_name(i))],
                ))
                .size(11),
            );
        }
        // The depth only for a scanner that gives 16 bits.
        if self.offers_16bit() {
            pane = pane
                .push(labelled(
                    tr("Profondità"),
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
                        tr(
                            "16 bit: ombre e sfumature senza scalini anche dopo curve forti; \
                         più memoria.",
                        )
                    } else {
                        tr("8 bit: come il driver HP. Per foto e RAW meglio 16 bit.")
                    })
                    .size(11),
                );
        }
        // The scanner's stripes, where it declares them (films too: the
        // same sensor), and, on the glass only, twice the resolution, where
        // it offers it.
        {
            let scanned = if self.doubles() {
                self.double_dpi().unwrap_or(self.dpi())
            } else {
                self.dpi()
            };
            if self.stripes_at(self.source(), scanned).is_some() {
                pane = pane
                    .push(
                        checkbox(self.settings().destripe)
                            .label(tr("Togli le righe dello scanner"))
                            .on_toggle(Message::Destripe)
                            .text_size(13),
                    )
                    .push(
                        text(tr(
                            "Le righe chiare e scure che lo scanner lascia in ogni scansione: \
                             tolte dopo, senza tempo in più.",
                        ))
                        .size(11),
                    );
            }
            // Films: the noise of the colour, measured on each picture.
            if self.filming() {
                pane = pane
                    .push(
                        checkbox(self.settings().colour_noise)
                            .label(tr("Riduci il rumore di colore"))
                            .on_toggle(Message::ColourNoise)
                            .text_size(13),
                    )
                    .push(
                        text(tr(
                            "I puntini colorati nelle parti chiare e azzurre: misurati su ogni \
                             foto e ammorbiditi quanto serve, senza toccare i dettagli.",
                        ))
                        .size(11),
                    );
                if self.caps.colour_at(self.source()).is_some() {
                    pane = pane
                        .push(
                            checkbox(self.settings().scanner_colour)
                                .label(tr("Correggi i colori dello scanner"))
                                .on_toggle(Message::ScannerColour)
                                .text_size(13),
                        )
                        .push(
                            text(tr(
                                "La correzione che lo scanner dichiara per questa pellicola.                                  Senza, i colori come li legge il sensore.",
                            ))
                            .size(11),
                        );
                }
                if self.film_kind() == Film::Negative {
                    pane = pane.push(self.roll_view());
                }
            }
            if let Some(double) = self.double_dpi().filter(|_| !self.filming()) {
                pane = pane
                    .push(
                        checkbox(self.settings().double)
                            .label(trf("Scansiona a {dpi} dpi e riduci", &[("dpi", &double)]))
                            .on_toggle(Message::Double)
                            .text_size(13),
                    )
                    .push(
                        text(tr(
                            "Più fine, meno rumore e righe più deboli; circa il doppio del \
                             tempo e quattro volte la memoria durante la scansione.",
                        ))
                        .size(11),
                    );
            }
        }
        pane = pane.push(heading(tr("Area"))).push(
            checkbox(self.settings().auto_crop)
                .label(tr("Ritaglio automatico"))
                .on_toggle(Message::AutoCrop)
                .text_size(13),
        );
        let plan = self.plan();
        if !plan.is_empty() {
            let n = self.found.len();
            pane = pane.push(
                text(match n {
                    1 => tr("Un originale riconosciuto sul vetro: sarà ritagliato e raddrizzato.")
                        .to_string(),
                    n => trf(
                        "{n} originali riconosciuti sul vetro: ognuno diventa un'immagine, \
                         ritagliata e raddrizzata{how}.",
                        &[
                            ("n", &n),
                            (
                                "how",
                                &if plan.len() == 1 {
                                    tr(", in una passata sola")
                                } else {
                                    tr(", uno alla volta")
                                },
                            ),
                        ],
                    ),
                })
                .size(12),
            );
            let mm = 25.4 / f32::from(PREVIEW_DPI);
            for (i, f) in self.found.iter().enumerate() {
                let tilt = if f.angle == 0.0 {
                    tr("dritto").to_string()
                } else {
                    trf(
                        "storto di {deg}°",
                        &[("deg", &i18n::decimal(format!("{:.1}", f.angle.abs())))],
                    )
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
                        button(text(tr("Escludi")).size(11))
                            .padding([1, 6])
                            .style(button::text)
                            .on_press(Message::DropFound(i)),
                    ]
                    .align_y(Alignment::Center),
                );
            }
            pane = pane.push(
                text(trf(
                    "in memoria fino a {mb} MB",
                    &[("mb", &format!("{:.0}", memory / 1e6))],
                ))
                .size(12),
            );
        } else {
            if self.settings().auto_crop && self.preview.is_some() {
                pane = pane.push(
                    text(if self.selection.is_some() {
                        tr("La selezione ha la precedenza sul ritaglio automatico.")
                    } else {
                        tr(
                            "Nessun originale riconosciuto (un foglio bianco sul coperchio \
                         bianco può sfuggire): trascina sull'anteprima per scegliere l'area.",
                        )
                    })
                    .size(11),
                );
            }
            pane = pane
                .push(
                    text(if self.selection.is_none() && self.filming() {
                        tr(
                            "Tutto il supporto della pellicola. Senza anteprima: con il ritaglio \
                             automatico Scansiona trova i fotogrammi da sé, ognuno un'immagine.",
                        )
                    } else if self.selection.is_none() {
                        tr(
                            "Tutto il formato A4. Trascina sull'anteprima per scegliere un'area, \
                            o scrivila qui sotto.",
                        )
                    } else {
                        tr("Frecce: sposta l'area di 1 mm (Maiusc 10 mm, Alt 0,1 mm); \
                            Ctrl + frecce: la misura; Home: al centro del vetro.")
                    })
                    .size(11),
                )
                .push(self.mm_pair(
                    (tr("Da sinistra"), MmField::Area(0), x, true),
                    (tr("Dall'alto"), MmField::Area(1), y, true),
                ))
                .push(self.mm_pair(
                    (tr("Larghezza"), MmField::Area(2), wmm, true),
                    (tr("Altezza"), MmField::Area(3), hmm, true),
                ))
                .push(
                    text(trf(
                        "{w} × {h} pixel · in memoria {mb} MB",
                        &[
                            ("w", &w),
                            ("h", &h),
                            ("mb", &format!("{:.0}", memory / 1e6)),
                        ],
                    ))
                    .size(12),
                );
        }
        if self.settings().auto_crop && self.preview.is_some() {
            pane = pane.push(
                button(text(tr("Riconosci di nuovo")).size(12))
                    .padding([3, 8])
                    .style(button::secondary)
                    .on_press(Message::Detect),
            );
        }
        if memory > LARGE_SCAN {
            pane = pane.push(
                text(trf(
                    "Scansione molto grande: circa {gb} GB e alcuni minuti. \
                     Seleziona un'area o abbassa la risoluzione se non serve tutta.",
                    &[("gb", &i18n::decimal(format!("{:.1}", memory / 1e9)))],
                ))
                .size(12)
                .style(text::danger),
            );
        }
        if self.dpi() > 1200 && self.selection.is_none() && plan.is_empty() {
            pane = pane.push(
                text(tr(
                    "2400 dpi servono per piccoli originali: seleziona l'area sull'anteprima.",
                ))
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
            pane = pane.push(heading(tr("Tasti dello scanner"))).push(
                text(trf(
                    "{buttons} (si cambiano nelle Preferenze)",
                    &[("buttons", &what.join(" · "))],
                ))
                .size(12),
            );
        }
        pane.into()
    }

    fn adjust_pane(&self) -> Element<'_, Message> {
        let editing = if self.editing_page() {
            match (&self.redone, self.current) {
                (Some((i, _)), _) => trf(
                    "Nuova scansione della {name}",
                    &[("name", &self.page_name(*i))],
                ),
                (None, Some(i)) => self.page_title(i),
                (None, None) => String::new(),
            }
        } else {
            tr("Prossima scansione").to_string()
        };
        let scope: Element<'_, Message> = match (self.kind, self.editing_page()) {
            (Kind::Document, true) => {
                let own = self.shown().is_some_and(|p| p.own.is_some());
                let mut scope = column![
                    checkbox(own)
                        .label(tr("Solo questa pagina"))
                        .on_toggle(Message::OwnLook)
                        .text_size(13),
                    text(if own {
                        tr("Le modifiche valgono solo per questa pagina.")
                    } else {
                        tr(
                            "Segue le regolazioni comuni: le modifiche valgono anche per \
                         l'anteprima e per le pagine senza «Solo questa pagina».",
                        )
                    })
                    .size(11),
                ]
                .spacing(4);
                if self.pages.len() > 1 {
                    scope = scope.push(
                        button(text(tr("Applica a tutte le pagine")).size(12))
                            .style(button::text)
                            .padding([2, 6])
                            .on_press(Message::LookApplyToAll),
                    );
                }
                scope.into()
            }
            (Kind::Document, false) => text(tr(
                "Le modifiche valgono per l'anteprima, per le prossime scansioni \
                 e per le pagine senza «Solo questa pagina».",
            ))
            .size(11)
            .into(),
            (Kind::Photo, true) => {
                let own = self.shown().is_some_and(|p| p.own.is_some());
                let scope = column![text(if own {
                    tr("Regolazioni solo di questa foto.")
                } else {
                    tr(
                        "Segue le regolazioni comuni finché non ne cambi una: da lì valgono \
                         solo per questa foto.",
                    )
                })
                .size(11)]
                .spacing(4);
                let mut buttons = row![].spacing(4);
                if own {
                    buttons = buttons.push(
                        button(text(tr("Torna alle regolazioni comuni")).size(12))
                            .style(button::text)
                            .padding([2, 6])
                            .on_press(Message::OwnLook(false)),
                    );
                }
                if self.pages.len() > 1 || own {
                    buttons = buttons.push(
                        button(text(tr("Applica a tutte le foto")).size(12))
                            .style(button::text)
                            .padding([2, 6])
                            .on_press(Message::LookApplyToAll),
                    );
                }
                scope.push(buttons).into()
            }
            (Kind::Photo, false) => text(tr(
                "Le modifiche valgono per l'anteprima, per le prossime scansioni \
                 e per le foto che seguono le regolazioni comuni.",
            ))
            .size(11)
            .into(),
        };
        let mut top = column![text(editing.to_uppercase()).size(11), scope].spacing(10);
        // The colour mode of a page already scanned (the next scans have
        // theirs in "Scansione").
        if self.editing_page() {
            top = top.push(labelled(
                tr("Modo"),
                pick_list(ALL_COLOURS, Some(self.active().colour), Message::PageColour)
                    .width(Length::Fill)
                    .text_size(13)
                    .into(),
            ));
        }
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
                            text(trf(
                                "Neri tagliati {p}%",
                                &[("p", &i18n::decimal(format!("{black:.1}")))],
                            ))
                            .size(11)
                            .width(Length::Fill),
                            text(trf(
                                "Bianchi tagliati {p}%",
                                &[("p", &i18n::decimal(format!("{white:.1}")))],
                            ))
                            .size(11),
                        ],
                    ]
                    .spacing(4)
                    .into()
                }
                _ => text(tr("L'istogramma compare dopo l'anteprima"))
                    .size(11)
                    .into(),
            }
        };
        let colour = self.group(
            Group::Colour,
            tr("Colore"),
            s.colour == Colour::Rgb,
            tr("Solo per le scansioni a colori"),
            column![
                row![button(text(tr("Contagocce neutro")).size(12))
                    .style(if self.eyedropper {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press(Message::Eyedropper)],
                self.adjuster(Field::Temperature, &s),
                self.adjuster(Field::Tint, &s),
                self.adjuster(Field::Vibrance, &s),
                self.adjuster(Field::Saturation, &s),
            ]
            .spacing(8)
            .into(),
        );
        let tone = self.group(
            Group::Tone,
            tr("Tono"),
            !lineart,
            tr("In bianco e nero conta la soglia, qui sotto"),
            column![
                self.adjuster(Field::Exposure, &s),
                self.adjuster(Field::Contrast, &s),
                self.adjuster(Field::Highlights, &s),
                self.adjuster(Field::Shadows, &s),
                self.adjuster(Field::Whites, &s),
                self.adjuster(Field::Blacks, &s),
            ]
            .spacing(8)
            .into(),
        );
        let levels = self.group(
            Group::Levels,
            tr("Livelli"),
            !lineart,
            tr("In bianco e nero conta la soglia, qui sotto"),
            column![
                row![button(text(tr("Livelli auto")).size(12))
                    .style(button::secondary)
                    .on_press(Message::AutoLevels)],
                self.adjuster(Field::Black, &s),
                self.adjuster(Field::White, &s),
                self.adjuster(Field::Gamma, &s),
                self.adjuster(Field::Brightness, &s),
            ]
            .spacing(8)
            .into(),
        );
        let detail = self.group(
            Group::Detail,
            tr("Dettaglio"),
            true,
            "",
            column![
                self.adjuster(Field::Sharpen, &s),
                self.adjuster(Field::Noise, &s),
                labelled(
                    tr("Descreening"),
                    pick_list(SCREENS, Some(Screen(s.adjust.descreen)), Message::Descreen)
                        .width(Length::Fill)
                        .text_size(13)
                        .into()
                ),
                text(tr(
                    "Rimuove il retino di stampa di riviste e giornali (effetto moiré)."
                ))
                .size(11),
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
        let mut tabs = row![channel_tab(tr("Tutti"), CurveChannel::All)].spacing(4);
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
                .on_press(Message::CurveSet(c.points().to_vec(), None))
        };
        let curve_histogram = if self.editing_page() {
            self.shown().map(|p| &p.shown)
        } else {
            self.preview.as_ref().map(|(_, s, _)| s)
        }
        .and_then(|img| Histogram::of(img).levels());
        let curve_group = self.group(
            Group::Curve,
            tr("Curva"),
            !lineart,
            tr("Non serve in bianco e nero"),
            column![
                tabs,
                canvas(curve::Editor {
                    curve: s.adjust.curve[channel],
                    colour: curve_colour,
                    histogram: curve_histogram,
                    selected: self.curve_selected,
                    on_change: Message::CurveSet,
                    on_pick: Message::CurvePoint,
                })
                .width(Length::Fill)
                .height(220),
                row![
                    curve_preset(tr("Lineare"), scan_imaging::adjust::Curve::IDENTITY),
                    curve_preset(
                        tr("Contrasto medio"),
                        scan_imaging::adjust::Curve::MEDIUM_CONTRAST
                    ),
                    curve_preset(
                        tr("Contrasto forte"),
                        scan_imaging::adjust::Curve::STRONG_CONTRAST
                    ),
                ]
                .spacing(4),
                text(tr(
                    "Clic per aggiungere un punto (fino a 16), trascina per spostarlo, \
                     doppio clic o trascina fuori per toglierlo. Gli estremi sono il punto \
                     del nero e del bianco."
                ))
                .size(11),
            ]
            .spacing(6)
            .into(),
        );
        let bw = self.group(
            Group::Lineart,
            tr("Bianco e nero"),
            lineart,
            tr("Solo con il colore «Bianco e nero»"),
            column![
                button(text(tr("Soglia automatica")).size(12))
                    .style(button::secondary)
                    .on_press(Message::AutoThreshold),
                self.adjuster(Field::Threshold, &s),
                checkbox(s.adjust.adaptive)
                    .label(tr(
                        "Soglia adattiva (pagine illuminate in modo non uniforme)"
                    ))
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
            .placeholder(tr("Applica un preset…"))
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
                iced::widget::text_input(tr("Nome del preset"), &self.preset_name)
                    .on_input(Message::PresetName)
                    .on_submit(Message::PresetSave)
                    .size(13),
                button(text(tr("Salva")).size(13))
                    .style(button::secondary)
                    .on_press_maybe((!full).then_some(Message::PresetSave)),
            ]
            .spacing(6),
        );
        if full {
            presets = presets.push(
                text(tr(
                    "Tre preset salvati: scrivi il nome di uno per sostituirlo, o eliminane uno.",
                ))
                .size(11),
            );
        }
        for p in &self.presets {
            presets = presets.push(
                row![
                    text(&p.name).size(12).width(Length::Fill),
                    button(text(tr("Elimina")).size(11))
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
            colour,
            tone,
            levels,
            curve_group,
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
                text(tr(
                    "Lo scanner è collegato, ma Linux non lo lascia usare a questo utente."
                ))
                .size(13),
                text(trf(
                    "Basta una volta: installa la regola {rule} (chiede la password).",
                    &[("rule", &access::RULE_PATH_SHOWN)],
                ))
                .size(11),
                button(text(tr("Permetti l'accesso allo scanner")).size(13))
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
                    tr("Windows lascia i tasti a questa app.")
                } else {
                    tr("Windows apre anche la sua app di scansione a ogni tasto, \
                     e toglie lo scanner a questa.")
                })
                .size(11),
                button(
                    text(if quiet {
                        tr("Ridai i tasti a Windows")
                    } else {
                        tr("Usa i tasti dello scanner solo con questa app")
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
                text(tr("Chiede una volta la conferma di amministratore.")).size(11),
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
            col = col.push(heading(tr("Scanner"))).push(labelled(
                tr("Usa"),
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
                Kind::Document => tr("Tasti dello scanner in Documento"),
                Kind::Photo => tr("Tasti dello scanner in Foto"),
            }));
            for (i, b) in self.caps.buttons.iter().enumerate() {
                col = col.push(self.button_choice(i, b.label.clone()));
            }
            col = col
                .push(
                    text(tr(
                        "Valgono anche con l'app in secondo piano; ignorati durante una \
                         scansione.",
                    ))
                    .size(11),
                )
                .push(self.wia_row());
        }
        if self.caps.lamp.is_some() {
            col = col
                .push(heading(tr("Lampada")))
                .push(labelled(
                    tr("Spegnila"),
                    pick_list(
                        LAMP_CHOICES,
                        Some(LampAfter(self.lamp_off)),
                        Message::LampAfter,
                    )
                    .width(Length::Fill)
                    .into(),
                ))
                .push(
                    text(tr(
                        "Senza scansioni per questo tempo l'app spegne la lampada, e la \
                         spegne sempre chiudendosi. Riaccenderla costa circa 10 secondi di \
                         riscaldamento alla scansione dopo. Lo scanner la spegne comunque da \
                         solo dopo 13 minuti.",
                    ))
                    .size(11),
                );
        }
        col.into()
    }

    /// The choice of what a front-panel button does: the label above, so
    /// that the longest action ("Save the document and open the folder")
    /// has the whole width.
    fn button_choice(&self, index: usize, label: String) -> Element<'_, Message> {
        let options = ButtonAction::allowed(self.kind);
        let current = self.button_action(self.kind, index);
        column![
            text(label).size(13),
            pick_list(options, Some(current), move |a| {
                Message::ButtonAction(index, a)
            })
            .width(Length::Fill)
            .text_size(13),
        ]
        .spacing(4)
        .into()
    }

    fn prefs_pane(&self) -> Element<'_, Message> {
        let path = prefs::Prefs::path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        column![
            heading(tr("Lingua")),
            pick_list(
                i18n::LangChoice::ALL,
                Some(self.language),
                Message::Language
            )
            .width(Length::Fill)
            .text_size(13),
            heading(tr("All'avvio")),
            labelled(
                tr("Modo"),
                pick_list(
                    [StartMode::Last, StartMode::Document, StartMode::Photo],
                    Some(self.start_mode),
                    Message::StartMode
                )
                .width(Length::Fill)
                .into()
            ),
            text(tr(
                "Risoluzione, colore, formato e pagina PDF di ogni modo restano come li \
                 lasci. Le regolazioni ripartono da zero a ogni apertura: per ritrovarle, \
                 salvale in un preset."
            ))
            .size(12),
            heading(tr("Cartella dei file")),
            text(self.folder().display().to_string()).size(12),
            row![
                button(text(tr("Scegli…")).size(13))
                    .style(button::secondary)
                    .on_press(Message::ChooseFolder),
                button(text(tr("Apri")).size(13))
                    .style(button::secondary)
                    .on_press(Message::ShowFolder),
                button(
                    text(trf(
                        "Immagini{sep}Scansioni",
                        &[("sep", &std::path::MAIN_SEPARATOR)],
                    ))
                    .size(13),
                )
                .style(button::text)
                .on_press_maybe(
                    self.custom_folder
                        .is_some()
                        .then_some(Message::DefaultFolder)
                ),
            ]
            .spacing(8),
            heading(tr("Nomi dei file")),
            labelled(
                tr("Documenti"),
                iced::widget::text_input("documento", &self.document_name)
                    .on_input(Message::DocumentName)
                    .size(13)
                    .into()
            ),
            labelled(
                tr("Foto"),
                iced::widget::text_input("foto", &self.photo_name)
                    .on_input(Message::PhotoName)
                    .size(13)
                    .into()
            ),
            text(trf(
                "Esempio: {name}",
                &[(
                    "name",
                    &Self::stamped(self.name(), self.settings().output.extension()),
                )],
            ))
            .size(12),
            self.scanner_prefs(),
            self.stripes_prefs(),
            heading(tr("Impostazioni")),
            text(trf("Salvate in {path}", &[("path", &path)])).size(11),
            button(text(tr("Ripristina impostazioni di fabbrica")).size(13))
                .style(button::danger)
                .on_press(Message::FactoryReset),
        ]
        .spacing(10)
        .into()
    }

    /// "Righe dello scanner": the stripes known for the scanner in use, at
    /// each resolution, and "Misura le righe".
    fn stripes_prefs(&self) -> Element<'_, Message> {
        let mut pane = column![
            heading(tr("Righe dello scanner")),
            text(tr(
                "Righe chiare e scure che alcuni scanner lasciano in ogni scansione, sempre \
                 uguali: misurate una volta su un foglio bianco, «Togli le righe dello \
                 scanner» le toglie.",
            ))
            .size(11),
        ]
        .spacing(6);
        let measured = self.measured.get(&self.caps.name());
        for dpi in self.measure_resolutions() {
            let line = match measured.and_then(|m| m.iter().find(|s| s.dpi == dpi)) {
                Some(s) if s.repeat == 0 => trf("{dpi} dpi: nessuna (misurate)", &[("dpi", &dpi)]),
                Some(s) => trf(
                    "{dpi} dpi: ogni {n} righe, {pct} % (misurate)",
                    &[
                        ("dpi", &dpi),
                        ("n", &s.repeat),
                        ("pct", &i18n::decimal(format!("{:.1}", s.percent))),
                    ],
                ),
                None => match self.caps.stripes_at(Source::Glass, dpi) {
                    Some(n) => trf(
                        "{dpi} dpi: ogni {n} righe (dal modello)",
                        &[("dpi", &dpi), ("n", &n)],
                    ),
                    None => trf("{dpi} dpi: non misurate", &[("dpi", &dpi)]),
                },
            };
            pane = pane.push(text(line).size(12));
        }
        let idle = self.busy.is_none() && self.worker.is_some();
        let mut buttons = row![button(text(tr("Misura le righe…")).size(13))
            .style(button::secondary)
            .on_press_maybe(idle.then_some(Message::MeasureStripes))]
        .spacing(6);
        if measured.is_some() {
            buttons = buttons.push(
                button(text(tr("Quelle del modello")).size(13))
                    .style(button::secondary)
                    .on_press_maybe(idle.then_some(Message::ForgetStripes)),
            );
        }
        pane.push(buttons).into()
    }

    fn output_pane(&self) -> Element<'_, Message> {
        // The formats of the colour of the scans, as in the panel Scansione.
        let s = *self.settings();
        let options = outputs(self.kind, s.colour, self.depth());
        let mut pane = column![
            heading(tr("Formato")),
            pick_list(options, Some(self.settings().output), Message::Output).width(Length::Fill),
        ]
        .spacing(10);
        if s.colour == Colour::Lineart {
            pane = pane.push(
                text(tr(
                    "Bianco e nero: niente JPEG/AVIF, rovinerebbero i bordi.",
                ))
                .size(11),
            );
        }
        if let Output::Raw(_) = self.settings().output {
            pane = pane.push(
                text(tr(
                    "I campioni dello scanner, lineari e senza regolazioni (ritaglio e \
                     raddrizzatura sì): si sviluppano in Lightroom, Camera Raw o Photoshop. \
                     Le regolazioni a destra valgono solo per l'anteprima.",
                ))
                .size(11),
            );
        } else if self.kind == Kind::Photo && s.colour == Colour::Rgb {
            pane = pane.push(
                text(tr(
                    "Per Lightroom o Photoshop scegli un formato RAW (DNG o TIFF 16 bit).",
                ))
                .size(11),
            );
        }
        if self.settings().output == Output::Pdf {
            let s = *self.settings();
            pane = pane.push(labelled(
                tr("Foglio"),
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
                            .label(trf(
                                "Impaginazione propria per la {name}",
                                &[("name", &self.page_name(i))],
                            ))
                            .on_toggle(Message::PdfOwnLayout)
                            .text_size(13),
                    );
                }
                pane = pane.push(
                    text(if own {
                        tr("Le scelte qui sotto valgono solo per questa pagina.")
                    } else {
                        tr("Le scelte qui sotto valgono per tutte le pagine senza impaginazione propria.")
                    })
                    .size(11),
                );
                pane = pane.push(labelled(
                    tr("Orientamento"),
                    pick_list(SHEETS, Some(l.sheet), Message::PdfSheet)
                        .width(Length::Fill)
                        .into(),
                ));
                pane = pane.push(labelled(
                    tr("Sul foglio"),
                    pick_list(PDF_FITS, Some(l.fit), Message::PdfFit)
                        .width(Length::Fill)
                        .into(),
                ));
                if l.fit != PdfFit::Actual {
                    pane = pane.push(labelled(
                        tr("Margine"),
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
                            button(text(tr("Applica a tutte le pagine")).size(13))
                                .style(button::secondary)
                                .on_press(Message::PdfApplyToAll),
                            button(text(tr("Come le altre")).size(13))
                                .style(button::text)
                                .on_press(Message::PdfOwnLayout(false)),
                        ]
                        .spacing(8),
                    );
                }
                if let Some(page) = self.current.and_then(|i| self.pages.get(i)) {
                    let (w, h) = page.inches();
                    let placed = s.layout(page.layout, page.focus).place(w, h);
                    let [left, top] = placed.offset_mm();
                    let [rx, ry] = placed.room();
                    pane = pane
                        .push(heading(tr("Posizione sul foglio")))
                        .push(self.mm_pair(
                            (tr("Da sinistra"), MmField::Offset(0), left, rx.abs() > 0.5),
                            (tr("Dall'alto"), MmField::Offset(1), top, ry.abs() > 0.5),
                        ));
                    let mut buttons = row![button(text(tr("Centra")).size(12))
                        .padding([3, 8])
                        .style(button::secondary)
                        .on_press_maybe(
                            (page.focus != [0.5, 0.5]).then_some(Message::PageFocus([0.5, 0.5]))
                        )]
                    .spacing(8);
                    if self.pages.len() > 1 {
                        buttons = buttons.push(
                            button(text(tr("Uguale per tutte le pagine")).size(12))
                                .padding([3, 8])
                                .style(button::text)
                                .on_press_maybe(
                                    self.pages
                                        .iter()
                                        .any(|p| p.focus != page.focus)
                                        .then_some(Message::PageFocusToAll),
                                ),
                        );
                    }
                    pane = pane.push(buttons).push(
                        text(tr(
                            "Trascina la pagina sul foglio o spostala con le frecce (Maiusc \
                             10 mm, Alt 0,1 mm); Home la centra.",
                        ))
                        .size(11),
                    );
                }
                pane = pane.push(
                    text(match l.fit {
                        PdfFit::Actual => {
                            tr("Alla sua misura vera, al centro del foglio (rimpicciolita solo se non ci sta): stampata al 100% misura come l'originale.")
                        }
                        PdfFit::Page => {
                            tr("Grande quanto il foglio permette, intera, dentro il margine: un ritaglio piccolo riempie l'A4.")
                        }
                        PdfFit::Fill => {
                            tr("Copre il foglio dentro il margine; quello che avanza si taglia: trascina la pagina sul foglio, a sinistra, per scegliere cosa tenere.")
                        }
                    })
                    .size(11),
                );
                pane = pane.push(
                    text(
                        tr("Il foglio con la pagina sopra è nel visore: in rosso il bordo di quello che finisce sulla carta."),
                    )
                    .size(11),
                );
            }
        }
        if self.settings().output == Output::Pdf {
            pane = pane.push(self.pdf_compression());
        }
        pane = pane
            .push(heading(tr("Cartella")))
            .push(text(self.folder().display().to_string()).size(12));
        if self.kind == Kind::Photo {
            pane = pane.push(
                checkbox(self.settings().auto_save)
                    .label(tr("Salva da sola ogni foto nella cartella"))
                    .on_toggle(Message::AutoSave)
                    .text_size(13),
            );
        }
        if self.kind == Kind::Document {
            pane = pane.push(
                button(text(tr("Nuovo documento")).size(13))
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
                trf("Salva {output}…", &[("output", &output)]),
                !self.pages.is_empty() && self.busy.is_none(),
            ),
            Kind::Photo => (
                trf("Salva {output}…", &[("output", &output)]),
                self.shown().is_some() && self.busy.is_none(),
            ),
        };
        // Saving, and next to it the folder the files go to.
        row![
            button(text(label).size(14).center().width(Length::Fill))
                .width(Length::Fill)
                .padding([9, 12])
                .on_press_maybe(enabled.then_some(Message::Save)),
            button(text(tr("Apri la cartella")).size(13).center())
                .style(button::secondary)
                .padding([10, 10])
                .on_press(Message::ShowFolder),
        ]
        .spacing(8)
        .into()
    }

    /// Facts at the bottom: scanner, area, resolution, last note.
    fn status_bar(&self) -> Element<'_, Message> {
        let s = self.settings();
        let request = self.request();
        let link = match &self.status {
            // The scanner, and what it says first about itself ("USB 2.0").
            Some(Ok(st)) => {
                let detail = st
                    .details
                    .first()
                    .map_or_else(|| tr("collegato").to_string(), |(l, v)| format!("{l} {v}"));
                format!("{} · {detail}", self.caps.name())
            }
            Some(Err(_)) => tr("non collegato").to_string(),
            None => "…".to_string(),
        };
        let fact = |label: &'static str, value: String| {
            row![text(label).size(11), text(value).size(11)].spacing(4)
        };
        container(
            row![
                fact(tr("Scanner"), link),
                fact(
                    tr("Risoluzione"),
                    format!("{} dpi · {}", self.dpi(), s.colour)
                ),
                fact(
                    tr("Immagine"),
                    format!("{} × {}", request.area.width, request.area.height)
                ),
                text(self.note.clone()).size(11).width(Length::Fill),
                text(tr(
                    "Rotella zoom · tasto destro sposta · doppio clic su un cursore lo azzera"
                ))
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

    /// Two fields of millimetres side by side, each a label, the field
    /// (being typed, or the value) and "mm"; a field not `enabled` cannot
    /// be typed in.
    fn mm_pair<'a>(
        &self,
        a: (&'a str, MmField, f64, bool),
        b: (&'a str, MmField, f64, bool),
    ) -> Element<'a, Message> {
        let field = |(label, field, value, enabled): (&'a str, MmField, f64, bool)| {
            let shown = match &self.mm_draft {
                Some((f, typed)) if *f == field => typed.clone(),
                _ => i18n::decimal(keys::format_mm(value)),
            };
            column![
                text(label).size(12),
                row![
                    iced::widget::text_input("", &shown)
                        .on_input_maybe(enabled.then_some(move |t| Message::MmInput(field, t)))
                        .on_submit(Message::MmSubmit)
                        .size(12)
                        .padding([2, 6])
                        .width(Length::Fill)
                        .align_x(iced::alignment::Horizontal::Right),
                    text("mm").size(12),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            ]
            .spacing(2)
            .width(Length::Fill)
        };
        row![field(a), field(b)].spacing(12).into()
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
            button(text(tr("Ripristina")).size(12))
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
            .appname(APP_NAME)
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
            let area = within(
                area_mm(dpi, 150.0, 240.0, 66.9, 58.2),
                caps.glass_pixels(dpi),
            );
            assert!(area.x + area.width <= max_w, "{dpi} dpi: {area:?}");
            assert!(area.y + area.height <= max_h, "{dpi} dpi: {area:?}");
            assert!(area.width > 0 && area.height > 0);
            // An area inside is left alone.
            let inside = area_mm(dpi, 10.0, 10.0, 100.0, 100.0);
            assert_eq!(within(inside, caps.glass_pixels(dpi)), inside);
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
