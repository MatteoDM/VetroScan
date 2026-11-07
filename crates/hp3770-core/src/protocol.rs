// SPDX-License-Identifier: GPL-2.0-or-later
//! USB control-request layer of the RTS8822.
//!
//! Every chip operation is a vendor control request with `bRequest = 0x04`.
//! `wValue` carries a register (or EEPROM / DMA channel) address and `wIndex`
//! selects the operation. The mapping below comes from the SANE `hp3900`
//! backend (`hp3900_usb.c`, `hp3900_rts8822.c`) and is confirmed by the USB
//! captures of the original HP driver for the 3770.
//!
//! | `wIndex` | Direction | Operation                                   |
//! |----------|-----------|---------------------------------------------|
//! | `0x0000` | OUT       | write registers starting at `wValue`        |
//! | `0x0100` | IN        | read registers starting at `wValue`         |
//! | `0x0200` | IN / OUT  | read / write EEPROM words                    |
//! | `0x0400` | OUT       | enable DMA for reading (6-byte descriptor)  |
//! | `0x0401` | OUT       | enable DMA for writing (6-byte descriptor)  |
//! | `0x0600` | OUT       | cancel DMA                                  |
//! | `0x0800` | OUT       | DMA control word (`0` = reset, `0x14` ...)  |
//! | `0x0801` | OUT       | chipset reset (no data)                     |

use std::fmt;

/// `bRequest` used for every vendor request of the RTS8822.
pub const VENDOR_REQUEST: u8 = 0x04;
/// `bmRequestType` for vendor, device, host-to-device.
pub const REQUEST_TYPE_OUT: u8 = 0x40;
/// `bmRequestType` for vendor, device, device-to-host.
pub const REQUEST_TYPE_IN: u8 = 0xc0;

/// `wIndex` values selecting the operation.
pub mod index {
    /// Write registers.
    pub const REG_WRITE: u16 = 0x0000;
    /// Read registers.
    pub const REG_READ: u16 = 0x0100;
    /// EEPROM access (read with an IN request, write with an OUT request).
    pub const EEPROM: u16 = 0x0200;
    /// Enable DMA, device to host.
    pub const DMA_ENABLE_READ: u16 = 0x0400;
    /// Enable DMA, host to device.
    pub const DMA_ENABLE_WRITE: u16 = 0x0401;
    /// Cancel the current DMA transfer.
    pub const DMA_CANCEL: u16 = 0x0600;
    /// DMA control word.
    pub const DMA_CONTROL: u16 = 0x0800;
    /// Reset the chipset.
    pub const CHIPSET_RESET: u16 = 0x0801;
}

/// A USB SETUP packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SetupPacket {
    /// `bmRequestType`.
    pub request_type: u8,
    /// `bRequest`.
    pub request: u8,
    /// `wValue`.
    pub value: u16,
    /// `wIndex`.
    pub index: u16,
    /// `wLength`.
    pub length: u16,
}

impl SetupPacket {
    /// Parses the 8 bytes of a SETUP packet (little-endian fields).
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let b: &[u8; 8] = bytes.get(..8)?.try_into().ok()?;
        Some(Self {
            request_type: b[0],
            request: b[1],
            value: u16::from_le_bytes([b[2], b[3]]),
            index: u16::from_le_bytes([b[4], b[5]]),
            length: u16::from_le_bytes([b[6], b[7]]),
        })
    }

    /// Serialises the packet to its 8 wire bytes.
    pub fn to_bytes(self) -> [u8; 8] {
        let v = self.value.to_le_bytes();
        let i = self.index.to_le_bytes();
        let l = self.length.to_le_bytes();
        [
            self.request_type,
            self.request,
            v[0],
            v[1],
            i[0],
            i[1],
            l[0],
            l[1],
        ]
    }

    /// True when the data stage goes from device to host.
    pub fn is_in(self) -> bool {
        self.request_type & 0x80 != 0
    }

    /// True for a vendor request (`bmRequestType` type bits = 2).
    pub fn is_vendor(self) -> bool {
        (self.request_type >> 5) & 0x03 == 2
    }

    fn vendor_in(value: u16, index: u16, length: u16) -> Self {
        Self {
            request_type: REQUEST_TYPE_IN,
            request: VENDOR_REQUEST,
            value,
            index,
            length,
        }
    }

    fn vendor_out(value: u16, index: u16, length: u16) -> Self {
        Self {
            request_type: REQUEST_TYPE_OUT,
            request: VENDOR_REQUEST,
            value,
            index,
            length,
        }
    }
}

/// Direction of a DMA transfer, as seen from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DmaDirection {
    /// Device to host (bulk IN, e.g. image or calibration data).
    Read,
    /// Host to device (bulk OUT, e.g. gamma or shading tables).
    Write,
}

/// One decoded chip operation.
///
/// Read operations carry only what is requested; the data returned by the
/// device is not part of the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Read `length` bytes of registers starting at `address`.
    ReadRegisters { address: u16, length: u16 },
    /// Write `data` to registers starting at `address`.
    WriteRegisters { address: u16, data: Vec<u8> },
    /// Read `length` bytes of EEPROM starting at `address`.
    ReadEeprom { address: u16, length: u16 },
    /// Write `data` to EEPROM starting at `address`.
    WriteEeprom { address: u16, data: Vec<u8> },
    /// Arm the DMA engine for the next bulk transfer.
    DmaEnable {
        /// Transfer direction.
        direction: DmaDirection,
        /// DMA channel / chip-select, sent in `wValue`.
        channel: u16,
        /// 24-bit option field (big-endian on the wire).
        options: u32,
        /// Transfer size in bytes (sent as a 24-bit little-endian word count).
        size: u32,
    },
    /// Cancel the running DMA transfer.
    DmaCancel,
    /// Write the DMA control word (`0` resets the DMA engine).
    DmaControl { value: u16 },
    /// Reset the chipset.
    ChipsetReset,
    /// A standard USB request (descriptors, configuration, ...).
    Standard(SetupPacket),
    /// A vendor request this decoder does not know.
    Unknown { setup: SetupPacket, data: Vec<u8> },
}

impl Command {
    /// Decodes a control request. `data` is the OUT data stage (empty for IN
    /// requests).
    pub fn decode(setup: SetupPacket, data: &[u8]) -> Self {
        if !setup.is_vendor() || setup.request != VENDOR_REQUEST {
            return Self::Standard(setup);
        }
        let unknown = || Self::Unknown {
            setup,
            data: data.to_vec(),
        };
        if setup.is_in() {
            return match setup.index {
                index::REG_READ => Self::ReadRegisters {
                    address: setup.value,
                    length: setup.length,
                },
                index::EEPROM => Self::ReadEeprom {
                    address: setup.value,
                    length: setup.length,
                },
                _ => unknown(),
            };
        }
        match setup.index {
            index::REG_WRITE => Self::WriteRegisters {
                address: setup.value,
                data: data.to_vec(),
            },
            index::EEPROM => Self::WriteEeprom {
                address: setup.value,
                data: data.to_vec(),
            },
            index::DMA_ENABLE_READ | index::DMA_ENABLE_WRITE if data.len() == 6 => {
                let direction = if setup.index == index::DMA_ENABLE_READ {
                    DmaDirection::Read
                } else {
                    DmaDirection::Write
                };
                let options = u32::from_be_bytes([0, data[0], data[1], data[2]]);
                let words = u32::from_le_bytes([data[3], data[4], data[5], 0]);
                Self::DmaEnable {
                    direction,
                    channel: setup.value,
                    options,
                    size: words * 2,
                }
            }
            index::DMA_CANCEL => Self::DmaCancel,
            index::DMA_CONTROL if data.len() == 2 => Self::DmaControl {
                value: u16::from_le_bytes([data[0], data[1]]),
            },
            index::CHIPSET_RESET => Self::ChipsetReset,
            _ => unknown(),
        }
    }

    /// Encodes the command as a SETUP packet plus OUT data.
    ///
    /// Returns `None` for [`Command::Standard`], which this layer never sends.
    pub fn encode(&self) -> Option<(SetupPacket, Vec<u8>)> {
        let out = |value, index, data: Vec<u8>| {
            let len = u16::try_from(data.len()).ok()?;
            Some((SetupPacket::vendor_out(value, index, len), data))
        };
        match self {
            Self::ReadRegisters { address, length } => Some((
                SetupPacket::vendor_in(*address, index::REG_READ, *length),
                Vec::new(),
            )),
            Self::ReadEeprom { address, length } => Some((
                SetupPacket::vendor_in(*address, index::EEPROM, *length),
                Vec::new(),
            )),
            Self::WriteRegisters { address, data } => out(*address, index::REG_WRITE, data.clone()),
            Self::WriteEeprom { address, data } => out(*address, index::EEPROM, data.clone()),
            Self::DmaEnable {
                direction,
                channel,
                options,
                size,
            } => {
                let o = options.to_be_bytes();
                let w = (size / 2).to_le_bytes();
                let idx = match direction {
                    DmaDirection::Read => index::DMA_ENABLE_READ,
                    DmaDirection::Write => index::DMA_ENABLE_WRITE,
                };
                out(*channel, idx, vec![o[1], o[2], o[3], w[0], w[1], w[2]])
            }
            Self::DmaCancel => out(0, index::DMA_CANCEL, vec![0, 0]),
            Self::DmaControl { value } => out(0, index::DMA_CONTROL, value.to_le_bytes().to_vec()),
            Self::ChipsetReset => out(0, index::CHIPSET_RESET, Vec::new()),
            Self::Unknown { setup, data } => Some((*setup, data.clone())),
            Self::Standard(_) => None,
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadRegisters { address, length } => {
                write!(f, "READ  reg {address:04X} len {length}")
            }
            Self::WriteRegisters { address, data } => {
                write!(f, "WRITE reg {address:04X} = {}", Hex(data))
            }
            Self::ReadEeprom { address, length } => {
                write!(f, "READ  eeprom {address:04X} len {length}")
            }
            Self::WriteEeprom { address, data } => {
                write!(f, "WRITE eeprom {address:04X} = {}", Hex(data))
            }
            Self::DmaEnable {
                direction,
                channel,
                options,
                size,
            } => write!(
                f,
                "DMA   enable {} channel {channel:04X} options {options:06X} size {size}",
                match direction {
                    DmaDirection::Read => "read ",
                    DmaDirection::Write => "write",
                }
            ),
            Self::DmaCancel => f.write_str("DMA   cancel"),
            Self::DmaControl { value } => write!(f, "DMA   control {value:04X}"),
            Self::ChipsetReset => f.write_str("RESET chipset"),
            Self::Standard(s) => write!(
                f,
                "STD   type {:02X} req {:02X} value {:04X} index {:04X} len {}",
                s.request_type, s.request, s.value, s.index, s.length
            ),
            Self::Unknown { setup: s, data } => write!(
                f,
                "????  type {:02X} req {:02X} value {:04X} index {:04X} len {} data {}",
                s.request_type,
                s.request,
                s.value,
                s.index,
                s.length,
                Hex(data)
            ),
        }
    }
}

/// Formats bytes as space-separated upper-case hex, eliding long buffers.
#[derive(Debug, Clone, Copy)]
pub struct Hex<'a>(pub &'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const MAX: usize = 32;
        for (i, b) in self.0.iter().take(MAX).enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{b:02X}")?;
        }
        if self.0.len() > MAX {
            write!(f, " ... ({} bytes)", self.0.len())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(hex: &str) -> SetupPacket {
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        SetupPacket::parse(&bytes).unwrap()
    }

    // The hex strings below are taken verbatim from 00_collegamento.pcap.

    #[test]
    fn decodes_register_read() {
        let s = setup("c00411fe00010200");
        assert_eq!(
            Command::decode(s, &[]),
            Command::ReadRegisters {
                address: 0xfe11,
                length: 2
            }
        );
    }

    #[test]
    fn decodes_full_register_dump() {
        let s = setup("c00400e800011907");
        assert_eq!(
            Command::decode(s, &[]),
            Command::ReadRegisters {
                address: 0xe800,
                length: 0x0719
            }
        );
    }

    #[test]
    fn decodes_register_write() {
        let s = setup("4004fff800000200");
        assert_eq!(
            Command::decode(s, &[0x01, 0x10]),
            Command::WriteRegisters {
                address: 0xf8ff,
                data: vec![0x01, 0x10]
            }
        );
    }

    #[test]
    fn decodes_dma_enable() {
        // options 0x000000, 0x1234 words -> 0x2468 bytes.
        let s = setup("4004040001040600");
        let cmd = Command::decode(s, &[0, 0, 0, 0x34, 0x12, 0]);
        assert_eq!(
            cmd,
            Command::DmaEnable {
                direction: DmaDirection::Write,
                channel: 4,
                options: 0,
                size: 0x2468
            }
        );
    }

    #[test]
    fn standard_requests_are_not_vendor() {
        let s = setup("8006000100001200");
        assert!(matches!(Command::decode(s, &[]), Command::Standard(_)));
    }

    #[test]
    fn encode_decode_round_trip() {
        let cmds = [
            Command::ReadRegisters {
                address: 0xe800,
                length: 0x719,
            },
            Command::WriteRegisters {
                address: 0xe954,
                data: vec![0xb0, 0x01],
            },
            Command::ReadEeprom {
                address: 0x0021,
                length: 2,
            },
            Command::WriteEeprom {
                address: 0x0022,
                data: vec![1, 2],
            },
            Command::DmaEnable {
                direction: DmaDirection::Read,
                channel: 0,
                options: 0x123456,
                size: 1024,
            },
            Command::DmaEnable {
                direction: DmaDirection::Write,
                channel: 4,
                options: 0,
                size: 512,
            },
            Command::DmaCancel,
            Command::DmaControl { value: 0x0014 },
            Command::ChipsetReset,
        ];
        for cmd in cmds {
            let (s, data) = cmd.encode().unwrap();
            assert_eq!(SetupPacket::parse(&s.to_bytes()), Some(s));
            assert_eq!(Command::decode(s, &data), cmd, "{cmd}");
        }
    }
}
