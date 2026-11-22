// SPDX-License-Identifier: GPL-2.0-or-later
//! Backend over `nusb` (usbfs on Linux, IOKit on macOS, WinUSB on Windows).

use crate::{Backend, Found, BULK_TIMEOUT, CONTROL_TIMEOUT};
use hp3770_core::device::{EP_BULK_IN, EP_BULK_OUT, EP_INTERRUPT_IN, PRODUCT_ID, VENDOR_ID};
use hp3770_core::protocol::SetupPacket;
use hp3770_core::transport::Transport;
use hp3770_core::{Error, Result};
use nusb::transfer::{
    Buffer, Bulk, ControlIn, ControlOut, ControlType, In, Interrupt, Out, Recipient,
};
use nusb::{DeviceInfo, Endpoint, Interface, MaybeFuture};
use std::fmt;
use std::time::Duration;

const INTERFACE: u8 = 0;

fn err(context: &str, e: impl fmt::Display) -> Error {
    Error::Transport(format!("{context}: {e}"))
}

/// Copies what a bulk IN transfer received into `buf`. The request is
/// rounded up to whole packets, so the device could send more than `buf`
/// holds: that is an error (the DMA size on the chip and the read disagree),
/// not data to drop in silence.
fn copy_bulk_in(buf: &mut [u8], received: &[u8]) -> Result<usize> {
    let n = received.len();
    if n > buf.len() {
        return Err(Error::Transport(format!(
            "bulk IN: the device sent {n} bytes, {} expected",
            buf.len()
        )));
    }
    buf[..n].copy_from_slice(received);
    Ok(n)
}

fn is_3770(d: &DeviceInfo) -> bool {
    d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID
}

fn location(d: &DeviceInfo) -> String {
    format!("bus {} address {}", d.bus_id(), d.device_address())
}

/// Lists the scanners `nusb` can see.
pub(crate) fn list() -> Vec<Found> {
    let Ok(devices) = nusb::list_devices().wait() else {
        return Vec::new();
    };
    devices
        .filter(is_3770)
        .map(|d| Found {
            backend: Backend::Nusb,
            location: location(&d),
            note: note(&d),
        })
        .collect()
}

#[cfg(windows)]
fn note(d: &DeviceInfo) -> Option<String> {
    match d.driver() {
        Some(drv) if drv.eq_ignore_ascii_case("winusb") => None,
        Some(drv) => Some(format!("bound to {drv}, nusb needs WinUSB")),
        None => Some("no driver bound".into()),
    }
}

#[cfg(not(windows))]
fn note(_d: &DeviceInfo) -> Option<String> {
    None
}

/// Claims interface 0 of a scanner that no one has configured, after setting its only
/// configuration; `None` if it has one, and the claim's own error stands. Asked only when the
/// claim failed: a scanner that opens keeps the path it always had. Linux and Windows configure
/// it at plugging in, and so did the Mac it was tried on (22 November 2026), but macOS sets none
/// on a device of vendor class that no driver of the system takes, as it did with the Epson
/// 4490: this is the same request, a fallback for a Mac that does so with the 3770.
#[cfg(not(windows))]
fn claim_unconfigured(device: &nusb::Device) -> Option<Result<Interface>> {
    if device.active_configuration().is_ok() {
        return None;
    }
    let value = device.configurations().next()?.configuration_value();
    let claimed = device
        .set_configuration(value)
        .wait()
        .map_err(|e| {
            err(
                &format!("setting configuration {value} (is another program using the scanner?)"),
                e,
            )
        })
        .and_then(|()| {
            device
                .claim_interface(INTERFACE)
                .wait()
                .map_err(|e| err("claiming interface 0 after setting the configuration", e))
        });
    Some(claimed)
}

/// On Windows the driver bound to the scanner (WinUSB) has set its configuration, and WinUSB
/// cannot set one: the claim's error stands.
#[cfg(windows)]
fn claim_unconfigured(_device: &nusb::Device) -> Option<Result<Interface>> {
    None
}

fn control_type(setup: SetupPacket) -> Result<(ControlType, Recipient)> {
    let ty = match (setup.request_type >> 5) & 0x03 {
        0 => ControlType::Standard,
        1 => ControlType::Class,
        2 => ControlType::Vendor,
        _ => {
            return Err(Error::InvalidArgument(format!(
                "reserved request type {:02x}",
                setup.request_type
            )))
        }
    };
    let rcpt = match setup.request_type & 0x1f {
        0 => Recipient::Device,
        1 => Recipient::Interface,
        2 => Recipient::Endpoint,
        _ => Recipient::Other,
    };
    Ok((ty, rcpt))
}

/// A scanner opened through `nusb`.
pub struct NusbScanner {
    interface: Interface,
    bulk_in: Endpoint<Bulk, In>,
    bulk_out: Endpoint<Bulk, Out>,
    interrupt: Endpoint<Interrupt, In>,
    location: String,
}

impl fmt::Debug for NusbScanner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NusbScanner")
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl NusbScanner {
    /// Opens the first HP Scanjet 3770 found.
    pub fn open_first() -> Result<Self> {
        let info = nusb::list_devices()
            .wait()
            .map_err(|e| err("listing USB devices", e))?
            .find(is_3770)
            .ok_or_else(|| Error::Transport("scanner not found".into()))?;
        Self::open(&info)
    }

    /// Opens a specific device.
    pub fn open(info: &DeviceInfo) -> Result<Self> {
        let location = location(info);
        let device = info
            .open()
            .wait()
            .map_err(|e| err("opening the device", e))?;
        let interface = match device.claim_interface(INTERFACE).wait() {
            Ok(interface) => interface,
            Err(e) => match claim_unconfigured(&device) {
                Some(claimed) => claimed?,
                None => return Err(err("claiming interface 0", e)),
            },
        };
        let bulk_in = interface
            .endpoint::<Bulk, In>(EP_BULK_IN)
            .map_err(|e| err("bulk IN endpoint", e))?;
        let bulk_out = interface
            .endpoint::<Bulk, Out>(EP_BULK_OUT)
            .map_err(|e| err("bulk OUT endpoint", e))?;
        let interrupt = interface
            .endpoint::<Interrupt, In>(EP_INTERRUPT_IN)
            .map_err(|e| err("interrupt endpoint", e))?;
        Ok(Self {
            interface,
            bulk_in,
            bulk_out,
            interrupt,
            location,
        })
    }

    /// Where the scanner is connected.
    pub fn location(&self) -> &str {
        &self.location
    }
}

impl Transport for NusbScanner {
    fn control_in(&mut self, setup: SetupPacket) -> Result<Vec<u8>> {
        let (control_type, recipient) = control_type(setup)?;
        self.interface
            .control_in(
                ControlIn {
                    control_type,
                    recipient,
                    request: setup.request,
                    value: setup.value,
                    index: setup.index,
                    length: setup.length,
                },
                CONTROL_TIMEOUT,
            )
            .wait()
            .map_err(|e| err("control IN", e))
    }

    fn control_out(&mut self, setup: SetupPacket, data: &[u8]) -> Result<()> {
        let (control_type, recipient) = control_type(setup)?;
        self.interface
            .control_out(
                ControlOut {
                    control_type,
                    recipient,
                    request: setup.request,
                    value: setup.value,
                    index: setup.index,
                    data,
                },
                CONTROL_TIMEOUT,
            )
            .wait()
            .map_err(|e| err("control OUT", e))
    }

    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // nusb wants IN requests in whole packets; the DMA size set on the
        // chip bounds what the device actually sends.
        let mps = self.bulk_in.max_packet_size().max(1);
        let requested = buf.len().div_ceil(mps) * mps;
        let completion = self
            .bulk_in
            .transfer_blocking(self.bulk_in.allocate(requested), BULK_TIMEOUT);
        completion.status.map_err(|e| err("bulk IN", e))?;
        let got = completion.actual_len.min(completion.buffer.len());
        copy_bulk_in(buf, &completion.buffer[..got])
    }

    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        let completion = self
            .bulk_out
            .transfer_blocking(Buffer::from(data), BULK_TIMEOUT);
        completion.status.map_err(|e| err("bulk OUT", e))?;
        if completion.actual_len != data.len() {
            return Err(Error::ShortTransfer {
                expected: data.len(),
                actual: completion.actual_len,
            });
        }
        Ok(())
    }

    fn interrupt_in(&mut self, timeout: Duration) -> Result<Option<u8>> {
        // Keep one request pending between calls, like the HP driver does, so
        // that a press between two calls is not lost.
        if self.interrupt.pending() == 0 {
            let len = self.interrupt.max_packet_size().max(1);
            let buf = self.interrupt.allocate(len);
            self.interrupt.submit(buf);
        }
        let Some(completion) = self.interrupt.wait_next_complete(timeout) else {
            return Ok(None);
        };
        completion.status.map_err(|e| err("interrupt IN", e))?;
        Ok(completion.buffer.first().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bulk_in_longer_than_the_buffer_is_an_error() {
        let mut buf = [0u8; 4];
        assert_eq!(copy_bulk_in(&mut buf, &[1, 2, 3]).unwrap(), 3);
        assert_eq!(buf, [1, 2, 3, 0]);
        assert_eq!(copy_bulk_in(&mut buf, &[5, 6, 7, 8]).unwrap(), 4);
        assert_eq!(buf, [5, 6, 7, 8]);
        let err = copy_bulk_in(&mut buf, &[0; 5]).unwrap_err().to_string();
        assert!(err.contains("sent 5 bytes, 4 expected"), "{err}");
    }
}
