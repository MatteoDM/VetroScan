// SPDX-License-Identifier: GPL-2.0-or-later
//! Embeds the scanner's firmware in the program.
//!
//! By default the copy in the repository, `firmware/esfw54.bin` (the one of
//! Epson's Windows driver; the repository is private since 10 November
//! 2026, and every build carries it, CI's included). `VETROSCAN_ESFW54`
//! names another copy on the builder's disk instead (the AVASYS one, for a
//! try). Only the two known copies are accepted (`src/known_firmware.rs`,
//! by SHA-256); the file is copied unchanged into `OUT_DIR` and the crate
//! includes it. With neither (a checkout without the file) nothing is
//! embedded and the backend looks for the file at run time.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

mod known {
    #![allow(dead_code)]
    include!("src/known_firmware.rs");
}

const VAR: &str = "VETROSCAN_ESFW54";
/// The repository's copy, relative to the crate.
const IN_REPOSITORY: &str = "firmware/esfw54.bin";

fn main() {
    println!("cargo::rerun-if-env-changed={VAR}");
    println!("cargo::rerun-if-changed=src/known_firmware.rs");
    println!("cargo::rustc-check-cfg=cfg(esfw54_embedded)");

    let chosen = std::env::var_os(VAR).filter(|v| !v.is_empty());
    let path = match &chosen {
        Some(path) => PathBuf::from(path),
        None => {
            let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
            let path = Path::new(&manifest).join(IN_REPOSITORY);
            println!("cargo::rerun-if-changed={}", path.display());
            if !path.is_file() {
                return;
            }
            path
        }
    };
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
    // Said only when the builder chose another copy: the repository's is
    // the usual one.
    if chosen.is_some() {
        println!(
            "cargo::warning=embedding esfw54.bin ({build}) from {}",
            path.display()
        );
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}
