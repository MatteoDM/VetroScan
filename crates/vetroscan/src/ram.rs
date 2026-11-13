// SPDX-License-Identifier: GPL-2.0-or-later
//! How much memory the computer has, for the limit on a scan's size.
//!
//! Read once, on a thread of its own started with the app ([`start`]): `/proc/meminfo` on
//! Linux, `sysctl -n hw.memsize` on macOS, PowerShell on Windows (the workspace forbids
//! `unsafe`, so no system call of our own). Until it is known, or where it cannot be read,
//! [`total`] answers `None` and no scan is refused.

use std::sync::OnceLock;

static TOTAL: OnceLock<Option<u64>> = OnceLock::new();

/// Starts reading the computer's memory, once.
pub fn start() {
    if cfg!(test) {
        return;
    }
    std::thread::spawn(|| {
        let _ = TOTAL.set(read());
    });
}

/// The computer's memory in bytes, once read; in tests, what the test set
/// ([`crate::testing::set_ram`]).
pub fn total() -> Option<u64> {
    if cfg!(test) {
        return crate::testing::ram();
    }
    TOTAL.get().copied().flatten()
}

#[cfg(target_os = "linux")]
fn read() -> Option<u64> {
    meminfo_total(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(target_os = "macos")]
fn read() -> Option<u64> {
    let out = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    number(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(windows)]
fn read() -> Option<u64> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            // not translated: a command
            "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    number(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn read() -> Option<u64> {
    None
}

/// `MemTotal` of `/proc/meminfo`, in bytes.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn meminfo_total(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kb.checked_mul(1024)
}

/// A number of bytes printed alone on a line (`sysctl`, PowerShell).
#[cfg_attr(not(any(target_os = "macos", windows, test)), allow(dead_code))]
fn number(text: &str) -> Option<u64> {
    text.trim().parse().ok().filter(|&n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_is_read_from_what_each_system_prints() {
        // This laptop's /proc/meminfo, 13 November 2026 (first lines).
        let meminfo = "MemTotal:        7956276 kB\nMemFree:         1023456 kB\n";
        assert_eq!(meminfo_total(meminfo), Some(7_956_276 * 1024));
        assert_eq!(meminfo_total("MemFree: 1 kB\n"), None);
        assert_eq!(number("17179869184\n"), Some(17_179_869_184));
        assert_eq!(number("\r\n8589934592\r\n"), Some(8_589_934_592));
        assert_eq!(number(""), None);
        assert_eq!(number("Get-CimInstance: not found"), None);
    }
}
