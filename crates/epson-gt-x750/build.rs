// SPDX-License-Identifier: GPL-2.0-or-later
//! Embeds the scanner's firmware in the program when the builder has it.
//!
//! `VETROSCAN_ESFW54` names a copy of `esfw54.bin` on the builder's disk
//! (on Windows with Epson's driver: `C:\Windows\System32\esfw54.bin`). Only
//! the two known copies are accepted (`src/known_firmware.rs`, by SHA-256);
//! the file is copied unchanged into `OUT_DIR` and the crate includes it.
//! Without the variable nothing is embedded and the backend looks for the
//! file at run time: the repository never holds the firmware, and public
//! builds (CI) are made without it.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

mod known {
    #![allow(dead_code)]
    include!("src/known_firmware.rs");
}

const VAR: &str = "VETROSCAN_ESFW54";

fn main() {
    println!("cargo::rerun-if-env-changed={VAR}");
    println!("cargo::rerun-if-changed=src/known_firmware.rs");
    println!("cargo::rustc-check-cfg=cfg(esfw54_embedded)");

    let Some(path) = std::env::var_os(VAR).filter(|v| !v.is_empty()) else {
        return;
    };
    let path = PathBuf::from(path);
    println!("cargo::rerun-if-changed={}", path.display());

    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{VAR}: cannot read {}: {e}", path.display()));
    let sha = hex(&Sha256::digest(&bytes));
    let build = if sha == known::WINDOWS_DRIVER_SHA256 {
        "the Windows driver's copy"
    } else if sha == known::AVASYS_SHA256 {
        "the AVASYS copy"
    } else {
        panic!(
            "{VAR}: {} is not a known esfw54.bin (SHA-256 {sha}, {} bytes; \
             expected {} or {})",
            path.display(),
            bytes.len(),
            known::WINDOWS_DRIVER_SHA256,
            known::AVASYS_SHA256
        );
    };
    assert_eq!(bytes.len(), known::FILE_LEN, "{VAR}: unexpected size");

    let out = Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("esfw54.bin");
    std::fs::write(&out, &bytes).unwrap_or_else(|e| panic!("writing {}: {e}", out.display()));
    println!("cargo::rustc-cfg=esfw54_embedded");
    println!(
        "cargo::warning=embedding esfw54.bin ({build}) from {}",
        path.display()
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}
