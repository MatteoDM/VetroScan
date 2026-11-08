// SPDX-License-Identifier: GPL-2.0-or-later
//! Protocol core for the HP Scanjet 3770.
//!
//! The scanner is built around a Realtek RTS8822 ASIC, the same family driven
//! by the SANE `hp3900` backend. Everything the host does goes through a small
//! set of USB requests:
//!
//! * vendor control requests (`bRequest = 0x04`) on EP0 to read and write the
//!   chip registers, the EEPROM and the DMA engine ([`protocol`]);
//! * bulk IN on endpoint `0x81` for image and calibration data;
//! * bulk OUT on endpoint `0x02` for tables (gamma, shading);
//! * interrupt IN on endpoint `0x83` for the front-panel buttons.
//!
//! This crate has no USB dependency: the [`transport::Transport`] trait is
//! implemented by a real USB backend in another crate, and by a replay of
//! recorded captures in tests.

pub mod arrange;
pub mod calibration;
pub mod device;
pub mod error;
pub mod geometry;
pub mod init;
pub mod lamp;
pub mod motor;
pub mod motor_curves;
pub mod profile;
pub mod protocol;
pub mod read;
pub mod scan;
pub mod scan_modes;
pub mod session;
pub mod setup;
pub mod status;
pub mod transport;

pub use error::{Error, Result};
