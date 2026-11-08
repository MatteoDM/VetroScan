// SPDX-License-Identifier: GPL-2.0-or-later
//! The tone curve editor, as in Photoshop and Lightroom: click to add a
//! point (up to 16), drag to move it, drag it out of the box or click it
//! twice to remove it; the end points are the black and white points and
//! can be moved too. The histogram is drawn behind, with a 4 × 4 grid, and
//! the input and output of the point being moved are shown.

use iced::mouse;
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};
use scan_imaging::adjust::Curve;
use std::time::{Duration, Instant};

/// The editor of one curve.
#[derive(Debug)]
pub struct Editor<Message> {
    pub curve: Curve,
    /// Colour of the curve.
    pub colour: Color,
    /// Counts of each level of the image, drawn behind.
    pub histogram: Option<[u32; 256]>,
    /// Message with the new points of the curve.
    pub on_change: fn(Vec<[f32; 2]>) -> Message,
}

/// The point being dragged, and the last click (for the double click).
#[derive(Debug, Default)]
pub struct State {
    dragging: Option<usize>,
    last_click: Option<(usize, Instant)>,
}

/// How close (on screen) a click must be to take a point.
const GRAB: f32 = 9.0;
/// Least distance between two points' inputs.
const GAP: f32 = 0.01;

fn to_screen(b: Size, p: [f32; 2]) -> Point {
    Point::new(p[0] * b.width, (1.0 - p[1]) * b.height)
}

fn from_screen(b: Size, p: Point) -> [f32; 2] {
    [
        (p.x / b.width).clamp(0.0, 1.0),
        (1.0 - p.y / b.height).clamp(0.0, 1.0),
    ]
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
        let points = self.curve.points();
        let n = points.len();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let at = cursor.position_in(bounds)?;
                let near = points
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i, to_screen(size, *p).distance(at)))
                    .filter(|(_, d)| *d <= GRAB)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i);
                if let Some(i) = near {
                    // Twice on an inner point: remove it.
                    let twice = matches!(
                        state.last_click,
                        Some((j, t)) if j == i && t.elapsed() < Duration::from_millis(400)
                    );
                    state.last_click = Some((i, Instant::now()));
                    if twice && i > 0 && i + 1 < n {
                        let mut v = points.to_vec();
                        v.remove(i);
                        state.dragging = None;
                        return Some(Action::publish((self.on_change)(v)).and_capture());
                    }
                    state.dragging = Some(i);
                    return Some(Action::capture());
                }
                if n >= Curve::MAX {
                    return Some(Action::capture());
                }
                // A new point: on the curve if the click is close to it.
                let [x, y] = from_screen(size, at);
                let on_curve = self.curve.eval(x);
                let y = if (to_screen(size, [x, on_curve]).y - at.y).abs() < 14.0 {
                    on_curve
                } else {
                    y
                };
                let mut v = points.to_vec();
                v.push([x, y]);
                v.sort_by(|a, b| a[0].total_cmp(&b[0]));
                let index = v.iter().position(|p| p[0] == x && p[1] == y);
                state.dragging = index;
                state.last_click = index.map(|i| (i, Instant::now()));
                Some(Action::publish((self.on_change)(v)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let i = state.dragging?;
                let local = Point::new(position.x - bounds.x, position.y - bounds.y);
                let mut v = points.to_vec();
                // An inner point dragged well out of the box goes away.
                let outside = local.x < -20.0
                    || local.y < -20.0
                    || local.x > size.width + 20.0
                    || local.y > size.height + 20.0;
                if outside && i > 0 && i + 1 < n {
                    v.remove(i);
                    state.dragging = None;
                    return Some(Action::publish((self.on_change)(v)).and_capture());
                }
                let [x, y] = from_screen(size, local);
                let low = if i == 0 { 0.0 } else { v[i - 1][0] + GAP };
                let high = if i + 1 == n { 1.0 } else { v[i + 1][0] - GAP };
                v[i] = [x.clamp(low, high.max(low)), y];
                Some(Action::publish((self.on_change)(v)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging.take().map(|_| Action::capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let size = bounds.size();
        let palette = theme.extended_palette();
        let mut frame = Frame::new(renderer, size);
        frame.fill_rectangle(Point::ORIGIN, size, palette.background.weak.color);
        // The histogram behind (square root of the counts).
        if let Some(h) = &self.histogram {
            let peak = h[1..255].iter().copied().max().unwrap_or(1).max(1) as f32;
            let shade = Color {
                a: 0.35,
                ..palette.background.strong.color
            };
            let path = Path::new(|b| {
                b.move_to(Point::new(0.0, size.height));
                for (i, &c) in h.iter().enumerate() {
                    let y = size.height * (1.0 - ((c as f32).sqrt() / peak.sqrt()).min(1.0));
                    b.line_to(Point::new(i as f32 / 255.0 * size.width, y));
                }
                b.line_to(Point::new(size.width, size.height));
                b.close();
            });
            frame.fill(&path, shade);
        }
        // Grid in quarters, and the diagonal.
        let grid = Stroke::default().with_color(palette.background.strong.color);
        for k in 1..4 {
            let f = k as f32 / 4.0;
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
            &Path::line(to_screen(size, [0.0, 0.0]), to_screen(size, [1.0, 1.0])),
            grid,
        );
        let path = Path::new(|b| {
            b.move_to(to_screen(size, [0.0, self.curve.eval(0.0)]));
            for i in 1..=128 {
                let x = i as f32 / 128.0;
                b.line_to(to_screen(size, [x, self.curve.eval(x)]));
            }
        });
        frame.stroke(
            &path,
            Stroke::default().with_width(2.0).with_color(self.colour),
        );
        for (i, p) in self.curve.points().iter().enumerate() {
            let at = to_screen(size, *p);
            let active = state.dragging == Some(i);
            frame.fill(
                &Path::circle(at, if active { 6.0 } else { 4.5 }),
                if active { self.colour } else { Color::WHITE },
            );
            frame.stroke(
                &Path::circle(at, if active { 6.0 } else { 4.5 }),
                Stroke::default().with_width(2.0).with_color(self.colour),
            );
        }
        // Input and output of the point being moved, as levels 0-255.
        if let Some(p) = state.dragging.and_then(|i| self.curve.points().get(i)) {
            frame.fill_text(canvas::Text {
                content: format!("Ingresso {:.0} → Uscita {:.0}", p[0] * 255.0, p[1] * 255.0),
                position: Point::new(6.0, 4.0),
                color: palette.background.base.text,
                size: 12.0.into(),
                ..canvas::Text::default()
            });
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
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}
