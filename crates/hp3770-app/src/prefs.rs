// SPDX-License-Identifier: GPL-2.0-or-later
//! What the application remembers between runs, in a JSON file in the
//! user's configuration folder (`%APPDATA%\hp3770\impostazioni.json` on
//! Windows, `~/.config/hp3770/impostazioni.json` on Linux).
//!
//! The file mirrors the interface's settings with plain fields, so that it
//! stays readable and a field added later only takes its default.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The adjustments, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredAdjust {
    pub temperature: f32,
    pub tint: f32,
    pub exposure: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub vibrance: f32,
    pub black: f32,
    pub white: f32,
    pub gamma: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub sharpen: f32,
}

impl Default for StoredAdjust {
    fn default() -> Self {
        let a = hp3770_core::adjust::Adjustments::default();
        Self {
            temperature: a.temperature,
            tint: a.tint,
            exposure: a.exposure,
            highlights: a.highlights,
            shadows: a.shadows,
            whites: a.whites,
            blacks: a.blacks,
            vibrance: a.vibrance,
            black: a.black,
            white: a.white,
            gamma: a.gamma,
            brightness: a.brightness,
            contrast: a.contrast,
            saturation: a.saturation,
            sharpen: a.sharpen,
        }
    }
}

/// The settings of one way of working, as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredMode {
    pub dpi: u16,
    /// "colore", "grigio" or "bn".
    pub colour: String,
    pub threshold: u8,
    /// "pdf", "tiff-pagine", "png", "jpeg", "tiff", "webp" or "avif".
    pub output: String,
    /// "scansione", "a4" or "letter".
    pub pdf_size: String,
    pub auto_save: bool,
    pub adjust: StoredAdjust,
}

impl Default for StoredMode {
    fn default() -> Self {
        Self {
            dpi: 300,
            colour: "colore".into(),
            threshold: hp3770_core::convert::DEFAULT_THRESHOLD,
            output: "pdf".into(),
            pdf_size: "scansione".into(),
            auto_save: false,
            adjust: StoredAdjust::default(),
        }
    }
}

/// Everything remembered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// "documento", "foto" or "ultimo" (the last one used).
    pub start_mode: String,
    /// The last way of working used.
    pub last_mode: String,
    /// "sistema", "chiaro" or "scuro".
    pub theme: String,
    /// Where scans are saved; empty for Pictures\Scansioni.
    pub folder: String,
    /// File names start with this, then date and time.
    pub document_name: String,
    pub photo_name: String,
    pub document: StoredMode,
    pub photo: StoredMode,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            start_mode: "ultimo".into(),
            last_mode: "documento".into(),
            theme: "sistema".into(),
            folder: String::new(),
            document_name: "documento".into(),
            photo_name: "foto".into(),
            document: StoredMode::default(),
            photo: StoredMode {
                dpi: 600,
                output: "jpeg".into(),
                ..StoredMode::default()
            },
        }
    }
}

impl Prefs {
    /// The file the preferences live in.
    pub fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("hp3770").join("impostazioni.json"))
    }

    /// The saved preferences, or the defaults if there are none (or the
    /// file cannot be read).
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the preferences (errors are reported, not fatal).
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("cartella delle impostazioni sconosciuta")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_missing_fields() {
        let mut p = Prefs::default();
        p.document.output = "tiff-pagine".into();
        p.document.adjust.contrast = 12.0;
        let text = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&text).unwrap(), p);
        // A file from an older version: what is missing takes the default.
        let old: Prefs = serde_json::from_str(r#"{"theme":"scuro"}"#).unwrap();
        assert_eq!(old.theme, "scuro");
        assert_eq!(old.photo.dpi, 600);
    }
}
