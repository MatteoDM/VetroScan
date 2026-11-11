// SPDX-License-Identifier: GPL-2.0-or-later
//! `epson4490`: the Epson Perfection 4490 from the command line, for its
//! first days on the desk (phase 12). But for `replay` (below), nothing it
//! sends moves the carriage: every write goes through the backend's guard.
//!
//! ```text
//! epson4490 list                       the scanners found, nothing sent
//! epson4490 firmware [--firmware F]    which esfw54.bin would be sent, no scanner needed
//! epson4490 probe                      list, firmware, then the state (1B 03) of the first
//! epson4490 upload [--firmware F]      sends the firmware if it is not running
//! epson4490 buttons [SECONDS]          reads the buttons (1E 85) for a while, 10 s by default
//! epson4490 replay SCRIPT OUT --move   replays a script made from a capture of Epson's driver
//! ```
//!
//! `replay` is the one command that moves the carriage, so it asks for `--move`. It sends only
//! the bytes of its script (made by `captures/epson4490/tools/script.py` from a capture: what
//! Epson's driver sent), checks every answer against the capture and stops at the first that
//! differs; the image data read goes to OUT. A script made with `--shading` has its calibration
//! marked: the replay then computes the black pixels' `1E 42`, the `1E 42` with K and the
//! minimum dark, the scales in byte 15 of `1E 41` and the shading table from its own
//! calibration reads, in place of the capture's. It does not go through the
//! guard (the script is the list of what may go out), on a scanner opened like the others (on
//! Windows with the STI lock).

use epson_gt_x750::guard::Guarded;
use epson_gt_x750::protocol::{self, FirmwareOutcome, State};
use epson_gt_x750::{firmware, Found, Transport, PRODUCT_ID, VENDOR_ID};
use scanner_api::Error;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: epson4490 list | firmware [--firmware FILE] | probe |                      upload [--firmware FILE] | buttons [SECONDS] | \
                     replay SCRIPT OUT --move";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut firmware_path = None;
    let mut command = None;
    let mut seconds = None;
    let mut operands: Vec<String> = Vec::new();
    let mut moves = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--firmware" => match rest.next() {
                Some(path) => firmware_path = Some(PathBuf::from(path)),
                None => return usage(),
            },
            "--move" => moves = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            c if command.is_none() && !c.starts_with('-') => command = Some(c.to_owned()),
            o if command.as_deref() == Some("replay") && operands.len() < 2 => {
                operands.push(o.to_owned())
            }
            n if command.as_deref() == Some("buttons") && seconds.is_none() => {
                match n.parse::<u64>() {
                    Ok(n) => seconds = Some(n),
                    Err(_) => return usage(),
                }
            }
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
        Some("buttons") => buttons(seconds.unwrap_or(10)),
        Some("replay") if operands.len() == 2 && moves => replay(&operands[0], &operands[1]),
        Some("replay") if operands.len() == 2 => {
            eprintln!("epson4490: replay moves the carriage: add --move to run it");
            return ExitCode::from(2);
        }
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
                println!("opened {} {} ({t:?})", f.link, f.location);
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

/// The button names, in the order of the backend's capabilities.
const BUTTONS: [&str; 4] = ["Start", "Scan to PDF", "Scan to E-mail", "Copy"];

fn buttons(seconds: u64) -> Result<(), Error> {
    let mut t = open_first()?;
    println!("state (1B 03): {}", describe(protocol::state(&mut t)?));
    println!("reading the buttons (1E 85) for {seconds} s: press them");
    let end = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut last = 0;
    while std::time::Instant::now() < end {
        let n = protocol::button(&mut t)?;
        if n != last {
            let name = match n {
                0 => "none",
                1..=4 => BUTTONS[usize::from(n) - 1],
                _ => "unknown",
            };
            println!("1E 85 -> {n:02X} ({name})");
            last = n;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Ok(())
}

/// One step of a replay script (see `captures/epson4490/tools/script.py`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Write these bytes.
    Write(Vec<u8>),
    /// Read exactly these bytes.
    Read(Vec<u8>),
    /// Read this many bytes, any content (the capture's are shown when they differ).
    Any(Vec<u8>),
    /// Read this many bytes of image data.
    Data(usize),
    /// Write the command, read the answer's length, again until it is the answer.
    Poll(Vec<u8>, Vec<u8>, u64),
    /// Write the answer of the last `1E 92`.
    WriteBack,
    /// What the following image data is: a calibration read, or none.
    Calibration(Option<Cal>),
    /// The `1E 42` block before the white calibration: the black pixels' means, computed.
    Block42Black(Vec<u8>),
    /// The `1E 41` block before the shading table: its byte 15 becomes the computed scales.
    Block41(Vec<u8>),
    /// The `1E 42` block with K and the minimum dark: computed.
    Block42(Vec<u8>),
    /// The shading table (`1E 84` type 05): computed, as long as this one, as wide as its
    /// non-zero part.
    Table(Vec<u8>),
}

/// The calibration reads a replay keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cal {
    /// The sensor's black pixels (4 lines, a few columns).
    Black,
    /// The white strip, lamp on.
    White,
    /// The same, lamp off.
    Dark,
}

/// The `1E 42` block before the white calibration: zero, then each channel's mean of the
/// black pixels twice (`raw`: 16-bit RGB, possibly ending with the 8-byte block trailer).
fn black_block(raw: &[u8]) -> Vec<u8> {
    let n = raw.len();
    let raw = if n >= 8 && raw[n - 2..] == [0x80, 0x00] && raw[n - 8] == 0x02 {
        &raw[..n - 8]
    } else {
        raw
    };
    let mut sum = [0f64; 3];
    let mut count = 0usize;
    for p in raw.chunks_exact(6) {
        for (ch, s) in sum.iter_mut().enumerate() {
            *s += f64::from(u16::from_le_bytes([p[2 * ch], p[2 * ch + 1]]));
        }
        count += 1;
    }
    let mean: [u16; 3] = std::array::from_fn(|ch| (sum[ch] / count.max(1) as f64).floor() as u16);
    [
        0, 0, 0, mean[0], mean[0], mean[1], mean[1], mean[2], mean[2],
    ]
    .iter()
    .flat_map(|v: &u16| v.to_le_bytes())
    .collect()
}

/// The shading computed from a replay's own calibration (`captures/epson4490/06-formato-dati.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shading {
    /// The white's base per channel (`1E 42`).
    k: [u16; 3],
    /// The minimum dark per channel (`1E 42`).
    dark_min: [u16; 3],
    /// The white's scale, 2^`white_shift` (`1E 41` byte 15, high nibble).
    white_shift: u8,
    /// The dark's scale, 2^`dark_shift` (low nibble).
    dark_shift: u8,
    /// The table: per column, per channel, `white byte << 8 | dark byte`.
    table: Vec<u16>,
}

/// The per-column statistics of a calibration read: 16-bit RGB, `lines` x `cols`.
fn columns(raw: &[u8], cols: usize) -> Vec<Vec<[u16; 3]>> {
    let line = cols * 6;
    raw.chunks_exact(line)
        .map(|l| {
            l.chunks_exact(6)
                .map(|p| {
                    [
                        u16::from_le_bytes([p[0], p[1]]),
                        u16::from_le_bytes([p[2], p[3]]),
                        u16::from_le_bytes([p[4], p[5]]),
                    ]
                })
                .collect()
        })
        .collect()
}

/// The smallest shift that brings `span` to 255 or less.
fn shift_for(span: f64) -> u8 {
    (0u8..16)
        .find(|&s| span / f64::from(1u32 << s) <= 255.0)
        .unwrap_or(15)
}

impl Shading {
    /// From the white and dark reads (16-bit RGB over `cols` columns), for a table of `len`
    /// values covering the first `width` columns.
    fn compute(white: &[u8], dark: &[u8], cols: usize, width: usize, len: usize) -> Self {
        let w = columns(white, cols);
        let d = columns(dark, cols);
        let width = width.min(cols);
        // The white per column: the mean of its brightest lines (the driver leaves out the
        // lines darkened by dust on the strip; its exact rule is not known).
        let keep = w.len() - w.len() * 5 / 16;
        let mut white_col = vec![[0f64; 3]; width];
        let mut dark_col = vec![[0f64; 3]; width];
        for col in 0..width {
            for ch in 0..3 {
                let mut v: Vec<u16> = w.iter().map(|l| l[col][ch]).collect();
                v.sort_unstable();
                let top = &v[v.len() - keep..];
                white_col[col][ch] = top.iter().map(|&x| f64::from(x)).sum::<f64>() / keep as f64;
                dark_col[col][ch] =
                    d.iter().map(|l| f64::from(l[col][ch])).sum::<f64>() / d.len() as f64;
            }
        }
        let min = |v: &[[f64; 3]], ch: usize| v.iter().map(|c| c[ch]).fold(f64::MAX, f64::min);
        let max = |v: &[[f64; 3]], ch: usize| v.iter().map(|c| c[ch]).fold(f64::MIN, f64::max);
        let k: [u16; 3] = std::array::from_fn(|ch| min(&white_col, ch).floor() as u16);
        let dark_min: [u16; 3] = std::array::from_fn(|ch| min(&dark_col, ch).floor() as u16);
        let white_shift = (0..3)
            .map(|ch| shift_for(max(&white_col, ch) - f64::from(k[ch])))
            .max()
            .unwrap_or(0);
        let dark_shift = (0..3)
            .map(|ch| shift_for(max(&dark_col, ch) - f64::from(dark_min[ch])))
            .max()
            .unwrap_or(0);
        let mut table = vec![0u16; len];
        for col in 0..width {
            for ch in 0..3 {
                let i = col * 3 + ch;
                if i >= len {
                    break;
                }
                let hi = ((white_col[col][ch] - f64::from(k[ch])) / f64::from(1u32 << white_shift))
                    .floor()
                    .clamp(0.0, 255.0) as u16;
                let lo = ((dark_col[col][ch] - f64::from(dark_min[ch]))
                    / f64::from(1u32 << dark_shift))
                .floor()
                .clamp(0.0, 255.0) as u16;
                table[i] = hi << 8 | lo;
            }
        }
        Self {
            k,
            dark_min,
            white_shift,
            dark_shift,
            table,
        }
    }

    /// The `1E 42` block: K per channel, then each minimum dark twice.
    fn block42(&self) -> Vec<u8> {
        let [d0, d1, d2] = self.dark_min;
        [self.k[0], self.k[1], self.k[2], d0, d0, d1, d1, d2, d2]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }

    fn table_bytes(&self) -> Vec<u8> {
        self.table.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
}

fn hex(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return Err(format!("not hex: {s}"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("{s}: {e}")))
        .collect()
}

fn parse_script(text: &str) -> Result<Vec<Step>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(n, l)| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let step = match f.as_slice() {
                ["W", h] => Step::Write(hex(h)?),
                ["R", h] => Step::Read(hex(h)?),
                ["A", h] => Step::Any(hex(h)?),
                ["D", n] => Step::Data(n.parse().map_err(|e| format!("{n}: {e}"))?),
                ["P", c, h, s] => Step::Poll(
                    hex(c)?,
                    hex(h)?,
                    s.parse().map_err(|e| format!("{s}: {e}"))?,
                ),
                ["M"] => Step::WriteBack,
                ["C", "black"] => Step::Calibration(Some(Cal::Black)),
                ["C", "white"] => Step::Calibration(Some(Cal::White)),
                ["C", "dark"] => Step::Calibration(Some(Cal::Dark)),
                ["C", "none"] => Step::Calibration(None),
                ["X42B", h] => Step::Block42Black(hex(h)?),
                ["X41", h] => Step::Block41(hex(h)?),
                ["X42", h] => Step::Block42(hex(h)?),
                ["XT", h] => Step::Table(hex(h)?),
                _ => return Err(format!("line {}: not a step: {l}", n + 1)),
            };
            Ok(step)
        })
        .collect()
}

fn read_n(t: &mut dyn Transport, n: usize) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; n];
    let mut done = 0;
    while done < n {
        let got = t.bulk_in(&mut buf[done..])?;
        if got == 0 {
            return Err(Error::Other(format!(
                "the scanner answered {done} bytes of {n}"
            )));
        }
        done += got;
    }
    Ok(buf)
}

fn show(b: &[u8]) -> String {
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    if h.len() > 48 {
        format!("[{}] {}...", b.len(), &h[..48])
    } else {
        h
    }
}

/// Runs `steps` on `t`; the image data read, in order.
fn run_steps(t: &mut dyn Transport, steps: &[Step]) -> Result<Vec<u8>, Error> {
    let mut data = Vec::new();
    let mut last92: Option<Vec<u8>> = None;
    let mut last_write: Vec<u8> = Vec::new();
    let start = std::time::Instant::now();
    // The calibration this replay reads, and the shading computed from it.
    let (mut white, mut dark) = (Vec::new(), Vec::new());
    let mut black = Vec::new();
    let mut target: Option<Cal> = None;
    let mut shading: Option<Shading> = None;
    // The table's length and width, from the capture's: needed before the table is reached.
    let table_shape = steps.iter().find_map(|s| match s {
        Step::Table(t) => {
            let values: Vec<u16> = t
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect();
            let used = values.iter().rposition(|&v| v != 0).map_or(0, |i| i + 1);
            Some((values.len(), used.div_ceil(3)))
        }
        _ => None,
    });
    for (i, step) in steps.iter().enumerate() {
        let stop = |what: String| Error::Other(format!("step {}: {what}; stopped", i + 1));
        let computed = matches!(step, Step::Block41(_) | Step::Block42(_) | Step::Table(_));
        if computed && shading.is_none() {
            let (len, width) =
                table_shape.ok_or_else(|| stop("no shading table in the script".into()))?;
            // 16 lines of 16-bit RGB: the columns of the calibration.
            let cols = white.len() / 16 / 6;
            if cols == 0 || white.len() != dark.len() {
                return Err(stop(format!(
                    "calibration reads of {} and {} bytes",
                    white.len(),
                    dark.len()
                )));
            }
            let s = Shading::compute(&white, &dark, cols, width, len);
            println!(
                "  shading from this calibration: {cols} columns, table {width} wide; K {:?}, \
                 dark {:?}, scales 2^{} and 2^{}",
                s.k, s.dark_min, s.white_shift, s.dark_shift
            );
            shading = Some(s);
        }
        match step {
            Step::Write(b) => {
                t.bulk_out(b)?;
                last_write.clone_from(b);
            }
            Step::WriteBack => {
                let b = last92
                    .clone()
                    .ok_or_else(|| stop("no 1E 92 answer to write back".into()))?;
                t.bulk_out(&b)?;
            }
            Step::Read(want) => {
                let got = read_n(t, want.len())?;
                if &got != want {
                    return Err(stop(format!(
                        "after {} the scanner answered {}, the capture {}",
                        show(&last_write),
                        show(&got),
                        show(want)
                    )));
                }
            }
            Step::Any(seen) => {
                let got = read_n(t, seen.len())?;
                if &got != seen {
                    println!(
                        "  step {}: after {} {} (the capture: {})",
                        i + 1,
                        show(&last_write),
                        show(&got),
                        show(seen)
                    );
                }
                if last_write == [0x1E, 0x92] {
                    last92 = Some(got);
                }
            }
            Step::Data(n) => {
                let got = read_n(t, *n)?;
                match target {
                    Some(Cal::Black) => black.extend_from_slice(&got),
                    Some(Cal::White) => white.extend_from_slice(&got),
                    Some(Cal::Dark) => dark.extend_from_slice(&got),
                    None => {}
                }
                data.extend(got);
            }
            Step::Calibration(to) => target = *to,
            Step::Block42Black(captured) => {
                if black.is_empty() {
                    return Err(stop("no black pixels read before their 1E 42".into()));
                }
                let b = black_block(&black);
                println!(
                    "  1E 42 (black pixels): {} (the capture: {})",
                    show(&b),
                    show(captured)
                );
                t.bulk_out(&b)?;
                last_write = b;
            }
            Step::Block41(captured) => {
                let s = shading.as_ref().expect("computed above");
                let mut b = captured.clone();
                b[15] = (s.white_shift << 4) | s.dark_shift;
                println!(
                    "  1E 41 byte 15: {:02x} (the capture: {:02x})",
                    b[15], captured[15]
                );
                t.bulk_out(&b)?;
                last_write = b;
            }
            Step::Block42(captured) => {
                let b = shading.as_ref().expect("computed above").block42();
                println!("  1E 42: {} (the capture: {})", show(&b), show(captured));
                t.bulk_out(&b)?;
                last_write = b;
            }
            Step::Table(captured) => {
                let b = shading.as_ref().expect("computed above").table_bytes();
                let (mut hi, mut lo, mut n) = (0f64, 0f64, 0usize);
                for (o, c) in b.chunks_exact(2).zip(captured.chunks_exact(2)) {
                    if c != [0, 0] {
                        hi += f64::from(o[1].abs_diff(c[1]));
                        lo += f64::from(o[0].abs_diff(c[0]));
                        n += 1;
                    }
                }
                let n = n.max(1) as f64;
                println!(
                    "  shading table: mean |difference| from the capture's, white byte {:.2}, \
                     dark byte {:.2}",
                    hi / n,
                    lo / n
                );
                t.bulk_out(&b)?;
                last_write = b;
            }
            Step::Poll(cmd, want, secs) => {
                let until = std::time::Instant::now() + std::time::Duration::from_secs(*secs);
                loop {
                    t.bulk_out(cmd)?;
                    let got = read_n(t, want.len())?;
                    if &got == want {
                        break;
                    }
                    if std::time::Instant::now() > until {
                        return Err(stop(format!(
                            "{} still answers {} after {secs} s, not {}",
                            show(cmd),
                            show(&got),
                            show(want)
                        )));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                last_write.clone_from(cmd);
            }
        }
        if (i + 1) % 50 == 0 {
            println!(
                "  {} of {} steps, {} bytes of data, {:.1} s",
                i + 1,
                steps.len(),
                data.len(),
                start.elapsed().as_secs_f64()
            );
        }
    }
    Ok(data)
}

fn replay(script: &str, out: &str) -> Result<(), Error> {
    let text =
        std::fs::read_to_string(script).map_err(|e| Error::Other(format!("{script}: {e}")))?;
    let steps = parse_script(&text).map_err(Error::Other)?;
    let writes = steps.iter().filter(|s| matches!(s, Step::Write(_))).count();
    println!("{script}: {} steps, {writes} writes", steps.len());
    let found = epson_gt_x750::list();
    let f = found
        .iter()
        .find(|f| f.note.is_none())
        .ok_or_else(|| Error::Disconnected("no Epson Perfection 4490 found".into()))?;
    let mut t = epson_gt_x750::open(&f.location)?;
    println!("opened {} {} ({t:?})", f.link, f.location);
    let start = std::time::Instant::now();
    let data = run_steps(&mut *t, &steps)?;
    std::fs::write(out, &data).map_err(|e| Error::Other(format!("{out}: {e}")))?;
    println!(
        "done in {:.1} s: {} bytes of image data -> {out}",
        start.elapsed().as_secs_f64(),
        data.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Answers what it is given, records the writes.
    #[derive(Debug, Default)]
    struct Script {
        writes: Vec<Vec<u8>>,
        answers: VecDeque<Vec<u8>>,
    }

    impl Transport for Script {
        fn bulk_out(&mut self, data: &[u8]) -> Result<(), Error> {
            self.writes.push(data.to_vec());
            Ok(())
        }
        fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            let a = self.answers.pop_front().unwrap_or_default();
            buf[..a.len()].copy_from_slice(&a);
            Ok(a.len())
        }
    }

    fn script(answers: &[&[u8]]) -> Script {
        Script {
            answers: answers.iter().map(|a| a.to_vec()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_script_is_parsed() {
        let s = parse_script("P 1b03 1881 120\nW 1e92\nA 7400\nW 1e31\nM\nD 4\nR 06\n").unwrap();
        assert_eq!(
            s,
            [
                Step::Poll(vec![0x1B, 0x03], vec![0x18, 0x81], 120),
                Step::Write(vec![0x1E, 0x92]),
                Step::Any(vec![0x74, 0x00]),
                Step::Write(vec![0x1E, 0x31]),
                Step::WriteBack,
                Step::Data(4),
                Step::Read(vec![0x06]),
            ]
        );
        assert!(parse_script("X 00").is_err());
        assert!(parse_script("W 0").is_err());
    }

    #[test]
    fn it_polls_writes_back_and_keeps_the_data() {
        let steps = parse_script("P 1b03 1881 5\nW 1e92\nA 7400\nW 1e31\nM\nR 06\nD 3\n").unwrap();
        // Busy once, then ready; 1E 92 answers other values than the capture.
        let mut t = script(&[
            &[0x58, 0x81],
            &[0x18, 0x81],
            &[0x75, 0x01],
            &[6],
            &[1, 2, 3],
        ]);
        let data = run_steps(&mut t, &steps).unwrap();
        assert_eq!(data, [1, 2, 3]);
        assert_eq!(
            t.writes,
            [
                vec![0x1B, 0x03],
                vec![0x1B, 0x03],
                vec![0x1E, 0x92],
                vec![0x1E, 0x31],
                vec![0x75, 0x01],
            ]
        );
    }

    #[test]
    fn it_stops_at_the_first_answer_that_differs() {
        let steps = parse_script("W 1ea1\nR 01\nW 1e57\n").unwrap();
        let mut t = script(&[&[0x02]]);
        let e = run_steps(&mut t, &steps).unwrap_err().to_string();
        assert!(e.contains("step 2") && e.contains("answered 02"), "{e}");
        // Nothing after it went out.
        assert_eq!(t.writes, [vec![0x1E, 0xA1]]);
    }
}

#[cfg(test)]
mod shading_tests {
    use super::*;

    fn rgb16(pixels: &[[u16; 3]]) -> Vec<u8> {
        pixels
            .iter()
            .flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes()))
            .collect()
    }

    #[test]
    fn the_black_block_is_the_floor_of_each_channels_mean() {
        // Two pixels, then the 8-byte block trailer, left out.
        let mut raw = rgb16(&[[1136, 1238, 1330], [1139, 1239, 1331]]);
        raw.extend([0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x80, 0x00]);
        let want: Vec<u8> = [0u16, 0, 0, 1137, 1137, 1238, 1238, 1330, 1330]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(black_block(&raw), want);
    }

    #[test]
    fn the_shading_packs_white_and_dark_per_column() {
        // Two columns, 16 lines: white 1000 / 1000+255*128 (one line darkened by dust in the
        // second, left out), dark 100 / 100+4*10.
        let cols = 2;
        let mut white = Vec::new();
        let mut dark = Vec::new();
        for line in 0..16 {
            let dust = if line == 3 { 5000 } else { 0 };
            white.extend(rgb16(&[[1000; 3], [1000 + 255 * 128 - dust; 3]]));
            dark.extend(rgb16(&[[100; 3], [140; 3]]));
        }
        let s = Shading::compute(&white, &dark, cols, 2, 8);
        assert_eq!(s.k, [1000; 3]);
        assert_eq!(s.dark_min, [100; 3]);
        assert_eq!((s.white_shift, s.dark_shift), (7, 0));
        // Column 0: white 0, dark 0; column 1: white 255, dark 40; then zeros.
        assert_eq!(
            s.table,
            [0, 0, 0, 255 << 8 | 40, 255 << 8 | 40, 255 << 8 | 40, 0, 0]
        );
        assert_eq!(s.block42()[..6], [0xE8, 0x03, 0xE8, 0x03, 0xE8, 0x03]);
    }
}
