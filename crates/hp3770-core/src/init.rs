// SPDX-License-Identifier: GPL-2.0-or-later
//! Start-up of the chip: the USB part of `Init_USBData` and `Init_Registers`.
//!
//! The order of the requests follows the HP driver as seen in the captures,
//! which is `hp3900` with two small differences, noted where they occur:
//! the RAM type is set directly instead of being probed, and `Motor_Change`
//! writes a whole word.

use crate::error::{Error, Result};
use crate::profile::{Profile, Sscg};
use crate::transport::{Chip, Transport};
use std::time::{Duration, Instant};

/// Length of the register block as the driver keeps it in memory
/// (`RT_BUFFER_LEN`). The chip is read with one byte less.
pub const BLOCK_LEN: usize = 0x071a;
/// First register of the block.
pub const BLOCK_START: u16 = 0xe800;

/// A copy of the register block `E800`-`EF19`, indexed from `E800`.
#[derive(Clone, PartialEq, Eq)]
pub struct Registers(pub Box<[u8; BLOCK_LEN]>);

impl std::fmt::Debug for Registers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registers").finish_non_exhaustive()
    }
}

impl Default for Registers {
    fn default() -> Self {
        Self(Box::new([0; BLOCK_LEN]))
    }
}

impl Registers {
    /// Byte at `offset` from `E800`.
    pub fn get(&self, offset: usize) -> u8 {
        self.0[offset]
    }

    /// Sets the bits of `mask` at `offset` to `value`, shifted to the lowest
    /// bit of the mask (`data_bitset` in hp3900).
    pub fn set_bits(&mut self, offset: usize, mask: u8, value: u8) {
        if mask == 0 {
            return;
        }
        let shifted = value.wrapping_shl(mask.trailing_zeros());
        self.0[offset] = (self.0[offset] & !mask) | (shifted & mask);
    }
}

impl<T: Transport> Chip<T> {
    /// `Read_FE3E`.
    pub fn read_fe3e(&mut self) -> Result<u8> {
        self.read_byte(0xfe3e)
    }

    /// Sets bit 0 of `F8FF`, the first step of `Init_USBData`.
    pub fn enable_usb_data(&mut self) -> Result<()> {
        let data = self.read_byte(0xf8ff)?;
        self.write_byte(0xf8ff, data | 0x01)
    }

    /// Programs the spread-spectrum clock (`SSCG_Enable`).
    pub fn sscg_enable(&mut self, sscg: Sscg) -> Result<()> {
        let data1 = self.read_byte(0xfe3a)?;
        let data2 = self.read_byte(0xfe39)?;
        let word = if sscg.enable {
            let low = if sscg.mode == 0 {
                data2 & 0x7f
            } else {
                data2 | 0x80
            };
            let high = (data1 & 0xf3) | (((sscg.clock & 0x03) | 0x04) << 2);
            u16::from_le_bytes([low, high])
        } else {
            u16::from_le_bytes([data2, data1 & 0xef])
        };
        self.write_word(0xfe39, word)
    }

    /// Reads the whole register block (`RTS_ReadRegs`).
    pub fn read_block(&mut self) -> Result<Registers> {
        let data = self.read_registers(BLOCK_START, (BLOCK_LEN - 1) as u16)?;
        let mut regs = Registers::default();
        regs.0[..data.len()].copy_from_slice(&data);
        Ok(regs)
    }

    /// Selects the type of the external RAM (`RTS_DMA_SetType`).
    pub fn dma_set_type(&mut self, regs: &mut Registers, ram_type: u8) -> Result<()> {
        const EF08: usize = 0x708;
        regs.set_bits(EF08, 0x08, 0);
        self.write_byte(0xef08, regs.get(EF08))?;
        regs.set_bits(EF08, 0xe0, ram_type);
        self.write_byte(0xef08, regs.get(EF08))?;
        regs.set_bits(EF08, 0x08, 1);
        self.write_byte(0xef08, regs.get(EF08))
    }

    /// Waits until the DMA engine is ready (`RTS_DMA_WaitReady`, bit 0 of
    /// `EF09`).
    pub fn dma_wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let end = Instant::now() + timeout;
        loop {
            if self.read_byte(0xef09)? & 0x01 != 0 {
                return Ok(());
            }
            if Instant::now() >= end {
                return Err(Error::Transport("DMA engine not ready".into()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Sets the motor current bits of `E954` (`Motor_Change`). Like the HP
    /// driver, writes the word just read instead of re-reading `E955`.
    pub fn motor_change(&mut self, regs: &mut Registers, value: u8) -> Result<()> {
        let mut data = self.read_word(0xe954)? & 0xffcf;
        data |= match value {
            3 => 0x30,
            2 => 0x20,
            1 => 0x10,
            _ => 0,
        };
        regs.0[0x154] = data as u8;
        self.write_word(0xe954, data)
    }

    /// Releases the motor (`Motor_Release`, bit 2 of `E8D9`).
    pub fn motor_release(&mut self) -> Result<()> {
        let data = self.read_byte(0xe8d9)?;
        self.write_byte(0xe8d9, data | 0x04)
    }

    /// The register traffic of `Init_USBData`, in the order of the HP driver:
    /// USB data enable, spread spectrum, block read, RAM type, motor current,
    /// motor release. Returns the block as read, with the bits these steps
    /// set. The in-memory adjustments of `Init_Registers` and
    /// `Init_USBData` that write nothing come in a later step.
    pub fn init_usb_data(&mut self, profile: &Profile) -> Result<Registers> {
        self.enable_usb_data()?;
        self.sscg_enable(profile.sscg)?;
        let mut regs = self.read_block()?;
        self.read_fe3e()?;
        // hp3900 probes the RAM type (RTS_DMA_CheckType); the HP driver sets
        // it directly.
        self.dma_set_type(&mut regs, profile.ram_type)?;
        self.dma_wait_ready(Duration::from_millis(1500))?;
        if let Some(value) = profile.motor_current {
            self.motor_change(&mut regs, value)?;
        }
        self.motor_release()?;
        Ok(regs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_bits_shifts_to_the_mask() {
        let mut r = Registers::default();
        r.0[0x26] = 0xff;
        r.set_bits(0x26, 0x70, 5);
        assert_eq!(r.get(0x26), 0xdf);
        r.set_bits(0x26, 0x0f, 0);
        assert_eq!(r.get(0x26), 0xd0);
        r.set_bits(0x12, 0xc0, 3);
        assert_eq!(r.get(0x12), 0xc0);
    }
}
