// SPDX-License-Identifier: GPL-2.0-or-later
//! Letting the user open the scanner, on Linux.
//!
//! Linux gives USB devices to root only. The udev rule in
//! `packaging/linux` gives the scanner to whoever sits at the computer; a
//! package (AUR) installs it, but an AppImage or a build from source cannot.
//! So when the scanner is there but cannot be opened, the app offers to
//! install the rule itself: one `pkexec` (the desktop's password dialog)
//! writes it, reloads the rules and applies them to the scanner already
//! plugged in, which then opens without unplugging it.

/// The rule, the same file the packages install.
#[cfg(target_os = "linux")]
const RULE: &str = include_str!("../../../packaging/linux/70-hp3770.rules");

/// Where the rule goes.
#[cfg(target_os = "linux")]
const RULE_PATH: &str = "/etc/udev/rules.d/70-hp3770.rules";

/// The same, for the interface (the file name only).
pub const RULE_PATH_SHOWN: &str = "70-hp3770.rules";

/// Whether a 3770 is plugged in but this user may not open it.
#[cfg(target_os = "linux")]
pub fn denied() -> bool {
    use std::fs;
    let read = |dir: &std::path::Path, name: &str| {
        fs::read_to_string(dir.join(name))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    let Ok(devices) = fs::read_dir("/sys/bus/usb/devices") else {
        return false;
    };
    devices.flatten().any(|entry| {
        let dir = entry.path();
        if read(&dir, "idVendor") != "03f0" || read(&dir, "idProduct") != "2505" {
            return false;
        }
        let (Ok(bus), Ok(dev)) = (
            read(&dir, "busnum").parse::<u16>(),
            read(&dir, "devnum").parse::<u16>(),
        ) else {
            return false;
        };
        let node = format!("/dev/bus/usb/{bus:03}/{dev:03}");
        matches!(
            fs::OpenOptions::new().read(true).write(true).open(node),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied
        )
    })
}

#[cfg(not(target_os = "linux"))]
pub fn denied() -> bool {
    false
}

/// Installs the rule and applies it to the scanner plugged in, asking the
/// administrator password once.
#[cfg(target_os = "linux")]
pub fn grant() -> Result<(), String> {
    // The rule travels as an argument, not through a file another user
    // could swap.
    let script = format!(
        "printf '%s' \"$1\" > {RULE_PATH} && chmod 644 {RULE_PATH} \
         && udevadm control --reload \
         && udevadm trigger --action=add --subsystem-match=usb \
            --attr-match=idVendor=03f0 --attr-match=idProduct=2505 \
         && udevadm settle --timeout=5"
    );
    let status = std::process::Command::new("pkexec")
        .args(["sh", "-c", &script, "sh", RULE])
        .status()
        .map_err(|e| format!("pkexec non si avvia ({e})"))?;
    match status.code() {
        Some(0) => Ok(()),
        // pkexec: the dialog was dismissed, or the password refused.
        Some(126 | 127) => Err("autorizzazione non concessa".into()),
        _ => Err(format!(
            "installazione della regola non riuscita ({status})"
        )),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn grant() -> Result<(), String> {
    Err("serve solo su Linux".into())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn rule_is_the_packaged_one() {
        assert!(RULE.contains(r#"ATTR{idVendor}=="03f0", ATTR{idProduct}=="2505""#));
        assert!(RULE.contains(r#"TAG+="uaccess""#));
        // printf '%s' keeps it as is: no format directives to fear, and
        // the file ends with its own newline.
        assert!(RULE.ends_with('\n'));
    }
}
