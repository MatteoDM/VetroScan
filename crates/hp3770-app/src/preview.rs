// SPDX-License-Identifier: GPL-2.0-or-later
//! The preview of the glass, on which the area to scan is drawn with the
//! mouse.

use iced::mouse;
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke};
use iced::widget::image;
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

/// A rectangle in pixels of the preview image.
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

/// The canvas program: the preview image (or the empty glass) and the
/// selection.
#[derive(Debug)]
pub struct Preview<'a, Message> {
    /// The preview image and its size in pixels.
    pub image: Option<(&'a image::Handle, Size)>,
    /// Size of the glass in preview pixels (when there is no image yet).
    pub glass: Size,
    /// The selected area.
    pub selection: Option<Selection>,
    /// Message for a new selection (`None`: cleared).
    pub on_select: fn(Option<Selection>) -> Message,
}

/// Drag in progress, in image pixels.
#[derive(Debug, Default)]
pub struct State {
    start: Option<Point>,
}

impl<Message> Preview<'_, Message> {
    fn size(&self) -> Size {
        self.image.map_or(self.glass, |(_, s)| s)
    }

    /// Where the image is drawn in `bounds` (aspect ratio kept, centred)
    /// and its scale.
    fn placement(&self, bounds: Size) -> (Rectangle, f32) {
        let size = self.size();
        let scale = (bounds.width / size.width).min(bounds.height / size.height);
        let w = size.width * scale;
        let h = size.height * scale;
        (
            Rectangle {
                x: (bounds.width - w) / 2.0,
                y: (bounds.height - h) / 2.0,
                width: w,
                height: h,
            },
            scale,
        )
    }

    /// The image pixel under a position in the canvas, clamped to the image.
    fn to_image(&self, bounds: Size, p: Point) -> Point {
        let (rect, scale) = self.placement(bounds);
        let size = self.size();
        Point::new(
            ((p.x - rect.x) / scale).clamp(0.0, size.width),
            ((p.y - rect.y) / scale).clamp(0.0, size.height),
        )
    }
}

impl<Message> canvas::Program<Message> for Preview<'_, Message> {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let size = bounds.size();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                let (rect, _) = self.placement(size);
                if !rect.contains(p) {
                    return None;
                }
                state.start = Some(self.to_image(size, p));
                Some(Action::publish((self.on_select)(None)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let start = state.start?;
                let p = cursor.position_in(bounds).or_else(|| {
                    cursor
                        .position()
                        .map(|p| Point::new(p.x - bounds.x, p.y - bounds.y))
                })?;
                let end = self.to_image(size, p);
                Some(Action::publish((self.on_select)(Some(
                    Selection::from_corners(start, end),
                ))))
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.start.take().map(|_| Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                cursor.position_in(bounds)?;
                Some(Action::publish((self.on_select)(None)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (rect, scale) = self.placement(bounds.size());
        match self.image {
            Some((handle, _)) => frame.draw_image(rect, handle),
            None => {
                frame.fill_rectangle(
                    rect.position(),
                    rect.size(),
                    Color::from_rgb8(235, 235, 235),
                );
                frame.fill_text(canvas::Text {
                    content: "Fai un'anteprima per vedere il vetro".into(),
                    position: rect.center(),
                    color: Color::from_rgb8(120, 120, 120),
                    size: 18.0.into(),
                    align_x: iced::widget::text::Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    ..canvas::Text::default()
                });
            }
        }
        frame.stroke(
            &Path::rectangle(rect.position(), rect.size()),
            Stroke::default().with_color(Color::from_rgb8(160, 160, 160)),
        );
        if let Some(s) = self.selection {
            let top_left = Point::new(rect.x + s.x * scale, rect.y + s.y * scale);
            let size = Size::new(s.width * scale, s.height * scale);
            frame.fill_rectangle(
                top_left,
                size,
                Color {
                    a: 0.15,
                    ..theme.palette().primary
                },
            );
            frame.stroke(
                &Path::rectangle(top_left, size),
                Stroke::default()
                    .with_width(2.0)
                    .with_color(theme.palette().primary),
            );
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match cursor.position_in(bounds) {
            Some(p) if self.placement(bounds.size()).0.contains(p) => mouse::Interaction::Crosshair,
            _ => mouse::Interaction::default(),
        }
    }
}
