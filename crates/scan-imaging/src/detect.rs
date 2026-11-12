// SPDX-License-Identifier: GPL-2.0-or-later
//! Finding what lies on the glass in a preview: photos, cards, pages.
//!
//! The lid is white and even, so anything that differs from it in colour
//! or that has edges is an object. The preview is cut into blocks of
//! about 0.5 mm; a block is part of an object when its colour is far
//! from the lid's or when it is on an edge. Gaps are closed, holes filled
//! (a photo with a white sky is still one photo, as long as its outline is
//! seen), and each connected group large enough becomes an object. Its
//! outline is fitted with the smallest rectangle at any angle, so a photo
//! laid slightly askew comes out straight ([`Image::straightened`]).
//!
//! A white sheet on the white lid is seen only through the faint shadow of
//! its edges: a document is found when those show, not always.

use crate::{Format, Image};

/// Something found on the glass: a rectangle at an angle, in pixels of
/// the image searched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Found {
    /// Centre.
    pub cx: f32,
    pub cy: f32,
    /// Sides: along the direction of `angle`, and across it.
    pub width: f32,
    pub height: f32,
    /// Turn in degrees (clockwise on screen, -45 to 45) of the object.
    pub angle: f32,
}

impl Found {
    /// The four corners, clockwise from the top left.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let (s, c) = self.angle.to_radians().sin_cos();
        let (hw, hh) = (self.width / 2.0, self.height / 2.0);
        [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
            .map(|(u, v)| [self.cx + u * c - v * s, self.cy + u * s + v * c])
    }

    /// The upright rectangle around it: left, top, width, height.
    pub fn bounds(&self) -> [f32; 4] {
        let p = self.corners();
        let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for [x, y] in p {
            l = l.min(x);
            t = t.min(y);
            r = r.max(x);
            b = b.max(y);
        }
        [l, t, r - l, b - t]
    }

    /// The same object in an image `factor` times larger, moved by
    /// (`dx`, `dy`) pixels of that image.
    pub fn scaled(&self, factor: f32, dx: f32, dy: f32) -> Found {
        Found {
            cx: self.cx * factor + dx,
            cy: self.cy * factor + dy,
            width: self.width * factor,
            height: self.height * factor,
            angle: self.angle,
        }
    }

    /// Does it contain the point.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let (s, c) = self.angle.to_radians().sin_cos();
        let (dx, dy) = (x - self.cx, y - self.cy);
        let u = dx * c + dy * s;
        let v = -dx * s + dy * c;
        u.abs() <= self.width / 2.0 && v.abs() <= self.height / 2.0
    }
}

/// Smallest side of an object, in millimetres.
pub const MIN_SIDE_MM: f32 = 15.0;

/// Below this tilt (degrees) an object is taken as straight: a fit of a
/// few blocks is not more precise than that.
const STRAIGHT: f32 = 0.4;

/// The objects on the glass in `image` (an RGB or grey preview of the
/// whole glass, linear or not), in reading order: top to bottom, left to
/// right.
pub fn objects(image: &Image) -> Vec<Found> {
    let channels = match image.format {
        Format::Rgb => 3,
        Format::Grey => 1,
        Format::Lineart => return Vec::new(),
    };
    if image.data.len() != image.width * image.height * channels {
        return Vec::new();
    }
    // Blocks of about 0.5 mm.
    let block = (usize::from(image.dpi) / 50).max(1);
    let (bw, bh) = (image.width / block, image.height / block);
    if bw < 8 || bh < 8 {
        return Vec::new();
    }
    // Mean colour of each block, and how much its brightness varies inside
    // it (texture: a shadow has none, a photo has some).
    let mut mean = vec![[0f32; 3]; bw * bh];
    let mut texture = vec![0f32; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let mut sum = [0u32; 3];
            let (mut luma, mut luma2) = (0f32, 0f32);
            for y in by * block..(by + 1) * block {
                let row = (y * image.width + bx * block) * channels;
                for x in 0..block {
                    let mut l = 0u32;
                    for (c, s) in sum.iter_mut().enumerate() {
                        let v = u32::from(image.data[row + x * channels + c.min(channels - 1)]);
                        *s += v;
                        l += v;
                    }
                    let l = l as f32 / 3.0;
                    luma += l;
                    luma2 += l * l;
                }
            }
            let n = (block * block) as f32;
            mean[by * bw + bx] = sum.map(|s| s as f32 / n);
            let m = luma / n;
            texture[by * bw + bx] = (luma2 / n - m * m).max(0.0).sqrt();
        }
    }
    // The lid: the commonest colour along the edges of the glass (the lid
    // is even, so its colour is a sharp peak; originals touching the edges
    // spread over many colours). Not the brightest colour: white paper is
    // often whiter than an old lid.
    let lid = {
        let ring = |bx: usize, by: usize| {
            let d = bx.min(by).min(bw - 1 - bx).min(bh - 1 - by);
            (2..6).contains(&d)
        };
        // The lid is light and nearly neutral: a green envelope along an
        // edge must not pass for it.
        let brightest = mean
            .iter()
            .map(|m| m[0] + m[1] + m[2])
            .fold(0.0f32, f32::max);
        let lid_like = |m: &[f32; 3]| {
            let sum = m[0] + m[1] + m[2];
            let spread = m.iter().copied().fold(0.0f32, f32::max)
                - m.iter().copied().fold(f32::MAX, f32::min);
            sum > 0.55 * brightest && spread < 0.12 * sum / 3.0 + 6.0
        };
        let mut bins = std::collections::HashMap::<[u8; 3], Vec<usize>>::new();
        for by in 0..bh {
            for bx in 0..bw {
                if ring(bx, by) {
                    let m = mean[by * bw + bx];
                    if !lid_like(&m) {
                        continue;
                    }
                    bins.entry(m.map(|v| (v / 6.0) as u8))
                        .or_default()
                        .push(by * bw + bx);
                }
            }
        }
        let peak = bins
            .values()
            .max_by_key(|v| v.len())
            .cloned()
            .unwrap_or_default();
        // No lid along the edges (the lid open, originals over every
        // edge): nothing to tell the originals from, so nothing found.
        if peak.is_empty() {
            return Vec::new();
        }
        std::array::from_fn::<f32, 3, _>(|c| {
            peak.iter().map(|&i| mean[i][c]).sum::<f32>() / peak.len() as f32
        })
    };
    let level = (lid[0] + lid[1] + lid[2]) / 3.0;
    // Differences from the lid, and edges (difference between
    // neighbours), both against the brightness of the lid.
    let far = 0.14 * level;
    let edge = 0.08 * level;
    let diff = |a: &[f32; 3], b: &[f32; 3]| (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max);
    // The shadow of the lid where thick prints keep it up (the narrow
    // gaps between them, a band along their edges): the lid's own colour,
    // only darker, and smooth. It is lid, not an original: else prints a
    // few millimetres apart come out as one.
    let luma = |i: usize| (mean[i][0] + mean[i][1] + mean[i][2]) / 3.0;
    // The lid's colour, as bright or a little darker (lid, shadow, a white
    // sheet, the edge between them).
    let lidish = |i: usize| {
        let (m, l) = (&mean[i], luma(i));
        let tint = (0..3)
            .map(|c| ((m[c] - l) - (lid[c] - level)).abs())
            .fold(0.0, f32::max);
        l > 0.72 * level && tint < 0.03 * level + 2.0
    };
    let shadow = |i: usize| lidish(i) && luma(i) < level && texture[i] < 0.02 * level + 1.5;
    let mut mask = vec![false; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let i = by * bw + bx;
            if shadow(i) {
                continue;
            }
            let m = &mean[i];
            // An edge only counts on the side that is not the lid (else
            // the object would grow by a block).
            let from_lid = diff(m, &lid);
            let mut on = from_lid > far;
            if !on && from_lid > edge / 2.0 && bx > 0 && by > 0 && bx + 1 < bw && by + 1 < bh {
                // Between two blocks of the lid's colour, an edge is a thin
                // dark line (the edge of a white sheet: darker than both
                // sides), not a step (into a shadow, lighter on one side).
                let line = |j: usize, o: usize| {
                    luma(i) + edge / 2.0 < luma(j) && luma(i) + edge / 2.0 < luma(o)
                };
                on = [
                    (i - 1, i + 1),
                    (i + 1, i - 1),
                    (i - bw, i + bw),
                    (i + bw, i - bw),
                ]
                .iter()
                .any(|&(j, o)| {
                    diff(m, &mean[j]) > edge && (!lidish(i) || !lidish(j) || line(j, o))
                });
            }
            mask[i] = on;
        }
    }
    // The outermost blocks: the edge of the glass and of the lid often
    // show a dark line, which is not an object.
    let rim = 2;
    for by in 0..bh {
        for bx in 0..bw {
            if bx < rim || by < rim || bx + rim >= bw || by + rim >= bh {
                mask[by * bw + bx] = false;
            }
        }
    }
    // Close gaps of up to about 1 mm, fill holes, open again.
    let close = 2;
    let mut mask = dilate(&mask, bw, bh, close);
    fill_holes(&mut mask, bw, bh);
    let mask = erode(&mask, bw, bh, close);
    // Also drop specks and thin lines (dust, hairs): erode then dilate.
    let mask = dilate(&erode(&mask, bw, bh, 1), bw, bh, 1);

    let min_side = MIN_SIDE_MM / 25.4 * f32::from(image.dpi) / block as f32;
    let scale = block as f32;
    let mut found: Vec<Found> = components(&mask, bw, bh)
        .into_iter()
        .filter_map(|outline| fit(&outline))
        .filter(|f| f.width >= min_side && f.height >= min_side)
        // Block centres are half a block in from the true edge.
        .map(|f| Found {
            cx: (f.cx + 0.5) * scale,
            cy: (f.cy + 0.5) * scale,
            width: (f.width + 1.0) * scale,
            height: (f.height + 1.0) * scale,
            angle: f.angle,
        })
        .map(|f| refine(image, channels, &lid, far, block as f32, f))
        .collect();
    reading_order(&mut found);
    found
}

/// Sorts in reading order: rows of objects side by side, that is
/// overlapping the row's topmost object for at least half the height of
/// the lower one (a postcard askew reaches a little into the row under it
/// without being part of it), then left to right in each row. The rows are
/// decided first, top to bottom, so the order is the same whatever order
/// the objects came in.
pub(crate) fn reading_order(found: &mut [Found]) {
    found.sort_by(|a, b| {
        let (ta, tb) = (a.bounds(), b.bounds());
        ta[1].total_cmp(&tb[1]).then(ta[0].total_cmp(&tb[0]))
    });
    let mut rows = Vec::with_capacity(found.len());
    let mut first: Option<[f32; 4]> = None;
    for f in found.iter() {
        let t = f.bounds();
        let same_row = first.is_some_and(|r| {
            let overlap = (r[1] + r[3]).min(t[1] + t[3]) - r[1].max(t[1]);
            overlap > 0.5 * r[3].min(t[3])
        });
        if !same_row {
            first = Some(t);
        }
        rows.push(
            rows.last()
                .map_or(0, |&n: &usize| n + usize::from(!same_row)),
        );
    }
    let mut keyed: Vec<(usize, Found)> = rows.into_iter().zip(found.iter().copied()).collect();
    keyed.sort_by(|(ra, a), (rb, b)| ra.cmp(rb).then(a.bounds()[0].total_cmp(&b.bounds()[0])));
    for (slot, (_, f)) in found.iter_mut().zip(keyed) {
        *slot = f;
    }
}

/// Each side of `f` moved to where the image really changes from the lid,
/// at pixel precision: the blocks give it to a block or so. Lines parallel
/// to the side are tried from two blocks out to two blocks in; the side
/// is the outermost line mostly unlike the lid. A side that shows no such
/// line (a white sheet seen by its shadow) stays where it was.
fn refine(image: &Image, channels: usize, lid: &[f32; 3], far: f32, block: f32, f: Found) -> Found {
    let (s, c) = f.angle.to_radians().sin_cos();
    let at = |u: f32, v: f32| -> Option<bool> {
        let x = f.cx + u * c - v * s;
        let y = f.cy + u * s + v * c;
        if x < 0.0 || y < 0.0 || x >= image.width as f32 || y >= image.height as f32 {
            return None;
        }
        let i = (y as usize * image.width + x as usize) * channels;
        let p: [f32; 3] = std::array::from_fn(|k| f32::from(image.data[i + k.min(channels - 1)]));
        let d = (0..3).map(|k| (p[k] - lid[k]).abs()).fold(0.0, f32::max);
        // The lid's shadow (see `objects`): its colour, a little darker.
        let level = (lid[0] + lid[1] + lid[2]) / 3.0;
        let luma = (p[0] + p[1] + p[2]) / 3.0;
        let tint = (0..3)
            .map(|k| ((p[k] - luma) - (lid[k] - level)).abs())
            .fold(0.0, f32::max);
        let shadow = luma < level && luma > 0.72 * level && tint < 0.03 * level + 4.0;
        Some(d > far && !shadow)
    };
    let (hw, hh) = (f.width / 2.0, f.height / 2.0);
    // Distance from the centre of each side: right, left, bottom, top
    // (sign of the axis, along u or across v).
    let side = |along_u: bool, sign: f32, half: f32, other: f32| -> f32 {
        let reach = 2.0 * block;
        let mut t = half + reach;
        while t >= half - reach {
            let (mut on, mut n) = (0, 0);
            let mut k = -other * 0.8;
            while k <= other * 0.8 {
                let hit = if along_u {
                    at(sign * t, k)
                } else {
                    at(k, sign * t)
                };
                if let Some(h) = hit {
                    n += 1;
                    on += usize::from(h);
                }
                k += 2.0;
            }
            if n > 0 && on * 2 > n {
                return t + 0.5;
            }
            t -= 0.5;
        }
        half
    };
    let right = side(true, 1.0, hw, hh);
    let left = side(true, -1.0, hw, hh);
    let bottom = side(false, 1.0, hh, hw);
    let top = side(false, -1.0, hh, hw);
    let (du, dv) = ((right - left) / 2.0, (bottom - top) / 2.0);
    Found {
        cx: f.cx + du * c - dv * s,
        cy: f.cy + du * s + dv * c,
        width: right + left,
        height: bottom + top,
        angle: f.angle,
    }
}

pub(crate) fn dilate(mask: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    // Square structuring element, done along rows then columns.
    let mut rows = vec![false; mask.len()];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            rows[y * w + x] = mask[y * w + a..=y * w + b].iter().any(|&v| v);
        }
    }
    let mut out = vec![false; mask.len()];
    for y in 0..h {
        let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            out[y * w + x] = (a..=b).any(|yy| rows[yy * w + x]);
        }
    }
    out
}

pub(crate) fn erode(mask: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let inverted: Vec<bool> = mask.iter().map(|&v| !v).collect();
    dilate(&inverted, w, h, r).into_iter().map(|v| !v).collect()
}

/// Sets every block not reachable from the border through empty blocks.
pub(crate) fn fill_holes(mask: &mut [bool], w: usize, h: usize) {
    let mut outside = vec![false; mask.len()];
    let mut stack: Vec<usize> = Vec::new();
    for x in 0..w {
        stack.push(x);
        stack.push((h - 1) * w + x);
    }
    for y in 0..h {
        stack.push(y * w);
        stack.push(y * w + w - 1);
    }
    while let Some(i) = stack.pop() {
        if outside[i] || mask[i] {
            continue;
        }
        outside[i] = true;
        let (x, y) = (i % w, i / w);
        if x > 0 {
            stack.push(i - 1);
        }
        if x + 1 < w {
            stack.push(i + 1);
        }
        if y > 0 {
            stack.push(i - w);
        }
        if y + 1 < h {
            stack.push(i + w);
        }
    }
    for (m, o) in mask.iter_mut().zip(outside) {
        *m = !o;
    }
}

/// The outline (blocks next to an empty one) of each connected group.
pub(crate) fn components(mask: &[bool], w: usize, h: usize) -> Vec<Vec<[f32; 2]>> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        let mut outline = Vec::new();
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let mut border = false;
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx >= w || ny >= h {
                    border = true;
                    continue;
                }
                let j = ny * w + nx;
                if !mask[j] {
                    border = true;
                } else if !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
            if border {
                outline.push([x as f32, y as f32]);
            }
        }
        out.push(outline);
    }
    out
}

/// The smallest rectangle, at any angle, around the points: the convex
/// hull, then every turn from -45° to 45° in steps of 0.1°.
pub(crate) fn fit(points: &[[f32; 2]]) -> Option<Found> {
    let hull = hull(points);
    if hull.len() < 3 {
        return None;
    }
    let extent = |angle: f32| {
        let (s, c) = angle.to_radians().sin_cos();
        let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for &[x, y] in &hull {
            let u = x * c + y * s;
            let v = -x * s + y * c;
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        (u0, u1, v0, v1)
    };
    let area = |angle: f32| {
        let (u0, u1, v0, v1) = extent(angle);
        (u1 - u0) * (v1 - v0)
    };
    let mut best = 0.0f32;
    let mut best_area = area(0.0);
    for step in -450..=450 {
        let a = step as f32 / 10.0;
        let ar = area(a);
        if ar < best_area - 1e-3 {
            best = a;
            best_area = ar;
        }
    }
    // Straight unless clearly askew.
    if best.abs() < STRAIGHT {
        best = 0.0;
    }
    let (u0, u1, v0, v1) = extent(best);
    let (s, c) = best.to_radians().sin_cos();
    let (mu, mv) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
    Some(Found {
        cx: mu * c - mv * s,
        cy: mu * s + mv * c,
        width: u1 - u0,
        height: v1 - v0,
        angle: best,
    })
}

/// Convex hull (Andrew's monotone chain).
fn hull(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut p = points.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut lower: Vec<[f32; 2]> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<[f32; 2]> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The pixels of `found` in an image (`width` × `height`, `channels`
/// samples a pixel), turned straight with bilinear interpolation; `blank`
/// where the image does not reach.
fn resample<T: Copy + Into<f32> + FromF32>(
    data: &[T],
    width: usize,
    height: usize,
    channels: usize,
    found: &Found,
    blank: f32,
) -> Vec<T> {
    let (w, h) = (
        found.width.round().max(1.0) as usize,
        found.height.round().max(1.0) as usize,
    );
    let (s, c) = found.angle.to_radians().sin_cos();
    let mut out = vec![T::from_f32(blank); w * h * channels];
    let (iw, ih) = (width as f32, height as f32);
    for v in 0..h {
        let dv = v as f32 + 0.5 - h as f32 / 2.0;
        for u in 0..w {
            let du = u as f32 + 0.5 - w as f32 / 2.0;
            // Pixel centres: the source point minus half a pixel.
            let x = found.cx + du * c - dv * s - 0.5;
            let y = found.cy + du * s + dv * c - 0.5;
            if x < -0.5 || y < -0.5 || x > iw - 0.5 || y > ih - 0.5 {
                continue;
            }
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            let clampx = |x: f32| x.clamp(0.0, iw - 1.0) as usize;
            let clampy = |y: f32| y.clamp(0.0, ih - 1.0) as usize;
            let (xa, xb, ya, yb) = (clampx(x0), clampx(x0 + 1.0), clampy(y0), clampy(y0 + 1.0));
            let at = |x: usize, y: usize, ch: usize| data[(y * width + x) * channels + ch].into();
            let o = (v * w + u) * channels;
            for ch in 0..channels {
                let top = at(xa, ya, ch) * (1.0 - fx) + at(xb, ya, ch) * fx;
                let bottom = at(xa, yb, ch) * (1.0 - fx) + at(xb, yb, ch) * fx;
                out[o + ch] = T::from_f32(top * (1.0 - fy) + bottom * fy);
            }
        }
    }
    out
}

/// A sample from a value (rounded and clamped).
pub trait FromF32 {
    fn from_f32(v: f32) -> Self;
}

impl FromF32 for u8 {
    fn from_f32(v: f32) -> Self {
        v.round().clamp(0.0, 255.0) as u8
    }
}

impl FromF32 for u16 {
    fn from_f32(v: f32) -> Self {
        v.round().clamp(0.0, 65535.0) as u16
    }
}

/// 16-bit RGB samples (`width` × `height`) of `found`, turned straight:
/// the same as [`Image::straightened`] for the chip's 16-bit samples.
/// Returns the size and the samples.
pub fn straightened16(
    data: &[u16],
    width: usize,
    height: usize,
    found: &Found,
) -> (usize, usize, Vec<u16>) {
    let out = resample(data, width, height, 3, found, 65535.0);
    (
        found.width.round().max(1.0) as usize,
        found.height.round().max(1.0) as usize,
        out,
    )
}

/// 16-bit RGB samples turned clockwise by `quarter_turns` × 90°.
pub fn rotated16(data: &[u16], width: usize, height: usize, quarter_turns: u8) -> Vec<u16> {
    let turns = quarter_turns % 4;
    if turns == 0 {
        return data.to_vec();
    }
    let (w, h) = (width, height);
    let nw = if turns == 2 { w } else { h };
    let mut out = vec![0u16; data.len()];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match turns {
                1 => (h - 1 - y, x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            out[(ny * nw + nx) * 3..][..3].copy_from_slice(&data[(y * w + x) * 3..][..3]);
        }
    }
    out
}

impl Image {
    /// The object `found` (in pixels of this image) cut out and turned
    /// straight, with bilinear interpolation. Parts outside the image are
    /// white. Black and white images are cut without turning.
    pub fn straightened(&self, found: &Found) -> Image {
        let channels = match self.format {
            Format::Rgb => 3,
            Format::Grey => 1,
            Format::Lineart => 0,
        };
        let (w, h) = (
            found.width.round().max(1.0) as usize,
            found.height.round().max(1.0) as usize,
        );
        if channels == 0 {
            let [l, t, ..] = found.bounds();
            return self.cropped(l.max(0.0) as usize, t.max(0.0) as usize, w, h);
        }
        let data = resample(&self.data, self.width, self.height, channels, found, 255.0);
        Image {
            width: w,
            height: h,
            dpi: self.dpi,
            format: self.format,
            data,
        }
    }

    /// A part of the image (clipped to it).
    pub fn cropped(&self, x: usize, y: usize, width: usize, height: usize) -> Image {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let width = width.min(self.width - x);
        let height = height.min(self.height - y);
        let line = |row: usize, from: usize, n: usize| -> &[u8] {
            match self.format {
                Format::Rgb => &self.data[(row * self.width + from) * 3..][..n * 3],
                Format::Grey => &self.data[row * self.width + from..][..n],
                Format::Lineart => {
                    let stride = self.width.div_ceil(8);
                    &self.data[row * stride..][..stride]
                }
            }
        };
        let mut data = Vec::new();
        for row in y..y + height {
            match self.format {
                Format::Lineart => {
                    // Shift the bits so that the part starts a byte.
                    let src = line(row, 0, 0);
                    let mut packed = vec![0u8; width.div_ceil(8)];
                    for i in 0..width {
                        let s = x + i;
                        if src[s / 8] & (0x80 >> (s % 8)) != 0 {
                            packed[i / 8] |= 0x80 >> (i % 8);
                        }
                    }
                    data.extend_from_slice(&packed);
                }
                _ => data.extend_from_slice(line(row, x, width)),
            }
        }
        Image {
            width,
            height,
            dpi: self.dpi,
            format: self.format,
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white glass at 200 dpi with dark rectangles drawn at an angle.
    fn glass(objects: &[Found]) -> Image {
        let (w, h) = (1000, 1300);
        let mut data = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let p = (y * w + x) * 3;
                let inside = objects
                    .iter()
                    .any(|f| f.contains(x as f32 + 0.5, y as f32 + 0.5));
                let v = if inside {
                    // A photo: some texture, a bright patch in the middle.
                    if (x / 40 + y / 40) % 5 == 0 {
                        250
                    } else {
                        90 + ((x * 7 + y * 3) % 60) as u8
                    }
                } else {
                    // The lid, with a little noise.
                    240 + ((x * 13 + y * 7) % 5) as u8
                };
                data[p] = v;
                data[p + 1] = v;
                data[p + 2] = v.saturating_sub(10);
            }
        }
        Image {
            width: w,
            height: h,
            dpi: 200,
            format: Format::Rgb,
            data,
        }
    }

    fn near(a: &Found, b: &Found) {
        assert!(
            (a.cx - b.cx).abs() < 3.0 && (a.cy - b.cy).abs() < 3.0,
            "{a:?} {b:?}"
        );
        assert!(
            (a.width - b.width).abs() < 4.0 && (a.height - b.height).abs() < 4.0,
            "{a:?} {b:?}"
        );
        assert!((a.angle - b.angle).abs() < 0.6, "{a:?} {b:?}");
    }

    #[test]
    fn finds_photos_straight_and_askew_in_reading_order() {
        let truth = [
            Found {
                cx: 250.0,
                cy: 220.0,
                width: 300.0,
                height: 200.0,
                angle: 0.0,
            },
            Found {
                cx: 700.0,
                cy: 260.0,
                width: 320.0,
                height: 240.0,
                angle: 7.0,
            },
            Found {
                cx: 450.0,
                cy: 850.0,
                width: 500.0,
                height: 350.0,
                angle: -3.0,
            },
        ];
        let found = objects(&glass(&truth));
        assert_eq!(found.len(), 3, "{found:?}");
        for (f, t) in found.iter().zip(&truth) {
            near(f, t);
        }
    }

    /// The whole glass at 200 dpi (1700 × 2340) with thick prints on it:
    /// colourful photos, and where the lid cannot come down (the narrow
    /// gaps between prints, a band around each) a smooth grey shadow,
    /// darker than the lid.
    fn glass_with_shadows(objects: &[Found], shadow: u8, band: f32) -> Image {
        let (w, h) = (1700, 2340);
        let mut data = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let p = (y * w + x) * 3;
                let inside = objects.iter().position(|f| f.contains(fx, fy));
                let near = objects.iter().any(|f| {
                    Found {
                        width: f.width + 2.0 * band,
                        height: f.height + 2.0 * band,
                        ..*f
                    }
                    .contains(fx, fy)
                });
                let rgb = match inside {
                    // A photo: leaves and flowers, colour and texture.
                    Some(i) => {
                        let t = ((x * 7 + y * 3 + i * 50) % 70) as u8;
                        [60 + t, 110 + t / 2, 40 + t / 3]
                    }
                    // Lifted lid: shadow, smooth and neutral.
                    None if near => [shadow, shadow, shadow.saturating_sub(4)],
                    None => {
                        let v = 236 + ((x * 13 + y * 7) % 4) as u8;
                        [v, v, v.saturating_sub(4)]
                    }
                };
                data[p..p + 3].copy_from_slice(&rgb);
            }
        }
        Image {
            width: w,
            height: h,
            dpi: 200,
            format: Format::Rgb,
            data,
        }
    }

    /// As on the glass that started this: a postcard askew at the top, two
    /// photos under it touching the bottom and right edges of the glass,
    /// about 3 mm apart, the gaps in shadow.
    fn postcard_and_two_photos() -> [Found; 3] {
        [
            Found {
                cx: 819.0,
                cy: 531.0,
                width: 1150.0,
                height: 780.0,
                angle: -14.8,
            },
            Found {
                cx: 455.0,
                cy: 1700.0,
                width: 850.0,
                height: 1240.0,
                angle: 0.0,
            },
            Found {
                cx: 1302.0,
                cy: 1693.0,
                width: 796.0,
                height: 1254.0,
                angle: 0.0,
            },
        ]
    }

    #[test]
    fn prints_a_few_millimetres_apart_stay_apart_despite_the_shadow() {
        let truth = postcard_and_two_photos();
        for (shadow, band) in [(214, 12.0), (200, 16.0), (186, 20.0)] {
            let found = objects(&glass_with_shadows(&truth, shadow, band));
            assert_eq!(found.len(), 3, "shadow {shadow}, band {band}: {found:?}");
            for (f, t) in found.iter().zip(&truth) {
                // The shadow may add a little; never another print.
                assert!(
                    (f.cx - t.cx).abs() < 25.0 && (f.cy - t.cy).abs() < 25.0,
                    "{f:?} {t:?}"
                );
                assert!(
                    f.width < t.width + 50.0 && f.height < t.height + 50.0,
                    "{f:?} {t:?}"
                );
                assert!(
                    f.width > t.width - 30.0 && f.height > t.height - 30.0,
                    "{f:?} {t:?}"
                );
            }
        }
    }

    #[test]
    fn a_white_sheet_is_still_found_by_the_thin_shadow_of_its_edges() {
        // A sheet as white as the lid, outlined by a 2-pixel dark line.
        let (w, h) = (1000, 1300);
        let sheet = [150usize, 120, 850, 1180];
        let mut data = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let inside = x >= sheet[0] && x < sheet[2] && y >= sheet[1] && y < sheet[3];
                let edge = inside
                    && (x < sheet[0] + 2
                        || x >= sheet[2] - 2
                        || y < sheet[1] + 2
                        || y >= sheet[3] - 2);
                let v = if edge {
                    190
                } else {
                    236 + ((x * 13 + y * 7) % 4) as u8
                };
                data[(y * w + x) * 3..][..3].copy_from_slice(&[v, v, v - 4]);
            }
        }
        let image = Image {
            width: w,
            height: h,
            dpi: 200,
            format: Format::Rgb,
            data,
        };
        let found = objects(&image);
        assert_eq!(found.len(), 1, "{found:?}");
        let b = found[0].bounds();
        assert!(
            (b[0] - 150.0).abs() < 8.0 && (b[2] - 700.0).abs() < 16.0,
            "{b:?}"
        );
    }

    #[test]
    fn empty_glass_and_specks() {
        assert!(objects(&glass(&[])).is_empty());
        // A speck of 1 mm is dust.
        let speck = Found {
            cx: 500.0,
            cy: 500.0,
            width: 8.0,
            height: 8.0,
            angle: 0.0,
        };
        assert!(objects(&glass(&[speck])).is_empty());
    }

    #[test]
    fn straightened_turns_back() {
        let truth = Found {
            cx: 500.0,
            cy: 600.0,
            width: 400.0,
            height: 300.0,
            angle: 10.0,
        };
        let (w, h) = (1000, 1300);
        let mut data = vec![240u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                if truth.contains(x as f32 + 0.5, y as f32 + 0.5) {
                    data[(y * w + x) * 3..][..3].copy_from_slice(&[100, 100, 100]);
                }
            }
        }
        let image = Image {
            width: w,
            height: h,
            dpi: 200,
            format: Format::Rgb,
            data,
        };
        let out = image.straightened(&truth);
        assert_eq!((out.width, out.height), (400, 300));
        // All photo, none of the lid, from two pixels in.
        for y in 2..298 {
            for x in 2..398 {
                let v = out.data[(y * 400 + x) * 3];
                assert!(v <= 102, "lid at {x},{y}: {v}");
            }
        }
        let c = image.cropped(10, 20, 30, 40);
        assert_eq!((c.width, c.height, c.data.len()), (30, 40, 30 * 40 * 3));
        assert_eq!(c.data[0..3], image.data[(20 * 1000 + 10) * 3..][..3]);
    }

    #[test]
    fn the_reading_order_does_not_depend_on_the_order_found() {
        // Staggered: A and B overlap, B and C overlap, A and C do not.
        let at = |cx: f32, cy: f32| Found {
            cx,
            cy,
            width: 100.0,
            height: 100.0,
            angle: 0.0,
        };
        let (a, b, c, d) = (
            at(100.0, 100.0),
            at(300.0, 140.0),
            at(500.0, 180.0),
            at(100.0, 400.0),
        );
        let orders = [
            [a, b, c, d],
            [c, b, a, d],
            [b, d, c, a],
            [d, c, a, b],
            [a, c, d, b],
        ];
        let mut first = None;
        for order in orders {
            let mut v = order.to_vec();
            reading_order(&mut v);
            match &first {
                None => first = Some(v),
                Some(f) => assert_eq!(&v, f),
            }
        }
        // A, then B beside it; C below A's row starts the next one, then D.
        assert_eq!(first.unwrap(), [a, b, c, d]);
    }

    #[test]
    fn without_a_lid_nothing_is_found() {
        // The lid open: black all round, a grey photo in the middle.
        let (w, h) = (400, 400);
        let mut data = vec![0u8; w * h * 3];
        for y in 150..250 {
            for x in 150..250 {
                data[(y * w + x) * 3..][..3].copy_from_slice(&[120, 120, 120]);
            }
        }
        let image = Image {
            width: w,
            height: h,
            dpi: 100,
            format: Format::Rgb,
            data,
        };
        assert!(objects(&image).is_empty());
    }

    #[test]
    fn an_image_shorter_than_its_size_finds_nothing() {
        let image = Image {
            width: 1000,
            height: 1300,
            dpi: 200,
            format: Format::Rgb,
            data: vec![240; 1000 * 1300 * 3 - 3],
        };
        assert!(objects(&image).is_empty());
        assert!(crate::film::frames(&image, crate::film::Film::Slide).is_empty());
    }
}
