// SPDX-License-Identifier: GPL-2.0-or-later
//! `hp3770`: command line tool for the HP Scanjet 3770.

use clap::{Parser, Subcommand, ValueEnum};
use hp3770_core::device::Button;
use hp3770_core::transport::{Chip, ReadOnly, Transport};
use hp3770_usb::{Backend, Scanner};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Debug, Parser)]
#[command(name = "hp3770", version, about = "HP Scanjet 3770 driver tool")]
struct Cli {
    /// USB backend to use. By default the first one that finds a scanner.
    #[arg(long, global = true, value_enum, default_value_t = BackendArg::Auto)]
    backend: BackendArg,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BackendArg {
    /// usbscan.sys on Windows, then nusb.
    Auto,
    /// Direct USB (Linux, macOS, Windows with WinUSB).
    Nusb,
    /// Windows still-image driver installed by the HP software.
    Usbscan,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List the scanners every backend can see.
    List,
    /// Read the chip state without changing anything: registers and EEPROM.
    ///
    /// Every write is refused before it reaches the scanner, so the lamp, the
    /// motor and the EEPROM are never touched.
    Probe {
        /// Also save the report to this file.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Print the scanner state: lamp, carriage, transparency adapter, buttons.
    /// Register reads only.
    Status,
    /// Print the front-panel buttons as they are pressed.
    Buttons {
        /// Stop after this many presses (default: run until Ctrl+C).
        #[arg(short = 'n', long)]
        count: Option<u32>,
        /// How to detect the presses.
        #[arg(short, long, value_enum, default_value_t = ButtonMethod::Poll)]
        method: ButtonMethod,
        /// With `--method poll`, also print every change of the button
        /// registers (E968-E96A).
        #[arg(long)]
        raw: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ButtonMethod {
    /// Read the chip's button register every 50 ms. Works even when another
    /// program (e.g. the Windows WIA service) listens for button events.
    Poll,
    /// Wait for the event the scanner sends on its interrupt endpoint.
    Interrupt,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::List => list(),
        Command::Status => open(cli.backend).and_then(status),
        Command::Probe { output } => open(cli.backend).and_then(|s| probe(s, output)),
        Command::Buttons { count, method, raw } => {
            open(cli.backend).and_then(|s| buttons(s, count, method, raw))
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hp3770: {e}");
            ExitCode::FAILURE
        }
    }
}

type CliResult = Result<(), Box<dyn std::error::Error>>;

fn open(backend: BackendArg) -> Result<Scanner, Box<dyn std::error::Error>> {
    let scanner = match backend {
        BackendArg::Auto => hp3770_usb::open(),
        BackendArg::Nusb => hp3770_usb::open_with(Backend::Nusb),
        BackendArg::Usbscan => hp3770_usb::open_with(Backend::Usbscan),
    }?;
    eprintln!(
        "Opened the scanner with {} at {}",
        scanner.backend(),
        scanner.location()
    );
    Ok(scanner)
}

fn list() -> CliResult {
    let found = hp3770_usb::list();
    if found.is_empty() {
        println!("No HP Scanjet 3770 found.");
    }
    for f in found {
        match f.note {
            Some(note) => println!("{:<12} {}  ({note})", f.backend, f.location),
            None => println!("{:<12} {}", f.backend, f.location),
        }
    }
    Ok(())
}

fn status(scanner: Scanner) -> CliResult {
    let s = Chip::new(ReadOnly(scanner)).status()?;
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    let held: Vec<&str> = [Button::Scan, Button::Copy, Button::InstantShare]
        .into_iter()
        .filter(|b| s.buttons_held & b.mask() != 0)
        .map(Button::name)
        .collect();
    println!("USB speed          {}", if s.usb2 { "2.0" } else { "1.1" });
    println!("chipset id         {:02X}", s.chipset_id);
    println!("initialised        {}", yes_no(s.init_done));
    println!("operation running  {}", yes_no(s.executing));
    println!("carriage at home   {}", yes_no(s.head_at_home));
    println!("lamp               {}", s.lamp);
    println!("TMA plugged in     {}", yes_no(s.tma_attached));
    println!(
        "buttons held       {}",
        if held.is_empty() {
            "none".to_owned()
        } else {
            held.join(", ")
        }
    );
    println!("link word (E829)   {:04X}", s.link_word);
    Ok(())
}

/// Registers the HP driver reads outside the main block, with their length.
const STATUS_REGISTERS: [(u16, u16); 10] = [
    (0xfe11, 2),
    (0xfe39, 2),
    (0xfe3a, 2),
    (0xfe3e, 2),
    (0xf8ff, 2),
    (0xf900, 2),
    (0xf910, 2),
    (0xef09, 2),
    (0xef16, 4),
    (0xef19, 2),
];
/// Main register block, as the HP driver reads it.
const REGISTER_BLOCK: (u16, u16) = (0xe800, 0x0719);
/// EEPROM bytes known to be in use (`hp3900` touches 0x21-0x7c).
const EEPROM_SIZE: u16 = 0x80;

fn probe(scanner: Scanner, output: Option<PathBuf>) -> CliResult {
    let mut report = String::new();
    let mut failures = 0;
    let _ = writeln!(
        report,
        "hp3770 probe {} | backend {} | {}",
        env!("CARGO_PKG_VERSION"),
        scanner.backend(),
        scanner.location()
    );
    let mut chip = Chip::new(ReadOnly(scanner));

    let _ = writeln!(report, "\n[status registers]");
    for (address, len) in STATUS_REGISTERS {
        match chip.read_registers(address, len) {
            Ok(data) => {
                let _ = writeln!(report, "{address:04X}: {}", hex(&data));
            }
            Err(e) => {
                failures += 1;
                let _ = writeln!(report, "{address:04X}: ERROR {e}");
            }
        }
    }

    let (address, len) = REGISTER_BLOCK;
    let _ = writeln!(report, "\n[registers {address:04X}, {len} bytes]");
    match chip.read_registers(address, len) {
        Ok(data) => dump(&mut report, address, &data),
        Err(e) => {
            failures += 1;
            let _ = writeln!(report, "ERROR {e}");
        }
    }

    let _ = writeln!(report, "\n[eeprom 0000-{:04X}]", EEPROM_SIZE - 1);
    let mut eeprom = Vec::with_capacity(EEPROM_SIZE.into());
    for address in (0..EEPROM_SIZE).step_by(2) {
        match chip.read_eeprom_word(address) {
            Ok(word) => eeprom.extend_from_slice(&word.to_le_bytes()),
            Err(e) => {
                failures += 1;
                let _ = writeln!(report, "ERROR at {address:04X}: {e}");
                break;
            }
        }
    }
    dump(&mut report, 0, &eeprom);

    print!("{report}");
    if let Some(path) = output {
        std::fs::write(&path, &report)?;
        eprintln!("Report saved to {}", path.display());
    }
    if failures > 0 {
        return Err(format!("{failures} read(s) failed").into());
    }
    Ok(())
}

fn buttons(scanner: Scanner, count: Option<u32>, method: ButtonMethod, raw: bool) -> CliResult {
    eprintln!("Press a button on the scanner (Ctrl+C to stop).");
    let start = Instant::now();
    let mut seen = 0;
    match method {
        ButtonMethod::Interrupt => {
            let mut scanner = scanner;
            while count.is_none_or(|n| seen < n) {
                let Some(code) = scanner.interrupt_in(Duration::from_secs(1))? else {
                    continue;
                };
                seen += 1;
                let t = start.elapsed().as_secs_f32();
                let name = Button::from_code(code).map_or("unknown", Button::name);
                println!("{t:8.3}  code {code:02X}  {name}");
            }
        }
        ButtonMethod::Poll => {
            // Register reads only, through ReadOnly. E968 holds the buttons
            // pressed now (0 = pressed), E96A the ones released since the
            // last read (always 0 in the hardware test, kept as a fallback
            // for presses shorter than the polling interval).
            let mut chip = Chip::new(ReadOnly(scanner));
            let first = chip.read_registers(0xe968, 3)?;
            if raw {
                println!("   0.000  E968-E96A {}", hex(&first));
            }
            let mut last = first.clone();
            let mut held = !first[0] & 0x3f;
            while count.is_none_or(|n| seen < n) {
                std::thread::sleep(Duration::from_millis(50));
                let regs = chip.read_registers(0xe968, 3)?;
                let t = start.elapsed().as_secs_f32();
                if raw && regs != last {
                    println!("{t:8.3}  E968-E96A {}", hex(&regs));
                }
                let now = !regs[0] & 0x3f;
                let released = regs[2] & 0x3f;
                // A press seen in E968, or one so short that only the
                // release latch in E96A caught it.
                let pressed = (now & !held) | (released & !held & !now);
                for bit in (0..6).map(|i| 1u8 << i).filter(|b| pressed & b != 0) {
                    seen += 1;
                    let name = Button::from_mask(bit).map_or("unknown", Button::name);
                    println!("{t:8.3}  bit {bit:02X}  {name}");
                }
                held = now;
                last = regs;
            }
        }
    }
    Ok(())
}

fn hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02X}");
    }
    s
}

/// Hex dump, 16 bytes per line, addressed from `base`.
fn dump(out: &mut String, base: u16, data: &[u8]) {
    for (i, line) in data.chunks(16).enumerate() {
        let address = usize::from(base) + i * 16;
        let _ = writeln!(out, "{address:04X}: {}", hex(line));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_addresses_lines() {
        let mut out = String::new();
        dump(&mut out, 0xe800, &[0x04, 0x5c].repeat(9));
        assert_eq!(
            out,
            "E800: 04 5C 04 5C 04 5C 04 5C 04 5C 04 5C 04 5C 04 5C\nE810: 04 5C\n"
        );
    }

    #[test]
    fn cli_parses() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
        let cli = Cli::parse_from(["hp3770", "--backend", "usbscan", "probe", "-o", "x.txt"]);
        assert_eq!(cli.backend, BackendArg::Usbscan);
        assert!(matches!(cli.command, Command::Probe { output: Some(_) }));
    }
}
