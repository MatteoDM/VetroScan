// SPDX-License-Identifier: GPL-2.0-or-later
//! Decodes real USB captures of the HP driver.
//!
//! The small captures in `tests/fixtures` are always tested. The full scan
//! captures (up to ~27 MB each) are not in git: set `HP3770_CAPTURES` to the
//! folder that holds them to test those too.

use hp3770_capture::trace::{Event, Trace};
use hp3770_core::device::{Button, PRODUCT_ID, VENDOR_ID};
use hp3770_core::protocol::Command;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> Trace {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    Trace::from_path(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn full_capture(name: &str) -> Option<Trace> {
    let dir = PathBuf::from(std::env::var_os("HP3770_CAPTURES")?);
    let path = dir.join(name);
    Some(Trace::from_path(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

#[test]
fn device_descriptor_identifies_the_3770() {
    let trace = fixture("00_collegamento.pcap");
    let descriptor = trace
        .events
        .iter()
        .find_map(|e| match e {
            Event::Control {
                command: Command::Standard(s),
                response,
                ..
            } if s.request == 0x06 && s.value == 0x0100 && response.len() >= 18 => {
                Some(response.clone())
            }
            _ => None,
        })
        .expect("GET_DESCRIPTOR(device) in the capture");
    assert_eq!(
        u16::from_le_bytes([descriptor[8], descriptor[9]]),
        VENDOR_ID
    );
    assert_eq!(
        u16::from_le_bytes([descriptor[10], descriptor[11]]),
        PRODUCT_ID
    );
}

#[test]
fn every_vendor_request_is_understood() {
    for name in [
        "00_collegamento.pcap",
        "12_pulsante_centrale_fax.pcap",
        "13_pulsante_sinistro_scan.pcap",
    ] {
        let s = fixture(name).summary();
        assert_eq!(s.unknown_requests, 0, "{name}");
        assert!(s.register_reads > 0, "{name}");
    }
}

#[test]
fn buttons_match_the_front_panel() {
    assert_eq!(
        fixture("13_pulsante_sinistro_scan.pcap").summary().buttons,
        [Button::Scan]
    );
    assert_eq!(
        fixture("12_pulsante_centrale_fax.pcap").summary().buttons,
        [Button::Copy]
    );
    assert_eq!(
        fixture("15_pulsante_destro_mondo.pcap").summary().buttons,
        [Button::InstantShare]
    );
}

#[test]
fn button_press_triggers_full_register_dump() {
    // After a button event the HP driver reads the whole E800 register block.
    let trace = fixture("13_pulsante_sinistro_scan.pcap");
    let dump = trace.events.iter().find_map(|e| match e {
        Event::Control {
            command:
                Command::ReadRegisters {
                    address: 0xe800,
                    length,
                },
            response,
            ..
        } if *length > 2 => Some((*length, response.len())),
        _ => None,
    });
    assert_eq!(dump, Some((0x0719, 0x0719)));
}

#[test]
fn full_scan_moves_the_whole_image() {
    // 300 dpi A4 colour: the driver delivered 2480 x 3508 x 3 = 26,099,560
    // bytes; the scanner sends at least that much over bulk IN.
    let Some(trace) = full_capture("04_colore_300dpi_A4.pcap") else {
        return;
    };
    let s = trace.summary();
    assert_eq!(s.unknown_requests, 0);
    assert!(s.bulk_in_bytes >= 26_099_560, "bulk in {}", s.bulk_in_bytes);
    assert!(s.bulk_out_transfers > 0);
}

#[test]
fn gray_and_lineart_transfer_colour_data() {
    // The scanner always sends RGB: grey and black/white move as much data
    // as colour, the conversion is done on the host.
    let (Some(colour), Some(grey), Some(bw)) = (
        full_capture("04_colore_300dpi_A4.pcap"),
        full_capture("07_grigio_300dpi_A4.pcap"),
        full_capture("08_bn_300dpi_A4.pcap"),
    ) else {
        return;
    };
    let c = colour.summary().bulk_in_bytes as f64;
    for (name, t) in [("grey", grey), ("bw", bw)] {
        let b = t.summary().bulk_in_bytes as f64;
        assert!((b / c - 1.0).abs() < 0.02, "{name}: {b} vs colour {c}");
    }
}
