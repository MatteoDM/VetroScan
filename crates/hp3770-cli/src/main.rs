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
    /// Experimental: scan slides or negatives with the transparency adapter.
    ///
    /// Switches the adapter's lamp on and lets it warm up for 30 s,
    /// calibrates on the slot without film at the top of the adapter, and
    /// scans the adapter's whole area in colour at 16 bits. Saves the
    /// picture (NAME.png) and the chip's samples (NAME-16bit.png). With
    /// --film the picture is the positive (`scan_imaging::film`), measured
    /// on the frames found in the holder (`film::frames`), scanned with the
    /// negatives' longer line (600 and 1200 dpi only); without, the light
    /// as it came through, with the glass's line.
    /// Remove the lid's white backing first.
    TmaScan {
        /// Output file (PNG, or PPM if the name ends in .ppm).
        output: PathBuf,
        /// Resolution: 200, 300, 600, 1200 or 2400 dpi.
        #[arg(long, default_value_t = 200)]
        dpi: u16,
        /// What is in the holder.
        #[arg(long, value_enum)]
        film: Option<FilmArg>,
        /// For measuring the adapter's light: calibrate at this distance
        /// from the top of the glass, in mm, instead of on the slot without
        /// film (with the holder empty, the middle of its window).
        #[arg(long)]
        calibrate_at: Option<f64>,
        /// For measuring where the adapter's light ends: scan this many mm
        /// down from the top of the adapter's area instead of its height
        /// (at most 130: the glass, never past it).
        #[arg(long, value_parser = parse_height)]
        height: Option<f64>,
    },
    /// Turn a saved scan of the transparency adapter (the NAME-16bit.png of
    /// `tma-scan`) into its picture again, without the scanner.
    Develop {
        /// The 16-bit samples (NAME-16bit.png).
        input: PathBuf,
        /// The picture to write.
        output: PathBuf,
        /// Resolution of the scan.
        #[arg(long, default_value_t = 1200)]
        dpi: u16,
        /// What is in the holder.
        #[arg(long, value_enum)]
        film: Option<FilmArg>,
        /// Take the scanner's stripes away first (on the linear samples,
        /// before the film is measured and turned into its picture).
        #[arg(long)]
        destripe: bool,
        /// Smooth the colour's noise of the picture, as much as measured on
        /// it (`scan_imaging::chroma`).
        #[arg(long)]
        colour_noise: bool,
        /// A negative's base as given, transmission per channel (0-1, red
        /// green blue, as "negative base" prints it): a roll's base locked,
        /// instead of the one between this strip's frames.
        #[arg(long, num_args = 3, value_delimiter = ',', value_names = ["R", "G", "B"])]
        base: Option<Vec<f32>>,
        /// A 3x3 colour matrix applied to the picture, row by row (red out
        /// from red green blue in, then green, then blue), before the
        /// colour noise: a trial of a colour correction, nothing measured.
        #[arg(long, num_args = 9, allow_negative_numbers = true)]
        matrix: Option<Vec<f32>>,
        /// The gamma of the values the matrix works on: 1 on the linear
        /// picture, 2.2 on the picture as displayed.
        #[arg(long, default_value_t = 1.0)]
        matrix_gamma: f32,
    },
    /// Switch the flatbed lamp, or the transparency adapter's, on or off.
    ///
    /// Writes E946 and E954 like the HP driver (E965 and E946 for the
    /// adapter). One lamp at a time: switching one on switches the other
    /// off. The flatbed lamp also switches off by itself after the timer
    /// set by `init` (13 minutes).
    Lamp {
        /// on or off.
        #[arg(value_enum)]
        state: LampState,
        /// The transparency adapter's lamp (in the lid) instead of the
        /// flatbed lamp.
        #[arg(long)]
        tma: bool,
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
        /// Colour, grey (8 bits) or lineart (black and white, 1 bit).
        #[arg(long, value_enum, default_value_t = Mode::Colour)]
        mode: Mode,
        /// Lineart threshold, as the HP driver's: white from 255 minus this.
        #[arg(long, default_value_t = scan_imaging::convert::DEFAULT_THRESHOLD)]
        threshold: u8,
        /// Save the chip's linear samples, without the tone curve the HP
        /// driver applies on the host (white at 196, gamma 2.2).
        #[arg(long)]
        linear: bool,
        /// Let the carriage travel to the area at scan speed (no fast
        /// move first; for comparisons).
        #[arg(long)]
        no_fast_travel: bool,
        /// Sharpening as the HP driver's; 0 for none.
        #[arg(long, default_value_t = scan_imaging::convert::HP_SHARPEN)]
        sharpen: f32,
        /// Bits per sample asked of the chip (16: saves
        /// also NAME-16bit.png with the samples as sent, and prints how
        /// many distinct low bytes they have).
        #[arg(long, default_value_t = 8)]
        depth: u8,
    },
    /// Find the originals on a preview of the glass (a PNG scanned at 200
    /// dpi, as `scan --dpi 200 --linear`) and print where they are.
    Detect {
        /// The preview.
        input: PathBuf,
        /// Only this part: left, top, width, height in pixels.
        #[arg(long, value_delimiter = ',')]
        crop: Option<Vec<usize>>,
    },
    /// Measure the stripes across a scan of something uniform (a white
    /// sheet or the lid, as `scan --linear --sharpen 0`): every how many
    /// lines they repeat and how strong they are, before and after
    /// VetroScan takes them away.
    Stripes {
        /// The scan (PNG, 8-bit RGB).
        input: PathBuf,
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
enum Mode {
    Colour,
    Grey,
    Lineart,
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
        Command::Lamp { state, tma } => open(cli.backend).and_then(|s| lamp(s, state, tma)),
        Command::Probe { output } => open(cli.backend).and_then(|s| probe(s, output)),
        Command::Scan {
            output,
            dpi,
            left,
            top,
            width,
            height,
            mode,
            threshold,
            linear,
            sharpen,
            no_fast_travel,
            depth,
        } => {
            let area = hp3770_core::geometry::Area::from_mm(dpi, left, top, width, height);
            scan(
                cli.backend,
                &output,
                dpi,
                area,
                (mode, threshold, linear, sharpen, !no_fast_travel),
                depth,
            )
        }
        Command::TmaScan {
            output,
            dpi,
            film,
            calibrate_at,
            height,
        } => open(cli.backend).and_then(|s| tma_scan(s, &output, dpi, film, calibrate_at, height)),
        Command::Develop {
            input,
            output,
            dpi,
            film,
            destripe,
            colour_noise,
            base,
            matrix,
            matrix_gamma,
        } => read_png16(&input).and_then(|(w, h, mut samples)| {
            if destripe {
                destripe_samples(&mut samples, w, h, dpi)?;
            }
            let base = match base.as_deref() {
                None => None,
                Some(&[r, g, b]) if [r, g, b].iter().all(|v| *v > 0.0 && *v <= 1.0) => {
                    Some([r, g, b])
                }
                Some(_) => return Err("--base wants three transmissions from 0 to 1".into()),
            };
            let matrix = match matrix.as_deref() {
                None => None,
                Some(m) if m.len() == 9 && matrix_gamma > 0.0 => {
                    Some((std::array::from_fn(|i| m[i]), matrix_gamma))
                }
                Some(_) => return Err("--matrix wants nine numbers, --matrix-gamma above 0".into()),
            };
            develop(
                samples,
                w,
                h,
                dpi,
                film,
                colour_noise,
                base,
                matrix,
                &output,
            )
        }),
        Command::Detect { input, crop } => detect(&input, crop.as_deref()),
        Command::Stripes { input } => stripes(&input),
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
    println!("TMA lamp lit       {}", yes_no(s.tma_lamp_lit));
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
    backend: BackendArg,
    output: &std::path::Path,
    dpi: u16,
    area: hp3770_core::geometry::Area,
    (mode, threshold, linear, sharpen, fast_travel): (Mode, u8, bool, f32, bool),
    depth: u8,
) -> CliResult {
    use scanner_api::{Progress, ScanRequest};
    // Through the registry, like the application; the driver itself when a
    // USB backend is chosen or the fast travel is off.
    let mut scanner: Box<dyn scanner_api::Scanner> = if backend == BackendArg::Auto && fast_travel {
        let scanner = scanner_api::Registry::new()
            .with(hp3770_usb::UsbBackend)
            .open_first()?;
        eprintln!("Opened the {}", scanner.capabilities().name());
        scanner
    } else {
        Box::new(
            hp3770_core::rts8822::Rts8822::new(open(backend)?, &hp3770_core::profile::HP3770)
                .fast_travel(fast_travel),
        )
    };
    let caps = scanner.capabilities().clone();
    let request = ScanRequest {
        dpi,
        source: scanner_api::Source::Glass,
        area: scanner_api::Area {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
        },
        depth,
    };
    let start = Instant::now();
    let mut last_percent = None;
    let result = scanner.scan(&request, &mut |progress| {
        match progress {
            Progress::Starting => eprintln!("starting"),
            Progress::WarmingUp => eprintln!(
                "lamp             was off, warming up for {} s",
                caps.lamp.map_or(0, |l| l.warm_up.as_secs())
            ),
            Progress::Calibrating => eprintln!("calibrating"),
            Progress::Scanning { done, total } => {
                let percent = done * 100 / total.max(1) / 10 * 10;
                if last_percent != Some(percent) {
                    last_percent = Some(percent);
                    eprintln!("scanning         {percent} %");
                }
            }
            Progress::Finishing => eprintln!("finishing"),
        }
        true
    })?;
    for (label, value) in &result.details {
        eprintln!("{label:<17}{value}");
    }
    // The scanner gives its linear samples: the tone curve, the grey or
    // black and white and the sharpening are done here.
    let raw = &result.image;
    let image = scan_imaging::adjust::render(
        &raw.data,
        raw.width,
        raw.height,
        raw.dpi,
        match mode {
            Mode::Colour => scan_imaging::Mode::Colour,
            Mode::Grey => scan_imaging::Mode::Grey,
            Mode::Lineart => scan_imaging::Mode::Lineart { threshold },
        },
        &scan_imaging::adjust::Adjustments {
            sharpen,
            ..Default::default()
        },
        linear,
        &caps.tone,
    );
    if let Some(samples) = &result.samples16 {
        save_png16(output, image.width, image.height, samples)?;
    }
    save_image(output, image.width, image.height, &image.data, image.format)?;
    println!(
        "scanned          {} x {} pixels in {:.1} s",
        image.width,
        image.height,
        start.elapsed().as_secs_f32()
    );
    println!("saved            {}", output.display());
    Ok(())
}

/// Saves the 16-bit samples next to `output` (NAME-16bit.png) and prints
/// what their low bytes look like: all zero (or few values) means the
/// chip only widened 8-bit samples.
fn save_png16(output: &std::path::Path, width: usize, height: usize, samples: &[u16]) -> CliResult {
    let mut low = [0u64; 256];
    for &v in samples {
        low[usize::from(v & 0xff)] += 1;
    }
    let distinct = low.iter().filter(|&&n| n > 0).count();
    let mean = samples.iter().map(|&v| f64::from(v)).sum::<f64>() / samples.len().max(1) as f64;
    let max = samples.iter().copied().max().unwrap_or(0);
    eprintln!(
        "16 bit           {} samples, mean {mean:.0}, max {max}, {distinct} distinct low bytes",
        samples.len()
    );
    let stem = output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scan");
    let path = output.with_file_name(format!("{stem}-16bit.png"));
    let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
    let mut encoder = png::Encoder::new(file, u32::try_from(width)?, u32::try_from(height)?);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header()?;
    let bytes: Vec<u8> = samples
        .iter()
        .take(width * height * 3)
        .flat_map(|v| v.to_be_bytes())
        .collect();
    writer.write_image_data(&bytes)?;
    eprintln!("saved            {}", path.display());
    Ok(())
}

/// Reads a 16-bit RGB PNG (as [`save_png16`] writes it): width, height and
/// the samples.
fn read_png16(
    path: &std::path::Path,
) -> Result<(usize, usize, Vec<u16>), Box<dyn std::error::Error>> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    if info.bit_depth != png::BitDepth::Sixteen || info.color_type != png::ColorType::Rgb {
        return Err(format!("{} is not a 16-bit RGB PNG", path.display()).into());
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("PNG too large")?];
    reader.next_frame(&mut buf)?;
    let samples = buf[..w * h * 6]
        .chunks_exact(2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .collect();
    Ok((w, h, samples))
}

/// Writes an image as PNG, or as binary PNM (PPM, PGM or PBM) if the name
/// ends in `.ppm`, `.pgm` or `.pbm`.
fn save_image(
    path: &std::path::Path,
    width: usize,
    height: usize,
    data: &[u8],
    format: scan_imaging::Format,
) -> CliResult {
    use scan_imaging::Format;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("ppm" | "pgm" | "pbm")) {
        let (magic, body) = match format {
            Format::Rgb => ("P6", data.to_vec()),
            Format::Grey => ("P5", data.to_vec()),
            // PBM has 1 = black.
            Format::Lineart => ("P4", data.iter().map(|b| !b).collect()),
        };
        let header = if format == Format::Lineart {
            format!("{magic}\n{width} {height}\n")
        } else {
            format!("{magic}\n{width} {height}\n255\n")
        };
        let mut file = header.into_bytes();
        file.extend_from_slice(&body);
        std::fs::write(path, file)?;
        return Ok(());
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, u32::try_from(width)?, u32::try_from(height)?);
    let (color, depth) = match format {
        Format::Rgb => (png::ColorType::Rgb, png::BitDepth::Eight),
        Format::Grey => (png::ColorType::Grayscale, png::BitDepth::Eight),
        Format::Lineart => (png::ColorType::Grayscale, png::BitDepth::One),
    };
    encoder.set_color(color);
    encoder.set_depth(depth);
    encoder.write_header()?.write_image_data(data)?;
    Ok(())
}

/// Takes the HP 3770's stripes away from linear 16-bit samples of a film,
/// with the repeat its profile declares for the adapter at `dpi`.
fn destripe_samples(samples: &mut [u16], width: usize, height: usize, dpi: u16) -> CliResult {
    let repeat = hp3770_core::profile::HP3770
        .transparency
        .as_ref()
        .ok_or("this scanner has no transparency adapter")?
        .stripes
        .iter()
        .find(|&&(d, _)| d == dpi)
        .map(|&(_, r)| usize::from(r))
        .ok_or_else(|| format!("no stripes known at {dpi} dpi"))?;
    let mut image = scan_imaging::Image {
        width,
        height,
        dpi,
        format: scan_imaging::Format::Rgb,
        data: samples.iter().map(|&v| (v >> 8) as u8).collect(),
    };
    let done = scan_imaging::stripes::remove(&mut image, Some(samples), repeat);
    eprintln!(
        "stripes          {} (every {repeat} lines)",
        if done {
            "removed"
        } else {
            "image too small, left"
        }
    );
    Ok(())
}

/// What a scan with the transparency adapter becomes: its 16-bit samples
/// (`width` x `height` RGB, the adapter's area from its top left corner,
/// at `dpi`), measured and turned into the picture with `film` (a
/// negative from its `base` if given), saved as `output`. Shared by
/// `tma-scan` and `develop`.
#[allow(clippy::too_many_arguments)]
fn develop(
    samples: Vec<u16>,
    width: usize,
    height: usize,
    dpi: u16,
    film: Option<FilmArg>,
    colour_noise: bool,
    given: Option<[f32; 3]>,
    matrix: Option<([f32; 9], f32)>,
    output: &std::path::Path,
) -> CliResult {
    use scan_imaging::adjust::{render_samples, Adjustments, Samples};
    use scan_imaging::film::{self, Film, Negative, Slide};
    let profile = &hp3770_core::profile::HP3770;
    let adapter = profile
        .transparency
        .as_ref()
        .ok_or("this scanner has no transparency adapter")?;
    // The linear picture at 8 bits, for finding the frames.
    let image = scan_imaging::Image {
        width,
        height,
        dpi,
        format: scan_imaging::Format::Rgb,
        data: samples.iter().map(|&v| (v >> 8) as u8).collect(),
    };
    // How much light the calibration made of the slot without film (its
    // middle third, 65535 = the lamp at full scale).
    let px = |tenths: u16| (f64::from(tenths) / 254.0 * f64::from(dpi)) as usize;
    let rows = px(adapter.slot[1].saturating_sub(adapter.area[1]))
        ..px((adapter.slot[1] + adapter.slot[3]).saturating_sub(adapter.area[1]));
    let cols = image.width / 3..image.width * 2 / 3;
    let mut slot = [0u64; 3];
    let mut n = 0u64;
    for y in rows.start + (rows.end - rows.start) / 3..rows.end - (rows.end - rows.start) / 3 {
        for x in cols.clone() {
            let i = (y.min(image.height - 1) * image.width + x) * 3;
            for (c, s) in slot.iter_mut().enumerate() {
                *s += u64::from(samples[i + c]);
            }
            n += 1;
        }
    }
    eprintln!("slot             {:?} of 65535", slot.map(|s| s / n.max(1)));
    // The film: the frames found in the holder, else everything below the
    // slot.
    let kind = match film {
        Some(FilmArg::Slide) => Film::Slide,
        _ => Film::Negative,
    };
    let frames = film::frames(&image, kind);
    eprintln!("frames found     {}", frames.len());
    for f in &frames {
        let mm = |px: f32| px * 25.4 / f32::from(dpi);
        eprintln!(
            "  frame          centre {:.1} x {:.1} mm, {:.1} x {:.1} mm, {:.1} degrees",
            mm(f.cx),
            mm(f.cy),
            mm(f.width),
            mm(f.height),
            f.angle
        );
    }
    // The base in the gaps between frames; the levels inside the frames,
    // a twentieth in from each side (away from the holder's edges).
    let gaps = film::base_between(&samples, image.width, &frames);
    eprintln!("base in the gaps {gaps:.3?}");
    let base = given.or(gaps);
    let film_part: Vec<u16> = if frames.is_empty() {
        let skip = (rows.end.min(image.height) * image.width * 3).min(samples.len());
        samples[skip..].to_vec()
    } else {
        film::inside(&samples, image.width, &frames)
    };
    let film_part = &film_part[..];
    let mut picture = match film {
        None => samples,
        Some(FilmArg::Negative) => {
            let n = Negative::measure(film_part, base).ok_or("too little film in the scan")?;
            eprintln!(
                "negative         base {:.3?}, highlights {:.2?}, black {:.2}, range {:.1}",
                n.base,
                n.high,
                n.black,
                n.scene_range()
            );
            n.positive(&samples)
        }
        Some(FilmArg::Slide) => {
            let s = Slide::measure(film_part).ok_or("too little film in the scan")?;
            eprintln!("slide            black {}, white {}", s.black, s.white);
            s.positive(&samples)
        }
    };
    if let Some((m, gamma)) = matrix {
        apply_matrix(&mut picture, m, gamma);
        eprintln!("colour matrix    {m:?} at gamma {gamma}");
    }
    if colour_noise {
        let noise = scan_imaging::chroma::estimate(&picture, image.width);
        let radius = noise.and_then(scan_imaging::chroma::radius_for);
        if let Some(r) = radius {
            scan_imaging::chroma::reduce(&mut picture, image.width, r);
        }
        eprintln!("colour noise     {noise:?}, smoothed {radius:?} px");
    }
    let shown = render_samples(
        Samples::Sixteen(&picture),
        image.width,
        image.height,
        dpi,
        scan_imaging::Mode::Colour,
        &Adjustments {
            sharpen: 0.0,
            ..Default::default()
        },
        false,
        &film::TONE,
    );
    save_image(output, shown.width, shown.height, &shown.data, shown.format)?;
    println!("picture          {} x {} pixels", shown.width, shown.height);
    println!("saved            {}", output.display());
    Ok(())
}

/// `develop --matrix`: each pixel of the linear 16-bit `picture` raised to
/// 1 / `gamma`, multiplied by the 3x3 matrix `m` (row by row), clipped and
/// brought back to linear.
fn apply_matrix(picture: &mut [u16], m: [f32; 9], gamma: f32) {
    let full = f32::from(u16::MAX);
    let encode: Vec<f32> = (0..=u16::MAX)
        .map(|v| (f32::from(v) / full).powf(1.0 / gamma))
        .collect();
    for p in picture.chunks_exact_mut(3) {
        let v = [0, 1, 2].map(|c| encode[usize::from(p[c])]);
        for (c, out) in p.iter_mut().enumerate() {
            let x = m[c * 3] * v[0] + m[c * 3 + 1] * v[1] + m[c * 3 + 2] * v[2];
            *out = (x.clamp(0.0, 1.0).powf(gamma) * full).round() as u16;
        }
    }
}

/// What is in the adapter's holder (`tma-scan --film`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FilmArg {
    /// Slides: positives.
    Slide,
    /// Negatives, colour or black and white.
    Negative,
}

/// `--height` of `tma-scan`: 10-130 mm, so that the area stays on the glass.
fn parse_height(text: &str) -> Result<f64, String> {
    let mm: f64 = text.parse().map_err(|e| format!("{e}"))?;
    if (10.0..=130.0).contains(&mm) {
        Ok(mm)
    } else {
        Err("from 10 to 130 mm".into())
    }
}

fn tma_scan(
    scanner: Scanner,
    output: &std::path::Path,
    dpi: u16,
    film: Option<FilmArg>,
    calibrate_at: Option<f64>,
    height: Option<f64>,
) -> CliResult {
    use hp3770_core::geometry::Area;
    use hp3770_core::session::{self, Request, Stage, TMA_WARMUP};
    use scan_imaging::adjust::Adjustments;
    let mut moved = hp3770_core::profile::HP3770.clone();
    if let (Some(mm), Some(adapter)) = (calibrate_at, moved.transparency.as_mut()) {
        // The glass starts 69 lines from home at 200 dpi (8.76 mm); the
        // motor makes 1200 steps per inch.
        let steps = (mm + 69.0 * 25.4 / 200.0) / 25.4 * f64::from(moved.platform.motor_resolution);
        adapter.strip_steps = steps.round().clamp(0.0, f64::from(u16::MAX)) as u16;
        eprintln!(
            "calibrating at   {mm:.1} mm ({} steps from home)",
            adapter.strip_steps
        );
    }
    let profile = &moved;
    let adapter = profile
        .transparency
        .as_ref()
        .ok_or("this scanner has no transparency adapter")?;
    let mut chip = Chip::new(scanner);
    if !chip.status()?.tma_attached {
        return Err("the transparency adapter is not plugged in".into());
    }
    let mm = |tenths: u16| f64::from(tenths) / 10.0;
    let [left, top, width, area_height] = adapter.area.map(mm);
    let area_height = height.unwrap_or(area_height);
    if height.is_some() {
        eprintln!("area             {top:.1}-{:.1} mm down", top + area_height);
    }
    let area = Area::from_mm(dpi, left, top, width, area_height);
    let request = Request {
        linear: true,
        transparency: true,
        // Films with the negatives' longer line, slides too (as the driver).
        negative: film.is_some(),
        depth: 16,
        adjust: Adjustments {
            sharpen: 0.0,
            ..Default::default()
        },
        ..Request::new(dpi, area, scan_imaging::Mode::Colour)
    };
    let start = Instant::now();
    let mut calibrating = false;
    let mut last_percent = None;
    let (image, report) = session::scan(&mut chip, profile, &request, &mut |stage| {
        match stage {
            Stage::Starting => eprintln!("starting"),
            Stage::WarmingUp => eprintln!(
                "TMA lamp         on, warming up for {} s",
                TMA_WARMUP.as_secs()
            ),
            Stage::Calibrating if !calibrating => {
                calibrating = true;
                eprintln!("calibrating      (adapter's lamp, slot without film)");
            }
            Stage::Calibrating => {}
            Stage::Scanning { done, total } => {
                let percent = done * 100 / total.max(1) / 10 * 10;
                if last_percent != Some(percent) {
                    last_percent = Some(percent);
                    eprintln!("scanning         {percent} %");
                }
            }
            Stage::Finishing => eprintln!("finishing        (adapter's lamp off)"),
        }
        true
    })?;
    eprintln!(
        "calibration      offsets {:?}, gains {:?}",
        report.offsets, report.gains
    );
    let samples = report.samples16.ok_or("no 16-bit samples")?;
    save_png16(output, image.width, image.height, &samples)?;
    develop(
        samples,
        image.width,
        image.height,
        dpi,
        film,
        false,
        None,
        None,
        output,
    )?;
    println!("scanned in       {:.1} s", start.elapsed().as_secs_f32());
    Ok(())
}

fn lamp(scanner: Scanner, state: LampState, tma: bool) -> CliResult {
    let mut chip = Chip::new(scanner);
    // The block kept in memory by a long-running driver; this one-shot
    // command has none, and the lamp functions read the chip anyway.
    let mut init = hp3770_core::init::Registers::default();
    let on = state == LampState::On;
    if tma {
        if !chip.status()?.tma_attached {
            return Err("the transparency adapter is not plugged in".into());
        }
        chip.lamp_transparency(&mut init, on)?;
    } else {
        chip.lamp_flatbed(&mut init, on)?;
    }
    let s = chip.status()?;
    println!("lamp             {}", s.lamp);
    println!(
        "TMA lamp lit     {}",
        if s.tma_lamp_lit { "yes" } else { "no" }
    );
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

/// Prints the stripes measured on a scan of something uniform.
fn stripes(input: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use scan_imaging::{Format, Image};
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(input)?));
    let mut reader = decoder.read_info()?;
    let mut data = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let info = reader.next_frame(&mut data)?;
    data.truncate(info.buffer_size());
    if (info.color_type, info.bit_depth) != (png::ColorType::Rgb, png::BitDepth::Eight) {
        return Err("an 8-bit RGB PNG is needed".into());
    }
    let image = Image {
        width: info.width as usize,
        height: info.height as usize,
        dpi: 0,
        format: Format::Rgb,
        data,
    };
    match scan_imaging::stripes::measure(&image, None) {
        Some(m) => println!(
            "stripes          repeat every {} lines, {:.2} % row to row, {:.2} % after removal",
            m.repeat,
            m.strength * 100.0,
            m.left * 100.0
        ),
        None => println!("stripes          none worth removing"),
    }
    Ok(())
}

/// Prints the originals found on a preview.
fn detect(
    input: &std::path::Path,
    crop: Option<&[usize]>,
) -> Result<(), Box<dyn std::error::Error>> {
    use scan_imaging::{Format, Image};
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(input)?));
    let mut reader = decoder.read_info()?;
    let mut data = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("immagine troppo grande")?
    ];
    let info = reader.next_frame(&mut data)?;
    data.truncate(info.buffer_size());
    let format = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgb, png::BitDepth::Eight) => Format::Rgb,
        (png::ColorType::Grayscale, png::BitDepth::Eight) => Format::Grey,
        other => return Err(format!("formato non gestito: {other:?}").into()),
    };
    let image = Image {
        width: info.width as usize,
        height: info.height as usize,
        dpi: 200,
        format,
        data,
    };
    let image = match crop {
        Some(&[x, y, w, h]) => image.cropped(x, y, w, h),
        _ => image,
    };
    let found = scan_imaging::detect::objects(&image);
    let mm = 25.4 / 200.0;
    println!("{} originals", found.len());
    for (i, f) in found.iter().enumerate() {
        let b = f.bounds();
        println!(
            "{}: centre {:.1}, {:.1} mm, {:.1} x {:.1} mm, turned {:.1} deg (box {:.0},{:.0} {:.0}x{:.0} px)",
            i + 1,
            f.cx * mm,
            f.cy * mm,
            f.width * mm,
            f.height * mm,
            f.angle,
            b[0],
            b[1],
            b[2],
            b[3]
        );
    }
    Ok(())
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
                state: LampState::Off,
                tma: false
            }
        ));
        let cli = Cli::parse_from(["hp3770", "lamp", "on", "--tma"]);
        assert!(matches!(
            cli.command,
            Command::Lamp {
                state: LampState::On,
                tma: true
            }
        ));
    }
}
