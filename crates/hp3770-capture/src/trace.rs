// SPDX-License-Identifier: GPL-2.0-or-later
//! Pairs USBPcap submissions with completions and decodes them.

use crate::pcap::{Pcap, LINKTYPE_USBPCAP};
use crate::usbpcap::{ControlStage, TransferType, UsbPacket};
use crate::{Error, Result};
use hp3770_core::device::Button;
use hp3770_core::protocol::{Command, Hex, SetupPacket};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::Path;

/// One completed USB transfer, decoded.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A control request and, for IN requests, the device's answer.
    Control {
        /// Submission time, seconds since the start of the capture.
        time: f64,
        /// Decoded request.
        command: Command,
        /// Data returned by the device (IN requests only).
        response: Vec<u8>,
        /// USBD status of the completion (0 = success).
        status: u32,
    },
    /// Bulk transfer from the device.
    BulkIn {
        /// Completion time.
        time: f64,
        /// Endpoint address.
        endpoint: u8,
        /// Received bytes.
        data: Vec<u8>,
        /// USBD status of the completion.
        status: u32,
    },
    /// Bulk transfer to the device.
    BulkOut {
        /// Submission time.
        time: f64,
        /// Endpoint address.
        endpoint: u8,
        /// Sent bytes.
        data: Vec<u8>,
        /// USBD status of the completion.
        status: u32,
    },
    /// Interrupt transfer from the device (button press).
    Interrupt {
        /// Completion time.
        time: f64,
        /// Endpoint address.
        endpoint: u8,
        /// Received bytes.
        data: Vec<u8>,
    },
}

impl Event {
    /// Time of the event, seconds since the start of the capture.
    pub fn time(&self) -> f64 {
        match self {
            Self::Control { time, .. }
            | Self::BulkIn { time, .. }
            | Self::BulkOut { time, .. }
            | Self::Interrupt { time, .. } => *time,
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:>10.3}  ", self.time())?;
        match self {
            Self::Control {
                command,
                response,
                status,
                ..
            } => {
                write!(f, "{command}")?;
                if !response.is_empty() {
                    write!(f, " -> {}", Hex(response))?;
                }
                if *status != 0 {
                    write!(f, "  [status {status:08X}]")?;
                }
                Ok(())
            }
            Self::BulkIn {
                endpoint,
                data,
                status,
                ..
            } => {
                write!(f, "BULK  in  ep{endpoint:02X} {} bytes", data.len())?;
                if *status != 0 {
                    write!(f, "  [status {status:08X}]")?;
                }
                Ok(())
            }
            Self::BulkOut {
                endpoint,
                data,
                status,
                ..
            } => {
                write!(
                    f,
                    "BULK  out ep{endpoint:02X} {} bytes: {}",
                    data.len(),
                    Hex(data)
                )?;
                if *status != 0 {
                    write!(f, "  [status {status:08X}]")?;
                }
                Ok(())
            }
            Self::Interrupt { endpoint, data, .. } => {
                write!(f, "INTR  ep{endpoint:02X} {}", Hex(data))?;
                if let Some(b) = data.first().copied().and_then(Button::from_code) {
                    write!(f, "  (button {})", b.name())?;
                }
                Ok(())
            }
        }
    }
}

/// Pending submission waiting for its completion.
#[derive(Debug)]
struct Pending {
    timestamp: f64,
    setup: Option<SetupPacket>,
    out_data: Vec<u8>,
}

/// The decoded content of a capture.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    /// Completed transfers, in completion order.
    pub events: Vec<Event>,
    /// Submissions that never completed (e.g. the interrupt request still
    /// waiting when the capture stopped).
    pub incomplete: usize,
}

impl Trace {
    /// Reads and decodes a capture file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    /// Decodes a capture held in memory.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let pcap = Pcap::parse(bytes)?;
        if pcap.link_type != LINKTYPE_USBPCAP {
            return Err(Error::Format(format!(
                "link type {} is not USBPcap ({LINKTYPE_USBPCAP})",
                pcap.link_type
            )));
        }
        let mut start = None;
        let mut pending: HashMap<u64, Pending> = HashMap::new();
        let mut events = Vec::new();

        for record in pcap.records() {
            let record = record?;
            let p = UsbPacket::parse(record.timestamp, record.data)?;
            let t0 = *start.get_or_insert(p.timestamp);
            let rel = |t: f64| t - t0;

            if !p.completion {
                let (setup, out_data) = match (p.transfer, p.stage) {
                    (TransferType::Control, Some(ControlStage::Setup)) => (
                        SetupPacket::parse(p.data),
                        p.data.get(8..).unwrap_or_default().to_vec(),
                    ),
                    (TransferType::Control, _) => continue,
                    _ => (None, p.data.to_vec()),
                };
                pending.insert(
                    p.irp_id,
                    Pending {
                        timestamp: p.timestamp,
                        setup,
                        out_data,
                    },
                );
                continue;
            }

            let sub = match pending.remove(&p.irp_id) {
                Some(sub) => sub,
                // Submitted before the capture began. IN transfers carry their
                // data in the completion, so they can still be decoded (the
                // HP driver keeps an interrupt request waiting for buttons).
                None if p.transfer != TransferType::Control && p.is_in() => Pending {
                    timestamp: p.timestamp,
                    setup: None,
                    out_data: Vec::new(),
                },
                None => continue,
            };
            let event = match p.transfer {
                TransferType::Control => {
                    let Some(setup) = sub.setup else { continue };
                    let response = if setup.is_in() {
                        p.data.to_vec()
                    } else {
                        Vec::new()
                    };
                    Event::Control {
                        time: rel(sub.timestamp),
                        command: Command::decode(setup, &sub.out_data),
                        response,
                        status: p.status,
                    }
                }
                TransferType::Bulk if p.is_in() => Event::BulkIn {
                    time: rel(p.timestamp),
                    endpoint: p.endpoint,
                    data: p.data.to_vec(),
                    status: p.status,
                },
                TransferType::Bulk => Event::BulkOut {
                    time: rel(sub.timestamp),
                    endpoint: p.endpoint,
                    data: sub.out_data,
                    status: p.status,
                },
                TransferType::Interrupt if p.status == 0 && !p.data.is_empty() => {
                    Event::Interrupt {
                        time: rel(p.timestamp),
                        endpoint: p.endpoint,
                        data: p.data.to_vec(),
                    }
                }
                _ => continue,
            };
            events.push(event);
        }
        Ok(Self {
            events,
            incomplete: pending.len(),
        })
    }

    /// Aggregate counts over the whole trace.
    pub fn summary(&self) -> Summary {
        let mut s = Summary::default();
        for e in &self.events {
            match e {
                Event::Control { command, .. } => match command {
                    Command::ReadRegisters { .. } => s.register_reads += 1,
                    Command::WriteRegisters { address, data } => {
                        s.register_writes += 1;
                        for i in 0..data.len() {
                            if let Some(a) = address.checked_add(i as u16) {
                                s.registers_written.insert(a);
                            }
                        }
                    }
                    Command::ReadEeprom { .. } | Command::WriteEeprom { .. } => s.eeprom_ops += 1,
                    Command::DmaEnable { .. }
                    | Command::DmaCancel
                    | Command::DmaControl { .. }
                    | Command::ChipsetReset => s.dma_ops += 1,
                    Command::Standard(_) => s.standard_requests += 1,
                    Command::Unknown { .. } => s.unknown_requests += 1,
                },
                Event::BulkIn { data, .. } => {
                    s.bulk_in_transfers += 1;
                    s.bulk_in_bytes += data.len() as u64;
                }
                Event::BulkOut { data, .. } => {
                    s.bulk_out_transfers += 1;
                    s.bulk_out_bytes += data.len() as u64;
                }
                Event::Interrupt { data, .. } => {
                    if let Some(b) = data.first().copied().and_then(Button::from_code) {
                        s.buttons.push(b);
                    }
                }
            }
        }
        s.duration = self.events.last().map_or(0.0, Event::time);
        s.incomplete = self.incomplete;
        s
    }
}

/// Aggregate counts of a [`Trace`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    /// Seconds from the first packet to the last completed event.
    pub duration: f64,
    /// Register read requests.
    pub register_reads: usize,
    /// Register write requests.
    pub register_writes: usize,
    /// Distinct register addresses written.
    pub registers_written: BTreeSet<u16>,
    /// EEPROM requests.
    pub eeprom_ops: usize,
    /// DMA and reset requests.
    pub dma_ops: usize,
    /// Standard USB requests (descriptors, configuration).
    pub standard_requests: usize,
    /// Vendor requests the decoder does not know.
    pub unknown_requests: usize,
    /// Completed bulk IN transfers.
    pub bulk_in_transfers: usize,
    /// Bytes received on bulk IN.
    pub bulk_in_bytes: u64,
    /// Completed bulk OUT transfers.
    pub bulk_out_transfers: usize,
    /// Bytes sent on bulk OUT.
    pub bulk_out_bytes: u64,
    /// Buttons reported on the interrupt endpoint, in order.
    pub buttons: Vec<Button>,
    /// Submissions without a completion.
    pub incomplete: usize,
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "duration          {:.3} s", self.duration)?;
        writeln!(f, "register reads    {}", self.register_reads)?;
        writeln!(
            f,
            "register writes   {} ({} distinct registers)",
            self.register_writes,
            self.registers_written.len()
        )?;
        writeln!(f, "eeprom requests   {}", self.eeprom_ops)?;
        writeln!(f, "dma/reset         {}", self.dma_ops)?;
        writeln!(f, "standard requests {}", self.standard_requests)?;
        writeln!(f, "unknown requests  {}", self.unknown_requests)?;
        writeln!(
            f,
            "bulk in           {} transfers, {} bytes",
            self.bulk_in_transfers, self.bulk_in_bytes
        )?;
        writeln!(
            f,
            "bulk out          {} transfers, {} bytes",
            self.bulk_out_transfers, self.bulk_out_bytes
        )?;
        let buttons: Vec<_> = self.buttons.iter().map(|b| b.name()).collect();
        writeln!(
            f,
            "buttons           {}",
            if buttons.is_empty() {
                "-".into()
            } else {
                buttons.join(", ")
            }
        )?;
        write!(f, "incomplete        {}", self.incomplete)
    }
}
