// SPDX-License-Identifier: GPL-2.0-or-later
//! A glass scan computed from the request: the sequence Epson's WIA driver
//! sends (the captures of 11-12 November 2026, `captures/epson4490/`, outside
//! the repository), with every request-dependent block from
//! [`crate::setup`], the shading from our own calibration
//! ([`crate::shading`]) and the x origin's correction from the scanner's own
//! `1E 87` answer.
//!
//! In order: the opening (`1E 53`, `1B 40`, `1B 13`, `1E A1`), the gamma
//! tables (`1E B3`), the request (`1E 57`), the lamp (`1E A2 01`), `1E 87`,
//! the line time (`1E 46`, waiting on `1E 04`), `1E 92`/`1E 8C`; the black
//! pixels, the white strip (moving 160 steps), the lamp off and the dark; the
//! lamp on, at 1200 dpi `1E 8B`/`1E 83`, the tone tables (`1E 84` type 02),
//! the start move; the image's set-up with the shading, the read, and the
//! colours realigned and cropped on the computer. The carriage goes home by
//! itself after the read.
//!
//! Everything goes through the [`Guarded`] transport, which lets the scan's
//! commands out only for a request and a plan inside what the captures cover
//! ([`Guarded::permit_request`], [`Guarded::permit_plan`]).

use crate::guard::Guarded;
use crate::protocol::{State, ACK, GET_STATE};
use crate::setup::{self, Plan, Request, XCorrection, TRAILER_LEN};
use crate::shading::{black_block, Shading};
use crate::{tables, Transport};
use scanner_api::{Error, Result};
use std::time::{Duration, Instant};

/// What the scanner names itself (`1B 13`): the only model and firmware seen.
pub const IDENTITY: &[u8; 28] = b"EPSON   GT-X750         1.27";
/// `1E 83`'s header at 1200 dpi: reads 600 bytes at `7ff9a0` (captures 33,
/// 37), what Epson's driver does when `1E 8B` answers `06`.
pub const READ_83: [u8; 8] = [0x07, 0xA0, 0xF9, 0x7F, 0x00, 0x58, 0x02, 0x00];
/// The longest single read asked of the transport.
const READ_CHUNK: usize = 65_536;
/// How long the scanner may stay busy (the lamp warms up for about a minute).
const WAIT: Duration = Duration::from_secs(180);

/// A scanned image: RGB interleaved, 8 bits or 16 bits little-endian.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Pixels across.
    pub width: u32,
    /// Lines.
    pub height: u32,
    /// Bits per channel, 8 or 16.
    pub bits: u8,
    /// `height` lines of `width` x 3 samples.
    pub data: Vec<u8>,
}

/// What a scan did, besides the image.
#[derive(Debug, Clone)]
pub struct Scanned {
    /// The image, realigned and cropped.
    pub image: Image,
    /// The set-up sent.
    pub plan: Plan,
    /// The shading computed from this scan's calibration.
    pub shading: Shading,
    /// The image data as the scanner sent it, trailers removed (the lines
    /// before realigning, every pixel read).
    pub raw: Vec<u8>,
}

fn other(what: String) -> Error {
    Error::Other(what)
}

fn read_exact(t: &mut dyn Transport, n: usize, what: &str) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    let mut done = 0;
    while done < n {
        let end = n.min(done + READ_CHUNK);
        let got = t.bulk_in(&mut buf[done..end])?;
        if got == 0 {
            return Err(other(format!(
                "{what}: the scanner answered {done} bytes of {n}"
            )));
        }
        done += got;
    }
    Ok(buf)
}

fn ack(t: &mut dyn Transport, after: &str) -> Result<()> {
    let a = read_exact(t, 1, after)?;
    if a[0] != ACK {
        return Err(other(format!(
            "after {after} the scanner answered {:02X}, not 06",
            a[0]
        )));
    }
    Ok(())
}

fn name(cmd: [u8; 2]) -> String {
    format!("{:02X} {:02X}", cmd[0], cmd[1])
}

/// A command answered by `06`.
fn command(t: &mut dyn Transport, cmd: [u8; 2]) -> Result<()> {
    t.bulk_out(&cmd)?;
    ack(t, &name(cmd))
}

/// A command with a block: `cmd`, `06`, the block, `06`.
fn send(t: &mut dyn Transport, cmd: [u8; 2], block: &[u8]) -> Result<()> {
    command(t, cmd)?;
    t.bulk_out(block)?;
    ack(t, &format!("{}'s block", name(cmd)))
}

/// A command answered by `n` bytes.
fn ask(t: &mut dyn Transport, cmd: [u8; 2], n: usize) -> Result<Vec<u8>> {
    t.bulk_out(&cmd)?;
    read_exact(t, n, &name(cmd))
}

/// `1E 84`: a table at the place its header says.
fn table(t: &mut dyn Transport, header: &[u8; 8], values: &[u8]) -> Result<()> {
    command(t, [0x1E, 0x84])?;
    t.bulk_out(header)?;
    t.bulk_out(values)?;
    ack(t, "a 1E 84 table")
}

fn ramp_bytes(r: &[u16]) -> Vec<u8> {
    r.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Polls `1B 03` until the scanner is idle with its lamp warm (`18 81` at rest
/// in every capture; `0x40` moving, `0x02` busy, `0x01` the lamp warming).
fn wait_ready(t: &mut dyn Transport) -> Result<()> {
    let until = Instant::now() + WAIT;
    loop {
        let s = State(read_two(ask(t, GET_STATE, 2)?));
        if s.firmware_running() && s.0[0] & 0x43 == 0 {
            return Ok(());
        }
        if Instant::now() > until {
            return Err(other(format!(
                "the scanner still answers {:02X} {:02X} to 1B 03 after {} s",
                s.0[0],
                s.0[1],
                WAIT.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn read_two(v: Vec<u8>) -> [u8; 2] {
    [v[0], v[1]]
}

/// Polls `1E 04` until `01` (`02`: still waiting), after `1E 46`.
fn wait_04(t: &mut dyn Transport) -> Result<()> {
    let until = Instant::now() + WAIT;
    loop {
        let a = ask(t, [0x1E, 0x04], 1)?;
        match a[0] {
            0x01 => return Ok(()),
            0x02 if Instant::now() <= until => std::thread::sleep(Duration::from_millis(50)),
            other_ => {
                return Err(other(format!(
                    "1E 04 answered {other_:02X} (01 ready, 02 waiting)"
                )))
            }
        }
    }
}

/// `1E A1`: the lamp in use, `01` for the glass's.
fn lamp_is_the_glass(t: &mut dyn Transport) -> Result<()> {
    let a = ask(t, [0x1E, 0xA1], 1)?;
    if a[0] != 0x01 {
        return Err(other(format!(
            "1E A1 answered {:02X}: the lid's lamp is in use (after films), not the glass's; \
             a scan of the glass with Epson's driver switches it back",
            a[0]
        )));
    }
    Ok(())
}

/// The set-up before every read: `1E 31` (the `1E 92` answer back), `1E 11`,
/// `1E 21`, `1E 41`, `1E 42`, `1E 43`.
fn setup_read(
    t: &mut dyn Transport,
    p: &Plan,
    back: &[u8],
    e21: &[u8],
    e41: &[u8],
    e42: &[u8],
    e43: &[u8],
) -> Result<()> {
    send(t, [0x1E, 0x31], back)?;
    send(t, [0x1E, 0x11], &[p.e11])?;
    send(t, [0x1E, 0x21], e21)?;
    send(t, [0x1E, 0x41], e41)?;
    send(t, [0x1E, 0x42], e42)?;
    send(t, [0x1E, 0x43], e43)
}

/// `1E 47` and the read of `lines` lines of `line` bytes in blocks of `per`
/// lines, each with its trailer; the data without the trailers.
fn read_lines(
    t: &mut dyn Transport,
    lines: u32,
    per: u32,
    line: u32,
    progress: &mut dyn FnMut(u32, u32),
) -> Result<Vec<u8>> {
    t.bulk_out(&[0x1E, 0x47])?;
    let mut data = Vec::with_capacity((lines * line) as usize);
    let mut left = lines;
    while left > 0 {
        let k = per.min(left);
        let n = (k * line) as usize;
        let mut b = read_exact(t, n + TRAILER_LEN, "image data")?;
        let trailer = b.split_off(n);
        let len = (n as u32).to_le_bytes();
        if trailer != [0x02, len[0], len[1], len[2], 0, 0, 0x80, 0] {
            return Err(other(format!(
                "a block of {n} bytes ended with {trailer:02X?}, not its trailer"
            )));
        }
        data.extend(b);
        left -= k;
        progress(lines - left, lines);
    }
    Ok(data)
}

/// Realigns the colours (green `offset` lines below blue, red twice) and crops
/// to the request: exactly what WIA returns from the same data (captures 03,
/// 04, 30).
pub fn assemble(raw: &[u8], p: &Plan) -> Image {
    let r = p.request;
    let s = usize::from(r.bits / 8);
    let line = p.bytes_per_line as usize;
    let k = p.colour_offset() as usize;
    let offsets = [2 * k, k, 0];
    let (w, h) = (r.width as usize, r.height as usize);
    let mut data = Vec::with_capacity(w * h * 3 * s);
    for y in 0..h {
        for x in 0..w {
            for (c, off) in offsets.iter().enumerate() {
                let at = (y + off) * line + (x * 3 + c) * s;
                data.extend_from_slice(&raw[at..at + s]);
            }
        }
    }
    Image {
        width: r.width,
        height: r.height,
        bits: r.bits,
        data,
    }
}

/// Scans `request` on the glass. `log` gets a line for each phase, `progress`
/// the image lines read and their total.
pub fn scan_glass<T: Transport>(
    g: &mut Guarded<T>,
    request: Request,
    log: &mut dyn FnMut(String),
    progress: &mut dyn FnMut(u32, u32),
) -> Result<Scanned> {
    g.permit_request(&request)?;
    let result = run(g, request, log, progress);
    g.end_scan();
    result
}

fn run<T: Transport>(
    g: &mut Guarded<T>,
    request: Request,
    log: &mut dyn FnMut(String),
    progress: &mut dyn FnMut(u32, u32),
) -> Result<Scanned> {
    let e57 = setup::request_block(&request)?;
    // The opening, as WIA's driver does it.
    wait_ready(g)?;
    ask(g, [0x1E, 0x53], 42)?;
    wait_ready(g)?;
    command(g, [0x1B, 0x40])?;
    ask(g, [0x1E, 0x53], 42)?;
    wait_ready(g)?;
    let id = ask(g, [0x1B, 0x13], IDENTITY.len())?;
    if id != IDENTITY {
        return Err(other(format!(
            "1B 13 answered {:?}, not {:?}: a model or firmware never seen",
            String::from_utf8_lossy(&id),
            String::from_utf8_lossy(IDENTITY)
        )));
    }
    for _ in 0..2 {
        wait_ready(g)?;
        lamp_is_the_glass(g)?;
    }
    wait_ready(g)?;
    command(g, [0x1B, 0x40])?;
    ask(g, [0x1E, 0x53], 42)?;
    wait_ready(g)?;
    lamp_is_the_glass(g)?;
    let gamma = tables::gamma();
    for channel in 0..3u8 {
        command(g, [0x1E, 0xB3])?;
        g.bulk_out(&[channel])?;
        g.bulk_out(&gamma)?;
    }
    wait_ready(g)?;
    lamp_is_the_glass(g)?;
    ask(g, [0x1E, 0x53], 42)?;
    send(g, [0x1E, 0x57], &e57)?;
    wait_ready(g)?;
    lamp_is_the_glass(g)?;
    send(g, [0x1E, 0xA2], &[0x01])?;
    log("lamp on (1E A2 01)".into());
    wait_ready(g)?;

    // The x origin's correction, stored in the scanner; then the plan.
    command(g, [0x1E, 0x87])?;
    g.bulk_out(&[0x00])?;
    let a87 = read_exact(g, 10, "1E 87")?;
    let xc = XCorrection::from_1e87(&a87)?;
    let p = Plan::new(request, xc)?;
    g.permit_plan(&p)?;
    log(format!(
        "1E 87: x correction {} (4800ths of an inch); columns {}-{}, {} lines of {} pixels, \
         T {} µs, {} steps/s ({:?}), start move {} steps, lead {}",
        xc.0,
        p.columns.0,
        p.columns.1,
        p.lines,
        p.pixels,
        p.t,
        p.speed,
        p.mode,
        p.start_move.as_ref().map_or(0, |m| m.steps),
        p.lead
    ));

    ask(g, [0x1E, 0x53], 42)?;
    send(g, [0x1E, 0x57], &e57)?;
    ask(g, [0x1E, 0x86], 4)?;
    send(g, [0x1E, 0x46], &p.e46[0])?;
    wait_04(g)?;
    send(g, [0x1E, 0x46], &p.e46[1])?;
    wait_04(g)?;
    let back = ask(g, [0x1E, 0x92], 12)?;
    ask(g, [0x1E, 0x8C], 1)?;

    // The sensor's black pixels: 4 lines, still.
    let zeros = [0u8; 18];
    setup_read(
        g,
        &p,
        &back,
        &p.e21_cal,
        &p.e41_black,
        &zeros,
        &setup::GAIN_CAL,
    )?;
    wait_ready(g)?;
    let black_cols = p.black_column_count();
    let black = read_lines(g, 4, 4, black_cols * 6, &mut |_, _| {})?;
    let e42_black = black_block(&black);

    // The white strip: 16 lines at 16 bits, the carriage moving 160 steps.
    wait_ready(g)?;
    setup_read(
        g,
        &p,
        &back,
        &p.e21_cal,
        &p.e41_white,
        &e42_black,
        &setup::GAIN_CAL,
    )?;
    table(g, &setup::ramp_header(1), &ramp_bytes(&p.ramp_cal))?;
    send(g, [0x1E, 0x01], &p.move_cal)?;
    wait_ready(g)?;
    let cols = p.column_count();
    let white = read_lines(g, 16, 16, cols * 6, &mut |_, _| {})?;

    // The lamp off, the dark, still.
    wait_ready(g)?;
    send(g, [0x1E, 0xA2], &[0x00])?;
    setup_read(
        g,
        &p,
        &back,
        &p.e21_cal,
        &p.e41_dark,
        &zeros,
        &setup::GAIN_CAL,
    )?;
    wait_ready(g)?;
    let dark = read_lines(g, 16, 16, cols * 6, &mut |_, _| {})?;
    wait_ready(g)?;
    send(g, [0x1E, 0xA2], &[0x01])?;
    log("calibration read: black pixels, white, dark; lamp on again".into());

    if p.base == 2400 {
        // 06: the scanner holds what Epson's driver would otherwise compute and load
        // (1E 84 type 07, seen once, capture 05); then it reads 600 bytes back.
        let a = ask(g, [0x1E, 0x8B], 1)?;
        if a[0] != ACK {
            return Err(other(format!(
                "1E 8B answered {:02X}: the scanner wants the 1200 dpi table Epson's driver \
                 computes (not known yet); a 1200 dpi scan with Epson's driver loads it until \
                 the scanner is switched off",
                a[0]
            )));
        }
        wait_ready(g)?;
        command(g, [0x1E, 0x83])?;
        g.bulk_out(&READ_83)?;
        read_exact(g, 600, "1E 83")?;
    }
    let at = ask(g, [0x1E, 0x86], 4)?;
    let at = u16::from_le_bytes([at[0], at[1]]);
    if u32::from(at) != p.after_calibration {
        return Err(other(format!(
            "after the calibration the carriage is at {at}, not at {}",
            p.after_calibration
        )));
    }
    for header in &tables::TONE_HEADERS {
        table(g, header, tables::TONE)?;
    }

    // The move to the lead.
    if let Some(m) = &p.start_move {
        table(g, &setup::ramp_header(m.ramp.len()), &ramp_bytes(&m.ramp))?;
        send(g, [0x1E, 0x01], &m.block)?;
        command(g, [0x1E, 0x05])?;
        wait_ready(g)?;
    }

    // The image.
    let shading = Shading::compute(
        &white,
        &dark,
        cols as usize,
        p.shading_columns as usize,
        cols as usize * 3,
    );
    log(format!(
        "shading: K {:?}, minimum dark {:?}, scales 2^{} and 2^{}",
        shading.k, shading.dark_min, shading.white_shift, shading.dark_shift
    ));
    let mut e41 = p.e41_data;
    e41[15] = (shading.white_shift << 4) | shading.dark_shift;
    setup_read(
        g,
        &p,
        &back,
        &p.e21_data,
        &e41,
        &shading.block42(),
        &setup::GAIN_DATA,
    )?;
    table(g, &setup::shading_header(cols), &shading.table_bytes())?;
    table(
        g,
        &setup::ramp_header(p.ramp_data.len()),
        &ramp_bytes(&p.ramp_data),
    )?;
    send(g, [0x1E, 0x01], &p.move_data)?;
    wait_ready(g)?;
    log(format!("reading {} lines", p.lines));
    let raw = read_lines(g, p.lines, p.lines_per_block, p.bytes_per_line, progress)?;
    wait_ready(g)?;
    lamp_is_the_glass(g)?;
    let image = assemble(&raw, &p);
    Ok(Scanned {
        image,
        plan: p,
        shading,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::tests::{parse, request_of, Line, CAPTURES};
    use std::collections::VecDeque;

    /// A scanner that answers as in a capture and checks every write against
    /// Epson's driver's, in order.
    #[derive(Debug)]
    struct Follow {
        name: &'static str,
        lines: Vec<Line>,
        at: usize,
        answer: VecDeque<u8>,
        last_cmd: [u8; 2],
        writes: usize,
    }

    impl Follow {
        fn new(name: &'static str, text: &str) -> Self {
            Self {
                name,
                lines: parse(text),
                at: 0,
                answer: VecDeque::new(),
                last_cmd: [0; 2],
                writes: 0,
            }
        }

        fn fail(&self, what: String) -> Error {
            Error::Other(format!(
                "capture {}, line {} (write {}): {what}",
                self.name, self.at, self.writes
            ))
        }
    }

    impl Transport for Follow {
        fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
            if !self.answer.is_empty() {
                return Err(self.fail(format!("{} bytes of answer not read", self.answer.len())));
            }
            let line = self.lines.get(self.at).cloned();
            self.at += 1;
            self.writes += 1;
            let ok = match &line {
                Some(Line::P(cmd, answer)) if cmd == data => {
                    self.answer.extend(answer);
                    true
                }
                Some(Line::W(want, None)) => {
                    let mut want = want.clone();
                    let mut got = data.to_vec();
                    // The colour correction (WIA's own, any value works) and the scales.
                    if self.last_cmd == [0x1E, 0x57] && want.len() == 42 {
                        want[33] = 0;
                    }
                    if self.last_cmd == [0x1E, 0x41] && want.len() == 22 && want[11] == 0x80 {
                        want[16] = 0;
                        (want[15], got[15]) = (0, 0);
                    }
                    want == got
                }
                Some(Line::W(_, Some(tag))) => match tag.split(' ').collect::<Vec<_>>()[..] {
                    ["gamma"] => data == tables::gamma(),
                    ["tone"] => data == tables::TONE,
                    ["shading", n] => data.len() == n.parse::<usize>().unwrap(),
                    ["42", _] => data.len() == 18,
                    _ => false,
                },
                _ => false,
            };
            if !ok {
                return Err(self.fail(format!("wrote {data:02x?}, the capture {line:?}")));
            }
            if data.len() == 2 && (data[0] == 0x1E || data[0] == 0x1B) {
                self.last_cmd = [data[0], data[1]];
            }
            Ok(())
        }

        fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
            if self.answer.is_empty() {
                let line = self.lines.get(self.at).cloned();
                self.at += 1;
                match line {
                    Some(Line::R(b) | Line::A(b)) => self.answer.extend(b),
                    // Image data: a ramp across the line, so the realigning shows.
                    Some(Line::D(n)) => self.answer.extend((0..n).map(|i| (i % 251) as u8)),
                    other_ => return Err(self.fail(format!("a read, the capture {other_:?}"))),
                }
            }
            let n = buf.len().min(self.answer.len());
            for (b, a) in buf.iter_mut().zip(self.answer.drain(..n)) {
                *b = a;
            }
            Ok(n)
        }
    }

    /// Scans a capture's request on a scanner answering as in it: the result, then
    /// how far into the capture it got and how many writes it made.
    fn scan(name: &'static str, text: &str) -> (Result<Scanned>, usize, usize) {
        let (request, _, _) = request_of(&parse(text));
        let mut g = Guarded::new(Follow::new(name, text));
        let r = scan_glass(&mut g, request, &mut |_| {}, &mut |_, _| {});
        let f = g.inner();
        let r = r.and_then(|s| match f.lines.len() - f.at {
            0 => Ok(s),
            left => Err(Error::Other(format!("{left} lines of the capture left"))),
        });
        (r, f.at, f.writes)
    }

    /// The whole native scan writes, command by command and block by block, what
    /// Epson's WIA driver wrote in these captures (but the values computed from the
    /// calibration, and the colour correction byte WIA sets at random).
    #[test]
    fn the_scan_writes_what_epsons_driver_wrote() {
        let same = [
            "03", "06", "07", "10", "29", "32", "33", "34", "35", "36", "37",
        ];
        for (name, text) in CAPTURES.iter().filter(|(n, _)| same.contains(n)) {
            let (r, _, writes) = scan(name, text);
            let s = r.unwrap_or_else(|e| panic!("{e}"));
            assert!(writes > 100, "{name}");
            let q = s.plan.request;
            assert_eq!(
                (s.image.width, s.image.height),
                (q.width, q.height),
                "{name}"
            );
            assert_eq!(
                s.raw.len() as u32,
                s.plan.lines * s.plan.bytes_per_line,
                "{name}"
            );
        }
    }

    /// At 1200 dpi, a scanner without the table Epson's driver computes (`1E 8B`
    /// answering `15`, capture 05) stops the scan before the tone tables and the
    /// start move.
    #[test]
    fn without_the_1200_dpi_table_the_scan_stops() {
        let (_, text) = CAPTURES.iter().find(|(n, _)| *n == "05").unwrap();
        let (r, at, _) = scan("05", text);
        let e = r.unwrap_err().to_string();
        assert!(e.contains("1E 8B answered 15"), "{e}");
        let lines = parse(text);
        let i8b = lines
            .iter()
            .position(|l| matches!(l, Line::W(b, None) if b == &[0x1E, 0x8B]))
            .unwrap();
        // 1E 8B and its answer, nothing after.
        assert_eq!(at, i8b + 2);
    }

    #[test]
    fn the_colours_are_realigned_and_cropped() {
        // 300 dpi: green 6 lines below blue, red 12; 768 pixels read for 591.
        let p = Plan::new(Request::new(300, 0, 0, 591, 3, 8), XCorrection(-93)).unwrap();
        assert_eq!((p.pixels, p.lines), (768, 15));
        let line = p.bytes_per_line as usize;
        let raw: Vec<u8> = (0..p.lines as usize * line)
            .map(|i| ((i / line) * 10 + i % 3) as u8)
            .collect();
        let im = assemble(&raw, &p);
        assert_eq!(im.data.len(), 591 * 3 * 3);
        // Line 1, any pixel: red from raw line 13, green from 7, blue from 1.
        let px = &im.data[591 * 3 + 5 * 3..591 * 3 + 6 * 3];
        assert_eq!(px, [130, 71, 12]);
        // 16 bits: two bytes a sample, the same lines.
        let p = Plan::new(Request::new(600, 0, 0, 400, 2, 16), XCorrection(-93)).unwrap();
        let line = p.bytes_per_line as usize;
        let raw: Vec<u8> = (0..p.lines as usize * line)
            .map(|i| (i / line) as u8)
            .collect();
        let im = assemble(&raw, &p);
        assert_eq!(im.data[..6], [24, 24, 12, 12, 0, 0]);
    }
}
