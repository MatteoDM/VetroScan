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

mod curve;
mod histogram;
mod prefs;
mod viewer;
mod wia;
mod worker;

use histogram::Histogram;
use hp3770_core::adjust::{self, Adjustments};
use hp3770_core::convert;
use hp3770_core::detect::{self, Found};
use hp3770_core::geometry::{Area, GLASS_INCHES, HP_WHITE, RESOLUTIONS};
use hp3770_core::profile::HP3770;
use hp3770_core::session::{self, Format, Image, Request, Stage};
use hp3770_core::status::Status;
use hp3770_export::{ImageFormat, PageSize};
use iced::widget::{
    button, canvas, checkbox, column, container, pick_list, progress_bar, row, rule, scrollable,
    slider, text, Space,
};
use iced::{Alignment, Element, Length, Point, Size, Subscription, Task, Theme};
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
        .window_size(Size::new(1360.0, 900.0))
        .run()
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
fn outputs(kind: Kind, colour: Colour) -> Vec<Output> {
    let all: Vec<Output> = match kind {
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
            Output::Raw(RawFormat::Tiff16),
            Output::Raw(RawFormat::Dng),
        ],
    };
    all.into_iter()
        .filter(|o| colour != Colour::Lineart || !o.lossy())
        .filter(|o| colour == Colour::Rgb || !matches!(o, Output::Raw(_)))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PdfSize {
    Fit,
    A4,
    Letter,
}

impl std::fmt::Display for PdfSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Fit => "Pagina come la scansione",
            Self::A4 => "A4",
            Self::Letter => "Letter",
        })
    }
}

impl From<PdfSize> for PageSize {
    fn from(s: PdfSize) -> Self {
        match s {
            PdfSize::Fit => PageSize::Fit,
            PdfSize::A4 => PageSize::A4,
            PdfSize::Letter => PageSize::Letter,
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
            Self::System => "Tema di sistema",
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
    auto_save: bool,
    /// Find the originals on the glass (photos, cards, receipts) and scan
    /// each one straightened, as its own image.
    auto_crop: bool,
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
            auto_save: false,
            auto_crop: false,
        }
    }

    fn photo() -> Self {
        Self {
            dpi: 600,
            output: Output::Image(ImageFormat::Jpeg { quality: 92 }),
            auto_crop: true,
            ..Self::document()
        }
    }

    fn mode(&self) -> session::Mode {
        match self.colour {
            Colour::Rgb => session::Mode::Colour,
            Colour::Grey => session::Mode::Grey,
            Colour::Lineart => session::Mode::Lineart {
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
                Output::Image(ImageFormat::Bmp) => "bmp",
            }
            .into(),
            pdf_size: match self.pdf_size {
                PdfSize::Fit => "scansione",
                PdfSize::A4 => "a4",
                PdfSize::Letter => "letter",
            }
            .into(),
            auto_save: self.auto_save,
            auto_crop: Some(self.auto_crop),
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
            _ => base.output,
        };
        let allowed = outputs(kind, colour);
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
            dpi: if RESOLUTIONS.contains(&m.dpi) {
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
                        .map(|p| hp3770_core::adjust::Curve::new(p))
                        .unwrap_or(hp3770_core::adjust::Curve::IDENTITY)
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
            pdf_size: match m.pdf_size.as_str() {
                "a4" => PdfSize::A4,
                "letter" => PdfSize::Letter,
                _ => PdfSize::Fit,
            },
            auto_save: m.auto_save,
            auto_crop: m.auto_crop.unwrap_or(base.auto_crop),
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

    /// The actions of `stored` for `kind`, or the defaults.
    fn from_keys(kind: Kind, stored: &[String]) -> [ButtonAction; 3] {
        let (allowed, default): (&[ButtonAction], [ButtonAction; 3]) = match kind {
            Kind::Document => (
                &Self::DOCUMENT,
                [Self::AddPage, Self::RedoLast, Self::SaveDocument],
            ),
            Kind::Photo => (
                &Self::PHOTO,
                [Self::ScanPhoto, Self::Photocopy, Self::OpenFolder],
            ),
        };
        std::array::from_fn(|i| {
            stored
                .get(i)
                .and_then(|k| allowed.iter().copied().find(|a| a.key() == k))
                .unwrap_or(default[i])
        })
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

/// The scanner's linear colour samples of a scan, and a smaller copy for
/// live adjustments.
#[derive(Debug, Clone)]
struct Raw {
    full: Arc<Image>,
    proxy: Arc<Image>,
}

impl Raw {
    fn new(image: Image) -> Self {
        let proxy = shrink(&image, PROXY);
        Self {
            full: Arc::new(image),
            proxy: Arc::new(proxy),
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

/// The adjusted image of a raw scan (on the proxy, or at full size).
fn render(raw: &Image, settings: &Settings, adjust: &Adjustments) -> Image {
    adjust::render(
        &raw.data,
        raw.width,
        raw.height,
        raw.dpi,
        settings.mode(),
        adjust,
        false,
        &HP3770,
    )
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
/// of its way of working, unless it has a look of its own.
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
}

impl Page {
    fn new(raw: Raw, settings: Settings) -> Self {
        let shown = render(&raw.proxy, &settings, &settings.adjust);
        let rgb = shown.to_rgb();
        Self {
            picture: Picture::from_rgb(shown.width, shown.height, &rgb),
            thumb: Picture::from_rgb_max(shown.width, shown.height, &rgb, 240),
            raw,
            settings,
            own: None,
            shown,
            origin: None,
        }
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
        self.shown = render(&self.raw.proxy, &settings, &adjust);
        let rgb = self.shown.to_rgb();
        self.picture =
            Picture::from_rgb(self.shown.width, self.shown.height, &rgb).replacing(&self.picture);
        self.thumb = Picture::from_rgb_max(self.shown.width, self.shown.height, &rgb, 240);
    }

    /// The finished image at full resolution.
    fn finished(&self, shared: &Settings) -> Image {
        let settings = self.effective(shared);
        render(&self.raw.full, &settings, &settings.adjust)
    }

    fn rotate(&mut self, clockwise: bool, shared: &Settings) {
        let turns = if clockwise { 1 } else { 3 };
        self.raw = Raw {
            full: Arc::new(self.raw.full.rotated(turns)),
            proxy: Arc::new(self.raw.proxy.rotated(turns)),
        };
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
    PresetName(String),
    PresetSave,
    PresetApply(String),
    PresetDelete(String),
    Key(iced::keyboard::Event),
    StartMode(StartMode),
    /// A button's action in the current way of working: which button and
    /// what it does.
    ButtonAction(usize, ButtonAction),
    /// Make Windows leave the scanner buttons alone (or not).
    WiaQuiet(bool),
    WiaDone(Result<(), String>),
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
    Colour(Colour),
    Output(Output),
    PdfSize(PdfSize),
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
    PageRedo(usize),
    AcceptRedo,
    RejectRedo,
    NewDocument,
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
    status: Option<Result<Status, String>>,
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
    document_buttons: [ButtonAction; 3],
    photo_buttons: [ButtonAction; 3],
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
    busy: Option<(Purpose, Option<Stage>)>,
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
                system_dark: false,
                theme,
                kind,
                document: Settings::from_stored(Kind::Document, &stored.document),
                photo: Settings::from_stored(Kind::Photo, &stored.photo),
                tab: Tab::Preview,
                pane: Pane::Scan,
                start_mode,
                custom_folder: (!stored.folder.is_empty()).then(|| PathBuf::from(&stored.folder)),
                document_name: stored.document_name.clone(),
                photo_name: stored.photo_name.clone(),
                document_buttons: ButtonAction::from_keys(Kind::Document, &stored.document_buttons),
                photo_buttons: ButtonAction::from_keys(Kind::Photo, &stored.photo_buttons),
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
        ])
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
        let shown = self.current;
        if self.shown().is_some_and(|p| p.own.is_none()) {
            self.dirty_page = true;
        }
        self.stale = self
            .pages
            .iter()
            .enumerate()
            .filter(|(i, p)| p.own.is_none() && Some(*i) != shown)
            .map(|(i, _)| i)
            .collect();
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
        let entry = if self.editing_own() {
            (Target::Page, self.active())
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
            Target::Page if self.editing_own() => {
                let shared = *self.settings();
                let page = self.shown_mut()?;
                let current = page.effective(&shared);
                page.own = Some(Look::of(&settings));
                page.settings.colour = settings.colour;
                self.dirty_page = true;
                Some((Target::Page, current))
            }
            Target::Preview => {
                let current = *self.settings();
                *self.settings_mut() = settings;
                self.shared_changed();
                Some((Target::Preview, current))
            }
            _ => None,
        }
    }

    /// A change to what the panel edits. On a page with its own look, that
    /// look; otherwise the shared settings, which every page following
    /// them shows. The colour mode of a page shown stays its own.
    fn apply(&mut self, f: impl Fn(&mut Settings)) {
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
            Target::Preview => self
                .preview
                .as_ref()
                .map(|(raw, ..)| (raw.proxy.clone(), shared)),
            Target::Page => self
                .shown()
                .map(|p| (p.raw.proxy.clone(), p.effective(&shared))),
            Target::Index(i) => self
                .pages
                .get(i)
                .filter(|p| p.own.is_none())
                .map(|p| (p.raw.proxy.clone(), p.effective(&shared))),
        };
        let Some((proxy, settings)) = job else {
            return Task::none();
        };
        self.rendering = true;
        Task::perform(
            async move {
                let adjust = if before {
                    Adjustments::default()
                } else {
                    settings.adjust
                };
                let shown = render(&proxy, &settings, &adjust);
                let rgb = shown.to_rgb();
                let picture = Picture::from_rgb(shown.width, shown.height, &rgb);
                let thumb = (target != Target::Preview)
                    .then(|| Picture::from_rgb_max(shown.width, shown.height, &rgb, 240));
                Arc::new(Rendered {
                    target,
                    shown,
                    picture,
                    thumb,
                })
            },
            Message::Rendered,
        )
    }

    fn redraw_preview(&mut self) {
        let settings = *self.settings();
        let adjust = if self.before {
            Adjustments::default()
        } else {
            settings.adjust
        };
        if let Some((raw, shown, picture)) = &mut self.preview {
            *shown = render(&raw.proxy, &settings, &adjust);
            *picture =
                Picture::from_rgb(shown.width, shown.height, &shown.to_rgb()).replacing(picture);
        }
    }

    /// Size of the glass in preview pixels.
    fn glass() -> Size {
        let dpi = f32::from(PREVIEW_DPI);
        Size::new(
            (GLASS_INCHES.0 as f32 * dpi).round(),
            (GLASS_INCHES.1 as f32 * dpi).round(),
        )
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
    fn request(&self) -> Request {
        self.request_mm(self.area_mm())
    }

    /// The scan of an area given in millimetres.
    fn request_mm(&self, (x, y, w, h): (f64, f64, f64, f64)) -> Request {
        let dpi = self.settings().dpi;
        Request {
            linear: true,
            adjust: Adjustments {
                sharpen: 0.0,
                ..Adjustments::default()
            },
            ..Request::new(dpi, Area::from_mm(dpi, x, y, w, h), session::Mode::Colour)
        }
    }

    /// The passes for the originals found, when the automatic crop applies
    /// (it on, something found, no selection): all in one pass, or each
    /// on its own when one pass would take too much memory.
    fn plan(&self) -> Vec<Job> {
        if !self.settings().auto_crop || self.selection.is_some() || self.found.is_empty() {
            return Vec::new();
        }
        let glass = Self::glass();
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
    fn job_request(&self, job: &Job) -> Request {
        let mm = |px: f32| f64::from(px) * 25.4 / f64::from(PREVIEW_DPI);
        let [x, y, w, h] = job.bounds;
        self.request_mm((mm(x), mm(y), mm(w), mm(h)))
    }

    fn job_memory(&self, job: &Job) -> f64 {
        Self::bytes(&self.job_request(job), self.settings().dpi)
    }

    fn bytes(request: &Request, dpi: u16) -> f64 {
        let a = request.area;
        let raw = 3.0 * f64::from(a.width) * f64::from(a.height);
        if dpi > 1200 {
            raw * 2.0
        } else {
            raw
        }
    }

    /// Bytes the scan takes in memory.
    fn memory(&self) -> f64 {
        let plan = self.plan();
        if !plan.is_empty() {
            return plan.iter().map(|j| self.job_memory(j)).fold(0.0, f64::max);
        }
        let a = self.request().area;
        let raw = 3.0 * f64::from(a.width) * f64::from(a.height);
        if self.settings().dpi > 1200 {
            raw * 2.0
        } else {
            raw
        }
    }

    fn start(&mut self, purpose: Purpose) {
        let request = if purpose == Purpose::Preview {
            let glass = Self::glass();
            Request {
                area: Area {
                    x: 0,
                    y: 0,
                    width: glass.width as u32,
                    height: glass.height as u32,
                },
                dpi: PREVIEW_DPI,
                ..self.request()
            }
        } else if let Some(job) = self.next_job() {
            let mut request = self.job_request(&job);
            self.job = Some(Job {
                area: Some(request.area),
                ..job
            });
            request.dpi = self.settings().dpi;
            request
        } else {
            self.batch = (0, 1);
            self.request()
        };
        if let (Some(worker), None) = (&self.worker, &self.busy) {
            worker.send(Command::Scan(purpose, Box::new(request)));
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
    fn cut(image: Image, job: Option<&Job>) -> Vec<(Image, Option<Found>)> {
        let Some((job, area)) = job.and_then(|j| Some((j, j.area?))) else {
            return vec![(image, None)];
        };
        let factor = f32::from(image.dpi) / f32::from(PREVIEW_DPI);
        let inset = INSET_MM / 25.4 * f32::from(image.dpi);
        job.crops
            .iter()
            .map(|c| {
                let mut f = c.scaled(factor, -(area.x as f32), -(area.y as f32));
                f.width = (f.width - 2.0 * inset).max(1.0);
                f.height = (f.height - 2.0 * inset).max(1.0);
                (image.straightened(&f), Some(*c))
            })
            .collect()
    }

    /// The usual folder of saved scans: Pictures\Scansioni.
    fn default_folder() -> PathBuf {
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
            document_buttons: self
                .document_buttons
                .iter()
                .map(|a| a.key().into())
                .collect(),
            photo_buttons: self.photo_buttons.iter().map(|a| a.key().into()).collect(),
            document: self.document.to_stored(),
            photo: self.photo.to_stored(),
        }
    }

    /// A file name with the date and time.
    fn stamped(prefix: &str, extension: &str) -> String {
        format!(
            "{prefix}-{}.{extension}",
            chrono::Local::now().format("%Y-%m-%d-%H%M%S")
        )
    }

    /// Writes the document (all pages) or the shown photo to `path`.
    fn write(&self, path: &std::path::Path) -> Result<String, String> {
        let settings = self.settings();
        if let Output::Raw(format) = settings.output {
            return self.write_raw(path, format);
        }
        let pages: Vec<Image> = self.pages.iter().map(|p| p.finished(settings)).collect();
        if pages.is_empty() {
            return Err("niente da salvare".into());
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("pagina")
            .to_string();
        let result = match extension.as_str() {
            "pdf" => hp3770_export::save_pdf(path, &pages, settings.pdf_size.into(), 88),
            "tif" | "tiff" if pages.len() > 1 => hp3770_export::save_tiff(path, &pages),
            "zip" => hp3770_export::save_zip(path, &name, &pages, ImageFormat::Png),
            _ if pages.len() > 1 => {
                // One image per page, next to the chosen name.
                let folder = path.parent().unwrap_or(std::path::Path::new("."));
                let format = ImageFormat::from_path(path).unwrap_or(ImageFormat::Png);
                hp3770_export::save_folder(folder, &name, &pages, format).map(|_| ())
            }
            _ => hp3770_export::save(path, &pages[0]),
        };
        result
            .map(|()| format!("{} pagine in {}", pages.len(), path.display()))
            .map_err(|e| e.to_string())
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
            let raw = hp3770_export::Linear16::from_linear8(&page.raw.full, HP_WHITE)
                .map_err(|e| e.to_string())?;
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
        let task = self.handle(message);
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
        match message {
            Message::Worker(event) => self.on_worker(event),
            Message::Rendered(done) => {
                self.rendering = false;
                let Ok(Rendered {
                    target,
                    shown,
                    picture,
                    thumb,
                }) = Arc::try_unwrap(done)
                else {
                    return Task::none();
                };
                match target {
                    Target::Preview => {
                        if let Some((_, old_shown, old)) = &mut self.preview {
                            *old = picture.replacing(old);
                            *old_shown = shown;
                        }
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
                if self.kind != kind {
                    self.kind = kind;
                    self.redone = None;
                    if kind == Kind::Photo {
                        // A photo at a time: keep the last scan only.
                        let last = self.pages.pop();
                        self.pages.clear();
                        self.pages.extend(last);
                        self.current = (!self.pages.is_empty()).then_some(0);
                    }
                    self.redraw_preview();
                }
            }
            Message::Tab(tab) => self.tab = tab,
            Message::Pane(pane) => self.pane = pane,
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
            Message::ButtonAction(i, action) => match self.kind {
                Kind::Document => self.document_buttons[i] = action,
                Kind::Photo => self.photo_buttons[i] = action,
            },
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
                self.document_buttons = ButtonAction::from_keys(Kind::Document, &[]);
                self.photo_buttons = ButtonAction::from_keys(Kind::Photo, &[]);
                self.theme = ThemeChoice::System;
                self.redraw_preview();
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
            Message::Colour(colour) => {
                let kind = self.kind;
                self.change(|s| {
                    s.colour = colour;
                    if !outputs(kind, colour).contains(&s.output) {
                        s.output = outputs(kind, colour)[0];
                    }
                });
            }
            Message::Output(output) => self.settings_mut().output = output,
            Message::PdfSize(size) => self.settings_mut().pdf_size = size,
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
                let curve = hp3770_core::adjust::Curve::new(&points);
                self.apply(|s| s.adjust.curve[c] = curve);
            }
            Message::Descreen(screen) => self.change(|s| s.adjust.descreen = screen.0),
            Message::Adaptive(on) => self.change(|s| s.adjust.adaptive = on),
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
                if let Some(raw) = self.active_raw() {
                    let settings = self.active();
                    let base = Adjustments {
                        black: 0.0,
                        white: 255.0,
                        sharpen: 0.0,
                        ..settings.adjust
                    };
                    let shown = render(&raw.proxy, &settings, &base);
                    let (black, white) = adjust::auto_levels(&shown);
                    self.change(|s| {
                        s.adjust.black = black;
                        s.adjust.white = white;
                    });
                }
            }
            Message::AutoThreshold => {
                if let Some(raw) = self.active_raw() {
                    let settings = Settings {
                        colour: Colour::Grey,
                        ..self.active()
                    };
                    let grey = render(&raw.proxy, &settings, &settings.adjust);
                    let t = adjust::auto_threshold(&grey.data);
                    self.change(|s| s.threshold = t);
                }
            }
            Message::Eyedropper => self.eyedropper = !self.eyedropper,
            Message::Picked(point) => {
                self.eyedropper = false;
                if let Some(raw) = self.active_raw() {
                    // The viewer gives full-size coordinates.
                    let scale = raw.proxy.width as f32 / raw.full.width.max(1) as f32;
                    let sample = raw.sample(Point::new(point.x * scale, point.y * scale));
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
                let shared = *self.settings();
                if let Some(p) = self.shown_mut() {
                    p.redraw(on, &shared);
                }
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
                let file = Self::stamped(prefix, ext);
                let folder = self.folder();
                let _ = std::fs::create_dir_all(&folder);
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
                self.note = match self.write(&path) {
                    Ok(what) => format!("Salvato: {what}"),
                    Err(e) => format!("Salvataggio non riuscito: {e}"),
                };
            }
            Message::SaveTo(None) => {}
            Message::PageShow(i) => {
                self.current = Some(i);
                self.redone = None;
                self.result_view = View::default();
                self.tab = Tab::Result;
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
                if j < self.pages.len() {
                    self.pages.swap(i, j);
                    self.current = Some(j);
                }
            }
            Message::PageDelete(i) => {
                if i < self.pages.len() {
                    self.pages.remove(i);
                    self.redone = None;
                    self.current = (!self.pages.is_empty()).then(|| i.min(self.pages.len() - 1));
                    self.note = format!("Pagina {} eliminata", i + 1);
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
                        self.note = format!("Pagina {} sostituita", index + 1);
                    }
                }
            }
            Message::RejectRedo => {
                if let Some((index, _)) = self.redone.take() {
                    self.current = Some(index);
                    self.note = format!("Pagina {} lasciata com'era", index + 1);
                }
            }
            Message::NewDocument => {
                self.pages.clear();
                self.current = None;
                self.redone = None;
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
        let message = match (key.as_ref(), modifiers.command(), modifiers.shift()) {
            (Key::Character("z"), true, false) => Some(Message::Undo),
            (Key::Character("z"), true, true) | (Key::Character("y"), true, _) => {
                Some(Message::Redo)
            }
            (Key::Character("p"), true, _) => Some(Message::Preview),
            (Key::Named(Named::Enter), true, _) => Some(Message::Scan),
            (Key::Character("s"), true, _) => Some(Message::Save),
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

    /// The raw scan the panel acts on.
    fn active_raw(&self) -> Option<Raw> {
        if self.editing_page() {
            self.shown().map(|p| p.raw.clone())
        } else {
            self.preview.as_ref().map(|(raw, ..)| raw.clone())
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
            }
            Event::Status(status) => self.status = Some(status),
            Event::Progress(stage) => {
                if let Some((_, s)) = &mut self.busy {
                    *s = Some(stage);
                }
            }
            Event::Done(purpose, result) => {
                self.busy = None;
                match result {
                    Ok(done) => self.on_scan(purpose, done.0.clone()),
                    Err(e) => {
                        self.redo = None;
                        self.redo_auto = false;
                        self.print_next = false;
                        self.job = None;
                        self.jobs.clear();
                        self.note = format!("Errore: {e}");
                        if purpose == Purpose::Page {
                            notify("Scansione non riuscita", &e, true);
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

    fn on_scan(&mut self, purpose: Purpose, image: Image) {
        match purpose {
            Purpose::Preview => {
                let raw = Raw::new(image);
                let settings = *self.settings();
                let shown = render(&raw.proxy, &settings, &settings.adjust);
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
                for (image, origin) in Self::cut(image, job.as_ref()) {
                    self.add_page(image, origin);
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
    fn add_page(&mut self, image: Image, origin: Option<Found>) {
        let (w, h, dpi) = (image.width, image.height, image.dpi);
        let mut page = Page::new(Raw::new(image), *self.settings());
        page.origin = origin;
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
                }
                self.note = format!("Pagina {} rifatta", index + 1);
                notify(&self.note, "", false);
            }
            (Some(index), _) => {
                self.redo = None;
                self.redone = Some((index, page));
                self.note = format!(
                    "Nuova scansione della pagina {}: tienila o lascia la vecchia",
                    index + 1
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
            let path = folder.join(Self::stamped(self.name(), self.photo.output.extension()));
            self.note = match self.write(&path) {
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
    fn on_button(&mut self, button: hp3770_core::device::Button) {
        use hp3770_core::device::Button;
        if self.busy.is_some() {
            self.note = format!("Tasto {} ignorato: scansione in corso", button.name());
            return;
        }
        let index = match button {
            Button::Scan => 0,
            Button::Copy => 1,
            Button::InstantShare => 2,
        };
        let action = match self.kind {
            Kind::Document => self.document_buttons[index],
            Kind::Photo => self.photo_buttons[index],
        };
        match action {
            ButtonAction::AddPage | ButtonAction::ScanPhoto => {
                self.redone = None;
                self.start(Purpose::Page);
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
                notify("Fotocopia", "Scansione in corso, poi la stampa", false);
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
                let path = folder.join(Self::stamped(
                    self.name(),
                    self.settings().output.extension(),
                ));
                match self.write(&path) {
                    Ok(what) => {
                        self.note = format!("Salvato: {what}");
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
        column![self.top_bar(), body, self.status_bar()].into()
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
                .on_press_maybe(kind.map(Message::Kind))
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
                match s.lamp {
                    hp3770_core::status::Lamp::Off => "Pronto · lampada spenta".to_string(),
                    _ => "Pronto · lampada accesa".to_string(),
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
        let bar = row![
            text("Scanjet 3770").size(15),
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
        for (i, page) in self.pages.iter().enumerate() {
            let selected = self.current == Some(i) && self.tab == Tab::Result;
            let thumb = button(
                iced::widget::image(page.thumb.handle.clone())
                    .width(Length::Fill)
                    .height(140),
            )
            .padding(2)
            .style(move |theme: &Theme, status| {
                let mut style = button::text(theme, status);
                style.border = iced::Border {
                    radius: 6.0.into(),
                    width: 2.0,
                    color: if selected {
                        theme.palette().primary
                    } else {
                        iced::Color::TRANSPARENT
                    },
                };
                style
            })
            .on_press(Message::PageShow(i));
            list = list.push(
                column![
                    thumb,
                    row![
                        text(if page.own.is_some() {
                            format!("{} · propria", i + 1)
                        } else {
                            format!("{}", i + 1)
                        })
                        .size(12)
                        .width(Length::Fill),
                        tool("↑", Message::PageMove(i, false), i > 0),
                        tool("↓", Message::PageMove(i, true), i + 1 < n),
                        tool("↻", Message::PageRotate(i, true), idle),
                    ]
                    .align_y(Alignment::Center),
                    row![
                        tool("Rifai", Message::PageRedo(i), idle),
                        Space::new().width(Length::Fill),
                        tool("Elimina", Message::PageDelete(i), idle),
                    ],
                ]
                .spacing(2),
            );
        }
        list = list.push(
            container(
                text("Il tasto Scan dello scanner aggiunge una pagina")
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
        container(scrollable(list.padding([12, 10])).height(Length::Fill))
            .width(156)
            .height(Length::Fill)
            .style(surface)
            .into()
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
                    Self::glass(),
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
                ),
                self.preview_view.scale,
            ),
            Tab::Result => (
                viewer::viewer(
                    self.shown().map(|p| &p.picture),
                    Self::glass(),
                    "Nessuna scansione: premi «Scansiona»",
                    self.result_view,
                    None,
                    Vec::new(),
                    Message::View,
                    None,
                    pick,
                    0.0,
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
        let tools = card(
            column![
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
                            text(format!("Nuova scansione della pagina {}", index + 1)).size(13),
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
            None | Some(Stage::Starting) => (0.02, "Avvio dello scanner"),
            Some(Stage::WarmingUp) => (0.05, "La lampada era spenta: riscaldamento"),
            Some(Stage::Calibrating) => (0.1, "Calibrazione su striscia bianca"),
            Some(Stage::Scanning { .. }) => (0.0, "Lettura"),
            Some(Stage::Finishing) => (0.97, "Ritorno del carrello ed elaborazione"),
        };
        let fraction = match stage {
            Some(Stage::Scanning { done, total }) => {
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
            heading("Originale"),
            labelled(
                "Risoluzione",
                pick_list(RESOLUTIONS, Some(self.settings().dpi), Message::Dpi)
                    .width(Length::Fill)
                    .into()
            ),
            labelled(
                "Colore",
                pick_list(
                    [Colour::Rgb, Colour::Grey, Colour::Lineart],
                    Some(s.colour),
                    Message::Colour
                )
                .width(Length::Fill)
                .into()
            ),
            heading("Area"),
            checkbox(self.settings().auto_crop)
                .label("Ritaglio automatico")
                .on_toggle(Message::AutoCrop)
                .text_size(13),
        ]
        .spacing(10);
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
        if self.settings().dpi > 1200 && self.selection.is_none() && plan.is_empty() {
            pane = pane.push(
                text("2400 dpi servono per piccoli originali: seleziona l'area sull'anteprima.")
                    .size(12)
                    .style(text::warning),
            );
        }
        let buttons = match self.kind {
            Kind::Document => self.document_buttons,
            Kind::Photo => self.photo_buttons,
        };
        pane = pane.push(heading("Tasti dello scanner")).push(
            text(format!(
                "Scan: {} · Copy: {} · Instant Share: {} (si cambiano nelle Preferenze)",
                buttons[0], buttons[1], buttons[2]
            ))
            .size(12),
        );
        pane.into()
    }

    fn adjust_pane(&self) -> Element<'_, Message> {
        let s = self.active();
        let lineart = s.colour == Colour::Lineart;
        let editing = if self.editing_page() {
            match self.kind {
                Kind::Document => format!("Pagina {}", self.current.map_or(0, |i| i + 1)),
                Kind::Photo => "Foto".to_string(),
            }
        } else {
            "Prossima scansione".to_string()
        };
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
        let curve_preset = |label: &'static str, c: hp3770_core::adjust::Curve| {
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
                    curve_preset("Lineare", hp3770_core::adjust::Curve::IDENTITY),
                    curve_preset(
                        "Contrasto medio",
                        hp3770_core::adjust::Curve::MEDIUM_CONTRAST
                    ),
                    curve_preset(
                        "Contrasto forte",
                        hp3770_core::adjust::Curve::STRONG_CONTRAST
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
        let scope: Element<'_, Message> = match (self.kind, self.editing_page()) {
            (Kind::Document, true) => {
                let own = self.shown().is_some_and(|p| p.own.is_some());
                column![
                    checkbox(own)
                        .label("Solo questa pagina")
                        .on_toggle(Message::OwnLook)
                        .text_size(13),
                    text(if own {
                        "Le modifiche valgono solo per questa pagina."
                    } else {
                        "Le modifiche valgono per l'anteprima e per tutte le pagine \
                         che non hanno regolazioni proprie."
                    })
                    .size(11),
                ]
                .spacing(4)
                .into()
            }
            (Kind::Document, false) => text(
                "Le modifiche valgono per l'anteprima e per tutte le pagine \
                 che non hanno regolazioni proprie.",
            )
            .size(11)
            .into(),
            (Kind::Photo, _) => text("Anteprima e foto hanno sempre le stesse regolazioni.")
                .size(11)
                .into(),
        };
        column![
            text(editing.to_uppercase()).size(11),
            scope,
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

    /// Whether Windows opens its own scan app on a button press, and the
    /// switch (Windows only, with the HP driver).
    fn wia_row(&self) -> Element<'_, Message> {
        match self.wia_quiet {
            None => Space::new().into(),
            Some(quiet) => column![
                text(if quiet {
                    "Windows lascia i tasti a questa app."
                } else {
                    "Windows apre anche la sua app di scansione a ogni tasto,                      e toglie lo scanner a questa."
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
                .style(if quiet { button::secondary } else { button::primary })
                .on_press(Message::WiaQuiet(!quiet)),
                text("Chiede una volta la conferma di amministratore.").size(11),
            ]
            .spacing(4)
            .into(),
        }
    }

    /// The choice of what a front-panel button does.
    fn button_choice(&self, index: usize, label: &'static str) -> Element<'_, Message> {
        let (options, current): (&[ButtonAction], ButtonAction) = match self.kind {
            Kind::Document => (&ButtonAction::DOCUMENT, self.document_buttons[index]),
            Kind::Photo => (&ButtonAction::PHOTO, self.photo_buttons[index]),
        };
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
                "Risoluzione, colore, formato, pagina PDF e regolazioni di ogni modo \
                 restano come li lasci: li ritrovi alla prossima apertura."
            )
            .size(12),
            heading("Cartella dei file"),
            text(self.folder().display().to_string()).size(12),
            row![
                button(text("Scegli…").size(13))
                    .style(button::secondary)
                    .on_press(Message::ChooseFolder),
                button(text("Immagini\\Scansioni").size(13))
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
            heading(match self.kind {
                Kind::Document => "Tasti dello scanner in Documento",
                Kind::Photo => "Tasti dello scanner in Foto",
            }),
            self.button_choice(0, "Scan"),
            self.button_choice(1, "Copy"),
            self.button_choice(2, "Instant Share"),
            text("Valgono anche con l'app in secondo piano; ignorati durante una scansione.")
                .size(11),
            self.wia_row(),
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
        let options = outputs(self.kind, s.colour);
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
                    "I campioni dello scanner, lineari e senza regolazioni (ritaglio e                      raddrizzatura sì): si sviluppano in Lightroom, Camera Raw o Photoshop.                      Le regolazioni a destra valgono solo per l'anteprima.",
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
            pane = pane.push(labelled(
                "Pagina PDF",
                pick_list(
                    [PdfSize::Fit, PdfSize::A4, PdfSize::Letter],
                    Some(self.settings().pdf_size),
                    Message::PdfSize,
                )
                .width(Length::Fill)
                .into(),
            ));
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
            Some(Ok(st)) => format!("USB {}", if st.usb2 { "2.0" } else { "1.1" }),
            Some(Err(_)) => "non collegato".to_string(),
            None => "…".to_string(),
        };
        let fact = |label: &'static str, value: String| {
            row![text(label).size(11), text(value).size(11)].spacing(4)
        };
        container(
            row![
                fact("Scanner", link),
                fact("Risoluzione", format!("{} dpi · {}", s.dpi, s.colour)),
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

fn labelled<'a>(label: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
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
