// SPDX-License-Identifier: GPL-2.0-or-later
//! `epson4490`: the Epson Perfection 4490 from the command line, for its
//! first days on the desk (phase 12, milestone 1). Nothing it sends moves
//! the carriage: every write goes through the backend's guard.
//!
//! ```text
//! epson4490 list                       the scanners found, nothing sent
//! epson4490 firmware [--firmware F]    which esfw54.bin would be sent, no scanner needed
//! epson4490 probe                      list, firmware, then the state (1B 03) of the first
//! epson4490 upload [--firmware F]      sends the firmware if it is not running
//! ```

use epson_gt_x750::guard::Guarded;
use epson_gt_x750::protocol::{self, FirmwareOutcome, State};
use epson_gt_x750::{firmware, Found, Transport, PRODUCT_ID, VENDOR_ID};
use scanner_api::Error;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str =
    "usage: epson4490 list | firmware [--firmware FILE] | probe | upload [--firmware FILE]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut firmware_path = None;
    let mut command = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--firmware" => match rest.next() {
                Some(path) => firmware_path = Some(PathBuf::from(path)),
                None => return usage(),
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            c if command.is_none() && !c.starts_with('-') => command = Some(c.to_owned()),
            _ => return usage(),
        }
    }
    let result = match command.as_deref() {
        Some("list") => {
            list();
            Ok(())
        }
        Some("firmware") => show_firmware(firmware_path.as_deref()),
        Some("probe") => probe(firmware_path.as_deref()),
        Some("upload") => upload(firmware_path),
        _ => return usage(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("epson4490: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn list() -> Vec<Found> {
    let found = epson_gt_x750::list();
    if found.is_empty() {
        println!("no Epson Perfection 4490 ({VENDOR_ID:04x}:{PRODUCT_ID:04x}) found");
    }
    for f in &found {
        match &f.note {
            Some(note) => println!("{} {} (cannot open: {note})", f.link, f.location),
            None => println!("{} {}", f.link, f.location),
        }
    }
    found
}

fn show_firmware(path: Option<&std::path::Path>) -> Result<(), Error> {
    println!(
        "embedded in this build: {}",
        if firmware::embedded() { "yes" } else { "no" }
    );
    let fw = firmware::find(path).map_err(Error::Other)?;
    println!(
        "would send: {} ({}), SHA-256 {}",
        fw.origin(),
        fw.build(),
        firmware::sha256_hex(fw.image())
    );
    Ok(())
}

/// The first scanner a link can open, guarded.
fn open_first() -> Result<Guarded<Box<dyn Transport>>, Error> {
    let mut last = Error::Disconnected("no Epson Perfection 4490 found".into());
    for f in epson_gt_x750::list()
        .into_iter()
        .filter(|f| f.note.is_none())
    {
        match epson_gt_x750::open(&f.location) {
            Ok(t) => {
                println!("opened {} {}", f.link, f.location);
                return Ok(Guarded::new(t));
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn describe(state: State) -> String {
    format!(
        "{:02X} {:02X}: firmware {}, {}",
        state.0[0],
        state.0[1],
        if state.firmware_running() {
            "running"
        } else {
            "not running"
        },
        if state.busy() { "busy" } else { "not busy" }
    )
}

fn probe(path: Option<&std::path::Path>) -> Result<(), Error> {
    if list().is_empty() {
        return Ok(());
    }
    if let Err(e) = show_firmware(path) {
        println!("{e}");
    }
    let mut t = open_first()?;
    println!("state (1B 03): {}", describe(protocol::state(&mut t)?));
    Ok(())
}

fn upload(path: Option<PathBuf>) -> Result<(), Error> {
    let mut t = open_first()?;
    let outcome = protocol::ensure_firmware(&mut t, || {
        firmware::find(path.as_deref()).map_err(Error::Other)
    })?;
    match outcome {
        FirmwareOutcome::AlreadyRunning => println!("the firmware was running already"),
        FirmwareOutcome::Uploaded { build, origin } => {
            println!("firmware sent ({build}, {origin}), every step answered 06")
        }
    }
    match protocol::state(&mut t) {
        Ok(state) => println!("state (1B 03): {}", describe(state)),
        Err(e) => println!(
            "state (1B 03) not read after the upload ({e}): the scanner may have left the bus \
             and come back; run `epson4490 probe`"
        ),
    }
    Ok(())
}
