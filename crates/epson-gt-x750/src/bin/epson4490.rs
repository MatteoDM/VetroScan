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
//! Epson's driver sent, nothing computed here), checks every answer against the capture and
//! stops at the first that differs; the image data read goes to OUT. It does not go through the
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
    for (i, step) in steps.iter().enumerate() {
        let stop = |what: String| Error::Other(format!("step {}: {what}; stopped", i + 1));
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
            Step::Data(n) => data.extend(read_n(t, *n)?),
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
