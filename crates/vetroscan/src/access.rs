// SPDX-License-Identifier: GPL-2.0-or-later
//! Letting the user open the scanner, on Linux.
//!
//! Linux gives USB devices to root only. A udev rule gives a scanner to
//! whoever sits at the computer; the packages (deb, rpm, pacman, AUR)
//! install one per scanner from `packaging/linux`, but an AppImage or a
//! build from source cannot. So when a scanner the app knows is there but
//! cannot be opened, the app offers to install the rule itself: one
//! `pkexec` (the desktop's password dialog) writes [`RULE_PATH_SHOWN`], a
//! line for each scanner the drivers know (their `Capabilities::usb`),
//! reloads the rules and applies them to the scanners already plugged in,
//! which then open without unplugging them.

use scanner_api::UsbId;

/// Where the rule goes.
#[cfg(target_os = "linux")]
const RULE_PATH: &str = "/etc/udev/rules.d/70-vetroscan.rules";

/// The same, for the interface (the file name only).
pub const RULE_PATH_SHOWN: &str = "70-vetroscan.rules";

/// The line that gives one scanner to the user at the seat, as the
/// packages' rules write it.
fn rule_line(usb: UsbId) -> String {
    // not translated: a udev rule
    format!(
        "SUBSYSTEM==\"usb\", ATTR{{idVendor}}==\"{:04x}\", ATTR{{idProduct}}==\"{:04x}\", \
         MODE=\"0660\", TAG+=\"uaccess\"",
        usb.vendor, usb.product
    )
}

/// The whole rule for these scanners, ending with its own newline.
fn rule(ids: &[UsbId]) -> String {
    let mut text = String::from(
        // not translated: a udev rule's comment
        "# VetroScan: lets the user logged in at the seat open these scanners without root.\n",
    );
    for &usb in ids {
        text.push_str(&rule_line(usb));
        text.push('\n');
    }
    text
}

/// Whether one of these scanners is plugged in but this user may not open
/// it.
#[cfg(target_os = "linux")]
pub fn denied(ids: &[UsbId]) -> bool {
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
        let (vendor, product) = (read(&dir, "idVendor"), read(&dir, "idProduct"));
        let known = ids.iter().any(|u| {
            vendor == format!("{:04x}", u.vendor) && product == format!("{:04x}", u.product)
        });
        if !known {
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
pub fn denied(_ids: &[UsbId]) -> bool {
    false
}

/// Installs the rule for these scanners and applies it to those plugged
/// in, asking the administrator password once.
#[cfg(target_os = "linux")]
pub fn grant(ids: &[UsbId]) -> Result<(), String> {
    let triggers: String = ids
        .iter()
        .map(|u| {
            format!(
                // not translated: a command
                " && udevadm trigger --action=add --subsystem-match=usb \
                 --attr-match=idVendor={:04x} --attr-match=idProduct={:04x}",
                u.vendor, u.product
            )
        })
        .collect();
    // The rule travels as an argument, not through a file another user
    // could swap.
    let script = format!(
        // not translated: a command
        "printf '%s' \"$1\" > {RULE_PATH} && chmod 644 {RULE_PATH} \
         && udevadm control --reload{triggers} \
         && udevadm settle --timeout=5"
    );
    let status = std::process::Command::new("pkexec")
        .args(["sh", "-c", &script, "sh", &rule(ids)])
        .status()
        .map_err(|e| crate::i18n::trf("pkexec non si avvia ({e})", &[("e", &e)]))?;
    match status.code() {
        Some(0) => Ok(()),
        // pkexec: the dialog was dismissed, or the password refused.
        Some(126 | 127) => Err(crate::i18n::tr("autorizzazione non concessa").into()),
        _ => Err(crate::i18n::trf(
            "installazione della regola non riuscita ({status})",
            &[("status", &status)],
        )),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn grant(_ids: &[UsbId]) -> Result<(), String> {
    Err(crate::i18n::tr("serve solo su Linux").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HP3770: &str = include_str!("../../../packaging/linux/70-hp3770.rules");
    const EPSON4490: &str = include_str!("../../../packaging/linux/70-epson-4490.rules");

    #[test]
    fn each_line_is_the_one_its_package_installs() {
        // The drivers' USB ids, so that the rule follows them.
        let hp = hp3770_core::rts8822::capabilities(&hp3770_core::profile::PROFILES[0])
            .usb
            .unwrap();
        let epson = epson_gt_x750::capabilities().usb.unwrap();
        let packaged = |file: &str| {
            file.lines()
                .find(|l| l.starts_with("SUBSYSTEM"))
                .unwrap()
                .to_owned()
        };
        assert_eq!(rule_line(hp), packaged(HP3770));
        assert_eq!(rule_line(epson), packaged(EPSON4490));
        let both = rule(&[hp, epson]);
        assert_eq!(
            both.lines().filter(|l| l.starts_with("SUBSYSTEM")).count(),
            2
        );
        // printf '%s' keeps it as is: no format directives to fear, and
        // the file ends with its own newline.
        assert!(!both.contains('%'));
        assert!(both.ends_with('\n'));
    }
}
