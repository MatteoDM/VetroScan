// SPDX-License-Identifier: GPL-2.0-or-later
//! The editor of a scan already made: its colours again, turning,
//! straightening and cropping, its size.
//!
//! Nothing is taken from the scanner's samples: a page keeps its look and
//! its [`Transform`], and the editor changes only those, live, on the page
//! itself. What it was when the editor opened is kept, so "Scarta" puts it
//! back; the editor has its own Ctrl+Z.

use crate::viewer::Selection;
use crate::{heading, labelled, App, Colour, Field, Look, Message, Page, Tab, View, ALL_COLOURS};
use iced::widget::{
    button, checkbox, column, container, mouse_area, pick_list, row, slider, text, text_input,
    Space,
};
use iced::{Alignment, Element, Length, Theme};
use scan_imaging::transform::{Crop, Transform, MAX_ANGLE};
use std::time::{Duration, Instant};

/// The parts of the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Colour,
    Frame,
    Size,
}

/// Units of the size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Pixels,
    Percent,
    Cm,
    Inches,
}

pub const UNITS: [Unit; 4] = [Unit::Pixels, Unit::Percent, Unit::Cm, Unit::Inches];

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pixels => "Pixel",
            Self::Percent => "Percentuale",
            Self::Cm => "Centimetri",
            Self::Inches => "Pollici",
        })
    }
}

impl Unit {
    fn suffix(self) -> &'static str {
        match self {
            Self::Pixels => "px",
            Self::Percent => "%",
            Self::Cm => "cm",
            Self::Inches => "in",
        }
    }
}

/// Pixels of `v` in `unit`, for a side of `base` pixels at `dpi`.
pub fn to_pixels(v: f32, unit: Unit, base: f32, dpi: f32) -> f32 {
    match unit {
        Unit::Pixels => v,
        Unit::Percent => base * v / 100.0,
        Unit::Cm => v / 2.54 * dpi,
        Unit::Inches => v * dpi,
    }
}

/// `px` pixels in `unit`, for a side of `base` pixels at `dpi`.
pub fn from_pixels(px: f32, unit: Unit, base: f32, dpi: f32) -> f32 {
    match unit {
        Unit::Pixels => px,
        Unit::Percent => px / base * 100.0,
        Unit::Cm => px / dpi * 2.54,
        Unit::Inches => px / dpi,
    }
}

/// A number the Italian way, with `decimals` (trailing zeros dropped).
pub fn number(v: f32, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    s.replace('.', ",")
}

/// A number typed by the user (comma or point).
pub fn parse(s: &str) -> Option<f32> {
    s.trim()
        .trim_end_matches(['%', '°'])
        .trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Proportions of the crop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aspect {
    Free,
    Original,
    Square,
    R4x3,
    R3x2,
    R16x9,
    A4,
    Photo10x15,
}

pub const ASPECTS: [Aspect; 8] = [
    Aspect::Free,
    Aspect::Original,
    Aspect::Square,
    Aspect::R4x3,
    Aspect::R3x2,
    Aspect::R16x9,
    Aspect::A4,
    Aspect::Photo10x15,
];

impl std::fmt::Display for Aspect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Free => "Libere",
            Self::Original => "Come l'originale",
            Self::Square => "Quadrato 1:1",
            Self::R4x3 => "4:3",
            Self::R3x2 => "3:2",
            Self::R16x9 => "16:9",
            Self::A4 => "A4 / A3 / A5",
            Self::Photo10x15 => "Foto 10 × 15",
        })
    }
}

impl Aspect {
    /// Width over height, upright or lying, for a frame `w` × `h`.
    pub fn ratio(self, portrait: bool, w: f32, h: f32) -> Option<f32> {
        let r = match self {
            Self::Free => return None,
            Self::Original => return Some(w / h),
            Self::Square => 1.0,
            Self::R4x3 => 4.0 / 3.0,
            Self::R3x2 => 1.5,
            Self::R16x9 => 16.0 / 9.0,
            Self::A4 => std::f32::consts::SQRT_2,
            Self::Photo10x15 => 1.5,
        };
        Some(if portrait { 1.0 / r } else { r })
    }

    /// Whether upright and lying differ.
    fn turns(self) -> bool {
        !matches!(self, Self::Free | Self::Original | Self::Square)
    }
}

/// The largest rectangle (left, top, width, height) of proportions `r`
/// inside `area`, as near as it can be centred on `centre`.
pub fn fit_ratio(area: [f32; 4], centre: (f32, f32), r: f32) -> [f32; 4] {
    let [ax, ay, aw, ah] = area;
    let w = aw.min(ah * r);
    let h = w / r;
    let x = (centre.0 - w / 2.0).clamp(ax, ax + aw - w);
    let y = (centre.1 - h / 2.0).clamp(ay, ay + ah - h);
    [x, y, w, h]
}

/// A crop dragged to `sel` from `prev` (left, top, width, height, in
/// pixels of a frame `fw` × `fh`), held to proportions `r`: the side
/// moved most leads, the edges not moved stay.
pub fn constrain(sel: [f32; 4], prev: [f32; 4], r: f32, fw: f32, fh: f32) -> [f32; 4] {
    let [sx, sy, sw, sh] = sel;
    let [px, py, pw, ph] = prev;
    let (mut w, mut h) = if (sw - pw).abs() >= (sh - ph).abs() {
        (sw, sw / r)
    } else {
        (sh * r, sh)
    };
    // The edge that stays: the right one if the left moved.
    let left_moved = (sx - px).abs() > 0.01 && ((sx + sw) - (px + pw)).abs() <= 0.01;
    let top_moved = (sy - py).abs() > 0.01 && ((sy + sh) - (py + ph)).abs() <= 0.01;
    let anchor_x = if left_moved { sx + sw } else { sx };
    let anchor_y = if top_moved { sy + sh } else { sy };
    // Room from the anchors to the edges of the frame.
    let room_w = if left_moved { anchor_x } else { fw - anchor_x };
    let room_h = if top_moved { anchor_y } else { fh - anchor_y };
    let k = (room_w / w).min(room_h / h).clamp(0.0, 1.0);
    w *= k;
    h *= k;
    let x = if left_moved { anchor_x - w } else { anchor_x };
    let y = if top_moved { anchor_y - h } else { anchor_y };
    [x, y, w, h]
}

/// What the editor changes of a page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edit {
    /// Its own look (None: the shared one).
    pub look: Option<Look>,
    pub colour: Colour,
    pub transform: Transform,
}

/// What a change was, so that a slider dragged or a number typed is one
/// step of Ctrl+Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Field(Field),
    Angle,
    Crop,
    Size(SizeField),
}

/// The numbers of the size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeField {
    Width,
    Height,
    Dpi,
}

/// The editor open on a page.
#[derive(Debug)]
pub struct Editor {
    /// The page edited.
    pub index: usize,
    pub section: Section,
    /// The page as it was when the editor opened.
    pub start: Edit,
    pub undo: Vec<Edit>,
    pub redo: Vec<Edit>,
    last: Option<(Key, Instant)>,
    pub unit: Unit,
    pub lock: bool,
    pub aspect: Aspect,
    pub portrait: bool,
    /// A number being typed.
    pub typing: Option<(SizeField, String)>,
}

impl Editor {
    pub fn new(index: usize, start: Edit, section: Section, portrait: bool) -> Self {
        Self {
            index,
            section,
            start,
            undo: Vec::new(),
            redo: Vec::new(),
            last: None,
            unit: Unit::Pixels,
            lock: true,
            aspect: Aspect::Free,
            portrait,
            typing: None,
        }
    }

    /// Keeps `current` for Ctrl+Z, unless the change continues the last
    /// one (the same control within a moment).
    pub fn remember(&mut self, current: Edit, key: Option<Key>) {
        let now = Instant::now();
        let same = matches!(
            (key, self.last),
            (Some(k), Some((l, at))) if k == l && now - at < Duration::from_millis(1500)
        );
        self.last = key.map(|k| (k, now));
        if same {
            return;
        }
        self.undo.push(current);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// The next change starts a new step.
    pub fn break_step(&mut self) {
        self.last = None;
    }
}

/// Messages of the editor.
#[derive(Debug, Clone)]
pub enum EditMsg {
    Open(usize),
    Apply,
    Discard,
    Section(Section),
    Undo,
    Redo,
    ResetAll,
    ResetColour,
    ResetFrame,
    Rotate(bool),
    HalfTurn,
    MirrorH,
    MirrorV,
    Angle(f32),
    AngleBy(f32),
    Crop(Option<Selection>),
    Aspect(Aspect),
    Portrait,
    CropFull,
    Unit(Unit),
    Lock(bool),
    Input(SizeField, String),
    Submit,
    Percent(f32),
    SizeReset,
}

impl From<EditMsg> for Message {
    fn from(m: EditMsg) -> Self {
        Message::Edit(m)
    }
}

/// The crop of a selection on the viewer.
pub fn crop_message(s: Option<Selection>) -> Message {
    Message::Edit(EditMsg::Crop(s))
}

// ------------------------------------------------------------- the app

impl App {
    /// The editor, when it is open on the page shown.
    pub fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref().filter(|e| {
            self.tab == Tab::Result && self.redone.is_none() && self.current == Some(e.index)
        })
    }

    fn edited_page(&self) -> Option<&Page> {
        self.editor().and_then(|e| self.pages.get(e.index))
    }

    fn edited_page_mut(&mut self) -> Option<&mut Page> {
        let i = self.editor()?.index;
        self.pages.get_mut(i)
    }

    /// Whether the crop is shown (the whole frame, with the crop on it).
    pub fn cropping(&self) -> bool {
        !self.before && self.editor().is_some_and(|e| e.section == Section::Frame)
    }

    /// Opens the editor on page `i`.
    pub fn open_editor(&mut self, i: usize) {
        if self.redone.is_some() || i >= self.pages.len() {
            return;
        }
        let section = match &self.editor {
            Some(e) if e.index == i => {
                // Open already, maybe while another page was shown.
                self.current = Some(i);
                self.tab = Tab::Result;
                return;
            }
            Some(e) => e.section,
            None => self.edit_section,
        };
        self.close_editor(true);
        let page = &self.pages[i];
        let (w, h) = page
            .transform
            .oriented_size(page.raw.full.width, page.raw.full.height);
        self.editor = Some(Editor::new(i, page.edit(), section, h > w));
        self.current = Some(i);
        self.tab = Tab::Result;
        self.before = false;
        self.eyedropper = false;
        self.result_view = View::default();
        self.dirty_page = true;
        self.note = format!(
            "Modifica di {}: Invio applica, Esc scarta, Ctrl+Z annulla",
            self.page_name(i)
        );
    }

    /// Closes the editor, keeping the changes or putting the page back.
    pub fn close_editor(&mut self, keep: bool) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        self.edit_section = editor.section;
        self.before = false;
        let shared = *self.settings();
        let name = self.page_name(editor.index);
        if let Some(page) = self.pages.get_mut(editor.index) {
            if !keep {
                page.set_edit(editor.start);
            }
            let changed = page.edit() != editor.start;
            page.redraw(false, &shared);
            // A render still running may be of the editor's view.
            self.dirty_page = true;
            self.note = match (keep, changed) {
                (true, true) => format!("Modifiche di {name} applicate"),
                (true, false) => String::new(),
                (false, _) => format!("Modifiche di {name} scartate"),
            };
        }
        self.result_view = View::default();
    }

    /// "Pagina 2" or "Foto".
    pub fn page_name(&self, i: usize) -> String {
        crate::item_name(self.kind, i, self.pages.len())
    }

    /// "Pagina 2" or "Foto", at the start of a sentence.
    pub fn page_title(&self, i: usize) -> String {
        let name = self.page_name(i);
        let mut chars = name.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    }

    /// Keeps the page as it is for the editor's Ctrl+Z.
    pub fn edit_remember(&mut self, key: Option<Key>) {
        let Some(current) = self.edited_page().map(Page::edit) else {
            return;
        };
        if let Some(e) = self.editor.as_mut() {
            e.remember(current, key);
        }
    }

    /// A change of the geometry, live.
    fn edit_transform(&mut self, key: Option<Key>, f: impl FnOnce(&mut Transform, usize, usize)) {
        self.edit_remember(key);
        if let Some(page) = self.edited_page_mut() {
            let (w, h) = (page.raw.full.width, page.raw.full.height);
            f(&mut page.transform, w, h);
            self.dirty_page = true;
        }
    }

    pub fn on_edit(&mut self, message: EditMsg) {
        if !matches!(message, EditMsg::Open(_)) && self.editor().is_none() {
            return;
        }
        match message {
            EditMsg::Open(i) => self.open_editor(i),
            EditMsg::Apply => self.close_editor(true),
            EditMsg::Discard => self.close_editor(false),
            EditMsg::Section(section) => {
                if let Some(e) = self.editor.as_mut() {
                    if e.section != section {
                        e.section = section;
                        e.typing = None;
                        self.dirty_page = true;
                        self.result_view = View::default();
                    }
                }
            }
            EditMsg::Undo | EditMsg::Redo => {
                let Some(current) = self.edited_page().map(Page::edit) else {
                    return;
                };
                let undo = matches!(message, EditMsg::Undo);
                let Some(e) = self.editor.as_mut() else {
                    return;
                };
                e.typing = None;
                e.break_step();
                let (from, to) = if undo {
                    (&mut e.undo, &mut e.redo)
                } else {
                    (&mut e.redo, &mut e.undo)
                };
                let Some(edit) = from.pop() else {
                    self.note = if undo {
                        "Niente da annullare".into()
                    } else {
                        "Niente da ripristinare".into()
                    };
                    return;
                };
                to.push(current);
                if let Some(page) = self.edited_page_mut() {
                    page.set_edit(edit);
                }
                self.dirty_page = true;
                self.note = if undo { "Annullato" } else { "Ripristinato" }.into();
            }
            EditMsg::ResetAll => {
                self.edit_remember(None);
                if let Some(page) = self.edited_page_mut() {
                    page.set_edit(Edit {
                        look: None,
                        colour: page.scanned_colour,
                        transform: Transform::default(),
                    });
                }
                self.dirty_page = true;
                self.note = "Tornata com'era stata scansionata (Ctrl+Z per annullare)".into();
            }
            EditMsg::ResetColour => {
                self.edit_remember(None);
                if let Some(page) = self.edited_page_mut() {
                    page.own = None;
                    page.settings.colour = page.scanned_colour;
                }
                self.dirty_page = true;
            }
            EditMsg::ResetFrame => self.edit_transform(None, |t, _, _| {
                *t = Transform {
                    scale: t.scale,
                    dpi: t.dpi,
                    ..Transform::default()
                }
            }),
            EditMsg::Rotate(clockwise) => self.edit_transform(None, |t, _, _| t.rotate(clockwise)),
            EditMsg::HalfTurn => self.edit_transform(None, |t, _, _| {
                t.rotate(true);
                t.rotate(true);
            }),
            EditMsg::MirrorH => self.edit_transform(None, |t, _, _| t.mirror_horizontal()),
            EditMsg::MirrorV => self.edit_transform(None, |t, _, _| t.mirror_vertical()),
            EditMsg::Angle(a) => self.edit_transform(Some(Key::Angle), |t, _, _| {
                t.angle = (a * 10.0).round() / 10.0;
            }),
            EditMsg::AngleBy(d) => self.edit_transform(Some(Key::Angle), |t, _, _| {
                t.angle = ((t.angle + d) * 10.0)
                    .round()
                    .clamp(-MAX_ANGLE * 10.0, MAX_ANGLE * 10.0)
                    / 10.0;
            }),
            EditMsg::Crop(None) => {}
            EditMsg::Crop(Some(sel)) => {
                let Some(e) = self.editor() else { return };
                let (aspect, portrait) = (e.aspect, e.portrait);
                self.edit_transform(Some(Key::Crop), |t, w, h| {
                    let (fw, fh) = t.oriented_size(w, h);
                    let (fw, fh) = (fw as f32, fh as f32);
                    if sel.width < 2.0 || sel.height < 2.0 {
                        return;
                    }
                    let c = t.effective_crop(w, h);
                    let prev = [c.x * fw, c.y * fh, c.width * fw, c.height * fh];
                    let mut r = [sel.x, sel.y, sel.width, sel.height];
                    if let Some(ratio) = aspect.ratio(portrait, fw, fh) {
                        r = constrain(r, prev, ratio, fw, fh);
                    }
                    t.crop = Some(
                        Crop {
                            x: r[0] / fw,
                            y: r[1] / fh,
                            width: r[2] / fw,
                            height: r[3] / fh,
                        }
                        .clamped(),
                    );
                });
            }
            EditMsg::Aspect(a) => {
                if let Some(e) = self.editor.as_mut() {
                    e.aspect = a;
                }
                self.fit_aspect();
            }
            EditMsg::Portrait => {
                if let Some(e) = self.editor.as_mut() {
                    e.portrait = !e.portrait;
                }
                self.fit_aspect();
            }
            EditMsg::CropFull => {
                if let Some(e) = self.editor.as_mut() {
                    e.aspect = Aspect::Free;
                }
                self.edit_transform(None, |t, _, _| t.crop = None);
            }
            EditMsg::Unit(unit) => {
                if let Some(e) = self.editor.as_mut() {
                    e.unit = unit;
                    e.typing = None;
                }
            }
            EditMsg::Lock(on) => {
                if let Some(e) = self.editor.as_mut() {
                    e.lock = on;
                }
                if on {
                    // Back to the proportions of the crop, from the width.
                    self.edit_transform(None, |t, _, _| {
                        if let Some([x, _]) = t.scale {
                            t.scale = Some([x, x]);
                        }
                    });
                }
            }
            EditMsg::Input(field, typed) => self.size_input(field, typed),
            EditMsg::Submit => {
                if let Some(e) = self.editor.as_mut() {
                    e.typing = None;
                    e.break_step();
                }
            }
            EditMsg::Percent(p) => self.edit_transform(None, |t, _, _| {
                let k = p / 100.0;
                t.scale = (k != 1.0).then_some([k, k]);
            }),
            EditMsg::SizeReset => self.edit_transform(None, |t, _, _| {
                t.scale = None;
                t.dpi = None;
            }),
        }
    }

    /// The crop held to the proportions chosen, as large as it can be
    /// around its centre.
    fn fit_aspect(&mut self) {
        let Some(e) = self.editor() else { return };
        let (aspect, portrait) = (e.aspect, e.portrait);
        self.edit_transform(None, |t, w, h| {
            let (fw, fh) = t.oriented_size(w, h);
            let (fw, fh) = (fw as f32, fh as f32);
            let Some(r) = aspect.ratio(portrait, fw, fh) else {
                return;
            };
            let c = t.effective_crop(w, h);
            let centre = ((c.x + c.width / 2.0) * fw, (c.y + c.height / 2.0) * fh);
            // Inside the part without empty corners.
            let a = Transform { crop: None, ..*t }.effective_crop(w, h);
            let area = [a.x * fw, a.y * fh, a.width * fw, a.height * fh];
            let [x, y, cw, ch] = fit_ratio(area, centre, r);
            t.crop = Some(Crop {
                x: x / fw,
                y: y / fh,
                width: cw / fw,
                height: ch / fh,
            });
        });
    }

    /// A number of the size typed: applied as soon as it is one.
    fn size_input(&mut self, field: SizeField, typed: String) {
        let Some(e) = self.editor.as_mut() else {
            return;
        };
        e.typing = Some((field, typed.clone()));
        let (unit, lock) = (e.unit, e.lock);
        let Some(v) = parse(&typed).filter(|v| *v > 0.0) else {
            return;
        };
        let Some(page) = self.edited_page() else {
            return;
        };
        let full = &page.raw.full;
        let t = page.transform;
        let (_, _, cw, ch) = t.crop_pixels(full.width, full.height);
        let (ow, oh) = t.output_size(full.width, full.height);
        let dpi = f32::from(t.output_dpi(full.width, full.height, full.dpi));
        let (cw, ch, ow, oh) = (cw as f32, ch as f32, ow as f32, oh as f32);
        let physical = matches!(unit, Unit::Cm | Unit::Inches);
        let mut new = t;
        match field {
            SizeField::Width => {
                let px = to_pixels(v, unit, cw, dpi).clamp(1.0, 60000.0);
                let sx = px / cw;
                new.scale = Some([sx, if lock { sx } else { oh / ch }]);
            }
            SizeField::Height => {
                let px = to_pixels(v, unit, ch, dpi).clamp(1.0, 60000.0);
                let sy = px / ch;
                new.scale = Some([if lock { sy } else { ow / cw }, sy]);
            }
            SizeField::Dpi => {
                let d = v.round().clamp(10.0, 9600.0);
                if physical {
                    // Same size on paper: more or fewer pixels.
                    let k = d / dpi;
                    new.scale = Some([ow * k / cw, oh * k / ch]);
                }
                new.dpi = Some(d as u16);
            }
        }
        if physical && new.dpi.is_none() {
            new.dpi = Some(dpi.round() as u16);
        }
        if new.scale == Some([1.0, 1.0]) {
            new.scale = None;
        }
        self.edit_transform(Some(Key::Size(field)), |t, _, _| *t = new);
    }

    // ---------------------------------------------------------- views

    /// The inspector while editing.
    pub fn editor_pane(&self) -> Element<'_, Message> {
        let Some(e) = self.editor() else {
            return Space::new().into();
        };
        let section_tab = |label: &'static str, s: Section| {
            let on = e.section == s;
            button(text(label).size(13).center().width(Length::Fill))
                .width(Length::Fill)
                .padding([6, 6])
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
                    }
                    style
                })
                .on_press(EditMsg::Section(s).into())
        };
        let sections = container(
            row![
                section_tab("Colore", Section::Colour),
                section_tab("Ritaglia", Section::Frame),
                section_tab("Dimensioni", Section::Size),
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
        let title = row![
            text(format!(
                "MODIFICA · {}",
                self.page_name(e.index).to_uppercase()
            ))
            .size(11)
            .width(Length::Fill),
            button(text("Ripristina originale").size(12))
                .style(button::text)
                .padding([2, 6])
                .on_press(EditMsg::ResetAll.into()),
        ]
        .align_y(Alignment::Center);
        let body = match e.section {
            Section::Colour => self.colour_section(),
            Section::Frame => self.frame_section(e),
            Section::Size => self.size_section(e),
        };
        column![title, sections, body].spacing(12).into()
    }

    fn colour_section(&self) -> Element<'_, Message> {
        let s = self.active();
        let own = self.edited_page().is_some_and(|p| p.own.is_some());
        let top = column![
            labelled(
                "Modo",
                pick_list(ALL_COLOURS, Some(s.colour), Message::Colour)
                    .width(Length::Fill)
                    .text_size(13)
                    .into(),
            ),
            row![
                text(if own {
                    "Regolazioni solo di questa scansione."
                } else {
                    "Segue le regolazioni comuni finché non ne cambi una."
                })
                .size(11)
                .width(Length::Fill),
                button(text("Ripristina colori").size(12))
                    .style(button::text)
                    .padding([2, 6])
                    .on_press(EditMsg::ResetColour.into()),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(6);
        self.adjust_controls(top.into())
    }

    fn frame_section(&self, e: &Editor) -> Element<'_, Message> {
        let Some(page) = self.edited_page() else {
            return Space::new().into();
        };
        let t = page.transform;
        let full = &page.raw.full;
        let tool = |label: &'static str, m: EditMsg| {
            button(text(label).size(12).center().width(Length::Fill))
                .width(Length::Fill)
                .padding([5, 6])
                .style(button::secondary)
                .on_press(m.into())
        };
        let nudge = |label: &'static str, d: f32| {
            button(text(label).size(11).center().width(Length::Fill))
                .width(Length::Fill)
                .padding([3, 4])
                .style(button::secondary)
                .on_press(EditMsg::AngleBy(d).into())
        };
        let (fw, fh) = t.oriented_size(full.width, full.height);
        let (_, _, cw, ch) = t.crop_pixels(full.width, full.height);
        let mm = |px: usize| px as f32 * 25.4 / f32::from(full.dpi.max(1));
        let crop_info = format!(
            "Ritaglio {cw} × {ch} pixel · {} × {} mm",
            number(mm(cw), 0),
            number(mm(ch), 0)
        );
        let mut crop = column![labelled(
            "Proporzioni",
            pick_list(ASPECTS, Some(e.aspect), |a| EditMsg::Aspect(a).into())
                .width(Length::Fill)
                .text_size(13)
                .into(),
        ),]
        .spacing(6);
        if e.aspect.turns() {
            crop = crop.push(
                button(
                    text(if e.portrait {
                        "Verticale · passa a orizzontale"
                    } else {
                        "Orizzontale · passa a verticale"
                    })
                    .size(12),
                )
                .style(button::secondary)
                .padding([4, 8])
                .on_press(EditMsg::Portrait.into()),
            );
        }
        crop = crop
            .push(
                row![
                    text(crop_info).size(11).width(Length::Fill),
                    button(text("Tutta l'immagine").size(12))
                        .style(button::text)
                        .padding([2, 6])
                        .on_press_maybe(t.crop.is_some().then_some(EditMsg::CropFull.into())),
                ]
                .align_y(Alignment::Center),
            )
            .push(
                text(
                    "Trascina sull'immagine per un nuovo ritaglio; sposta i bordi con le \
                     maniglie o tutto il riquadro da dentro.",
                )
                .size(11),
            );
        let angle = t.angle;
        column![
            heading("Ruota"),
            row![
                tool("90° a sinistra", EditMsg::Rotate(false)),
                tool("90° a destra", EditMsg::Rotate(true)),
                tool("180°", EditMsg::HalfTurn),
            ]
            .spacing(6),
            row![
                tool("Specchia", EditMsg::MirrorH),
                tool("Capovolgi", EditMsg::MirrorV),
            ]
            .spacing(6),
            heading("Raddrizza"),
            row![
                text("Angolo").size(12).width(Length::Fill),
                text(format!("{}°", number(angle, 1))).size(12),
            ],
            mouse_area(
                slider(-MAX_ANGLE..=MAX_ANGLE, angle, |a| EditMsg::Angle(a).into()).step(0.1f32)
            )
            .on_double_click(EditMsg::Angle(0.0).into()),
            row![
                nudge("−1°", -1.0),
                nudge("−0,1°", -0.1),
                nudge("+0,1°", 0.1),
                nudge("+1°", 1.0),
            ]
            .spacing(4),
            text(
                "Allinea un bordo o una riga di testo alla griglia. Finché non scegli un \
                 ritaglio, gli angoli vuoti vengono tagliati da soli.",
            )
            .size(11),
            heading("Ritaglio"),
            crop,
            text(format!("Immagine girata: {fw} × {fh} pixel")).size(11),
            button(text("Ripristina rotazione e ritaglio").size(12))
                .style(button::text)
                .padding([2, 0])
                .on_press(EditMsg::ResetFrame.into()),
        ]
        .spacing(8)
        .into()
    }

    fn size_section(&self, e: &Editor) -> Element<'_, Message> {
        let Some(page) = self.edited_page() else {
            return Space::new().into();
        };
        let t = page.transform;
        let full = &page.raw.full;
        let (_, _, cw, ch) = t.crop_pixels(full.width, full.height);
        let (ow, oh) = t.output_size(full.width, full.height);
        let dpi = t.output_dpi(full.width, full.height, full.dpi);
        let d = f32::from(dpi);
        let decimals = match e.unit {
            Unit::Pixels => 0,
            Unit::Percent => 1,
            Unit::Cm | Unit::Inches => 2,
        };
        let shown = |field: SizeField, value: String| match &e.typing {
            Some((f, typed)) if *f == field => typed.clone(),
            _ => value,
        };
        let width = shown(
            SizeField::Width,
            number(from_pixels(ow as f32, e.unit, cw as f32, d), decimals),
        );
        let height = shown(
            SizeField::Height,
            number(from_pixels(oh as f32, e.unit, ch as f32, d), decimals),
        );
        let resolution = shown(SizeField::Dpi, dpi.to_string());
        let input = |field: SizeField, value: String| {
            text_input("", &value)
                .on_input(move |s| EditMsg::Input(field, s).into())
                .on_submit(EditMsg::Submit.into())
                .size(13)
                .padding([4, 8])
                .width(Length::Fill)
        };
        let percent = |p: f32| {
            button(
                text(format!("{} %", number(p, 0)))
                    .size(12)
                    .center()
                    .width(Length::Fill),
            )
            .width(Length::Fill)
            .padding([4, 4])
            .style(button::secondary)
            .on_press(EditMsg::Percent(p).into())
        };
        let cm = |px: usize| px as f32 / d * 2.54;
        let upscaled = ow > cw || oh > ch;
        let mut pane = column![
            text(format!(
                "Dopo il ritaglio: {cw} × {ch} pixel a {} dpi",
                full.dpi
            ))
            .size(11),
            labelled(
                "Unità",
                pick_list(UNITS, Some(e.unit), |u| EditMsg::Unit(u).into())
                    .width(Length::Fill)
                    .text_size(13)
                    .into(),
            ),
            row![
                column![
                    text(format!("Larghezza ({})", e.unit.suffix())).size(12),
                    input(SizeField::Width, width),
                ]
                .spacing(2)
                .width(Length::Fill),
                column![
                    text(format!("Altezza ({})", e.unit.suffix())).size(12),
                    input(SizeField::Height, height),
                ]
                .spacing(2)
                .width(Length::Fill),
            ]
            .spacing(6)
            .align_y(Alignment::End),
            checkbox(e.lock)
                .label("Mantieni le proporzioni")
                .on_toggle(|on| EditMsg::Lock(on).into())
                .text_size(13),
            labelled(
                "Risoluzione",
                row![input(SizeField::Dpi, resolution), text("dpi").size(12)]
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .into()
            ),
            text(match e.unit {
                Unit::Cm | Unit::Inches => {
                    "In centimetri e pollici la misura sulla carta resta: cambiando i dpi \
                     cambiano i pixel."
                }
                _ => {
                    "Con pixel e percentuale la misura sulla carta resta, cambiano i dpi \
                     scritti nel file; scrivili qui per sceglierli tu."
                }
            })
            .size(11),
            row![percent(25.0), percent(50.0), percent(75.0), percent(200.0)].spacing(4),
            container(
                column![
                    text(format!("Risultato: {ow} × {oh} pixel")).size(14),
                    text(format!(
                        "{} × {} cm a {dpi} dpi · {} megapixel",
                        number(cm(ow), 1),
                        number(cm(oh), 1),
                        number((ow * oh) as f32 / 1e6, 1)
                    ))
                    .size(12),
                    text("Ricampionamento Lanczos (3 lobi)").size(11),
                ]
                .spacing(2),
            )
            .padding(8)
            .width(Length::Fill)
            .style(container::rounded_box),
        ]
        .spacing(8);
        if upscaled {
            pane = pane.push(
                text("Ingrandire non aggiunge dettagli: per più definizione rifai la scansione a più dpi.")
                    .size(11),
            );
        }
        pane.push(
            button(text("Dimensione originale").size(12))
                .style(button::text)
                .padding([2, 0])
                .on_press_maybe(
                    (t.scale.is_some() || t.dpi.is_some()).then_some(EditMsg::SizeReset.into()),
                ),
        )
        .into()
    }

    /// The bar on the viewer while editing.
    pub fn editor_bar(&self) -> Option<Element<'_, Message>> {
        let e = self.editor()?;
        let small = |label: &'static str, m: Option<EditMsg>| {
            button(text(label).size(12))
                .padding([5, 9])
                .style(button::text)
                .on_press_maybe(m.map(Into::into))
        };
        Some(crate::card(
            row![
                text(format!("Modifica di {}", self.page_name(e.index))).size(13),
                small("Annulla", (!e.undo.is_empty()).then_some(EditMsg::Undo)),
                small("Ripeti", (!e.redo.is_empty()).then_some(EditMsg::Redo)),
                button(text("Scarta").size(13))
                    .style(button::secondary)
                    .on_press(EditMsg::Discard.into()),
                button(text("Applica").size(13)).on_press(EditMsg::Apply.into()),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ))
    }

    /// The crop on the viewer, in pixels of the turned frame.
    pub fn crop_selection(&self) -> Option<Selection> {
        if !self.cropping() {
            return None;
        }
        let page = self.edited_page()?;
        let full = &page.raw.full;
        let t = page.transform;
        let (fw, fh) = t.oriented_size(full.width, full.height);
        let c = t.effective_crop(full.width, full.height);
        Some(Selection {
            x: c.x * fw as f32,
            y: c.y * fh as f32,
            width: c.width * fw as f32,
            height: c.height * fh as f32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_go_and_come_back() {
        for unit in UNITS {
            let px = to_pixels(12.5, unit, 2000.0, 300.0);
            assert!(
                (from_pixels(px, unit, 2000.0, 300.0) - 12.5).abs() < 1e-3,
                "{unit}"
            );
        }
        assert_eq!(to_pixels(50.0, Unit::Percent, 2000.0, 300.0), 1000.0);
        assert!((to_pixels(2.54, Unit::Cm, 0.0, 300.0) - 300.0).abs() < 1e-3);
        assert_eq!(to_pixels(2.0, Unit::Inches, 0.0, 300.0), 600.0);
    }

    #[test]
    fn numbers_the_italian_way() {
        assert_eq!(number(21.0, 2), "21");
        assert_eq!(number(29.7, 2), "29,7");
        assert_eq!(number(-1.5, 1), "-1,5");
        assert_eq!(number(300.0, 0), "300");
        assert_eq!(parse("12,5"), Some(12.5));
        assert_eq!(parse(" 50 % "), Some(50.0));
        assert_eq!(parse("abc"), None);
    }

    #[test]
    fn crops_keep_their_proportions() {
        // The right edge dragged: the height follows, the top-left stays.
        let r = constrain(
            [10.0, 10.0, 60.0, 30.0],
            [10.0, 10.0, 40.0, 30.0],
            1.0,
            100.0,
            100.0,
        );
        assert_eq!(r, [10.0, 10.0, 60.0, 60.0]);
        // The left edge dragged: the right edge stays.
        let r = constrain(
            [20.0, 10.0, 30.0, 40.0],
            [10.0, 10.0, 40.0, 40.0],
            1.0,
            100.0,
            100.0,
        );
        assert_eq!(r, [20.0, 10.0, 30.0, 30.0]);
        // Too big for the frame: shrunk to fit.
        let r = constrain(
            [50.0, 50.0, 80.0, 10.0],
            [50.0, 50.0, 10.0, 10.0],
            2.0,
            100.0,
            100.0,
        );
        assert!(r[0] + r[2] <= 100.0 + 1e-3 && r[1] + r[3] <= 100.0 + 1e-3);
        assert!((r[2] / r[3] - 2.0).abs() < 1e-3);
        // The largest square around a centre.
        let r = fit_ratio([0.0, 0.0, 100.0, 50.0], (90.0, 25.0), 1.0);
        assert_eq!(r, [50.0, 0.0, 50.0, 50.0]);
    }

    #[test]
    fn aspects() {
        assert_eq!(Aspect::Free.ratio(false, 3.0, 2.0), None);
        assert_eq!(Aspect::Original.ratio(true, 3.0, 2.0), Some(1.5));
        assert_eq!(Aspect::R3x2.ratio(true, 3.0, 2.0), Some(1.0 / 1.5));
    }
}
