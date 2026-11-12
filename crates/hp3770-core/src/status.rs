// SPDX-License-Identifier: GPL-2.0-or-later
//! Scanner state that can be read without changing anything.
//!
//! Each field is the translation of one `hp3900` helper; the registers are
//! only read, so [`Chip::status`] is safe to call at any time, also through
//! a [`crate::transport::ReadOnly`] transport.

use crate::error::Result;
use crate::transport::{buttons_from_e968, Chip, Transport};
use std::fmt;

/// Which lamp is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lamp {
    /// Both lamps off.
    Off,
    /// The flatbed lamp, in the carriage.
    Flatbed,
    /// The lamp of the transparency adapter (TMA).
    Transparency,
}

impl Lamp {
    /// The lamp `E946` asks for (`Lamp_Status_Get`, as hp3900 does for the
    /// RTS8822BL-03A): bit 6 the flatbed lamp, bit 5 the transparency
    /// adapter's. The adapter's only lights if bit 6 of `E965` is set too
    /// (see [`Lamp::from_registers`]); the flatbed lamp wins if both are
    /// asked for.
    pub fn from_e946(e946: u8) -> Self {
        if e946 & 0x40 != 0 {
            Self::Flatbed
        } else if e946 & 0x20 != 0 {
            Self::Transparency
        } else {
            Self::Off
        }
    }

    /// Which lamp is lit, from `E946` and `E965`. Found on the HP 3770 on
    /// 8 November 2026, with the lid's light checked by eye and bit 4 of
    /// `E969` as witness: the adapter's lamp lights with bit 5 of `E946`
    /// and bit 6 of `E965` both set, and with neither alone; bit 6 of
    /// `E946` is the flatbed lamp whatever `E965` holds; bit 7 of `E965`,
    /// the lamp timer bit of `E946` and `E955` play no part.
    pub fn from_registers(e946: u8, e965: u8) -> Self {
        match Self::from_e946(e946) {
            Self::Transparency if e965 & 0x40 == 0 => Self::Off,
            lamp => lamp,
        }
    }
}

impl fmt::Display for Lamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Flatbed => "flatbed lamp on",
            Self::Transparency => "transparency adapter lamp on",
        })
    }
}

/// A snapshot of the scanner state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Connected at USB 2.0 speed (`RTS_USBType`, bit 0 of `FE11`).
    pub usb2: bool,
    /// Chipset identifier (`Chipset_ID`, `FE3C`).
    pub chipset_id: u8,
    /// The chip finished its power-on initialisation (`RTS_WaitInitEnd`,
    /// bit 3 of `F910`).
    pub init_done: bool,
    /// An operation is running (`RTS_IsExecuting`, bit 7 of `E800`).
    pub executing: bool,
    /// The carriage is at its home position (`Head_IsAtHome`, bit 6 of
    /// `E96F`).
    pub head_at_home: bool,
    /// Lamp state (`Lamp_Status_Get`).
    pub lamp: Lamp,
    /// The transparency adapter is plugged in (`RTS_isTmaAttached`: bit 1 of
    /// `E969` is 0 when it is).
    pub tma_attached: bool,
    /// The transparency adapter's lamp is lit: bit 4 of `E969`, set exactly
    /// when the lid's light was on in the tries of 8 November 2026.
    pub tma_lamp_lit: bool,
    /// Buttons held down, one bit each (see [`crate::device::Button::mask`]).
    pub buttons_held: u8,
    /// `E829`-`E82A`, which `IsScannerLinked` reads to tell the first
    /// connection after power-on (0) from a later one. Kept raw: the HP
    /// driver may use it differently.
    pub link_word: u16,
}

impl<T: Transport> Chip<T> {
    /// Reads the scanner state. Register reads only.
    pub fn status(&mut self) -> Result<Status> {
        let fe11 = self.read_byte(0xfe11)?;
        let chipset_id = self.read_byte(0xfe3c)?;
        let f910 = self.read_byte(0xf910)?;
        let e800 = self.read_byte(0xe800)?;
        let link_word = self.read_word(0xe829)?;
        let e946 = self.read_byte(0xe946)?;
        let e965 = self.read_byte(0xe965)?;
        let e968 = self.read_registers(0xe968, 2)?;
        let e96f = self.read_byte(0xe96f)?;
        Ok(Status {
            usb2: fe11 & 0x01 != 0,
            chipset_id,
            init_done: f910 & 0x08 != 0,
            executing: e800 & 0x80 != 0,
            head_at_home: e96f & 0x40 != 0,
            lamp: Lamp::from_registers(e946, e965),
            tma_attached: e968[1] & 0x02 == 0,
            tma_lamp_lit: e968[1] & 0x10 != 0,
            buttons_held: buttons_from_e968(e968[0]),
            link_word,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::protocol::{Command, SetupPacket};
    use std::collections::HashMap;
    use std::time::Duration;

    /// Answers register reads from a fixed register map.
    struct Registers(HashMap<u16, u8>);

    impl Transport for Registers {
        fn control_in(&mut self, setup: SetupPacket) -> Result<Vec<u8>> {
            match Command::decode(setup, &[]) {
                Command::ReadRegisters { address, length } => Ok((0..length)
                    .map(|i| *self.0.get(&(address + i)).unwrap_or(&0))
                    .collect()),
                other => Err(Error::Transport(format!("unexpected {other}"))),
            }
        }
        fn control_out(&mut self, _: SetupPacket, _: &[u8]) -> Result<()> {
            Err(Error::Transport("unexpected write".into()))
        }
        fn bulk_in(&mut self, _: &mut [u8]) -> Result<usize> {
            Err(Error::Transport("unexpected bulk".into()))
        }
        fn bulk_out(&mut self, _: &[u8]) -> Result<()> {
            Err(Error::Transport("unexpected bulk".into()))
        }
        fn interrupt_in(&mut self, _: Duration) -> Result<Option<u8>> {
            Ok(None)
        }
    }

    #[test]
    fn status_from_the_hardware_probe() {
        // Values read with `hp3770 probe` on Matteo's scanner (2026-11-07).
        let regs = [
            (0xfe11, 0x03),
            (0xfe12, 0x0a),
            (0xf910, 0x7a),
            (0xe800, 0x04),
            (0xe829, 0x24),
            (0xe82a, 0x10),
            (0xe946, 0xd9),
            (0xe954, 0xb0),
            (0xe955, 0x09),
            (0xe968, 0xbf),
            (0xe969, 0xed),
            (0xe96f, 0xc0),
        ];
        let mut chip = Chip::new(Registers(regs.into_iter().collect()));
        let s = chip.status().unwrap();
        assert!(s.usb2);
        assert!(s.init_done);
        assert!(!s.executing);
        assert!(s.head_at_home);
        assert_eq!(s.lamp, Lamp::Flatbed);
        assert!(s.tma_attached);
        assert!(!s.tma_lamp_lit);
        assert_eq!(s.buttons_held, 0);
        assert_eq!(s.link_word, 0x1024);
    }

    #[test]
    fn a_reset_chip_holds_no_button() {
        // E968 read as 00 while another program resets the chip: bit 7 is
        // 0, so the read is not every button held.
        let regs = [(0xe968, 0x00), (0xe969, 0xed)];
        let mut chip = Chip::new(Registers(regs.into_iter().collect()));
        assert_eq!(chip.status().unwrap().buttons_held, 0);
        assert_eq!(chip.buttons_held().unwrap(), 0);
        // The first button pressed: bit 0 cleared, bit 7 still 1.
        let regs = [(0xe968, 0xbe), (0xe969, 0xed)];
        let mut chip = Chip::new(Registers(regs.into_iter().collect()));
        assert_eq!(chip.status().unwrap().buttons_held, 0x01);
        assert_eq!(chip.buttons_held().unwrap(), 0x01);
    }

    #[test]
    fn lamp_bits() {
        // During a scan the HP driver writes E946 = C9 then 89, E955 = 09.
        assert_eq!(Lamp::from_e946(0xc9), Lamp::Flatbed);
        assert_eq!(Lamp::from_e946(0x89), Lamp::Off);
        // The tries of 8 November 2026 (E946, E965, the lid's light).
        assert_eq!(Lamp::from_registers(0xa9, 0xcc), Lamp::Transparency);
        assert_eq!(Lamp::from_registers(0xa9, 0x4c), Lamp::Transparency);
        assert_eq!(Lamp::from_registers(0xb9, 0xcc), Lamp::Transparency);
        assert_eq!(Lamp::from_registers(0xa9, 0x8c), Lamp::Off);
        assert_eq!(Lamp::from_registers(0x89, 0xcc), Lamp::Off);
        assert_eq!(Lamp::from_registers(0xd9, 0xcc), Lamp::Flatbed);
        assert_eq!(Lamp::from_registers(0xd9, 0x0c), Lamp::Flatbed);
    }
}
