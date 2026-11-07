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

impl<T: Transport> Chip<T> {
    /// Reads a calibration strip (`RTS_GetImage`) and returns its bytes:
    /// `bytes_per_line × height`, 8 bits per channel, colours interleaved.
    ///
    /// `init` is the in-memory initial block (`init_regs` in hp3900); the
    /// read is set up on a copy of it, and `Motor_Change` at the end updates
    /// its `E954` like the HP driver. Shading tables, when the strip uses
    /// them, must already be in the chip RAM (see [`Chip::dma_write`]).
    ///
    /// Sequence of the HP driver (capture 09, 2.358-2.732 s): first block,
    /// motor current of 1200 dpi, motor release, warm reset, second block,
    /// start, wait for the data, DMA read on channel 8, wait for the end,
    /// DMA unlock, motor current 3 on `init`.
    pub fn get_image(
        &mut self,
        init: &mut Registers,
        profile: &Profile,
        params: &ScanParams,
        hw: &Hardware,
        gain_offset: &GainOffset,
    ) -> Result<Vec<u8>> {
        if params.width == 0 || params.height == 0 {
            return Err(Error::InvalidArgument("empty read".into()));
        }
        let (mut regs, params, info) =
            setup::image_registers(init, profile, params, hw, gain_offset)?;
        self.write_block(&regs)?;
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
}
