// SPDX-License-Identifier: GPL-2.0-or-later
//! Calibration arithmetic of the HP driver.
//!
//! Before every scan the HP driver calibrates the analog front end and the
//! chip's shading correction in four steps, each one reading a small image
//! from the scanner:
//!
//! 1. **Offset** of the ADC, per channel: two probe reads at fixed offsets,
//!    a linear prediction of the offset that puts the optical black pixels at
//!    the target level, then 10-line checks that raise it in small steps
//!    until no channel is below the target ([`predict_offset`],
//!    [`offset_too_dark`]).
//! 2. **Gain** of the variable gain amplifier, per channel, from the
//!    brightest columns of a 10-line image with the lamp on ([`gain_peak`],
//!    [`gain_value`]).
//! 3. **Black shading**: a 10-line image with the lamp off gives one black
//!    level per channel, written to the whole black table
//!    ([`black_level`]).
//! 4. **White shading**: a 24-line image of the white reference gives one
//!    multiplier per pixel and channel ([`white_coefficients`]).
//!
//! The formulas were read in the HP driver (`hpgt3770.dll` 1.0.2.6: gain at
//! `0x1001c2c0`, black shading at `0x100232d0`, white shading at
//! `0x1001d810`) and checked against every scan capture: they give the same
//! registers and tables as the HP driver, value for value. The offset
//! procedure was recovered from the captures (11 scans, every channel). The
//! parameters are in [`CalibrationConfig`].
//!
//! The images handled here are what the chip sends during calibration:
//! 8 bits per sample, RGB interleaved per pixel, lines one after the other.
//!
//! [`Chip::calibrate`] runs the whole procedure on the scanner, in the
//! order of the HP driver, and gives the settings and tables of the scan.

use crate::error::{Error, Result};
use crate::init::Registers;
use crate::motor::{park_registers, Direction, MotorPosition};
use crate::profile::Profile;
use crate::read::Tables;
use crate::setup::{
    lamp_gain_mode, lamp_set_gain_mode, ArrangeLine, GainOffset, Hardware, ScanParams,
};
use crate::transport::{Chip, Transport};

/// Calibration parameters of a model.
///
/// For the 3770 they come from the `[CalibReflective]` section of the HP
/// configuration (the first number where a key holds several).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibrationConfig {
    /// Offsets of the two probe reads (all channels at once).
    pub offset_probes: (u16, u16),
    /// Level the optical black pixels must reach (`OffsetAvgTargetR/G/B`).
    pub offset_target: u32,
    /// Raise of a channel's offset after a check that found it too dark
    /// (`OffsetTuneStep1`), when even and odd pixels are calibrated together.
    pub offset_step: u16,
    /// The same raise at 1200 dpi, where even and odd pixels are checked
    /// separately.
    pub offset_step_even_odd: u16,
    /// Lines of the gain image (`GainHeight`).
    pub gain_lines: u16,
    /// White reference level of each channel, 8-bit (`WRefR/G/B`).
    pub white_ref: [u16; 3],
    /// Fraction of the white reference the gain aims at, in percent
    /// (`GainTargetFactor`).
    pub gain_target_percent: u16,
    /// Lines of the black shading image (`BShadingHeight`).
    pub black_lines: u16,
    /// Correction added to the measured black level, in hundredths of a
    /// level (`U2_S_8B_24D_Cut_R/G/B`).
    pub black_cut_hundredths: [i16; 3],
    /// Lines of the white shading image (`WShadingHeight`).
    pub white_lines: u16,
    /// Vertical resolution of the white shading image.
    pub white_resolution: u16,
    /// Gain of every channel until the gain calibration (`vgag1`).
    pub start_gain: u8,
    /// Second gain of every channel (`vgag2`), never calibrated.
    pub gain2: u8,
    /// Programmable gain of every channel (`pag`), never calibrated.
    pub pga: u8,
}

/// Pixels of each offset probe and check line.
pub const OFFSET_WIDTH: u32 = 100;
/// Lines of each offset probe.
pub const PROBE_LINES: u32 = 1;
/// Lines of each offset check.
pub const CHECK_LINES: u32 = 10;
/// Most offset checks before giving up. The captures need 1 to 3.
pub const MAX_OFFSET_CHECKS: usize = 20;

/// Wait after the transparency adapter's lamp is switched on again for the
/// white shading (not captured: the HP driver never got that far).
const TMA_RELIGHT: std::time::Duration = std::time::Duration::from_secs(15);

/// The lamp a calibration works with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Light {
    /// The flatbed lamp, on the white strip under the lid.
    Flatbed,
    /// The transparency adapter's lamp, on its slot without film.
    Transparency,
}

impl CalibrationConfig {
    /// First pixel of the offset probes and checks
    /// (`OffsetPixelStart_<dpi>`): the optical black pixels at the start of
    /// the sensor. Values of the captures: 7 at 200 and 300 dpi (01, 02,
    /// 04, 07-11), 17 at 600 dpi (05), 15 at 1200 dpi (06, 06b).
    pub fn offset_pixel_start(&self, resolution: u16) -> u32 {
        match resolution {
            0..=300 => 7,
            301..=600 => 17,
            _ => 15,
        }
    }

    /// Number of optical black pixels, at the start of each probe line,
    /// that the offset calibration looks at (`OffsetNPixel_<dpi>`).
    ///
    /// At 1200 dpi the HP driver reads `OffsetNPixel_1200 = 10` pixels of
    /// each parity, so 20 in a row.
    pub fn offset_pixels(&self, resolution: u16) -> usize {
        match resolution {
            0..=100 => 3,
            101..=200 => 3,
            201..=300 => 5,
            301..=600 => 10,
            _ => 20,
        }
    }

    /// Whether this resolution calibrates even and odd pixels separately.
    ///
    /// The HP driver does so at 1200 dpi (the sensor's optical resolution,
    /// also used for 2400 dpi scans).
    pub fn even_odd(&self, resolution: u16) -> bool {
        resolution >= 1200
    }

    /// How many times the pair of offset probes is read. Below 1200 dpi the
    /// HP driver reads the pair twice and uses the second.
    pub fn offset_probe_rounds(&self, resolution: u16) -> usize {
        if self.even_odd(resolution) {
            1
        } else {
            2
        }
    }

    /// Raise applied by an offset check that found a channel too dark.
    pub fn offset_raise(&self, resolution: u16) -> u16 {
        if self.even_odd(resolution) {
            self.offset_step_even_odd
        } else {
            self.offset_step
        }
    }
}

/// An 8-bit RGB image read during calibration.
#[derive(Debug, Clone, Copy)]
pub struct Image<'a> {
    data: &'a [u8],
    width: usize,
}

impl<'a> Image<'a> {
    /// Wraps `data`, `width` pixels per line (3 bytes each). A trailing
    /// partial line is ignored.
    pub fn new(data: &'a [u8], width: usize) -> Self {
        Self { data, width }
    }

    /// Pixels per line.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Complete lines.
    pub fn lines(&self) -> usize {
        match self.width {
            0 => 0,
            w => self.data.len() / (w * 3),
        }
    }

    /// Sample of `channel` (0 = red, 1 = green, 2 = blue) at a pixel.
    pub fn sample(&self, line: usize, pixel: usize, channel: usize) -> u8 {
        self.data[(line * self.width + pixel) * 3 + channel]
    }

    /// Sum of one pixel's samples over every line.
    fn column_sum(&self, pixel: usize, channel: usize) -> u32 {
        (0..self.lines())
            .map(|line| u32::from(self.sample(line, pixel, channel)))
            .sum()
    }
}

/// Which pixels of a line a measure covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pixels {
    /// Every pixel.
    All,
    /// Pixels 0, 2, 4, ...
    Even,
    /// Pixels 1, 3, 5, ...
    Odd,
}

impl Pixels {
    fn contains(self, pixel: usize) -> bool {
        match self {
            Self::All => true,
            Self::Even => pixel % 2 == 0,
            Self::Odd => pixel % 2 == 1,
        }
    }
}

/// Sum and number of the samples of `channel` in the first `count` pixels
/// of every line (the optical black pixels), restricted to `which`.
pub fn dark_sum(image: &Image, channel: usize, count: usize, which: Pixels) -> (u32, u32) {
    let pixels = count.min(image.width());
    let mut sum = 0;
    let mut n = 0;
    for pixel in (0..pixels).filter(|&p| which.contains(p)) {
        sum += image.column_sum(pixel, channel);
        n += image.lines() as u32;
    }
    (sum, n)
}

/// Offset that should bring the black pixels to the target, from the dark
/// sums of the two probe reads.
///
/// `low` and `high` are [`dark_sum`] of the reads at the first and second
/// probe offset (same pixels, one line each). The HP driver interpolates
/// linearly and rounds down. Returns `None` if the two reads are equal (no
/// slope to interpolate).
pub fn predict_offset(
    config: &CalibrationConfig,
    low: (u32, u32),
    high: (u32, u32),
) -> Option<u16> {
    let (lo, hi) = config.offset_probes;
    let (a, n) = (i64::from(low.0), i64::from(low.1));
    let b = i64::from(high.0);
    if b == a || n == 0 {
        return None;
    }
    let target = i64::from(config.offset_target) * n;
    let step = (target - a) * (i64::from(hi) - i64::from(lo));
    let offset = i64::from(lo) + step.div_euclid(b - a);
    // The offset field is 9 bits wide.
    Some(offset.clamp(0, 0x1ff) as u16)
}

/// Whether a 10-line check found the black pixels below the target: the
/// average of the measured samples is strictly below it.
pub fn offset_too_dark(config: &CalibrationConfig, dark: (u32, u32)) -> bool {
    dark.0 < config.offset_target * dark.1
}

/// Brightness measure of the gain image for one channel: the average of
/// the brightest tenth of the column averages.
///
/// Each column is averaged over the lines with integer division, then the
/// `width / 10` largest averages are averaged (as the HP driver does with a
/// histogram of the column averages).
pub fn gain_peak(image: &Image, channel: usize) -> f64 {
    let lines = image.lines() as u32;
    if lines == 0 {
        return 0.0;
    }
    let mut columns: Vec<u32> = (0..image.width())
        .map(|p| image.column_sum(p, channel) / lines)
        .collect();
    columns.sort_unstable_by(|a, b| b.cmp(a));
    let count = image.width() / 10;
    if count == 0 {
        return 0.0;
    }
    let sum: u32 = columns[..count].iter().sum();
    f64::from(sum) / count as f64
}

/// Largest column average of the gain image for one channel. If no channel
/// reaches `offset_target + 5` the HP driver reports a failed calibration.
pub fn gain_max(image: &Image, channel: usize) -> u32 {
    let lines = image.lines() as u32;
    if lines == 0 {
        return 0;
    }
    (0..image.width())
        .map(|p| image.column_sum(p, channel) / lines)
        .max()
        .unwrap_or(0)
}

/// Gain register value (5 bits) for one channel.
///
/// `peak` is [`gain_peak`] of the image read with gain `current`. The
/// amplifier's gain is `40 / (44 - value)`, so the new value is the one
/// that brings `peak` to `white_ref * gain_target_percent / 100`.
pub fn gain_value(config: &CalibrationConfig, channel: usize, peak: f64, current: u8) -> u8 {
    if peak <= 0.0 {
        return 31;
    }
    let factor = f64::from(config.gain_target_percent) * 0.01;
    // Same operation order as the HP driver (8-bit reference depth).
    let target = f64::from(config.white_ref[channel]) * factor * 256.0 * 0.00390625;
    let ratio = target / peak * (40.0 / (31.0 - f64::from(current) + 13.0));
    if ratio <= 0.9090909090909091 {
        return 0;
    }
    let value = (44.0 - 40.0 / ratio).min(31.0);
    value as u8
}

/// Black level of one channel, as written to the black shading table
/// (16 bits, 1/256 level units).
///
/// The HP driver averages each pixel over the lines, averages the pixels
/// (of one parity when even and odd pixels are calibrated separately), adds
/// the configured correction and scales by 256, rounding down.
pub fn black_level(
    config: &CalibrationConfig,
    image: &Image,
    channel: usize,
    which: Pixels,
) -> u16 {
    let lines = image.lines() as f64;
    if lines == 0.0 {
        return 0;
    }
    let mut total = 0.0f64;
    let mut count = 0usize;
    for pixel in (0..image.width()).filter(|&p| which.contains(p)) {
        total += f64::from(image.column_sum(pixel, channel)) / lines;
        count += 1;
    }
    if count == 0 {
        return 0;
    }
    let cut = f64::from(config.black_cut_hundredths[channel]) / 100.0;
    let level = (total / count as f64 + cut).max(0.0);
    (level * 256.0).min(65535.0) as u16
}

/// Average white level of one pixel, from its samples down the white
/// shading image.
///
/// The HP driver averages every run of 4 consecutive lines, drops the 2
/// lowest and the 2 highest of these averages and averages the rest. It
/// does so in one pass: the first `runs - 4` averages are summed while the
/// lowest and highest seen so far are remembered, and each of the last 4
/// is swapped with a remembered one when it is more extreme (or left out).
/// This function reproduces that pass exactly, including which values end
/// up in the sum.
pub fn white_average(samples: &[u8]) -> f64 {
    if samples.len() < 8 {
        let n = samples.len().max(1) as f64;
        return samples.iter().map(|&s| f64::from(s)).sum::<f64>() / n;
    }
    let runs = samples.len() - 3;
    let split = runs - 4;
    // `low` keeps the smallest run averages, `high` the largest.
    let mut low = [255.0f64; 2];
    let mut high = [0.0f64; 2];
    let mut lp = 0;
    let mut hp = 0;
    let mut sum = 0.0f64;
    let max_index = |a: &[f64; 2], start: usize| {
        let mut k = start;
        for i in 0..a.len() {
            if a[i] > a[k] {
                k = i;
            }
        }
        k
    };
    let min_index = |a: &[f64; 2], start: usize| {
        let mut k = start;
        for i in 0..a.len() {
            if a[i] < a[k] {
                k = i;
            }
        }
        k
    };
    for i in 0..runs {
        let w = 0.25
            * (f64::from(samples[i])
                + f64::from(samples[i + 1])
                + f64::from(samples[i + 2])
                + f64::from(samples[i + 3]));
        if i < split {
            if w < low[lp] {
                low[lp] = w;
                lp = max_index(&low, lp);
            }
            if w > high[hp] {
                high[hp] = w;
                hp = min_index(&high, hp);
            }
            sum += w;
        } else {
            if i == split {
                lp = min_index(&low, lp);
                hp = max_index(&high, hp);
            }
            if w > low[lp] {
                sum = sum - low[lp] + w;
                low[lp] = w;
                lp = min_index(&low, lp);
            } else if w < high[hp] {
                sum = sum - high[hp] + w;
                high[hp] = w;
                hp = max_index(&high, hp);
            }
        }
    }
    sum / split as f64
}

/// White shading multipliers of one channel, one per pixel (16 bits,
/// 8192 = gain 1 relative to the white reference).
pub fn white_coefficients(config: &CalibrationConfig, image: &Image, channel: usize) -> Vec<u16> {
    let lines = image.lines();
    let reference = f64::from(config.white_ref[channel]);
    (0..image.width())
        .map(|pixel| {
            let samples: Vec<u8> = (0..lines)
                .map(|l| image.sample(l, pixel, channel))
                .collect();
            let average = white_average(&samples).max(1.0);
            (reference / average * 8192.0).min(65535.0) as u16
        })
        .collect()
}

/// A 16-bit table as the chip receives it (little-endian words).
pub fn table_bytes(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// What the calibration gives the scan: the analog front-end settings and
/// the shading tables of red, green and blue, as the chip takes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calibration {
    /// Offsets and gains.
    pub gain_offset: GainOffset,
    /// Black shading tables.
    pub black: [Vec<u8>; 3],
    /// White shading tables.
    pub white: [Vec<u8>; 3],
}

impl Calibration {
    /// The tables of the scan, with `gamma` (the three gamma tables one
    /// after the other) if the scan applies gamma.
    pub fn tables<'a>(&'a self, gamma: Option<&'a [u8]>) -> Tables<'a> {
        let three = |t: &'a [Vec<u8>; 3]| [&t[0][..], &t[1][..], &t[2][..]];
        Tables {
            gamma,
            black: Some(three(&self.black)),
            white: Some(three(&self.white)),
        }
    }
}

/// Offsets as `[channel][parity]` (even, odd).
type Offsets = [[u16; 2]; 3];

/// Pixels per line of a calibration image of `lines` lines.
fn image_width(data: &[u8], lines: u32) -> usize {
    data.len() / 3 / (lines as usize).max(1)
}

impl<T: Transport> Chip<T> {
    /// Calibrates for the scan `scan` (the HP driver's `Calibration`, as in
    /// capture 09 from 2.358 s): offset probes and checks, gain, black
    /// shading with the lamp off, white shading on the white strip under
    /// the lid, then the carriage back home at base speed.
    ///
    /// `scan` is the scan as the chip reads it: sensor resolution
    /// (`resolution_x`, 1200 for a 2400 dpi scan), window in its pixels.
    /// The calibration reads at that resolution, over the scan's columns.
    /// `init` is the in-memory initial block after [`Chip::prepare_scan`];
    /// the lamp bits of `E946` are kept up to date in it. Its `E954`-`E955`
    /// are left as they are, as the HP driver does: the scan block carries
    /// them (capture 06).
    ///
    /// Not done, unlike the HP driver: nothing is read from or written to
    /// the EEPROM (gain and offset counters). The HP driver calibrates
    /// fully at every scan anyway.
    pub fn calibrate(
        &mut self,
        init: &mut Registers,
        profile: &Profile,
        scan: &ScanParams,
    ) -> Result<Calibration> {
        self.calibrate_with(init, profile, scan, Light::Flatbed)
    }

    /// Calibrates for a scan with the transparency adapter: the same steps
    /// as [`Chip::calibrate`], with the adapter's parameters
    /// ([`crate::profile::Transparency`]), on the slot without film at the
    /// top of the adapter. The carriage first goes there from home (hp3900
    /// moves it to `WStripYPos` before calibrating), the adapter's lamp is
    /// switched off for the black shading and on again for the white, and
    /// the carriage goes home at the end. The adapter's lamp must already
    /// be on and warm ([`Chip::lamp_transparency`]). Not captured: the HP
    /// driver on Windows 11 never gets this far.
    pub fn calibrate_transparency(
        &mut self,
        init: &mut Registers,
        profile: &Profile,
        scan: &ScanParams,
    ) -> Result<Calibration> {
        self.calibrate_with(init, profile, scan, Light::Transparency)
    }

    fn calibrate_with(
        &mut self,
        init: &mut Registers,
        profile: &Profile,
        scan: &ScanParams,
        light: Light,
    ) -> Result<Calibration> {
        let cfg = match light {
            Light::Flatbed => &profile.calibration,
            Light::Transparency => {
                let adapter = profile.transparency.as_ref().ok_or_else(|| {
                    Error::InvalidArgument("this scanner has no transparency adapter".into())
                })?;
                // To the slot, at the speed of the park.
                self.motor_move(
                    &park_registers(init),
                    &profile.park,
                    &MotorPosition {
                        direction: Direction::Forward,
                        e801_bit2: false,
                        steps: adapter.strip_steps,
                    },
                )?;
                &adapter.calibration
            }
        };
        let dpi = scan.resolution_x;
        // The calibration never reads at a lower vertical resolution than
        // the sensor's (2400 at 2400 dpi).
        let y = scan.resolution_y.max(dpi);
        let read = |left, width, height, resolution_y| ScanParams {
            negative: scan.negative,
            depth: 8,
            resolution_x: dpi,
            resolution_y,
            left,
            top: 1,
            width,
            height,
        };
        let still = Hardware {
            black_shading: false,
            white_shading: false,
            gamma: false,
            gamma_size: 0,
            backward: false,
            static_head: true,
            dummy_scan: false,
            compression: false,
            arrange_line: ArrangeLine::None,
            high_resolution: false,
        };
        let gain_offset = |offsets: &Offsets, gain: [u8; 3]| GainOffset {
            even_offset: offsets.map(|o| o[0]),
            odd_offset: offsets.map(|o| o[1]),
            even_fine: [0; 3],
            odd_fine: [0; 3],
            gain,
            gain2: [cfg.gain2; 3],
            pga: [cfg.pga; 3],
        };
        let start_gain = [cfg.start_gain; 3];
        let left = cfg.offset_pixel_start(dpi);
        let dark = cfg.offset_pixels(dpi);

        // 1. Offset probes: one line at each probe offset (the pair twice
        // below 1200 dpi); the last pair predicts the offsets.
        let probe = read(left, OFFSET_WIDTH, PROBE_LINES, y);
        // Each step reads on a copy of the initial block as it is then (the
        // lamp is off for the black strip), with the lamp gain mode of the
        // resolution (capture 06: E954-E955 = 70 00 at 1200 dpi in every
        // read, B0 01 in the initial block). The probes and checks share
        // one copy (see `get_image`).
        let block = |init: &Registers| {
            let mut regs = init.clone();
            lamp_set_gain_mode(&mut regs, dpi, lamp_gain_mode(dpi));
            regs
        };
        let mut work = block(init);
        let mut pair = (Vec::new(), Vec::new());
        for _ in 0..cfg.offset_probe_rounds(dpi) {
            for (offset, out) in [
                (cfg.offset_probes.0, &mut pair.0),
                (cfg.offset_probes.1, &mut pair.1),
            ] {
                let go = gain_offset(&[[offset; 2]; 3], start_gain);
                *out = self.get_image(&mut work, profile, &probe, &still, &go, None)?;
            }
        }
        let width = OFFSET_WIDTH as usize;
        let (low, high) = (Image::new(&pair.0, width), Image::new(&pair.1, width));
        let mut offsets: Offsets = [[0; 2]; 3];
        for (c, offset) in offsets.iter_mut().enumerate() {
            let o = predict_offset(
                cfg,
                dark_sum(&low, c, dark, Pixels::All),
                dark_sum(&high, c, dark, Pixels::All),
            )
            .ok_or_else(|| {
                Error::Calibration(format!(
                    "offset of channel {c}: both probes read the same black level"
                ))
            })?;
            *offset = [o, o];
        }

        // 2. Offset checks: ten lines; every channel (and parity at 1200
        // dpi) still below the target is raised, until none is.
        let check = read(left, OFFSET_WIDTH, CHECK_LINES, y);
        let raise = cfg.offset_raise(dpi);
        // Which pixels each measure covers, and which offsets it raises.
        let measures: &[(Pixels, &[usize])] = if cfg.even_odd(dpi) {
            &[(Pixels::Even, &[0]), (Pixels::Odd, &[1])]
        } else {
            &[(Pixels::All, &[0, 1])]
        };
        let mut settled = false;
        for _ in 0..MAX_OFFSET_CHECKS {
            let go = gain_offset(&offsets, start_gain);
            let data = self.get_image(&mut work, profile, &check, &still, &go, None)?;
            let img = Image::new(&data, width);
            let mut changed = false;
            for (c, channel) in offsets.iter_mut().enumerate() {
                for &(which, parities) in measures {
                    if offset_too_dark(cfg, dark_sum(&img, c, dark, which)) {
                        for &p in parities {
                            channel[p] = (channel[p] + raise).min(0x1ff);
                        }
                        changed = true;
                    }
                }
            }
            if !changed {
                settled = true;
                break;
            }
        }
        if !settled {
            return Err(Error::Calibration(format!(
                "offsets still too dark after {MAX_OFFSET_CHECKS} checks"
            )));
        }

        // 3. Gain, over the columns of the scan.
        let gain_read = read(scan.left, scan.width, u32::from(cfg.gain_lines), y);
        let go = gain_offset(&offsets, start_gain);
        let data = self.get_image(&mut block(init), profile, &gain_read, &still, &go, None)?;
        let img = Image::new(&data, image_width(&data, gain_read.height));
        if (0..3).all(|c| gain_max(&img, c) < cfg.offset_target + 5) {
            return Err(Error::Calibration(
                match light {
                    Light::Flatbed => "the lamp does not light the white strip",
                    Light::Transparency => "the adapter's lamp does not light its slot",
                }
                .into(),
            ));
        }
        let gains: [u8; 3] =
            std::array::from_fn(|c| gain_value(cfg, c, gain_peak(&img, c), start_gain[c]));
        let go = gain_offset(&offsets, gains);

        // 4. Black shading, lamp off. The HP driver writes E946 once more
        // after switching it off (captures 04 and 09).
        match light {
            Light::Flatbed => self.lamp_flatbed(init, false)?,
            Light::Transparency => self.lamp_transparency(init, false)?,
        }
        self.write_byte(0xe946, init.get(0x146))?;
        let black_read = read(scan.left, scan.width, u32::from(cfg.black_lines), y);
        let data = self.get_image(
            &mut block(init),
            profile,
            &black_read,
            &still,
            &go,
            Some(&Tables::default()),
        )?;
        match light {
            Light::Flatbed => self.lamp_flatbed(init, true)?,
            Light::Transparency => {
                self.lamp_transparency(init, true)?;
                std::thread::sleep(TMA_RELIGHT);
            }
        }
        let img = Image::new(&data, image_width(&data, black_read.height));
        let black: [Vec<u8>; 3] = std::array::from_fn(|c| {
            let levels = if cfg.even_odd(dpi) {
                [
                    black_level(cfg, &img, c, Pixels::Even),
                    black_level(cfg, &img, c, Pixels::Odd),
                ]
            } else {
                let l = black_level(cfg, &img, c, Pixels::All);
                [l, l]
            };
            let words: Vec<u16> = (0..img.width()).map(|p| levels[p % 2]).collect();
            table_bytes(&words)
        });

        // 5. White shading: the strip under the lid (the adapter's slot),
        // read with the head moving and the black shading applied.
        let white_read = read(
            scan.left,
            scan.width,
            u32::from(cfg.white_lines),
            cfg.white_resolution,
        );
        let moving = Hardware {
            black_shading: true,
            static_head: false,
            ..still
        };
        let tables = Tables {
            black: Some([&black[0], &black[1], &black[2]]),
            ..Default::default()
        };
        let data = self.get_image(
            &mut block(init),
            profile,
            &white_read,
            &moving,
            &go,
            Some(&tables),
        )?;
        let img = Image::new(&data, image_width(&data, white_read.height));
        let white: [Vec<u8>; 3] =
            std::array::from_fn(|c| table_bytes(&white_coefficients(cfg, &img, c)));

        // 6. Back home at base speed, from the calibration's block. The HP
        // driver first sets the motor current of 1200 dpi, kept in no block.
        if profile.motor_current.is_some() {
            let mut scratch = init.clone();
            self.motor_change(&mut scratch, 0)?;
        }
        self.head_park_home(&park_registers(&block(init)), &profile.base_speed)?;

        Ok(Calibration {
            gain_offset: go,
            black,
            white,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::HP3770;

    fn image(width: usize, lines: usize, f: impl Fn(usize, usize, usize) -> u8) -> Vec<u8> {
        let mut data = Vec::new();
        for l in 0..lines {
            for p in 0..width {
                for c in 0..3 {
                    data.push(f(l, p, c));
                }
            }
        }
        data
    }

    #[test]
    fn offset_prediction_interpolates_and_rounds_down() {
        let cfg = &HP3770.calibration;
        // 5 pixels: 10 per pixel would be 50. At 300 the sum is 48, at 350
        // it is 93: 300 + (50 - 48) * 50 / 45 = 302.2 -> 302.
        assert_eq!(predict_offset(cfg, (48, 5), (93, 5)), Some(302));
        // Already too bright at 300: the offset goes below the probe,
        // rounding towards minus infinity (capture 07: 297).
        assert_eq!(predict_offset(cfg, (51, 5), (92, 5)), Some(298));
        assert_eq!(predict_offset(cfg, (52, 5), (92, 5)), Some(297));
        assert_eq!(predict_offset(cfg, (50, 5), (50, 5)), None);
    }

    #[test]
    fn offset_check_is_strict() {
        let cfg = &HP3770.calibration;
        assert!(offset_too_dark(cfg, (499, 50)));
        assert!(!offset_too_dark(cfg, (500, 50)));
    }

    #[test]
    fn dark_sum_takes_the_first_pixels_of_each_line() {
        let data = image(8, 2, |l, p, c| (l * 100 + p * 10 + c) as u8);
        let img = Image::new(&data, 8);
        // Red, pixels 0..3: line 0 -> 0+10+20, line 1 -> 100+110+120.
        assert_eq!(dark_sum(&img, 0, 3, Pixels::All), (360, 6));
        assert_eq!(dark_sum(&img, 1, 3, Pixels::Odd), (11 + 111, 2));
        assert_eq!(dark_sum(&img, 2, 3, Pixels::Even), (2 + 22 + 102 + 122, 4));
    }

    #[test]
    fn gain_uses_the_brightest_tenth() {
        let cfg = &HP3770.calibration;
        // 20 columns: 18 at 100, 2 at 150 -> the top 2 average 150.
        let data = image(20, 3, |_, p, _| if p < 2 { 150 } else { 100 });
        let img = Image::new(&data, 20);
        assert_eq!(gain_peak(&img, 0), 150.0);
        // Target 245 * 0.80 = 196: 44 - 40 * 150 / 196 = 13.4 -> 13.
        assert_eq!(gain_value(cfg, 0, 150.0, 4), 13);
        // Dim image: capped at 31.
        assert_eq!(gain_value(cfg, 0, 20.0, 4), 31);
        // Very bright image: ratio below 1/1.1 gives 0.
        assert_eq!(gain_value(cfg, 0, 240.0, 4), 0);
    }

    #[test]
    fn black_level_subtracts_the_cut() {
        let cfg = &HP3770.calibration;
        let data = image(4, 10, |_, _, _| 13);
        let img = Image::new(&data, 4);
        // (13 - 0.06) * 256 = 3312.64 -> 3312.
        assert_eq!(black_level(cfg, &img, 0, Pixels::All), 3312);
    }

    #[test]
    fn white_average_drops_the_extremes() {
        // Constant column: every run average is the same.
        assert_eq!(white_average(&[100; 24]), 100.0);
        // One bright line raises 4 run averages; 2 of them are dropped.
        let mut s = [100u8; 24];
        s[10] = 140;
        // Runs 7..=10 contain line 10 and average 110; they are among the
        // first 17, so they are summed and remembered as the highest. Two
        // of the last four runs (100) then take the place of two of them,
        // so the sum is 15 * 100 + 2 * 110 over 17.
        let expected = (15.0 * 100.0 + 2.0 * 110.0) / 17.0;
        assert!((white_average(&s) - expected).abs() < 1e-12);
    }

    #[test]
    fn white_coefficient_is_reference_over_average() {
        let cfg = &HP3770.calibration;
        let data = image(2, 24, |_, p, _| if p == 0 { 245 } else { 122 });
        let img = Image::new(&data, 2);
        let w = white_coefficients(cfg, &img, 0);
        assert_eq!(w[0], 8192);
        // 245 / 122 * 8192 = 16451.1
        assert_eq!(w[1], 16451);
        assert_eq!(table_bytes(&[0x0cd6, 0x8e2a]), [0xd6, 0x0c, 0x2a, 0x8e]);
    }
}
