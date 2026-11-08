// SPDX-License-Identifier: GPL-2.0-or-later
//! Static facts about the device: USB identity, endpoints and front-panel buttons.

/// Hewlett-Packard USB vendor id.
pub const VENDOR_ID: u16 = 0x03f0;
/// HP Scanjet 3770 product id.
pub const PRODUCT_ID: u16 = 0x2505;

/// Bulk IN endpoint carrying image and calibration data.
pub const EP_BULK_IN: u8 = 0x81;
/// Bulk OUT endpoint carrying tables (gamma, shading) to the ASIC.
pub const EP_BULK_OUT: u8 = 0x02;
/// Interrupt IN endpoint reporting front-panel button presses (1 byte).
pub const EP_INTERRUPT_IN: u8 = 0x83;

/// Highest optical resolution of the CCD, in dpi.
pub const MAX_OPTICAL_DPI: u16 = 1200;

/// A front-panel button, as reported on [`EP_INTERRUPT_IN`].
///
/// Values confirmed with USB captures of the original HP driver
/// (2026-11-07): left = Scan (`0x00`), centre = Copy (`0x02`),
/// right = HP Instant Share (`0x03`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    /// Left button, scanner icon.
    Scan,
    /// Centre button, copier/fax icon.
    Copy,
    /// Right button, globe with a sheet ("HP Instant Share").
    InstantShare,
}

impl Button {
    /// Decodes the byte sent on the interrupt endpoint.
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0x00 => Some(Self::Scan),
            0x02 => Some(Self::Copy),
            0x03 => Some(Self::InstantShare),
            _ => None,
        }
    }

    /// The byte the scanner sends for this button.
    pub fn code(self) -> u8 {
        match self {
            Self::Scan => 0x00,
            Self::Copy => 0x02,
            Self::InstantShare => 0x03,
        }
    }

    /// The bit of this button in the button status register `E968`, where
    /// the chip shows a held button as 0.
    ///
    /// Measured on the hardware (2026-11-07, `hp3770 buttons --raw`): idle
    /// `E968 = BF`; pressed one by one in the application (2026-11-08):
    /// left (Scan, scanner icon) `BB`, centre (Copy) `BD`, right (Instant
    /// Share) `BE`. The first test had named them wrongly. The order
    /// differs from the interrupt codes.
    pub fn mask(self) -> u8 {
        match self {
            Self::Scan => 0x04,
            Self::Copy => 0x02,
            Self::InstantShare => 0x01,
        }
    }

    /// The button whose bit is `mask` (see [`Button::mask`]).
    pub fn from_mask(mask: u8) -> Option<Self> {
        [Self::Scan, Self::Copy, Self::InstantShare]
            .into_iter()
            .find(|b| b.mask() == mask)
    }

    /// Short human-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Scan => "Scan",
            Self::Copy => "Copy",
            Self::InstantShare => "Instant Share",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_masks_match_the_hardware() {
        // E968 values read while each button was held.
        for (e968, button) in [
            (0xbb, Button::Scan),
            (0xbd, Button::Copy),
            (0xbe, Button::InstantShare),
        ] {
            assert_eq!(Button::from_mask(!e968 & 0x3f), Some(button));
        }
    }

    #[test]
    fn button_codes_round_trip() {
        for b in [Button::Scan, Button::Copy, Button::InstantShare] {
            assert_eq!(Button::from_code(b.code()), Some(b));
        }
        assert_eq!(Button::from_code(0x01), None);
    }
}
