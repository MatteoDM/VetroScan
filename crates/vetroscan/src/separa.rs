// SPDX-License-Identifier: GPL-2.0-or-later
//! "Separa" in steps: the originals found in a page scanned whole (the
//! photos on the glass, the frames of a film) shown on it, to correct,
//! and cut out only once they are as wanted.
//!
//! An area is chosen with a click and moved or resized with the handles
//! of the viewer; one is added by dragging on an empty part, one left out
//! from the panel. The handles move the sides as if the area were
//! upright; its outline shows the cut, at its angle, which the panel
//! changes.

use crate::i18n::{tr, trf};
use crate::viewer::{self, Picture, Selection};
use crate::{draw_page, heading, App, Message, Page, Tab, View, PREVIEW_DPI};
use iced::widget::{button, column, row, text, Space};
use iced::{Alignment, Element, Length, Size};
use scan_imaging::detect::Found;
use scan_imaging::transform::Transform;

/// The smallest side of an original, millimetres: below it an area drawn
/// by a slip of the mouse.
const MIN_MM: f32 = 5.0;

/// The largest turn of an area, degrees (as the automatic crop finds).
const MAX_TILT: f32 = 45.0;

/// "Separa" open on a page.
pub struct Separating {
    /// The page scanned whole.
    pub index: usize,
    /// The areas to cut out, in pixels of the scan at [`PREVIEW_DPI`].
    pub found: Vec<Found>,
    /// What was found first ("Riconosci di nuovo").
    detected: Vec<Found>,
    /// The area chosen, whose sides the handles move.
    pub chosen: Option<usize>,
    /// The scan as it was made (no turns, no crop), with its look.
    picture: Picture,
    /// Pixels of the scan per pixel at [`PREVIEW_DPI`].
    k: f32,
    /// Its millimetres per pixel of the scan.
    mm: f32,
    film: bool,
}

impl Separating {
    /// The areas big enough to be originals.
    pub fn kept(&self) -> Vec<Found> {
        let min = MIN_MM / 25.4 * f32::from(PREVIEW_DPI);
        self.found
            .iter()
            .copied()
            .filter(|f| f.width >= min && f.height >= min)
            .collect()
    }

    /// The area chosen as the viewer edits it: upright, around its centre,
    /// in pixels of the scan.
    fn selection(&self) -> Option<Selection> {
        let f = self.found.get(self.chosen?)?;
        Some(Selection {
            x: (f.cx - f.width / 2.0) * self.k,
            y: (f.cy - f.height / 2.0) * self.k,
            width: f.width * self.k,
            height: f.height * self.k,
        })
    }
}

/// Messages of "Separa".
#[derive(Debug, Clone)]
pub enum SplitMsg {
    /// An area chosen on the viewer or in the list.
    Choose(usize),
    /// The viewer's selection: the area chosen moved or resized, a new one
    /// drawn, or none chosen.
    Area(Option<Selection>),
    /// An area left out.
    Drop(usize),
    /// The area chosen turned by these degrees.
    Tilt(f32),
    /// The area chosen upright.
    Upright,
    /// Back to the areas found first.
    Detect,
    /// Cut them out.
    Apply,
    Cancel,
}

impl From<SplitMsg> for Message {
    fn from(m: SplitMsg) -> Self {
        Message::Split(m)
    }
}

fn choose_message(i: usize) -> Message {
    Message::Split(SplitMsg::Choose(i))
}

fn area_message(s: Option<Selection>) -> Message {
    Message::Split(SplitMsg::Area(s))
}

impl App {
    /// Opens "Separa" on page `i`: the originals found shown on the scan,
    /// nothing cut yet.
    pub fn begin_split(&mut self, i: usize) {
        let Some(page) = self.pages.get(i).filter(|p| self.splittable(p)) else {
            return;
        };
        let found = Self::originals(page);
        let film = page
            .whole
            .as_ref()
            .is_some_and(|w| w.transmission.is_some());
        let shared = *self.settings();
        let picture = scan_picture(page, &shared, &self.screen);
        let full = &page.raw.full;
        let dpi = f32::from(full.dpi.max(1));
        self.close_editor(true);
        self.separating = Some(Separating {
            index: i,
            detected: found.clone(),
            chosen: None,
            k: dpi / f32::from(PREVIEW_DPI),
            mm: 25.4 / dpi,
            film,
            picture,
            found,
        });
        self.current = Some(i);
        self.redone = None;
        self.tab = Tab::Result;
        self.result_view = View::default();
        self.eyedropper = false;
        self.before = false;
        self.note = self.split_note();
    }

    /// What the note says while separating.
    fn split_note(&self) -> String {
        let Some(sep) = &self.separating else {
            return String::new();
        };
        match (sep.found.len(), sep.film) {
            (0, true) => {
                tr("Nessun fotogramma riconosciuto: trascina sull'immagine per indicarne uno")
                    .into()
            }
            (0, false) => {
                tr("Nessun originale riconosciuto: trascina sull'immagine per indicarne uno").into()
            }
            (n, _) => trf(
                "{n} aree da separare: correggile se serve, poi conferma (Invio); Esc annulla",
                &[("n", &n)],
            ),
        }
    }

    /// Closes "Separa" without cutting anything.
    pub fn cancel_split(&mut self) {
        if self.separating.take().is_some() {
            self.result_view = View::default();
            self.note = tr("Separazione annullata: la scansione è rimasta intera").into();
        }
    }

    pub fn on_split(&mut self, message: SplitMsg) {
        let Some(sep) = self.separating.as_mut() else {
            return;
        };
        let k = sep.k;
        match message {
            SplitMsg::Choose(i) => sep.chosen = (i < sep.found.len()).then_some(i),
            SplitMsg::Area(None) => sep.chosen = None,
            SplitMsg::Area(Some(s)) => {
                let (cx, cy) = ((s.x + s.width / 2.0) / k, (s.y + s.height / 2.0) / k);
                let (width, height) = (s.width / k, s.height / k);
                match sep.chosen.and_then(|i| sep.found.get_mut(i)) {
                    Some(f) => {
                        *f = Found {
                            cx,
                            cy,
                            width,
                            height,
                            angle: f.angle,
                        }
                    }
                    // A new area, once it is drawn large enough to be one.
                    None => {
                        let min = MIN_MM / 25.4 * f32::from(PREVIEW_DPI);
                        if width >= min && height >= min {
                            sep.found.push(Found {
                                cx,
                                cy,
                                width,
                                height,
                                angle: 0.0,
                            });
                            sep.chosen = Some(sep.found.len() - 1);
                        }
                    }
                }
            }
            SplitMsg::Drop(i) => {
                if i < sep.found.len() {
                    sep.found.remove(i);
                    sep.chosen = match sep.chosen {
                        Some(c) if c == i => None,
                        Some(c) if c > i => Some(c - 1),
                        c => c,
                    };
                }
                self.note = self.split_note();
            }
            SplitMsg::Tilt(d) => {
                if let Some(f) = sep.chosen.and_then(|i| sep.found.get_mut(i)) {
                    f.angle = ((f.angle + d) * 10.0)
                        .round()
                        .clamp(-MAX_TILT * 10.0, MAX_TILT * 10.0)
                        / 10.0;
                }
            }
            SplitMsg::Upright => {
                if let Some(f) = sep.chosen.and_then(|i| sep.found.get_mut(i)) {
                    f.angle = 0.0;
                }
            }
            SplitMsg::Detect => {
                sep.found = sep.detected.clone();
                sep.chosen = None;
                self.note = self.split_note();
            }
            SplitMsg::Apply => {
                let kept = sep.kept();
                if kept.is_empty() {
                    self.note =
                        tr("Nessuna area da separare: trascina sull'immagine per indicarne una")
                            .into();
                    return;
                }
                let index = sep.index;
                self.separating = None;
                self.split_into(index, kept);
            }
            SplitMsg::Cancel => self.cancel_split(),
        }
    }

    // ---------------------------------------------------------- views

    /// The scan with the areas on it.
    pub fn split_viewer(&self) -> Element<'_, Message> {
        let Some(sep) = &self.separating else {
            return Space::new().into();
        };
        viewer::viewer(
            Some(&sep.picture),
            sep.picture.size,
            "",
            self.result_view,
            sep.selection(),
            sep.found
                .iter()
                .map(|f| f.scaled(sep.k, 0.0, 0.0).corners())
                .collect(),
            Message::View,
            Some(area_message),
            None,
            Some(choose_message),
            sep.mm,
            false,
        )
    }

    /// The panel while separating.
    pub fn split_pane(&self) -> Element<'_, Message> {
        let Some(sep) = &self.separating else {
            return Space::new().into();
        };
        let mm = 25.4 / f32::from(PREVIEW_DPI);
        let mut list = column![
            text(trf(
                "SEPARA · {name}",
                &[("name", &self.page_name(sep.index).to_uppercase())],
            ))
            .size(11),
            text(tr(
                "Clic su un'area per sceglierla: trascinala o tirane i lati con le maniglie. \
                 Trascina su un punto vuoto per aggiungerne una."
            ))
            .size(11),
            heading(if sep.film {
                tr("Fotogrammi")
            } else {
                tr("Originali")
            }),
        ]
        .spacing(8);
        if sep.found.is_empty() {
            list = list.push(
                text(tr(
                    "Nessuna area: trascina sull'immagine per aggiungerne una.",
                ))
                .size(12),
            );
        }
        for (i, f) in sep.found.iter().enumerate() {
            let tilt = if f.angle == 0.0 {
                tr("dritto").to_string()
            } else {
                trf(
                    "storto di {deg}°",
                    &[(
                        "deg",
                        &crate::i18n::decimal(format!("{:.1}", f.angle.abs())),
                    )],
                )
            };
            let chosen = sep.chosen == Some(i);
            list = list.push(
                row![
                    button(
                        text(format!(
                            "{} · {:.0} × {:.0} mm · {tilt}",
                            i + 1,
                            f.width * mm,
                            f.height * mm
                        ))
                        .size(12)
                    )
                    .width(Length::Fill)
                    .padding([2, 6])
                    .style(if chosen {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(SplitMsg::Choose(i).into()),
                    button(text(tr("Escludi")).size(11))
                        .padding([1, 6])
                        .style(button::text)
                        .on_press(SplitMsg::Drop(i).into()),
                ]
                .align_y(Alignment::Center),
            );
        }
        if let Some(f) = sep.chosen.and_then(|i| sep.found.get(i)) {
            let nudge = |d: f32| {
                let sign = if d < 0.0 { "−" } else { "+" };
                let label = format!("{sign}{}°", crate::editor::number(d.abs(), 1));
                button(text(label).size(11).center().width(Length::Fill))
                    .width(Length::Fill)
                    .padding([3, 4])
                    .style(button::secondary)
                    .on_press(SplitMsg::Tilt(d).into())
            };
            list = list
                .push(text(trf("Area {n}", &[("n", &(sep.chosen.unwrap_or(0) + 1))])).size(16))
                .push(row![
                    text(tr("Inclinazione")).size(12).width(Length::Fill),
                    text(format!("{}°", crate::editor::number(f.angle, 1))).size(12),
                ])
                .push(row![nudge(-1.0), nudge(-0.1), nudge(0.1), nudge(1.0)].spacing(4))
                .push(
                    button(text(tr("Dritta")).size(12))
                        .style(button::text)
                        .padding([2, 0])
                        .on_press_maybe((f.angle != 0.0).then_some(SplitMsg::Upright.into())),
                );
        }
        list.push(
            button(text(tr("Riconosci di nuovo")).size(12))
                .padding([3, 8])
                .style(button::secondary)
                .on_press(SplitMsg::Detect.into()),
        )
        .into()
    }

    /// Below the panel: leave it whole, or cut.
    pub fn split_footer(&self) -> Element<'_, Message> {
        let n = self.separating.as_ref().map_or(0, |s| s.kept().len());
        let label = if n == 1 {
            tr("Ritaglia").to_string()
        } else {
            trf("Separa in {n}", &[("n", &n)])
        };
        row![
            button(text(tr("Annulla")).size(14).center().width(Length::Fill))
                .width(Length::Fill)
                .padding([9, 12])
                .style(button::secondary)
                .on_press(SplitMsg::Cancel.into()),
            button(text(label).size(14).center().width(Length::Fill))
                .width(Length::Fill)
                .padding([9, 12])
                .on_press_maybe((n > 0).then_some(SplitMsg::Apply.into())),
        ]
        .spacing(8)
        .into()
    }
}

/// The scan of a page as it was made, with its look, at the size it
/// stands for.
fn scan_picture(page: &Page, shared: &crate::Settings, screen: &crate::screen::Screen) -> Picture {
    let settings = page.effective(shared);
    let full = &page.raw.full;
    let size = Size::new(full.width as f32, full.height as f32);
    let (_, picture, _) = draw_page(
        &page.raw,
        &settings,
        &settings.adjust,
        &Transform::default(),
        size,
        false,
        screen,
    );
    picture
}
