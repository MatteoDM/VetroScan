// SPDX-License-Identifier: GPL-2.0-or-later
//! Error type shared by the core and its transports.

use std::fmt;

/// Result alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Errors raised while talking to the scanner.
#[derive(Debug)]
pub enum Error {
    /// The transport (USB backend, replay, ...) failed.
    Transport(String),
    /// A transfer moved fewer bytes than requested.
    ShortTransfer {
        /// Bytes requested.
        expected: usize,
        /// Bytes actually transferred.
        actual: usize,
    },
    /// An argument is outside what the hardware accepts.
    InvalidArgument(String),
    /// A write was attempted through a [`crate::transport::ReadOnly`]
    /// transport. Carries the refused request.
    ReadOnly(String),
    /// The calibration found the scanner unable to give a usable image
    /// (lamp off or too weak, offsets that never settle).
    Calibration(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
            Self::ShortTransfer { expected, actual } => {
                write!(f, "short transfer: expected {expected} bytes, got {actual}")
            }
            Self::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
            Self::ReadOnly(req) => write!(f, "refused in read-only mode: {req}"),
            Self::Calibration(msg) => write!(f, "calibration failed: {msg}"),
        }
    }
}

impl std::error::Error for Error {}
