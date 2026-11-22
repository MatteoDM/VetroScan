// SPDX-License-Identifier: GPL-2.0-or-later
//! The areas to scan, on the preview, in every way of working: the
//! originals found there (the photos on the glass, the frames of a film),
//! corrected by hand if need be, and areas drawn by hand. Each becomes an
//! image (a page in Documento).
//!
//! An area is chosen with a click and moved or resized with the handles of
//! the viewer; one is added by dragging on an empty part, one left out
//! from the panel or with Delete. Scansiona scans each: on films always one
//! scan per area (at high resolutions the strip around them would be most
//! of the scan), on the glass one pass for several originals found when it
//! fits in [`crate::ONE_PASS`].

use crate::i18n::{tr, trf};
use crate::viewer::Selection;
use crate::{heading, App, Job, Kind, Message, MmField, Tab, PREVIEW_DPI};
use iced::widget::{button, checkbox, row, text, Column};
use iced::{Alignment, Length};
use scan_imaging::detect::Found;

/// The smallest side of an area, millimetres: below it an area drawn by a
/// slip of the mouse.
pub const MIN_MM: f32 = 5.0;

/// The largest turn of an area, degrees (as the automatic crop finds).
pub const MAX_TILT: f32 = 45.0;

/// An area to scan, in pixels of the preview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub at: Found,
    /// Found by the automatic crop (shown only while it is on), not drawn
    /// by hand.
    pub auto: bool,
}

impl Region {
    /// An area drawn by hand: scanned as it is, while it stays upright.
    pub fn drawn(s: Selection) -> Self {
        Self {
            at: Found {
                cx: s.x + s.width / 2.0,
                cy: s.y + s.height / 2.0,
                width: s.width,
                height: s.height,
                angle: 0.0,
            },
            auto: false,
        }
    }

    /// Scanned as it is, nothing cut out of it: drawn by hand and upright.
    /// An original found is cut a little inside its edge and straightened.
    pub fn exact(&self) -> bool {
        !self.auto && self.at.angle == 0.0
    }

    /// The area as the viewer's handles move it: as if upright, around its
    /// centre (the viewer turns it by its angle).
    pub fn rect(&self) -> Selection {
        upright(&self.at, 1.0)
    }
}

/// An area upright around its centre, scaled by `k`.
pub fn upright(f: &Found, k: f32) -> Selection {
    Selection {
        x: (f.cx - f.width / 2.0) * k,
        y: (f.cy - f.height / 2.0) * k,
        width: f.width * k,
        height: f.height * k,
    }
}

/// `f` moved or resized to the upright rectangle `s` (scaled by `k`),
/// keeping its angle.
pub fn resized(f: Found, s: Selection, k: f32) -> Found {
    Found {
        cx: (s.x + s.width / 2.0) / k,
        cy: (s.y + s.height / 2.0) / k,
        width: s.width / k,
        height: s.height / k,
        angle: f.angle,
    }
}

/// Whether a rectangle (pixels of the preview) is large enough to be an
/// area rather than a slip of the mouse.
pub fn large_enough(width: f32, height: f32) -> bool {
    let min = MIN_MM / 25.4 * f32::from(PREVIEW_DPI);
    width >= min && height >= min
}

/// An angle turned by `d` degrees, in tenths, within [`MAX_TILT`].
pub fn tilted(angle: f32, d: f32) -> f32 {
    ((angle + d) * 10.0)
        .round()
        .clamp(-MAX_TILT * 10.0, MAX_TILT * 10.0)
        / 10.0
}

/// "dritto", or how crooked.
pub fn tilt_label(angle: f32) -> String {
    if angle == 0.0 {
        tr("dritto").to_string()
    } else {
        trf(
            "storto di {deg}°",
            &[("deg", &crate::i18n::decimal(format!("{:.1}", angle.abs())))],
        )
    }
}

/// Messages of the areas on the preview.
#[derive(Debug, Clone)]
pub enum AreaMsg {
    /// The area at this place among those shown chosen (on the viewer or
    /// in the list).
    Choose(usize),
    /// The area chosen turned by these degrees.
    Tilt(f32),
    /// The area chosen upright.
    Upright,
}

impl From<AreaMsg> for Message {
    fn from(m: AreaMsg) -> Self {
        Message::Area(m)
    }
}

/// For the viewer, which takes a function.
pub fn choose_message(i: usize) -> Message {
    Message::Area(AreaMsg::Choose(i))
}

impl App {
    /// Where the areas shown are in [`App::regions`]: those drawn by hand,
    /// and those found while the automatic crop is on.
    pub fn shown_places(&self) -> Vec<usize> {
        let auto = self.settings().auto_crop;
        (0..self.regions.len())
            .filter(|&i| auto || !self.regions[i].auto)
            .collect()
    }

    /// The areas shown, in order: each becomes an image.
    pub fn shown_regions(&self) -> Vec<Region> {
        self.shown_places()
            .into_iter()
            .map(|i| self.regions[i])
            .collect()
    }

    /// The place among those shown of the area chosen.
    pub fn chosen_place(&self) -> Option<usize> {
        let c = self.chosen?;
        self.shown_places().iter().position(|&i| i == c)
    }

    /// The area chosen, in [`App::regions`], while it is shown (an
    /// original found stays chosen in Foto while Documento, without the
    /// automatic crop, hides it).
    pub fn chosen_shown(&self) -> Option<usize> {
        self.chosen_place().and(self.chosen)
    }

    /// The area chosen, upright, as the viewer's handles move it.
    pub fn chosen_rect(&self) -> Option<Selection> {
        self.regions.get(self.chosen_shown()?).map(Region::rect)
    }

    /// The turn of the area chosen, degrees (0 when none is chosen): the
    /// viewer draws it and moves its sides at this angle.
    pub fn chosen_angle(&self) -> f32 {
        self.chosen_shown()
            .and_then(|c| self.regions.get(c))
            .map_or(0.0, |r| r.at.angle)
    }

    /// The area chosen, to change, while it is shown.
    pub fn chosen_mut(&mut self) -> Option<&mut Region> {
        let c = self.chosen_shown()?;
        self.regions.get_mut(c)
    }

    /// The viewer's selection: the area chosen moved or resized, a new one
    /// drawn (once large enough to be one), or none chosen.
    pub fn select_area(&mut self, s: Option<Selection>) {
        let Some(s) = s else {
            self.chosen = None;
            return;
        };
        match self.chosen_mut() {
            Some(r) => r.at = resized(r.at, s, 1.0),
            None => {
                if large_enough(s.width, s.height) {
                    self.regions.push(Region::drawn(s));
                    self.chosen = Some(self.regions.len() - 1);
                }
            }
        }
    }

    /// The area at place `i` among those shown left out.
    pub fn drop_area(&mut self, i: usize) {
        let Some(&at) = self.shown_places().get(i) else {
            return;
        };
        self.regions.remove(at);
        self.chosen = match self.chosen {
            Some(c) if c == at => None,
            Some(c) if c > at => Some(c - 1),
            c => c,
        };
    }

    /// The originals found on the preview put in place of those found
    /// before (corrected or not); the areas drawn by hand stay.
    pub fn set_found(&mut self, found: Vec<Found>) {
        self.regions.retain(|r| !r.auto);
        self.regions
            .extend(found.into_iter().map(|at| Region { at, auto: true }));
        self.chosen = None;
    }

    /// The originals found left out (another film in the holder); the
    /// areas drawn by hand stay.
    pub fn forget_found(&mut self) {
        self.set_found(Vec::new());
    }

    pub fn on_area(&mut self, message: AreaMsg) {
        match message {
            AreaMsg::Choose(i) => self.chosen = self.shown_places().get(i).copied(),
            AreaMsg::Tilt(d) => {
                if let Some(r) = self.chosen_mut() {
                    r.at.angle = tilted(r.at.angle, d);
                }
            }
            AreaMsg::Upright => {
                if let Some(r) = self.chosen_mut() {
                    r.at.angle = 0.0;
                }
            }
        }
    }

    /// The automatic crop on or off: an original found and chosen is no
    /// longer chosen once it is hidden.
    pub fn auto_crop_changed(&mut self) {
        if !self.settings().auto_crop
            && self
                .chosen
                .and_then(|c| self.regions.get(c))
                .is_some_and(|r| r.auto)
        {
            self.chosen = None;
        }
    }

    /// The passes for the areas shown: on films one for each; on the glass
    /// one for all the originals found when it fits in memory (and no area
    /// is drawn by hand), else one for each. An area drawn by hand is
    /// scanned as it is; an original found is cut out of its pass,
    /// straightened.
    pub fn plan(&self) -> Vec<Job> {
        let regions = self.shown_regions();
        if regions.is_empty() {
            return Vec::new();
        }
        let glass = self.glass();
        let clamp = |l: f32, t: f32, r: f32, b: f32| {
            let (l, t) = (l.max(0.0), t.max(0.0));
            let (r, b) = (r.min(glass.width), b.min(glass.height));
            [l, t, (r - l).max(1.0), (b - t).max(1.0)]
        };
        let margin = f32::from(PREVIEW_DPI) / 25.4;
        let expand = |b: [f32; 4]| {
            clamp(
                b[0] - margin,
                b[1] - margin,
                b[0] + b[2] + margin,
                b[1] + b[3] + margin,
            )
        };
        let alone = |r: &Region| {
            let b = r.at.bounds();
            if r.exact() {
                Job {
                    bounds: clamp(b[0], b[1], b[0] + b[2], b[1] + b[3]),
                    crops: Vec::new(),
                    area: None,
                    find: false,
                }
            } else {
                Job {
                    bounds: expand(b),
                    crops: vec![r.at],
                    area: None,
                    find: false,
                }
            }
        };
        if regions.len() > 1 && !self.filming() && regions.iter().all(|r| !r.exact()) {
            let union = regions
                .iter()
                .map(|r| r.at.bounds())
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
                crops: regions.iter().map(|r| r.at).collect(),
                area: None,
                find: false,
            };
            if self.job_memory(&one) <= crate::ONE_PASS {
                return vec![one];
            }
        }
        regions.iter().map(alone).collect()
    }

    // ---------------------------------------------------------- views

    /// What the panel says of the areas shown, if anything.
    fn areas_summary(&self, regions: &[Region], plan: &[Job]) -> Option<String> {
        let n = regions.len();
        let auto = regions.iter().all(|r| r.auto);
        Some(match (self.filming(), self.kind, n, auto) {
            (true, _, 1, _) => tr("Una foto da scansionare.").into(),
            (true, _, n, _) => trf(
                "{n} foto da scansionare: una scansione per ciascuna, ognuna un'immagine.",
                &[("n", &n)],
            ),
            (false, _, 1, false) => return None,
            (false, Kind::Photo, 1, true) => {
                tr("Un originale riconosciuto sul vetro: sarà ritagliato e raddrizzato.").into()
            }
            (false, Kind::Photo, n, true) => trf(
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
            (false, Kind::Document, n, _) => trf(
                "{n} aree: ognuna diventa una pagina, con una scansione per ciascuna.",
                &[("n", &n)],
            ),
            (false, _, n, _) => trf(
                "{n} aree: ognuna diventa un'immagine, con una scansione per ciascuna.",
                &[("n", &n)],
            ),
        })
    }

    /// The "Area" part of the panel of Scansione: the automatic crop, the
    /// areas shown and the one chosen, or the whole glass.
    pub fn area_section<'a>(&'a self, pane: Column<'a, Message>) -> Column<'a, Message> {
        let pane = self.areas_part(pane);
        if self.settings().auto_crop && self.preview.is_some() {
            pane.push(
                button(text(tr("Riconosci di nuovo")).size(12))
                    .padding([3, 8])
                    .style(button::secondary)
                    .on_press(Message::Detect),
            )
        } else {
            pane
        }
    }

    fn areas_part<'a>(&'a self, mut pane: Column<'a, Message>) -> Column<'a, Message> {
        let request = self.request();
        let (w, h) = (request.area.width, request.area.height);
        let (x, y, wmm, hmm) = self.area_mm();
        let memory = self.memory();
        let auto_crop = self.settings().auto_crop;
        pane = pane.push(heading(tr("Area"))).push(
            checkbox(auto_crop)
                .label(tr("Ritaglio automatico"))
                .on_toggle(Message::AutoCrop)
                .text_size(13),
        );
        let regions = self.shown_regions();
        let fields = |pane: Column<'a, Message>| {
            pane.push(self.mm_pair(
                (tr("Da sinistra"), MmField::Area(0), x, true),
                (tr("Dall'alto"), MmField::Area(1), y, true),
            ))
            .push(self.mm_pair(
                (tr("Larghezza"), MmField::Area(2), wmm, true),
                (tr("Altezza"), MmField::Area(3), hmm, true),
            ))
        };
        if regions.is_empty() {
            if auto_crop && self.preview.is_some() {
                pane = pane.push(
                    text(if self.filming() {
                        tr("Nessuna foto riconosciuta: trascina sull'anteprima per indicarne una.")
                    } else {
                        tr(
                            "Nessun originale riconosciuto (un foglio bianco sul coperchio \
                         bianco può sfuggire): trascina sull'anteprima per scegliere l'area.",
                        )
                    })
                    .size(11),
                );
            }
            pane = pane.push(
                text(if self.filming() {
                    tr(
                        "Tutto il supporto della pellicola. Fai l'anteprima per scegliere le \
                         foto; senza, con il ritaglio automatico Scansiona le trova da sé, \
                         ognuna un'immagine.",
                    )
                } else {
                    tr(
                        "Tutto il formato A4. Trascina sull'anteprima per scegliere un'area, \
                            o scrivila qui sotto.",
                    )
                })
                .size(11),
            );
            return fields(pane).push(
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
        let plan = self.plan();
        if let Some(summary) = self.areas_summary(&regions, &plan) {
            pane = pane.push(text(summary).size(12));
        }
        pane = pane.push(
            text(tr(
                "Clic su un'area per sceglierla: trascinala o tirane i lati con le maniglie. \
                 Trascina su un punto vuoto per aggiungerne una.",
            ))
            .size(11),
        );
        let mm = 25.4 / f32::from(PREVIEW_DPI);
        let chosen = self.chosen_place();
        for (i, r) in regions.iter().enumerate() {
            let f = r.at;
            pane = pane.push(
                row![
                    button(
                        text(format!(
                            "{} · {:.0} × {:.0} mm · {}",
                            i + 1,
                            f.width * mm,
                            f.height * mm,
                            tilt_label(f.angle)
                        ))
                        .size(12)
                    )
                    .width(Length::Fill)
                    .padding([2, 6])
                    .style(if chosen == Some(i) {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(AreaMsg::Choose(i).into()),
                    button(text(tr("Escludi")).size(11))
                        .padding([1, 6])
                        .style(button::text)
                        .on_press(Message::DropFound(i)),
                ]
                .align_y(Alignment::Center),
            );
        }
        if let (Some(i), Some(r)) = (chosen, self.chosen_shown().map(|c| self.regions[c])) {
            let angle = r.at.angle;
            let nudge = |d: f32| {
                let sign = if d < 0.0 { "−" } else { "+" };
                let label = format!("{sign}{}°", crate::editor::number(d.abs(), 1));
                button(text(label).size(11).center().width(Length::Fill))
                    .width(Length::Fill)
                    .padding([3, 4])
                    .style(button::secondary)
                    .on_press(AreaMsg::Tilt(d).into())
            };
            pane = fields(pane.push(text(trf("Area {n}", &[("n", &(i + 1))])).size(16)))
                .push(
                    text(tr(
                        "Frecce: sposta l'area di 1 mm (Maiusc 10 mm, Alt 0,1 mm); \
                            Ctrl + frecce: la misura; Home: al centro del vetro.",
                    ))
                    .size(11),
                )
                .push(row![
                    text(tr("Inclinazione")).size(12).width(Length::Fill),
                    text(format!("{}°", crate::editor::number(angle, 1))).size(12),
                ])
                .push(row![nudge(-1.0), nudge(-0.1), nudge(0.1), nudge(1.0)].spacing(4))
                .push(
                    button(text(tr("Dritta")).size(12))
                        .style(button::text)
                        .padding([2, 0])
                        .on_press_maybe((angle != 0.0).then_some(AreaMsg::Upright.into())),
                );
        }
        pane.push(
            text(trf(
                "in memoria fino a {mb} MB",
                &[("mb", &format!("{:.0}", memory / 1e6))],
            ))
            .size(12),
        )
    }

    /// Whether the keys move an area on the preview.
    pub fn moving_area(&self) -> bool {
        self.tab == Tab::Preview && self.chosen_shown().is_some()
    }
}
