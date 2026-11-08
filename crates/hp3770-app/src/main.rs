// SPDX-License-Identifier: GPL-2.0-or-later
//! `hp3770-app`: desktop application for the HP Scanjet 3770.
//!
//! The viewer on the left shows the preview of the glass (where the area
//! to scan is drawn) or the last scan, with zoom and pan; the settings are
//! on the right, progress at the bottom. Every scanner request goes
//! through [`worker`].

#![windows_subsystem = "windows"]

mod viewer;
mod worker;

use hp3770_core::convert;
use hp3770_core::geometry::{Area, GLASS_INCHES, RESOLUTIONS};
use hp3770_core::session::{self, Image, Request, Stage};
use hp3770_core::status::Status;
use iced::widget::{
    button, checkbox, column, container, pick_list, progress_bar, radio, row, rule, slider, text,
    Space,
};
use iced::{Alignment, Element, Length, Size, Subscription, Task, Theme};
use std::path::PathBuf;
use std::sync::Arc;
use viewer::{Picture, Selection, View};
use worker::{Command, Event, Handle, Purpose};

/// Resolution of the preview of the whole glass (the lowest the scanner
/// has).
const PREVIEW_DPI: u16 = 200;

/// Above this size (bytes in memory) the interface warns before scanning.
const LARGE_SCAN: f64 = 400e6;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("HP Scanjet 3770")
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size(Size::new(1280.0, 860.0))
        .run()
}

/// The colour mode as the interface offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeChoice {
    Colour,
    Grey,
    Lineart,
}

/// Which image the viewer shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Preview,
    Result,
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

#[derive(Debug, Clone)]
enum Message {
    Worker(Event),
    SystemTheme(iced::theme::Mode),
    Theme(ThemeChoice),
    Tab(Tab),
    View(View),
    Fit,
    ActualSize,
    Select(Option<Selection>),
    Dpi(u16),
    Mode(ModeChoice),
    Threshold(u8),
    Sharpen(bool),
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
    PdfSize(PdfSize),
    Export,
    ExportTo(Option<PathBuf>),
}

/// Page size of the exported PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PdfSize {
    Fit,
    A4,
    Letter,
}

impl std::fmt::Display for PdfSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Fit => "PDF: pagina come la scansione",
            Self::A4 => "PDF: A4",
            Self::Letter => "PDF: Letter",
        })
    }
}

/// A page of the document.
struct Page {
    image: Arc<Image>,
    picture: Picture,
    thumb: Picture,
}

impl Page {
    fn new(image: Arc<Image>) -> Self {
        let rgb = image.to_rgb();
        Self {
            picture: Picture::from_rgb(image.width, image.height, &rgb),
            thumb: Picture::from_rgb_max(image.width, image.height, &rgb, 240),
            image,
        }
    }
}

/// A page rescanned, waiting to replace page `index`.
struct Redo {
    index: usize,
    page: Page,
}

struct App {
    worker: Option<Handle>,
    status: Option<Result<Status, String>>,
    system_dark: bool,
    theme: ThemeChoice,
    tab: Tab,
    preview: Option<Picture>,
    preview_view: View,
    result_view: View,
    selection: Option<Selection>,
    dpi: u16,
    mode: ModeChoice,
    threshold: u8,
    sharpen: bool,
    /// The scan running, and where it is.
    busy: Option<(Purpose, Option<Stage>)>,
    /// The pages of the document.
    pages: Vec<Page>,
    /// The page shown in the result tab.
    current: Option<usize>,
    /// The page being rescanned.
    redo: Option<usize>,
    /// Its new scan, waiting for a choice.
    redone: Option<Redo>,
    pdf_size: PdfSize,
    /// Last thing that happened, for the status line.
    note: String,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                worker: None,
                status: None,
                system_dark: false,
                theme: ThemeChoice::System,
                tab: Tab::Preview,
                preview: None,
                preview_view: View::default(),
                result_view: View::default(),
                selection: None,
                dpi: 300,
                mode: ModeChoice::Colour,
                threshold: convert::DEFAULT_THRESHOLD,
                sharpen: true,
                busy: None,
                pages: Vec::new(),
                current: None,
                redo: None,
                redone: None,
                pdf_size: PdfSize::Fit,
                note: "Pronto".into(),
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

    fn request(&self) -> Request {
        let (x, y, w, h) = self.area_mm();
        let mode = match self.mode {
            ModeChoice::Colour => session::Mode::Colour,
            ModeChoice::Grey => session::Mode::Grey,
            ModeChoice::Lineart => session::Mode::Lineart {
                threshold: self.threshold,
            },
        };
        Request {
            sharpen: if self.sharpen {
                convert::HP_SHARPEN
            } else {
                0.0
            },
            ..Request::new(self.dpi, Area::from_mm(self.dpi, x, y, w, h), mode)
        }
    }

    /// Bytes the scan takes in memory while it is processed (the chip's
    /// colour data, plus the arranged copy at 2400 dpi).
    fn memory(&self) -> f64 {
        let a = self.request().area;
        let raw = 3.0 * f64::from(a.width) * f64::from(a.height);
        if self.dpi > 1200 {
            raw * 2.0
        } else {
            raw
        }
    }

    fn start(&mut self, purpose: Purpose) {
        let request = if purpose == Purpose::Preview {
            let glass = Self::glass();
            Request {
                sharpen: 0.0,
                ..Request::new(
                    PREVIEW_DPI,
                    Area {
                        x: 0,
                        y: 0,
                        width: glass.width as u32,
                        height: glass.height as u32,
                    },
                    session::Mode::Colour,
                )
            }
        } else {
            self.request()
        };
        if let (Some(worker), None) = (&self.worker, &self.busy) {
            worker.send(Command::Scan(purpose, request));
            self.busy = Some((purpose, None));
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Worker(event) => self.on_worker(event),
            Message::SystemTheme(mode) => self.system_dark = mode == iced::theme::Mode::Dark,
            Message::Theme(choice) => self.theme = choice,
            Message::Tab(tab) => self.tab = tab,
            Message::View(view) => *self.current_view() = view,
            Message::Fit => *self.current_view() = View::default(),
            Message::ActualSize => {
                let v = self.current_view();
                *v = v.actual_size();
            }
            Message::Select(selection) => self.selection = selection,
            Message::Dpi(dpi) => self.dpi = dpi,
            Message::Mode(mode) => self.mode = mode,
            Message::Threshold(t) => self.threshold = t,
            Message::Sharpen(on) => self.sharpen = on,
            Message::RefreshStatus => {
                if let Some(w) = &self.worker {
                    w.send(Command::Status);
                }
            }
            Message::Preview => self.start(Purpose::Preview),
            Message::Scan => self.start(Purpose::Page),
            Message::Cancel => {
                if let Some(w) = &self.worker {
                    w.cancel();
                    self.note = "Annullamento…".into();
                }
            }
            Message::Save => {
                if self.shown().is_some() {
                    return Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .add_filter("PNG", &["png"])
                                .add_filter("JPEG", &["jpg", "jpeg"])
                                .add_filter("TIFF", &["tif", "tiff"])
                                .add_filter("WebP", &["webp"])
                                .add_filter("AVIF", &["avif"])
                                .add_filter("BMP", &["bmp"])
                                .add_filter("PDF", &["pdf"])
                                .set_file_name("scansione.png")
                                .save_file()
                                .await
                                .map(|f| f.path().to_path_buf())
                        },
                        Message::SaveTo,
                    );
                }
            }
            Message::SaveTo(Some(path)) => {
                if let Some(page) = self.shown() {
                    let page = page.image.clone();
                    let pdf = path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
                    let result = if pdf {
                        hp3770_export::save_pdf(
                            &path,
                            std::slice::from_ref(&*page),
                            hp3770_export::PageSize::Fit,
                            90,
                        )
                    } else {
                        hp3770_export::save(&path, &page)
                    };
                    self.note = match result {
                        Ok(()) => format!("Salvata in {}", path.display()),
                        Err(e) => format!("Salvataggio non riuscito: {e}"),
                    };
                }
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
                    let turns = if clockwise { 1 } else { 3 };
                    *page = Page::new(Arc::new(page.image.rotated(turns)));
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
                    self.current = if self.pages.is_empty() {
                        None
                    } else {
                        Some(i.min(self.pages.len() - 1))
                    };
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
                if let Some(redo) = self.redone.take() {
                    if redo.index < self.pages.len() {
                        self.pages[redo.index] = redo.page;
                        self.current = Some(redo.index);
                        self.note = format!("Pagina {} sostituita", redo.index + 1);
                    }
                }
            }
            Message::RejectRedo => {
                if let Some(redo) = self.redone.take() {
                    self.current = Some(redo.index);
                    self.note = format!("Pagina {} lasciata com'era", redo.index + 1);
                }
            }
            Message::NewDocument => {
                self.pages.clear();
                self.current = None;
                self.redone = None;
                self.note = "Nuovo fascicolo".into();
            }
            Message::PdfSize(size) => self.pdf_size = size,
            Message::Export => {
                if !self.pages.is_empty() {
                    return Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .add_filter("PDF", &["pdf"])
                                .add_filter("TIFF multipagina", &["tif", "tiff"])
                                .add_filter("ZIP di immagini PNG", &["zip"])
                                .set_file_name("fascicolo.pdf")
                                .save_file()
                                .await
                                .map(|f| f.path().to_path_buf())
                        },
                        Message::ExportTo,
                    );
                }
            }
            Message::ExportTo(Some(path)) => {
                let pages: Vec<Image> = self.pages.iter().map(|p| (*p.image).clone()).collect();
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default();
                let result = match ext.as_str() {
                    "tif" | "tiff" => hp3770_export::save_tiff(&path, &pages),
                    "zip" => {
                        let name = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("pagina")
                            .to_string();
                        hp3770_export::save_zip(
                            &path,
                            &name,
                            &pages,
                            hp3770_export::ImageFormat::Png,
                        )
                    }
                    _ => hp3770_export::save_pdf(
                        &path,
                        &pages,
                        match self.pdf_size {
                            PdfSize::Fit => hp3770_export::PageSize::Fit,
                            PdfSize::A4 => hp3770_export::PageSize::A4,
                            PdfSize::Letter => hp3770_export::PageSize::Letter,
                        },
                        88,
                    ),
                };
                self.note = match result {
                    Ok(()) => format!(
                        "Fascicolo di {} pagine esportato in {}",
                        pages.len(),
                        path.display()
                    ),
                    Err(e) => format!("Esportazione non riuscita: {e}"),
                };
            }
            Message::ExportTo(None) => {}
        }
        Task::none()
    }

    /// The page the result tab shows: a rescan waiting for a choice, else
    /// the current page.
    fn shown(&self) -> Option<&Page> {
        self.redone
            .as_ref()
            .map(|r| &r.page)
            .or_else(|| self.current.and_then(|i| self.pages.get(i)))
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
                    Ok(done) => {
                        let (img, report) = &*done;
                        match purpose {
                            Purpose::Preview => {
                                self.preview =
                                    Some(Picture::from_rgb(img.width, img.height, &img.to_rgb()));
                                self.preview_view = View::default();
                                self.tab = Tab::Preview;
                                self.note =
                                    "Anteprima pronta: trascina sull'immagine per scegliere l'area"
                                        .into();
                            }
                            Purpose::Page => {
                                self.note = format!(
                                    "Scansione {} × {} pixel a {} dpi{}",
                                    img.width,
                                    img.height,
                                    img.dpi,
                                    if report.warmed_up {
                                        " (lampada riscaldata)"
                                    } else {
                                        ""
                                    }
                                );
                                let page = Page::new(Arc::new(img.clone()));
                                match self.redo.take() {
                                    Some(index) => {
                                        self.redone = Some(Redo { index, page });
                                        self.note = format!(
                                            "Nuova scansione della pagina {}: tienila o lascia la vecchia",
                                            index + 1
                                        );
                                    }
                                    None => {
                                        self.pages.push(page);
                                        self.current = Some(self.pages.len() - 1);
                                        self.redone = None;
                                        self.note = format!(
                                            "Pagina {} acquisita · {}",
                                            self.pages.len(),
                                            self.note
                                        );
                                    }
                                }
                                self.result_view = View::default();
                                self.tab = Tab::Result;
                            }
                        }
                    }
                    Err(e) => {
                        self.redo = None;
                        self.note = format!("Errore: {e}");
                    }
                }
                if let Some(w) = &self.worker {
                    w.send(Command::Status);
                }
            }
            Event::Button(button) => {
                self.note = format!("Tasto {} premuto", button.name());
            }
        }
    }

    /// The pages of the document as thumbnails, each with its tools.
    fn strip(&self) -> Element<'_, Message> {
        if self.pages.is_empty() {
            return text("Il fascicolo è vuoto: ogni scansione aggiunge una pagina qui.")
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
            let selected = self.current == Some(i);
            let thumb = button(iced::widget::image(page.thumb.handle.clone()).height(110))
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
        iced::widget::scrollable(row(cards).spacing(12).padding([4, 0]))
            .direction(iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::default(),
            ))
            .width(Length::Fill)
            .into()
    }

    fn view(&self) -> Element<'_, Message> {
        let idle = self.busy.is_none() && self.worker.is_some();

        // Viewer with its tool bar.
        let tab_button = |label, tab| {
            button(text(label))
                .style(if self.tab == tab {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::Tab(tab))
        };
        let tools = row![
            tab_button("Anteprima", Tab::Preview),
            tab_button("Risultato", Tab::Result),
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
        let view = match self.tab {
            Tab::Preview => viewer::viewer(
                self.preview.as_ref(),
                Self::glass(),
                "Fai un'anteprima per vedere il vetro",
                self.preview_view,
                self.selection,
                Message::View,
                Some(Message::Select),
            ),
            Tab::Result => viewer::viewer(
                self.shown().map(|p| &p.picture),
                Self::glass(),
                "Nessuna scansione",
                self.result_view,
                None,
                Message::View,
                None,
            ),
        };
        let redo_bar: Element<'_, Message> = match &self.redone {
            Some(r) if self.tab == Tab::Result => row![
                text(format!("Nuova scansione della pagina {}", r.index + 1)).width(Length::Fill),
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
        let left = column![
            tools,
            redo_bar,
            container(view).height(Length::Fill).width(Length::Fill),
            self.strip(),
            text("Rotella: zoom · tasto destro: sposta · sinistro: seleziona l'area").size(12),
        ]
        .spacing(8)
        .width(Length::Fill);

        let action = |label, message| {
            button(text(label).width(Length::Fill).center())
                .width(Length::Fill)
                .on_press_maybe(idle.then_some(message))
        };

        let status = match &self.status {
            None => text("Stato: …"),
            Some(Err(e)) => text(format!("Scanner non trovato: {e}")),
            Some(Ok(s)) => text(format!(
                "Lampada: {} · Carrello: {}",
                match s.lamp {
                    hp3770_core::status::Lamp::Off => "spenta",
                    _ => "accesa",
                },
                if s.head_at_home {
                    "a casa"
                } else {
                    "fuori casa"
                }
            )),
        }
        .size(13);

        let request = self.request();
        let (w, h) = (request.area.width, request.area.height);
        let file_bytes = match self.mode {
            ModeChoice::Colour => 3.0,
            ModeChoice::Grey => 1.0,
            ModeChoice::Lineart => 0.125,
        } * f64::from(w)
            * f64::from(h);
        let memory = self.memory();
        let (x, y, wmm, hmm) = self.area_mm();
        let warning: Element<'_, Message> = if memory > LARGE_SCAN {
            text(format!(
                "Scansione molto grande: circa {:.1} GB di memoria e alcuni minuti. \
                 Seleziona un'area più piccola o abbassa la risoluzione se non serve tutta.",
                memory / 1e9
            ))
            .size(13)
            .style(text::danger)
            .into()
        } else {
            Space::new().into()
        };

        let modes = column![
            radio("Colore", ModeChoice::Colour, Some(self.mode), Message::Mode),
            radio("Grigio", ModeChoice::Grey, Some(self.mode), Message::Mode),
            radio(
                "Bianco e nero",
                ModeChoice::Lineart,
                Some(self.mode),
                Message::Mode
            ),
        ]
        .spacing(6);
        let threshold: Element<'_, Message> = if self.mode == ModeChoice::Lineart {
            column![
                text(format!("Soglia: {}", self.threshold)).size(13),
                slider(0..=255u8, self.threshold, Message::Threshold),
            ]
            .spacing(4)
            .into()
        } else {
            Space::new().into()
        };

        let panel = column![
            row![
                text("HP Scanjet 3770").size(22).width(Length::Fill),
                pick_list(
                    [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark],
                    Some(self.theme),
                    Message::Theme
                )
                .text_size(13),
            ]
            .align_y(Alignment::Center),
            row![
                status.width(Length::Fill),
                button(text("Aggiorna").size(13))
                    .style(button::text)
                    .on_press_maybe(idle.then_some(Message::RefreshStatus)),
            ]
            .align_y(Alignment::Center),
            rule::horizontal(1),
            text("Risoluzione (dpi)").size(15),
            pick_list(RESOLUTIONS, Some(self.dpi), Message::Dpi).width(Length::Fill),
            text("Modo").size(15),
            modes,
            threshold,
            checkbox(self.sharpen)
                .label("Nitidezza come il driver HP")
                .on_toggle(Message::Sharpen),
            rule::horizontal(1),
            text(format!(
                "Area: {x:.0}, {y:.0} mm · {wmm:.0} × {hmm:.0} mm{}",
                if self.selection.is_none() {
                    " (A4)"
                } else {
                    ""
                }
            ))
            .size(13),
            text(format!(
                "{w} × {h} pixel · file circa {:.1} MB",
                file_bytes / 1e6
            ))
            .size(13),
            warning,
            action("Anteprima", Message::Preview),
            action("Scansiona (aggiunge una pagina)", Message::Scan),
            button(text("Salva questa pagina…").width(Length::Fill).center())
                .width(Length::Fill)
                .style(button::secondary)
                .on_press_maybe(
                    (self.shown().is_some() && self.busy.is_none()).then_some(Message::Save)
                ),
            rule::horizontal(1),
            text(format!("Fascicolo: {} pagine", self.pages.len())).size(15),
            pick_list(
                [PdfSize::Fit, PdfSize::A4, PdfSize::Letter],
                Some(self.pdf_size),
                Message::PdfSize
            )
            .width(Length::Fill)
            .text_size(13),
            button(text("Esporta fascicolo…").width(Length::Fill).center())
                .width(Length::Fill)
                .on_press_maybe(
                    (!self.pages.is_empty() && self.busy.is_none()).then_some(Message::Export)
                ),
            button(text("Nuovo fascicolo").width(Length::Fill).center())
                .width(Length::Fill)
                .style(button::secondary)
                .on_press_maybe(
                    (!self.pages.is_empty() && self.busy.is_none()).then_some(Message::NewDocument)
                ),
        ]
        .spacing(10)
        .width(320);

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
                    Some(Stage::Finishing) => (0.97, format!("{what}: chiusura ed elaborazione")),
                }
            }
        };
        let bottom = row![
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
        .align_y(Alignment::Center);

        container(column![row![left, panel].spacing(20).height(Length::Fill), bottom].spacing(12))
            .padding(16)
            .into()
    }
}
