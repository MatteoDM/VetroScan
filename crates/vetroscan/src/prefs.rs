// SPDX-License-Identifier: GPL-2.0-or-later
//! What the application remembers between runs, in a JSON file in the
//! user's configuration folder, named after the app's package
//! (`%APPDATA%\vetroscan\impostazioni.json` on Windows,
//! `~/.config/vetroscan/impostazioni.json` on Linux; `hp3770` up to v0.2,
//! see [`config_dir`]).
//!
//! The file mirrors the interface's settings with plain fields, so that it
//! stays readable and a field added later only takes its default.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The scanner whose buttons the preferences before v0.2.0 meant, and the
/// ids of its buttons in that order.
pub const LEGACY_MODEL: &str = "HP Scanjet 3770"; // not translated: a model name
pub const LEGACY_BUTTONS: [&str; 3] = ["scan", "copy", "instant-share"];

/// What the buttons of one scanner model do in each way of working:
/// The stripes measured on a scanner at one resolution ("Misura le
/// righe").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredStripes {
    pub dpi: u16,
    /// Lines after which they repeat; 0: none worth taking away.
    pub repeat: u16,
    /// How strong they are, percent, row to row.
    pub percent: f32,
}

/// button id → name of the action.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredButtons {
    pub document: BTreeMap<String, String>,
    pub photo: BTreeMap<String, String>,
}

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
    /// Take away the scanner's stripes (absent: no).
    pub destripe: Option<bool>,
    /// Scan at twice the resolution and halve (absent: no).
    pub double: Option<bool>,
    /// Films: smooth the colour's noise (absent: the default of the way of
    /// working, yes for films).
    pub colour_noise: Option<bool>,
    /// Correct the colours with the scanner's matrix for where they come from, the glass or
    /// the film, where it declares one (absent: the default of the way of working, yes).
    pub scanner_colour: Option<bool>,
    /// "Senza elaborazioni": the scan as the scanner gives it (absent: no).
    pub untouched: Option<bool>,
    /// With `untouched`, negatives left as they are, not inverted (absent:
    /// inverted).
    pub keep_negative: Option<bool>,
    /// Films: read the infrared channel where the scanner offers it, for dust and scratches
    /// (absent: no).
    pub infrared: Option<bool>,
    /// Films: the base of the roll in the adapter, locked ("Blocca la base"),
    /// transmission per channel (absent: each strip measures its own).
    pub roll_base: Option<[f32; 3]>,
    /// Films: slides in the holder rather than negatives ("Diapositive";
    /// absent: negatives).
    pub slides: bool,
    /// Films: the negatives are black and white ("Negativi B/N": read as negatives, made
    /// grey; absent: colour).
    pub bw_negatives: Option<bool>,
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
            destripe: None,
            double: None,
            colour_noise: None,
            scanner_colour: None,
            untouched: None,
            keep_negative: None,
            infrared: None,
            roll_base: None,
            slides: false,
            bw_negatives: None,
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
    /// What the Scan, Copy and Instant Share buttons of the HP Scanjet 3770
    /// do in each way of working (names of the actions; unknown names take
    /// the default). Before v0.2.0 the only record; now a copy of
    /// [`Prefs::buttons`] for that model, kept for older versions.
    pub document_buttons: Vec<String>,
    pub photo_buttons: Vec<String>,
    /// What the buttons do, by scanner model ("HP Scanjet 3770").
    pub buttons: BTreeMap<String, StoredButtons>,
    pub document: StoredMode,
    pub photo: StoredMode,
    /// Films in the adapter (Foto, "Pellicola").
    pub film: StoredMode,
    /// Minutes without scanning before the app switches the lamp off (0:
    /// as soon as a scan is over).
    pub lamp_off_minutes: u8,
    /// "automatica" (the system's), "italiano" or "english".
    pub language: String,
    /// The stripes measured on each scanner ("HP Scanjet 3770"), in place
    /// of what its model declares.
    pub stripes: BTreeMap<String, Vec<StoredStripes>>,
    /// Whether [`Prefs::migrated`] has cleared the "off" every older file keeps for the
    /// scanner's colours in Documento and Foto: the glass had no such correction then, and its
    /// default became "on" (14 November 2026).
    pub glass_colour_default: bool,
    /// What the screen is taken to be: "automatico" (the monitor's
    /// profile; also when absent, files before 0.10), "srgb", "nessuno",
    /// or the path of a profile.
    pub monitor: String,
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
            buttons: BTreeMap::new(),
            document: StoredMode::default(),
            photo: StoredMode {
                dpi: 600,
                output: "jpeg".into(),
                ..StoredMode::default()
            },
            film: StoredMode {
                dpi: 1200,
                output: "jpeg".into(),
                ..StoredMode::default()
            },
            lamp_off_minutes: 2,
            language: "automatica".into(),
            stripes: BTreeMap::new(),
            glass_colour_default: false,
            monitor: "automatico".into(),
        }
    }
}

/// The name of the settings folder before v0.3, when the app was named
/// after the scanner.
pub const LEGACY_DIR: &str = "hp3770";

/// The folder of the preferences, the presets and the log of the buttons:
/// the app's own (its package name), filled the first time with a copy of
/// the folder of older versions. In tests, a folder of the test's own:
/// tests never touch the real settings, nor each other's.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(crate::testing::dir().join("config"))
    }
    #[cfg(not(test))]
    {
        let base = dirs::config_dir()?;
        let dir = base.join(env!("CARGO_PKG_NAME"));
        static MIGRATED: std::sync::Once = std::sync::Once::new();
        MIGRATED.call_once(|| {
            let _ = migrate_dir(&base.join(LEGACY_DIR), &dir);
        });
        Some(dir)
    }
}

/// What a file of the settings folder holds, read as JSON: `None` when there
/// is none. A file that is there but cannot be read so (a save cut short) is
/// kept aside, as `<name>.illeggibile`, so that the next save does not write
/// over what is left of it.
fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read(path).ok()?;
    match serde_json::from_slice(&text) {
        Ok(value) => Some(value),
        Err(_) => {
            let mut aside = path.as_os_str().to_owned();
            aside.push(".illeggibile");
            let _ = std::fs::rename(path, aside);
            None
        }
    }
}

/// Writes `text` as the file `path` of the settings folder: into a file
/// beside it first, which takes its place only once complete, so that a
/// save cut short leaves the previous file whole.
fn write_safely(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&part)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&part, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written.map_err(|e| e.to_string())
}

/// Copies the files of the settings folder of older versions (`old`) into
/// the new one (`new`), only when the new one does not exist yet; says
/// whether it did. The old folder stays as it is, for an older version.
pub fn migrate_dir(old: &std::path::Path, new: &std::path::Path) -> std::io::Result<bool> {
    if new.exists() || !old.is_dir() {
        return Ok(false);
    }
    std::fs::create_dir_all(new)?;
    for entry in std::fs::read_dir(old)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), new.join(entry.file_name()))?;
        }
    }
    Ok(true)
}

impl Prefs {
    /// The file the preferences live in.
    pub fn path() -> Option<PathBuf> {
        Some(config_dir()?.join("impostazioni.json"))
    }

    /// The saved preferences, or the defaults if there are none (or the
    /// file cannot be read: it is then kept aside, see [`read_json`]).
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| read_json::<Self>(&p))
            .unwrap_or_default()
            .migrated()
    }

    /// Preferences of an older version brought up to date: the actions of
    /// the HP Scanjet 3770's buttons, by position, become the actions of
    /// its buttons by id; the "off" kept for the scanner's colours in Documento and Foto,
    /// once, becomes the default (on).
    pub fn migrated(mut self) -> Self {
        if !self.glass_colour_default {
            self.document.scanner_colour = None;
            self.photo.scanner_colour = None;
            self.glass_colour_default = true;
        }
        if !self.buttons.contains_key(LEGACY_MODEL) {
            let by_id = |actions: &[String]| -> BTreeMap<String, String> {
                LEGACY_BUTTONS
                    .iter()
                    .zip(actions)
                    .map(|(id, a)| ((*id).to_string(), a.clone()))
                    .collect()
            };
            let stored = StoredButtons {
                document: by_id(&self.document_buttons),
                photo: by_id(&self.photo_buttons),
            };
            self.buttons.insert(LEGACY_MODEL.into(), stored);
        }
        self
    }

    /// Writes the preferences (errors are reported, not fatal).
    pub fn save(&self) -> Result<(), String> {
        let path =
            Self::path().ok_or(crate::i18n::tr("cartella delle impostazioni sconosciuta"))?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        write_safely(&path, &text)
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

/// The saved presets (none if there is no file or it cannot be read: it is
/// then kept aside, see [`read_json`]).
pub fn load_presets() -> Vec<Preset> {
    presets_path()
        .and_then(|p| read_json::<Vec<Preset>>(&p))
        .map(|mut v| {
            v.truncate(MAX_PRESETS);
            v
        })
        .unwrap_or_default()
}

/// Writes the presets.
pub fn save_presets(presets: &[Preset]) -> Result<(), String> {
    let path = presets_path().ok_or(crate::i18n::tr("cartella delle impostazioni sconosciuta"))?;
    let text = serde_json::to_string_pretty(presets).map_err(|e| e.to_string())?;
    write_safely(&path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_file_is_kept_aside_and_a_save_replaces_it_whole() {
        let dir = crate::testing::dir().join("unreadable");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("impostazioni.json");
        // Half written: the defaults, and the remains kept, not overwritten.
        std::fs::write(&path, "{\"theme\": \"scu").unwrap();
        assert_eq!(read_json::<Prefs>(&path), None);
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("impostazioni.json.illeggibile")).unwrap(),
            "{\"theme\": \"scu"
        );
        // No file at all: nothing kept aside.
        assert_eq!(read_json::<Prefs>(&dir.join("none.json")), None);
        // A save goes through a file beside it, renamed over it.
        let prefs = Prefs {
            theme: "scuro".into(),
            ..Prefs::default()
        };
        write_safely(&path, &serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(read_json::<Prefs>(&path).unwrap().theme, "scuro");
        assert!(!dir.join("impostazioni.json.part").exists());
    }

    #[test]
    fn the_settings_of_older_versions_move_to_the_new_folder_once() {
        let base = crate::testing::dir().join("migration");
        let (old, new) = (base.join(LEGACY_DIR), base.join("app"));
        // Nothing to bring: no old folder.
        assert!(!migrate_dir(&old, &new).unwrap());
        assert!(!new.exists());
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("impostazioni.json"), "{\"theme\": \"scuro\"}").unwrap();
        std::fs::write(old.join("preset.json"), "[]").unwrap();
        assert!(migrate_dir(&old, &new).unwrap());
        assert_eq!(
            std::fs::read_to_string(new.join("impostazioni.json")).unwrap(),
            "{\"theme\": \"scuro\"}"
        );
        assert!(new.join("preset.json").exists());
        // The old folder stays, and a second start copies nothing over.
        assert!(old.join("impostazioni.json").exists());
        std::fs::write(new.join("impostazioni.json"), "{}").unwrap();
        assert!(!migrate_dir(&old, &new).unwrap());
        assert_eq!(
            std::fs::read_to_string(new.join("impostazioni.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn button_actions_by_position_become_actions_by_id() {
        // A file of v0.1.x: actions by position only.
        let old = r#"{"document_buttons": ["rifai-ultima", "aggiungi-pagina", "niente"],
                      "photo_buttons": ["fotocopia"]}"#;
        let p = serde_json::from_str::<Prefs>(old).unwrap().migrated();
        let hp = &p.buttons[LEGACY_MODEL];
        assert_eq!(hp.document["scan"], "rifai-ultima");
        assert_eq!(hp.document["copy"], "aggiungi-pagina");
        assert_eq!(hp.document["instant-share"], "niente");
        assert_eq!(hp.photo["scan"], "fotocopia");
        assert!(!hp.photo.contains_key("copy"), "the rest takes the default");
        // Once migrated, the record by id wins.
        let mut again = p.clone();
        again.document_buttons = vec!["niente".into()];
        assert_eq!(
            again.migrated().buttons[LEGACY_MODEL].document["scan"],
            "rifai-ultima"
        );
        // A new file: the defaults, by id.
        let fresh = Prefs::default().migrated();
        assert_eq!(
            fresh.buttons[LEGACY_MODEL].document["scan"],
            "aggiungi-pagina"
        );
    }

    #[test]
    fn the_glass_colour_off_of_older_files_becomes_the_default_once() {
        // A file of v0.7: the scanner's colours kept "off" in Documento and Foto (the glass
        // had no correction then), on for films.
        let old = r#"{"document": {"dpi": 300, "colour": "colore", "threshold": 128,
                        "output": "pdf", "pdf_size": "scansione", "pdf_sheet": "auto",
                        "pdf_fit": "reale", "auto_save": false, "scanner_colour": false,
                        "slides": false, "adjust": {}},
                      "film": {"dpi": 1200, "colour": "colore", "threshold": 128,
                        "output": "jpeg", "pdf_size": "scansione", "pdf_sheet": "auto",
                        "pdf_fit": "reale", "auto_save": false, "scanner_colour": false,
                        "slides": false, "adjust": {}}}"#;
        let p = serde_json::from_str::<Prefs>(old).unwrap().migrated();
        assert_eq!(p.document.scanner_colour, None, "the default, on");
        assert_eq!(p.photo.scanner_colour, None);
        assert_eq!(p.film.scanner_colour, Some(false), "films: a choice, kept");
        assert!(p.glass_colour_default);
        // Once migrated, an "off" chosen since stays.
        let mut again = p.clone();
        again.document.scanner_colour = Some(false);
        assert_eq!(again.migrated().document.scanner_colour, Some(false));
    }

    #[test]
    fn files_before_the_screens_colours_take_the_monitors_profile() {
        // A file of v0.9: no "monitor".
        let p = serde_json::from_str::<Prefs>(r#"{"theme": "scuro"}"#)
            .unwrap()
            .migrated();
        assert_eq!(p.monitor, "automatico");
        assert_eq!(
            crate::screen::Choice::from_key(&p.monitor),
            crate::screen::Choice::Auto
        );
    }

    #[test]
    fn files_before_senza_elaborazioni_scan_as_before() {
        // A file of v0.9: no "untouched" nor "keep_negative".
        let old = r#"{"film": {"dpi": 1200, "colour": "colore", "threshold": 128,
                        "output": "jpeg", "pdf_size": "scansione", "pdf_sheet": "auto",
                        "pdf_fit": "reale", "auto_save": false, "slides": false,
                        "adjust": {}}}"#;
        let p = serde_json::from_str::<Prefs>(old).unwrap().migrated();
        assert_eq!(p.film.untouched, None, "processed, as before");
        assert_eq!(p.film.keep_negative, None);
        let mut chosen = p.clone();
        chosen.film.untouched = Some(true);
        chosen.film.keep_negative = Some(true);
        let text = serde_json::to_string(&chosen).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&text).unwrap(), chosen);
    }

    #[test]
    fn files_before_the_infrared_read_none() {
        // A file of v0.10: no "infrared".
        let old = r#"{"film": {"dpi": 1200, "colour": "colore", "threshold": 128,
                        "output": "jpeg", "pdf_size": "scansione", "pdf_sheet": "auto",
                        "pdf_fit": "reale", "auto_save": false, "slides": false,
                        "untouched": false, "adjust": {}}}"#;
        let p = serde_json::from_str::<Prefs>(old).unwrap().migrated();
        assert_eq!(p.film.infrared, None, "no infrared pass, as before");
        let mut chosen = p.clone();
        chosen.film.infrared = Some(true);
        let text = serde_json::to_string(&chosen).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&text).unwrap(), chosen);
    }

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
        // One of 0.5-0.6 with the Epson firmware's path, which is no
        // longer a preference (the driver finds its firmware itself).
        let epson: Prefs =
            serde_json::from_str(r#"{"theme":"scuro","epson_firmware":"C:\\esfw54.bin"}"#).unwrap();
        assert_eq!(epson.theme, "scuro");
    }

    /// A film's settings from before "Negativi B/N" (v0.10): colour negatives; the choice
    /// kept once made, and left out of the file when it is not.
    #[test]
    fn black_and_white_negatives_are_remembered() {
        let old = r#"{"film": {"dpi": 1200, "colour": "colore", "threshold": 128,
                        "output": "jpeg", "pdf_size": "scansione", "pdf_sheet": "auto",
                        "pdf_fit": "reale", "auto_save": false, "slides": false,
                        "adjust": {}}}"#;
        let p = serde_json::from_str::<Prefs>(old).unwrap().migrated();
        assert_eq!(p.film.bw_negatives, None, "colour negatives");
        let mut bw = Prefs::default();
        bw.film.bw_negatives = Some(true);
        bw.film.colour = "grigio".into();
        let text = serde_json::to_string(&bw).unwrap();
        let back = serde_json::from_str::<Prefs>(&text).unwrap().migrated();
        assert_eq!(
            (back.film.bw_negatives, back.film.colour.as_str()),
            (Some(true), "grigio")
        );
    }
}
