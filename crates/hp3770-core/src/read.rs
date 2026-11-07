// SPDX-License-Identifier: GPL-2.0-or-later
//! One read of the chip (`RTS_GetImage` in hp3900): the calibration strips
//! and, with a different block, the scan.
//!
//! The register blocks come from [`crate::setup`]; this module sends them,
//! starts the chip and collects the image through the DMA engine, in the
//! order the HP driver does it in the captures.

use crate::error::{Error, Result};
use crate::init::Registers;
use crate::profile::Profile;
use crate::protocol::DmaDirection;
use crate::setup::{self, GainOffset, Hardware, ScanParams};
use crate::transport::{Chip, Transport};
use std::time::{Duration, Instant};

/// Longest wait for new image data before giving up (`Reading_Wait`: the
/// clock restarts every time the amount in the chip buffer grows).
const DATA_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest single DMA transfer (`RTS_GetImage_GetBuffer`: half-size limit
/// 0x1FFE0).
const MAX_TRANSFER: usize = 0x1ffe0 * 2;

/// DMA channel of the image data.
const IMAGE_CHANNEL: u16 = 8;

/// DMA channel through which the chip RAM is read back.
const RAM_CHANNEL: u16 = 4;

/// Unit of the shading read-back (`Calibrate_Malloc` with USB 2.0: 0x200).
const READBACK_UNIT: usize = 0x200;

/// Attempts at sending each shading table (`Shading_black_apply`: 11).
const SHADING_TRIES: usize = 11;

/// Tables a read sends to the chip RAM after its first block: gamma and the
/// black and white shading of each colour, as the bytes the chip takes
/// (little-endian words for shading). The registers of the block say
/// whether the chip uses them; a table that is not used is not sent.
#[derive(Debug, Clone, Copy, Default)]
pub struct Tables<'a> {
    /// The three gamma tables, one after the other.
    pub gamma: Option<&'a [u8]>,
    /// Black shading of red, green and blue.
    pub black: Option<[&'a [u8]; 3]>,
    /// White shading of red, green and blue.
    pub white: Option<[&'a [u8]; 3]>,
}

impl Tables<'_> {
    /// No table at all: the HP driver then skips the whole transfer.
    pub fn is_empty(&self) -> bool {
        self.gamma.is_none() && self.black.is_none() && self.white.is_none()
    }
}

impl<T: Transport> Chip<T> {
    /// Reads a calibration strip (`RTS_GetImage`) and returns its bytes:
    /// `bytes_per_line × height`, 8 bits per channel, colours interleaved.
    ///
    /// `init` is the in-memory initial block (`init_regs` in hp3900); the
    /// read is set up on a copy of it, and `Motor_Change` at the end updates
    /// its `E954` like the HP driver. `tables` holds the gamma and shading
    /// tables the read uses, sent after the first block.
    ///
    /// Sequence of the HP driver (capture 09, 2.358-2.732 s): motor step
    /// tables when the head moves, first block, tables, motor current of 1200 dpi, motor release, warm reset, second block,
    /// start, wait for the data, DMA read on channel 8, wait for the end,
    /// DMA unlock, motor current 3 on `init`.
    pub fn get_image(
        &mut self,
        init: &mut Registers,
        profile: &Profile,
        params: &ScanParams,
        hw: &Hardware,
        gain_offset: &GainOffset,
        tables: &Tables,
    ) -> Result<Vec<u8>> {
        if params.width == 0 || params.height == 0 {
            return Err(Error::InvalidArgument("empty read".into()));
        }
        let (mut regs, params, info) =
            setup::image_registers(init, profile, params, hw, gain_offset)?;
        if let Some(steps) = &info.step_tables {
            self.send_step_tables(&mut regs, steps, info.tables_start)?;
        }
        self.write_block(&regs)?;
        if !tables.is_empty() {
            self.send_tables(&mut regs, profile, tables)?;
        }
        let e954 = self.read_word(0xe954)?.to_le_bytes();
        setup::finish_image_registers(&mut regs, profile, e954, &params);
        if profile.motor_current.is_some() {
            // Motor_Change(1200 dpi): the HP driver writes the word it read
            // with the current bits of 1200 dpi (0).
            self.write_registers(0xe954, &regs.0[0x154..0x156])?;
        }
        self.motor_release()?;
        self.warm_reset()?;
        self.write_block(&regs)?;
        self.execute()?;
        let size = info.bytes_per_line as usize * params.height as usize;
        let image = self.read_image(size)?;
        self.wait_scan_end(Duration::from_millis(1500))?;
        self.set_lock(&mut regs, false)?;
        if let Some(level) = profile.motor_current {
            self.motor_change(init, level)?;
        }
        Ok(image)
    }

    /// Collects `size` bytes of image data (`RTS_GetImage_GetBuffer`): waits
    /// until the chip holds each piece, arms the DMA engine on channel 8 and
    /// reads it.
    pub fn read_image(&mut self, size: usize) -> Result<Vec<u8>> {
        let mut image = vec![0; size];
        let mut pos = 0;
        while pos < size {
            let piece = (size - pos).min(MAX_TRANSFER);
            self.wait_image_data(piece)?;
            self.dma_enable(DmaDirection::Read, IMAGE_CHANNEL, piece as u32, 0)?;
            let read = self.transport_mut().bulk_in(&mut image[pos..pos + piece])?;
            if read == 0 {
                self.dma_cancel()?;
                return Err(Error::Transport("no image data".into()));
            }
            pos += read;
        }
        Ok(image)
    }

    /// Bytes of image data waiting in the chip (`Reading_BufferSize_Get`):
    /// `EF16` counts units of 32 dots, `E812` gives the channels per dot.
    pub fn image_data_available(&mut self) -> Result<usize> {
        let channels = (self.read_byte(0xe812)? >> 6).max(1);
        let amount = self.read_u32(0xef16)?;
        Ok(usize::from(channels) * 32 * amount as usize)
    }

    /// Waits until the chip holds at least `size` bytes (`Reading_Wait`).
    /// Fails if the amount stops growing for [`DATA_TIMEOUT`]. Like the HP
    /// driver, polls without pausing: each poll is two USB requests.
    fn wait_image_data(&mut self, size: usize) -> Result<()> {
        let mut last = 0;
        let mut deadline = Instant::now() + DATA_TIMEOUT;
        loop {
            let amount = self.image_data_available()?;
            if amount >= size {
                return Ok(());
            }
            if amount != last {
                last = amount;
                deadline = Instant::now() + DATA_TIMEOUT;
            } else if Instant::now() >= deadline {
                return Err(Error::Transport(format!(
                    "image data: {amount} of {size} bytes after {} s",
                    DATA_TIMEOUT.as_secs()
                )));
            }
        }
    }

    /// Sends the gamma and shading tables of a read (the HP driver's
    /// `CdRegSetGammaShadingData`, `0x10039920` in `hpgt3770.dll`; hp3900
    /// has the same steps in `RTS_GetImage` and `Shading_apply`).
    ///
    /// Bits 4 and 6 of `EE0B` are cleared during the transfer. The gamma
    /// tables go with the DMA lock on, written and read back. Each shading
    /// table is written on channel `0x10` (black) or `0x14` (white) plus
    /// the colour, then read back through channel 4 and compared; a table
    /// that does not read back the same is sent again, up to 11 times.
    pub fn send_tables(
        &mut self,
        regs: &mut Registers,
        profile: &Profile,
        tables: &Tables,
    ) -> Result<()> {
        let saved = regs.get(0x60b) & 0x50;
        regs.0[0x60b] &= 0xaf;
        self.write_byte(0xee0b, regs.get(0x60b))?;
        if let Some(gamma) = tables.gamma {
            if regs.get(0x1d0) & 0x40 != 0 {
                let start =
                    (u32::from(regs.get(0x1b5) & 0x3f) << 8 | u32::from(regs.get(0x1b4))) >> 4;
                self.set_lock(regs, true)?;
                self.dma_write(0, start, gamma)?;
                self.set_lock(regs, false)?;
            }
        }
        let colours = profile.platform.channel_color;
        for (enabled, kind, set) in [
            (regs.get(0x1cf) & 0x08 != 0, 0x10, tables.black),
            (regs.get(0x1cf) & 0x04 != 0, 0x14, tables.white),
        ] {
            let (true, Some(set)) = (enabled, set) else {
                continue;
            };
            for (colour, table) in colours.iter().zip(set) {
                self.send_shading(regs, *colour, kind, table)?;
            }
        }
        regs.0[0x60b] |= saved;
        self.write_byte(0xee0b, regs.get(0x60b))
    }

    /// Writes one shading table and checks it, up to [`SHADING_TRIES`]
    /// times (`Shading_black_apply` / `Shading_white_apply`).
    fn send_shading(
        &mut self,
        regs: &Registers,
        colour: u8,
        kind: u16,
        table: &[u8],
    ) -> Result<()> {
        let size = u32::try_from(table.len())
            .map_err(|_| Error::InvalidArgument("shading table too long".into()))?;
        for _ in 0..SHADING_TRIES {
            self.dma_enable(DmaDirection::Write, kind | u16::from(colour), size, 0)?;
            self.transport_mut().bulk_out(table)?;
            if self.shading_reads_back(regs, colour, kind == 0x14, table)? {
                return Ok(());
            }
            self.dma_cancel()?;
        }
        Err(Error::Transport(format!(
            "shading table {:#x}: never read back the same",
            kind | u16::from(colour)
        )))
    }

    /// Reads a shading table back from the chip RAM and compares it
    /// (`fn3730` in hp3900, `0x10037910` in the HP driver).
    ///
    /// The chip keeps a table in 2 or 4 interleaved banks of 16-byte
    /// groups. The HP driver reads back the part after the first
    /// `READBACK_UNIT × (n − 1)` bytes (`n` = whole units in the table),
    /// one read of each bank, then, if there is such a first part, the
    /// groups of it that went to the first bank. hp3900 reads only the
    /// whole table at once and never compares.
    fn shading_reads_back(
        &mut self,
        regs: &Registers,
        colour: u8,
        white: bool,
        table: &[u8],
    ) -> Result<bool> {
        let banks = if regs.get(0x1bf) & 0x18 == 0
            && (regs.get(0x1cf) >> 1) & regs.get(0x1cf) & 0x04 != 0
        {
            2
        } else {
            4
        };
        let len = table.len();
        let unit = READBACK_UNIT;
        let whole = len / unit;
        let read_size = if len % unit != 0 && len >= unit {
            unit * 2
        } else {
            unit
        };
        let tail_len = if len > unit { len % unit + unit } else { len };
        let head_units = if len >= unit * 2 { whole - 1 } else { 0 };
        // In 16-byte groups.
        let skip = unit / 16 * head_units;

        let tail = &table[skip * 16..(skip * 16 + tail_len).min(len)];
        let mut parts = vec![Vec::new(); banks];
        for (i, group) in tail.chunks(16).enumerate() {
            parts[i % banks].extend_from_slice(group);
        }
        if !self.compare_banks(regs, colour, white, banks, skip, read_size, &parts)? {
            return Ok(false);
        }
        if head_units > 0 {
            let head = &table[..head_units * unit];
            let first: Vec<u8> = head
                .chunks(16)
                .step_by(banks)
                .filter(|g| g.len() == 16)
                .flatten()
                .copied()
                .collect();
            let size = (head_units * unit / banks).div_ceil(unit) * unit;
            return self.compare_banks(regs, colour, white, banks, 0, size, &[first]);
        }
        Ok(true)
    }

    /// Reads each bank of a table and compares its start with `parts`
    /// (`0x10037340` in the HP driver, `fn3330` in hp3900). `offset` is in
    /// 16-byte groups of the whole table. Stops after the last part with
    /// data.
    #[allow(clippy::too_many_arguments)]
    fn compare_banks(
        &mut self,
        regs: &Registers,
        colour: u8,
        white: bool,
        banks: usize,
        offset: usize,
        size: usize,
        parts: &[Vec<u8>],
    ) -> Result<bool> {
        // RAM address of the table of this colour, in units of 16 bytes.
        let base = match colour {
            0 => u32::from(regs.get(0x1ba)),
            1 => {
                u32::from(regs.get(0x1bf) & 1) << 16
                    | u32::from(regs.get(0x1bc)) << 8
                    | u32::from(regs.get(0x1bb))
            }
            _ => {
                u32::from((regs.get(0x1bf) >> 1) & 3) << 16
                    | u32::from(regs.get(0x1be)) << 8
                    | u32::from(regs.get(0x1bd))
            }
        };
        let mut buffer = vec![0; size];
        for (bank, part) in parts.iter().enumerate() {
            // Which bank, in the two address bits above 20: the HP driver
            // (and hp3900) number them differently for the second colour
            // and for black and white tables when there are only two.
            let order: [u32; 4] = match (banks, colour, white) {
                (2, 1, false) => [3, 2, 0, 0],
                (2, 1, true) => [1, 0, 0, 0],
                (2, _, false) => [0, 1, 0, 0],
                (2, _, true) => [2, 3, 0, 0],
                (_, 1, _) => [3, 1, 2, 0],
                _ => [0, 2, 1, 3],
            };
            let options = (base + (offset / banks) as u32) | order[bank] << 20;
            self.dma_control(0)?;
            self.dma_enable(DmaDirection::Read, RAM_CHANNEL, size as u32, options)?;
            let read = self.transport_mut().bulk_in(&mut buffer)?;
            if read < part.len() || buffer[..part.len()] != part[..] {
                return Ok(false);
            }
            if parts.get(bank + 1).is_none_or(|p| p.is_empty()) {
                break;
            }
        }
        Ok(true)
    }
}
