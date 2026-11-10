// SPDX-License-Identifier: GPL-2.0-or-later
//! What Windows does when a scanner button is pressed.
//!
//! When the scanner's maker installed its own driver, the still-image
//! service (WIA) also sees the presses, and by default opens its own scan
//! application, which takes the scanner away from this one. The driver is
//! an instance of the imaging devices' class, under
//! `HKLM\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-…}\NNNN`, whose
//! `MatchingDeviceId` is the scanner's USB id (`usb\vid_03f0&pid_2505`);
//! each event of the device is a key under its `Events`, and its
//! `DefaultHandler` value names what runs. Choosing "Take no action" in the
//! scanner's Events tab writes the handler [`NO_ACTION`] there, and that is
//! all this module does (with one administrator confirmation, as the key is
//! the system's). Removing the value gives Windows its default back. The
//! scanner is found by the USB id its driver declares, whoever made it.

use scanner_api::UsbId;

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

/// `HKEY_LOCAL_MACHINE\…` as `reg` takes it back.
#[cfg(any(windows, test))]
fn short(key: &str) -> String {
    // not translated: registry names
    key.replacen("HKEY_LOCAL_MACHINE", "HKLM", 1)
}

/// The keys of the class's instances whose `MatchingDeviceId` is this USB
/// id, from `reg query <class> /s /v MatchingDeviceId`.
#[cfg(any(windows, test))]
fn instances(listing: &str, usb: UsbId) -> Vec<String> {
    // not translated: a registry value
    let id = format!(r"usb\vid_{:04x}&pid_{:04x}", usb.vendor, usb.product);
    let mut key = None;
    let mut found = Vec::new();
    for line in listing.lines().map(str::trim) {
        if line.starts_with("HKEY_") {
            key = Some(line);
            continue;
        }
        let mut words = line.split_whitespace();
        // not translated: a registry value
        if words.next() != Some("MatchingDeviceId") || words.next() != Some("REG_SZ") {
            continue;
        }
        let value = words.next().unwrap_or_default().to_ascii_lowercase();
        let same = value == id || value.starts_with(&format!("{id}&"));
        if let (true, Some(k)) = (same, key) {
            found.push(short(k));
        }
    }
    found
}

/// The event keys listed by `reg query <instance>\Events`.
#[cfg(any(windows, test))]
fn events(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        // not translated: registry names
        .filter(|l| l.starts_with("HKEY_") && l.contains(r"\Events\"))
        .map(short)
        .collect()
}

/// The event keys of the driver Windows has for this scanner.
#[cfg(windows)]
fn keys(usb: UsbId) -> Vec<String> {
    // not translated: registry names
    let listing = reg(&["query", CLASS, "/s", "/v", "MatchingDeviceId"]).unwrap_or_default();
    instances(&listing, usb)
        .iter()
        .flat_map(|k| events(&reg(&["query", &format!(r"{k}\Events")]).unwrap_or_default()))
        .collect()
}

/// Whether Windows leaves the scanner's buttons alone: `None` when there is
/// nothing to set (not Windows, not a USB scanner, or no driver of its
/// maker with events).
#[cfg(windows)]
pub fn quiet(usb: Option<UsbId>) -> Option<bool> {
    let keys = keys(usb?);
    if keys.is_empty() {
        return None;
    }
    Some(keys.iter().all(|k| {
        // not translated: a registry value
        reg(&["query", k, "/v", "DefaultHandler"]).is_some_and(|o| o.contains(NO_ACTION))
    }))
}

#[cfg(not(windows))]
pub fn quiet(_usb: Option<UsbId>) -> Option<bool> {
    None
}

/// Makes Windows leave the buttons alone (`true`) or gives it its default
/// back, asking once for the administrator's confirmation. Waits for it.
#[cfg(windows)]
pub fn set_quiet(usb: Option<UsbId>, on: bool) -> Result<(), String> {
    let keys = usb.map(keys).unwrap_or_default();
    if keys.is_empty() {
        return Err(crate::i18n::tr("tasti dello scanner non trovati nel registro").into());
    }
    let commands: Vec<String> = keys
        .iter()
        .map(|k| {
            if on {
                // not translated: commands
                format!("reg add \"{k}\" /v DefaultHandler /t REG_SZ /d {NO_ACTION} /f")
            } else {
                // not translated: commands
                format!("reg delete \"{k}\" /v DefaultHandler /f")
            }
        })
        .collect();
    let line = commands.join(" & ");
    // PowerShell starts cmd elevated (the confirmation) and waits.
    let script = format!(
        // not translated: a command
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
        return Err(crate::i18n::tr("conferma di amministratore negata").into());
    }
    match quiet(usb) {
        Some(now) if now == on => Ok(()),
        _ => Err(crate::i18n::tr("Windows non ha accettato la modifica").into()),
    }
}

#[cfg(not(windows))]
pub fn set_quiet(_usb: Option<UsbId>, _on: bool) -> Result<(), String> {
    Err(crate::i18n::tr("solo su Windows").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As `reg` printed them on 10 November 2026, with the HP 3770's driver
    /// installed (the Italian Windows' last line), and a made-up second
    /// instance.
    const CLASS_LISTING: &str = r"
HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}\0000
    MatchingDeviceId    REG_SZ    usb\vid_03f0&pid_2505

HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}\0001
    MatchingDeviceId    REG_SZ    USB\VID_04B8&PID_0119&REV_0100

Fine della ricerca: 2 corrispondenze trovate.
";

    const EVENTS_LISTING: &str = r"
HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}\0000\Events\3770_COPY
HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}\0000\Events\3770_SCAN
HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}\0000\Events\3770_SHARE
";

    #[test]
    fn the_driver_is_found_by_the_scanners_usb_id() {
        let hp = UsbId {
            vendor: 0x03f0,
            product: 0x2505,
        };
        assert_eq!(
            instances(CLASS_LISTING, hp),
            [
                r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}\0000"
            ]
        );
        // Another maker's, in capitals and with a revision.
        let epson = UsbId {
            vendor: 0x04b8,
            product: 0x0119,
        };
        assert_eq!(instances(CLASS_LISTING, epson).len(), 1);
        assert!(instances(CLASS_LISTING, epson)[0].ends_with(r"\0001"));
        // A scanner without a driver of its maker: nothing.
        let other = UsbId {
            vendor: 0x03f0,
            product: 0x2405,
        };
        assert!(instances(CLASS_LISTING, other).is_empty());
    }

    #[test]
    fn every_event_of_the_driver() {
        let keys = events(EVENTS_LISTING);
        assert_eq!(keys.len(), 3);
        assert!(keys.iter().all(|k| k.starts_with(r"HKLM\SYSTEM")));
        assert!(keys[1].ends_with(r"\Events\3770_SCAN"));
    }
}
