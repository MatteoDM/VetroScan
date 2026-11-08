// SPDX-License-Identifier: GPL-2.0-or-later
//! The print preview of a PDF page: the sheet, the page laid on it exactly
//! as the PDF will have it ([`hp3770_export::Layout::place`]), the margin
//! dashed, and in red the edge of what ends up on paper. What falls
//! outside (a page filling the sheet with other proportions) shows dimmed
//! beyond the red edge, and dragging the page chooses which part is kept.

use hp3770_export::Placed;
use iced::mouse;
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::viewer::Picture;

/// The preview of one page on its sheet.
pub struct Sheet<'a, Message> {
    picture: &'a Picture,
    placed: Placed,
    /// The margin, points (dashed when not zero).
    margin: f64,
    focus: [f32; 2],
    on_focus: fn([f32; 2]) -> Message,
    caption: String,
}

/// A drag of the page: where it started and the part kept then.
#[derive(Debug, Default)]
pub struct Drag(Option<(Point, [f32; 2])>);

/// The preview widget.
pub fn sheet<'a, Message: 'a>(
    picture: &'a Picture,
    placed: Placed,
    margin: f64,
    focus: [f32; 2],
    on_focus: fn([f32; 2]) -> Message,
    caption: String,
) -> Element<'a, Message> {
    canvas::Canvas::new(Sheet {
        picture,
        placed,
        margin,
        focus,
        on_focus,
        caption,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Room left around the sheet, pixels (the cards of the viewer sit there).
const ROOM: f32 = 64.0;

impl<Message> Sheet<'_, Message> {
    /// Pixels per point and the screen position of the sheet's top left.
    fn scale(&self, size: Size) -> (f32, Point) {
        let (pw, ph) = (self.placed.page.0 as f32, self.placed.page.1 as f32);
        let s = ((size.width - 2.0 * ROOM) / pw)
            .min((size.height - 2.0 * ROOM) / ph)
            .max(0.01);
        let origin = Point::new((size.width - pw * s) / 2.0, (size.height - ph * s) / 2.0);
        (s, origin)
    }

    /// A rectangle of the page (points, origin bottom left) on the screen.
    fn screen(&self, size: Size, [x, y, w, h]: [f64; 4]) -> Rectangle {
        let (s, o) = self.scale(size);
        let ph = self.placed.page.1;
        Rectangle {
            x: o.x + x as f32 * s,
            y: o.y + (ph - y - h) as f32 * s,
            width: w as f32 * s,
            height: h as f32 * s,
        }
    }

    /// What is kept of the page on paper: the clip, or the whole page.
    fn kept(&self) -> [f64; 4] {
        self.placed.clip.unwrap_or(self.placed.image)
    }

    /// How far, in points, the page overflows what is kept, across and down.
    fn overflow(&self) -> (f64, f64) {
        let [_, _, w, h] = self.placed.image;
        let [_, _, kw, kh] = self.kept();
        ((w - kw).max(0.0), (h - kh).max(0.0))
    }

    fn draggable(&self) -> bool {
        let (ox, oy) = self.overflow();
        ox > 0.5 || oy > 0.5
    }
}

impl<Message> canvas::Program<Message> for Sheet<'_, Message> {
    type State = Drag;

    fn update(
        &self,
        drag: &mut Drag,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let size = bounds.size();
        let at = cursor
            .position()
            .map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                if !self.draggable() || !self.screen(size, self.placed.image).contains(p) {
                    return None;
                }
                drag.0 = Some((p, self.focus));
                Some(Action::capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let ((start, focus), p) = (drag.0?, at?);
                let (s, _) = self.scale(size);
                let (ox, oy) = self.overflow();
                // Dragging the page right shows more of its left part.
                let along = |moved: f32, over: f64, from: f32| {
                    if over > 0.5 {
                        (from - moved / s / over as f32).clamp(0.0, 1.0)
                    } else {
                        from
                    }
                };
                let new = [
                    along(p.x - start.x, ox, focus[0]),
                    along(p.y - start.y, oy, focus[1]),
                ];
                Some(Action::publish((self.on_focus)(new)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                drag.0.take()?;
                Some(Action::capture())
            }
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
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        let palette = theme.extended_palette();
        let page = self.screen(size, [0.0, 0.0, self.placed.page.0, self.placed.page.1]);
        // The sheet, with a shadow.
        frame.fill_rectangle(
            page.position() + Vector::new(0.0, 4.0),
            page.size(),
            Color::from_rgba(0.0, 0.0, 0.0, 0.25),
        );
        frame.fill_rectangle(page.position(), page.size(), Color::WHITE);
        // The page laid on it, all of it, beyond the sheet too.
        let image = self.screen(size, self.placed.image);
        frame.draw_image(image, &self.picture.handle);
        // What does not reach the paper, dimmed: outside what is kept.
        let kept = self.screen(size, self.kept());
        let dim = Color::from_rgba(0.06, 0.07, 0.09, 0.62);
        let (l, t) = (image.x.min(kept.x), image.y.min(kept.y));
        let (r, b) = (
            (image.x + image.width).max(kept.x + kept.width),
            (image.y + image.height).max(kept.y + kept.height),
        );
        for rect in [
            Rectangle::new(Point::new(l, t), Size::new(r - l, kept.y - t)),
            Rectangle::new(
                Point::new(l, kept.y + kept.height),
                Size::new(r - l, b - kept.y - kept.height),
            ),
            Rectangle::new(Point::new(l, kept.y), Size::new(kept.x - l, kept.height)),
            Rectangle::new(
                Point::new(kept.x + kept.width, kept.y),
                Size::new(r - kept.x - kept.width, kept.height),
            ),
        ] {
            if rect.width > 0.0 && rect.height > 0.0 {
                frame.fill_rectangle(rect.position(), rect.size(), dim);
            }
        }
        // The margin, dashed.
        if self.margin > 0.0 {
            let m = self.margin;
            let (pw, ph) = self.placed.page;
            let guide = self.screen(size, [m, m, pw - 2.0 * m, ph - 2.0 * m]);
            frame.stroke(
                &Path::rectangle(guide.position(), guide.size()),
                Stroke {
                    line_dash: canvas::LineDash {
                        segments: &[5.0, 4.0],
                        offset: 0,
                    },
                    ..Stroke::default()
                        .with_width(1.0)
                        .with_color(Color::from_rgb(0.62, 0.65, 0.7))
                },
            );
        }
        // In red, the edge of what ends up on paper, a little outside it so
        // that the page does not hide it.
        let edge = Rectangle {
            x: kept.x - 1.5,
            y: kept.y - 1.5,
            width: kept.width + 3.0,
            height: kept.height + 3.0,
        };
        frame.stroke(
            &Path::rectangle(edge.position(), edge.size()),
            Stroke::default()
                .with_width(3.0)
                .with_color(Color::from_rgb(0.92, 0.15, 0.15)),
        );
        // What it is, under the sheet.
        let hint = if self.draggable() {
            format!(
                "{} · trascina la pagina per scegliere cosa tenere",
                self.caption
            )
        } else {
            self.caption.clone()
        };
        frame.fill_text(canvas::Text {
            content: hint,
            position: Point::new(size.width / 2.0, page.y + page.height + 18.0),
            color: palette.background.base.text,
            size: 13.0.into(),
            align_x: iced::widget::text::Alignment::Center,
            align_y: iced::alignment::Vertical::Center,
            ..canvas::Text::default()
        });
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        drag: &Drag,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if drag.0.is_some() {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_in(bounds) {
            Some(p)
                if self.draggable()
                    && self.screen(bounds.size(), self.placed.image).contains(p) =>
            {
                mouse::Interaction::Grab
            }
            _ => mouse::Interaction::default(),
        }
    }
}
