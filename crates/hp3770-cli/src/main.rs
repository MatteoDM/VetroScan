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
    /// Initialise the chip like the HP driver does after a button press.
    ///
    /// Writes registers (USB clock, RAM type, motor current, warm reset,
    /// buttons, lamp timer) with the values the HP driver uses. Never writes
    /// the EEPROM and never moves the carriage.
    Init,
    /// Bring the carriage home if it is not there.
    ///
    /// Runs `init`, then, only if the carriage is away from home, moves it
    /// back with the motor table and the register block the HP driver uses
    /// (capture-verified). Does nothing else if it is already home.
    Park,
    /// Move the carriage away from home by a few full motor steps (test).
    ///
    /// Runs `init`, then the same move as `park` towards the far end. The
    /// HP driver does not move forward this way in the captures, so the
    /// direction bit is hp3900's, not verified. Limited to 600 steps.
    Forward {
        /// Full motor steps (at most 600).
        #[arg(value_parser = clap::value_parser!(u16).range(1..=600))]
        steps: u16,
    },
    /// Switch the flatbed lamp on or off.
    ///
    /// Writes E946 and E954 like the HP driver. The lamp also switches off
    /// by itself after the timer set by `init` (13 minutes).
    Lamp {
        /// on or off.
        #[arg(value_enum)]
        state: LampState,
    },
    /// Scan an area of the glass in colour and save it as PNG (or PPM if
    /// the file name ends in .ppm).
    ///
    /// Runs `init`, parks the carriage if needed, calibrates and scans
    /// exactly as the HP driver does (capture-verified), then stops. The
    /// area is in millimetres from the top left corner of the glass. Never
    /// writes the EEPROM.
    Scan {
        /// Output file.
        output: PathBuf,
        /// Resolution: 200, 300, 600, 1200 or 2400 dpi.
        #[arg(long, default_value_t = 300)]
        dpi: u16,
        /// Left edge, mm.
        #[arg(long, default_value_t = 0.0)]
        left: f64,
        /// Top edge, mm.
        #[arg(long, default_value_t = 0.0)]
        top: f64,
        /// Width, mm (default: A4).
        #[arg(long, default_value_t = 210.0)]
        width: f64,
        /// Height, mm (default: A4).
        #[arg(long, default_value_t = 297.0)]
        height: f64,
        /// Save the chip's linear samples, without the host gamma 2.2 the
        /// HP driver applies.
        #[arg(long)]
        linear: bool,
    },
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
enum LampState {
    On,
    Off,
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
        Command::Init => open(cli.backend).and_then(init),
        Command::Park => open(cli.backend).and_then(park),
        Command::Forward { steps } => open(cli.backend).and_then(|s| forward(s, steps)),
        Command::Lamp { state } => open(cli.backend).and_then(|s| lamp(s, state)),
        Command::Probe { output } => open(cli.backend).and_then(|s| probe(s, output)),
        Command::Scan {
            output,
            dpi,
            left,
            top,
            width,
            height,
            linear,
        } => open(cli.backend).and_then(|s| {
            let area = hp3770_core::geometry::Area::from_mm(dpi, left, top, width, height);
            scan(s, &output, dpi, area, linear)
        }),
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

fn init(scanner: Scanner) -> CliResult {
    let start = Instant::now();
    let link = Chip::new(scanner).link(&hp3770_core::profile::HP3770)?;
    let regs = &link.registers;
    println!("done in          {:.1} s", start.elapsed().as_secs_f32());
    println!("link word (E829) {:04X}", link.link_word);
    println!(
        "carriage         {}",
        if link.at_home {
            "at home"
        } else {
            "NOT at home (run `park` to bring it back)"
        }
    );
    println!(
        "lamp timer       {:02X} {:02X} (E946 E947)",
        regs.get(0x146),
        regs.get(0x147)
    );
    Ok(())
}

fn park(scanner: Scanner) -> CliResult {
    let profile = &hp3770_core::profile::HP3770;
    let mut chip = Chip::new(scanner);
    let link = chip.link(profile)?;
    let start = Instant::now();
    let base = hp3770_core::motor::park_registers(&link.registers);
    if chip.head_park_home(&base, &profile.park)? {
        println!(
            "carriage         parked in {:.1} s",
            start.elapsed().as_secs_f32()
        );
        println!(
            "at home now      {}",
            if chip.head_is_at_home()? { "yes" } else { "NO" }
        );
    } else {
        println!("carriage         already at home, not moved");
    }
    Ok(())
}

fn forward(scanner: Scanner, steps: u16) -> CliResult {
    use hp3770_core::motor::{park_registers, Direction, MotorPosition};
    let profile = &hp3770_core::profile::HP3770;
    let mut chip = Chip::new(scanner);
    let link = chip.link(profile)?;
    // Head_Relocate in hp3900: forward, E801 bit 2 clear.
    let position = MotorPosition {
        direction: Direction::Forward,
        e801_bit2: false,
        steps,
    };
    let start = Instant::now();
    let base = park_registers(&link.registers);
    let finished = chip.motor_move(&base, &profile.park, &position)?;
    println!(
        "moved            {steps} steps in {:.1} s{}",
        start.elapsed().as_secs_f32(),
        if finished {
            ""
        } else {
            " (still running after 30 s)"
        }
    );
    println!(
        "at home now      {}",
        if chip.head_is_at_home()? { "yes" } else { "no" }
    );
    Ok(())
}

fn scan(
    scanner: Scanner,
    output: &std::path::Path,
    dpi: u16,
    area: hp3770_core::geometry::Area,
    linear: bool,
) -> CliResult {
    use hp3770_core::geometry;
    let profile = &hp3770_core::profile::HP3770;
    let (params, hw) = geometry::scan_window(dpi, area)?;
    let gamma = geometry::identity_gamma();

    let start = Instant::now();
    let mut chip = Chip::new(scanner);
    let link = chip.link(profile)?;
    let mut init = link.registers.clone();
    if !link.at_home {
        chip.head_park_home(&hp3770_core::motor::park_registers(&init), &profile.park)?;
    }
    chip.prepare_scan(&mut init, profile)?;
    if !link.lamp_was_on {
        // The start-up switched the lamp on: let it warm up.
        eprintln!(
            "lamp             was off, warming up for {} s",
            hp3770_core::scan::LAMP_WARMUP.as_secs()
        );
        std::thread::sleep(hp3770_core::scan::LAMP_WARMUP);
    }
    let calibration = chip.calibrate(&mut init, profile, &params)?;
    eprintln!(
        "calibrated in    {:.1} s (offsets {:?}, gains {:?})",
        start.elapsed().as_secs_f32(),
        calibration.gain_offset.even_offset,
        calibration.gain_offset.gain
    );
    let average = |table: &[u8]| {
        let words: Vec<f64> = table
            .chunks(2)
            .map(|w| f64::from(u16::from_le_bytes([w[0], w[1]])))
            .collect();
        words.iter().sum::<f64>() / words.len().max(1) as f64
    };
    eprintln!(
        "shading          black {:.1} {:.1} {:.1}, white x{:.3} x{:.3} x{:.3}",
        average(&calibration.black[0]) / 256.0,
        average(&calibration.black[1]) / 256.0,
        average(&calibration.black[2]) / 256.0,
        average(&calibration.white[0]) / 8192.0,
        average(&calibration.white[1]) / 8192.0,
        average(&calibration.white[2]) / 8192.0,
    );
    let tables = calibration.tables(Some(&gamma));
    let mut job = chip.start_scan(
        &init,
        profile,
        &params,
        &hw,
        &calibration.gain_offset,
        &tables,
    )?;
    let mut image = Vec::with_capacity(job.total());
    let mut piece = Vec::new();
    while chip.read_scan(&mut job, &mut piece)? > 0 {
        image.extend_from_slice(&piece);
    }
    let parked = chip.stop_scan(&mut init, profile)?;
    // The chip brings the carriage back by itself below 600 dpi; make sure.
    chip.head_park_home(&hp3770_core::motor::park_registers(&init), &profile.park)?;
    let mut pixels = job.bytes_per_line / 3;
    let mut lines = image.len() / job.bytes_per_line;
    image.truncate(lines * job.bytes_per_line);
    if hw.arrange_line == hp3770_core::setup::ArrangeLine::Software {
        // 2400 dpi: lines arranged here, width doubled.
        use hp3770_core::arrange;
        let d = arrange::Distances::of(profile, &params, hw.high_resolution);
        lines = params.height as usize;
        image = arrange::double_width(&arrange::arrange(&image, pixels, lines, &d), pixels);
        pixels *= 2;
    }
    if !linear {
        let lut = geometry::host_gamma(2.2);
        for sample in &mut image {
            *sample = lut[usize::from(*sample)];
        }
    }
    save_image(output, pixels, lines, &image)?;
    println!(
        "scanned          {pixels} x {lines} pixels in {:.1} s{}",
        start.elapsed().as_secs_f32(),
        if parked { " (parked)" } else { "" }
    );
    println!("saved            {}", output.display());
    Ok(())
}

/// Writes an 8-bit RGB image as PNG, or as binary PPM if the name ends in
/// `.ppm`.
fn save_image(path: &std::path::Path, width: usize, height: usize, rgb: &[u8]) -> CliResult {
    let ppm = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ppm"));
    if ppm {
        let mut file = format!("P6\n{width} {height}\n255\n").into_bytes();
        file.extend_from_slice(rgb);
        std::fs::write(path, file)?;
        return Ok(());
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, u32::try_from(width)?, u32::try_from(height)?);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgb)?;
    Ok(())
}

fn lamp(scanner: Scanner, state: LampState) -> CliResult {
    let mut chip = Chip::new(scanner);
    // The block kept in memory by a long-running driver; this one-shot
    // command has none, and lamp_flatbed reads the chip anyway.
    let mut init = hp3770_core::init::Registers::default();
    chip.lamp_flatbed(&mut init, state == LampState::On)?;
    println!("lamp             {}", chip.lamp_status()?);
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
        assert!(Cli::try_parse_from(["hp3770", "forward", "601"]).is_err());
        let cli = Cli::parse_from(["hp3770", "forward", "300"]);
        assert!(matches!(cli.command, Command::Forward { steps: 300 }));
        let cli = Cli::parse_from(["hp3770", "park"]);
        assert!(matches!(cli.command, Command::Park));
        let cli = Cli::parse_from(["hp3770", "lamp", "off"]);
        assert!(matches!(
            cli.command,
            Command::Lamp {
                state: LampState::Off
            }
        ));
    }
}
