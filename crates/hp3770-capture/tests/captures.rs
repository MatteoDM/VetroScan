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

#[test]
fn execute_matches_the_hp_driver() {
    use hp3770_capture::replay::Replay;
    use hp3770_core::transport::Chip;

    // Cut from 04_colore_300dpi_A4.pcap (2.480-2.623 s): the first
    // RTS_Execute of a 300 dpi scan.
    let mut chip = Chip::new(Replay::from_trace(&fixture("04_execute.pcap")));
    chip.execute().unwrap();
    chip.transport_mut().finish().unwrap();
}

#[test]
fn executions_of_a_full_scan() {
    let Some(trace) = full_capture("04_colore_300dpi_A4.pcap") else {
        return;
    };
    let executions = trace.executions();
    assert_eq!(executions.len(), 10);
    for ex in &executions {
        let block = ex.bytes().expect("the HP driver writes the whole block");
        assert_eq!(block[0] & 0x80, 0x80);
    }
}

#[test]
fn init_usb_data_matches_the_hp_driver() {
    use hp3770_capture::replay::Replay;
    use hp3770_core::profile::HP3770;
    use hp3770_core::transport::Chip;

    // 00_collegamento.pcap, after a button press: Read_FE3E, then the
    // register traffic of Init_USBData up to Motor_Release (45.864-45.982 s).
    let trace = fixture("00_collegamento.pcap");
    let mut chip = Chip::new(Replay::window(&trace, 45.863, 45.983));
    chip.read_fe3e().unwrap();
    let regs = chip.init_usb_data(&HP3770).unwrap();
    chip.transport_mut().finish().unwrap();
    assert_eq!(regs.get(0x000), 0x05);
    assert_eq!(regs.get(0x708), 0x08);
    assert_eq!(regs.get(0x154), 0xb0);
}

#[test]
fn init_regs_match_the_hp_driver() {
    use hp3770_capture::replay::Replay;
    use hp3770_core::profile::HP3770;
    use hp3770_core::transport::Chip;

    // Our initial block, computed from the block read in 00_collegamento.
    let trace = fixture("00_collegamento.pcap");
    let mut chip = Chip::new(Replay::window(&trace, 45.863, 45.983));
    chip.read_fe3e().unwrap();
    let ours = chip.init_usb_data(&HP3770).unwrap();

    // The HP driver's initial block, written at the start of every scan
    // (cut from 04_colore_300dpi_A4.pcap, 2.130-2.2155 s; the same 1818
    // bytes in all scan captures 01-11).
    let hp = fixture("04_init_regs.pcap")
        .events
        .into_iter()
        .find_map(|e| match e {
            Event::Control {
                command:
                    Command::WriteRegisters {
                        address: 0xe800,
                        data,
                    },
                ..
            } if data.len() == 0x71a => Some(data),
            _ => None,
        })
        .expect("block write");

    let differences: Vec<u16> = (0xe800u16..)
        .zip(ours.0.iter().zip(&hp))
        .filter(|(_, (a, b))| a != b)
        .map(|(address, _)| address)
        .collect();
    // Every other register (1794 of 1818) is the same. The rest are not set
    // by the start-up code:
    let expected = [
        // Unknown: bit 6 of E813, and E969 (ED in the chip, CC in the HP
        // block and in every execution).
        0xe813, 0xe969, //
        // Scan geometry, resolution ratio, dummy lines, motor options and
        // curve steps. hp3900 writes them into init_regs when it prepares a
        // scan (SetScanParams and the motor code): the HP block carries the
        // values of an earlier scan, and every scan writes its own.
        0xe8b0, 0xe8b1, 0xe8b2, 0xe8b3, 0xe8c0, 0xe8d2, 0xe8d3, 0xe8d6, 0xe8dd, 0xe8e0, 0xe8e4,
        0xe8e5, 0xe8e7, 0xe8ea, 0xe8eb, 0xe8f0, 0xe8f1, 0xe9cf, //
        // Lamp timer: set later by Lamp_Status_Timer_Set and the lamp code.
        0xe946, 0xe947,
    ];
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    let report: Vec<String> = differences
        .iter()
        .map(|&a| {
            let i = usize::from(a - 0xe800);
            format!("{a:04X}: ours {:02X}, HP {:02X}", ours.0[i], hp[i])
        })
        .collect();
    assert_eq!(differences, expected, "\n{}", report.join("\n"));
}
