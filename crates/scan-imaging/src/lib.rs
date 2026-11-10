// SPDX-License-Identifier: GPL-2.0-or-later
//! Image processing on the host, the same for every scanner.
//!
//! A scanner gives linear samples; everything after that lives here: the
//! [`Image`] and its pixel layouts, the adjustments through the scanner's
//! tone curve ([`adjust`]), grey and black-and-white conversion and
//! sharpening ([`convert`]), crops, rotation and straightening
//! ([`transform`]), and finding the photos and pages on the glass
//! ([`detect`]), slides and negatives in transmitted light ([`film`]), and
//! the noise of the colour ([`chroma`]), a scanner's colours corrected
//! with a matrix it declares ([`colour`]).
//! Nothing here knows about USB or about a scanner model.

pub mod adjust;
pub mod chroma;
pub mod colour;
pub mod convert;
pub mod detect;
pub mod film;
mod image;
pub mod stripes;
pub mod transform;

pub use image::{Format, Image, Mode};
