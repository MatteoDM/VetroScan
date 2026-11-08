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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Points (input, output) of the curves: all channels, red, green,
    /// blue.
    pub curve_points: Vec<Vec<[f32; 2]>>,
    pub noise: f32,
    pub descreen: f32,
    pub adaptive: bool,
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
        let a = scan_imaging::adjust::Adjustments::default();
        Self {
            temperature: a.temperature,
            tint: a.tint,
            exposure: a.exposure,
            highlights: a.highlights,
            shadows: a.shadows,
            whites: a.whites,
            blacks: a.blacks,
            vibrance: a.vibrance,
            curve_points: a.curve.iter().map(|c| c.points().to_vec()).collect(),
            noise: a.noise,
            descreen: a.descreen,
            adaptive: a.adaptive,
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
    /// "scansione", "a4", "a3", "a5", "letter", "legal", "10x15" or "13x18".
    pub pdf_size: String,
    /// Which way the sheets are turned: "auto", "verticale" or
    /// "orizzontale".
    pub pdf_sheet: String,
    /// Where the scan goes on the sheet: "reale", "adatta" or "riempi".
    pub pdf_fit: String,
    /// Margin of a scan fitted to the sheet, millimetres (absent: the
    /// default).
    pub pdf_margin: Option<u8>,
    /// Quality of the PDF's pictures, 30 to 100% (absent: 100).
    pub pdf_quality: Option<u8>,
    /// Largest PDF in kilobytes (absent: no limit).
    pub pdf_limit_kb: Option<u32>,
    pub auto_save: bool,
    /// Find the originals on the glass and scan each on its own (absent:
    /// the default of the way of working).
    pub auto_crop: Option<bool>,
    /// Bits per sample asked of the scanner, 8 or 16 (absent: the default
    /// of the way of working).
    pub depth: Option<u8>,
    pub adjust: StoredAdjust,
}

impl Default for StoredMode {
    fn default() -> Self {
        Self {
            dpi: 300,
            colour: "colore".into(),
            threshold: scan_imaging::convert::DEFAULT_THRESHOLD,
            output: "pdf".into(),
            pdf_size: "scansione".into(),
            pdf_sheet: "auto".into(),
            pdf_fit: "reale".into(),
            pdf_margin: None,
            pdf_quality: None,
            pdf_limit_kb: None,
            auto_save: false,
            auto_crop: None,
            depth: None,
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
    /// What the Scan, Copy and Instant Share buttons do in each way of
    /// working (names of the actions; unknown names take the default).
    pub document_buttons: Vec<String>,
    pub photo_buttons: Vec<String>,
    pub document: StoredMode,
    pub photo: StoredMode,
    /// Minutes without scanning before the app switches the lamp off (0:
    /// as soon as a scan is over).
    pub lamp_off_minutes: u8,
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
            document_buttons: vec![
                "aggiungi-pagina".into(),
                "rifai-ultima".into(),
                "salva-documento".into(),
            ],
            photo_buttons: vec![
                "scansiona-foto".into(),
                "fotocopia".into(),
                "apri-cartella".into(),
            ],
            document: StoredMode::default(),
            photo: StoredMode {
                dpi: 600,
                output: "jpeg".into(),
                ..StoredMode::default()
            },
            lamp_off_minutes: 2,
        }
    }
}

/// The folder of the preferences, the presets and the log of the buttons.
/// In tests, a folder of the test's own: tests never touch the real
/// settings, nor each other's.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(crate::testing::dir().join("config"))
    }
    #[cfg(not(test))]
    {
        Some(dirs::config_dir()?.join("hp3770"))
    }
}

impl Prefs {
    /// The file the preferences live in.
    pub fn path() -> Option<PathBuf> {
        Some(config_dir()?.join("impostazioni.json"))
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

/// How many presets can be kept.
pub const MAX_PRESETS: usize = 3;

/// A named setup: everything of a way of working, adjustments included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub setup: StoredMode,
}

/// The file of the presets, next to the preferences.
pub fn presets_path() -> Option<PathBuf> {
    Some(config_dir()?.join("preset.json"))
}

/// The saved presets (none if there is no file or it cannot be read).
pub fn load_presets() -> Vec<Preset> {
    presets_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str::<Vec<Preset>>(&text).ok())
        .map(|mut v| {
            v.truncate(MAX_PRESETS);
            v
        })
        .unwrap_or_default()
}

/// Writes the presets.
pub fn save_presets(presets: &[Preset]) -> Result<(), String> {
    let path = presets_path().ok_or("cartella delle impostazioni sconosciuta")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(presets).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip() {
        let p = vec![Preset {
            name: "Documento nitido".into(),
            setup: StoredMode::default(),
        }];
        let text = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Preset>>(&text).unwrap(), p);
    }

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
