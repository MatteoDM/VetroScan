// SPDX-License-Identifier: GPL-2.0-or-later
//! An image viewer with zoom and pan, and the selection of an area.
//!
//! Two canvases on top of each other: the image below, the selection and
//! the mouse handling above. (Images drawn in a canvas end up above its
//! shapes, which would hide the selection.)
//!
//! Coordinates are pixels of the full-size image; what is drawn may be a
//! smaller copy (graphics cards take textures of at most 8192 pixels a
//! side, a 1200 dpi A4 is 9900 × 14000).
//!
//! Mouse: wheel zooms on the pointer, right or middle button drags the
//! image, left button draws the selection (where selecting is allowed,
//! else it drags too).
//!
//! A selection may be turned (an original laid askew on the glass): its
//! outline, handles and shade are drawn at its angle, and its handles move
//! its sides along it.

use iced::mouse;
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke};
use iced::widget::{image, stack, Canvas};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

/// Longest side of the texture drawn for an image.
pub const MAX_TEXTURE: u32 = 8192;

/// A rectangle in pixels of the image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Selection {
    fn from_corners(a: Point, b: Point) -> Self {
        Self {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            width: (a.x - b.x).abs(),
            height: (a.y - b.y).abs(),
        }
    }
}

/// Zoom and position of the view.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct View {
    /// Screen pixels per image pixel; `None` fits the image to the window.
    pub scale: Option<f32>,
    /// Image point at the centre of the window; `None` is the image centre.
    pub center: Option<Point>,
}

impl View {
    /// One screen pixel per image pixel, around the same centre.
    pub fn actual_size(self) -> Self {
        Self {
            scale: Some(1.0),
            ..self
        }
    }

    fn scale(&self, bounds: Size, image: Size) -> f32 {
        self.scale
            .unwrap_or_else(|| (bounds.width / image.width).min(bounds.height / image.height))
    }

    /// Where the whole image lies on the canvas, and the scale.
    fn placement(&self, bounds: Size, image: Size) -> (Rectangle, f32) {
        let scale = self.scale(bounds, image);
        let c = self
            .center
            .unwrap_or(Point::new(image.width / 2.0, image.height / 2.0));
        (
            Rectangle {
                x: bounds.width / 2.0 - c.x * scale,
                y: bounds.height / 2.0 - c.y * scale,
                width: image.width * scale,
                height: image.height * scale,
            },
            scale,
        )
    }
}

/// An image to show: the drawn texture and the full size.
#[derive(Debug, Clone)]
pub struct Picture {
    /// Texture, possibly smaller than the image.
    pub handle: image::Handle,
    /// Size of the full image in pixels.
    pub size: Size,
    /// The texture this picture replaces, drawn underneath: a new texture
    /// takes a frame to reach the graphics card, and without it the image
    /// would flash while a slider moves.
    pub previous: Option<image::Handle>,
}

impl Picture {
    /// This picture in place of `old`, which stays underneath until the new
    /// texture is drawn.
    pub fn replacing(mut self, old: &Picture) -> Self {
        if old.size == self.size {
            self.previous = Some(old.handle.clone());
        }
        self
    }

    /// A picture from 8-bit RGB pixels of a page drawn at `gamma`, shrunk
    /// to [`MAX_TEXTURE`] if needed (by averaging whole blocks of pixels)
    /// and converted for `screen`.
    pub fn from_rgb(
        width: usize,
        height: usize,
        rgb: &[u8],
        screen: &crate::screen::Screen,
        gamma: f64,
    ) -> Self {
        Self::from_rgb_max(width, height, rgb, MAX_TEXTURE, screen, gamma)
    }

    /// A picture shrunk so that its longest side is at most `max` pixels
    /// (a thumbnail), converted for `screen`: every picture the app shows
    /// of a page goes through here.
    pub fn from_rgb_max(
        width: usize,
        height: usize,
        rgb: &[u8],
        max: u32,
        screen: &crate::screen::Screen,
        gamma: f64,
    ) -> Self {
        let factor = (width.max(height) as u32).div_ceil(max).max(1) as usize;
        let (w, h) = (width / factor, height / factor);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 3];
                for yy in 0..factor {
                    let row = (y * factor + yy) * width;
                    for xx in 0..factor {
                        let p = (row + x * factor + xx) * 3;
                        for c in 0..3 {
                            sum[c] += u32::from(rgb[p + c]);
                        }
                    }
                }
                let n = (factor * factor) as u32;
                rgba.extend_from_slice(&[
                    (sum[0] / n) as u8,
                    (sum[1] / n) as u8,
                    (sum[2] / n) as u8,
                    255,
                ]);
            }
        }
        screen.show(&mut rgba, gamma);
        Self {
            handle: image::Handle::from_rgba(w as u32, h as u32, rgba),
            size: Size::new(width as f32, height as f32),
            previous: None,
        }
    }
}

/// The viewer of a picture (or of an empty area of `placeholder` size).
#[allow(clippy::too_many_arguments)]
pub fn viewer<'a, Message: Clone + 'a>(
    picture: Option<&'a Picture>,
    placeholder: Size,
    hint: &'a str,
    view: View,
    selection: Option<Selection>,
    angle: f32,
    marks: Vec<[[f32; 2]; 4]>,
    on_view: fn(View) -> Message,
    on_select: Option<fn(Option<Selection>) -> Message>,
    on_pick: Option<fn(Point) -> Message>,
    on_mark: Option<fn(usize) -> Message>,
    mm_per_pixel: f32,
    grid: bool,
) -> Element<'a, Message> {
    let size = picture.map_or(placeholder, |p| p.size);
    let below = Canvas::new(ImageLayer {
        picture,
        size,
        view,
        hint,
    })
    .width(Length::Fill)
    .height(Length::Fill);
    let above = Canvas::new(Overlay {
        size,
        view,
        selection,
        angle,
        marks,
        on_view,
        on_select,
        on_pick,
        on_mark,
        mm_per_pixel,
        grid,
    })
    .width(Length::Fill)
    .height(Length::Fill);
    stack![below, above].into()
}

struct ImageLayer<'a> {
    picture: Option<&'a Picture>,
    size: Size,
    view: View,
    hint: &'a str,
}

impl<Message> canvas::Program<Message> for ImageLayer<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (rect, _) = self.view.placement(bounds.size(), self.size);
        match self.picture {
            Some(p) => {
                if let Some(previous) = &p.previous {
                    frame.draw_image(rect, previous);
                }
                frame.draw_image(rect, &p.handle);
            }
            None => {
                let palette = theme.extended_palette();
                frame.fill_rectangle(rect.position(), rect.size(), palette.background.weak.color);
                frame.fill_text(canvas::Text {
                    content: self.hint.to_string(),
                    position: rect.center(),
                    color: palette.background.weak.text,
                    size: 18.0.into(),
                    align_x: iced::widget::text::Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    ..canvas::Text::default()
                });
            }
        }
        vec![frame.into_geometry()]
    }
}

struct Overlay<Message> {
    size: Size,
    view: View,
    /// The selection as if it were upright, around its centre.
    selection: Option<Selection>,
    /// The selection's turn around its centre, degrees clockwise on screen
    /// (as [`scan_imaging::detect::Found::angle`]).
    angle: f32,
    /// Originals found on the glass (corners in image pixels), numbered.
    marks: Vec<[[f32; 2]; 4]>,
    on_view: fn(View) -> Message,
    on_select: Option<fn(Option<Selection>) -> Message>,
    /// When set, a left click picks the image point under the pointer (the
    /// eyedropper) instead of drawing a selection.
    on_pick: Option<fn(Point) -> Message>,
    /// When set, a left click on a mark (outside the selection) chooses
    /// it instead of starting a selection.
    on_mark: Option<fn(usize) -> Message>,
    /// Millimetres per image pixel, for the size label (0: no label).
    mm_per_pixel: f32,
    /// Thirds drawn in the selection (cropping, straightening).
    grid: bool,
}

/// What a drag is doing.
#[derive(Debug, Default)]
pub enum Drag {
    #[default]
    None,
    /// Selecting from this image point.
    Select(Point),
    /// Panning: canvas point where it started and centre then.
    Pan(Point, Point),
    /// Moving or resizing the selection: which grip, the selection and the
    /// image point at the start.
    Edit(Grip, Selection, Point),
}

/// Where the selection is taken: one of its eight handles, or inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grip {
    Move,
    /// Edges taken: left, top, right, bottom.
    Edges(bool, bool, bool, bool),
}

/// Size of a handle on screen.
const HANDLE: f32 = 9.0;

/// The handles of a selection on screen: position and grip.
fn handles(r: Rectangle) -> [(Point, Grip); 8] {
    let (l, t, rr, b) = (r.x, r.y, r.x + r.width, r.y + r.height);
    let (cx, cy) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
    [
        (Point::new(l, t), Grip::Edges(true, true, false, false)),
        (Point::new(cx, t), Grip::Edges(false, true, false, false)),
        (Point::new(rr, t), Grip::Edges(false, true, true, false)),
        (Point::new(rr, cy), Grip::Edges(false, false, true, false)),
        (Point::new(rr, b), Grip::Edges(false, false, true, true)),
        (Point::new(cx, b), Grip::Edges(false, false, false, true)),
        (Point::new(l, b), Grip::Edges(true, false, false, true)),
        (Point::new(l, cy), Grip::Edges(true, false, false, false)),
    ]
}

/// `p` turned by `angle` degrees (clockwise on screen) around `c`.
fn turned(p: Point, c: Point, angle: f32) -> Point {
    if angle == 0.0 {
        return p;
    }
    let (sin, cos) = angle.to_radians().sin_cos();
    let (dx, dy) = (p.x - c.x, p.y - c.y);
    Point::new(c.x + dx * cos - dy * sin, c.y + dx * sin + dy * cos)
}

/// The part of a convex outline inside `r` (Sutherland–Hodgman).
fn clipped(outline: &[Point], r: Rectangle) -> Vec<Point> {
    // How far inside each side of `r` a point is.
    let sides: [&dyn Fn(Point) -> f32; 4] = [
        &|p| p.x - r.x,
        &|p| r.x + r.width - p.x,
        &|p| p.y - r.y,
        &|p| r.y + r.height - p.y,
    ];
    let mut out = outline.to_vec();
    for inside in sides {
        let points = std::mem::take(&mut out);
        for (i, &a) in points.iter().enumerate() {
            let b = points[(i + 1) % points.len()];
            let (da, db) = (inside(a), inside(b));
            if da >= 0.0 {
                out.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                out.push(Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
            }
        }
    }
    out
}

/// Selection `s` (as if upright, turned by `angle` around its centre)
/// moved or resized by `grip` dragged by (`dx`, `dy`) image pixels, within
/// an image of `size`: a turned one's sides move along its angle, its
/// centre stays on the image and its sides no longer than the image's.
fn edited(grip: Grip, s: Selection, dx: f32, dy: f32, angle: f32, size: Size) -> Selection {
    let (w, h) = (size.width, size.height);
    let (mut l, mut t, mut r, mut b) = (s.x, s.y, s.x + s.width, s.y + s.height);
    match grip {
        Grip::Move => {
            // No further off the image, but no jump back onto it either (a
            // turned area may stick out of it).
            let dx = dx.clamp((-l).min(0.0), (w - r).max(0.0));
            let dy = dy.clamp((-t).min(0.0), (h - b).max(0.0));
            l += dx;
            r += dx;
            t += dy;
            b += dy;
        }
        Grip::Edges(el, et, er, eb) if angle != 0.0 => {
            // The drag along the selection's sides.
            let (sin, cos) = angle.to_radians().sin_cos();
            let du = dx * cos + dy * sin;
            let dv = -dx * sin + dy * cos;
            let (hw, hh) = (s.width / 2.0, s.height / 2.0);
            let (mut l2, mut t2, mut r2, mut b2) = (-hw, -hh, hw, hh);
            // The side taken no farther from the opposite one than the
            // image is long, which stays where it is.
            if el {
                l2 = (l2 + du).clamp(r2 - w, r2 + w);
            }
            if et {
                t2 = (t2 + dv).clamp(b2 - h, b2 + h);
            }
            if er {
                r2 = (r2 + du).clamp(l2 - w, l2 + w);
            }
            if eb {
                b2 = (b2 + dv).clamp(t2 - h, t2 + h);
            }
            let c = Point::new(s.x + hw, s.y + hh);
            let mid = turned(
                Point::new(c.x + (l2 + r2) / 2.0, c.y + (t2 + b2) / 2.0),
                c,
                angle,
            );
            let mid = Point::new(mid.x.clamp(0.0, w), mid.y.clamp(0.0, h));
            let (hw, hh) = ((r2 - l2).abs() / 2.0, (b2 - t2).abs() / 2.0);
            (l, t, r, b) = (mid.x - hw, mid.y - hh, mid.x + hw, mid.y + hh);
        }
        Grip::Edges(el, et, er, eb) => {
            if el {
                l = (l + dx).clamp(0.0, w);
            }
            if et {
                t = (t + dy).clamp(0.0, h);
            }
            if er {
                r = (r + dx).clamp(0.0, w);
            }
            if eb {
                b = (b + dy).clamp(0.0, h);
            }
        }
    }
    Selection::from_corners(Point::new(l, t), Point::new(r, b))
}

impl<Message> Overlay<Message> {
    /// The selection on screen.
    fn selection_rect(&self, bounds: Size) -> Option<Rectangle> {
        let s = self.selection?;
        let (rect, scale) = self.view.placement(bounds, self.size);
        Some(Rectangle {
            x: rect.x + s.x * scale,
            y: rect.y + s.y * scale,
            width: s.width * scale,
            height: s.height * scale,
        })
    }

    /// Which part of the selection is under a canvas point.
    fn grip_at(&self, bounds: Size, p: Point) -> Option<Grip> {
        let r = self.selection_rect(bounds)?;
        // Seen from the selection, upright.
        let p = turned(p, r.center(), -self.angle);
        for (at, grip) in handles(r) {
            if (p.x - at.x).abs() <= HANDLE && (p.y - at.y).abs() <= HANDLE {
                return Some(grip);
            }
        }
        r.contains(p).then_some(Grip::Move)
    }

    /// The mark under an image point, the last drawn first (it is on top).
    fn mark_at(&self, p: Point) -> Option<usize> {
        self.marks.iter().rposition(|corners| {
            // Inside a convex outline: on the same side of each of its edges.
            let sides: Vec<f32> = (0..4)
                .map(|i| {
                    let [ax, ay] = corners[i];
                    let [bx, by] = corners[(i + 1) % 4];
                    (bx - ax) * (p.y - ay) - (by - ay) * (p.x - ax)
                })
                .collect();
            sides.iter().all(|&s| s >= 0.0) || sides.iter().all(|&s| s <= 0.0)
        })
    }

    fn image_point(&self, bounds: Size, p: Point) -> Point {
        let (rect, scale) = self.view.placement(bounds, self.size);
        Point::new((p.x - rect.x) / scale, (p.y - rect.y) / scale)
    }

    fn center(&self) -> Point {
        self.view
            .center
            .unwrap_or(Point::new(self.size.width / 2.0, self.size.height / 2.0))
    }
}

impl<Message> canvas::Program<Message> for Overlay<Message> {
    type State = Drag;

    fn update(
        &self,
        drag: &mut Drag,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let size = bounds.size();
        // Position relative to the canvas, also outside it while dragging.
        let at = cursor
            .position()
            .map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let p = cursor.position_in(bounds)?;
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 60.0,
                };
                let old = self.view.scale(size, self.size);
                let fit = View::default().scale(size, self.size);
                let new = (old * 1.2f32.powf(lines)).clamp(fit.min(1.0) * 0.5, 16.0);
                // Keep the image point under the pointer where it is.
                let under = self.image_point(size, p);
                let c = Point::new(
                    under.x - (p.x - size.width / 2.0) / new,
                    under.y - (p.y - size.height / 2.0) / new,
                );
                Some(
                    Action::publish((self.on_view)(View {
                        scale: Some(new),
                        center: Some(c),
                    }))
                    .and_capture(),
                )
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let p = cursor.position_in(bounds)?;
                if let (mouse::Button::Left, Some(on_pick)) = (button, self.on_pick) {
                    let (rect, _) = self.view.placement(size, self.size);
                    if !rect.contains(p) {
                        return None;
                    }
                    return Some(Action::publish(on_pick(self.image_point(size, p))).and_capture());
                }
                match (button, self.on_select) {
                    (mouse::Button::Left, Some(on_select)) => {
                        let start = self.image_point(size, p);
                        // A handle is taken also where it lies off the image
                        // (a turned area's corner).
                        if let (Some(grip), Some(selection)) =
                            (self.grip_at(size, p), self.selection)
                        {
                            *drag = Drag::Edit(grip, selection, start);
                            return Some(Action::capture());
                        }
                        let (rect, _) = self.view.placement(size, self.size);
                        if !rect.contains(p) {
                            return None;
                        }
                        if let Some(i) = self.on_mark.and_then(|_| self.mark_at(start)) {
                            let on_mark = self.on_mark?;
                            return Some(Action::publish(on_mark(i)).and_capture());
                        }
                        let start = Point::new(
                            start.x.clamp(0.0, self.size.width),
                            start.y.clamp(0.0, self.size.height),
                        );
                        *drag = Drag::Select(start);
                        Some(Action::publish(on_select(None)).and_capture())
                    }
                    (mouse::Button::Left | mouse::Button::Right | mouse::Button::Middle, _) => {
                        *drag = Drag::Pan(p, self.center());
                        Some(Action::capture())
                    }
                    _ => None,
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => match *drag {
                Drag::None => None,
                Drag::Select(start) => {
                    let end = self.image_point(size, at?);
                    let end = Point::new(
                        end.x.clamp(0.0, self.size.width),
                        end.y.clamp(0.0, self.size.height),
                    );
                    let on_select = self.on_select?;
                    Some(Action::publish(on_select(Some(Selection::from_corners(
                        start, end,
                    )))))
                }
                Drag::Edit(grip, s, from) => {
                    let on_select = self.on_select?;
                    let p = self.image_point(size, at?);
                    let (dx, dy) = (p.x - from.x, p.y - from.y);
                    Some(Action::publish(on_select(Some(edited(
                        grip, s, dx, dy, self.angle, self.size,
                    )))))
                }
                Drag::Pan(from, center) => {
                    let scale = self.view.scale(size, self.size);
                    let d: Vector = at? - from;
                    Some(Action::publish((self.on_view)(View {
                        scale: Some(scale),
                        center: Some(Point::new(center.x - d.x / scale, center.y - d.y / scale)),
                    })))
                }
            },
            Event::Mouse(mouse::Event::ButtonReleased(_)) => match std::mem::take(drag) {
                Drag::None => None,
                _ => Some(Action::capture()),
            },
            _ => None,
        }
    }

    fn draw(
        &self,
        _drag: &Drag,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (rect, scale) = self.view.placement(bounds.size(), self.size);
        frame.stroke(
            &Path::rectangle(rect.position(), rect.size()),
            Stroke::default().with_color(theme.extended_palette().background.strong.color),
        );
        {
            let primary = theme.palette().primary;
            for (i, corners) in self.marks.iter().enumerate() {
                let at = corners.map(|[x, y]| Point::new(rect.x + x * scale, rect.y + y * scale));
                let outline = Path::new(|b| {
                    b.move_to(at[0]);
                    for p in &at[1..] {
                        b.line_to(*p);
                    }
                    b.close();
                });
                frame.fill(&outline, Color { a: 0.12, ..primary });
                frame.stroke(
                    &outline,
                    Stroke::default().with_width(2.0).with_color(primary),
                );
                // The number, at the centre.
                let c = Point::new(
                    at.iter().map(|p| p.x).sum::<f32>() / 4.0,
                    at.iter().map(|p| p.y).sum::<f32>() / 4.0,
                );
                frame.fill(&Path::circle(c, 14.0), primary);
                frame.fill_text(canvas::Text {
                    content: (i + 1).to_string(),
                    position: c,
                    color: Color::WHITE,
                    size: 15.0.into(),
                    align_x: iced::widget::text::Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    ..canvas::Text::default()
                });
            }
        }
        if let Some(s) = self.selection {
            let top_left = Point::new(rect.x + s.x * scale, rect.y + s.y * scale);
            let size = Size::new(s.width * scale, s.height * scale);
            let primary = theme.palette().primary;
            // Darken what is left out, outline what is in.
            let shade = Color {
                a: 0.35,
                ..Color::BLACK
            };
            let r = rect;
            let c = Point::new(
                top_left.x + size.width / 2.0,
                top_left.y + size.height / 2.0,
            );
            // A point of the selection, as if upright, where it is drawn.
            let at = |x: f32, y: f32| turned(Point::new(x, y), c, self.angle);
            let (right, bottom) = (top_left.x + size.width, top_left.y + size.height);
            let outline = [
                at(top_left.x, top_left.y),
                at(right, top_left.y),
                at(right, bottom),
                at(top_left.x, bottom),
            ];
            if self.angle == 0.0 {
                for (x, y, w, h) in [
                    (r.x, r.y, r.width, top_left.y - r.y),
                    (r.x, bottom, r.width, r.y + r.height - bottom),
                    (r.x, top_left.y, top_left.x - r.x, size.height),
                    (right, top_left.y, r.x + r.width - right, size.height),
                ] {
                    if w > 0.0 && h > 0.0 {
                        frame.fill_rectangle(Point::new(x, y), Size::new(w, h), shade);
                    }
                }
            } else {
                // The image with a hole where the selection is (the part of
                // it on the image: a hole outside would be filled).
                let hole = clipped(&outline, r);
                let shaded = Path::new(|p| {
                    p.rectangle(r.position(), r.size());
                    if let Some((first, rest)) = hole.split_first().filter(|_| hole.len() >= 3) {
                        p.move_to(*first);
                        for q in rest {
                            p.line_to(*q);
                        }
                        p.close();
                    }
                });
                frame.fill(
                    &shaded,
                    canvas::Fill {
                        style: canvas::Style::Solid(shade),
                        rule: canvas::fill::Rule::EvenOdd,
                    },
                );
            }
            if self.grid {
                // Dark under light: seen on white paper and on dark photos.
                let dark = Stroke::default().with_width(3.0).with_color(Color {
                    a: 0.35,
                    ..Color::BLACK
                });
                let line = Stroke::default().with_width(1.0).with_color(Color {
                    a: 0.85,
                    ..Color::WHITE
                });
                for k in [1.0 / 3.0, 2.0 / 3.0] {
                    let x = top_left.x + size.width * k;
                    let y = top_left.y + size.height * k;
                    for stroke in [dark, line] {
                        frame.stroke(&Path::line(at(x, top_left.y), at(x, bottom)), stroke);
                        frame.stroke(&Path::line(at(top_left.x, y), at(right, y)), stroke);
                    }
                }
            }
            frame.stroke(
                &Path::new(|p| {
                    p.move_to(outline[0]);
                    for q in &outline[1..] {
                        p.line_to(*q);
                    }
                    p.close();
                }),
                Stroke::default().with_width(2.0).with_color(primary),
            );
            if self.on_select.is_some() {
                let r = Rectangle::new(top_left, size);
                for (h, _) in handles(r) {
                    let h = at(h.x, h.y);
                    let corner = Point::new(h.x - HANDLE / 2.0, h.y - HANDLE / 2.0);
                    frame.fill_rectangle(corner, Size::new(HANDLE, HANDLE), Color::WHITE);
                    frame.stroke(
                        &Path::rectangle(corner, Size::new(HANDLE, HANDLE)),
                        Stroke::default().with_width(1.5).with_color(primary),
                    );
                }
                if self.mm_per_pixel > 0.0 {
                    let label = format!(
                        "{:.0} × {:.0} mm",
                        s.width * self.mm_per_pixel,
                        s.height * self.mm_per_pixel
                    );
                    // Above the outline, at its left.
                    let left = outline.iter().map(|p| p.x).fold(f32::MAX, f32::min);
                    let top = outline.iter().map(|p| p.y).fold(f32::MAX, f32::min);
                    let place = Point::new(left, (top - 26.0).max(0.0));
                    frame.fill_rectangle(
                        place,
                        Size::new(8.0 + 7.5 * label.len() as f32, 20.0),
                        primary,
                    );
                    frame.fill_text(canvas::Text {
                        content: label,
                        position: Point::new(place.x + 6.0, place.y + 3.0),
                        color: Color::WHITE,
                        size: 13.0.into(),
                        ..canvas::Text::default()
                    });
                }
            }
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        drag: &Drag,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match drag {
            Drag::Pan(..) => mouse::Interaction::Grabbing,
            Drag::Select(_) => mouse::Interaction::Crosshair,
            Drag::Edit(Grip::Move, ..) => mouse::Interaction::Grabbing,
            Drag::Edit(..) => mouse::Interaction::Crosshair,
            Drag::None => match cursor.position_in(bounds) {
                Some(p) if self.on_select.is_some() && self.on_pick.is_none() => {
                    match self.grip_at(bounds.size(), p) {
                        Some(Grip::Move) => mouse::Interaction::Grab,
                        Some(Grip::Edges(l, t, r, b)) => match (l || r, t || b) {
                            (true, true) => {
                                if (l && t) || (r && b) {
                                    mouse::Interaction::ResizingDiagonallyDown
                                } else {
                                    mouse::Interaction::ResizingDiagonallyUp
                                }
                            }
                            (true, false) => mouse::Interaction::ResizingHorizontally,
                            _ => mouse::Interaction::ResizingVertically,
                        },
                        None if self.on_mark.is_some()
                            && self.mark_at(self.image_point(bounds.size(), p)).is_some() =>
                        {
                            mouse::Interaction::Pointer
                        }
                        None => mouse::Interaction::Crosshair,
                    }
                }
                Some(_) if self.on_pick.is_some() || self.on_select.is_some() => {
                    mouse::Interaction::Crosshair
                }
                Some(_) => mouse::Interaction::Grab,
                None => mouse::Interaction::default(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scan_imaging::detect::Found;

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    fn found(s: Selection, angle: f32) -> Found {
        Found {
            cx: s.x + s.width / 2.0,
            cy: s.y + s.height / 2.0,
            width: s.width,
            height: s.height,
            angle,
        }
    }

    const S: Selection = Selection {
        x: 100.0,
        y: 200.0,
        width: 300.0,
        height: 150.0,
    };

    /// The outline drawn is the area cut: the corners of the selection
    /// turned by its angle are those of the original found.
    #[test]
    fn the_outline_turns_as_the_area_found() {
        let c = Point::new(S.x + S.width / 2.0, S.y + S.height / 2.0);
        let (r, b) = (S.x + S.width, S.y + S.height);
        let upright = [(S.x, S.y), (r, S.y), (r, b), (S.x, b)];
        for angle in [-30.0, 0.0, 12.5, 45.0] {
            let corners = found(S, angle).corners();
            for ((x, y), [fx, fy]) in upright.into_iter().zip(corners) {
                assert!(close(
                    turned(Point::new(x, y), c, angle),
                    Point::new(fx, fy)
                ));
            }
        }
    }

    #[test]
    fn an_upright_selection_moves_its_sides_as_before() {
        let size = Size::new(1000.0, 1000.0);
        let right = Grip::Edges(false, false, true, false);
        let s = edited(right, S, 10.0, 7.0, 0.0, size);
        assert_eq!((s.x, s.y, s.width, s.height), (100.0, 200.0, 310.0, 150.0));
        // Within the image.
        let s = edited(right, S, 900.0, 0.0, 0.0, size);
        assert_eq!(s.x + s.width, 1000.0);
        let s = edited(Grip::Move, S, -500.0, 0.0, 0.0, size);
        assert_eq!((s.x, s.width), (0.0, 300.0));
    }

    /// A turned selection's side moves along its angle, the opposite side
    /// stays where it is.
    #[test]
    fn a_turned_selection_moves_its_sides_along_its_angle() {
        let size = Size::new(1000.0, 1000.0);
        let right = Grip::Edges(false, false, true, false);
        // At 90° the right side faces down: a drag across it does nothing.
        let s = edited(right, S, 10.0, 0.0, 90.0, size);
        assert!((s.width - S.width).abs() < 1e-3, "{s:?}");
        let s = edited(right, S, 0.0, 10.0, 90.0, size);
        assert!((s.width - 310.0).abs() < 1e-3, "{s:?}");
        for (grip, angle) in [
            (right, 30.0),
            (Grip::Edges(true, true, false, false), -20.0),
            (Grip::Edges(false, false, false, true), 7.5),
        ] {
            let after = edited(grip, S, 23.0, -11.0, angle, size);
            let (old, new) = (found(S, angle).corners(), found(after, angle).corners());
            // The corner opposite the one (or the side) taken stays.
            let kept = if grip == Grip::Edges(true, true, false, false) {
                2
            } else {
                0
            };
            let (a, b) = (old[kept], new[kept]);
            assert!(
                close(Point::new(a[0], a[1]), Point::new(b[0], b[1])),
                "{grip:?} {angle}: {a:?} → {b:?}"
            );
        }
    }

    /// A turned area grows no larger than the image, and one sticking out
    /// of it moves without jumping back (nor panicking).
    #[test]
    fn a_turned_selection_stays_no_larger_than_the_image() {
        let size = Size::new(1000.0, 1000.0);
        let wide = Selection {
            x: 10.0,
            y: 400.0,
            width: 980.0,
            height: 200.0,
        };
        let right = Grip::Edges(false, false, true, false);
        let s = edited(right, wide, 3000.0, 0.0, 0.1, size);
        assert!(s.width <= size.width + 1e-3, "{s:?}");
        // Its left side where it was.
        let ([a, ..], [b, ..]) = (found(wide, 0.1).corners(), found(s, 0.1).corners());
        assert!(
            close(Point::new(a[0], a[1]), Point::new(b[0], b[1])),
            "{a:?} → {b:?}"
        );
        let s = edited(right, wide, -3000.0, 0.0, 0.1, size);
        assert!(s.width <= size.width + 1e-3, "{s:?}");
        let moved = edited(Grip::Move, s, 1.0, 0.0, 0.1, size);
        assert!((moved.x - s.x).abs() <= 1.0 + 1e-3, "{s:?} → {moved:?}");
        let left = Grip::Edges(true, false, false, false);
        let out = edited(left, S, -400.0, 0.0, 20.0, size);
        assert!(out.x < 0.0, "{out:?}");
        let moved = edited(Grip::Move, out, 1.0, 0.0, 20.0, size);
        assert!((moved.x - out.x - 1.0).abs() < 1e-3, "{out:?} → {moved:?}");
        let moved = edited(Grip::Move, out, -1.0, 0.0, 20.0, size);
        assert_eq!(moved.x, out.x);
    }

    #[test]
    fn the_part_of_an_outline_on_the_image() {
        let r = Rectangle::new(Point::ORIGIN, Size::new(100.0, 100.0));
        let square = [
            Point::new(50.0, 50.0),
            Point::new(150.0, 50.0),
            Point::new(150.0, 150.0),
            Point::new(50.0, 150.0),
        ];
        let cut = clipped(&square, r);
        assert_eq!(cut.len(), 4);
        assert!(cut
            .iter()
            .all(|p| r.contains(*p) || p.x == 100.0 || p.y == 100.0));
        let outside = square.map(|p| Point::new(p.x + 200.0, p.y));
        assert!(clipped(&outside, r).is_empty());
        assert_eq!(
            clipped(&square.map(|p| Point::new(p.x - 40.0, p.y - 40.0)), r).len(),
            4
        );
    }

    /// The handles are taken where they are drawn, at the selection's angle.
    #[test]
    fn a_turned_selection_is_taken_by_its_turned_handles() {
        let overlay = |angle: f32| Overlay::<()> {
            size: Size::new(1000.0, 1000.0),
            view: View {
                scale: Some(1.0),
                center: None,
            },
            selection: Some(S),
            angle,
            marks: Vec::new(),
            on_view: |_| (),
            on_select: None,
            on_pick: None,
            on_mark: None,
            mm_per_pixel: 0.0,
            grid: false,
        };
        let bounds = Size::new(1000.0, 1000.0);
        let turned_overlay = overlay(30.0);
        let [tl, _, br, _] = found(S, 30.0).corners();
        assert_eq!(
            turned_overlay.grip_at(bounds, Point::new(tl[0], tl[1])),
            Some(Grip::Edges(true, true, false, false))
        );
        assert_eq!(
            turned_overlay.grip_at(bounds, Point::new(br[0], br[1])),
            Some(Grip::Edges(false, false, true, true))
        );
        // The upright corner, outside the turned area, is no handle.
        assert_eq!(turned_overlay.grip_at(bounds, Point::new(S.x, S.y)), None);
        assert_eq!(
            overlay(0.0).grip_at(bounds, Point::new(S.x, S.y)),
            Some(Grip::Edges(true, true, false, false))
        );
    }
}
