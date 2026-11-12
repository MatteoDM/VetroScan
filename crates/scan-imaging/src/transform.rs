// SPDX-License-Identifier: GPL-2.0-or-later
//! Geometry edits of a scan: quarter turns and mirroring, fine
//! straightening, cropping and resizing.
//!
//! A [`Transform`] is only a description: the samples of the scan are never
//! changed, the edits are worked out again from them each time
//! ([`Transform::apply`]), on the small copy while editing and at full size
//! when saving. The order is fixed: mirror, quarter turns, straightening
//! about the centre (the frame keeps its size, what comes in from outside
//! is `fill`), crop, resize.
//!
//! The crop is in fractions of the turned frame, so the same transform
//! fits the small copy and the full scan. Straightening samples with a
//! Catmull-Rom cubic, resizing with Lanczos (3 lobes): sharp, without the
//! blur of bilinear or the staircases of the nearest pixel.

/// A rectangle in fractions (0-1) of the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Crop {
    /// The whole frame.
    pub const FULL: Crop = Crop {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    /// The same rectangle kept inside the frame, at least a pixel's worth
    /// wide and high.
    pub fn clamped(self) -> Crop {
        let x = self.x.clamp(0.0, 0.999);
        let y = self.y.clamp(0.0, 0.999);
        Crop {
            x,
            y,
            width: self.width.clamp(0.001, 1.0 - x),
            height: self.height.clamp(0.001, 1.0 - y),
        }
    }
}

/// The geometry edits of an image. [`Transform::default`] changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Transform {
    /// Mirrored left to right (before turning).
    pub mirror: bool,
    /// Quarter turns clockwise, 0-3.
    pub turns: u8,
    /// Straightening in degrees, clockwise, −45 to 45.
    pub angle: f32,
    /// What is kept, in fractions of the turned frame; `None` is the whole
    /// frame, or when straightening the largest rectangle of the same
    /// proportions without empty corners.
    pub crop: Option<Crop>,
    /// The size of the result relative to the crop (across, down); `None`
    /// keeps it.
    pub scale: Option<[f32; 2]>,
    /// Resolution written in the result; `None` keeps the physical size
    /// (the scan's resolution times the scale).
    pub dpi: Option<u16>,
}

/// Most straightening, either way.
pub const MAX_ANGLE: f32 = 45.0;

impl Transform {
    /// Whether it changes nothing.
    pub fn is_identity(&self) -> bool {
        *self == Transform::default()
    }

    /// Whether the orientation, straightening and crop are the original
    /// ones (only the size or the resolution may change).
    pub fn keeps_frame(&self) -> bool {
        !self.mirror && self.turns % 4 == 0 && self.angle == 0.0 && self.crop.is_none()
    }

    /// Size of the frame after the quarter turns, for an image
    /// `width` × `height`.
    pub fn oriented_size(&self, width: usize, height: usize) -> (usize, usize) {
        if self.turns % 2 == 1 {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// The crop in force: the one chosen, or the largest without empty
    /// corners when straightening.
    pub fn effective_crop(&self, width: usize, height: usize) -> Crop {
        match self.crop {
            Some(c) => c.clamped(),
            None if self.angle != 0.0 => {
                let (w, h) = self.oriented_size(width, height);
                inscribed(w as f32, h as f32, self.angle)
            }
            None => Crop::FULL,
        }
    }

    /// The crop in pixels of the turned frame: left, top, width, height.
    pub fn crop_pixels(&self, width: usize, height: usize) -> (usize, usize, usize, usize) {
        let (w, h) = self.oriented_size(width, height);
        if w == 0 || h == 0 {
            // An empty image: nothing to crop.
            return (0, 0, 0, 0);
        }
        let c = self.effective_crop(width, height);
        let x = ((c.x * w as f32).round() as usize).min(w.saturating_sub(1));
        let y = ((c.y * h as f32).round() as usize).min(h.saturating_sub(1));
        let cw = ((c.width * w as f32).round() as usize).clamp(1, w - x);
        let ch = ((c.height * h as f32).round() as usize).clamp(1, h - y);
        (x, y, cw, ch)
    }

    /// Size of the result for an image `width` × `height`.
    pub fn output_size(&self, width: usize, height: usize) -> (usize, usize) {
        let (_, _, cw, ch) = self.crop_pixels(width, height);
        if cw == 0 || ch == 0 {
            return (0, 0);
        }
        match self.scale {
            Some([sx, sy]) => (
                ((cw as f32 * sx).round() as usize).max(1),
                ((ch as f32 * sy).round() as usize).max(1),
            ),
            None => (cw, ch),
        }
    }

    /// Resolution of the result of an image `width` × `height` at `dpi`.
    pub fn output_dpi(&self, width: usize, height: usize, dpi: u16) -> u16 {
        if let Some(d) = self.dpi {
            return d.max(1);
        }
        let (_, _, cw, _) = self.crop_pixels(width, height);
        if cw == 0 {
            return dpi.max(1);
        }
        let (ow, _) = self.output_size(width, height);
        ((f32::from(dpi) * ow as f32 / cw as f32).round() as u16).max(1)
    }

    /// A quarter turn more, clockwise or not; the crop and the size turn
    /// with the image.
    pub fn rotate(&mut self, clockwise: bool) {
        self.turns = (self.turns + if clockwise { 1 } else { 3 }) % 4;
        self.crop = self.crop.map(|c| {
            if clockwise {
                Crop {
                    x: 1.0 - c.y - c.height,
                    y: c.x,
                    width: c.height,
                    height: c.width,
                }
            } else {
                Crop {
                    x: c.y,
                    y: 1.0 - c.x - c.width,
                    width: c.height,
                    height: c.width,
                }
            }
        });
        self.scale = self.scale.map(|[x, y]| [y, x]);
    }

    /// The same orientation and straightening with the whole frame kept
    /// and no resizing: what is shown while choosing the crop.
    pub fn frame_only(&self) -> Transform {
        Transform {
            crop: Some(Crop::FULL),
            scale: None,
            dpi: None,
            ..*self
        }
    }

    /// The point of the original image `width` × `height` under point
    /// (`x`, `y`) of the result (pixel edges at whole numbers).
    pub fn source_point(&self, width: usize, height: usize, x: f32, y: f32) -> (f32, f32) {
        let (cx, cy, cw, ch) = self.crop_pixels(width, height);
        let (ow, oh) = self.output_size(width, height);
        let (fw, fh) = self.oriented_size(width, height);
        let (fw, fh) = (fw as f32, fh as f32);
        // In the straightened frame.
        let px = cx as f32 + x * cw as f32 / ow as f32;
        let py = cy as f32 + y * ch as f32 / oh as f32;
        // In the turned frame.
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let (vx, vy) = (px - fw / 2.0, py - fh / 2.0);
        let (ox, oy) = (
            vx * cos + vy * sin + fw / 2.0,
            -vx * sin + vy * cos + fh / 2.0,
        );
        // In the original.
        let (w, h) = (width as f32, height as f32);
        let (sx, sy) = match self.turns % 4 {
            0 => (ox, oy),
            1 => (oy, h - ox),
            2 => (w - ox, h - oy),
            _ => (w - oy, ox),
        };
        (if self.mirror { w - sx } else { sx }, sy)
    }

    /// Mirrored left to right as it is shown.
    pub fn mirror_horizontal(&mut self) {
        self.turns = (4 - self.turns % 4) % 4;
        self.mirror = !self.mirror;
        self.angle = -self.angle;
        self.crop = self.crop.map(|c| Crop {
            x: 1.0 - c.x - c.width,
            ..c
        });
    }

    /// Mirrored top to bottom as it is shown.
    pub fn mirror_vertical(&mut self) {
        self.turns = (6 - self.turns % 4) % 4;
        self.mirror = !self.mirror;
        self.angle = -self.angle;
        self.crop = self.crop.map(|c| Crop {
            y: 1.0 - c.y - c.height,
            ..c
        });
    }

    /// The image `width` × `height` with `channels` samples per pixel,
    /// edited; returns the samples and the new size. `fill` is the colour
    /// of what comes in from outside when straightening.
    pub fn apply<S: Sample>(
        &self,
        data: &[S],
        width: usize,
        height: usize,
        channels: usize,
        fill: &[S],
    ) -> (Vec<S>, usize, usize) {
        assert_eq!(data.len(), width * height * channels, "size of the samples");
        assert_eq!(fill.len(), channels, "channels of the fill");
        if self.is_identity() || width == 0 || height == 0 {
            return (data.to_vec(), width, height);
        }
        let frame = Frame::new(self, width, height, channels);
        let (cx, cy, cw, ch) = self.crop_pixels(width, height);
        let mut cut = vec![S::default(); cw * ch * channels];
        if self.angle == 0.0 {
            par_rows(&mut cut, cw * channels, |y, row| {
                for x in 0..cw {
                    let i = frame.index(cx + x, cy + y);
                    row[x * channels..][..channels].copy_from_slice(&data[i..i + channels]);
                }
            });
        } else {
            let (w, h) = (frame.width as f32, frame.height as f32);
            let (sin, cos) = self.angle.to_radians().sin_cos();
            par_rows(&mut cut, cw * channels, |y, row| {
                let mut px = vec![0.0f32; channels];
                for x in 0..cw {
                    // Back from the straightened frame to the turned one.
                    let vx = (cx + x) as f32 + 0.5 - w / 2.0;
                    let vy = (cy + y) as f32 + 0.5 - h / 2.0;
                    let sx = vx * cos + vy * sin + w / 2.0 - 0.5;
                    let sy = -vx * sin + vy * cos + h / 2.0 - 0.5;
                    let out = &mut row[x * channels..][..channels];
                    if frame.cubic(data, sx, sy, &mut px) {
                        for (o, v) in out.iter_mut().zip(&px) {
                            *o = S::from_f32(*v);
                        }
                    } else {
                        out.copy_from_slice(fill);
                    }
                }
            });
        }
        let (ow, oh) = self.output_size(width, height);
        if (ow, oh) == (cw, ch) {
            (cut, cw, ch)
        } else {
            (resize(&cut, cw, ch, channels, ow, oh), ow, oh)
        }
    }
}

/// The largest centred rectangle with the proportions of a `width` ×
/// `height` frame that stays inside it when turned by `angle` degrees.
pub fn inscribed(width: f32, height: f32, angle: f32) -> Crop {
    let (s, c) = angle.abs().to_radians().sin_cos();
    let k = (width / (width * c + height * s))
        .min(height / (width * s + height * c))
        .min(1.0);
    Crop {
        x: (1.0 - k) / 2.0,
        y: (1.0 - k) / 2.0,
        width: k,
        height: k,
    }
}

/// A sample type the edits work on: 8 or 16 bits.
pub trait Sample: Copy + Default + Send + Sync {
    /// The sample as a number.
    fn to_f32(self) -> f32;
    /// The nearest sample to a number (clamped).
    fn from_f32(v: f32) -> Self;
}

impl Sample for u8 {
    fn to_f32(self) -> f32 {
        f32::from(self)
    }
    fn from_f32(v: f32) -> Self {
        v.round().clamp(0.0, 255.0) as u8
    }
}

impl Sample for u16 {
    fn to_f32(self) -> f32 {
        f32::from(self)
    }
    fn from_f32(v: f32) -> Self {
        v.round().clamp(0.0, 65535.0) as u16
    }
}

/// The turned frame of an image: where its pixels are in the samples.
struct Frame {
    width: usize,
    height: usize,
    channels: usize,
    /// Pixel index of the frame's (0, 0), and its steps across and down.
    origin: isize,
    step_x: isize,
    step_y: isize,
}

impl Frame {
    fn new(t: &Transform, width: usize, height: usize, channels: usize) -> Self {
        let (fw, fh) = t.oriented_size(width, height);
        let (w, h) = (width as isize, height as isize);
        // Pixel of the source under pixel (x, y) of the frame.
        let source = |x: isize, y: isize| -> isize {
            let (sx, sy) = match t.turns % 4 {
                0 => (x, y),
                1 => (y, h - 1 - x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (w - 1 - y, x),
            };
            let sx = if t.mirror { w - 1 - sx } else { sx };
            sy * w + sx
        };
        let origin = source(0, 0);
        Self {
            width: fw,
            height: fh,
            channels,
            origin,
            step_x: source(1, 0) - origin,
            step_y: source(0, 1) - origin,
        }
    }

    /// Index in the samples of pixel (x, y) of the frame.
    fn index(&self, x: usize, y: usize) -> usize {
        (self.origin + x as isize * self.step_x + y as isize * self.step_y) as usize * self.channels
    }

    /// The samples at a point of the frame (pixel centres at whole
    /// numbers), Catmull-Rom cubic; false outside the frame.
    fn cubic<S: Sample>(&self, data: &[S], x: f32, y: f32, out: &mut [f32]) -> bool {
        let (w, h) = (self.width as f32, self.height as f32);
        if !(-0.5..w - 0.5).contains(&x) || !(-0.5..h - 0.5).contains(&y) {
            return false;
        }
        let (x0, y0) = (x.floor(), y.floor());
        let wx = catmull_rom(x - x0);
        let wy = catmull_rom(y - y0);
        out.iter_mut().for_each(|v| *v = 0.0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let clamp_x = |v: isize| v.clamp(0, self.width as isize - 1) as usize;
        let clamp_y = |v: isize| v.clamp(0, self.height as isize - 1) as usize;
        for (j, wy) in wy.iter().enumerate() {
            let yy = clamp_y(y0 - 1 + j as isize);
            for (i, wx) in wx.iter().enumerate() {
                let at = self.index(clamp_x(x0 - 1 + i as isize), yy);
                let k = wx * wy;
                for (c, o) in out.iter_mut().enumerate() {
                    *o += k * data[at + c].to_f32();
                }
            }
        }
        true
    }
}

/// Weights of the four neighbours for a fraction `t` (Catmull-Rom).
fn catmull_rom(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        (-t3 + 2.0 * t2 - t) / 2.0,
        (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0,
        (-3.0 * t3 + 4.0 * t2 + t) / 2.0,
        (t3 - t2) / 2.0,
    ]
}

/// Lanczos kernel with 3 lobes.
fn lanczos3(x: f32) -> f32 {
    let x = x.abs();
    if x < 1e-6 {
        1.0
    } else if x >= 3.0 {
        0.0
    } else {
        let px = std::f32::consts::PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    }
}

/// For each output position, the input positions and their weights.
fn weights(input: usize, output: usize) -> Vec<(usize, Vec<f32>)> {
    let ratio = input as f32 / output as f32;
    // When shrinking the kernel widens, so every input pixel counts.
    let stretch = ratio.max(1.0);
    let support = 3.0 * stretch;
    (0..output)
        .map(|i| {
            let centre = (i as f32 + 0.5) * ratio - 0.5;
            let first = (centre - support).ceil().max(0.0) as usize;
            let last = ((centre + support).floor() as usize).min(input - 1);
            let mut w: Vec<f32> = (first..=last)
                .map(|j| lanczos3((j as f32 - centre) / stretch))
                .collect();
            let sum: f32 = w.iter().sum();
            if sum.abs() > 1e-6 {
                w.iter_mut().for_each(|v| *v /= sum);
            }
            (first, w)
        })
        .collect()
}

/// Half the size: each sample the mean of the 2 × 2 under it (a last odd
/// column or line is left out). For a scan made at twice the resolution
/// wanted: finer than the scanner's own at half, with half the noise.
/// Returns the samples and their width and height.
pub fn halved<S: Sample>(
    data: &[S],
    width: usize,
    height: usize,
    channels: usize,
) -> (Vec<S>, usize, usize) {
    assert_eq!(data.len(), width * height * channels, "size of the samples");
    let (nw, nh) = (width / 2, height / 2);
    let mut out = Vec::with_capacity(nw * nh * channels);
    for y in 0..nh {
        let top = &data[2 * y * width * channels..][..width * channels];
        let bottom = &data[(2 * y + 1) * width * channels..][..width * channels];
        for x in 0..nw {
            for c in 0..channels {
                let i = 2 * x * channels + c;
                let sum = top[i].to_f32()
                    + top[i + channels].to_f32()
                    + bottom[i].to_f32()
                    + bottom[i + channels].to_f32();
                out.push(S::from_f32(sum / 4.0));
            }
        }
    }
    (out, nw, nh)
}

/// The image resized to `nw` × `nh` with Lanczos (3 lobes), across then
/// down.
pub fn resize<S: Sample>(
    data: &[S],
    width: usize,
    height: usize,
    channels: usize,
    nw: usize,
    nh: usize,
) -> Vec<S> {
    assert_eq!(data.len(), width * height * channels, "size of the samples");
    if (nw, nh) == (width, height) {
        return data.to_vec();
    }
    if nw == 0 || nh == 0 || width == 0 || height == 0 {
        return vec![S::default(); nw * nh * channels];
    }
    let across = weights(width, nw);
    let mut mid = vec![0.0f32; nw * height * channels];
    par_rows(&mut mid, nw * channels, |y, row| {
        let line = &data[y * width * channels..][..width * channels];
        for (x, (first, w)) in across.iter().enumerate() {
            for c in 0..channels {
                let mut sum = 0.0;
                for (k, wk) in w.iter().enumerate() {
                    sum += wk * line[(first + k) * channels + c].to_f32();
                }
                row[x * channels + c] = sum;
            }
        }
    });
    let down = weights(height, nh);
    let mut out = vec![S::default(); nw * nh * channels];
    par_rows(&mut out, nw * channels, |y, row| {
        let (first, w) = &down[y];
        let mut acc = vec![0.0f32; nw * channels];
        for (k, wk) in w.iter().enumerate() {
            let line = &mid[(first + k) * nw * channels..][..nw * channels];
            for (a, v) in acc.iter_mut().zip(line) {
                *a += wk * v;
            }
        }
        for (o, a) in row.iter_mut().zip(&acc) {
            *o = S::from_f32(*a);
        }
    });
    out
}

/// Fills `out` (rows of `row` samples) by calling `f` with each row's
/// number, on all the processor's cores.
fn par_rows<T: Send>(out: &mut [T], row: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    if row == 0 || out.is_empty() {
        return;
    }
    let rows = out.len() / row;
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(rows.div_ceil(16))
        .max(1);
    if threads == 1 {
        for (y, r) in out.chunks_mut(row).enumerate() {
            f(y, r);
        }
        return;
    }
    let per = rows.div_ceil(threads);
    std::thread::scope(|scope| {
        for (n, chunk) in out.chunks_mut(per * row).enumerate() {
            let f = &f;
            scope.spawn(move || {
                for (y, r) in chunk.chunks_mut(row).enumerate() {
                    f(n * per + y, r);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Format, Image};

    /// An RGB image whose pixels all differ.
    fn sample(w: usize, h: usize) -> Vec<u8> {
        (0..w * h * 3).map(|i| (i * 7 % 251) as u8).collect()
    }

    #[test]
    fn halved_averages_each_square() {
        // 5 × 3 grey: the last column and line are left out.
        let data: Vec<u16> = (0..15).map(|i| i * 1000).collect();
        let (out, w, h) = halved(&data, 5, 3, 1);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, vec![3000, 5000]);
        let rgb = sample(4, 4);
        let (out, w, h) = halved(&rgb, 4, 4, 3);
        assert_eq!((w, h, out.len()), (2, 2, 12));
        let mean = |x: usize, y: usize, c: usize| -> u8 {
            let at = |dx, dy| f32::from(rgb[((2 * y + dy) * 4 + 2 * x + dx) * 3 + c]);
            ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4.0).round() as u8
        };
        assert_eq!(out[(2 + 1) * 3 + 2], mean(1, 1, 2));
    }

    #[test]
    fn identity_changes_nothing() {
        let data = sample(5, 3);
        let (out, w, h) = Transform::default().apply(&data, 5, 3, 3, &[255; 3]);
        assert_eq!((out, w, h), (data, 5, 3));
    }

    #[test]
    fn quarter_turns_match_image_rotation() {
        let data = sample(5, 3);
        let image = Image {
            width: 5,
            height: 3,
            dpi: 300,
            format: Format::Rgb,
            data: data.clone(),
        };
        for turns in 1..4u8 {
            let t = Transform {
                turns,
                ..Transform::default()
            };
            let (out, w, h) = t.apply(&data, 5, 3, 3, &[255; 3]);
            let expected = image.rotated(turns);
            assert_eq!((w, h), (expected.width, expected.height));
            assert_eq!(out, expected.data, "{turns} quarter turns");
        }
    }

    #[test]
    fn turns_and_mirrors_come_back() {
        let data = sample(4, 3);
        let mut t = Transform::default();
        for _ in 0..4 {
            t.rotate(true);
        }
        assert!(t.is_identity());
        t.mirror_horizontal();
        t.mirror_horizontal();
        assert!(t.is_identity());
        // Mirroring left-right and top-bottom is a half turn.
        t.mirror_horizontal();
        t.mirror_vertical();
        let half = Transform {
            turns: 2,
            ..Transform::default()
        };
        assert_eq!(
            t.apply(&data, 4, 3, 3, &[0; 3]),
            half.apply(&data, 4, 3, 3, &[0; 3])
        );
    }

    #[test]
    fn mirror_is_shown_mirrored_after_a_turn() {
        // Mirroring what is shown, after a turn, is the turned image
        // mirrored left to right.
        let data = sample(4, 3);
        let mut t = Transform::default();
        t.rotate(true);
        let (turned, w, h) = t.apply(&data, 4, 3, 3, &[0; 3]);
        t.mirror_horizontal();
        let (mirrored, mw, mh) = t.apply(&data, 4, 3, 3, &[0; 3]);
        assert_eq!((mw, mh), (w, h));
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    mirrored[(y * w + x) * 3..][..3],
                    turned[(y * w + w - 1 - x) * 3..][..3]
                );
            }
        }
    }

    #[test]
    fn crop_turns_with_the_image() {
        let data = sample(8, 6);
        let mut t = Transform {
            crop: Some(Crop {
                x: 0.25,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            }),
            ..Transform::default()
        };
        let (cut, w, h) = t.apply(&data, 8, 6, 3, &[0; 3]);
        assert_eq!((w, h), (4, 3));
        assert_eq!(cut[..3], data[2 * 3..][..3]);
        t.rotate(true);
        let (turned, tw, th) = t.apply(&data, 8, 6, 3, &[0; 3]);
        let expected = Image {
            width: w,
            height: h,
            dpi: 1,
            format: Format::Rgb,
            data: cut,
        }
        .rotated(1);
        assert_eq!((tw, th), (expected.width, expected.height));
        assert_eq!(turned, expected.data);
    }

    #[test]
    fn straightening_keeps_flat_colour_and_leaves_no_corners() {
        let data = vec![100u8; 60 * 40 * 3];
        let t = Transform {
            angle: 7.5,
            ..Transform::default()
        };
        let (out, w, h) = t.apply(&data, 60, 40, 3, &[255; 3]);
        assert!(w < 60 && h < 40);
        assert!((w as f32 / h as f32 - 1.5).abs() < 0.1, "{w} × {h}");
        assert!(out.iter().all(|&v| v == 100), "no fill inside the crop");
        // With the whole frame kept, the corners come in from outside.
        let t = Transform {
            crop: Some(Crop::FULL),
            ..t
        };
        let (out, w, _) = t.apply(&data, 60, 40, 3, &[255; 3]);
        assert_eq!(w, 60);
        assert_eq!(out[0], 255);
    }

    #[test]
    fn straightening_turns_the_right_way() {
        // A bright dot right of centre goes down when turned clockwise.
        let (w, h) = (41, 41);
        let mut data = vec![0u8; w * h];
        data[20 * w + 35] = 255;
        let t = Transform {
            angle: 30.0,
            crop: Some(Crop::FULL),
            ..Transform::default()
        };
        let (out, _, _) = t.apply(&data, w, h, 1, &[0]);
        let brightest = out
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| **v)
            .map(|(i, _)| (i % w, i / w))
            .unwrap();
        // 15 pixels from centre at 30°: about (33, 27).
        assert!(
            (brightest.0 as i32 - 33).abs() <= 1 && (brightest.1 as i32 - 27).abs() <= 1,
            "{brightest:?}"
        );
    }

    #[test]
    fn resize_keeps_flat_colour_and_gradients() {
        let flat = vec![1234u16; 30 * 20 * 3];
        let out = resize(&flat, 30, 20, 3, 13, 7);
        assert_eq!(out.len(), 13 * 7 * 3);
        assert!(out.iter().all(|&v| v == 1234));
        let up = resize(&flat, 30, 20, 3, 71, 45);
        assert!(up.iter().all(|&v| v == 1234));
        // A ramp halved stays a ramp, in order.
        let ramp: Vec<u8> = (0..64).map(|x| (x * 4) as u8).collect();
        let half = resize(&ramp, 64, 1, 1, 32, 1);
        assert!(half.windows(2).all(|p| p[0] <= p[1]), "{half:?}");
        assert!((i32::from(half[16]) - 130).abs() <= 3, "{half:?}");
    }

    #[test]
    fn size_and_resolution_follow_the_scale() {
        let t = Transform {
            scale: Some([0.5, 0.5]),
            ..Transform::default()
        };
        assert_eq!(t.output_size(1000, 600), (500, 300));
        assert_eq!(t.output_dpi(1000, 600, 600), 300);
        let t = Transform {
            dpi: Some(150),
            ..t
        };
        assert_eq!(t.output_dpi(1000, 600, 600), 150);
        let data = vec![9u8; 10 * 6 * 3];
        let t = Transform {
            scale: Some([2.0, 0.5]),
            ..Transform::default()
        };
        let (out, w, h) = t.apply(&data, 10, 6, 3, &[0; 3]);
        assert_eq!((w, h, out.len()), (20, 3, 20 * 3 * 3));
    }

    #[test]
    fn points_of_the_result_lead_back_to_the_original() {
        // Each pixel of a turned, mirrored, cropped and halved image comes
        // from the pixel `source_point` names.
        let (w, h) = (12, 8);
        let data: Vec<u8> = (0..w * h).map(|i| i as u8).collect();
        let mut t = Transform {
            crop: Some(Crop {
                x: 0.25,
                y: 0.125,
                width: 0.5,
                height: 0.75,
            }),
            ..Transform::default()
        };
        t.rotate(true);
        t.mirror_horizontal();
        let (out, ow, oh) = t.apply(&data, w, h, 1, &[0]);
        for y in 0..oh {
            for x in 0..ow {
                let (sx, sy) = t.source_point(w, h, x as f32 + 0.5, y as f32 + 0.5);
                let i = sy.floor() as usize * w + sx.floor() as usize;
                assert_eq!(out[y * ow + x], data[i], "({x}, {y})");
            }
        }
        // Straightened: the centre stays where it is.
        let t = Transform {
            angle: 12.0,
            ..Transform::default()
        };
        let (ow, oh) = t.output_size(100, 60);
        let (sx, sy) = t.source_point(100, 60, ow as f32 / 2.0, oh as f32 / 2.0);
        assert!(
            (sx - 50.0).abs() < 0.6 && (sy - 30.0).abs() < 0.6,
            "{sx} {sy}"
        );
    }

    #[test]
    fn inscribed_rectangle() {
        assert_eq!(inscribed(100.0, 50.0, 0.0), Crop::FULL);
        let c = inscribed(100.0, 100.0, 45.0);
        assert!((c.width - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    }

    #[test]
    fn an_empty_image_is_zero_by_zero() {
        let t = Transform {
            crop: Some(Crop {
                x: 0.1,
                y: 0.1,
                width: 0.5,
                height: 0.5,
            }),
            scale: Some([2.0, 2.0]),
            ..Transform::default()
        };
        assert_eq!(t.crop_pixels(0, 10), (0, 0, 0, 0));
        assert_eq!(t.output_size(0, 10), (0, 0));
        assert_eq!(t.output_size(10, 0), (0, 0));
        assert_eq!(t.output_dpi(0, 0, 300), 300);
    }
}
