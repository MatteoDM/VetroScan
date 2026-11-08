// SPDX-License-Identifier: GPL-2.0-or-later
//! Development tool: sends the HP driver's requests from a capture to the
//! real scanner, with the same pauses.
//!
//! `cargo run -p hp3770-cli --example replay -- CAPTURE.pcap [FROM_S] [TO_S]`
//!
//! Register reads and writes and DMA control words are sent as the HP
//! driver sent them, between `FROM_S` and `TO_S` seconds of the capture
//! (all of it by default); each read is compared with what the scanner
//! answered the HP driver, and the differences are printed. Left out: the
//! EEPROM writes (this project never writes the EEPROM), bulk transfers,
//! standard requests and anything else not understood. Pauses longer than
//! two seconds are cut to two. For finding by hand which writes of the HP
//! driver do something the driver here does not; not part of the program.

use hp3770_capture::trace::{Event, Trace};
use hp3770_core::protocol::{Command, Hex};
use hp3770_core::transport::Transport;
use std::process::ExitCode;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: replay CAPTURE.pcap [FROM_S] [TO_S]");
        return ExitCode::FAILURE;
    };
    let from: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let to: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(f64::MAX);
    let trace = match Trace::from_path(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("replay: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut scanner = match hp3770_usb::open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("replay: {e}");
            return ExitCode::FAILURE;
        }
    };
    let start = Instant::now();
    let mut first: Option<f64> = None;
    let mut skipped = 0usize;
    for event in &trace.events {
        let Event::Control {
            time,
            command,
            response,
            ..
        } = event
        else {
            continue;
        };
        if *time < from || *time > to {
            continue;
        }
        let send = matches!(
            command,
            Command::ReadRegisters { .. }
                | Command::WriteRegisters { .. }
                | Command::ReadEeprom { .. }
                | Command::DmaControl { .. }
        );
        if !send {
            if !matches!(command, Command::Standard(_)) {
                println!("{time:>8.3}  skipped  {command}");
            }
            skipped += 1;
            continue;
        }
        // The capture's pace, pauses cut to two seconds.
        let t0 = *first.get_or_insert(*time);
        let due = Duration::from_secs_f64((time - t0).max(0.0));
        let elapsed = start.elapsed();
        if due > elapsed {
            std::thread::sleep((due - elapsed).min(Duration::from_secs(2)));
        }
        let Some((setup, data)) = command.encode() else {
            println!("{time:>8.3}  cannot encode {command}");
            continue;
        };
        let result = if setup.is_in() {
            scanner.control_in(setup).map(|got| {
                if &got != response {
                    println!(
                        "{time:>8.3}  {command}: HP got {}, now {}",
                        Hex(response),
                        Hex(&got)
                    );
                }
            })
        } else {
            scanner.control_out(setup, &data).map(|()| {
                if !matches!(command, Command::ReadRegisters { .. }) {
                    println!("{time:>8.3}  sent     {command}");
                }
            })
        };
        if let Err(e) = result {
            eprintln!("replay: at {time:.3} s, {command}: {e}");
            return ExitCode::FAILURE;
        }
    }
    println!("done, {skipped} requests not sent");
    ExitCode::SUCCESS
}
