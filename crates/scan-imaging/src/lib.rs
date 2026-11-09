// SPDX-License-Identifier: GPL-2.0-or-later
//! Image processing on the host, the same for every scanner.
//!
//! A scanner gives linear samples; everything after that lives here: the
//! [`Image`] and its pixel layouts, the adjustments through the scanner's
//! tone curve ([`adjust`]), grey and black-and-white conversion and
//! sharpening ([`convert`]), crops, rotation and straightening
//! ([`transform`]), and finding the photos and pages on the glass
//! ([`detect`]), and slides and negatives in transmitted light ([`film`]).
//! Nothing here knows about USB or about a scanner model.

pub mod adjust;
pub mod convert;
pub mod detect;
pub mod film;
mod image;
pub mod stripes;
pub mod transform;

pub use image::{Format, Image, Mode};
