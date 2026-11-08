// SPDX-License-Identifier: GPL-2.0-or-later
//! Development tool: writes chip registers, then reads them back.
//!
//! `cargo run -p hp3770-cli --example poke -- E950=BF4B E946=B9 E954=B010`
//! writes each group of bytes at its address, in order (hex, the first
//! byte at the address, the next ones after it), then prints `E940`-`E96F`.
//! Never the EEPROM, never the motor: only what is given. For trying the
//! lamps and the GPIO lines by hand; not part of the program.

use hp3770_core::transport::Chip;
use std::process::ExitCode;

fn parse(arg: &str) -> Option<(u16, Vec<u8>)> {
    let (address, bytes) = arg.split_once('=')?;
    let address = u16::from_str_radix(address, 16).ok()?;
    if !(0xe800..0xef20).contains(&address) || bytes.is_empty() || bytes.len() % 2 != 0 {
        return None;
    }
    let data = (0..bytes.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&bytes[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .ok()?;
    Some((address, data))
}

fn main() -> ExitCode {
    let writes: Option<Vec<(u16, Vec<u8>)>> = std::env::args().skip(1).map(|a| parse(&a)).collect();
    let Some(writes) = writes else {
        eprintln!("usage: poke ADDRESS=HEXBYTES ... (addresses E800-EF1F)");
        return ExitCode::FAILURE;
    };
    let scanner = match hp3770_usb::open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("poke: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut chip = Chip::new(scanner);
    let mut run = || -> hp3770_core::error::Result<()> {
        for (address, data) in &writes {
            chip.write_registers(*address, data)?;
            println!("wrote {address:04X}: {}", hex(data));
        }
        let regs = chip.read_registers(0xe940, 0x30)?;
        for (i, line) in regs.chunks(16).enumerate() {
            println!("{:04X}: {}", 0xe940 + i * 16, hex(line));
        }
        Ok(())
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("poke: {e}");
            ExitCode::FAILURE
        }
    }
}

fn hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
