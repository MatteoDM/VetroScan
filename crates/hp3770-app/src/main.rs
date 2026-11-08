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

mod histogram;
mod viewer;
mod worker;

use histogram::Histogram;
use hp3770_core::adjust::{self, Adjustments};
use hp3770_core::convert;
use hp3770_core::geometry::{Area, GLASS_INCHES, RESOLUTIONS};
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
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pdf => f.write_str("PDF"),
            Self::TiffPages => f.write_str("TIFF multipagina"),
            Self::Image(format) => write!(f, "{format}"),
        }
    }
}

impl Output {
    fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::TiffPages => "tif",
            Self::Image(format) => format.extension(),
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
        ],
    };
    all.into_iter()
        .filter(|o| colour != Colour::Lineart || !o.lossy())
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
        }
    }

    fn photo() -> Self {
        Self {
            dpi: 600,
            output: Output::Image(ImageFormat::Jpeg { quality: 92 }),
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

/// A page of the document (or the photo): its raw scan, its own
/// adjustments and what is shown.
struct Page {
    raw: Raw,
    settings: Settings,
    shown: Image,
    picture: Picture,
    thumb: Picture,
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
            shown,
        }
    }

    fn redraw(&mut self, before: bool) {
        let adjust = if before {
            Adjustments::default()
        } else {
            self.settings.adjust
        };
        self.shown = render(&self.raw.proxy, &self.settings, &adjust);
        let rgb = self.shown.to_rgb();
        self.picture =
            Picture::from_rgb(self.shown.width, self.shown.height, &rgb).replacing(&self.picture);
        self.thumb = Picture::from_rgb_max(self.shown.width, self.shown.height, &rgb, 240);
    }

    /// The finished image at full resolution.
    fn finished(&self) -> Image {
        render(&self.raw.full, &self.settings, &self.settings.adjust)
    }

    fn rotate(&mut self, clockwise: bool) {
        let turns = if clockwise { 1 } else { 3 };
        self.raw = Raw {
            full: Arc::new(self.raw.full.rotated(turns)),
            proxy: Arc::new(self.raw.proxy.rotated(turns)),
        };
        self.redraw(false);
    }
}

// ---------------------------------------------------------------- messages

/// Groups of the adjustments panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Group {
    Tone,
    Colour,
    Detail,
    Lineart,
}

/// An adjustment with a slider and a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Temperature,
    Tint,
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
            Self::Temperature => "Temperatura",
            Self::Tint => "Tinta",
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
            _ => -100.0..=100.0,
        }
    }

    fn step(self) -> f32 {
        match self {
            Self::Gamma => 0.05,
            Self::Sharpen => 0.1,
            _ => 1.0,
        }
    }

    fn get(self, s: &Settings) -> f32 {
        let a = &s.adjust;
        match self {
            Self::Temperature => a.temperature,
            Self::Tint => a.tint,
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
            Self::Tint => a.tint = v,
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
}

/// An adjusted image computed off the interface thread.
#[derive(Debug)]
struct Rendered {
    target: Target,
    shown: Image,
    picture: Picture,
    thumb: Option<Picture>,
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
    View(View),
    Fit,
    ActualSize,
    Select(Option<Selection>),
    Dpi(u16),
    Colour(Colour),
    Output(Output),
    PdfSize(PdfSize),
    AutoSave(bool),
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
    /// The raw preview of the glass, its adjusted image and picture.
    preview: Option<(Raw, Image, Picture)>,
    preview_view: View,
    result_view: View,
    selection: Option<Selection>,
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
    /// A number being typed, not applied yet.
    draft: Option<(Field, String)>,
    /// A render is running off the interface thread.
    rendering: bool,
    /// Something changed since it started: render again when it ends.
    dirty: Option<Target>,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                worker: None,
                status: None,
                system_dark: false,
                theme: ThemeChoice::System,
                kind: Kind::Document,
                document: Settings::document(),
                photo: Settings::photo(),
                tab: Tab::Preview,
                preview: None,
                preview_view: View::default(),
                result_view: View::default(),
                selection: None,
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
                rendering: false,
                dirty: None,
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
        if self.editing_page() {
            self.shown().map(|p| p.settings).unwrap_or(*self.settings())
        } else {
            *self.settings()
        }
    }

    /// Applies a change to the active settings and redraws what shows it.
    /// New scans start from the last settings used.
    fn change(&mut self, f: impl Fn(&mut Settings)) {
        if self.editing_page() {
            if let Some(page) = self.shown_mut() {
                f(&mut page.settings);
            }
            f(self.settings_mut());
            self.dirty = Some(Target::Page);
        } else {
            f(self.settings_mut());
            self.dirty = Some(Target::Preview);
        }
    }

    /// Starts the render of what changed, off the interface thread, unless
    /// one is running: then the latest values are rendered when it ends
    /// (a slider dragged fast skips the values in between).
    fn schedule(&mut self) -> Task<Message> {
        if self.rendering {
            return Task::none();
        }
        let Some(target) = self.dirty.take() else {
            return Task::none();
        };
        let before = self.before;
        let job = match target {
            Target::Preview => self
                .preview
                .as_ref()
                .map(|(raw, ..)| (raw.proxy.clone(), *self.settings())),
            Target::Page => self.shown().map(|p| (p.raw.proxy.clone(), p.settings)),
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
                let thumb = (target == Target::Page)
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
        let (x, y, w, h) = self.area_mm();
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

    /// Bytes the scan takes in memory.
    fn memory(&self) -> f64 {
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
        } else {
            self.request()
        };
        if let (Some(worker), None) = (&self.worker, &self.busy) {
            worker.send(Command::Scan(purpose, request));
            self.busy = Some((purpose, None));
            self.eyedropper = false;
        }
    }

    /// Default folder of saved scans.
    fn folder() -> PathBuf {
        dirs::picture_dir()
            .or_else(dirs::document_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Scansioni")
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
        let pages: Vec<Image> = match self.kind {
            Kind::Document => self.pages.iter().map(Page::finished).collect(),
            Kind::Photo => self.shown().map(Page::finished).into_iter().collect(),
        };
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

    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
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
                    Target::Page => {
                        if let Some(page) = self.shown_mut() {
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
            Message::Toggle(group) => {
                if let Some(i) = self.open.iter().position(|g| *g == group) {
                    self.open.remove(i);
                } else {
                    self.open.push(group);
                }
            }
            Message::Adjust(field, v) => {
                self.draft = None;
                self.change(|s| field.set(s, v));
            }
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
                        j.black = d.black;
                        j.white = d.white;
                        j.gamma = d.gamma;
                        j.brightness = d.brightness;
                        j.contrast = d.contrast;
                    }
                    Group::Colour => {
                        j.temperature = d.temperature;
                        j.tint = d.tint;
                        j.saturation = d.saturation;
                    }
                    Group::Detail => j.sharpen = d.sharpen,
                    Group::Lineart => s.threshold = convert::DEFAULT_THRESHOLD,
                }
            }),
            Message::Before(on) => {
                self.before = on;
                self.redraw_preview();
                if let Some(p) = self.shown_mut() {
                    p.redraw(on);
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
                        ("pagina", "Un file per pagina")
                    }
                    (Kind::Document, _) => ("documento", "Documento"),
                    (Kind::Photo, _) => ("foto", "Immagine"),
                };
                let file = Self::stamped(prefix, ext);
                let folder = Self::folder();
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
                if let Some(page) = self.pages.get_mut(i) {
                    page.rotate(clockwise);
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

    fn on_scan(&mut self, purpose: Purpose, image: Image) {
        let raw = Raw::new(image);
        match purpose {
            Purpose::Preview => {
                let settings = *self.settings();
                let shown = render(&raw.proxy, &settings, &settings.adjust);
                let picture = Picture::from_rgb(shown.width, shown.height, &shown.to_rgb());
                self.preview = Some((raw, shown, picture));
                self.preview_view = View::default();
                self.tab = Tab::Preview;
                self.note =
                    "Anteprima pronta: trascina per scegliere l'area, regola a destra".into();
            }
            Purpose::Page => {
                let (w, h, dpi) = (raw.full.width, raw.full.height, raw.full.dpi);
                let page = Page::new(raw, *self.settings());
                match (self.redo.take(), self.kind) {
                    (Some(index), _) if std::mem::take(&mut self.redo_auto) => {
                        if index < self.pages.len() {
                            self.pages[index] = page;
                            self.current = Some(index);
                        }
                        self.note = format!("Pagina {} rifatta", index + 1);
                        notify(&self.note, "", false);
                    }
                    (Some(index), _) => {
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
                            "Pagina {} acquisita · {w} × {h} pixel a {dpi} dpi",
                            self.pages.len()
                        );
                        notify(&format!("Pagina {} acquisita", self.pages.len()), "", false);
                    }
                    (None, Kind::Photo) => {
                        self.pages = vec![page];
                        self.current = Some(0);
                        self.note = format!("Foto acquisita · {w} × {h} pixel a {dpi} dpi");
                        if self.photo.auto_save {
                            let folder = Self::folder();
                            let _ = std::fs::create_dir_all(&folder);
                            let path =
                                folder.join(Self::stamped("foto", self.photo.output.extension()));
                            self.note = match self.write(&path) {
                                Ok(_) => {
                                    notify("Foto salvata", &path.display().to_string(), false);
                                    format!("Foto salvata in {}", path.display())
                                }
                                Err(e) => format!("Salvataggio non riuscito: {e}"),
                            };
                        }
                    }
                }
                self.result_view = View::default();
                self.tab = Tab::Result;
            }
        }
    }

    /// The front-panel buttons, by way of working. Ignored while a scan
    /// runs. Documento: Scan adds a page, Copy redoes the last one,
    /// Instant Share saves the document. Foto: Scan scans (and saves if
    /// automatic saving is on), Instant Share opens the folder.
    fn on_button(&mut self, button: hp3770_core::device::Button) {
        use hp3770_core::device::Button;
        if self.busy.is_some() {
            return;
        }
        match (self.kind, button) {
            (_, Button::Scan) => {
                self.redone = None;
                self.start(Purpose::Page);
            }
            (Kind::Document, Button::Copy) => match self.pages.len() {
                0 => self.start(Purpose::Page),
                n => {
                    self.redone = None;
                    self.redo = Some(n - 1);
                    self.redo_auto = true;
                    self.start(Purpose::Page);
                }
            },
            (Kind::Photo, Button::Copy) => notify(
                "Fotocopia non ancora disponibile",
                "La stampa diretta arriverà più avanti",
                true,
            ),
            (Kind::Document, Button::InstantShare) => {
                if self.pages.is_empty() {
                    notify(
                        "Documento vuoto",
                        "Premi Scan per acquisire una pagina",
                        true,
                    );
                    return;
                }
                let folder = Self::folder();
                let _ = std::fs::create_dir_all(&folder);
                let path =
                    folder.join(Self::stamped("documento", self.document.output.extension()));
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
            (Kind::Photo, Button::InstantShare) => open_folder(&Self::folder()),
        }
    }

    // ------------------------------------------------------------- views

    fn view(&self) -> Element<'_, Message> {
        let header = row![
            mode_button("📄  Documento", Kind::Document, self.kind),
            mode_button("🖼  Foto", Kind::Photo, self.kind),
            Space::new().width(Length::Fill),
            text(self.status_line()).size(13),
            button(text("Aggiorna").size(13))
                .style(button::text)
                .on_press_maybe(self.idle().then_some(Message::RefreshStatus)),
            pick_list(
                [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark],
                Some(self.theme),
                Message::Theme
            )
            .text_size(13),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let body = row![
            self.left(),
            scrollable(self.panel().padding(iced::Padding::ZERO.right(14))).width(360),
        ]
        .spacing(18)
        .height(Length::Fill);

        container(column![header, body, self.bottom()].spacing(12))
            .padding(16)
            .into()
    }

    fn idle(&self) -> bool {
        self.busy.is_none() && self.worker.is_some()
    }

    fn status_line(&self) -> String {
        match &self.status {
            None => "Scanner: …".into(),
            Some(Err(e)) => format!("Scanner non trovato: {e}"),
            Some(Ok(s)) => format!(
                "Lampada {} · carrello {}",
                match s.lamp {
                    hp3770_core::status::Lamp::Off => "spenta",
                    _ => "accesa",
                },
                if s.head_at_home {
                    "a casa"
                } else {
                    "fuori casa"
                }
            ),
        }
    }

    fn left(&self) -> Element<'_, Message> {
        let tab_button = |label, tab| {
            button(text(label))
                .style(if self.tab == tab {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::Tab(tab))
        };
        let result_label = match self.kind {
            Kind::Document => "Pagine",
            Kind::Photo => "Foto",
        };
        let tools = row![
            tab_button("Anteprima", Tab::Preview),
            tab_button(result_label, Tab::Result),
            Space::new().width(Length::Fill),
            button(text("Adatta"))
                .style(button::secondary)
                .on_press(Message::Fit),
            button(text("100%"))
                .style(button::secondary)
                .on_press(Message::ActualSize),
            button(text("Togli selezione"))
                .style(button::secondary)
                .on_press_maybe(
                    (self.tab == Tab::Preview && self.selection.is_some())
                        .then_some(Message::Select(None))
                ),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let pick = self
            .eyedropper
            .then_some(Message::Picked as fn(Point) -> Message);
        let view = match self.tab {
            Tab::Preview => viewer::viewer(
                self.preview.as_ref().map(|(_, _, p)| p),
                Self::glass(),
                "Fai un'anteprima per vedere il vetro",
                self.preview_view,
                self.selection,
                Message::View,
                Some(Message::Select),
                pick,
            ),
            Tab::Result => viewer::viewer(
                self.shown().map(|p| &p.picture),
                Self::glass(),
                "Nessuna scansione",
                self.result_view,
                None,
                Message::View,
                None,
                pick,
            ),
        };

        let redo_bar: Element<'_, Message> = match &self.redone {
            Some((index, _)) if self.tab == Tab::Result => row![
                text(format!("Nuova scansione della pagina {}", index + 1)).width(Length::Fill),
                button(text("Tieni la nuova")).on_press(Message::AcceptRedo),
                button(text("Lascia la vecchia"))
                    .style(button::secondary)
                    .on_press(Message::RejectRedo),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
            _ => Space::new().into(),
        };
        let hint = if self.eyedropper {
            "Contagocce: clicca su qualcosa che deve essere bianco o grigio"
        } else {
            "Rotella: zoom · tasto destro: sposta · sinistro: seleziona l'area"
        };
        let mut left = column![
            tools,
            redo_bar,
            container(view).height(Length::Fill).width(Length::Fill),
            text(hint).size(12),
        ]
        .spacing(8)
        .width(Length::Fill);
        if self.kind == Kind::Document {
            left = left.push(self.strip());
        }
        left.into()
    }

    /// The pages of the document as thumbnails, each with its tools.
    fn strip(&self) -> Element<'_, Message> {
        if self.pages.is_empty() {
            return text("Documento vuoto: ogni scansione aggiunge una pagina qui.")
                .size(13)
                .into();
        }
        let idle = self.busy.is_none();
        let tool = |label: &'static str, message: Message, enabled: bool| {
            button(text(label).size(13))
                .padding([2, 6])
                .style(button::text)
                .on_press_maybe(enabled.then_some(message))
        };
        let n = self.pages.len();
        let cards = self.pages.iter().enumerate().map(|(i, page)| {
            let selected = self.current == Some(i) && self.tab == Tab::Result;
            let thumb = button(iced::widget::image(page.thumb.handle.clone()).height(100))
                .padding(3)
                .style(if selected {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::PageShow(i));
            column![
                thumb,
                text(format!("Pagina {}", i + 1)).size(12),
                row![
                    tool("◀", Message::PageMove(i, false), i > 0),
                    tool("↺", Message::PageRotate(i, false), idle),
                    tool("↻", Message::PageRotate(i, true), idle),
                    tool("▶", Message::PageMove(i, true), i + 1 < n),
                ],
                row![
                    tool("Rifai", Message::PageRedo(i), idle),
                    tool("Elimina", Message::PageDelete(i), idle),
                ],
            ]
            .spacing(2)
            .align_x(Alignment::Center)
            .into()
        });
        scrollable(row(cards).spacing(12).padding([4, 0]))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .width(Length::Fill)
            .into()
    }

    fn panel(&self) -> iced::widget::Column<'_, Message> {
        let s = self.active();
        let editing = if self.editing_page() {
            "Regolazioni della pagina mostrata"
        } else {
            "Regolazioni (sull'anteprima)"
        };

        // Scan settings.
        let request = self.request();
        let (w, h) = (request.area.width, request.area.height);
        let (x, y, wmm, hmm) = self.area_mm();
        let memory = self.memory();
        let mut scan = column![
            heading("Scansione"),
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
            text(format!(
                "Area {x:.0}, {y:.0} mm · {wmm:.0} × {hmm:.0} mm{} · {w} × {h} pixel",
                if self.selection.is_none() {
                    " (A4)"
                } else {
                    ""
                }
            ))
            .size(12),
        ]
        .spacing(8);
        if memory > LARGE_SCAN {
            scan = scan.push(
                text(format!(
                    "Scansione molto grande: circa {:.1} GB e alcuni minuti. \
                     Seleziona un'area o abbassa la risoluzione se non serve tutta.",
                    memory / 1e9
                ))
                .size(12)
                .style(text::danger),
            );
        }
        if self.settings().dpi > 1200 && self.selection.is_none() {
            scan = scan.push(
                text("2400 dpi servono per piccoli originali: seleziona l'area sull'anteprima.")
                    .size(12)
                    .style(text::warning),
            );
        }
        let actions = row![
            button(text("Anteprima").center().width(Length::Fill))
                .width(Length::Fill)
                .style(button::secondary)
                .on_press_maybe(self.idle().then_some(Message::Preview)),
            button(
                text(match self.kind {
                    Kind::Document => "Scansiona pagina",
                    Kind::Photo => "Scansiona",
                })
                .center()
                .width(Length::Fill)
            )
            .width(Length::Fill)
            .on_press_maybe(self.idle().then_some(Message::Scan)),
        ]
        .spacing(8);

        // Adjustments.
        let colour_mode = s.colour == Colour::Rgb;
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
                        canvas(hist).width(Length::Fill).height(90),
                        text(format!(
                            "Neri tagliati {black:.1} % · bianchi tagliati {white:.1} %"
                        ))
                        .size(11),
                    ]
                    .spacing(4)
                    .into()
                }
                _ => Space::new().into(),
            }
        };
        let tone = self.group(
            Group::Tone,
            "Tono",
            !lineart,
            "Non serve in bianco e nero: usa la soglia",
            column![
                button(text("Livelli automatici").size(13))
                    .style(button::secondary)
                    .on_press(Message::AutoLevels),
                self.adjuster(Field::Black, &s),
                self.adjuster(Field::White, &s),
                self.adjuster(Field::Gamma, &s),
                self.adjuster(Field::Brightness, &s),
                self.adjuster(Field::Contrast, &s),
            ]
            .spacing(6)
            .into(),
        );
        let mut colour_body = column![
            button(
                text(if self.eyedropper {
                    "Annulla contagocce"
                } else {
                    "Contagocce: scegli il neutro"
                })
                .size(13)
            )
            .style(if self.eyedropper {
                button::primary
            } else {
                button::secondary
            })
            .on_press(Message::Eyedropper),
            self.adjuster(Field::Temperature, &s),
            self.adjuster(Field::Tint, &s),
        ]
        .spacing(6);
        if colour_mode {
            colour_body = colour_body.push(self.adjuster(Field::Saturation, &s));
        }
        let colour = self.group(
            Group::Colour,
            "Colore",
            !lineart,
            "Non serve in bianco e nero",
            colour_body.into(),
        );
        let detail = self.group(
            Group::Detail,
            "Dettaglio",
            true,
            "",
            self.adjuster(Field::Sharpen, &s),
        );
        let bw = self.group(
            Group::Lineart,
            "Bianco e nero",
            lineart,
            "Solo con il colore «Bianco e nero»",
            column![
                button(text("Soglia automatica").size(13))
                    .style(button::secondary)
                    .on_press(Message::AutoThreshold),
                self.adjuster(Field::Threshold, &s),
            ]
            .spacing(6)
            .into(),
        );
        let adjustments = column![
            row![
                heading(editing).width(Length::Fill),
                checkbox(self.before)
                    .label("Prima")
                    .on_toggle(Message::Before)
                    .text_size(13),
            ]
            .align_y(Alignment::Center),
            histogram,
            tone,
            colour,
            detail,
            bw,
        ]
        .spacing(8);

        // Output.
        let options = outputs(self.kind, s.colour);
        let mut out = column![
            heading("Uscita"),
            labelled(
                "Formato",
                pick_list(options, Some(self.settings().output), Message::Output)
                    .width(Length::Fill)
                    .into()
            ),
        ]
        .spacing(8);
        if lineart {
            out =
                out.push(text("Bianco e nero: niente JPEG/AVIF, rovinerebbero i bordi.").size(11));
        }
        if self.settings().output == Output::Pdf {
            out = out.push(labelled(
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
        out = out.push(text(format!("Cartella: {}", Self::folder().display())).size(11));
        match self.kind {
            Kind::Document => {
                out = out.push(
                    row![
                        button(text("Salva documento…").center().width(Length::Fill))
                            .width(Length::Fill)
                            .on_press_maybe(
                                (!self.pages.is_empty() && self.busy.is_none())
                                    .then_some(Message::Save)
                            ),
                        button(text("Nuovo").center())
                            .style(button::secondary)
                            .on_press_maybe(
                                (!self.pages.is_empty() && self.busy.is_none())
                                    .then_some(Message::NewDocument)
                            ),
                    ]
                    .spacing(8),
                );
                out = out.push(
                    text(
                        "Tasti dello scanner: Scan aggiunge una pagina, Copy rifà l'ultima, \
                         Instant Share salva il documento.",
                    )
                    .size(11),
                );
            }
            Kind::Photo => {
                out = out.push(
                    checkbox(self.settings().auto_save)
                        .label("Salva da sola ogni scansione nella cartella")
                        .on_toggle(Message::AutoSave)
                        .text_size(13),
                );
                out = out.push(
                    button(text("Salva foto…").center().width(Length::Fill))
                        .width(Length::Fill)
                        .on_press_maybe(
                            (self.shown().is_some() && self.busy.is_none())
                                .then_some(Message::Save),
                        ),
                );
                out = out.push(
                    text("Tasti dello scanner: Scan scansiona, Instant Share apre la cartella.")
                        .size(11),
                );
            }
        }

        column![
            scan,
            actions,
            rule::horizontal(1),
            adjustments,
            rule::horizontal(1),
            out
        ]
        .spacing(12)
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

    fn bottom(&self) -> Element<'_, Message> {
        let (fraction, stage) = match &self.busy {
            None => (0.0, String::new()),
            Some((purpose, stage)) => {
                let what = match purpose {
                    Purpose::Preview => "Anteprima",
                    Purpose::Page => "Scansione",
                };
                match stage {
                    None | Some(Stage::Starting) => (0.02, format!("{what}: avvio")),
                    Some(Stage::WarmingUp) => (0.05, format!("{what}: riscaldamento lampada")),
                    Some(Stage::Calibrating) => (0.1, format!("{what}: calibrazione")),
                    Some(Stage::Scanning { done, total }) => (
                        0.1 + 0.85 * *done as f32 / (*total).max(1) as f32,
                        format!("{what}: {} %", done * 100 / (*total).max(1)),
                    ),
                    Some(Stage::Finishing) => (0.97, format!("{what}: chiusura")),
                }
            }
        };
        row![
            text(if stage.is_empty() {
                self.note.clone()
            } else {
                stage
            })
            .width(Length::Fill),
            progress_bar(0.0..=1.0, fraction).length(260).girth(12),
            button(text("Annulla"))
                .style(button::danger)
                .on_press_maybe(self.busy.is_some().then_some(Message::Cancel)),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    }
}

// ----------------------------------------------------------------- widgets

fn mode_button(label: &str, kind: Kind, current: Kind) -> Element<'_, Message> {
    button(text(label).size(16))
        .padding([8, 18])
        .style(if kind == current {
            button::primary
        } else {
            button::secondary
        })
        .on_press(Message::Kind(kind))
        .into()
}

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
