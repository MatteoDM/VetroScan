// SPDX-License-Identifier: GPL-2.0-or-later
//! The tone tables WIA's scans load into the scanner, the same in every WIA
//! capture of the glass (11-12 November 2026): what was on the cable, not
//! read from Epson's library.

/// `1E B3`: one 256-byte table per channel, the same for the three:
/// `round(255 x (i / 255)^(1/1.8))`, exact on every value captured.
pub fn gamma() -> [u8; 256] {
    std::array::from_fn(|i| (255.0 * (i as f64 / 255.0).powf(1.0 / 1.8)).round() as u8)
}

/// `1E 84` type 02: 4,096 u16 per channel, the same for the three (12 bits in,
/// 12 bits out, rising from 0 to 4,095), as WIA sent them; no formula found.
pub const TONE: &[u8; 8192] = include_bytes!("../tables/wia_tone.bin");

/// The tables a scan loads into the scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Curves {
    /// WIA's: [`gamma`] and [`TONE`], as every capture.
    #[default]
    Wia,
    /// The identity in both (0..=255, 0..=4095): never sent by Epson's drivers; data the
    /// scanner looks samples up in, harmless to the mechanics. To be measured against WIA's
    /// (`docs/architecture-plan.md`, phase 12, "Linear samples").
    Linear,
}

impl Curves {
    /// The `1E B3` table, the same for the three channels.
    pub fn gamma(self) -> [u8; 256] {
        match self {
            Self::Wia => gamma(),
            Self::Linear => std::array::from_fn(|i| i as u8),
        }
    }

    /// The `1E 84` type 02 table, the same for the three channels.
    pub fn tone(self) -> Vec<u8> {
        match self {
            Self::Wia => TONE.to_vec(),
            Self::Linear => (0..4096u16).flat_map(|v| v.to_le_bytes()).collect(),
        }
    }
}

/// The `1E 84` headers of the three tone tables, one per channel.
pub const TONE_HEADERS: [[u8; 8]; 3] = [
    [0x02, 0x00, 0x00, 0x38, 0x00, 0x00, 0x20, 0x00],
    [0x02, 0x00, 0x20, 0x38, 0x00, 0x00, 0x20, 0x00],
    [0x02, 0x00, 0x40, 0x38, 0x00, 0x00, 0x20, 0x00],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_have_their_captured_shape() {
        // As capture 03 carried it after each 1E B3.
        let captured = "000c1116191d202325282a2c2f31333537393a3c3e404143454648494b4c4e4f50525355\
                        5657595a5b5c5e5f6061626465666768696b6c6d6e6f707172737475767778797a7b7c7d7e\
                        7f808182838485868788898a8b8b8c8d8e8f9091929293949596979898999a9b9c9d9d9e9f\
                        a0a1a1a2a3a4a5a5a6a7a8a9a9aaabacacadaeafafb0b1b2b2b3b4b5b5b6b7b7b8b9bababb\
                        bcbcbdbebfbfc0c1c1c2c3c3c4c5c6c6c7c8c8c9cacacbcccccdcececfd0d0d1d1d2d3d3d4\
                        d5d5d6d7d7d8d9d9dadadbdcdcdddededfdfe0e1e1e2e2e3e4e4e5e6e6e7e7e8e9e9eaeaeb\
                        ececededeeeeeff0f0f1f1f2f3f3f4f4f5f5f6f7f7f8f8f9f9fafbfbfcfcfdfdfefeff";
        assert_eq!(gamma().to_vec(), crate::setup::tests::hex(captured));
        let tone: Vec<u16> = TONE
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        assert_eq!(tone[..4], [0, 12, 24, 36]);
        assert_eq!(tone[4095], 4095);
        assert!(tone.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(Curves::Wia.tone(), TONE.to_vec());
        assert_eq!(Curves::Linear.tone().len(), TONE.len());
        assert_eq!(Curves::Linear.tone()[8190..], [0xFF, 0x0F]);
        assert_eq!(Curves::Linear.gamma()[200], 200);
    }
}
