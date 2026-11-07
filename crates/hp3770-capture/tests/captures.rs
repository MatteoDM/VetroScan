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
    // block in 05 (16.02 s) and 06 (15.16 s), at the end of the scan.
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

    // At 600 and 1200 dpi the chip leaves the carriage at the end of the
    // area and the HP driver brings it home when it stops the scan: same steps and same block as after a
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

/// Full register blocks the HP driver wrote (`WriteRegisters` of the 1818
/// bytes at `E800`), in order, with the value of `E954`-`E955` it last read
/// from the chip before each one (`Motor_Change` reads it between the two
/// writes of a read).
fn register_blocks(trace: &Trace) -> Vec<(Vec<u8>, Option<[u8; 2]>)> {
    let mut out = Vec::new();
    let mut e954 = None;
    for event in &trace.events {
        match event {
            Event::Control {
                command:
                    Command::WriteRegisters {
                        address: 0xe800,
                        data,
                    },
                ..
            } if data.len() == 0x71a => out.push((data.clone(), e954)),
            Event::Control {
                command:
                    Command::ReadRegisters {
                        address: 0xe954, ..
                    },
                response,
                ..
            } if response.len() == 2 => e954 = Some([response[0], response[1]]),
            _ => {}
        }
    }
    out
}

/// One read the HP driver set up with two block writes: what we give our
/// set-up, and which blocks of the capture it must give.
struct BlockRead {
    /// Index of the first of the two blocks among the full block writes.
    block: usize,
    /// A scan (else a calibration read, `RTS_GetImage`).
    scan: bool,
    /// The lamp is off (black shading strip).
    lamp_off: bool,
    params: hp3770_core::setup::ScanParams,
    hw: hp3770_core::setup::Hardware,
}

/// The analog front-end settings in a block: offsets of even and odd
/// pixels (9 bits) and gains (5 bits). Our calibration module decides them
/// (`calibration_matches_the_hp_driver`); here they are only an input.
fn gain_offset_of(block: &[u8]) -> hp3770_core::setup::GainOffset {
    let nine = |o: usize| u16::from(block[o]) | u16::from(block[o + 1] & 0x80) << 1;
    hp3770_core::setup::GainOffset {
        even_offset: [nine(0x1a), nine(0x1e), nine(0x22)],
        odd_offset: [nine(0x1c), nine(0x20), nine(0x24)],
        even_fine: [0; 3],
        odd_fine: [0; 3],
        gain: [block[0x14] & 0x1f, block[0x15] & 0x1f, block[0x16] & 0x1f],
        gain2: [4; 3],
        pga: [3; 3],
    }
}

fn diff(ours: &[u8], hp: &[u8]) -> String {
    (0..hp.len())
        .filter(|&i| ours[i] != hp[i])
        .map(|i| format!("E{:03X}: {:02x}/{:02x}", 0x800 + i, ours[i], hp[i]))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rebuilds both blocks of each read from its parameters and checks them
/// byte for byte against the HP driver's.
fn check_blocks(trace: &Trace, name: &str, reads: &[BlockRead]) {
    use hp3770_core::init::Registers;
    use hp3770_core::profile::HP3770;
    use hp3770_core::setup::{
        finish_image_registers, finish_scan_registers, image_registers, scan_registers,
    };

    let blocks = register_blocks(trace);
    // The initial block, as the HP driver keeps it in memory once the
    // calibration has enabled the three CCD channels (RTS_Enable_CCD:
    // bits 5-7 of E810, bit 7 of E813).
    let mut init = Registers::default();
    init.0.copy_from_slice(&blocks[0].0);
    init.0[0x10] |= 0xe0;
    init.0[0x13] |= 0x80;
    for read in reads {
        let (first, _) = &blocks[read.block];
        let (second, e954) = &blocks[read.block + 1];
        let mut base = init.clone();
        if read.lamp_off {
            // Lamp_Status_Set off: bit 6 of E946 in the in-memory block.
            base.0[0x146] &= !0x40;
        }
        // E954-E955 are not set up: the HP driver leaves the motor current
        // and lamp bits of the previous read in its in-memory block.
        base.0[0x154..0x156].copy_from_slice(&first[0x154..0x156]);
        let go = gain_offset_of(second);
        let (mut regs, params, _) = if read.scan {
            scan_registers(&base, &HP3770, &read.params, &read.hw, &go)
        } else {
            image_registers(&base, &HP3770, &read.params, &read.hw, &go)
        }
        .unwrap();
        assert!(
            regs.0[..] == first[..],
            "{name}, block {} (ours/HP): {}",
            read.block,
            diff(&regs.0[..], first)
        );
        let e954 = e954.expect("E954 read before the second block");
        if read.scan {
            finish_scan_registers(&mut regs, &HP3770, e954, &params);
        } else {
            finish_image_registers(&mut regs, &HP3770, e954, &params);
        }
        assert!(
            regs.0[..] == second[..],
            "{name}, block {} (ours/HP): {}",
            read.block + 1,
            diff(&regs.0[..], second)
        );
    }
}

/// The reads of a calibration and scan, as the HP driver sets them up at
/// `dpi` (the sensor resolution: 1200 for a 2400 dpi scan, which is
/// 1200 x 2400 on the chip). `first` is the index of the first probe block,
/// `checks` the number of offset checks (one more each time a channel is
/// still too dark), `scan` the window of the scan (left, width, top,
/// height) and `y` its vertical resolution.
fn hp_reads(
    first: usize,
    checks: usize,
    dpi: u16,
    probe_left: u32,
    scan: (u32, u32, u32, u32),
    y: u16,
    scan_hw: hp3770_core::setup::Hardware,
) -> Vec<BlockRead> {
    use hp3770_core::setup::{ArrangeLine, Hardware, ScanParams};
    let read = |left, width, height, resolution_y| ScanParams {
        depth: 8,
        resolution_x: dpi,
        resolution_y,
        left,
        top: 1,
        width,
        height,
    };
    let off = Hardware {
        black_shading: false,
        white_shading: false,
        gamma: false,
        gamma_size: 0,
        backward: false,
        static_head: true,
        dummy_scan: false,
        compression: false,
        arrange_line: ArrangeLine::None,
        high_resolution: false,
    };
    let (left, width, top, height) = scan;
    let calibration_y = if y > dpi { y } else { dpi };
    let mut reads = Vec::new();
    // Offset probes at 300 and 350 (twice below 1200 dpi), one line each,
    // then the check, ten lines.
    let probes = if dpi < 1200 { 4 } else { 2 };
    for i in 0..probes {
        reads.push(BlockRead {
            block: first + 2 * i,
            scan: false,
            lamp_off: false,
            params: read(probe_left, 100, 1, calibration_y),
            hw: off,
        });
    }
    for i in 0..checks {
        reads.push(BlockRead {
            block: first + 2 * (probes + i),
            scan: false,
            lamp_off: false,
            params: read(probe_left, 100, 10, calibration_y),
            hw: off,
        });
    }
    let next = first + 2 * (probes + checks - 1);
    // Gain, black (lamp off) over the scan width, ten lines; then the white
    // strip, 24 lines at 200 dpi with the head moving and black shading on.
    for (block, lamp_off) in [(next + 2, false), (next + 4, true)] {
        reads.push(BlockRead {
            block,
            scan: false,
            lamp_off,
            params: read(left, width, 10, calibration_y),
            hw: off,
        });
    }
    reads.push(BlockRead {
        block: next + 6,
        scan: false,
        lamp_off: false,
        params: read(left, width, 24, 200),
        hw: Hardware {
            black_shading: true,
            static_head: false,
            ..off
        },
    });
    // After a carriage move (one block), the scan.
    reads.push(BlockRead {
        block: next + 9,
        scan: true,
        lamp_off: false,
        params: ScanParams {
            top,
            height,
            ..read(left, width, height, y)
        },
        hw: scan_hw,
    });
    reads
}

/// The scan of the HP driver: shading and gamma on, lines arranged by the
/// chip (by the host at 2400 dpi), even and odd pixels apart from 1200 dpi.
fn hp_scan_hardware(
    dpi: u16,
    arrange_line: hp3770_core::setup::ArrangeLine,
    gamma: bool,
) -> hp3770_core::setup::Hardware {
    hp3770_core::setup::Hardware {
        black_shading: true,
        white_shading: true,
        gamma,
        gamma_size: 0,
        backward: false,
        static_head: false,
        dummy_scan: false,
        compression: false,
        arrange_line,
        high_resolution: dpi >= 1200,
    }
}

#[test]
fn register_blocks_match_the_hp_driver() {
    use hp3770_core::setup::ArrangeLine;
    // 09_calibrazione: a 300 dpi scan of 5 x 5 cm in the middle of the
    // glass. The HP driver arranged its lines by software (it keeps doing
    // so after a 2400 dpi scan: 06b came before 09); we arrange them by
    // hardware up to 1200 dpi, as in 04, which this also checks below.
    check_blocks(
        &fixture("09_calibrazione.pcap"),
        "09_calibrazione",
        &hp_reads(
            1,
            1,
            300,
            7,
            (1009, 592, 1561, 591),
            300,
            hp_scan_hardware(300, ArrangeLine::Software, true),
        ),
    );
}

#[test]
fn register_blocks_of_every_scan_capture() {
    use hp3770_core::setup::ArrangeLine::{Hardware, Software};
    // (capture, offset checks, sensor dpi, probe left, scan window,
    // vertical dpi, scan hardware). Grey (07) and lineart (08) scan in
    // colour without gamma.
    let captures = [
        (
            "01_anteprima.pcap",
            2,
            200,
            7,
            (43, 1656, 69, 2338),
            200,
            hp_scan_hardware(200, Hardware, true),
        ),
        (
            "02_colore_200dpi_A4.pcap",
            3,
            200,
            7,
            (43, 1656, 69, 2338),
            200,
            hp_scan_hardware(200, Hardware, true),
        ),
        (
            "04_colore_300dpi_A4.pcap",
            1,
            300,
            7,
            (65, 2480, 102, 3508),
            300,
            hp_scan_hardware(300, Hardware, true),
        ),
        (
            "05_colore_600dpi_5x5cm.pcap",
            2,
            600,
            17,
            (129, 1180, 198, 1180),
            600,
            hp_scan_hardware(600, Hardware, true),
        ),
        (
            "06_colore_1200dpi_2x2cm.pcap",
            2,
            1200,
            15,
            (259, 944, 373, 944),
            1200,
            hp_scan_hardware(1200, Hardware, true),
        ),
        (
            "06b_colore_2400dpi_2x2cm.pcap",
            2,
            1200,
            15,
            (259, 944, 746, 1896),
            2400,
            hp_scan_hardware(1200, Software, true),
        ),
        (
            "07_grigio_300dpi_A4.pcap",
            2,
            300,
            7,
            (65, 2480, 102, 3508),
            300,
            hp_scan_hardware(300, Software, false),
        ),
        (
            "08_bn_300dpi_A4.pcap",
            2,
            300,
            7,
            (65, 2480, 102, 3508),
            300,
            hp_scan_hardware(300, Software, false),
        ),
        (
            "10_colore_300dpi_luminosita.pcap",
            2,
            300,
            7,
            (65, 2480, 102, 3508),
            300,
            hp_scan_hardware(300, Software, true),
        ),
        (
            "11_annulla.pcap",
            2,
            300,
            7,
            (65, 2480, 102, 3508),
            300,
            hp_scan_hardware(300, Software, true),
        ),
    ];
    for (name, checks, dpi, probe_left, scan, y, hw) in captures {
        let Some(trace) = full_capture(name) else {
            return;
        };
        check_blocks(
            &trace,
            name,
            &hp_reads(1, checks, dpi, probe_left, scan, y, hw),
        );
    }
}

#[test]
fn get_image_matches_the_hp_driver() {
    use hp3770_capture::replay::Replay;
    use hp3770_core::init::Registers;
    use hp3770_core::profile::HP3770;
    use hp3770_core::setup::{ArrangeLine, Hardware, ScanParams};
    use hp3770_core::transport::Chip;

    // The first offset probe of 09_calibrazione (0.190-0.567 s): two
    // blocks, motor current, start, 9 polls of the data in the chip, one
    // line of 100 pixels read on DMA channel 8, unlock, motor current 3.
    let trace = fixture("09_calibrazione.pcap");
    let blocks = register_blocks(&trace);
    let mut init = Registers::default();
    init.0.copy_from_slice(&blocks[0].0);
    init.0[0x10] |= 0xe0;
    init.0[0x13] |= 0x80;
    init.0[0x154..0x156].copy_from_slice(&blocks[1].0[0x154..0x156]);
    let params = ScanParams {
        depth: 8,
        resolution_x: 300,
        resolution_y: 300,
        left: 7,
        top: 1,
        width: 100,
        height: 1,
    };
    let hw = Hardware {
        black_shading: false,
        white_shading: false,
        gamma: false,
        gamma_size: 0,
        backward: false,
        static_head: true,
        dummy_scan: false,
        compression: false,
        arrange_line: ArrangeLine::None,
        high_resolution: false,
    };
    let go = gain_offset_of(&blocks[2].0);
    let mut chip = Chip::new(Replay::window(&trace, 0.190, 0.567));
    let image = chip
        .get_image(&mut init, &HP3770, &params, &hw, &go, &Default::default())
        .unwrap();
    chip.transport_mut().finish().unwrap();
    assert_eq!(image, image_reads(&trace)[0].1);
    assert_eq!(init.0[0x154..0x156], [0xb0, 0x01]);
}

#[test]
fn white_strip_read_matches_the_hp_driver() {
    use hp3770_capture::replay::Replay;
    use hp3770_core::init::Registers;
    use hp3770_core::profile::HP3770;
    use hp3770_core::read::Tables;
    use hp3770_core::setup::{ArrangeLine, Hardware, ScanParams};
    use hp3770_core::transport::Chip;

    // The white strip of 09_calibrazione (3.209-4.113 s): motor step
    // tables, first block, the three black shading tables written and read
    // back, then 24 lines at 200 dpi with the head moving.
    let trace = fixture("09_calibrazione.pcap");
    let blocks = register_blocks(&trace);
    let mut init = Registers::default();
    init.0.copy_from_slice(&blocks[0].0);
    init.0[0x10] |= 0xe0;
    init.0[0x13] |= 0x80;
    init.0[0x154..0x156].copy_from_slice(&blocks[15].0[0x154..0x156]);
    let params = ScanParams {
        depth: 8,
        resolution_x: 300,
        resolution_y: 200,
        left: 1009,
        top: 1,
        width: 592,
        height: 24,
    };
    let hw = Hardware {
        black_shading: true,
        white_shading: false,
        gamma: false,
        gamma_size: 0,
        backward: false,
        static_head: false,
        dummy_scan: false,
        compression: false,
        arrange_line: ArrangeLine::None,
        high_resolution: false,
    };
    let go = gain_offset_of(&blocks[16].0);
    // The black tables the HP driver computed (calibration_matches_the_hp_driver
    // checks that ours are the same).
    let black: Vec<Vec<u8>> = table_writes(&trace)
        .into_iter()
        .filter(|(t, channel, _)| (3.2..4.2).contains(t) && (0x10..=0x12).contains(channel))
        .map(|(_, _, data)| data)
        .collect();
    assert_eq!(black.len(), 3);
    let tables = Tables {
        black: Some([&black[0], &black[1], &black[2]]),
        ..Default::default()
    };
    let mut chip = Chip::new(Replay::window(&trace, 3.209, 4.113).any_dma_control_value());
    let image = chip
        .get_image(&mut init, &HP3770, &params, &hw, &go, &tables)
        .unwrap();
    chip.transport_mut().finish().unwrap();
    assert_eq!(image.len(), 592 * 3 * 24);
    let hp = image_reads(&trace)
        .into_iter()
        .find(|(t, _)| (3.2..4.2).contains(t))
        .unwrap();
    assert_eq!(image, hp.1);
}

/// Replays a whole scan of a capture, `from`-`to` s: start, image data,
/// stop (with the park at 600 dpi and up). The HP driver also counts the
/// scan in the EEPROM (after reading E9A1); we never write the EEPROM, so
/// those requests are left out. `read` is the scan as the set-up checks of
/// [`hp_reads`] give it; `parked` says whether the HP driver parks the
/// carriage at the end.
fn check_scan(trace: &Trace, name: &str, read: &BlockRead, from: f64, to: f64, parked: bool) {
    use hp3770_capture::replay::Replay;
    use hp3770_core::init::Registers;
    use hp3770_core::profile::HP3770;
    use hp3770_core::read::Tables;
    use hp3770_core::transport::Chip;

    let blocks = register_blocks(trace);
    let mut init = Registers::default();
    init.0.copy_from_slice(&blocks[0].0);
    init.0[0x10] |= 0xe0;
    init.0[0x13] |= 0x80;
    init.0[0x154..0x156].copy_from_slice(&blocks[read.block].0[0x154..0x156]);
    let go = gain_offset_of(&blocks[read.block + 1].0);
    // Gamma (channel 0) and shading tables, written after the first block;
    // the motor step tables, also on channel 0, come before it.
    let first_block = block_times(trace)[read.block];
    let writes: Vec<(u16, Vec<u8>)> = table_writes(trace)
        .into_iter()
        .filter(|(t, ..)| (first_block..to).contains(t))
        .map(|(_, channel, data)| (channel, data))
        .collect();
    let table = |channel: u16| -> Option<&[u8]> {
        writes
            .iter()
            .find(|(c, _)| *c == channel)
            .map(|(_, data)| data.as_slice())
    };
    let three = |base: u16| -> Option<[&[u8]; 3]> {
        Some([table(base)?, table(base + 1)?, table(base + 2)?])
    };
    let tables = Tables {
        gamma: table(0),
        black: three(0x10),
        white: three(0x14),
    };
    let events = trace.events.iter().filter(|e| {
        (from..to).contains(&e.time())
            && !matches!(
                e,
                Event::Control {
                    command: Command::ReadEeprom { .. }
                        | Command::WriteEeprom { .. }
                        | Command::ReadRegisters {
                            address: 0xe9a1,
                            ..
                        },
                    ..
                }
            )
    });
    let mut chip = Chip::new(Replay::new(events.cloned()).any_dma_control_value());
    let mut scan = chip
        .start_scan(&init, &HP3770, &read.params, &read.hw, &go, &tables)
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut image = Vec::new();
    let mut piece = Vec::new();
    while chip
        .read_scan(&mut scan, &mut piece)
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        > 0
    {
        image.extend_from_slice(&piece);
    }
    assert_eq!(
        chip.stop_scan(&mut init, &HP3770)
            .unwrap_or_else(|e| panic!("{name}: {e}")),
        parked,
        "{name}"
    );
    chip.transport_mut()
        .finish()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    let hp: Vec<u8> = image_reads(trace)
        .into_iter()
        .filter(|(t, _)| (from..to).contains(t))
        .flat_map(|(_, data)| data)
        .collect();
    assert_eq!(image.len(), hp.len(), "{name}");
    assert!(image == hp, "{name}: different image data");
}

/// Times of the full register block writes.
fn block_times(trace: &Trace) -> Vec<f64> {
    trace
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Control {
                time,
                command:
                    Command::WriteRegisters {
                        address: 0xe800,
                        data,
                    },
                ..
            } if data.len() == 0x71a => Some(*time),
            _ => None,
        })
        .collect()
}

#[test]
fn scan_matches_the_hp_driver() {
    use hp3770_core::setup::ArrangeLine::{Hardware, Software};
    use hp3770_core::setup::ScanParams;

    // 09_colore_300dpi_area_centro (6.656-14.166 s): step tables, first
    // block, gamma and the six shading tables, second block, start, 17
    // pieces of image data each waited for in the chip, stop. Software line
    // arrangement: 13 extra lines.
    let Some(trace) = full_capture("09_colore_300dpi_area_centro.pcap") else {
        return;
    };
    let read = BlockRead {
        block: 18,
        scan: true,
        lamp_off: false,
        params: ScanParams {
            depth: 8,
            resolution_x: 300,
            resolution_y: 300,
            left: 1009,
            top: 1561,
            width: 592,
            height: 591,
        },
        hw: hp_scan_hardware(300, Software, true),
    };
    check_scan(&trace, "09", &read, 6.656, 14.166, false);

    // A4 at 300 dpi (three DMA transfers, the last two pieces waited for
    // with the margin), then 600, 1200 and 2400 dpi, which have no motor
    // curves: the chip leaves the carriage at the end of the area and the
    // stop parks it, before switching the CCD off. 2400 dpi arranges the
    // lines in software (105 extra lines).
    for (name, checks, dpi, probe_left, window, y, hw, from, to) in [
        (
            "04_colore_300dpi_A4.pcap",
            1,
            300,
            7,
            (65, 2480, 102, 3508),
            300,
            hp_scan_hardware(300, Hardware, true),
            6.601,
            18.515,
        ),
        (
            "05_colore_600dpi_5x5cm.pcap",
            2,
            600,
            17,
            (129, 1180, 198, 1180),
            600,
            hp_scan_hardware(600, Hardware, true),
            6.651,
            17.285,
        ),
        (
            "06_colore_1200dpi_2x2cm.pcap",
            2,
            1200,
            15,
            (259, 944, 373, 944),
            1200,
            hp_scan_hardware(1200, Hardware, true),
            6.001,
            15.926,
        ),
        (
            "06b_colore_2400dpi_2x2cm.pcap",
            2,
            1200,
            15,
            (259, 944, 746, 1896),
            2400,
            hp_scan_hardware(1200, Software, true),
            5.967,
            24.336,
        ),
    ] {
        let Some(trace) = full_capture(name) else {
            return;
        };
        let read = hp_reads(1, checks, dpi, probe_left, window, y, hw)
            .pop()
            .unwrap();
        check_scan(&trace, name, &read, from, to, dpi >= 600);
    }
}
