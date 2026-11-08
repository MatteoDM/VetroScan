// SPDX-License-Identifier: GPL-2.0-or-later
//! Moving things precisely from the keyboard, the same keys wherever
//! something on the screen is moved with the mouse:
//!
//! - arrows: by 1 mm; **Shift**: 10 mm; **Alt**: 0.1 mm, or one pixel of
//!   the scan when that is larger;
//! - **Ctrl** (⌘ on macOS) + arrows: the size instead of the position (the
//!   right and bottom edges), with the same steps;
//! - **Home**: back to the middle.
//!
//! What moves is what is active in the pane shown: the point of the curve
//! clicked last, the crop in the editor, the page on its sheet, the area
//! to scan on the preview; with none of them, the zoomed view. At an edge
//! it stops. The keys never reach here while a text field has them (the
//! keyboard subscription only gets what no widget took).

use iced::keyboard::{key::Named, Key, Modifiers};
use iced::Point;

use crate::editor::EditMsg;
use crate::viewer::{Selection, View};
use crate::{App, Message, MmField, Tab, PREVIEW_DPI};

/// How far one press goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// 1 mm.
    One,
    /// 10 mm (Shift).
    Ten,
    /// 0.1 mm, or a pixel (Alt).
    Tenth,
}

impl Step {
    /// Millimetres, `pixel` (the size of a pixel, mm) at least for the
    /// finest.
    pub fn mm(self, pixel: f32) -> f32 {
        match self {
            Self::One => 1.0,
            Self::Ten => 10.0,
            Self::Tenth => 0.1f32.max(pixel),
        }
    }

    /// Levels of the curve (0-255): the finest is one level too.
    fn levels(self) -> f32 {
        match self {
            Self::Ten => 10.0,
            Self::One | Self::Tenth => 1.0,
        }
    }

    /// Screen pixels the zoomed view goes.
    fn pan(self) -> f32 {
        match self {
            Self::One => 50.0,
            Self::Ten => 250.0,
            Self::Tenth => 5.0,
        }
    }
}

/// A move asked from the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// An arrow: across and down (-1, 0 or 1), how far, and whether the
    /// size changes (the right and bottom edges) instead of the position.
    By {
        dx: i8,
        dy: i8,
        step: Step,
        size: bool,
    },
    /// Home: back to the middle.
    Centre,
}

/// The move a key asks for, if it is one.
pub fn read(key: &Key, modifiers: Modifiers) -> Option<Move> {
    let (dx, dy) = match key.as_ref() {
        Key::Named(Named::ArrowLeft) => (-1, 0),
        Key::Named(Named::ArrowRight) => (1, 0),
        Key::Named(Named::ArrowUp) => (0, -1),
        Key::Named(Named::ArrowDown) => (0, 1),
        Key::Named(Named::Home) => return Some(Move::Centre),
        _ => return None,
    };
    let step = if modifiers.shift() {
        Step::Ten
    } else if modifiers.alt() {
        Step::Tenth
    } else {
        Step::One
    };
    Some(Move::By {
        dx,
        dy,
        step,
        size: modifiers.command(),
    })
}

/// A rectangle moved or resized inside `w` × `h`, `d` pixels a press: it
/// stops at the edges and never gets smaller than `least`.
fn moved(s: Selection, m: Move, d: f32, (w, h): (f32, f32), least: f32) -> Selection {
    let (l, t, r, b) = (s.x, s.y, s.x + s.width, s.y + s.height);
    match m {
        Move::Centre => Selection {
            x: ((w - s.width) / 2.0).max(0.0),
            y: ((h - s.height) / 2.0).max(0.0),
            ..s
        },
        Move::By {
            dx,
            dy,
            size: false,
            ..
        } => {
            let mx = (f32::from(dx) * d).clamp(-l, (w - r).max(-l));
            let my = (f32::from(dy) * d).clamp(-t, (h - b).max(-t));
            Selection {
                x: l + mx,
                y: t + my,
                ..s
            }
        }
        Move::By {
            dx, dy, size: true, ..
        } => {
            let r = (r + f32::from(dx) * d).clamp((l + least).min(w), w.max(l + least));
            let b = (b + f32::from(dy) * d).clamp((t + least).min(h), h.max(t + least));
            Selection {
                width: r - l,
                height: b - t,
                ..s
            }
        }
    }
}

/// Millimetres as a field shows them: whole, or to a tenth.
pub fn format_mm(v: f64) -> String {
    let tenths = (v * 10.0).round();
    if tenths % 10.0 == 0.0 {
        format!("{:.0}", tenths / 10.0)
    } else {
        format!("{:.1}", tenths / 10.0)
    }
}

impl App {
    /// A millimetre field typed: applied as soon as it is a number, kept
    /// on the glass or inside the margin.
    pub fn mm_input(&mut self, field: MmField, typed: String) {
        self.mm_draft = Some((field, typed.clone()));
        let Some(v) = crate::editor::parse(&typed).map(f64::from) else {
            return;
        };
        match field {
            MmField::Area(i) => {
                let glass = self.glass();
                let per_mm = f64::from(PREVIEW_DPI) / 25.4;
                let (gw, gh) = (
                    f64::from(glass.width) / per_mm,
                    f64::from(glass.height) / per_mm,
                );
                let (x, y, w, h) = self.area_mm();
                let mut a = [x, y, w, h];
                a[i] = v;
                let x = a[0].clamp(0.0, gw - 1.0);
                let y = a[1].clamp(0.0, gh - 1.0);
                let w = a[2].clamp(1.0, gw - x);
                let h = a[3].clamp(1.0, gh - y);
                let px = |mm: f64| (mm * per_mm) as f32;
                self.selection = Some(Selection {
                    x: px(x),
                    y: px(y),
                    width: px(w),
                    height: px(h),
                });
            }
            MmField::Offset(i) => {
                let Some(page) = self.current.and_then(|c| self.pages.get(c)) else {
                    return;
                };
                let (w, h) = page.inches();
                let layout = self.settings().layout(page.layout, page.focus);
                let placed = layout.place(w, h);
                let mut offset = placed.offset_mm();
                offset[i] = v;
                let focus = placed.focus_at(offset, layout.focus).map(|f| f as f32);
                if let Some(page) = self.current.and_then(|c| self.pages.get_mut(c)) {
                    page.focus = focus;
                }
            }
        }
    }

    /// What a move from the keyboard does now, if anything can move.
    pub fn on_move(&mut self, m: Move) -> Option<Message> {
        if let Some(message) = self.move_curve_point(m) {
            return Some(message);
        }
        if self.editor().is_some() {
            return if self.cropping() {
                self.move_crop(m)
            } else {
                self.pan(m)
            };
        }
        if self.print_preview() {
            return self.move_page(m);
        }
        if self.tab == Tab::Preview && self.selection.is_some() {
            return self.move_area(m);
        }
        self.pan(m)
    }

    /// Which curve the editor shows: the channel chosen, in colour.
    pub fn curve_index(&self) -> usize {
        if self.active().colour == crate::Colour::Rgb {
            self.curve_channel.index()
        } else {
            0
        }
    }

    /// Whether the curve editor is on the screen.
    pub fn curve_shown(&self) -> bool {
        let panel = match self.editor() {
            Some(e) => e.section == crate::editor::Section::Colour,
            None => self.pane == crate::Pane::Adjust,
        };
        panel
            && self.open.contains(&crate::Group::Curve)
            && self.active().colour != crate::Colour::Lineart
    }

    /// The curve shown, as its points, and the one clicked last.
    fn curve_point(&self) -> Option<(Vec<[f32; 2]>, usize)> {
        if !self.curve_shown() {
            return None;
        }
        let i = self.curve_selected?;
        let points = self.active().adjust.curve[self.curve_index()]
            .points()
            .to_vec();
        (i < points.len()).then_some((points, i))
    }

    /// The point of the curve clicked last, moved by one level a press.
    fn move_curve_point(&self, m: Move) -> Option<Message> {
        let (mut points, i) = self.curve_point()?;
        let Move::By { dx, dy, step, .. } = m else {
            // Nothing is "the middle" of a curve: Home does nothing (and
            // moves nothing else).
            return Some(Message::CurvePoint(Some(i)));
        };
        let d = step.levels() / 255.0;
        let n = points.len();
        let low = if i == 0 {
            0.0
        } else {
            points[i - 1][0] + crate::curve::GAP
        };
        let high = if i + 1 == n {
            1.0
        } else {
            points[i + 1][0] - crate::curve::GAP
        };
        let [x, y] = points[i];
        points[i] = [
            (x + f32::from(dx) * d).clamp(low, high.max(low)),
            (y - f32::from(dy) * d).clamp(0.0, 1.0),
        ];
        Some(Message::CurveSet(points, Some(i)))
    }

    /// Tab and Delete on the curve: the next point, or the point away (not
    /// the two ends).
    pub fn curve_key(&self, next: Option<bool>) -> Option<Message> {
        let (mut points, i) = self.curve_point()?;
        let n = points.len();
        Some(match next {
            Some(forward) => Message::CurvePoint(Some(if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            })),
            None if i > 0 && i + 1 < n => {
                points.remove(i);
                Message::CurveSet(points, None)
            }
            None => return None,
        })
    }

    /// The crop in the editor.
    fn move_crop(&self, m: Move) -> Option<Message> {
        let page = self.edited_page()?;
        let full = &page.raw.full;
        let (fw, fh) = page.transform.oriented_size(full.width, full.height);
        let per_mm = f32::from(full.dpi.max(1)) / 25.4;
        let d = match m {
            Move::By { step, .. } => step.mm(1.0 / per_mm) * per_mm,
            Move::Centre => 0.0,
        };
        let s = self.crop_selection()?;
        let new = moved(s, m, d, (fw as f32, fh as f32), 2.0);
        (new != s).then(|| EditMsg::Crop(Some(new)).into())
    }

    /// The page on its sheet, in the PDF.
    fn move_page(&self, m: Move) -> Option<Message> {
        let page = self.pages.get(self.current?)?;
        let (w, h) = page.inches();
        let layout = self.settings().layout(page.layout, page.focus);
        let placed = layout.place(w, h);
        let focus = match m {
            Move::Centre => [0.5, 0.5],
            Move::By { size: true, .. } => return None,
            Move::By { dx, dy, step, .. } => {
                // One pixel of the scan as it lies on the sheet.
                let full = &page.raw.full;
                let (pixels, _) = page.transform.output_size(full.width, full.height);
                let pixel = (placed.image[2] * 25.4 / 72.0 / pixels.max(1) as f64) as f32;
                let d = f64::from(step.mm(pixel));
                let [l, t] = placed.offset_mm();
                placed
                    .focus_at([l + f64::from(dx) * d, t + f64::from(dy) * d], layout.focus)
                    .map(|f| f as f32)
            }
        };
        (focus != page.focus).then_some(Message::PageFocus(focus))
    }

    /// The area to scan, on the preview.
    fn move_area(&self, m: Move) -> Option<Message> {
        let s = self.selection?;
        let glass = self.glass();
        let per_mm = f32::from(PREVIEW_DPI) / 25.4;
        let d = match m {
            Move::By { step, .. } => step.mm(1.0 / per_mm) * per_mm,
            Move::Centre => 0.0,
        };
        let new = moved(s, m, d, (glass.width, glass.height), 4.0);
        (new != s).then_some(Message::Select(Some(new)))
    }

    /// The zoomed view, when nothing else moves: the arrows show more of
    /// the image that way, Home its middle.
    fn pan(&mut self, m: Move) -> Option<Message> {
        let size = match self.tab {
            Tab::Preview => self
                .preview
                .as_ref()
                .map_or(self.glass(), |(_, _, p)| p.size),
            Tab::Result => self.shown()?.picture.size,
        };
        let view = *self.current_view();
        let scale = view.scale?;
        let centre = view
            .center
            .unwrap_or(Point::new(size.width / 2.0, size.height / 2.0));
        let new = match m {
            Move::Centre => View {
                center: None,
                ..view
            },
            Move::By { size: true, .. } => return None,
            Move::By { dx, dy, step, .. } => {
                let d = step.pan() / scale;
                View {
                    center: Some(Point::new(
                        (centre.x + f32::from(dx) * d).clamp(0.0, size.width),
                        (centre.y + f32::from(dy) * d).clamp(0.0, size.height),
                    )),
                    ..view
                }
            }
        };
        (new != view).then_some(Message::View(new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, modifiers: Modifiers) -> Option<Move> {
        read(&key, modifiers)
    }

    #[test]
    fn the_keys_and_their_steps() {
        let right = Key::Named(Named::ArrowRight);
        assert_eq!(
            press(right.clone(), Modifiers::empty()),
            Some(Move::By {
                dx: 1,
                dy: 0,
                step: Step::One,
                size: false
            })
        );
        assert_eq!(
            press(Key::Named(Named::ArrowUp), Modifiers::SHIFT),
            Some(Move::By {
                dx: 0,
                dy: -1,
                step: Step::Ten,
                size: false
            })
        );
        assert_eq!(
            press(right, Modifiers::ALT | Modifiers::COMMAND),
            Some(Move::By {
                dx: 1,
                dy: 0,
                step: Step::Tenth,
                size: true
            })
        );
        assert_eq!(
            press(Key::Named(Named::Home), Modifiers::empty()),
            Some(Move::Centre)
        );
        assert_eq!(press(Key::Character("a".into()), Modifiers::empty()), None);
        assert_eq!(Step::Tenth.mm(0.127), 0.127);
        assert_eq!(Step::Tenth.mm(0.02), 0.1);
    }

    #[test]
    fn a_rectangle_stops_at_the_edges() {
        let s = Selection {
            x: 5.0,
            y: 5.0,
            width: 10.0,
            height: 10.0,
        };
        let by = |dx, dy, size| Move::By {
            dx,
            dy,
            step: Step::One,
            size,
        };
        // 8 to the left from 5: it stops at 0.
        assert_eq!(moved(s, by(-1, 0, false), 8.0, (100.0, 100.0), 2.0).x, 0.0);
        // To the right edge of a 20-wide frame: 10.
        assert_eq!(moved(s, by(1, 0, false), 8.0, (20.0, 100.0), 2.0).x, 10.0);
        // Narrower: never below the least size.
        let narrow = moved(s, by(-1, 0, true), 50.0, (100.0, 100.0), 2.0);
        assert_eq!((narrow.x, narrow.width), (5.0, 2.0));
        // Taller, as far as the bottom.
        assert_eq!(
            moved(s, by(0, 1, true), 500.0, (100.0, 100.0), 2.0).height,
            95.0
        );
        // Home: in the middle.
        let c = moved(s, Move::Centre, 0.0, (100.0, 50.0), 2.0);
        assert_eq!((c.x, c.y), (45.0, 20.0));
    }
}
