// SPDX-License-Identifier: GPL-2.0-or-later
//! Tests on a real HP Scanjet 3770, through the `hp3770` command. They
//! move the carriage and switch the lamp on, so they only run when asked:
//!
//! ```text
//! cargo test -p hp3770-cli --test hardware -- --ignored --test-threads 1
//! ```
//!
//! The scanner must be plugged in, with nothing using it. Each scan is a
//! small square, so the whole run takes a few minutes.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `hp3770` with `args`; its output, or a panic with its error.
fn hp3770(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_hp3770"))
        .args(args)
        .output()
        .expect("hp3770 runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "hp3770 {args:?}:\n{text}");
    text
}

/// A folder for this run's files.
fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hp3770-hardware-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Width, height, colour type and bit depth of a PNG.
fn png_info(path: &Path) -> (u32, u32, png::ColorType, png::BitDepth) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let reader = decoder.read_info().unwrap();
    let info = reader.info();
    (info.width, info.height, info.color_type, info.bit_depth)
}

/// Pixels of `mm` millimetres at `dpi`.
fn pixels(mm: f64, dpi: u32) -> u32 {
    (mm / 25.4 * f64::from(dpi)).round() as u32
}

/// A square of `MM` millimetres, where the originals usually are.
const MM: f64 = 10.0;

fn scan(name: &str, dpi: u32, extra: &[&str]) -> PathBuf {
    let path = folder().join(name);
    let dpi_text = dpi.to_string();
    let size = MM.to_string();
    let mut args = vec![
        "scan", "--dpi", &dpi_text, "--left", "70", "--top", "150", "--width", &size, "--height",
        &size,
    ];
    args.extend_from_slice(extra);
    let path_text = path.to_str().unwrap().to_string();
    args.push(&path_text);
    let out = hp3770(&args);
    assert!(out.contains("saved"), "{out}");
    path
}

fn assert_size(path: &Path, dpi: u32) {
    let (w, h, ..) = png_info(path);
    let want = pixels(MM, dpi);
    assert!(
        w.abs_diff(want) <= 2 && h.abs_diff(want) <= 2,
        "{}: {w} × {h}, expected about {want} × {want} at {dpi} dpi",
        path.display()
    );
}

#[test]
#[ignore = "needs the scanner"]
fn the_scanner_is_found_and_answers() {
    let list = hp3770(&["list"]);
    assert!(
        list.contains("usbscan.sys") || list.contains("nusb"),
        "{list}"
    );
    let status = hp3770(&["status"]);
    assert!(status.contains("initialised        yes"), "{status}");
    assert!(status.contains("operation running  no"), "{status}");
}

#[test]
#[ignore = "needs the scanner"]
fn colour_at_every_resolution() {
    for dpi in [200, 300, 600, 1200, 2400] {
        let path = scan(&format!("colour-{dpi}.png"), dpi, &[]);
        assert_size(&path, dpi);
        let (_, _, colour, depth) = png_info(&path);
        assert_eq!(colour, png::ColorType::Rgb);
        assert_eq!(depth, png::BitDepth::Eight);
    }
}

#[test]
#[ignore = "needs the scanner"]
fn grey_and_black_and_white() {
    let grey = scan("grey.png", 300, &["--mode", "grey"]);
    assert_size(&grey, 300);
    assert_eq!(png_info(&grey).2, png::ColorType::Grayscale);
    let lineart = scan("lineart.png", 300, &["--mode", "lineart"]);
    assert_size(&lineart, 300);
    assert_eq!(png_info(&lineart).2, png::ColorType::Grayscale);
}

#[test]
#[ignore = "needs the scanner"]
fn sixteen_bits() {
    let path = scan("deep.png", 600, &["--depth", "16"]);
    assert_size(&path, 600);
    let deep = folder().join("deep-16bit.png");
    let (_, _, colour, depth) = png_info(&deep);
    assert_eq!(colour, png::ColorType::Rgb);
    assert_eq!(depth, png::BitDepth::Sixteen);
}

#[test]
#[ignore = "needs the scanner"]
fn the_carriage_is_home_after_a_scan() {
    scan("home.png", 200, &[]);
    let status = hp3770(&["status"]);
    assert!(status.contains("carriage at home   yes"), "{status}");
    assert!(status.contains("operation running  no"), "{status}");
}
