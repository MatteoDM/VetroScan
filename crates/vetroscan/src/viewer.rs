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

    /// A picture from 8-bit RGB pixels, shrunk to [`MAX_TEXTURE`] if needed
    /// (by averaging whole blocks of pixels).
    pub fn from_rgb(width: usize, height: usize, rgb: &[u8]) -> Self {
        Self::from_rgb_max(width, height, rgb, MAX_TEXTURE)
    }

    /// A picture shrunk so that its longest side is at most `max` pixels
    /// (a thumbnail).
    pub fn from_rgb_max(width: usize, height: usize, rgb: &[u8], max: u32) -> Self {
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
    marks: Vec<[[f32; 2]; 4]>,
    on_view: fn(View) -> Message,
    on_select: Option<fn(Option<Selection>) -> Message>,
    on_pick: Option<fn(Point) -> Message>,
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
        marks,
        on_view,
        on_select,
        on_pick,
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
    selection: Option<Selection>,
    /// Originals found on the glass (corners in image pixels), numbered;
    /// drawn while there is no selection.
    marks: Vec<[[f32; 2]; 4]>,
    on_view: fn(View) -> Message,
    on_select: Option<fn(Option<Selection>) -> Message>,
    /// When set, a left click picks the image point under the pointer (the
    /// eyedropper) instead of drawing a selection.
    on_pick: Option<fn(Point) -> Message>,
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
        for (at, grip) in handles(r) {
            if (p.x - at.x).abs() <= HANDLE && (p.y - at.y).abs() <= HANDLE {
                return Some(grip);
            }
        }
        r.contains(p).then_some(Grip::Move)
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
                        let (rect, _) = self.view.placement(size, self.size);
                        if !rect.contains(p) {
                            return None;
                        }
                        let start = self.image_point(size, p);
                        if let (Some(grip), Some(selection)) =
                            (self.grip_at(size, p), self.selection)
                        {
                            *drag = Drag::Edit(grip, selection, start);
                            return Some(Action::capture());
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
                    let (w, h) = (self.size.width, self.size.height);
                    let (mut l, mut t, mut r, mut b) = (s.x, s.y, s.x + s.width, s.y + s.height);
                    match grip {
                        Grip::Move => {
                            let dx = dx.clamp(-l, w - r);
                            let dy = dy.clamp(-t, h - b);
                            l += dx;
                            r += dx;
                            t += dy;
                            b += dy;
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
                    Some(Action::publish(on_select(Some(Selection::from_corners(
                        Point::new(l, t),
                        Point::new(r, b),
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
        if self.selection.is_none() {
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
            for (x, y, w, h) in [
                (r.x, r.y, r.width, top_left.y - r.y),
                (
                    r.x,
                    top_left.y + size.height,
                    r.width,
                    r.y + r.height - top_left.y - size.height,
                ),
                (r.x, top_left.y, top_left.x - r.x, size.height),
                (
                    top_left.x + size.width,
                    top_left.y,
                    r.x + r.width - top_left.x - size.width,
                    size.height,
                ),
            ] {
                if w > 0.0 && h > 0.0 {
                    frame.fill_rectangle(Point::new(x, y), Size::new(w, h), shade);
                }
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
                        frame.stroke(
                            &Path::line(
                                Point::new(x, top_left.y),
                                Point::new(x, top_left.y + size.height),
                            ),
                            stroke,
                        );
                        frame.stroke(
                            &Path::line(
                                Point::new(top_left.x, y),
                                Point::new(top_left.x + size.width, y),
                            ),
                            stroke,
                        );
                    }
                }
            }
            frame.stroke(
                &Path::rectangle(top_left, size),
                Stroke::default().with_width(2.0).with_color(primary),
            );
            if self.on_select.is_some() {
                let r = Rectangle::new(top_left, size);
                for (at, _) in handles(r) {
                    let corner = Point::new(at.x - HANDLE / 2.0, at.y - HANDLE / 2.0);
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
                    let at = Point::new(top_left.x, (top_left.y - 26.0).max(0.0));
                    frame.fill_rectangle(
                        at,
                        Size::new(8.0 + 7.5 * label.len() as f32, 20.0),
                        primary,
                    );
                    frame.fill_text(canvas::Text {
                        content: label,
                        position: Point::new(at.x + 6.0, at.y + 3.0),
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
