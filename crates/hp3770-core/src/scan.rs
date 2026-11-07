// SPDX-License-Identifier: GPL-2.0-or-later
//! The scan itself: start, image data, stop.
//!
//! Translated from hp3900 (`RTS_Scanner_StartScan`, `Read_Image`,
//! `RTS_Scanner_StopScan`) and checked against the HP driver in the
//! captures, where its read loop differs: it moves the image in pieces of
//! 64512 bytes, each waited for in the chip buffer, inside DMA transfers of
//! at most 195 pieces.

use crate::error::{Error, Result};
use crate::init::Registers;
use crate::motor::park_registers;
use crate::profile::Profile;
use crate::protocol::DmaDirection;
use crate::read::Tables;
use crate::setup::{self, GainOffset, Hardware, ScanParams};
use crate::status::Lamp;
use crate::transport::{Chip, Transport};
use std::time::{Duration, Instant};

/// Bytes moved by one bulk read during a scan, as the HP driver does.
pub const PIECE: usize = 64512;

/// Bytes of one DMA transfer: `DMASetLength=0x600000` words in
/// `[DMA_PARAM]`, rounded down to whole pieces (12579840 in the captures).
pub const CHUNK: usize = 0x60_0000 * 2 / PIECE * PIECE;

/// Margin of the last waits (`Reading_Wait` with `op`): the HP driver reads
/// a piece once the chip holds all but this many bytes of it.
const LAST_MARGIN: usize = 0x450;

/// Longest wait without new data. The first one covers the carriage
/// travelling to the top of the area (4.8 s for 13 cm at 300 dpi in
/// capture 09).
const DATA_TIMEOUT: Duration = Duration::from_secs(60);

/// Longest wait for the first data while the chip is still running. At
/// 2400 dpi the carriage travels to the top of the area at scan speed:
/// about 7.5 s per centimetre (capture 06b), so more than 3 minutes for the
/// bottom of an A4.
const TRAVEL_TIMEOUT: Duration = Duration::from_secs(600);

/// DMA channel of the image data.
const IMAGE_CHANNEL: u16 = 8;

/// DMA channel through which the chip RAM is read.
const RAM_CHANNEL: u16 = 4;

/// Bytes read from the chip RAM before the calibration and thrown away.
const DRAIN_SIZE: usize = 512;

/// Longest wait for the carriage to reach home before a calibration
/// (`Scan_Start` in hp3900).
const HOME_TIMEOUT: Duration = Duration::from_secs(10);

/// Time given to the lamp to warm up when a scan has to switch it on
/// (`overdrive_flb` in hp3900). The HP driver has not been captured doing
/// it.
pub const LAMP_WARMUP: Duration = Duration::from_secs(10);

/// A running scan.
#[derive(Debug, Clone)]
pub struct Scan {
    /// Bytes of one line sent by the chip (colours interleaved).
    pub bytes_per_line: usize,
    /// Lines the chip sends, extra lines for software line arrangement
    /// included.
    pub lines: usize,
    /// Bytes read so far.
    read: usize,
    /// Bytes left in the current DMA transfer.
    chunk_left: usize,
}

impl Scan {
    /// Bytes of the whole image.
    pub fn total(&self) -> usize {
        self.bytes_per_line * self.lines
    }

    /// Bytes read so far.
    pub fn bytes_read(&self) -> usize {
        self.read
    }

    /// Every byte has been read.
    pub fn is_done(&self) -> bool {
        self.read >= self.total()
    }
}

impl<T: Transport> Chip<T> {
    /// Gets the scanner ready for a calibration and scan: the HP driver's
    /// sequence before the first calibration read (capture 09,
    /// 2.166-2.358 s; the same in every scan capture).
    ///
    /// Lamp check (twice, as the HP driver), idle check, initial block
    /// written as it is in memory, lamp timer off (written twice), lamp
    /// duty cycle read, CCD channels on, wait for the carriage at home,
    /// warm reset, DMA reset with a read of the chip RAM whose data is
    /// thrown away, lamp check, lamp PWM.
    ///
    /// Not done, unlike the HP driver: the EEPROM requests in the middle
    /// (words 0x78-0x7A and 0x6A-0x6E read, 0x78 and 0x79 written back:
    /// lamp and gain/offset counters). This driver never writes the EEPROM
    /// and calibrates fully every time, so it has no use for them.
    ///
    /// If the lamp is off (never in the captures: the start-up switches it
    /// on and the chip only does so after about ten idle minutes), it is
    /// switched on and given [`LAMP_WARMUP`] to warm up, as hp3900 does.
    /// Fails if the carriage is not at home after 10 s: the calibration
    /// needs the white strip under the lid.
    pub fn prepare_scan(&mut self, init: &mut Registers, profile: &Profile) -> Result<()> {
        self.lamp_status()?;
        let lamp_was_on = self.lamp_status()? == Lamp::Flatbed;
        if !lamp_was_on {
            self.lamp_flatbed(init, true)?;
        }
        if self.read_byte(0xe800)? & 0x80 != 0 {
            return Err(Error::Transport("the chip is still running".into()));
        }
        self.write_block(init)?;
        self.read_word(0xe946)?;
        self.lamp_status_timer_set(init, 0)?;
        self.lamp_status_timer_set(init, 0)?;
        self.lamp_pwm_duty_cycle()?;
        self.enable_ccd(init, 0x0f)?;
        let deadline = Instant::now() + HOME_TIMEOUT;
        while !self.head_is_at_home()? {
            if Instant::now() >= deadline {
                return Err(Error::Transport("the carriage is not at home".into()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.warm_reset()?;
        // The HP driver resets the DMA engine (with a word it never sets,
        // 54 00) and reads 512 bytes of the chip RAM, which it does not
        // use. The scanner answered with no data in every capture made
        // with a good power supply, with 512 bytes in some made with the
        // weak one: what is left of an earlier transfer is drained.
        self.dma_control(0)?;
        self.dma_enable(DmaDirection::Read, RAM_CHANNEL, DRAIN_SIZE as u32, 0)?;
        let mut drain = [0; DRAIN_SIZE];
        self.transport_mut().bulk_in(&mut drain)?;
        // Lamp_Warmup.
        self.lamp_status()?;
        if !lamp_was_on {
            std::thread::sleep(LAMP_WARMUP);
        }
        self.lamp_pwm_setup(init, profile.fixed_pwm)
    }

    /// Starts a scan: the HP driver's sequence after the calibration
    /// (capture 09, 6.70-7.38 s). Motor step tables, first block, gamma and
    /// shading tables, motor current of the scan resolution, idle check,
    /// motor release, warm reset, second block, start, DMA lock.
    ///
    /// Not done, unlike the HP driver: the scan counter in the EEPROM
    /// (bytes 0x21-0x24, read and written back plus one, after reading
    /// `E9A1`). This driver never writes the EEPROM.
    pub fn start_scan(
        &mut self,
        init: &Registers,
        profile: &Profile,
        params: &ScanParams,
        hw: &Hardware,
        gain_offset: &GainOffset,
        tables: &Tables,
    ) -> Result<Scan> {
        if params.width == 0 || params.height == 0 {
            return Err(Error::InvalidArgument("empty scan".into()));
        }
        let (mut regs, params, info) =
            setup::scan_registers(init, profile, params, hw, gain_offset)?;
        if let Some(steps) = &info.step_tables {
            self.send_step_tables(&mut regs, steps, info.tables_start)?;
        }
        self.write_block(&regs)?;
        if !tables.is_empty() {
            self.send_tables(&mut regs, profile, tables)?;
        }
        let e954 = self.read_word(0xe954)?.to_le_bytes();
        setup::finish_scan_registers(&mut regs, profile, e954, &params);
        // Motor_Change on the value just read. The block then changes the
        // lamp gain bits of the same registers (capture 06: 80 01 written,
        // 40 00 in the block).
        if profile.motor_current.is_some() {
            let level = crate::scan_modes::motor_current_level(params.resolution_x).min(3);
            self.write_registers(0xe954, &[(e954[0] & 0xcf) | level << 4, e954[1]])?;
        }
        // The HP driver checks here that the chip is idle (RTS_IsExecuting);
        // it always is in the captures.
        if self.read_byte(0xe800)? & 0x80 != 0 {
            return Err(Error::Transport("the chip is still running".into()));
        }
        self.motor_release()?;
        self.warm_reset()?;
        self.write_block(&regs)?;
        self.execute()?;
        // SetLock on the chip's own value of EE00.
        let ee00 = self.read_byte(0xee00)?;
        self.write_byte(0xee00, ee00 | 0x04)?;
        Ok(Scan {
            bytes_per_line: info.bytes_per_line as usize,
            lines: (params.height + info.extra_lines) as usize,
            read: 0,
            chunk_left: 0,
        })
    }

    /// Reads the next piece of the image into `buf` (resized to it) and
    /// returns its size, 0 once the image is complete.
    ///
    /// Each piece is waited for in the chip buffer first. The DMA transfers
    /// are set up as the HP driver does: the first after the wait for its
    /// first piece, the others right after the last piece of the previous. The last two
    /// pieces are taken when the chip holds all but [`LAST_MARGIN`] bytes,
    /// or when it has stopped (`E800` bit 7), as the HP driver does.
    pub fn read_scan(&mut self, scan: &mut Scan, buf: &mut Vec<u8>) -> Result<usize> {
        let left = scan.total() - scan.read;
        if left == 0 {
            buf.clear();
            return Ok(0);
        }
        let first = scan.read == 0;
        if first {
            scan.chunk_left = left.min(CHUNK);
        }
        let piece = scan.chunk_left.min(PIECE);
        self.wait_scan_data(piece, left < piece + PIECE)?;
        if first {
            self.dma_enable(DmaDirection::Read, IMAGE_CHANNEL, scan.chunk_left as u32, 0)?;
        }
        buf.resize(piece, 0);
        let mut got = 0;
        while got < piece {
            let n = self.transport_mut().bulk_in(&mut buf[got..])?;
            if n == 0 {
                self.dma_cancel()?;
                return Err(Error::Transport("no image data".into()));
            }
            got += n;
        }
        scan.read += piece;
        scan.chunk_left -= piece;
        // The next transfer is set up right after the last piece of the
        // previous one, before waiting for data (capture 04, 12.753 s).
        if scan.chunk_left == 0 && !scan.is_done() {
            scan.chunk_left = (scan.total() - scan.read).min(CHUNK);
            self.dma_enable(DmaDirection::Read, IMAGE_CHANNEL, scan.chunk_left as u32, 0)?;
        }
        Ok(piece)
    }

    /// Waits until the chip holds `size` bytes of the image
    /// (`Reading_Wait`, colour, 8 bits: `EF16` × 96). With `near_end`, also
    /// stops waiting when only [`LAST_MARGIN`] bytes are missing or the
    /// chip has stopped.
    fn wait_scan_data(&mut self, size: usize, near_end: bool) -> Result<()> {
        let mut last = usize::MAX;
        let start = Instant::now();
        let mut deadline = start + DATA_TIMEOUT;
        loop {
            let amount = 3 * 32 * self.read_u32(0xef16)? as usize;
            if near_end && (amount + LAST_MARGIN >= size || self.read_byte(0xe800)? & 0x80 == 0) {
                return Ok(());
            }
            if amount >= size {
                return Ok(());
            }
            if amount != last {
                last = amount;
                deadline = Instant::now() + DATA_TIMEOUT;
            } else if Instant::now() >= deadline {
                // No new data for a while: fine as long as the chip is still
                // running (the carriage on its way), up to TRAVEL_TIMEOUT.
                // Never reached in the captures.
                let running = self.read_byte(0xe800)? & 0x80 != 0;
                if !running || start.elapsed() >= TRAVEL_TIMEOUT {
                    return Err(Error::Transport(format!(
                        "scan data: {amount} of {size} bytes after {} s{}",
                        start.elapsed().as_secs(),
                        if running {
                            ""
                        } else {
                            ", the chip has stopped"
                        }
                    )));
                }
                deadline = Instant::now() + DATA_TIMEOUT;
            }
        }
    }

    /// Ends a scan (`RTS_Scanner_StopScan` without waiting): DMA reset,
    /// `EE0A`-`EE0B` from `init` without bits 6 of `EE0A` and 4 of `EE0B`,
    /// motor current 3, CCD channels off, lamp timer bit off as the HP driver
    /// does.
    ///
    /// When the scan mode has no motor curves (600 dpi and up) the chip
    /// does not bring the carriage back by itself (`E801` bit 1 clear): the
    /// carriage is then parked here, before the CCD channels and the lamp
    /// timer, as in capture 06. Returns whether it was parked.
    pub fn stop_scan(&mut self, init: &mut Registers, profile: &Profile) -> Result<bool> {
        self.dma_control(0)?;
        init.0[0x60b] &= !0x10;
        init.0[0x60a] &= !0x40;
        self.write_registers(0xee0a, &init.0[0x60a..0x60c])?;
        if let Some(level) = profile.motor_current {
            self.motor_change(init, level)?;
        }
        std::thread::sleep(Duration::from_millis(200));
        let park = self.read_byte(0xe801)? & 0x02 == 0 && !self.head_is_at_home()?;
        if park {
            // Clears the execution bit first.
            init.0[0] &= 0x7f;
            self.write_byte(0xe800, init.0[0])?;
            self.head_park_home(&park_registers(init), &profile.park)?;
        }
        self.enable_ccd(init, 0)?;
        // hp3900 sets the timer to 13 minutes here. The HP driver clears its
        // enable bit instead (E946 = C9 in every capture, as at the start of
        // the calibration), reading E946 first and writing twice as at
        // start-up; the lamp still goes off by itself after about ten idle
        // minutes (capture 16).
        self.read_word(0xe946)?;
        self.lamp_status_timer_set(init, 0)?;
        self.lamp_status_timer_set(init, 0)?;
        Ok(park)
    }

    /// Switches the CCD channels on or off (`RTS_Enable_CCD`): bits 5-7 of
    /// `E810` and bit 7 of `E813` from `channels`, on the chip's own values.
    pub fn enable_ccd(&mut self, init: &mut Registers, channels: u8) -> Result<()> {
        let mut data = self.read_registers(0xe810, 4)?;
        data[0] = (data[0] & 0x1f) | (channels & 0x07) << 5;
        data[3] = (data[3] & 0x7f) | (channels >> 3 & 1) << 7;
        init.0[0x10..0x14].copy_from_slice(&data);
        self.write_registers(0xe810, &data)
    }
}
