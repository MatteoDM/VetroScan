// SPDX-License-Identifier: GPL-2.0-or-later
//! The histogram of the adjusted preview: red, green and blue (or grey),
//! with the share of pixels clipped to black and to white.

use hp3770_core::session::{Format, Image};
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

/// Counts of each level, per channel (one channel for grey).
#[derive(Debug, Clone, Default)]
pub struct Histogram {
    channels: Vec<[u32; 256]>,
    pixels: u64,
}

impl Histogram {
    /// The histogram of an image (black and white has none).
    pub fn of(image: &Image) -> Self {
        let channels = match image.format {
            Format::Rgb => 3,
            Format::Grey => 1,
            Format::Lineart => return Self::default(),
        };
        let mut counts = vec![[0u32; 256]; channels];
        for (i, &v) in image.data.iter().enumerate() {
            counts[i % channels][usize::from(v)] += 1;
        }
        Self {
            channels: counts,
            pixels: (image.data.len() / channels) as u64,
        }
    }

    /// Share of samples at 0 and at 255, in percent (the worst channel).
    pub fn clipped(&self) -> (f32, f32) {
        if self.pixels == 0 {
            return (0.0, 0.0);
        }
        let share = |level: usize| {
            self.channels
                .iter()
                .map(|c| c[level] as f32 / self.pixels as f32 * 100.0)
                .fold(0.0, f32::max)
        };
        (share(0), share(255))
    }

    /// The counts of the mean level of the channels (for the curve).
    pub fn levels(&self) -> Option<[u32; 256]> {
        if self.channels.is_empty() {
            return None;
        }
        let n = self.channels.len() as u32;
        Some(std::array::from_fn(|i| {
            self.channels.iter().map(|c| c[i]).sum::<u32>() / n
        }))
    }

    /// Is there anything to draw.
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }
}

impl<Message> canvas::Program<Message> for Histogram {
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
        let palette = theme.extended_palette();
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), palette.background.weak.color);
        // Square root of the counts: the tall peaks of a page (paper)
        // would flatten everything else.
        let peak = self
            .channels
            .iter()
            .flat_map(|c| c[1..255].iter())
            .copied()
            .max()
            .unwrap_or(1)
            .max(1) as f32;
        let colours: &[Color] = if self.channels.len() == 3 {
            &[
                Color::from_rgba(0.95, 0.25, 0.25, 0.55),
                Color::from_rgba(0.25, 0.85, 0.3, 0.55),
                Color::from_rgba(0.3, 0.45, 1.0, 0.55),
            ]
        } else {
            &[Color::from_rgba(0.6, 0.6, 0.6, 0.8)]
        };
        let (w, h) = (bounds.width, bounds.height);
        for (counts, colour) in self.channels.iter().zip(colours) {
            let path = Path::new(|b| {
                b.move_to(Point::new(0.0, h));
                for (i, &n) in counts.iter().enumerate() {
                    let y = h - h * ((n as f32).sqrt() / peak.sqrt()).min(1.0);
                    b.line_to(Point::new(i as f32 / 255.0 * w, y));
                }
                b.line_to(Point::new(w, h));
                b.close();
            });
            frame.fill(&path, *colour);
        }
        // Clipping marks at both ends.
        let (black, white) = self.clipped();
        let mark = |frame: &mut Frame, x: f32, on: bool| {
            if on {
                frame.fill_rectangle(
                    Point::new(x, 0.0),
                    Size::new(4.0, h),
                    Color::from_rgb(1.0, 0.3, 0.1),
                );
            }
        };
        mark(&mut frame, 0.0, black > 0.5);
        mark(&mut frame, w - 4.0, white > 0.5);
        frame.stroke(
            &Path::rectangle(Point::ORIGIN, bounds.size()),
            Stroke::default().with_color(palette.background.strong.color),
        );
        vec![frame.into_geometry()]
    }
}
