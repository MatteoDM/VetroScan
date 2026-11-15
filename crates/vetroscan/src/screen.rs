// SPDX-License-Identifier: GPL-2.0-or-later
//! The colours of the screen (phase 14, step 3): what the app shows is
//! converted from each page's profile (the one its file carries:
//! [`vetroscan_export::icc::rgb`] of the gamma it was drawn with) to the
//! monitor's, as colour-managed programs do. Only the picture on the
//! screen changes: the page's numbers, the histogram, the colour picker
//! and the files stay as they are.
//!
//! The monitor's profile is, by default, that of the monitor the window
//! is on ([`find`]: on Windows, asked of the system; elsewhere not yet).
//! Whatever cannot be found or read gives sRGB, which only brings each
//! page's gamma to sRGB's: never worse than drawing the numbers as they
//! are. The conversions are made with qcms (Firefox's, pure Rust):
//! measured on 15 November 2026 against littleCMS on Matteo's monitor
//! profiles, the same numbers, where moxcms got a LUT profile wrong.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// What the screen is taken to be, as chosen in Preferences.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Choice {
    /// The profile of the monitor the window is on (sRGB where it is not
    /// known).
    #[default]
    Auto,
    /// sRGB: each page brought to sRGB's gamma.
    Srgb,
    /// No conversion: the numbers as they are (as before 0.10).
    Off,
    /// A profile file chosen by hand.
    File(PathBuf),
}

impl Choice {
    /// As saved in the preferences.
    pub fn key(&self) -> String {
        match self {
            Self::Auto => "automatico".into(),
            Self::Srgb => "srgb".into(),
            Self::Off => "nessuno".into(),
            Self::File(path) => path.display().to_string(),
        }
    }

    /// From the preferences (empty or unknown: automatic).
    pub fn from_key(key: &str) -> Self {
        match key {
            "" | "automatico" => Self::Auto,
            "srgb" => Self::Srgb,
            "nessuno" => Self::Off,
            path => Self::File(PathBuf::from(path)),
        }
    }
}

/// A monitor found: its name and the bytes of its profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    /// The system's name of the monitor (`DISPLAY1`).
    pub name: String,
    /// The profile's file name, without its folder.
    pub profile_name: String,
    pub profile: Vec<u8>,
}

/// What the screen shows the pages through.
pub struct Screen {
    /// The profile shown into; `None`: the numbers as they are.
    target: Option<Box<qcms::Profile>>,
    /// What it is, for Preferences ("sRGB", "CG277 … (DISPLAY1)").
    pub label: String,
    /// The conversions made so far, one per page gamma.
    transforms: Mutex<Vec<(u64, Option<Arc<qcms::Transform>>)>>,
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Screen") // not translated: a type
            .field("label", &self.label)
            .finish()
    }
}

impl PartialEq for Screen {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label
    }
}

impl Screen {
    /// No conversion: the numbers as they are.
    pub fn off() -> Self {
        Self::with(None, String::new())
    }

    /// sRGB.
    pub fn srgb() -> Self {
        let mut srgb = qcms::Profile::new_sRGB();
        srgb.precache_output_transform();
        Self::with(Some(srgb), "sRGB".into())
    }

    /// A profile's bytes, named `label`; `None` when it cannot be read as
    /// an RGB profile to show into.
    pub fn profile(bytes: &[u8], label: String) -> Option<Self> {
        let mut profile = qcms::Profile::new_from_slice(bytes, false)?;
        profile.precache_output_transform();
        // Shown into from an RGB page, or it is no monitor's.
        let page = qcms::Profile::new_from_slice(&vetroscan_export::icc::rgb(2.2), false)?;
        qcms::Transform::new(
            &page,
            &profile,
            qcms::DataType::RGBA8,
            qcms::Intent::Perceptual,
        )?;
        Some(Self::with(Some(profile), label))
    }

    fn with(target: Option<Box<qcms::Profile>>, label: String) -> Self {
        Self {
            target,
            label,
            transforms: Mutex::new(Vec::new()),
        }
    }

    /// Whether it converts at all.
    pub fn converts(&self) -> bool {
        self.target.is_some()
    }

    /// The conversion of a page drawn at `gamma`, made once.
    fn transform(&self, gamma: f64) -> Option<Arc<qcms::Transform>> {
        let target = self.target.as_ref()?;
        let key = gamma.to_bits();
        let mut made = self.transforms.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, t)) = made.iter().find(|(k, _)| *k == key) {
            return t.clone();
        }
        let t = qcms::Profile::new_from_slice(&vetroscan_export::icc::rgb(gamma), false)
            .and_then(|page| {
                qcms::Transform::new(
                    &page,
                    target,
                    qcms::DataType::RGBA8,
                    qcms::Intent::Perceptual,
                )
            })
            .map(Arc::new);
        made.push((key, t.clone()));
        t
    }

    /// Converts RGBA pixels of a page drawn at `gamma` for the screen, in
    /// strips on every core (a LUT profile takes about 100 ms for a
    /// 2400 x 1600 picture on one).
    pub fn show(&self, rgba: &mut [u8], gamma: f64) {
        let Some(t) = self.transform(gamma) else {
            return;
        };
        const STRIP: usize = 256 * 1024 * 4;
        if rgba.len() <= STRIP {
            t.apply(rgba);
            return;
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let per = rgba.len().div_ceil(threads).div_ceil(4) * 4;
        std::thread::scope(|scope| {
            for strip in rgba.chunks_mut(per.max(STRIP)) {
                let t = &t;
                scope.spawn(move || t.apply(strip));
            }
        });
    }
}

/// The screen for `choice`, with the monitor found (for
/// [`Choice::Auto`]) and a note on what was found when it is not what was
/// asked (in Italian, through `tr`).
pub fn for_choice(choice: &Choice, found: Option<&Result<Monitor, String>>) -> (Screen, String) {
    use crate::i18n::{tr, trf};
    match choice {
        Choice::Off => (Screen::off(), String::new()),
        Choice::Srgb => (Screen::srgb(), String::new()),
        Choice::File(path) => {
            let name = file_name(path);
            match std::fs::read(path) {
                Ok(bytes) => match Screen::profile(&bytes, name.clone()) {
                    Some(screen) => (screen, String::new()),
                    None => (
                        Screen::srgb(),
                        trf(
                            "{file} non è un profilo di monitor: uso sRGB",
                            &[("file", &name)],
                        ),
                    ),
                },
                Err(e) => (
                    Screen::srgb(),
                    trf(
                        "{file} non si legge ({e}): uso sRGB",
                        &[("file", &name), ("e", &e)],
                    ),
                ),
            }
        }
        Choice::Auto => match found {
            Some(Ok(monitor)) => {
                let label = format!("{} ({})", monitor.profile_name, monitor.name);
                match Screen::profile(&monitor.profile, label) {
                    Some(screen) => (screen, String::new()),
                    None => (
                        Screen::srgb(),
                        trf(
                            "Il profilo di {monitor} ({file}) non si legge: uso sRGB",
                            &[("monitor", &monitor.name), ("file", &monitor.profile_name)],
                        ),
                    ),
                }
            }
            Some(Err(e)) => (Screen::srgb(), e.clone()),
            None => (Screen::srgb(), tr("Cerco il profilo del monitor…").into()),
        },
    }
}

/// Where the system keeps the colour profiles, for the dialog that picks
/// one.
pub fn profiles_folder() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        PathBuf::from(root).join(r"System32\spool\drivers\color")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/ColorSync/Profiles")
    } else {
        dirs::data_dir().map_or_else(|| PathBuf::from("/usr/share/color/icc"), |d| d.join("icc"))
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The profile of the monitor the app's window is on (the process `pid`'s
/// window): asked of the system, which takes about a second. An error, in
/// Italian through `tr`, where it is not known. In tests, what the test set
/// ([`crate::testing::set_monitor`]).
pub fn find(pid: u32) -> Result<Monitor, String> {
    #[cfg(test)]
    {
        let _ = pid;
        crate::testing::monitor()
    }
    #[cfg(not(test))]
    {
        find_monitor(pid)
    }
}

#[cfg(all(windows, not(test)))]
fn find_monitor(pid: u32) -> Result<Monitor, String> {
    use crate::i18n::{tr, trf};
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // The window's monitor (MonitorFromWindow, GetMonitorInfo) and the
    // profile Windows uses for it (GetICMProfile), through PowerShell: the
    // workspace forbids `unsafe`, so no system call of our own.
    let script =
        format!("{WINDOWS_SCRIPT}\n[VetroScanIcm]::Of((Get-Process -Id {pid}).MainWindowHandle)");
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| trf("Profilo del monitor non letto: {e}", &[("e", &e)]))?;
    let answer = String::from_utf8_lossy(&out.stdout);
    let (device, path) = answer.trim().split_once('|').ok_or_else(|| {
        tr("Windows non dice su quale monitor è la finestra: uso sRGB").to_string()
    })?;
    let name = device.trim_start_matches(r"\\.\").to_string();
    if path.is_empty() {
        return Err(trf(
            "{monitor} non ha un profilo colore: uso sRGB",
            &[("monitor", &name)],
        ));
    }
    let path = PathBuf::from(path);
    let profile = std::fs::read(&path).map_err(|e| {
        trf(
            "Il profilo di {monitor} non si legge ({e}): uso sRGB",
            &[("monitor", &name), ("e", &e)],
        )
    })?;
    Ok(Monitor {
        name,
        profile_name: file_name(&path),
        profile,
    })
}

#[cfg(all(not(windows), not(test)))]
fn find_monitor(_pid: u32) -> Result<Monitor, String> {
    Err(crate::i18n::tr(
        "Il profilo del monitor si trova da solo solo su Windows, per ora: uso sRGB",
    )
    .into())
}

/// The C# PowerShell compiles to ask Windows for the monitor of a window
/// and its colour profile (measured on 15 November 2026: about a second).
#[cfg(all(windows, not(test)))]
const WINDOWS_SCRIPT: &str = include_str!("monitor.ps1");

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture of four pixels: greys 64, 128, 192 and a red.
    fn pixels() -> Vec<u8> {
        vec![
            64, 64, 64, 255, 128, 128, 128, 255, 192, 192, 192, 255, 200, 80, 40, 255,
        ]
    }

    fn rgb(p: &[u8]) -> Vec<[u8; 3]> {
        p.chunks(4).map(|c| [c[0], c[1], c[2]]).collect()
    }

    /// Within one level of littleCMS's numbers (Pillow's ImageCms).
    fn near(got: &[[u8; 3]], expect: &[[u8; 3]]) -> bool {
        got.iter()
            .flatten()
            .zip(expect.iter().flatten())
            .all(|(a, b)| a.abs_diff(*b) <= 1)
    }

    #[test]
    fn a_page_at_1_8_on_srgb_as_littlecms_converts_it() {
        let mut p = pixels();
        Screen::srgb().show(&mut p, 1.8);
        // littleCMS, Epson Scan's profile and ours alike (15 November 2026).
        let expect = [
            [81, 81, 81],
            [146, 146, 146],
            [203, 203, 203],
            [210, 99, 53],
        ];
        assert!(near(&rgb(&p), &expect), "{:?}", rgb(&p));
        // The alpha kept.
        assert!(p.chunks(4).all(|c| c[3] == 255));
    }

    #[test]
    fn a_page_at_2_2_on_srgb_moves_only_in_the_shadows() {
        // A plain 2.2 against sRGB's curve, as littleCMS converts it (15
        // November 2026): a level or two above the shadows, more in them.
        let mut p = pixels();
        Screen::srgb().show(&mut p, 2.2);
        let expect = [
            [62, 62, 62],
            [129, 129, 129],
            [193, 193, 193],
            [201, 79, 35],
        ];
        assert!(near(&rgb(&p), &expect), "{:?}", rgb(&p));
    }

    #[test]
    fn a_wide_gamut_monitor_gets_colours_less_saturated() {
        // Adobe RGB's primaries (D50), gamma 2.2: greys stay, the red
        // comes closer to grey.
        let adobe = vetroscan_export::icc::matrix(
            "Adobe RGB-like",
            [
                [0.6097, 0.3111, 0.0195],
                [0.2053, 0.6257, 0.0609],
                [0.1492, 0.0632, 0.7446],
            ],
            2.2,
        );
        let screen = Screen::profile(&adobe, "wide".into()).unwrap();
        let mut p = pixels();
        screen.show(&mut p, 2.2);
        let got = rgb(&p);
        // littleCMS from our 2.2 profile to this one (15 November 2026).
        let expect = [
            [64, 64, 64],
            [128, 128, 128],
            [192, 192, 192],
            [176, 80, 43],
        ];
        assert!(near(&got, &expect), "{got:?}");
    }

    #[test]
    fn off_leaves_the_numbers_and_a_large_picture_is_done_in_strips() {
        let mut p = pixels();
        Screen::off().show(&mut p, 1.8);
        assert_eq!(p, pixels());
        // 2 million pixels, several strips: each converted as one alone.
        let mut big: Vec<u8> = pixels().repeat(500_000);
        Screen::srgb().show(&mut big, 1.8);
        let mut one = pixels();
        Screen::srgb().show(&mut one, 1.8);
        assert!(big.chunks(16).all(|c| c == one.as_slice()));
    }

    #[test]
    fn what_is_not_a_monitor_profile_gives_srgb_and_says_so() {
        let dir = crate::testing::dir();
        let path = dir.join("not-a-profile.icc");
        std::fs::write(&path, b"nothing").unwrap();
        let (screen, note) = for_choice(&Choice::File(path), None);
        assert_eq!(screen.label, "sRGB");
        assert!(note.contains("not-a-profile.icc"), "{note}");
        let (screen, note) = for_choice(&Choice::Auto, Some(&Err("no monitor".into())));
        assert_eq!(screen.label, "sRGB");
        assert_eq!(note, "no monitor");
        crate::testing::clean_up();
    }

    #[test]
    fn the_choice_is_saved_and_read_back() {
        for c in [
            Choice::Auto,
            Choice::Srgb,
            Choice::Off,
            Choice::File(PathBuf::from("C:/x/CG277.icm")),
        ] {
            assert_eq!(Choice::from_key(&c.key()), c);
        }
        // Files before it: automatic.
        assert_eq!(Choice::from_key(""), Choice::Auto);
    }
}
