// SPDX-License-Identifier: GPL-2.0-or-later
//! Start-up of the chip: `Init_USBData` and `Init_Registers`.
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

    /// Stores the low `len` bytes of `value` at `offset`, little endian
    /// (`data_lsb_set` in hp3900).
    pub fn set_lsb(&mut self, offset: usize, value: u32, len: usize) {
        self.0[offset..offset + len].copy_from_slice(&value.to_le_bytes()[..len]);
    }

    /// The in-memory part of `Init_Registers`: the values hp3900 puts in the
    /// block it has just read, before any of them is written back.
    ///
    /// Differences from hp3900, to match the HP driver: the binary threshold
    /// (`E99E`-`E9A1`) keeps the chip's value, the high bits of `E963` and
    /// `E965` are not cleared, and only the CCD branch is translated (the
    /// 3770 has a CCD sensor, `SensorType=1`).
    pub fn init_registers(&mut self) {
        // CCD sensor.
        self.set_bits(0x11, 0xc0, 0);
        self.set_bits(0x146, 0x80, 1);
        self.set_bits(0x146, 0x40, 1);
        // CCD channels off.
        self.set_bits(0x10, 0xe0, 0);
        self.set_bits(0x13, 0x80, 0);
        // Lamp switch-off timer on, longest time.
        self.set_bits(0x146, 0x10, 1);
        self.0[0x147] = 0xff;
        // Last step of the acceleration curve.
        self.set_lsb(0xe1, 0x2af8, 3);
        // MSI.
        self.0[0xda] = 0x02;
        self.set_bits(0xdd, 0x03, 0);
        // hp3900 writes the binary threshold here (BinaryThresholdL/H, 127
        // and 128 in the HP configuration). The initial block of the HP
        // driver keeps the chip's 63 00 64 00: left as read.
        self.set_bits(0x01, 0x08, 0);
        self.set_bits(0x16f, 0x40, 0);
        self.0[0xbf] = (self.0[0xbf] & 0xe0) | 0x20;
        // hp3900 also clears bit 7 of E963 and bit 6 of E965. The HP driver
        // keeps them: C0 and CC in its initial block and in every execution.
        self.0[0x163] |= 0x40;
        self.set_bits(0xd6, 0x0f, 8);
        self.set_bits(0x164, 0x80, 1);
        self.0[0xbc] = 0x00;
        self.0[0xbd] = 0x00;
        self.0[0x165] |= 0x80;
        self.0[0xed] = 0x10;
        self.0[0xbe] = 0x00;
        self.0[0xd5] = 0x00;
        self.0[0xee] = 0x00;
        self.0[0xef] = 0x00;
        self.0[0xde] = 0xff;
        // No curves, unknown low bits cleared.
        self.set_bits(0xdf, 0x10, 0);
        self.set_bits(0xdf, 0x0f, 0);
        // Motor driven outside the chip (MotorType=0): no PWM settings.
        self.set_bits(0xd7, 0x80, 0);
        self.0[0x600] &= 0xfb;
        self.0[0x1d8] |= 0x08;
    }

    /// The in-memory part of `Init_USBData`, with the values of `profile`.
    /// The motor current (`Motor_Change`) is set by the register traffic.
    pub fn init_usb_data(&mut self, profile: &Profile) {
        // Gamma tables of 256 entries.
        self.set_bits(0x1d0, 0x30, 0);
        // 3 channels per dot.
        self.set_bits(0x12, 0xc0, 3);
        // System clock.
        self.set_bits(0x00, 0x0f, 5);
        // Timing cnpp.
        self.set_bits(0x96, 0x3f, 0x17);
        // Sensor channel colour order.
        self.set_bits(0x60a, 0x7f, 0x24);
        self.set_bits(0x10, 0x1f, profile.crvs);
        // Reference voltages (RTS_Setup_RefVoltages).
        for (offset, v) in (0x14..).zip(profile.ref_voltages) {
            self.set_bits(offset, 0xe0, v);
        }
        self.0[0x11] |= 0x10;
        self.set_bits(0x26, 0x70, 5);
        self.0[0x185] = 0x88;
        self.0[0x186] = 0x88;
        // SDRAM clock.
        self.set_bits(0x187, 0x0f, 0x08);
        self.set_bits(0x187, 0xf0, profile.mclkioc);
        let data = profile.mclkioc.wrapping_sub(1);
        let special = match data {
            0 => Some(0xc0),
            1 => Some(0xa1),
            2 => Some(0xe2),
            3 => Some(0x93),
            4 => Some(0xd4),
            5 => Some(0xb5),
            6 => Some(0x06),
            _ => None,
        };
        if let Some(value) = special {
            self.0[0x187] = value;
        }
        self.set_bits(0x26, 0x0f, 0);
        self.0[0x27] &= 0x3f;
        self.0[0x29] = 0x24;
        self.0[0x2a] = 0x10;
        self.0[0x150] = 0xff;
        self.0[0x151] = profile.e951;
        self.0[0x156] = 0xf0;
        self.0[0x157] = 0xfd;
        self.0[0xde] = 0;
        self.set_bits(0xdf, 0x0f, 0);
        self.0[0x104..0x124].copy_from_slice(&profile.motor_resource);
        // Set by hp3900 without a known purpose.
        self.0[0x01] |= 0x40;
        self.0[0x124] = 0x94;
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

    /// `Init_USBData` with `Init_Registers`, in the order of the HP driver:
    /// USB data enable, spread spectrum, block read, RAM type, motor current,
    /// motor release. Returns the initial register block (`init_regs` in
    /// hp3900), the base of every later operation.
    pub fn init_usb_data(&mut self, profile: &Profile) -> Result<Registers> {
        self.enable_usb_data()?;
        self.sscg_enable(profile.sscg)?;
        let mut regs = self.read_block()?;
        self.read_fe3e()?;
        regs.init_registers();
        // hp3900 probes the RAM type (RTS_DMA_CheckType); the HP driver sets
        // it directly.
        self.dma_set_type(&mut regs, profile.ram_type)?;
        self.dma_wait_ready(Duration::from_millis(1500))?;
        regs.init_usb_data(profile);
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

    #[test]
    fn set_lsb_is_little_endian() {
        let mut r = Registers::default();
        r.0[0xe4] = 0x55;
        r.set_lsb(0xe1, 0x2af8, 3);
        assert_eq!(&r.0[0xe1..0xe5], &[0xf8, 0x2a, 0x00, 0x55]);
    }

    #[test]
    fn sdram_clock_follows_mclkioc() {
        let clock = |mclkioc| {
            let mut r = Registers::default();
            r.init_usb_data(&Profile {
                mclkioc,
                ..crate::profile::HP3770
            });
            r.get(0x187)
        };
        assert_eq!(clock(8), 0x88);
        assert_eq!(clock(1), 0xc0);
        assert_eq!(clock(3), 0xe2);
        assert_eq!(clock(7), 0x06);
        assert_eq!(clock(9), 0x98);
    }
}
