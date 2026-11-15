// SPDX-License-Identifier: GPL-2.0-or-later
//! What the tests put in place of the outside world: the folder of the
//! settings and of the scans saved, the "Salva" dialog, and what the app
//! would do outside its window (notifications, printing, opening a
//! folder), which tests read back instead.
//!
//! Everything is per thread: each test runs on its own, and tests run side
//! by side.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    static EFFECTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static SAVE_CANCELLED: RefCell<bool> = const { RefCell::new(false) };
    static RAM: RefCell<Option<u64>> = const { RefCell::new(None) };
    static MONITOR: RefCell<Option<crate::screen::Monitor>> = const { RefCell::new(None) };
    #[cfg(test)]
    static DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Notes something the app did outside its window (tests only).
pub fn record(what: String) {
    if cfg!(test) {
        EFFECTS.with(|e| e.borrow_mut().push(what));
    }
}

/// The answer of the "Salva" dialog in tests: the file it suggests, in
/// the folder it opens on, or nothing when the test cancels it.
pub fn save_answer(folder: &Path, file: &str) -> Option<PathBuf> {
    (!SAVE_CANCELLED.with(|c| *c.borrow())).then(|| folder.join(file))
}

/// What the app did outside its window since the last call.
#[cfg(test)]
pub fn take_effects() -> Vec<String> {
    EFFECTS.with(|e| std::mem::take(&mut *e.borrow_mut()))
}

/// Makes the "Salva" dialog be cancelled (or not) from now on.
#[cfg(test)]
pub fn cancel_save_dialog(cancelled: bool) {
    SAVE_CANCELLED.with(|c| *c.borrow_mut() = cancelled);
}

/// A folder of this test's own, empty when first asked for.
#[cfg(test)]
pub fn dir() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    DIR.with(|d| {
        d.borrow_mut()
            .get_or_insert_with(|| {
                let dir = std::env::temp_dir().join(format!(
                    "vetroscan-test-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::SeqCst)
                ));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).expect("test folder");
                dir
            })
            .clone()
    })
}

/// The computer's memory a test made up ([`set_ram`]): none, so no scan is refused, unless set.
pub fn ram() -> Option<u64> {
    RAM.with(|r| *r.borrow())
}

/// Makes the app see a computer with `bytes` of memory (tests only).
#[cfg(test)]
pub fn set_ram(bytes: Option<u64>) {
    RAM.with(|r| *r.borrow_mut() = bytes);
}

/// The monitor a test made up ([`set_monitor`]): none, unless set.
#[cfg(test)]
pub fn monitor() -> Result<crate::screen::Monitor, String> {
    MONITOR
        .with(|m| m.borrow().clone())
        .ok_or_else(|| "no monitor in the tests".to_string()) // not translated: tests only
}

/// Makes the app find a monitor with this profile (tests only).
#[cfg(test)]
pub fn set_monitor(monitor: Option<crate::screen::Monitor>) {
    MONITOR.with(|m| *m.borrow_mut() = monitor);
}

/// Removes this test's folder (at the end of a test).
#[cfg(test)]
pub fn clean_up() {
    if let Some(dir) = DIR.with(|d| d.borrow_mut().take()) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
