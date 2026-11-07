// SPDX-License-Identifier: GPL-2.0-or-later
//! Reads USB captures of the HP Scanjet 3770 and turns them into a readable
//! sequence of chip operations.
//!
//! The captures are classic `.pcap` files written by USBPcap on Windows
//! (link type 249). The pipeline is:
//!
//! 1. [`pcap`] splits the file into timestamped records;
//! 2. [`usbpcap`] parses the USBPcap header of each record;
//! 3. [`trace`] pairs each request with its completion and decodes control
//!    requests with [`hp3770_core::protocol::Command`].
//!
//! The resulting [`trace::Trace`] is the reference the Rust driver is tested
//! against: the same scan settings must produce the same register writes.

pub mod pcap;
pub mod trace;
pub mod usbpcap;

use std::fmt;

/// Errors raised while reading a capture.
#[derive(Debug)]
pub enum Error {
    /// I/O error reading the file.
    Io(std::io::Error),
    /// The file is not a capture this crate understands.
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Format(msg) => write!(f, "bad capture: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Format(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Result alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
