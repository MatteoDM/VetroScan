// SPDX-License-Identifier: GPL-2.0-or-later
//! The carriage motor: step tables, the DMA write that loads them, and the
//! moves the chip runs on its own (`Motor_Setup_Steps`, `RTS_DMA_Write`,
//! `Motor_Move` and `Head_ParkHome` in hp3900).
//!
//! The chip drives the motor from tables of step lengths kept in its RAM,
//! right after the gamma tables: one table to accelerate, one to slow down
//! when its buffer is full, and others for smearing and parking. Registers
//! `E8F6`-`E903` point to each table and `E8E1`-`E8F5` hold the last step of
//! each. A move is then a normal "dummy" scan with the motor on: the chip
//! accelerates, runs the requested number of lines and stops.
//!
//! Checked against the HP driver in three captures (05, 06 and 11), where it
//! brings the carriage home with the same table and the same block.

use crate::error::{Error, Result};
use crate::init::Registers;
use crate::protocol::DmaDirection;
use crate::transport::{Chip, Transport};
use std::time::Duration;

/// Where the step tables start in the chip RAM, in units of 16 bytes
/// (`v15f8` in hp3900): right after the three gamma tables of 256 bytes.
/// The HP driver uses this offset in every capture.
pub const STEP_TABLES_START: u16 = 0x30;

/// One motor setting (`st_motorcurve` in hp3900): the curves and which one
/// is used for each phase of a move. Curves are numbered from 1 like in the
/// HP configuration; 0 means the phase has no curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotorSetting {
    /// Acceleration curves (`[AccCurveN]`), step lengths in clock ticks.
    pub acc_curves: &'static [&'static [u16]],
    /// Deceleration curves (`[DecCurveN]`).
    pub dec_curves: &'static [&'static [u16]],
    /// `[ACC] NormalScan`.
    pub acc_normal: u8,
    /// `[ACC] ParkHome`.
    pub acc_park: u8,
    /// `[ACC] Smearing`.
    pub acc_smearing: u8,
    /// `[DEC] NormalScan`.
    pub dec_normal: u8,
    /// `[DEC] ParkHome`.
    pub dec_park: u8,
    /// `[DEC] Smearing`.
    pub dec_smearing: u8,
    /// `[DEC] ScanBufferFull`.
    pub dec_buffer_full: u8,
}

impl MotorSetting {
    fn acc(&self, number: u8) -> Option<&'static [u16]> {
        curve(self.acc_curves, number)
    }

    fn dec(&self, number: u8) -> Option<&'static [u16]> {
        curve(self.dec_curves, number)
    }
}

fn curve(curves: &'static [&'static [u16]], number: u8) -> Option<&'static [u16]> {
    let index = usize::from(number).checked_sub(1)?;
    curves.get(index).copied().filter(|c| !c.is_empty())
}

/// Motor step mode (`ScanMotorStepType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepType {
    /// Full steps.
    Full = 0,
    /// Half steps.
    Half = 1,
    /// Quarter steps.
    Quarter = 2,
    /// Eighth steps.
    Eighth = 3,
}

impl StepType {
    /// Micro-steps per full step.
    pub fn divisor(self) -> u32 {
        1 << (self as u32)
    }
}

/// How the motor runs during a move (`[MotorMoveN]`, `st_motormove` in
/// hp3900).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotorMove {
    /// System clock setting, low 4 bits of `E800`.
    pub system_clock: u8,
    /// Line period in clock ticks (`CTPC`), written to `E830`.
    pub ctpc: u32,
    /// Step mode.
    pub step_type: StepType,
    /// Acceleration curves; `None` runs at constant speed.
    pub setting: Option<&'static MotorSetting>,
}

/// Direction of a move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Towards the home position.
    Backward,
    /// Away from home. Not seen in the captures yet.
    Forward,
}

/// Where a move goes (`st_motorpos` in hp3900, always with the motor
/// enabled).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotorPosition {
    /// Direction.
    pub direction: Direction,
    /// Bit 2 of `E801` (`v12e448` in hp3900, meaning unknown). Set when
    /// parking.
    pub e801_bit2: bool,
    /// Length of the move in full steps (`coord_y`).
    pub steps: u16,
}

/// The step tables of one setting, as sent to the chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepTables {
    /// Steps of 3 bytes, big endian, padded with zeros to 16 bytes.
    pub bytes: Vec<u8>,
    /// Steps in the acceleration table (returned by `Motor_Setup_Steps`,
    /// `acccurvecount`).
    pub acc_steps: u32,
    /// Steps in the buffer-full deceleration table (`deccurvecount`).
    pub buffer_full_steps: u32,
    /// Steps of the smearing acceleration and deceleration curves
    /// (`smearacccurvecount + smeardeccurvecount`).
    pub smearing_steps: u32,
}

/// Builds the step tables of `setting` and points the registers to them
/// (the in-memory part of `Motor_Setup_Steps`). `start` is where the tables
/// go, in units of 16 bytes; `back_step` the steps the motor goes back when
/// the buffer is full (0 for a move). `E8E1` must already hold the last
/// step of the acceleration and `E830` the line period.
///
/// Differences from hp3900, both to match the HP driver:
/// - two phases share a table when their entries name the same curve.
///   hp3900 compares the steps, so with setting 9 it shares the normal
///   deceleration with the smearing one (same 32 steps); the HP driver
///   sends both (capture 11: `E8FC` = 1665, `E8FE` = 1761);
/// - each pointer is the offset where its table really starts. hp3900
///   computes it from the length of the previous curve, which is the same
///   unless an earlier phase shares a table; for the parking deceleration
///   it also copies the wrong registers when sharing. No 3770 setting used
///   so far shares a table.
pub fn build_step_tables(
    regs: &mut Registers,
    setting: &MotorSetting,
    start: u16,
    back_step: u16,
) -> Result<StepTables> {
    let acc = setting
        .acc(setting.acc_normal)
        .ok_or_else(|| Error::InvalidArgument("motor setting without acceleration".into()))?;
    let buffer_full = setting
        .dec(setting.dec_buffer_full)
        .ok_or_else(|| Error::InvalidArgument("motor setting without buffer-full curve".into()))?;

    let base = u32::from(start) << 4;
    let mut steps: Vec<u32> = Vec::new();
    let offset = |steps: &Vec<u32>| (base + 3 * steps.len() as u32) as u16;

    // Acceleration: the curve down to the last step, then the last step.
    let last_acc = regs.get_lsb(0xe1, 3);
    regs.set_word_bits(0xf6, 0x3fff, offset(&steps));
    let mut ticks: i64 = 0;
    for &step in acc {
        let step = u32::from(step);
        if step <= last_acc {
            break;
        }
        ticks += i64::from(step) + 1;
        steps.push(step);
    }
    if steps.is_empty() {
        ticks += i64::from(last_acc) + 2;
        steps.push(last_acc + 1);
    }
    ticks += i64::from(last_acc) + 1;
    steps.push(last_acc);
    let acc_steps = steps.len() as u32;
    // The first step is stretched so that the acceleration ends on a line
    // boundary.
    let line = i64::from(regs.get_lsb(0x30, 3)) + 1;
    let mut first = (ticks + line - 1) / line * line + i64::from(acc[0]) - ticks - 0x0d;
    steps[0] = first as u32;

    // Deceleration when the buffer is full: the last acceleration step, the
    // curve above it, then the back steps.
    regs.set_word_bits(0xf8, 0x3fff, offset(&steps));
    let last_dec = u32::from(buffer_full[buffer_full.len() - 1]);
    regs.set_lsb(0xea, last_dec, 3);
    let table = steps.len();
    steps.push(last_acc);
    for &step in &buffer_full[..buffer_full.len() - 1] {
        if u32::from(step) > last_acc {
            steps.push(u32::from(step));
        }
    }
    let mut extra = 0;
    if back_step > 0 {
        let step_size = i64::from(regs.get(0xe0)) + 1;
        // hp3900 counts the last step of the curve here, which is written
        // after the back steps.
        let count = (steps.len() - table) as i64 + 1;
        let wanted = i64::from(back_step) - count - i64::from(acc_steps) + 2;
        extra = wanted - wanted / step_size * step_size;
        first = i64::from(last_dec.max(last_acc + 1));
    }
    for _ in 0..extra.max(0) {
        steps.push((first - 1) as u32);
    }
    // With no back steps hp3900 and the HP driver end with the stretched
    // first step (4977 in capture 11).
    steps.push(first as u32);
    let buffer_full_steps = (steps.len() - table) as u32;
    let smearing_steps = [
        setting.acc(setting.acc_smearing),
        setting.dec(setting.dec_smearing),
    ]
    .iter()
    .map(|c| c.map_or(0, |c| c.len() as u32))
    .sum();

    // The other phases: a table of their own, or the one of an earlier
    // phase with the same curve.
    struct Phase {
        pointer: usize,
        last: usize,
        number: u8,
        curve: Option<&'static [u16]>,
    }
    let phase = |pointer, last, number, curve| Phase {
        pointer,
        last,
        number,
        curve,
    };
    let acc_normal = phase(0xf6, 0xe1, setting.acc_normal, None);
    let dec_buffer_full = phase(0xf8, 0xea, setting.dec_buffer_full, None);
    let acc_smearing = phase(
        0xfa,
        0xe4,
        setting.acc_smearing,
        setting.acc(setting.acc_smearing),
    );
    let dec_smearing = phase(
        0xfc,
        0xf0,
        setting.dec_smearing,
        setting.dec(setting.dec_smearing),
    );
    let dec_normal = phase(
        0xfe,
        0xed,
        setting.dec_normal,
        setting.dec(setting.dec_normal),
    );
    let acc_park = phase(0x100, 0xe7, setting.acc_park, setting.acc(setting.acc_park));
    let dec_park = phase(0x102, 0xf3, setting.dec_park, setting.dec(setting.dec_park));
    let order: [(&Phase, Vec<&Phase>, Option<u32>); 5] = [
        (&acc_smearing, vec![&acc_normal], None),
        (&dec_smearing, vec![&dec_buffer_full], None),
        (&dec_normal, vec![&dec_buffer_full, &dec_smearing], None),
        (&acc_park, vec![&acc_normal, &acc_smearing], Some(16)),
        (
            &dec_park,
            vec![&dec_buffer_full, &dec_smearing, &dec_normal],
            Some(16),
        ),
    ];
    for (this, earlier, last_if_none) in order {
        let Some(curve) = this.curve else {
            regs.set_word_bits(this.pointer, 0x3fff, 0);
            if let Some(last) = last_if_none {
                regs.set_lsb(this.last, last, 3);
            }
            continue;
        };
        if let Some(same) = earlier.iter().find(|p| p.number == this.number) {
            let pointer = regs.get_lsb(same.pointer, 2) as u16;
            regs.set_word_bits(this.pointer, 0x3fff, pointer);
            regs.set_lsb(this.last, regs.get_lsb(same.last, 3), 3);
        } else {
            regs.set_word_bits(this.pointer, 0x3fff, offset(&steps));
            regs.set_lsb(this.last, u32::from(curve[curve.len() - 1]), 3);
            steps.extend(curve.iter().map(|&s| u32::from(s)));
        }
    }

    let mut bytes: Vec<u8> = steps
        .iter()
        .flat_map(|s| s.to_be_bytes()[1..].to_vec())
        .collect();
    bytes.resize(bytes.len().next_multiple_of(16), 0);
    Ok(StepTables {
        bytes,
        acc_steps,
        buffer_full_steps,
        smearing_steps,
    })
}

/// The block to park from when no scan has been prepared yet.
///
/// hp3900 and the HP driver park from `init_regs`, where the set-up of the
/// last scan (`SetScanParams`) has left its values. Three of them reach the
/// block of the move: the CCD channels in `E810` and `E813` and the sensor
/// timing in `E896`. They are the same in every park of the captures (05,
/// 06 and 11, after different scans): this puts them in a copy of `init`,
/// so that a park right after the start-up sends the same block.
pub fn park_registers(init: &Registers) -> Registers {
    let mut regs = init.clone();
    regs.0[0x10] = 0xec;
    regs.0[0x13] = 0x80;
    regs.0[0x96] = 0x23;
    regs
}

/// Sets the scan window (`RTS_Setup_Coords`): left and right in pixels, top
/// and bottom in lines, with bits 16-19 of top and bottom in `E8D4`.
pub fn setup_coords(regs: &mut Registers, left: u32, top: u32, width: u32, height: u32) {
    regs.set_lsb(0xb0, left, 2);
    regs.set_lsb(0xb2, left + width, 2);
    regs.set_lsb(0xd0, top, 2);
    regs.set_bits(0xd4, 0x0f, (top >> 16) as u8);
    regs.set_lsb(0xd2, top + height, 2);
    regs.set_bits(0xd4, 0xf0, ((top + height) >> 16) as u8);
}

impl<T: Transport> Chip<T> {
    /// Sets or clears the lock bit of the DMA engine, bit 2 of `EE00`
    /// (`SetLock`), keeping `regs` in step.
    pub fn set_lock(&mut self, regs: &mut Registers, lock: bool) -> Result<()> {
        regs.set_bits(0x600, 0x04, u8::from(lock));
        self.write_byte(0xee00, regs.get(0x600))
    }

    /// Writes `data` to the chip RAM through the DMA engine and reads it
    /// back, retrying up to ten times until both match (`RTS_DMA_Write`).
    /// `options` is the RAM address in units of 16 bytes.
    ///
    /// The HP driver needs the retry: in capture 11 the first read-back
    /// stops after 1216 of 1728 bytes.
    pub fn dma_write(&mut self, channel: u16, options: u32, data: &[u8]) -> Result<()> {
        let size = u32::try_from(data.len())
            .map_err(|_| Error::InvalidArgument("DMA write too long".into()))?;
        self.dma_control(0)?;
        self.dma_enable(DmaDirection::Write, channel, size, options)?;
        let mut check = vec![0; data.len()];
        for _ in 0..10 {
            self.transport_mut().bulk_out(data)?;
            self.dma_enable(DmaDirection::Read, channel, size, options)?;
            let read = self.transport_mut().bulk_in(&mut check)?;
            if read == data.len() && check == data {
                return Ok(());
            }
            self.dma_cancel()?;
            self.dma_enable(DmaDirection::Write, channel, size, options)?;
        }
        Err(Error::Transport(
            "DMA write: the data read back never matched".into(),
        ))
    }

    /// Writes the whole register block. Like the HP driver, reads `EF19`
    /// first (always 0 in the captures; hp3900 does not read it).
    pub fn write_block(&mut self, regs: &Registers) -> Result<()> {
        self.read_word(0xef19)?;
        self.write_registers(crate::init::BLOCK_START, &regs.0[..])
    }

    /// Sends step tables to the chip RAM at `start` (the transfer part of
    /// `Motor_Setup_Steps`): warm reset, lock, DMA write, unlock.
    pub fn send_step_tables(
        &mut self,
        regs: &mut Registers,
        tables: &StepTables,
        start: u16,
    ) -> Result<()> {
        self.warm_reset()?;
        if tables.bytes.is_empty() {
            return Ok(());
        }
        self.set_lock(regs, true)?;
        self.dma_write(0, u32::from(start), &tables.bytes)?;
        self.set_lock(regs, false)
    }

    /// Moves the carriage (`Motor_Move`): prepares a copy of `regs` as a
    /// dummy scan with the motor on, loads the step tables, runs it and
    /// waits up to 30 s for the end. Returns `false` if the chip was still
    /// running after that.
    ///
    /// Differences from hp3900, to match the HP driver:
    /// - bit 6 of `E8DD` is set (hp3900 copies the motor-disable bit there);
    /// - the channel bits of `E812` and the timing of `E896` are left as
    ///   they are (hp3900 clears the first and writes 0x0B in the second);
    /// - hp3900 also writes 16 as last step of four deceleration tables in
    ///   the caller's block, not in the one it sends: left out, `regs` is
    ///   not changed.
    pub fn motor_move(
        &mut self,
        regs: &Registers,
        motor: &MotorMove,
        position: &MotorPosition,
    ) -> Result<bool> {
        let mut cp = regs.clone();
        // One dot per pixel.
        cp.set_bits(0xc0, 0x1f, 1);
        cp.set_bits(0xd9, 0x70, motor.step_type as u8);
        let options: u8 = match position.direction {
            Direction::Backward => 0x00,
            Direction::Forward => 0x08,
        };
        cp.set_bits(0xd9, 0x80, options >> 3);
        cp.set_bits(0xd9, 0x0f, options);
        // Motor enabled.
        cp.set_bits(0xdd, 0x80, 0);
        cp.set_bits(0xdd, 0x40, 1);
        let mut lines = ((u32::from(position.steps) * motor.step_type.divisor()) & 0xffff) as i64;
        lines = lines.max(2);
        // One dummy line, step size 1, dummy scan.
        cp.set_bits(0xd6, 0xf0, 1);
        cp.0[0xe0] = 0;
        cp.0[0x01] = (cp.get(0x01) & 0xf9) | (u8::from(position.e801_bit2) << 2);
        cp.set_bits(0x01, 0x10, 1);
        // Sample rate, unknown bit, one channel.
        cp.set_bits(0x1cf, 0x40, 0);
        cp.set_bits(0x1cf, 0x80, 1);
        cp.set_bits(0x12, 0xc0, 1);
        cp.set_bits(0x00, 0x0f, motor.system_clock);
        cp.set_lsb(0xe4, 2, 3);
        // MSI.
        cp.0[0xda] = 2;
        cp.0[0xdd] &= 0xfc;
        cp.set_bits(0xdf, 0x10, u8::from(motor.setting.is_some()));
        if let Some(setting) = motor.setting {
            let acc = setting.acc(setting.acc_normal).ok_or_else(|| {
                Error::InvalidArgument("motor setting without acceleration".into())
            })?;
            cp.set_lsb(0xe1, u32::from(acc[acc.len() - 1]), 3);
            let tables = build_step_tables(&mut cp, setting, STEP_TABLES_START, 0)?;
            self.send_step_tables(&mut cp, &tables, STEP_TABLES_START)?;
            cp.0[0xe0] = 0;
            if let Some(dec) = setting.dec(setting.dec_normal) {
                lines -= i64::from(tables.acc_steps) + dec.len() as i64;
            }
            cp.set_lsb(0x30, motor.ctpc, 3);
            cp.set_lsb(0xe4, 0, 3);
        } else {
            // Constant speed: a step length from the system clock of the
            // caller's block. Not seen in the captures.
            let mut step: u32 = match regs.get(0x00) & 0x0f {
                0x00 => 0x0089_5440,
                0x08 | 0x01 => 0x00b7_1b00,
                0x02 => 0x0112_a880,
                0x0a | 0x03 => 0x016e_3600,
                0x04 => 0x0225_5100,
                0x0c => 0x02dc_6c00,
                0x05 => 0x044a_a200,
                0x0d => 0x05b8_d800,
                0x09 => 0x00f4_2400,
                0x0b => 0x01e8_4800,
                _ => 0x0478_f7f8,
            };
            step /= u32::from(cp.get(0x96) & 0x3f) + 1;
            step = step.checked_div(motor.ctpc).unwrap_or(step);
            cp.set_lsb(0x30, step, 3);
            cp.set_lsb(0xe1, step, 3);
        }
        let top = (lines - 1).clamp(0, 0x000f_ffff) as u32;
        setup_coords(&mut cp, 100, top, 800, 1);
        // Head movement on.
        cp.set_bits(0xd8, 0x80, 1);
        self.motor_release()?;
        self.warm_reset()?;
        self.write_block(&cp)?;
        self.execute()?;
        self.wait_scan_end(Duration::from_secs(10))?;
        self.wait_scan_end(Duration::from_secs(20))
    }

    /// Brings the carriage home if it is not there (`Head_ParkHome`).
    /// Waits for a running operation first. Returns `true` if it moved.
    ///
    /// As in the HP driver there is no wait after the move beyond the 30 s
    /// of [`Chip::motor_move`] (hp3900 waits 15 s more): it is an error if
    /// the chip is still running then.
    pub fn head_park_home(&mut self, regs: &Registers, motor: &MotorMove) -> Result<bool> {
        self.wait_scan_end(Duration::from_millis(0x3a98))?;
        if self.head_is_at_home()? {
            return Ok(false);
        }
        let position = MotorPosition {
            direction: Direction::Backward,
            e801_bit2: true,
            steps: 0x4e20,
        };
        if !self.motor_move(regs, motor, &position)? {
            return Err(Error::Transport(
                "the carriage was still moving after 30 s".into(),
            ));
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motor_curves::SETTING_9;

    fn steps(bytes: &[u8]) -> Vec<u32> {
        bytes
            .chunks(3)
            .map(|c| u32::from_be_bytes([0, c[0], c[1], c[2]]))
            .collect()
    }

    #[test]
    fn park_tables_match_the_hp_driver() {
        // Capture 11, 19.119 s: 1728 bytes, registers of the block that
        // follows.
        let mut regs = Registers::default();
        regs.set_lsb(0xe1, 166, 3);
        regs.set_lsb(0x30, 1991, 3);
        let tables = build_step_tables(&mut regs, &SETTING_9, STEP_TABLES_START, 0).unwrap();
        assert_eq!(tables.bytes.len(), 1728);
        assert_eq!(tables.acc_steps, 210);
        let s = steps(&tables.bytes);
        assert_eq!(&s[..7], &[4977, 3136, 3136, 3136, 3136, 3136, 2036]);
        assert_eq!(&s[205..212], &[168, 168, 167, 167, 166, 166, 168]);
        assert_eq!(&s[263..268], &[998, 1664, 3136, 4977, 3136]);
        assert_eq!(s[575], 455);
        let pointers: Vec<u32> = [0xf6, 0xf8, 0xfa, 0xfc, 0xfe, 0x100, 0x102]
            .iter()
            .map(|&o| regs.get_lsb(o, 2) & 0x3fff)
            .collect();
        assert_eq!(pointers, [768, 1398, 1569, 1665, 1761, 1857, 2493]);
        let last: Vec<u32> = [0xe1, 0xe4, 0xe7, 0xea, 0xed, 0xf0, 0xf3]
            .iter()
            .map(|&o| regs.get_lsb(o, 3))
            .collect();
        assert_eq!(last, [166, 455, 166, 3136, 3136, 3136, 455]);
    }

    #[test]
    fn shared_curves_share_the_table() {
        let mut setting = SETTING_9;
        setting.acc_park = setting.acc_normal;
        setting.dec_normal = setting.dec_smearing;
        let mut regs = Registers::default();
        regs.set_lsb(0xe1, 166, 3);
        regs.set_lsb(0x30, 1991, 3);
        let tables = build_step_tables(&mut regs, &setting, STEP_TABLES_START, 0).unwrap();
        assert_eq!(regs.get_lsb(0x100, 2), regs.get_lsb(0xf6, 2));
        assert_eq!(regs.get_lsb(0xfe, 2), regs.get_lsb(0xfc, 2));
        assert_eq!(regs.get_lsb(0xed, 3), regs.get_lsb(0xf0, 3));
        // Acceleration 210, buffer full 57, smearing 32 + 32, park 1.
        assert_eq!(tables.bytes.len(), (332 * 3usize).next_multiple_of(16));
    }

    #[test]
    fn coords_split_the_high_bits() {
        let mut regs = Registers::default();
        setup_coords(&mut regs, 100, 0x1_2345, 800, 0x1_0000);
        assert_eq!(regs.get_lsb(0xb0, 2), 100);
        assert_eq!(regs.get_lsb(0xb2, 2), 900);
        assert_eq!(regs.get_lsb(0xd0, 2), 0x2345);
        assert_eq!(regs.get_lsb(0xd2, 2), 0x2345);
        assert_eq!(regs.get(0xd4), 0x21);
    }
}
