// SPDX-License-Identifier: GPL-2.0-or-later
//! The register block of a scan (`RTS_Setup` and the in-memory part of
//! `RTS_GetImage` in hp3900).
//!
//! Every read of the chip, the short reads of the calibration as well as the
//! scan itself, is the same thing: a copy of the initial block where a
//! few dozen registers say which pixels to read, how fast, with which
//! sensor clocks, where the shading tables live and how the motor moves.
//!
//! The HP driver builds the block with the `hp3900` code, but its version
//! differs in places. Each difference is noted where it occurs; all of them
//! come from `hpgt3770.dll` (function at `0x10038290` for `RTS_Setup`,
//! `0x1002d490` for `RTS_GetImage`) and are checked against the blocks the
//! HP driver writes in the captures.
//!
//! A read takes two block writes: [`image_registers`] or [`scan_registers`]
//! give the first, [`finish_image_registers`] or [`finish_scan_registers`]
//! turn it into the second once `E954` has been read from the chip. The
//! test `register_blocks_of_every_scan_capture` rebuilds both blocks of
//! every calibration read and of the scan in each capture (200 to 2400 dpi,
//! colour, grey and lineart), byte for byte.
//!
//! One thing we do differently on purpose: after a 2400 dpi scan the HP
//! driver keeps arranging the colour lines by software at every resolution
//! (captures 07-11, taken after 06b), where before it let the chip do it
//! (01-06). Both blocks are rebuilt by the tests; we use the chip up to
//! 1200 dpi every time.

use crate::init::Registers;
use crate::motor::{self, StepTables};
use crate::profile::Profile;
use crate::scan_modes::{self, Cph, SampleRate, ScanMode, Timing};
use crate::{Error, Result};

/// Size of the chip RAM in bytes (`mem_total` in hp3900).
const MEM_TOTAL: u32 = 0x80000;
/// Block size of the shading tables in RAM (`v160c_block_size`).
const BLOCK_SIZE: u32 = 4;

/// What to read: resolution and window, in pixels and lines of the scan
/// resolution (`st_scanparams` in hp3900, colour only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanParams {
    /// Bits per sample: 8 or 16.
    pub depth: u8,
    /// Horizontal resolution in dpi; selects the scan mode.
    pub resolution_x: u16,
    /// Vertical resolution in dpi; sets the motor step.
    pub resolution_y: u16,
    /// First pixel, from the left edge of the sensor.
    pub left: u32,
    /// First line, from the current position of the carriage.
    pub top: u32,
    /// Pixels per line.
    pub width: u32,
    /// Lines.
    pub height: u32,
}

/// How the chip arranges the lines of the three colours, which the CCD
/// reads at different heights (`arrangeline` in hp3900).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrangeLine {
    /// Not at all: every colour of a line comes from the same sensor line.
    /// Used by the calibration, which reads a uniform strip.
    None,
    /// The chip delays the colours by the distance between the sensor lines
    /// (`FIX_BY_HARD`). Used by the scan up to 1200 dpi.
    Hardware,
    /// The host arranges the lines (`FIX_BY_SOFT`): the chip reads as many
    /// more lines as the colours are apart. Used by the scan at 2400 dpi,
    /// where the distances do not fit in the 6-bit registers.
    Software,
}

/// How a read uses the chip (`st_hwdconfig` in hp3900).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hardware {
    /// Apply the black shading table.
    pub black_shading: bool,
    /// Apply the white shading table.
    pub white_shading: bool,
    /// Apply the gamma tables.
    pub gamma: bool,
    /// Size code of the gamma tables (low 4 bits of `E9D0`).
    pub gamma_size: u8,
    /// Move towards home while reading.
    pub backward: bool,
    /// Leave the carriage where it is.
    pub static_head: bool,
    /// Read nothing back (`dummy_scan`).
    pub dummy_scan: bool,
    /// Compressed transfer. Not used by the HP driver.
    pub compression: bool,
    /// Line arrangement.
    pub arrange_line: ArrangeLine,
    /// Separate even and odd pixels by their distance on the CCD
    /// (`highresolution` in hp3900). The HP driver sets it from 1200 dpi
    /// up; hp3900 only above 1200.
    pub high_resolution: bool,
}

/// Analog front-end settings of the three colours (`st_gain_offset`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GainOffset {
    /// Offset of even pixels, 9 bits (`edcg1`).
    pub even_offset: [u16; 3],
    /// Fine offset of even pixels, 7 bits (`edcg2`).
    pub even_fine: [u8; 3],
    /// Offset of odd pixels, 9 bits (`odcg1`).
    pub odd_offset: [u16; 3],
    /// Fine offset of odd pixels, 7 bits (`odcg2`).
    pub odd_fine: [u8; 3],
    /// Variable gain, 5 bits (`vgag1`).
    pub gain: [u8; 3],
    /// Second variable gain, 5 bits (`vgag2`).
    pub gain2: [u8; 3],
    /// Programmable gain, 2 bits (`pag`).
    pub pga: [u8; 3],
}

/// What [`setup`] tells the caller besides the block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupInfo {
    /// The scan mode used.
    pub mode: &'static ScanMode,
    /// Bytes of one line of the image the chip sends.
    pub bytes_per_line: u32,
    /// Motor step tables to load before the read, if the motor uses curves.
    pub step_tables: Option<StepTables>,
    /// Where the step tables go in the chip RAM, in units of 16 bytes
    /// (`v15f8` in hp3900).
    pub tables_start: u16,
    /// Lines the chip sends beyond the height when the host arranges the
    /// colour lines ([`ArrangeLine::Software`]); 0 otherwise.
    pub extra_lines: u32,
}

/// Sets the analog front end (`RTS_Setup_GainOffset`, calibrating branch).
pub fn setup_gain_offset(regs: &mut Registers, go: &GainOffset) {
    for c in 0..3 {
        let even = go.even_offset[c];
        let odd = go.odd_offset[c];
        regs.0[0x1a + 4 * c] = even as u8;
        regs.0[0x1b + 4 * c] = ((even >> 1) & 0x80) as u8 | (go.even_fine[c] & 0x7f);
        regs.0[0x1c + 4 * c] = odd as u8;
        regs.0[0x1d + 4 * c] = ((odd >> 1) & 0x80) as u8 | (go.odd_fine[c] & 0x7f);
        regs.set_bits(0x14 + c, 0x1f, go.gain[c]);
        regs.set_bits(0x17 + c, 0x1f, go.gain2[c]);
    }
    regs.set_bits(0x13, 0x03, go.pga[0]);
    regs.set_bits(0x13, 0x0c, go.pga[1]);
    regs.set_bits(0x13, 0x30, go.pga[2]);
}

/// Writes a sensor clock (`Timing_SetLinearImageSensorClock`) at `offset`.
fn sensor_clock(regs: &mut Registers, offset: usize, cph: &Cph) {
    let p1 = cph.p1.to_le_bytes();
    regs.0[offset..offset + 4].copy_from_slice(&p1[..4]);
    regs.0[offset + 4] = (regs.0[offset + 4] & 0x80)
        | (p1[4] & 0x0f)
        | (u8::from(cph.ps) << 6)
        | (u8::from(cph.ge) << 5)
        | (u8::from(cph.go) << 4);
    let p2 = cph.p2.to_le_bytes();
    regs.0[offset + 5..offset + 9].copy_from_slice(&p2[..4]);
    regs.0[offset + 9] = (regs.0[offset + 9] & 0xf0) | (p2[4] & 0x0f);
}

/// Programs the sensor timing (`RTS_Setup_SensorTiming`).
fn setup_sensor_timing(regs: &mut Registers, t: &Timing) {
    regs.set_bits(0x92, 0x3f, t.cdss[0]);
    regs.set_bits(0x93, 0x3f, t.cdsc[0]);
    regs.set_bits(0x94, 0x3f, t.cdss[1]);
    regs.set_bits(0x95, 0x3f, t.cdsc[1]);
    regs.set_bits(0x96, 0x3f, t.cnpp);
    regs.set_bits(0x45, 0x80, u8::from(t.cvtrp[0]));
    regs.set_bits(0x45, 0x40, u8::from(t.cvtrp[1]));
    regs.set_bits(0x45, 0x20, u8::from(t.cvtrp[2]));
    regs.set_bits(0x45, 0x1f, t.cvtrfpw);
    regs.set_bits(0x46, 0x1f, t.cvtrbpw);
    regs.0[0x47] = t.cvtrw;
    regs.set_lsb(0x84, t.cphbp2s, 3);
    regs.set_lsb(0x87, t.cphbp2e, 3);
    regs.set_lsb(0x8a, t.clamps, 3);
    regs.set_lsb(0x8d, t.clampe.unwrap_or(u32::MAX), 3);
    let a = t.adcclkp[0].to_le_bytes();
    regs.0[0x97..0x9b].copy_from_slice(&a[..4]);
    regs.0[0x9b] = (regs.0[0x9b] & 0xf0) | (a[4] & 0x0f);
    let b = t.adcclkp[1].to_le_bytes();
    regs.0[0xc1..0xc5].copy_from_slice(&b[..4]);
    regs.0[0xc5] = (regs.0[0xc5] & 0xe0) | (b[4] & 0x0f) | ((t.adcclkp2e & 1) << 4);
    for (i, cph) in t.cph.iter().enumerate() {
        sensor_clock(regs, 0x48 + 10 * i, cph);
    }
}

/// Builds the register block of a read in `regs` (`RTS_Setup`). `params`
/// is updated like hp3900 does (the top line moves by the acceleration).
pub fn setup(
    regs: &mut Registers,
    profile: &Profile,
    params: &mut ScanParams,
    hw: &Hardware,
    gain_offset: &GainOffset,
) -> Result<SetupInfo> {
    let mode = scan_modes::colour_mode(params.resolution_x).ok_or_else(|| {
        Error::InvalidArgument(format!("no scan mode at {} dpi", params.resolution_x))
    })?;
    let timing = mode.timing;
    let platform = &profile.platform;
    let sensor_resolution = u32::from(timing.sensor_resolution);
    let resolution_x = u32::from(params.resolution_x);
    let resolution_y = u32::from(params.resolution_y);

    // System clock.
    regs.set_bits(0x00, 0x0f, mode.system_clock);

    // Exposure times (RTS_Setup_Exposure_Times). The maximum exposure of
    // each colour is the line period, the exposure 0 (see COLOUR_MODES).
    let mut line = mode.ctpc + 1;
    if params.depth > 8 {
        // MultiExposureFor16BitMode: 1 (no key in the file).
        line *= 1;
    }
    let line = line - 1;
    for offset in [0x36, 0x3c, 0x42] {
        regs.set_lsb(offset, 0, 3);
    }
    for offset in [0x33, 0x39, 0x3f] {
        regs.set_lsb(offset, mode.ctpc, 3);
    }
    regs.set_lsb(0x30, line, 3);

    // Line distances (RTS_Setup_Line_Distances).
    let sensor = u32::from(platform.sensor_resolution);
    let even_odd = if hw.high_resolution {
        u32::from(platform.even_odd_distance) * resolution_y / sensor
    } else {
        0
    };
    let mut line_distance = 0u32;
    match hw.arrange_line {
        ArrangeLine::Hardware => {
            let distance = u32::from(platform.line_distance) * resolution_y / sensor;
            regs.set_bits(0x149, 0x3f, even_odd as u8);
            regs.set_bits(0x14a, 0x3f, distance as u8);
            regs.set_bits(0x14b, 0x3f, (distance + even_odd) as u8);
            regs.set_bits(0x14c, 0x3f, (distance * 2) as u8);
            regs.set_bits(0x14d, 0x3f, (distance * 2 + even_odd) as u8);
        }
        ArrangeLine::None | ArrangeLine::Software => {
            for offset in 0x149..=0x14d {
                regs.set_bits(offset, 0x3f, 0);
            }
            if hw.arrange_line == ArrangeLine::Software {
                // Colour: two line distances, plus the even/odd one in
                // high resolution, and one more line.
                let lines = if hw.high_resolution {
                    (u32::from(platform.line_distance) * 2 + u32::from(platform.even_odd_distance))
                        * resolution_y
                } else {
                    u32::from(platform.line_distance) * resolution_y * 2
                };
                line_distance = ((lines / sensor + 1).max(2)) & 0xffff;
            }
        }
    }

    // Colour channels (RTS_Setup_Channels, colour branch).
    regs.set_bits(0x12, 0xc0, 3);
    regs.set_bits(0x1cf, 0x40, (mode.sample_rate == SampleRate::Line) as u8);
    regs.set_bits(0x1cf, 0x80, 0);
    regs.set_bits(0x60a, 0x30, platform.channel_color[0]);
    regs.set_bits(0x60a, 0x0c, platform.channel_color[1]);
    regs.set_bits(0x60a, 0x03, platform.channel_color[2]);
    regs.set_bits(0x12, 0x03, platform.rgb_order[0]);
    regs.set_bits(0x12, 0x0c, platform.rgb_order[1]);
    regs.set_bits(0x12, 0x30, platform.rgb_order[2]);

    // Compression (not used).
    if hw.compression {
        regs.0[0x60b] = (regs.0[0x60b] & 0x9f) | 0x10;
        regs.0[0x1cf] = (regs.0[0x1cf] & 0xef) | 0x20;
    } else {
        regs.0[0x60b] &= 0x80;
    }

    // Depth (RTS_Setup_Depth).
    let channels_per_line = u32::from(regs.get(0x12) >> 6) * params.width;
    let bytes_per_line = match params.depth {
        16 => {
            regs.set_bits(0x1cf, 0x30, 2);
            channels_per_line * 2
        }
        12 => {
            regs.set_bits(0x1cf, 0x30, 1);
            channels_per_line * 2
        }
        _ => {
            regs.set_bits(0x1cf, 0x30, 0);
            channels_per_line
        }
    };

    // Resolution ratio.
    let ratio = (sensor_resolution / resolution_x) as u8;
    regs.set_bits(0xc0, 0x1f, ratio);
    // HP driver: E8BC-E8BF from fields of the scan mode that hp3900 does
    // not have (an X sample table?). Same values as the initial block in
    // every capture: 00 00 00 and 32 in the low 6 bits of E8BF.
    regs.0[0xbc] = 0;
    regs.0[0xbd] = 0;
    regs.0[0xbe] = 0;
    regs.set_bits(0xbf, 0x3f, 0x20);

    // Coordinates, computed here by the HP driver before the sensor timing:
    // pixels at the sensor resolution, the left one odd.
    if params.left == 0 {
        params.left = 1;
    }
    let mut left = params.left * sensor_resolution / resolution_x;
    if left & 1 == 0 {
        left += 1;
    }
    let right = left + params.width * sensor_resolution / resolution_x;

    setup_sensor_timing(regs, timing);

    // Scan type (normal) and moving head.
    regs.set_bits(0xd8, 0x40, 0);
    regs.set_bits(0xd8, 0x80, u8::from(!hw.static_head));

    // Gamma (RTS_Setup_Gamma). The motor step tables go right after the
    // gamma tables in the chip RAM, or at its start without gamma.
    regs.0[0x1d0] &= 0xbf;
    let mut tables_start = 0;
    if hw.gamma {
        regs.0[0x1d0] |= 0x40;
        regs.set_bits(0x1d0, 0x0f, hw.gamma_size);
        let size: u32 = match regs.get(0x1d0) & 0x0c {
            0 => u32::from(regs.get(0x1d0) & 1) | 0x100,
            4 => u32::from(regs.get(0x1d0) & 1) | 0x400,
            8 => u32::from(regs.get(0x1d0) & 1) | 0x1000,
            _ => 0,
        };
        regs.0[0x1b4] = 0;
        regs.0[0x1b5] &= 0xc0;
        regs.set_word_bits(0x1b6, 0x3fff, size as u16);
        regs.set_word_bits(0x1b8, 0x3fff, (size * 2) as u16);
        tables_start = ((size * 3 + 15) >> 4) as u16;
    }

    // Shading (RTS_Setup_Shading), as the HP driver computes it.
    setup_shading(regs, hw, params.width, bytes_per_line);

    // Motor.
    let step_tables = setup_motor(regs, profile, mode, params, hw, tables_start)?;

    // Coordinates.
    if params.top == 0 {
        params.top = 1;
    }
    let dummy_line = u32::from(regs.get(0xd6) >> 4);
    let top = params.top * dummy_line;
    let bottom =
        (u32::from(regs.get(0x14d) & 0x3f) + params.height + line_distance) * dummy_line + top;
    regs.set_lsb(0xb0, left, 2);
    regs.set_lsb(0xb2, right, 2);
    regs.set_lsb(0xd0, top, 2);
    regs.set_lsb(0xd2, bottom, 2);
    regs.0[0xd4] = (((bottom >> 16) as u8) << 4) | ((top >> 16) as u8 & 0x0f);

    regs.0[0x163] |= 0xc0;
    regs.0[0x01] = (regs.get(0x01) & 0xe9) | (u8::from(hw.dummy_scan) << 4);

    setup_gain_offset(regs, gain_offset);

    Ok(SetupInfo {
        mode,
        bytes_per_line,
        step_tables,
        tables_start,
        extra_lines: line_distance,
    })
}

/// Shading memory layout (`RTS_Setup_Shading`). The HP driver computes the
/// second image pointer from the first as rounded in the registers, and
/// writes the high bits of the free memory in the low nibble of `EF12`.
fn setup_shading(regs: &mut Registers, hw: &Hardware, width: u32, bytes_per_line: u32) {
    let ratio = u32::from(regs.get(0xc0) & 0x1f);
    regs.set_bits(0x1bf, 0x18, 0);
    regs.set_bits(0x1cf, 0x08, u8::from(hw.black_shading));
    regs.set_bits(0x1cf, 0x04, u8::from(hw.white_shading));

    let mut size = 0;
    if regs.get(0x1cf) & 0x08 != 0 {
        size = ratio * width * 2;
    }
    if regs.get(0x1cf) & 0x04 != 0 {
        size += ratio * width * 2;
    }
    let size = size.div_ceil(BLOCK_SIZE).div_ceil(16) + 16;
    regs.0[0x1ba] = 0;
    regs.0[0x1bb] = size as u8;
    regs.0[0x1bc] = (size >> 8) as u8;
    regs.0[0x1bd] = (size * 2) as u8;
    regs.0[0x1be] = ((size * 2) >> 8) as u8;
    regs.0[0x1bf] =
        (regs.get(0x1bf) & 0xf8) | (((size * 2) >> 16) as u8 & 3) << 1 | ((size >> 16) as u8 & 1);
    let three = size * 3;
    regs.0[0x1c0] = three as u8;
    regs.0[0x1c1] = (three >> 8) as u8;
    regs.0[0x1c2] = (regs.get(0x1c2) & 0xf0) | ((three >> 16) as u8 & 0x0f);

    let mut free = MEM_TOTAL - three * 16;
    let line_distance = u32::from(regs.get(0x14a) & 0x3f);
    let dots = match regs.get(0x12) >> 6 {
        3 => {
            let dots = bytes_per_line / 3;
            let lines = ((line_distance + 1) * dots).div_ceil(BLOCK_SIZE);
            free = (free - lines * 3).div_ceil(3);
            let first = ((lines + three * 8) * 2 + free) / 16 + 1;
            regs.0[0x1c3] = first as u8;
            regs.0[0x1c4] = (first >> 8) as u8;
            regs.0[0x1c2] = (regs.get(0x1c2) & 0x0f) | (((first >> 16) as u8) << 4);
            let second = (first * 16 + lines + free) / 16 + 1;
            regs.0[0x1c5] = second as u8;
            regs.0[0x1c6] = (second >> 8) as u8;
            regs.0[0x1c7] = (regs.get(0x1c7) & 0xf0) | ((second >> 16) as u8 & 0x0f);
            dots
        }
        2 => {
            let dots = bytes_per_line / 2;
            let lines = ((line_distance + 1) * dots).div_ceil(BLOCK_SIZE);
            free = (free - lines).div_ceil(2);
            let first = ((three << 4) + lines + free) / 16 + 1;
            regs.0[0x1c3] = first as u8;
            regs.0[0x1c4] = (first >> 8) as u8;
            regs.0[0x1c2] = (regs.get(0x1c2) & 0x0f) | (((first >> 16) as u8) << 4);
            dots
        }
        _ => bytes_per_line,
    };
    let last = (MEM_TOTAL - 1) >> 4;
    regs.0[0x1c8] = last as u8;
    regs.0[0x1c9] = (last >> 8) as u8;
    regs.0[0x1c7] = (regs.get(0x1c7) & 0x0f) | (((last >> 16) as u8) << 4);
    regs.0[0x715] &= 0xf0;
    regs.0[0x713] = 0;
    regs.0[0x714] = 0x10;
    let free = free - dots.div_ceil(BLOCK_SIZE);
    regs.0[0x710] = (free >> 4) as u8;
    regs.0[0x711] = (free >> 12) as u8;
    regs.0[0x712] = (regs.get(0x712) & 0xf0) | ((free >> 20) as u8 & 0x0f);
}

/// Motor of a read (`RTS_Setup_Motor` as the HP driver has it): curves
/// only when the head moves, the line period rounded up to a multiple of
/// the step, and the top line moved back by the acceleration.
fn setup_motor(
    regs: &mut Registers,
    profile: &Profile,
    mode: &ScanMode,
    params: &mut ScanParams,
    hw: &Hardware,
    tables_start: u16,
) -> Result<Option<StepTables>> {
    let platform = &profile.platform;
    regs.0[0x104..0x124].copy_from_slice(if mode.motor_phase {
        &platform.motor_resource_phase
    } else {
        &profile.motor_resource
    });

    let mut step_type = mode.step_type;
    if step_type == motor::StepType::Full
        && u32::from(params.resolution_y) == 2 * u32::from(platform.motor_resolution)
    {
        step_type = motor::StepType::Half;
    }
    let direction: u8 = if hw.backward { 0x00 } else { 0x08 };
    regs.0[0xd9] = (((step_type as u8) | direction) << 4) ^ (direction & 0x0f);
    regs.0[0xdd] = (regs.get(0xdd) & 0x3f) | (((step_type as u8 | direction) & 0xf0) << 3);
    let steps = step_type.divisor();

    regs.set_bits(0xd6, 0xf0, mode.dummy_line.max(1));
    let curves = if regs.get(0xd8) & 0x80 != 0 {
        mode.motor_curve
    } else {
        None
    };
    regs.set_bits(0xdf, 0x10, u8::from(curves.is_some()));

    let dummy_line = u32::from(regs.get(0xd6) >> 4);
    let step_size = (steps * u32::from(platform.motor_resolution))
        / (dummy_line * u32::from(params.resolution_y));
    regs.0[0xe0] = (step_size - 1) as u8;
    let mut line = regs.get_lsb(0x30, 3);
    let rest = (line + 1) % step_size;
    if rest != 0 {
        line += step_size - rest;
    }
    regs.set_lsb(0x30, line, 3);
    let last = (line + 1) / step_size - 1;
    regs.set_lsb(0xe1, last, 3);
    for offset in [0xea, 0xed, 0xf0, 0xf3] {
        regs.set_lsb(offset, 0x10, 3);
    }

    let msi: u16 = 0x10;
    let mut acc_steps = 0u32;
    let tables = if let Some(setting) = curves {
        let tables = motor::build_step_tables(regs, setting, tables_start, mode.motor_back_step)?;
        acc_steps = tables.acc_steps;
        let back = u32::from(mode.motor_back_step);
        let smearing = tables.smearing_steps;
        let normal = tables.acc_steps + tables.buffer_full_steps;
        let back_a = if back >= smearing {
            back - smearing + 2
        } else {
            0
        };
        let back_b = if back >= normal { back - normal + 2 } else { 0 };
        regs.0[0xda] = msi as u8;
        regs.0[0xdb] = back_a as u8;
        regs.0[0xdc] = back_b as u8;
        regs.0[0xdd] = (regs.get(0xdd) & 0xc0)
            | (((back_b >> 8) as u8 & 3) << 4)
            | (((back_a >> 8) as u8 & 3) << 2)
            | ((msi >> 8) as u8 & 3);
        Some(tables)
    } else {
        for offset in [0xe1, 0xe4, 0xe7] {
            regs.set_lsb(offset, last, 3);
        }
        let back = u32::from(mode.motor_back_step);
        regs.0[0xda] = msi as u8;
        regs.0[0xdb] = back as u8;
        regs.0[0xdc] = back as u8;
        let high = (back >> 8) as u8 & 3;
        regs.0[0xdd] = (regs.get(0xdd) & 0xc0) | (high << 4) | (high << 2) | ((msi >> 8) as u8 & 3);
        None
    };

    let per_line = dummy_line * step_size;
    if acc_steps >= params.top * per_line {
        params.top = 1;
    } else {
        params.top = params.top - acc_steps / per_line - 1;
    }
    Ok(tables)
}

/// The block of a read up to its first write to the chip (in-memory part
/// of `RTS_GetImage` before `RTS_WriteRegs`): [`setup`] on a copy of
/// `base`, then [`set_multi_exposure`] and the clamp end. Returns the block
/// and the parameters as `setup` left them.
///
/// The HP driver moves the window 24 pixels right for its CCD (hp3900 also
/// moves it 63 pixels left at 1200 dpi and 126 at 2400; the HP driver does
/// not), and sets the end of the clamp pulse to the line period after the
/// exposure is final (hp3900 only does that for one chip model, and only
/// with the value of `CPHBP2E`).
pub fn image_registers(
    base: &Registers,
    profile: &Profile,
    params: &ScanParams,
    hw: &Hardware,
    gain_offset: &GainOffset,
) -> Result<(Registers, ScanParams, SetupInfo)> {
    let mut regs = base.clone();
    let mut params = *params;
    params.left += 24;
    let info = setup(&mut regs, profile, &mut params, hw, gain_offset)?;
    set_multi_exposure(&mut regs, profile);
    regs.0.copy_within(0x30..0x33, 0x8d);
    Ok((regs, params, info))
}

/// The block of a scan up to its first write to the chip (the in-memory
/// part of the HP scan start before `RTS_WriteRegs`): [`setup`] on a copy
/// of `base`, with the window moved 24 pixels right as for [`image_registers`].
/// Unlike a calibration read, a scan keeps the line period of the scan mode
/// (no [`set_multi_exposure`]) and the clamp end of the timing until the
/// second write.
pub fn scan_registers(
    base: &Registers,
    profile: &Profile,
    params: &ScanParams,
    hw: &Hardware,
    gain_offset: &GainOffset,
) -> Result<(Registers, ScanParams, SetupInfo)> {
    let mut regs = base.clone();
    let mut params = *params;
    params.left += 24;
    let info = setup(&mut regs, profile, &mut params, hw, gain_offset)?;
    Ok((regs, params, info))
}

/// The changes between the first and the second write of a scan: the
/// motor starts on its acceleration curve when the scan mode has one
/// (`E801` bit 1, as hp3900 does after `RTS_Setup`), the binary thresholds
/// are cleared (the chip always sends colour; the HP driver makes grey and
/// black and white itself), the motor current of the scan resolution from
/// `Motor_Change` given the value of `E954`-`E955` read from the chip, the
/// lamp gain mode, and the clamp end set to the line period.
///
/// Checked on the scans of every capture at 200, 300, 600, 1200 and 2400
/// dpi; unlike a calibration read, the DMA lock bit of `EE00` stays clear.
pub fn finish_scan_registers(
    regs: &mut Registers,
    profile: &Profile,
    e954: [u8; 2],
    params: &ScanParams,
) {
    let curves = regs.get(0xdf) & 0x10 != 0;
    regs.set_bits(0x01, 0x02, u8::from(curves));
    regs.set_lsb(0x19e, 0, 2);
    regs.set_lsb(0x1a0, 0, 2);
    if profile.motor_current.is_some() {
        let level = scan_modes::motor_current_level(params.resolution_x);
        regs.0[0x154] = (e954[0] & 0xcf) | (level.min(3) << 4);
        regs.0[0x155] = e954[1];
    }
    lamp_set_gain_mode(
        regs,
        params.resolution_x,
        lamp_gain_mode(params.resolution_x),
    );
    regs.0.copy_within(0x30..0x33, 0x8d);
}

/// Slows the line down to the base speed of the motor when the scan mode
/// is faster (`SetMultiExposure`). Without curves the motor cannot start
/// faster than `BaseSpeedPPS`; the line period becomes the smallest
/// multiple of the scan mode's that is at least as long as one motor step
/// at that speed.
///
/// The HP driver rounds the step from the base speed without adding 1
/// (hp3900 adds 1): the white strip at 300 dpi (capture 04) has a line of
/// 15000 ticks, where hp3900 would give 18000.
pub fn set_multi_exposure(regs: &mut Registers, profile: &Profile) {
    regs.set_bits(0xdf, 0x10, 0);
    let clock: u32 = match regs.get(0x00) & 0x0f {
        0x00 => 0x0089_5440,
        0x08 | 0x01 => 0x00b7_1b00,
        0x09 => 0x00f4_2400,
        0x02 => 0x0112_a880,
        0x0a | 0x03 => 0x016e_3600,
        0x0b => 0x01e8_4800,
        0x04 => 0x0225_5100,
        0x0c => 0x02dc_6c00,
        0x05 => 0x044a_a200,
        0x0d => 0x05b8_d800,
        _ => 0x0478_f7f8,
    };
    let base_step =
        clock / (u32::from(regs.get(0x96) & 0x3f) + 1) / u32::from(profile.platform.base_speed_pps);
    let line = regs.get_lsb(0x30, 3) + 1;
    if regs.get_lsb(0xe1, 3) >= base_step {
        return;
    }
    for offset in [0x36, 0x3c, 0x42] {
        if regs.get_lsb(offset, 3) == 0 {
            regs.set_lsb(offset, line - 1, 3);
        }
    }
    let step_size = u32::from(regs.get(0xe0)) + 1;
    let target = (step_size * base_step).div_ceil(line) * line;
    regs.set_lsb(0x30, target - 1, 3);
    regs.set_lsb(0xe1, target / step_size - 1, 3);
}

/// The changes between the first and the second write of a read (the rest
/// of the in-memory part of `RTS_GetImage`): the motor current from
/// `Motor_Change`, given the value of `E954`-`E955` read from the chip, no
/// motor lock, the DMA lock and the lamp gain mode.
///
/// The HP driver always asks `Motor_Change` for the current of 1200 dpi
/// (hp3900 asks for the current of the scan resolution), and takes both
/// bytes from the chip.
pub fn finish_image_registers(
    regs: &mut Registers,
    profile: &Profile,
    e954: [u8; 2],
    params: &ScanParams,
) {
    if profile.motor_current.is_some() {
        regs.0[0x154] = e954[0] & 0xcf;
        regs.0[0x155] = e954[1];
    }
    regs.0[0x00] &= 0xef;
    regs.0[0xdf] &= 0xf0;
    regs.0[0xde] = 0;
    regs.set_bits(0x600, 0x04, u8::from(params.depth != 16));
    lamp_set_gain_mode(
        regs,
        params.resolution_x,
        lamp_gain_mode(params.resolution_x),
    );
}

/// Whether the analog front end uses its gain control at a horizontal
/// resolution (`Lamp_GetGainMode`): `600DpiGainControl=1` in `[SCAN_PARAM]`
/// turns it on up to 600 dpi. Bit 6 of `E954` in the second block of every
/// read in the captures: on at 200, 300 and 600 dpi, off at 1200 (and at
/// 2400, which reads the sensor at 1200).
pub fn lamp_gain_mode(resolution: u16) -> bool {
    resolution <= 600
}

/// `Lamp_SetGainMode` as the HP driver has it for its sensor (type 2 in
/// the driver; hp3900 has no branch for it).
pub fn lamp_set_gain_mode(regs: &mut Registers, resolution: u16, gain_control: bool) {
    let mut data = (regs.get_lsb(0x154, 2) & 0xfe7f) as u16;
    if gain_control {
        data &= 0xffbf;
    } else {
        data |= 0x40;
    }
    match resolution {
        100 | 200 | 300 | 600 => data |= 0x180,
        1200 => data &= 0xf7ff,
        2400 => data |= 0x180,
        _ => {}
    }
    regs.set_lsb(0x154, u32::from(data), 2);
}
