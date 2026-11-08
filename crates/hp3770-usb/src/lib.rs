// SPDX-License-Identifier: GPL-2.0-or-later
//! USB access to the HP Scanjet 3770.
//!
//! Two backends implement [`hp3770_core::transport::Transport`]:
//!
//! * [`NusbScanner`]: direct USB access through `nusb`. Works on Linux
//!   (usbfs, needs a udev rule for non-root users), on macOS (IOKit) and on
//!   Windows when the scanner is bound to WinUSB.
//! * `UsbscanScanner` (Windows only): talks to the scanner through
//!   `usbscan.sys`, the Windows still-image driver that the HP driver
//!   installs. No driver change is needed, and the HP software keeps working.
//!
//! [`open`] picks the first backend that finds a scanner, and
//! [`UsbBackend`] offers the scanners to the application through the
//! [`scanner_api::Registry`].

mod nusb_backend;
#[cfg(windows)]
mod usbscan;

pub use nusb_backend::NusbScanner;
#[cfg(windows)]
pub use usbscan::UsbscanScanner;

use hp3770_core::device::{PRODUCT_ID, VENDOR_ID};
use hp3770_core::protocol::SetupPacket;
use hp3770_core::transport::Transport;
use hp3770_core::{Error, Result};
use std::fmt;
use std::time::Duration;

/// Timeout of a control request.
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
/// Timeout of a bulk transfer. Long, because the first line of a scan only
/// arrives after the carriage has moved to the document.
pub const BULK_TIMEOUT: Duration = Duration::from_secs(30);

/// Which backend a scanner was opened with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Direct USB through `nusb`.
    Nusb,
    /// Windows `usbscan.sys` (HP driver installed).
    Usbscan,
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Nusb => "nusb",
            Self::Usbscan => "usbscan.sys",
        })
    }
}

/// A scanner found on the system, before it is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Backend that sees it.
    pub backend: Backend,
    /// Human-readable location (bus/address, or the usbscan device path).
    pub location: String,
    /// Whether this backend can open it (e.g. nusb on Windows needs WinUSB).
    pub note: Option<String>,
}

/// An opened scanner, whatever the backend.
#[derive(Debug)]
pub enum Scanner {
    /// Opened with `nusb`.
    Nusb(NusbScanner),
    /// Opened with `usbscan.sys`.
    #[cfg(windows)]
    Usbscan(UsbscanScanner),
}

impl Scanner {
    /// The backend in use.
    pub fn backend(&self) -> Backend {
        match self {
            Self::Nusb(_) => Backend::Nusb,
            #[cfg(windows)]
            Self::Usbscan(_) => Backend::Usbscan,
        }
    }
}

/// Lists the HP Scanjet 3770 devices every backend can see.
pub fn list() -> Vec<Found> {
    #[cfg(windows)]
    return nusb_backend::list()
        .into_iter()
        .chain(usbscan::list())
        .collect();
    #[cfg(not(windows))]
    nusb_backend::list()
}

/// Opens the first scanner found.
///
/// On Windows `usbscan.sys` is tried first, because that is the driver bound
/// to the scanner when the HP software is installed; `nusb` is used when the
/// scanner has been switched to WinUSB.
pub fn open() -> Result<Scanner> {
    let mut errors = Vec::new();
    #[cfg(windows)]
    match UsbscanScanner::open_first() {
        Ok(s) => return Ok(Scanner::Usbscan(s)),
        Err(e) => errors.push(format!("usbscan.sys: {e}")),
    }
    match NusbScanner::open_first() {
        Ok(s) => return Ok(Scanner::Nusb(s)),
        Err(e) => errors.push(format!("nusb: {e}")),
    }
    Err(Error::Transport(format!(
        "no HP Scanjet 3770 ({VENDOR_ID:04x}:{PRODUCT_ID:04x}) could be opened ({})",
        errors.join("; ")
    )))
}

/// Opens the first scanner found by a specific backend.
pub fn open_with(backend: Backend) -> Result<Scanner> {
    match backend {
        Backend::Nusb => NusbScanner::open_first().map(Scanner::Nusb),
        #[cfg(windows)]
        Backend::Usbscan => UsbscanScanner::open_first().map(Scanner::Usbscan),
        #[cfg(not(windows))]
        Backend::Usbscan => Err(Error::Transport(
            "usbscan.sys exists only on Windows".into(),
        )),
    }
}

impl Scanner {
    /// Where the scanner is connected (bus and address, or device path).
    pub fn location(&self) -> String {
        match self {
            Self::Nusb(s) => s.location().to_owned(),
            #[cfg(windows)]
            Self::Usbscan(s) => s.path(),
        }
    }
}

impl Transport for Scanner {
    fn control_in(&mut self, setup: SetupPacket) -> Result<Vec<u8>> {
        match self {
            Self::Nusb(s) => s.control_in(setup),
            #[cfg(windows)]
            Self::Usbscan(s) => s.control_in(setup),
        }
    }

    fn control_out(&mut self, setup: SetupPacket, data: &[u8]) -> Result<()> {
        match self {
            Self::Nusb(s) => s.control_out(setup, data),
            #[cfg(windows)]
            Self::Usbscan(s) => s.control_out(setup, data),
        }
    }

    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Nusb(s) => s.bulk_in(buf),
            #[cfg(windows)]
            Self::Usbscan(s) => s.bulk_in(buf),
        }
    }

    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        match self {
            Self::Nusb(s) => s.bulk_out(data),
            #[cfg(windows)]
            Self::Usbscan(s) => s.bulk_out(data),
        }
    }

    fn interrupt_in(&mut self, timeout: Duration) -> Result<Option<u8>> {
        match self {
            Self::Nusb(s) => s.interrupt_in(timeout),
            #[cfg(windows)]
            Self::Usbscan(s) => s.interrupt_in(timeout),
        }
    }
}

/// The scanners on USB, as a backend of the [`scanner_api::Registry`]: the
/// RTS8822 driver with the profile of each model.
#[derive(Debug, Default, Clone, Copy)]
pub struct UsbBackend;

impl scanner_api::Backend for UsbBackend {
    fn name(&self) -> &'static str {
        hp3770_core::rts8822::BACKEND
    }

    /// One entry for each scanner a backend can open (not the ones `nusb`
    /// sees but cannot open, as on Windows without WinUSB).
    fn list(&self) -> Vec<scanner_api::DeviceInfo> {
        let Some(profile) = hp3770_core::profile::for_device(VENDOR_ID, PRODUCT_ID) else {
            return Vec::new();
        };
        list()
            .into_iter()
            .filter(|f| f.note.is_none())
            .map(|f| scanner_api::DeviceInfo {
                backend: hp3770_core::rts8822::BACKEND,
                id: f.location,
                name: format!("{} {}", profile.device.manufacturer, profile.device.name),
            })
            .collect()
    }

    fn models(&self) -> Vec<scanner_api::Capabilities> {
        hp3770_core::profile::PROFILES
            .into_iter()
            .map(hp3770_core::rts8822::capabilities)
            .collect()
    }

    /// Opens the scanner as [`open`] does (`usbscan.sys` first on Windows),
    /// so its errors say what each way of opening it found.
    fn open(
        &self,
        _device: &scanner_api::DeviceInfo,
    ) -> scanner_api::Result<Box<dyn scanner_api::Scanner>> {
        let profile = hp3770_core::profile::for_device(VENDOR_ID, PRODUCT_ID)
            .ok_or(scanner_api::Error::Unsupported("this USB scanner"))?;
        let scanner = open().map_err(hp3770_core::rts8822::api_error)?;
        Ok(Box::new(hp3770_core::rts8822::Rts8822::new(
            scanner, profile,
        )))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scanners_can_move_to_the_scanner_thread() {
        fn send<T: Send>() {}
        send::<super::Scanner>();
        send::<super::UsbBackend>();
    }
}
