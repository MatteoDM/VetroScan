// SPDX-License-Identifier: GPL-2.0-or-later
//! The tone curve editor: the curve of one channel with three points (at
//! the quarter, half and three quarters of the input) dragged up and down.

use hp3770_core::adjust::curve;
use iced::mouse;
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

/// The editor of one curve.
#[derive(Debug)]
pub struct Editor<Message> {
    /// Output of the three points (0-1).
    pub points: [f32; 3],
    /// Colour of the curve.
    pub colour: Color,
    /// Message for a point moved: index and new output.
    pub on_change: fn(usize, f32) -> Message,
}

/// The point being dragged.
#[derive(Debug, Default)]
pub struct State {
    dragging: Option<usize>,
}

const XS: [f32; 3] = [0.25, 0.5, 0.75];

fn to_screen(b: Size, x: f32, y: f32) -> Point {
    Point::new(x * b.width, (1.0 - y) * b.height)
}

impl<Message> canvas::Program<Message> for Editor<Message> {
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
                // The nearest point, if close enough across.
                let (i, d) = XS
                    .iter()
                    .enumerate()
                    .map(|(i, &x)| (i, (to_screen(size, x, self.points[i]).x - p.x).abs()))
                    .min_by(|a, b| a.1.total_cmp(&b.1))?;
                if d > size.width / 8.0 {
                    return None;
                }
                state.dragging = Some(i);
                let y = (1.0 - p.y / size.height).clamp(0.0, 1.0);
                Some(Action::publish((self.on_change)(i, y)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let i = state.dragging?;
                let y = (1.0 - (position.y - bounds.y) / size.height).clamp(0.0, 1.0);
                Some(Action::publish((self.on_change)(i, y)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging.take().map(|_| Action::capture())
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
        let size = bounds.size();
        let palette = theme.extended_palette();
        let mut frame = Frame::new(renderer, size);
        frame.fill_rectangle(Point::ORIGIN, size, palette.background.weak.color);
        // Grid in quarters, and the diagonal.
        for k in 1..4 {
            let f = k as f32 / 4.0;
            let grid = Stroke::default().with_color(palette.background.strong.color);
            frame.stroke(
                &Path::line(
                    Point::new(f * size.width, 0.0),
                    Point::new(f * size.width, size.height),
                ),
                grid,
            );
            frame.stroke(
                &Path::line(
                    Point::new(0.0, f * size.height),
                    Point::new(size.width, f * size.height),
                ),
                grid,
            );
        }
        frame.stroke(
            &Path::line(to_screen(size, 0.0, 0.0), to_screen(size, 1.0, 1.0)),
            Stroke::default().with_color(palette.background.strong.color),
        );
        let path = Path::new(|b| {
            b.move_to(to_screen(size, 0.0, 0.0));
            for i in 1..=64 {
                let x = i as f32 / 64.0;
                b.line_to(to_screen(size, x, curve(&self.points, x)));
            }
        });
        frame.stroke(
            &path,
            Stroke::default().with_width(2.0).with_color(self.colour),
        );
        for (i, &x) in XS.iter().enumerate() {
            let at = to_screen(size, x, self.points[i]);
            frame.fill(&Path::circle(at, 5.0), Color::WHITE);
            frame.stroke(
                &Path::circle(at, 5.0),
                Stroke::default().with_width(2.0).with_color(self.colour),
            );
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging.is_some() {
            mouse::Interaction::ResizingVertically
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}
