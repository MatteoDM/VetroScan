// SPDX-License-Identifier: GPL-2.0-or-later
//! Transport abstraction and register helpers.
//!
//! A [`Transport`] moves raw USB transfers. [`Chip`] builds the RTS8822
//! operations on top of it, with the same semantics as the `hp3900` backend
//! (`hp3900_usb.c`), so that the translated driver logic can be checked
//! against the original byte for byte.

use crate::error::{Error, Result};
use crate::protocol::{Command, DmaDirection, SetupPacket};
use std::time::Duration;

/// Raw USB access to the scanner.
///
/// Implemented by the real USB backend and by capture replays in tests.
pub trait Transport {
    /// Control transfer, device to host. Returns the bytes received.
    fn control_in(&mut self, setup: SetupPacket) -> Result<Vec<u8>>;
    /// Control transfer, host to device.
    fn control_out(&mut self, setup: SetupPacket, data: &[u8]) -> Result<()>;
    /// Bulk read from the image endpoint into `buf`. Returns the bytes read.
    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize>;
    /// Bulk write to the table endpoint.
    fn bulk_out(&mut self, data: &[u8]) -> Result<()>;
    /// Waits for a button event on the interrupt endpoint.
    /// Returns `None` on timeout.
    fn interrupt_in(&mut self, timeout: Duration) -> Result<Option<u8>>;
}

impl<T: Transport + ?Sized> Transport for &mut T {
    fn control_in(&mut self, setup: SetupPacket) -> Result<Vec<u8>> {
        (**self).control_in(setup)
    }
    fn control_out(&mut self, setup: SetupPacket, data: &[u8]) -> Result<()> {
        (**self).control_out(setup, data)
    }
    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).bulk_in(buf)
    }
    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        (**self).bulk_out(data)
    }
    fn interrupt_in(&mut self, timeout: Duration) -> Result<Option<u8>> {
        (**self).interrupt_in(timeout)
    }
}

/// A transport that refuses every host-to-device transfer.
///
/// Used by diagnostic commands that must never change the scanner state:
/// control OUT and bulk OUT fail with [`Error::ReadOnly`] before anything
/// reaches the device. Control IN requests are limited to register and
/// EEPROM reads, the only ones known to have no side effects.
#[derive(Debug)]
pub struct ReadOnly<T>(pub T);

impl<T: Transport> Transport for ReadOnly<T> {
    fn control_in(&mut self, setup: SetupPacket) -> Result<Vec<u8>> {
        match Command::decode(setup, &[]) {
            Command::ReadRegisters { .. } | Command::ReadEeprom { .. } => self.0.control_in(setup),
            other => Err(Error::ReadOnly(other.to_string())),
        }
    }
    fn control_out(&mut self, setup: SetupPacket, data: &[u8]) -> Result<()> {
        Err(Error::ReadOnly(Command::decode(setup, data).to_string()))
    }
    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.0.bulk_in(buf)
    }
    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        Err(Error::ReadOnly(format!("bulk OUT of {} bytes", data.len())))
    }
    fn interrupt_in(&mut self, timeout: Duration) -> Result<Option<u8>> {
        self.0.interrupt_in(timeout)
    }
}

/// RTS8822 operations over a [`Transport`].
#[derive(Debug)]
pub struct Chip<T> {
    transport: T,
}

impl<T: Transport> Chip<T> {
    /// Wraps a transport.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Gives back the transport.
    pub fn into_inner(self) -> T {
        self.transport
    }

    /// Mutable access to the transport.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    fn send(&mut self, cmd: &Command) -> Result<()> {
        let (setup, data) = encode(cmd)?;
        self.transport.control_out(setup, &data)
    }

    fn fetch(&mut self, cmd: &Command, expected: usize) -> Result<Vec<u8>> {
        let (setup, _) = encode(cmd)?;
        let data = self.transport.control_in(setup)?;
        if data.len() != expected {
            return Err(Error::ShortTransfer {
                expected,
                actual: data.len(),
            });
        }
        Ok(data)
    }

    /// Reads `len` bytes of registers starting at `address`.
    pub fn read_registers(&mut self, address: u16, len: u16) -> Result<Vec<u8>> {
        self.fetch(
            &Command::ReadRegisters {
                address,
                length: len,
            },
            len.into(),
        )
    }

    /// Writes registers starting at `address`.
    pub fn write_registers(&mut self, address: u16, data: &[u8]) -> Result<()> {
        self.send(&Command::WriteRegisters {
            address,
            data: data.to_vec(),
        })
    }

    /// Reads one register. The chip always answers 2 bytes; the first is the
    /// register at `address` (`IRead_Byte`).
    pub fn read_byte(&mut self, address: u16) -> Result<u8> {
        Ok(self.read_registers(address, 2)?[0])
    }

    /// Reads a little-endian 16-bit value (`IRead_Word`).
    pub fn read_word(&mut self, address: u16) -> Result<u16> {
        let b = self.read_registers(address, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// Reads a little-endian 32-bit value (`IRead_Integer`).
    pub fn read_u32(&mut self, address: u16) -> Result<u32> {
        let b = self.read_registers(address, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Writes one register without touching its neighbour.
    ///
    /// Writes always move 2 bytes, so like `IWrite_Byte` this first reads the
    /// register at `address + 1` and writes it back unchanged.
    pub fn write_byte(&mut self, address: u16, value: u8) -> Result<()> {
        let next = address.checked_add(1).ok_or_else(|| {
            Error::InvalidArgument(format!("register {address:04X} has no neighbour"))
        })?;
        let neighbour = self.read_registers(next, 2)?[0];
        self.write_registers(address, &[value, neighbour])
    }

    /// Writes a little-endian 16-bit value (`IWrite_Word`).
    pub fn write_word(&mut self, address: u16, value: u16) -> Result<()> {
        self.write_registers(address, &value.to_le_bytes())
    }

    /// Writes a little-endian 32-bit value (`IWrite_Integer`).
    pub fn write_u32(&mut self, address: u16, value: u32) -> Result<()> {
        self.write_registers(address, &value.to_le_bytes())
    }

    /// Starts the operation programmed in the registers (`RTS_Execute`).
    ///
    /// Pulses bit 6 of `E800` with bit 6 of `E813` cleared then set, waits
    /// 100 ms and sets bit 7 of `E800`. Same sequence as `hp3900` and as the
    /// HP driver in the captures.
    pub fn execute(&mut self) -> Result<()> {
        let e800 = self.read_byte(0xe800)?;
        let e813 = self.read_byte(0xe813)? & 0xbf;
        self.write_byte(0xe813, e813)?;
        self.write_byte(0xe800, e800 | 0x40)?;
        self.write_byte(0xe813, e813 | 0x40)?;
        self.write_byte(0xe800, e800 & 0xbf)?;
        std::thread::sleep(Duration::from_millis(100));
        self.write_byte(0xe800, (e800 & 0xbf) | 0x80)
    }

    /// Reads a 16-bit EEPROM word.
    pub fn read_eeprom_word(&mut self, address: u16) -> Result<u16> {
        let b = self.fetch(&Command::ReadEeprom { address, length: 2 }, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// Arms the DMA engine for a transfer of `size` bytes.
    pub fn dma_enable(
        &mut self,
        direction: DmaDirection,
        channel: u16,
        size: u32,
        options: u32,
    ) -> Result<()> {
        if size % 2 != 0 || size / 2 > 0x00ff_ffff || options > 0x00ff_ffff {
            return Err(Error::InvalidArgument(format!(
                "DMA size {size} / options {options:#x}"
            )));
        }
        self.send(&Command::DmaEnable {
            direction,
            channel,
            options,
            size,
        })
    }

    /// Cancels the running DMA transfer.
    pub fn dma_cancel(&mut self) -> Result<()> {
        self.send(&Command::DmaCancel)
    }

    /// Writes the DMA control word; `0` resets the DMA engine.
    pub fn dma_control(&mut self, value: u16) -> Result<()> {
        self.send(&Command::DmaControl { value })
    }

    /// Resets the chipset.
    pub fn chipset_reset(&mut self) -> Result<()> {
        self.send(&Command::ChipsetReset)
    }
}

fn encode(cmd: &Command) -> Result<(SetupPacket, Vec<u8>)> {
    cmd.encode()
        .ok_or_else(|| Error::InvalidArgument(format!("cannot encode {cmd}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Records outgoing requests and answers IN requests from a queue.
    #[derive(Debug, Default)]
    struct Mock {
        sent: Vec<Command>,
        answers: VecDeque<Vec<u8>>,
    }

    impl Transport for Mock {
        fn control_in(&mut self, setup: SetupPacket) -> Result<Vec<u8>> {
            self.sent.push(Command::decode(setup, &[]));
            self.answers
                .pop_front()
                .ok_or_else(|| Error::Transport("no answer".into()))
        }
        fn control_out(&mut self, setup: SetupPacket, data: &[u8]) -> Result<()> {
            self.sent.push(Command::decode(setup, data));
            Ok(())
        }
        fn bulk_in(&mut self, _buf: &mut [u8]) -> Result<usize> {
            Ok(0)
        }
        fn bulk_out(&mut self, _data: &[u8]) -> Result<()> {
            Ok(())
        }
        fn interrupt_in(&mut self, _timeout: Duration) -> Result<Option<u8>> {
            Ok(None)
        }
    }

    #[test]
    fn read_only_blocks_writes() {
        let mut chip = Chip::new(ReadOnly(Mock {
            answers: VecDeque::from([vec![0x13, 0x02], vec![0x00, 0x55]]),
            ..Default::default()
        }));
        assert_eq!(chip.read_word(0xfe3a).unwrap(), 0x0213);
        assert_eq!(chip.read_eeprom_word(0x7c).unwrap(), 0x5500);
        assert!(matches!(
            chip.write_registers(0xe800, &[0x44, 0x5c]),
            Err(Error::ReadOnly(_))
        ));
        assert!(matches!(chip.chipset_reset(), Err(Error::ReadOnly(_))));
        assert!(matches!(
            chip.transport_mut().bulk_out(&[0; 4]),
            Err(Error::ReadOnly(_))
        ));
        // A vendor IN request that is not a register or EEPROM read.
        let odd = SetupPacket {
            request_type: 0xc0,
            request: 0x04,
            value: 0,
            index: 0x0300,
            length: 2,
        };
        assert!(matches!(
            chip.transport_mut().control_in(odd),
            Err(Error::ReadOnly(_))
        ));
        // Only the two reads reached the device.
        assert_eq!(chip.into_inner().0.sent.len(), 2);
    }

    #[test]
    fn write_byte_preserves_neighbour() {
        // Same sequence as the HP driver in 00_collegamento.pcap:
        // read FE3A -> 13 02, then write FE39 = A3 13.
        let mut chip = Chip::new(Mock {
            answers: VecDeque::from([vec![0x13, 0x02]]),
            ..Default::default()
        });
        chip.write_byte(0xfe39, 0xa3).unwrap();
        let sent = chip.into_inner().sent;
        assert_eq!(
            sent,
            [
                Command::ReadRegisters {
                    address: 0xfe3a,
                    length: 2
                },
                Command::WriteRegisters {
                    address: 0xfe39,
                    data: vec![0xa3, 0x13]
                },
            ]
        );
    }

    #[test]
    fn short_read_is_an_error() {
        let mut chip = Chip::new(Mock {
            answers: VecDeque::from([vec![0x01]]),
            ..Default::default()
        });
        assert!(matches!(
            chip.read_word(0xe800),
            Err(Error::ShortTransfer {
                expected: 2,
                actual: 1
            })
        ));
    }

    #[test]
    fn dma_enable_rejects_odd_size() {
        let mut chip = Chip::new(Mock::default());
        assert!(chip.dma_enable(DmaDirection::Read, 0, 3, 0).is_err());
    }
}
