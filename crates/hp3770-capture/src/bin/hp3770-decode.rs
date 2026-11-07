// SPDX-License-Identifier: GPL-2.0-or-later
//! `hp3770-decode`: prints a USBPcap capture of the HP Scanjet 3770 as a
//! readable list of chip operations.

use hp3770_capture::pcap::Pcap;
use hp3770_capture::registers::{Execution, BLOCK_START};
use hp3770_capture::trace::{Event, Trace};
use hp3770_core::protocol::Command;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: hp3770-decode [OPTIONS] CAPTURE.pcap...

Prints the USB traffic of an HP Scanjet 3770 capture (USBPcap, classic pcap)
as decoded RTS8822 operations.

Options:
  -s, --summary   print only the totals of each capture
  -w, --writes    hide register/EEPROM reads (show what changes the scanner)
  -r, --raw       do not merge consecutive bulk IN transfers
  -x, --executions
                  print the register block at each execution (bit 7 of E800):
                  the first in full, then only the registers that changed
      --slice FROM TO OUT
                  write the records between FROM and TO seconds to OUT
                  (a smaller pcap, e.g. a test fixture)
  -h, --help      show this help
";

#[derive(Debug, Default)]
struct Options {
    summary: bool,
    writes_only: bool,
    raw: bool,
    executions: bool,
    slice: Option<(f64, f64, String)>,
    files: Vec<String>,
}

fn parse_args() -> Result<Options, String> {
    let mut opts = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-s" | "--summary" => opts.summary = true,
            "-w" | "--writes" => opts.writes_only = true,
            "-r" | "--raw" => opts.raw = true,
            "-x" | "--executions" => opts.executions = true,
            "--slice" => {
                let mut time = || -> Result<f64, String> {
                    let a = args.next().ok_or("--slice needs FROM TO OUT")?;
                    a.parse().map_err(|_| format!("bad time {a}"))
                };
                let (from, to) = (time()?, time()?);
                let out = args.next().ok_or("--slice needs FROM TO OUT")?;
                opts.slice = Some((from, to, out));
            }
            "-h" | "--help" => return Err(String::new()),
            a if a.starts_with('-') => return Err(format!("unknown option {a}")),
            file => opts.files.push(file.to_owned()),
        }
    }
    if opts.files.is_empty() {
        return Err("no capture given".into());
    }
    if opts.slice.is_some() && opts.files.len() != 1 {
        return Err("--slice takes exactly one capture".into());
    }
    Ok(opts)
}

fn is_read(event: &Event) -> bool {
    matches!(
        event,
        Event::Control {
            command: Command::ReadRegisters { .. } | Command::ReadEeprom { .. },
            ..
        }
    )
}

fn print_trace(out: &mut impl Write, trace: &Trace, opts: &Options) -> io::Result<()> {
    // A run of consecutive bulk IN transfers: (first time, count, bytes).
    let mut run: Option<(f64, usize, u64)> = None;
    let flush = |out: &mut dyn Write, run: &mut Option<(f64, usize, u64)>| -> io::Result<()> {
        if let Some((t, n, bytes)) = run.take() {
            writeln!(
                out,
                "{t:>10.3}  BULK  in  ep81 {bytes} bytes in {n} transfers"
            )?;
        }
        Ok(())
    };
    for event in &trace.events {
        if opts.writes_only && is_read(event) {
            continue;
        }
        if let (false, Event::BulkIn { time, data, .. }) = (opts.raw, event) {
            let r = run.get_or_insert((*time, 0, 0));
            r.1 += 1;
            r.2 += data.len() as u64;
            continue;
        }
        flush(out, &mut run)?;
        writeln!(out, "{event}")?;
    }
    flush(out, &mut run)
}

fn print_executions(out: &mut impl Write, trace: &Trace) -> io::Result<()> {
    let executions = trace.executions();
    let mut previous: Option<&Execution> = None;
    for (n, ex) in executions.iter().enumerate() {
        writeln!(out, "{:>10.3}  EXECUTE #{n} (event {})", ex.time, ex.event)?;
        match previous {
            None => {
                for (i, line) in ex.block.chunks(16).enumerate() {
                    write!(
                        out,
                        "            {:04X}:",
                        usize::from(BLOCK_START) + i * 16
                    )?;
                    for b in line {
                        match b {
                            Some(b) => write!(out, " {b:02X}")?,
                            None => write!(out, " ??")?,
                        }
                    }
                    writeln!(out)?;
                }
            }
            Some(prev) => {
                let diff = ex.diff(prev);
                if diff.is_empty() {
                    writeln!(out, "            same registers as #{}", n - 1)?;
                }
                let byte = |b: Option<u8>| b.map_or("??".to_owned(), |b| format!("{b:02X}"));
                for (addr, now, before) in diff {
                    writeln!(
                        out,
                        "            {addr:04X}: {} -> {}",
                        byte(before),
                        byte(now)
                    )?;
                }
            }
        }
        previous = Some(ex);
    }
    writeln!(out, "{} execution(s)", executions.len())
}

fn slice(file: &str, from: f64, to: f64, out: &str) -> Result<(), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
    let pcap = Pcap::parse(&bytes).map_err(|e| format!("{file}: {e}"))?;
    let sliced = pcap.slice(from, to).map_err(|e| format!("{file}: {e}"))?;
    std::fs::write(out, &sliced).map_err(|e| format!("{out}: {e}"))?;
    eprintln!("{out}: {} bytes", sliced.len());
    Ok(())
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("error: {msg}\n");
            }
            eprint!("{USAGE}");
            return if msg.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
    };
    if let Some((from, to, out)) = &opts.slice {
        return match slice(&opts.files[0], *from, *to, out) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let mut failed = false;
    for file in &opts.files {
        let trace = match Trace::from_path(file) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{file}: {e}");
                failed = true;
                continue;
            }
        };
        let result = (|| {
            if opts.files.len() > 1 || opts.summary {
                writeln!(out, "== {file}")?;
            }
            if opts.summary {
                writeln!(out, "{}\n", trace.summary())
            } else if opts.executions {
                print_executions(&mut out, &trace)
            } else {
                print_trace(&mut out, &trace, &opts)
            }
        })();
        if let Err(e) = result {
            // Broken pipe (e.g. `| head`) is not an error worth reporting.
            if e.kind() == io::ErrorKind::BrokenPipe {
                return ExitCode::SUCCESS;
            }
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    }
    if out.flush().is_err() {
        return ExitCode::SUCCESS;
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
