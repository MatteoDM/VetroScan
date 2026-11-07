// SPDX-License-Identifier: GPL-2.0-or-later
//! `hp3770-app`: desktop application for the HP Scanjet 3770.
//!
//! A preview of the glass on the left, on which the area to scan is drawn;
//! the settings and the last scan on the right; progress at the bottom.
//! Every scanner request goes through [`worker`].

#![windows_subsystem = "windows"]

mod preview;
mod save;
mod worker;

use hp3770_core::convert;
use hp3770_core::geometry::{Area, GLASS_INCHES, RESOLUTIONS};
use hp3770_core::session::{self, Image, Report, Request, Stage};
use hp3770_core::status::Status;
use iced::widget::{
    button, canvas, checkbox, column, container, image, pick_list, progress_bar, radio, row, rule,
    slider, text, Space,
};
use iced::{Alignment, Element, Length, Size, Subscription, Task, Theme};
use preview::{Preview, Selection};
use std::path::PathBuf;
use std::sync::Arc;
use worker::{Command, Event, Handle, Purpose};

/// Resolution of the preview of the whole glass (the lowest the scanner
/// has).
const PREVIEW_DPI: u16 = 200;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("HP Scanjet 3770")
        .subscription(App::subscription)
        .theme(|_: &App| Theme::Light)
        .window_size(Size::new(1200.0, 820.0))
        .run()
}

/// The colour mode as the interface offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeChoice {
    Colour,
    Grey,
    Lineart,
}

#[derive(Debug, Clone)]
enum Message {
    Worker(Event),
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
}

struct App {
    worker: Option<Handle>,
    status: Option<Result<Status, String>>,
    /// The preview: display handle, size, image.
    preview: Option<(image::Handle, Size)>,
    selection: Option<Selection>,
    dpi: u16,
    mode: ModeChoice,
    threshold: u8,
    sharpen: bool,
    /// The scan running, and where it is.
    busy: Option<(Purpose, Option<Stage>)>,
    /// The last page scanned and its display handle.
    page: Option<(Arc<(Image, Report)>, image::Handle)>,
    /// Last thing that happened, for the status line.
    note: String,
}

impl App {
    fn new() -> Self {
        Self {
            worker: None,
            status: None,
            preview: None,
            selection: None,
            dpi: 300,
            mode: ModeChoice::Colour,
            threshold: convert::DEFAULT_THRESHOLD,
            sharpen: true,
            busy: None,
            page: None,
            note: "Pronto".into(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(worker::events).map(Message::Worker)
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
            Some(s) if s.width >= 2.0 && s.height >= 2.0 => {
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
                if self.page.is_some() {
                    return Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .add_filter("PNG", &["png"])
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
                if let Some((page, _)) = &self.page {
                    self.note = match save::png(&path, &page.0) {
                        Ok(()) => format!("Salvata in {}", path.display()),
                        Err(e) => format!("Salvataggio non riuscito: {e}"),
                    };
                }
            }
            Message::SaveTo(None) => {}
        }
        Task::none()
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
                        let handle = image::Handle::from_rgba(
                            img.width as u32,
                            img.height as u32,
                            rgba(&img.to_rgb()),
                        );
                        match purpose {
                            Purpose::Preview => {
                                self.preview =
                                    Some((handle, Size::new(img.width as f32, img.height as f32)));
                                self.note =
                                    "Anteprima pronta: trascina sull'immagine per scegliere l'area"
                                        .into();
                            }
                            Purpose::Page => {
                                self.note = format!(
                                    "Scansione {} × {} a {} dpi{}",
                                    img.width,
                                    img.height,
                                    img.dpi,
                                    if report.warmed_up {
                                        " (lampada riscaldata)"
                                    } else {
                                        ""
                                    }
                                );
                                self.page = Some((done.clone(), handle));
                            }
                        }
                    }
                    Err(e) => self.note = format!("Errore: {e}"),
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

    fn view(&self) -> Element<'_, Message> {
        let preview = canvas(Preview {
            image: self.preview.as_ref().map(|(h, s)| (h, *s)),
            glass: Self::glass(),
            selection: self.selection,
            on_select: Message::Select,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        let idle = self.busy.is_none() && self.worker.is_some();
        let action = |label, message| {
            button(text(label).width(Length::Fill).center())
                .width(Length::Fill)
                .on_press_maybe(idle.then_some(message))
        };

        let status = match &self.status {
            None => text("Stato: …"),
            Some(Err(e)) => text(format!("Scanner non trovato: {e}")),
            Some(Ok(s)) => text(format!(
                "Lampada: {}\nCarrello: {}",
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
        };

        let request = self.request();
        let (w, h) = (request.area.width, request.area.height);
        let bytes = match self.mode {
            ModeChoice::Colour => 3.0,
            ModeChoice::Grey => 1.0,
            ModeChoice::Lineart => 0.125,
        } * f64::from(w)
            * f64::from(h);
        let (x, y, wmm, hmm) = self.area_mm();

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

        let page: Element<'_, Message> = match &self.page {
            Some((p, handle)) => column![
                text("Ultima scansione").size(15),
                image(handle.clone()).height(180),
                text(format!(
                    "{} × {} pixel, {} dpi",
                    p.0.width, p.0.height, p.0.dpi
                ))
                .size(13),
            ]
            .spacing(6)
            .into(),
            None => Space::new().into(),
        };

        let panel = column![
            text("HP Scanjet 3770").size(22),
            status,
            button(text("Aggiorna stato")).on_press_maybe(idle.then_some(Message::RefreshStatus)),
            rule::horizontal(1),
            text("Risoluzione").size(15),
            pick_list(RESOLUTIONS, Some(self.dpi), Message::Dpi).width(Length::Fill),
            text("Modo").size(15),
            modes,
            threshold,
            checkbox(self.sharpen)
                .label("Nitidezza come il driver HP")
                .on_toggle(Message::Sharpen),
            rule::horizontal(1),
            text(format!(
                "Area: {x:.0}, {y:.0} mm, {wmm:.0} × {hmm:.0} mm{}",
                if self.selection.is_none() {
                    " (A4)"
                } else {
                    ""
                }
            ))
            .size(13),
            text(format!("{w} × {h} pixel, circa {:.1} MB", bytes / 1e6)).size(13),
            action("Anteprima", Message::Preview),
            action("Scansiona", Message::Scan),
            button(text("Salva PNG…").width(Length::Fill).center())
                .width(Length::Fill)
                .on_press_maybe(
                    (self.page.is_some() && self.busy.is_none()).then_some(Message::Save)
                ),
            rule::horizontal(1),
            page,
        ]
        .spacing(10)
        .width(300);

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
        let bottom = row![
            text(if stage.is_empty() {
                self.note.clone()
            } else {
                stage
            })
            .width(Length::Fill),
            progress_bar(0.0..=1.0, fraction).length(260).girth(14),
            button(text("Annulla")).on_press_maybe(self.busy.is_some().then_some(Message::Cancel)),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        container(
            column![
                row![preview, panel].spacing(16).height(Length::Fill),
                bottom
            ]
            .spacing(12),
        )
        .padding(16)
        .into()
    }
}

/// RGB to RGBA for display.
fn rgba(rgb: &[u8]) -> Vec<u8> {
    rgb.chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}
