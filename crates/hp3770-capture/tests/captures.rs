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

/// Runs our start-up against the HP driver's after a button press in
/// 00_collegamento.pcap (45.857-48.808 s): RTS_USBType (FE11, twice),
/// IsScannerLinked, Set_E950_Mode, then the lamp switched on and the buttons
/// read. The HP driver also reads and writes the EEPROM (0x7B, 0x7C) in the
/// middle: those requests are taken out of the capture, and must be the
/// only difference.
fn link_on_capture_00() -> hp3770_core::init::Link {
    use hp3770_capture::replay::Replay;
    use hp3770_core::profile::HP3770;
    use hp3770_core::status::Lamp;
    use hp3770_core::transport::Chip;

    let trace = fixture("00_collegamento.pcap");
    let events = trace.events.into_iter().filter(|e| {
        (45.857..48.808).contains(&e.time())
            && !matches!(
                e,
                Event::Control {
                    command: Command::ReadEeprom { .. } | Command::WriteEeprom { .. },
                    ..
                }
            )
    });
    let mut chip = Chip::new(Replay::new(events));
    chip.read_byte(0xfe11).unwrap();
    chip.read_byte(0xfe11).unwrap();
    let mut link = chip.link(&HP3770).unwrap();
    // The HP driver then checks the lamp twice and switches it on.
    assert_eq!(chip.lamp_status().unwrap(), Lamp::Flatbed);
    assert_eq!(chip.lamp_status().unwrap(), Lamp::Flatbed);
    chip.lamp_flatbed(&mut link.registers, true).unwrap();
    chip.buttons_held().unwrap();
    chip.transport_mut().finish().unwrap();
    link
}

#[test]
fn link_matches_the_hp_driver() {
    let link = link_on_capture_00();
    assert_eq!(link.link_word, 0x1024);
    assert!(link.at_home);
    // Lamp timer: enabled, 13 minutes.
    assert_eq!(link.registers.get(0x146), 0xd9);
    assert_eq!(link.registers.get(0x147), 0x22);
}

#[test]
fn init_regs_match_the_hp_driver() {
    // Our initial block, computed from the block read in 00_collegamento.
    let ours = link_on_capture_00().registers;

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
    // Every other register (1795 of 1818) is the same. The rest are not set
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
        // Bit 4 of E946, the lamp timer: on after the start-up, off in the
        // HP block, as left by the lamp code of an earlier scan.
        0xe946,
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

#[test]
fn lamp_off_and_on_match_the_hp_driver() {
    use hp3770_capture::replay::Replay;
    use hp3770_core::init::Registers;
    use hp3770_core::transport::Chip;

    // Cut from 04_colore_300dpi_A4.pcap: during the calibration the HP
    // driver switches the flatbed lamp off (4.703 s) and on again (5.253 s).
    let mut init = Registers::default();
    init.0[0x146] = 0xd9;
    let mut chip = Chip::new(Replay::from_trace(&fixture("04_lamp_off.pcap")));
    chip.lamp_flatbed(&mut init, false).unwrap();
    chip.transport_mut().finish().unwrap();
    assert_eq!(init.get(0x146), 0x99);

    let mut chip = Chip::new(Replay::from_trace(&fixture("04_lamp_on.pcap")));
    chip.lamp_flatbed(&mut init, true).unwrap();
    chip.transport_mut().finish().unwrap();
    assert_eq!(init.get(0x146), 0xd9);
    assert_eq!(init.get(0x155), 0x09);
}

/// The HP driver's initial block (cut from 04_colore_300dpi_A4.pcap,
/// 2.130-2.2155 s; the same 1818 bytes in all scan captures 01-11).
fn hp_init_regs() -> hp3770_core::init::Registers {
    let data = fixture("04_init_regs.pcap")
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
    let mut regs = hp3770_core::init::Registers::default();
    regs.0.copy_from_slice(&data);
    regs
}

#[test]
fn park_matches_the_hp_driver() {
    use hp3770_capture::replay::Replay;
    use hp3770_core::motor::park_registers;
    use hp3770_core::profile::HP3770;
    use hp3770_core::transport::Chip;

    // Cut from 11_annulla.pcap (19.0845-23.9905 s): after the cancelled
    // scan the HP driver brings the carriage home. Same table and same
    // block in 05 (16.02 s) and 06 (15.16 s), before the scan.
    //
    // The base block is the HP initial block with the three registers the
    // scan set-up leaves in it (park_registers). Every other byte of the
    // 1818 the HP driver writes must come from our motor code, and the
    // replay checks all of them, the 1728 bytes of steps included, the
    // failed read-back and its retry, and the 1540 status polls.
    let base = park_registers(&hp_init_regs());
    let mut chip = Chip::new(Replay::from_trace(&fixture("11_park.pcap")));
    assert!(chip.head_park_home(&base, &HP3770.park).unwrap());
    chip.transport_mut().finish().unwrap();
}

#[test]
fn park_before_high_resolution_scans() {
    use hp3770_capture::replay::Replay;
    use hp3770_core::motor::park_registers;
    use hp3770_core::profile::HP3770;
    use hp3770_core::transport::Chip;

    // At 600 and 1200 dpi the HP driver brings the carriage home after the
    // calibration, before the scan: same steps and same block as after a
    // cancel, but for the lamp bits of E954 (70 00 in capture 06).
    for (name, from, to, e954) in [
        (
            "05_colore_600dpi_5x5cm.pcap",
            15.9915,
            17.2685,
            [0xb0, 0x01],
        ),
        (
            "06_colore_1200dpi_2x2cm.pcap",
            15.1305,
            15.9105,
            [0x70, 0x00],
        ),
    ] {
        let Some(trace) = full_capture(name) else {
            return;
        };
        let mut base = park_registers(&hp_init_regs());
        base.0[0x154..0x156].copy_from_slice(&e954);
        let mut chip = Chip::new(Replay::window(&trace, from, to));
        assert!(chip.head_park_home(&base, &HP3770.park).unwrap(), "{name}");
        chip.transport_mut().finish().unwrap();
    }
}

/// Image reads of a capture: each `DMA enable read` on channel 8 and the
/// bulk IN data that follows it, as `(time, data)`.
fn image_reads(trace: &Trace) -> Vec<(f64, Vec<u8>)> {
    use hp3770_core::protocol::DmaDirection;
    let mut out = Vec::new();
    let mut want: Option<(f64, usize, Vec<u8>)> = None;
    for event in &trace.events {
        match event {
            Event::Control {
                time,
                command:
                    Command::DmaEnable {
                        direction: DmaDirection::Read,
                        channel: 8,
                        size,
                        ..
                    },
                ..
            } => want = Some((*time, *size as usize, Vec::new())),
            Event::BulkIn { data, .. } => {
                if let Some((time, size, buf)) = want.as_mut() {
                    buf.extend_from_slice(data);
                    if buf.len() >= *size {
                        out.push((*time, std::mem::take(buf)));
                        want = None;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Table writes of a capture: each `DMA enable write` and the bulk OUT data
/// that follows it, as `(time, channel, data)`.
fn table_writes(trace: &Trace) -> Vec<(f64, u16, Vec<u8>)> {
    use hp3770_core::protocol::DmaDirection;
    let mut out = Vec::new();
    let mut pending: Option<(f64, u16)> = None;
    for event in &trace.events {
        match event {
            Event::Control {
                time,
                command:
                    Command::DmaEnable {
                        direction: DmaDirection::Write,
                        channel,
                        ..
                    },
                ..
            } => pending = Some((*time, *channel)),
            Event::BulkOut { data, .. } => {
                if let Some((time, channel)) = pending.take() {
                    out.push((time, channel, data.clone()));
                }
            }
            _ => {}
        }
    }
    out
}

/// Replays the HP driver's calibration decisions on a capture: from the
/// images the scanner sent, our calibration code must give the offsets and
/// gains of the following reads and the shading tables the HP driver wrote.
fn check_calibration(trace: &Trace, name: &str, dpi: u16, colour: bool) {
    use hp3770_core::calibration::{
        black_level, dark_sum, gain_peak, gain_value, offset_too_dark, predict_offset,
        white_coefficients, Image, Pixels,
    };
    use hp3770_core::profile::HP3770;

    let cfg = &HP3770.calibration;
    let executions = trace.executions();
    let reads = image_reads(trace);
    let tables = table_writes(trace);
    // The register block each read ran with.
    let block = |time: f64| -> Vec<u8> {
        let e = executions
            .iter()
            .rev()
            .find(|e| e.time < time)
            .expect("execution before the read");
        e.block.iter().map(|b| b.expect("known register")).collect()
    };
    let width = |b: &[u8]| {
        let x0 = u16::from_le_bytes([b[0xb0], b[0xb1]]);
        let x1 = u16::from_le_bytes([b[0xb2], b[0xb3]]);
        usize::from(x1 - x0)
    };
    // Offsets as [channel][parity].
    let offsets = |b: &[u8]| -> [[u16; 2]; 3] {
        std::array::from_fn(|c| {
            std::array::from_fn(|p| {
                let r = 0x1a + 4 * c + 2 * p;
                u16::from(b[r]) | (u16::from(b[r + 1] & 0x80) << 1)
            })
        })
    };
    let gains = |b: &[u8]| -> [u8; 3] { std::array::from_fn(|c| b[0x14 + c] & 0x1f) };

    let even_odd = cfg.even_odd(dpi);
    let n = cfg.offset_pixels(dpi);
    let probes = 2 * cfg.offset_probe_rounds(dpi);
    for (_, data) in &reads[..probes] {
        assert_eq!(data.len(), 300, "{name}: probe size");
    }

    // 1. Offset prediction from the last pair of probes.
    let low = Image::new(&reads[probes - 2].1, 100);
    let high = Image::new(&reads[probes - 1].1, 100);
    let mut expected: [[u16; 2]; 3] = std::array::from_fn(|c| {
        let o = predict_offset(
            cfg,
            dark_sum(&low, c, n, Pixels::All),
            dark_sum(&high, c, n, Pixels::All),
        )
        .unwrap();
        [o, o]
    });

    // 2. Checks: raise the channels found too dark until none is.
    let mut next = probes;
    loop {
        let (time, data) = &reads[next];
        assert_eq!(data.len(), 3000, "{name}: check {next}");
        assert_eq!(
            offsets(&block(*time)),
            expected,
            "{name}: offsets of read {next}"
        );
        let img = Image::new(data, 100);
        let mut changed = false;
        for (c, channel) in expected.iter_mut().enumerate() {
            if even_odd {
                for (p, which) in [Pixels::Even, Pixels::Odd].into_iter().enumerate() {
                    if offset_too_dark(cfg, dark_sum(&img, c, n, which)) {
                        channel[p] += cfg.offset_raise(dpi);
                        changed = true;
                    }
                }
            } else if offset_too_dark(cfg, dark_sum(&img, c, n, Pixels::All)) {
                channel[0] += cfg.offset_raise(dpi);
                channel[1] += cfg.offset_raise(dpi);
                changed = true;
            }
        }
        next += 1;
        if !changed {
            break;
        }
    }

    // 3. Gain, from the next read; the black read runs with the result.
    let (gain_time, gain_data) = &reads[next];
    let gain_block = block(*gain_time);
    assert_eq!(
        offsets(&gain_block),
        expected,
        "{name}: offsets of the gain read"
    );
    let img = Image::new(gain_data, width(&gain_block));
    assert_eq!(img.lines(), usize::from(cfg.gain_lines), "{name}");
    let current = gains(&gain_block);
    let new_gains: [u8; 3] =
        std::array::from_fn(|c| gain_value(cfg, c, gain_peak(&img, c), current[c]));
    let (black_time, black_data) = &reads[next + 1];
    let black_block = block(*black_time);
    assert_eq!(black_block[0x146] & 0x40, 0, "{name}: lamp off for black");
    assert_eq!(gains(&black_block), new_gains, "{name}: gains");

    // 4. Black shading: one level per channel (and parity at 1200 dpi),
    // in the tables written after the black read.
    let img = Image::new(black_data, width(&black_block));
    assert_eq!(img.lines(), usize::from(cfg.black_lines), "{name}");
    for c in 0..3 {
        let (_, _, table) = tables
            .iter()
            .find(|(t, ch, _)| t > black_time && *ch == 0x10 + c as u16)
            .expect("black table");
        let levels = if even_odd {
            [
                black_level(cfg, &img, c, Pixels::Even),
                black_level(cfg, &img, c, Pixels::Odd),
            ]
        } else {
            let l = black_level(cfg, &img, c, Pixels::All);
            [l, l]
        };
        for (i, word) in table.chunks(2).enumerate() {
            let v = u16::from_le_bytes([word[0], word[1]]);
            assert_eq!(v, levels[i % 2], "{name}: black {c} pixel {i}");
        }
    }

    // 5. White shading: one multiplier per pixel, in the tables written
    // after the white read.
    if !colour {
        return;
    }
    let (white_time, white_data) = &reads[next + 2];
    let white_block = block(*white_time);
    let img = Image::new(white_data, width(&white_block));
    assert_eq!(img.lines(), usize::from(cfg.white_lines), "{name}");
    for c in 0..3 {
        let (_, _, table) = tables
            .iter()
            .find(|(t, ch, _)| t > white_time && *ch == 0x14 + c as u16)
            .expect("white table");
        let coefficients = white_coefficients(cfg, &img, c);
        assert_eq!(
            hp3770_core::calibration::table_bytes(&coefficients),
            *table,
            "{name}: white {c}"
        );
    }
}

#[test]
fn calibration_matches_the_hp_driver() {
    // Cut from 09_colore_300dpi_area_centro.pcap (2.0-6.95 s): initial
    // block, offset probes and check, gain, black and white shading.
    check_calibration(
        &fixture("09_calibrazione.pcap"),
        "09_calibrazione",
        300,
        true,
    );
}

#[test]
fn calibration_of_every_scan_capture() {
    // Every scan capture, with the resolution the HP driver calibrated at
    // (01 and 02 are 200 dpi, 06b is a 2400 dpi scan on the 1200 dpi
    // sensor).
    //
    // In gray and lineart (07, 08) the HP driver makes the chip send each
    // channel at a third of its level, through the white shading table, and
    // adds them up; we scan in colour and convert ourselves, so those white
    // tables are not ours to match.
    let captures = [
        ("01_anteprima.pcap", 200, true),
        ("02_colore_200dpi_A4.pcap", 200, true),
        ("04_colore_300dpi_A4.pcap", 300, true),
        ("05_colore_600dpi_5x5cm.pcap", 600, true),
        ("06_colore_1200dpi_2x2cm.pcap", 1200, true),
        ("06b_colore_2400dpi_2x2cm.pcap", 1200, true),
        ("07_grigio_300dpi_A4.pcap", 300, false),
        ("08_bn_300dpi_A4.pcap", 300, false),
        ("09_colore_300dpi_area_centro.pcap", 300, true),
        ("10_colore_300dpi_luminosita.pcap", 300, true),
        ("11_annulla.pcap", 300, true),
    ];
    for (name, dpi, colour) in captures {
        let Some(trace) = full_capture(name) else {
            return;
        };
        check_calibration(&trace, name, dpi, colour);
    }
}
