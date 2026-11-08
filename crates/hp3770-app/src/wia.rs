// SPDX-License-Identifier: GPL-2.0-or-later
//! What Windows does when a scanner button is pressed.
//!
//! The still-image service (WIA) also sees the presses, and by default
//! opens its own scan application, which takes the scanner away from this
//! one. Each button is an event of the device, under
//! `HKLM\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-…}\NNNN\Events`;
//! its `DefaultHandler` value names what runs. Choosing "Take no action"
//! in the scanner's Events tab writes the handler
//! [`NO_ACTION`] there, and that is all this module does (with one
//! administrator confirmation, as the key is the system's). Removing the
//! value gives Windows its default back.

/// WIA's "take no action" event handler.
#[cfg(windows)]
pub const NO_ACTION: &str = "{E0372B7D-E115-4525-BC55-B629E68C745A}";

#[cfg(windows)]
const CLASS: &str =
    r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}";

#[cfg(windows)]
fn reg(args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The event keys of the 3770's buttons.
#[cfg(windows)]
fn keys() -> Vec<String> {
    reg(&["query", CLASS, "/s", "/k", "/f", "3770_"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("HKEY_") && l.contains(r"\Events\3770_"))
        .map(|l| l.replacen("HKEY_LOCAL_MACHINE", "HKLM", 1))
        .collect()
}

/// Whether Windows leaves the buttons alone: `None` when there is nothing
/// to set (not Windows, or the HP driver is not installed).
#[cfg(windows)]
pub fn quiet() -> Option<bool> {
    let keys = keys();
    if keys.is_empty() {
        return None;
    }
    Some(
        keys.iter().all(|k| {
            reg(&["query", k, "/v", "DefaultHandler"]).is_some_and(|o| o.contains(NO_ACTION))
        }),
    )
}

#[cfg(not(windows))]
pub fn quiet() -> Option<bool> {
    None
}

/// Makes Windows leave the buttons alone (`true`) or gives it its default
/// back, asking once for the administrator's confirmation. Waits for it.
#[cfg(windows)]
pub fn set_quiet(on: bool) -> Result<(), String> {
    let keys = keys();
    if keys.is_empty() {
        return Err("tasti dello scanner non trovati nel registro".into());
    }
    let commands: Vec<String> = keys
        .iter()
        .map(|k| {
            if on {
                format!("reg add \"{k}\" /v DefaultHandler /t REG_SZ /d {NO_ACTION} /f")
            } else {
                format!("reg delete \"{k}\" /v DefaultHandler /f")
            }
        })
        .collect();
    let line = commands.join(" & ");
    // PowerShell starts cmd elevated (the confirmation) and waits.
    let script = format!(
        "Start-Process -FilePath cmd.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '/c {}'",
        line.replace('\'', "''")
    );
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("conferma di amministratore negata".into());
    }
    match quiet() {
        Some(now) if now == on => Ok(()),
        _ => Err("Windows non ha accettato la modifica".into()),
    }
}

#[cfg(not(windows))]
pub fn set_quiet(_on: bool) -> Result<(), String> {
    Err("solo su Windows".into())
}
