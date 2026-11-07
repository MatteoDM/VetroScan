// SPDX-License-Identifier: GPL-2.0-or-later
//! USBPcap packet header (`USBPCAP_BUFFER_PACKET_HEADER`).
//!
//! Layout (little-endian, packed):
//!
//! | Offset | Size | Field                                   |
//! |--------|------|-----------------------------------------|
//! | 0      | 2    | header length                           |
//! | 2      | 8    | IRP id (pairs a request with completion) |
//! | 10     | 4    | USBD status                             |
//! | 14     | 2    | URB function                            |
//! | 16     | 1    | info: bit 0 set = completion (PDO→FDO)  |
//! | 17     | 2    | bus                                     |
//! | 19     | 2    | device address                          |
//! | 21     | 1    | endpoint (bit 7 = IN)                   |
//! | 22     | 1    | transfer type                           |
//! | 23     | 4    | data length                             |
//! | 27     | 1    | control stage (control transfers only)  |
//!
//! For control transfers the submission (stage 0, SETUP) carries the 8 setup
//! bytes followed by the OUT data, and the completion (stage 3) carries the
//! IN data.

use crate::{Error, Result};

const BASE_HEADER_LEN: usize = 27;

/// USBPcap transfer type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferType {
    /// Isochronous transfer.
    Isochronous,
    /// Interrupt transfer.
    Interrupt,
    /// Control transfer.
    Control,
    /// Bulk transfer.
    Bulk,
    /// IRP information only (pipe resets, aborts, ...).
    IrpInfo,
    /// Any other value.
    Other(u8),
}

impl From<u8> for TransferType {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Isochronous,
            1 => Self::Interrupt,
            2 => Self::Control,
            3 => Self::Bulk,
            0xfe => Self::IrpInfo,
            other => Self::Other(other),
        }
    }
}

/// Control transfer stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlStage {
    /// SETUP stage (submission).
    Setup,
    /// DATA stage.
    Data,
    /// STATUS stage.
    Status,
    /// Completion.
    Complete,
    /// Any other value.
    Other(u8),
}

impl From<u8> for ControlStage {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Setup,
            1 => Self::Data,
            2 => Self::Status,
            3 => Self::Complete,
            other => Self::Other(other),
        }
    }
}

/// One USBPcap packet.
#[derive(Debug, Clone, Copy)]
pub struct UsbPacket<'a> {
    /// Capture time, seconds since the Unix epoch.
    pub timestamp: f64,
    /// IRP id; the submission and the completion of a transfer share it.
    pub irp_id: u64,
    /// USBD status (0 = success).
    pub status: u32,
    /// URB function code.
    pub function: u16,
    /// True for a completion (device/driver back to the host software).
    pub completion: bool,
    /// USB bus.
    pub bus: u16,
    /// USB device address.
    pub device: u16,
    /// Endpoint address, bit 7 set for IN.
    pub endpoint: u8,
    /// Transfer type.
    pub transfer: TransferType,
    /// Control stage, for control transfers.
    pub stage: Option<ControlStage>,
    /// Payload following the header.
    pub data: &'a [u8],
}

impl<'a> UsbPacket<'a> {
    /// Parses a USBPcap record.
    pub fn parse(timestamp: f64, bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < BASE_HEADER_LEN {
            return Err(Error::Format(format!(
                "USBPcap record of {} bytes",
                bytes.len()
            )));
        }
        let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        let u32_at =
            |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        let header_len = usize::from(u16_at(0));
        if header_len < BASE_HEADER_LEN || header_len > bytes.len() {
            return Err(Error::Format(format!(
                "bad USBPcap header length {header_len}"
            )));
        }
        let mut irp = [0u8; 8];
        irp.copy_from_slice(&bytes[2..10]);
        let transfer = TransferType::from(bytes[22]);
        let data_len = u32_at(23) as usize;
        let stage = (transfer == TransferType::Control && header_len > BASE_HEADER_LEN)
            .then(|| ControlStage::from(bytes[BASE_HEADER_LEN]));
        // Trust the captured bytes over the declared length (snaplen).
        let end = header_len.saturating_add(data_len).min(bytes.len());
        Ok(Self {
            timestamp,
            irp_id: u64::from_le_bytes(irp),
            status: u32_at(10),
            function: u16_at(14),
            completion: bytes[16] & 0x01 != 0,
            bus: u16_at(17),
            device: u16_at(19),
            endpoint: bytes[21],
            transfer,
            stage,
            data: &bytes[header_len..end],
        })
    }

    /// True when the endpoint direction is IN.
    pub fn is_in(&self) -> bool {
        self.endpoint & 0x80 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_control_setup() {
        // Real record from 00_collegamento.pcap: GET_DESCRIPTOR(device).
        let mut rec = vec![28, 0];
        rec.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        rec.extend_from_slice(&0u32.to_le_bytes()); // status
        rec.extend_from_slice(&11u16.to_le_bytes()); // function
        rec.push(0); // info
        rec.extend_from_slice(&1u16.to_le_bytes()); // bus
        rec.extend_from_slice(&8u16.to_le_bytes()); // device
        rec.push(0x80); // endpoint
        rec.push(2); // control
        rec.extend_from_slice(&8u32.to_le_bytes());
        rec.push(0); // stage setup
        rec.extend_from_slice(&[0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]);
        let p = UsbPacket::parse(1.0, &rec).unwrap();
        assert_eq!(p.irp_id, 0x1122_3344_5566_7788);
        assert_eq!(p.device, 8);
        assert!(p.is_in());
        assert!(!p.completion);
        assert_eq!(p.transfer, TransferType::Control);
        assert_eq!(p.stage, Some(ControlStage::Setup));
        assert_eq!(p.data.len(), 8);
    }

    #[test]
    fn rejects_short_record() {
        assert!(UsbPacket::parse(0.0, &[1, 2, 3]).is_err());
    }
}
