// SPDX-License-Identifier: GPL-2.0-or-later
//! The scanner over `nusb` (usbfs on Linux, IOKit on macOS, WinUSB on
//! Windows).

use crate::{Found, Link, Transport, BULK_TIMEOUT, PRODUCT_ID, VENDOR_ID};
use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Bulk, Direction, In, Out};
use nusb::{DeviceInfo, Endpoint, MaybeFuture};
use scanner_api::{Error, Result};
use std::fmt;

const INTERFACE: u8 = 0;

fn err(context: &str, e: impl fmt::Display) -> Error {
    Error::Disconnected(format!("{context}: {e}"))
}

fn is_4490(d: &DeviceInfo) -> bool {
    d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID
}

fn location(d: &DeviceInfo) -> String {
    format!("bus {} address {}", d.bus_id(), d.device_address())
}

/// The scanners `nusb` sees.
pub(crate) fn list() -> Vec<Found> {
    let Ok(devices) = nusb::list_devices().wait() else {
        return Vec::new();
    };
    devices
        .filter(is_4490)
        .map(|d| Found {
            link: Link::Nusb,
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

/// The scanner, opened through `nusb`.
pub struct NusbTransport {
    bulk_in: Endpoint<Bulk, In>,
    bulk_out: Endpoint<Bulk, Out>,
    location: String,
}

impl fmt::Debug for NusbTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NusbTransport")
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl NusbTransport {
    /// Opens the scanner at `location` (as [`list`] gives it).
    pub fn open(location: &str) -> Result<Self> {
        let info = nusb::list_devices()
            .wait()
            .map_err(|e| err("listing USB devices", e))?
            .find(|d| is_4490(d) && self::location(d) == location)
            .ok_or_else(|| Error::Disconnected(format!("no Epson 4490 at {location}")))?;
        let device = info
            .open()
            .wait()
            .map_err(|e| err("opening the device", e))?;
        let interface = device
            .claim_interface(INTERFACE)
            .wait()
            .map_err(|e| err("claiming interface 0", e))?;
        // The endpoints' addresses are not known yet: the first bulk pair
        // of the interface (the scanner is not expected to have another).
        let (mut ep_in, mut ep_out) = (None, None);
        if let Some(alt) = interface.descriptor() {
            for ep in alt.endpoints() {
                if ep.transfer_type() != TransferType::Bulk {
                    continue;
                }
                match ep.direction() {
                    Direction::In => ep_in = ep_in.or(Some(ep.address())),
                    Direction::Out => ep_out = ep_out.or(Some(ep.address())),
                }
            }
        }
        let (Some(ep_in), Some(ep_out)) = (ep_in, ep_out) else {
            return Err(Error::Other(
                "the scanner's interface 0 has no pair of bulk endpoints".into(),
            ));
        };
        Ok(Self {
            bulk_in: interface
                .endpoint::<Bulk, In>(ep_in)
                .map_err(|e| err("bulk IN endpoint", e))?,
            bulk_out: interface
                .endpoint::<Bulk, Out>(ep_out)
                .map_err(|e| err("bulk OUT endpoint", e))?,
            location: location.to_owned(),
        })
    }
}

impl Transport for NusbTransport {
    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // nusb wants IN requests in whole packets.
        let mps = self.bulk_in.max_packet_size().max(1);
        let requested = buf.len().div_ceil(mps) * mps;
        let completion = self
            .bulk_in
            .transfer_blocking(self.bulk_in.allocate(requested), BULK_TIMEOUT);
        completion.status.map_err(|e| err("bulk IN", e))?;
        let n = completion.actual_len.min(buf.len());
        buf[..n].copy_from_slice(&completion.buffer[..n]);
        Ok(n)
    }

    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        let completion = self
            .bulk_out
            .transfer_blocking(Buffer::from(data), BULK_TIMEOUT);
        completion.status.map_err(|e| err("bulk OUT", e))?;
        if completion.actual_len != data.len() {
            return Err(Error::Other(format!(
                "bulk OUT: {} bytes of {} sent",
                completion.actual_len,
                data.len()
            )));
        }
        Ok(())
    }
}
