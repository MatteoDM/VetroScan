// SPDX-License-Identifier: GPL-2.0-or-later
//! A scan of ours in progress, marked by a file in the temporary folder for as long as it
//! lasts: how opening the scanner tells that a scan of ours was left half-way by a process now
//! gone (the application closed or killed while reading, captures 44-54), and only then asks
//! the scanner its state before Windows' STI lock ([`crate::Gt4490::start`]).
//!
//! On Windows the file is held open without sharing while the scan lasts: Windows closes it
//! when the process ends, however it ends, so a mark that can be opened again is a scan of a
//! process gone, and one that cannot is a live scan of another process of ours. Elsewhere a
//! file is never held so, and a mark is taken as left by a process gone; there no lock is taken
//! ([`crate::Transport::locks`]) and the mark is not asked.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// The mark of this computer's user: one Epson 4490 at a time.
pub fn default_path() -> PathBuf {
    std::env::temp_dir().join("epson4490-scan-in-progress")
}

/// A scan in progress, marked at `path` until dropped. Failing to make the mark costs only the
/// recovery of a scan left half-way by a process killed (then by switching the scanner off and
/// on), never a scan: errors are ignored.
#[derive(Debug)]
pub struct ScanMark {
    file: Option<File>,
    path: PathBuf,
}

impl ScanMark {
    /// Marks a scan beginning at `path`.
    pub fn begin(path: &Path) -> Self {
        Self {
            file: exclusive(path, true),
            path: path.to_path_buf(),
        }
    }
}

impl Drop for ScanMark {
    fn drop(&mut self) {
        self.file = None;
        clear(&self.path);
    }
}

/// Whether the mark at `path` is a scan of ours left by a process gone: it is there, and (on
/// Windows) no live process holds it.
pub fn left_by_a_process_gone(path: &Path) -> bool {
    path.exists() && (cfg!(not(windows)) || exclusive(path, false).is_some())
}

/// Removes the mark at `path`, once the scan it marks is over or was stopped.
pub fn clear(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// `path` opened for writing, shared with nobody on Windows; `create` makes it if missing.
fn exclusive(path: &Path, create: bool) -> Option<File> {
    let mut o = OpenOptions::new();
    o.write(true).create(create);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.share_mode(0);
    }
    o.open(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("epson4490-mark-test-{name}-{}", std::process::id()))
    }

    #[test]
    fn a_scan_marks_itself_until_it_ends() {
        let p = path("ends");
        clear(&p);
        assert!(!left_by_a_process_gone(&p));
        let mark = ScanMark::begin(&p);
        assert!(p.exists());
        // Held by a live scan: on Windows not taken for one left half-way.
        assert_eq!(left_by_a_process_gone(&p), cfg!(not(windows)));
        drop(mark);
        assert!(!p.exists());
    }

    #[test]
    fn a_mark_nobody_holds_is_a_scan_left_by_a_process_gone() {
        let p = path("gone");
        std::fs::write(&p, b"").unwrap();
        assert!(left_by_a_process_gone(&p));
        // Asking does not remove it: only `clear` does.
        assert!(left_by_a_process_gone(&p));
        clear(&p);
        assert!(!left_by_a_process_gone(&p));
    }
}
