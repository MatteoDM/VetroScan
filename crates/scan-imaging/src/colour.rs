// SPDX-License-Identifier: GPL-2.0-or-later
//! A scanner's colours brought nearer to the true ones with a 3x3 matrix.
//!
//! A scanner sees colours through its own lamp, filters and sensor: from
//! film, also through the film's dyes, which each absorb a little of the
//! other channels' light. A matrix mixes the three channels back: each
//! channel out is a sum of the three in. The matrix comes from whoever
//! knows the scanner (its driver, a profile measured on a target); nothing
//! here holds one of its own, and with none nothing changes.

/// A colour matrix, and the encoding of the values it works on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourMatrix {
    /// Row by row: red out from red, green and blue in, then green, then
    /// blue. Rows adding up to 1 leave grey grey.
    pub rows: [f32; 9],
    /// The gamma of the values it is applied to: 1 on linear light, 2.2
    /// on the values as displayed.
    pub gamma: f32,
}

impl ColourMatrix {
    /// Applies the matrix to linear 16-bit RGB `samples`: each pixel
    /// raised to 1 / gamma, mixed, clipped to the range and brought back to
    /// linear.
    pub fn apply(&self, samples: &mut [u16]) {
        let full = f32::from(u16::MAX);
        let gamma = if self.gamma > 0.0 { self.gamma } else { 1.0 };
        let encode: Vec<f32> = (0..=u16::MAX)
            .map(|v| (f32::from(v) / full).powf(1.0 / gamma))
            .collect();
        let m = &self.rows;
        for p in samples.chunks_exact_mut(3) {
            let v = [0, 1, 2].map(|c| encode[usize::from(p[c])]);
            for (c, out) in p.iter_mut().enumerate() {
                let x = m[c * 3] * v[0] + m[c * 3 + 1] * v[1] + m[c * 3 + 2] * v[2];
                *out = (x.clamp(0.0, 1.0).powf(gamma) * full).round() as u16;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

    #[test]
    fn the_identity_changes_nothing() {
        let mut samples = vec![0, 1000, 65535, 30000, 12345, 500];
        let before = samples.clone();
        ColourMatrix {
            rows: IDENTITY,
            gamma: 2.2,
        }
        .apply(&mut samples);
        // Within the rounding of the gamma's round trip.
        for (a, b) in samples.iter().zip(&before) {
            assert!(a.abs_diff(*b) <= 1, "{a} against {b}");
        }
    }

    #[test]
    fn grey_stays_grey_and_colours_spread() {
        let rows = [1.4, -0.2, -0.2, -0.1, 1.2, -0.1, -0.2, -0.2, 1.4];
        for gamma in [1.0, 2.2] {
            let mut samples = vec![20000, 20000, 20000, 30000, 20000, 10000];
            ColourMatrix { rows, gamma }.apply(&mut samples);
            assert!(samples[0].abs_diff(20000) <= 2 && samples[2].abs_diff(20000) <= 2);
            // The red pixel redder, its blue less.
            assert!(samples[3] > 30000 && samples[5] < 10000, "{samples:?}");
        }
    }

    #[test]
    fn values_beyond_the_range_are_clipped() {
        let mut samples = vec![65535, 0, 0];
        ColourMatrix {
            rows: [2.0, 0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            gamma: 1.0,
        }
        .apply(&mut samples);
        assert_eq!(samples, vec![65535, 0, 0]);
    }
}
