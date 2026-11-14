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
//!
//! For films, so far only the lid's lamp ([`light_lid_lamp`]): the beginning of Epson Scan's
//! film scan, as far as the lamp is lit; the film path itself is not here yet.

use crate::film::FilmRequest;
use crate::guard::Guarded;
use crate::protocol::{is_state, State, ACK, GET_STATE, TABLE_CHUNK, TABLE_PAUSE};
use crate::setup::{self, Plan, Request, XCorrection, TRAILER_LEN};
use crate::shading::{black_block, Shading};
use crate::table07;
use crate::tables::{self, Curves};
use crate::Transport;
use scanner_api::{Error, Result};
use std::time::{Duration, Instant, SystemTime};

/// What the scanner names itself (`1B 13`): the only model and firmware seen.
pub const IDENTITY: &[u8; 28] = b"EPSON   GT-X750         1.27";
/// `1E 83`'s header at 1200 dpi: reads 600 bytes at `7ff9a0` (captures 33,
/// 37), what Epson's driver does when `1E 8B` answers `06`.
pub const READ_83: [u8; 8] = [0x07, 0xA0, 0xF9, 0x7F, 0x00, 0x58, 0x02, 0x00];
/// Stops a scan under way (capture 11).
pub const CANCEL: [u8; 2] = [0x1E, 0x18];
/// The most image data taken to come before `1E 18`'s answer. The scanner sends what its
/// memory holds first: 262,176 bytes on 12 November 2026 (300 dpi, stopped after three
/// blocks), then `06`; a reader that gave up before it left the scanner stuck until switched
/// off. Its memory's size is not known: generously more.
const CANCEL_MAX_LEFT_OUT: usize = 16 << 20;
/// The most empty packets taken before `1E 18`'s `06` (one seen, capture 48).
const MAX_EMPTY: usize = 8;
/// The longest single read asked of the transport.
const READ_CHUNK: usize = 65_536;
/// How long the scanner may stay busy.
const WAIT: Duration = Duration::from_secs(180);
/// How long a cold lamp is given before the calibration, from when it was lit. Measured on
/// 12 November 2026 (`captures/epson4490/05-catture.md`, "La lampada fredda, misurata"): ten
/// scans right after switching on; `1B 03` flags the lamp warming for about 20 s only, and the
/// image was still 6 levels lighter in the shadows; within 1 level after about 1.5 minutes,
/// 0.5 after 3. Measured again on 13 November 2026 after the scanner had switched the lamp off
/// by itself: the light parts 1.6 % darker at 2:12, 0.4 % at 2:38, within 0.1 % from 3:08.
/// Two and a half minutes, chosen by Matteo (two until 13 November).
pub const WARM_UP: Duration = Duration::from_secs(150);
/// After this long without a scan the lamp is taken to be off: the scanner switches it off by
/// itself after about 15 minutes idle (capture 21), without saying so.
pub const LAMP_OFF_AFTER: Duration = Duration::from_secs(14 * 60);

/// What the scanner measured when it was switched on: `1E 87`'s answer (the x origin's
/// correction and three more numbers). The same at every scan while it stays on (captures 03-37,
/// two days and 35 scans with the lamp off, warming, warm, `1B 40` and the lid's lamp; 38-43;
/// 61-65) and different after each power-on seen (five, every number of it: `a3 ff 1b 00 32
/// 01 cc`, `6a ff 45 00 08 01 c0`, `9e ff fd ff 1a 01 d8`, `9a ff 03 00 1a 01 d8`, `60 ff 60 00
/// fc 00 ba`): how a scan tells that the scanner was switched off and on since the last
/// ([`lamp_is_cold`]), which nothing else it asks shows once `1B 03`'s warming flag (about
/// 20 s) has gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerOn(pub [u8; 10]);

/// The lamp's last use by a scan of ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LampUse {
    /// When the scan ended: the lamp was lit then.
    pub ended: SystemTime,
    /// The power-on it was made in, if the scan got as far as asking (`1E 87`).
    pub power_on: Option<PowerOn>,
}

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
    /// The power-on it was made in.
    pub power_on: PowerOn,
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
    for (i, piece) in values.chunks(TABLE_CHUNK).enumerate() {
        if i > 0 {
            std::thread::sleep(TABLE_PAUSE);
        }
        t.bulk_out(piece)?;
    }
    ack(t, "a 1E 84 table")
}

fn ramp_bytes(r: &[u16]) -> Vec<u8> {
    r.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Polls `1B 03` until the scanner is idle with its lamp warm (`18 81` at rest
/// in every capture; `0x40` moving, `0x02` busy, `0x01` the lamp warming).
/// An answer that is not a state ([`is_state`]: bytes of an image a scanner is still
/// sending), or the scanner's own error flag (`0x80`, [`State::error`]), is an error at once.
/// A wait longer than one poll goes to `log`, with the states seen. Answers when it first
/// saw the lamp warming, if it did.
fn wait_ready(t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<Option<Instant>> {
    match wait_state(t, log)? {
        Waited::Ready(warming) => Ok(warming),
        Waited::Error(s) => Err(other(s.error_text())),
    }
}

/// What [`wait_state`] found.
enum Waited {
    /// At rest, with when it first saw the lamp warming, if it did.
    Ready(Option<Instant>),
    /// The scanner's own error flag ([`State::error`]).
    Error(State),
}

/// [`wait_ready`], the scanner's own error answered rather than turned into one.
fn wait_state(t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<Waited> {
    let start = Instant::now();
    let mut warming = None;
    let until = start + WAIT;
    let mut seen: Vec<(f64, [u8; 2])> = Vec::new();
    loop {
        let s = State(read_two(ask(t, GET_STATE, 2)?));
        if !is_state(&s.0) {
            return Err(other(format!(
                "1B 03 answered {:02X} {:02X}, not a state: a scan still under way (it is \
                 stopped when the scanner is opened again)",
                s.0[0], s.0[1]
            )));
        }
        if s.error() {
            return Ok(Waited::Error(s));
        }
        let at = start.elapsed().as_secs_f64();
        if s.0[0] & 0x01 != 0 && warming.is_none() {
            warming = Some(Instant::now());
        }
        if seen.last().is_none_or(|(_, last)| *last != s.0) {
            seen.push((at, s.0));
        }
        if s.firmware_running() && s.0[0] & 0x43 == 0 {
            if seen.len() > 1 {
                let states: Vec<String> = seen
                    .iter()
                    .map(|(t, s)| format!("{:02X} {:02X} at {t:.2} s", s[0], s[1]))
                    .collect();
                log(format!("1B 03: waited {at:.2} s: {}", states.join(", ")));
            }
            return Ok(Waited::Ready(warming));
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
    match lamp_in_use(t)? {
        0x01 => Ok(()),
        other_ => Err(lamp_error(other_)),
    }
}

/// `1E A1`'s answer: `01` the glass's lamp, `02` the lid's (after films, capture 25), `00`
/// none: switched off by the host with `1E A2 00` (12 November 2026; switched off by the
/// scanner itself after 15 minutes idle it still answers `01`).
fn lamp_in_use(t: &mut dyn Transport) -> Result<u8> {
    Ok(ask(t, [0x1E, 0xA1], 1)?[0])
}

fn lamp_error(answer: u8) -> Error {
    other(format!(
        "1E A1 answered {answer:02X}: the lid's lamp is in use (after films), not the glass's; \
         a scan of the glass with Epson's driver switches it back"
    ))
}

/// `1E A1` at the opening: `01` the glass's lamp, `00` none (switched off by the host, lit
/// again by the scan's `1E A2 01`), `02` the lid's (after films, capture 25, or after the
/// scanner was reset with `1B 40`, captures 66 and 68: switched back with `1E 77`).
fn lamp_at_opening(t: &mut dyn Transport) -> Result<u8> {
    match lamp_in_use(t)? {
        a @ (0x00..=0x02) => Ok(a),
        other_ => Err(other(format!(
            "1E A1 answered {other_:02X}: neither the glass's lamp (01), the lid's (02) nor none (00)"
        ))),
    }
}

/// How long Epson Scan waited after `1B 40` before asking the state again, on a scanner it
/// found in its own error (captures 66 and 68: 7 and 6 s; the carriages went home meanwhile,
/// Matteo saw).
const AFTER_RESET: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(7)
};

/// How often the state is asked while the scanner's error clears.
const RESET_POLL: Duration = if cfg!(test) {
    Duration::from_millis(1)
} else {
    Duration::from_secs(1)
};

/// The scanner's own error ([`State::ERROR`], `98 81`, its light flashing red) cleared as Epson
/// Scan clears it when opened then: `1B 40` (captures 66 and 68, 14 November 2026: `06`, the
/// carriages home, then `19 81`, the lid's lamp selected and warming; ready again in under
/// two minutes in 68-69, back in error by itself once, 66-67, with Epson Scan alone). Waited
/// for as a warm-up, asked every second and stoppable; an error again ends it, to be switched
/// off and on.
fn clear_error(
    t: &mut dyn Transport,
    s: State,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<()> {
    log(format!(
        "1B 03: {:02X} {:02X}, the scanner's own error: 1B 40, as Epson Scan clears it",
        s.0[0], s.0[1]
    ));
    command(t, [0x1B, 0x40])?;
    let start = Instant::now();
    let until = start + AFTER_RESET + WAIT;
    loop {
        let waited = start.elapsed();
        let left = (until.saturating_duration_since(Instant::now())).as_secs();
        if !events(Event::WarmingUp { left }) {
            return Err(Error::Cancelled);
        }
        if waited >= AFTER_RESET {
            let s = State(read_two(ask(t, GET_STATE, 2)?));
            if s.error() {
                return Err(other(format!(
                    "the scanner reports an error of its own again (1B 03 answered {:02X} {:02X}, \
                     its light flashing red): it came back after 1B 40; switch it off and on",
                    s.0[0], s.0[1]
                )));
            }
            if s.firmware_running() && s.0[0] & 0x43 == 0 {
                log(format!(
                    "the scanner's error cleared: ready after {:.0} s",
                    waited.as_secs_f64()
                ));
                return Ok(());
            }
            if Instant::now() > until {
                return Err(other(format!(
                    "after 1B 40 the scanner still answers {:02X} {:02X} to 1B 03 after {} s",
                    s.0[0],
                    s.0[1],
                    (AFTER_RESET + WAIT).as_secs()
                )));
            }
        }
        std::thread::sleep(RESET_POLL);
    }
}

/// After `1E A2 01` on a lamp that was off: `1E A1` polled until the glass's (`01`), as
/// Epson's driver did after lighting it when the lid's was in use (capture 25: about 8 s).
fn wait_lamp_lit(t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<()> {
    let start = Instant::now();
    loop {
        let a = lamp_in_use(t)?;
        if a == 0x01 {
            log(format!(
                "1E A1: the glass's lamp after {:.1} s",
                start.elapsed().as_secs_f64()
            ));
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(30) {
            return Err(lamp_error(a));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The longest the lid's lamp is waited for after `1E A2 02`: Epson Scan waited for the
/// scanner's warming flag, 18.8, 20.8 and 21.7 s after the lamp's `06` (captures 72, 73 and
/// 16: the glass's lamp warm, the scanner just switched on, the first films), polling `1B 03`
/// and `1E A1` every second; the `06` came 2.52 s after `1E A2 02` each time.
pub const LID_LAMP_WAIT: Duration = Duration::from_secs(60);

/// How often the lid's lamp is asked while it warms: Epson Scan's second.
const LID_LAMP_POLL: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(1)
};

/// Lights the lid's lamp for `request` as Epson Scan does at the start of a film scan (capture
/// 72, glass to film with the glass's lamp warm; 73, right after a power-on, the same): the
/// scanner's own error cleared first, `1B 40`, the state, `1E A1`, the gamma tables (the
/// identity, as Epson Scan's), `1E A1`, the film's `1E 57`, `1E A1`, then `1E A2 02` (the
/// scanner answers `06` only once it has switched lamps, 2.5 s in 72) and `1B 03` and `1E A1`
/// every second until the scanner no longer flags its lamp warming (`19 81` to `18 81`) and the
/// lid's lamp is in use (`02`). Nothing moves. Answers how long the lamp took from `1E A2 02`.
/// `events` hears `Event::WarmingUp` every second; `false` stops the wait (the lamp stays lit).
///
/// Whether the lid's lamp needs more time than the scanner's flag before a film's calibration,
/// as the glass's does ([`WARM_UP`]), is not measured: Epson Scan calibrates at once.
pub fn light_lid_lamp<T: Transport>(
    g: &mut Guarded<T>,
    request: &FilmRequest,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<Duration> {
    g.permit_film(request)?;
    let result = lid_lamp(g, request, log, events).map_err(|e| after(e, g.last_command()));
    g.end_scan();
    result
}

fn lid_lamp<T: Transport>(
    g: &mut Guarded<T>,
    request: &FilmRequest,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<Duration> {
    let e57 = crate::film::request_block(request)?;
    if let Waited::Error(s) = wait_state(g, log)? {
        clear_error(g, s, log, events)?;
        wait_ready(g, log)?;
    }
    command(g, [0x1B, 0x40])?;
    ask(g, [0x1E, 0x53], 42)?;
    wait_ready(g, log)?;
    let before = lamp_at_opening(g)?;
    wait_ready(g, log)?;
    let gamma = Curves::Linear.gamma();
    for channel in 0..3u8 {
        command(g, [0x1E, 0xB3])?;
        g.bulk_out(&[channel])?;
        g.bulk_out(&gamma)?;
    }
    wait_ready(g, log)?;
    lamp_at_opening(g)?;
    ask(g, [0x1E, 0x53], 42)?;
    send(g, [0x1E, 0x57], &e57)?;
    wait_ready(g, log)?;
    lamp_at_opening(g)?;
    let asked = Instant::now();
    send(g, [0x1E, 0xA2], &[0x02])?;
    log(format!(
        "the lid's lamp (1E A2 02; 1E A1 answered {before:02X} before): 06 after {:.1} s",
        asked.elapsed().as_secs_f64()
    ));
    let lit = Instant::now();
    loop {
        let waited = lit.elapsed();
        let s = State(read_two(ask(g, GET_STATE, 2)?));
        if !is_state(&s.0) {
            return Err(other(format!(
                "1B 03 answered {:02X} {:02X} while the lid's lamp warmed, not a state",
                s.0[0], s.0[1]
            )));
        }
        if s.error() {
            return Err(other(s.error_text()));
        }
        let lamp = lamp_in_use(g)?;
        if s.firmware_running() && s.0[0] & 0x43 == 0 && lamp == 0x02 {
            log(format!(
                "the lid's lamp lit: 1B 03 {:02X} {:02X}, 1E A1 02 after {:.1} s",
                s.0[0],
                s.0[1],
                waited.as_secs_f64()
            ));
            return Ok(waited);
        }
        if waited > LID_LAMP_WAIT {
            return Err(other(format!(
                "the lid's lamp: still 1B 03 {:02X} {:02X} and 1E A1 {lamp:02X} after {} s",
                s.0[0],
                s.0[1],
                LID_LAMP_WAIT.as_secs()
            )));
        }
        let left = LID_LAMP_WAIT.saturating_sub(waited).as_secs();
        if !events(Event::WarmingUp { left }) {
            return Err(Error::Cancelled);
        }
        std::thread::sleep(LID_LAMP_POLL);
    }
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
/// lines, each with its trailer; the data without the trailers. After each
/// block `on_block` hears the bytes read and their total: `false` stops the
/// read there (none).
fn read_lines(
    t: &mut dyn Transport,
    lines: u32,
    per: u32,
    line: u32,
    on_block: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<Option<Vec<u8>>> {
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
        if !on_block(data.len(), (lines * line) as usize) {
            return Ok(None);
        }
    }
    Ok(Some(data))
}

/// A calibration read: never stopped.
fn read_all(t: &mut dyn Transport, lines: u32, per: u32, line: u32) -> Result<Vec<u8>> {
    Ok(read_lines(t, lines, per, line, &mut |_, _| true)?.expect("never stopped"))
}

/// Stops a read under way as Epson's driver does (capture 11, a scan cancelled half-way):
/// no more data read, half a second, `1E 18` (answered `06` after about a second), then
/// `1B 03` polled while the carriage goes home, and `1E A1`.
///
/// The scanner may have put out some of the image already (a packet waiting in its endpoint):
/// it comes before the `06`, so the answer is read a packet at a time and what is not a lone
/// `06` is left out (an image block's trailer ends with `00`, so a lone `06` is the answer).
/// On the scanner (12 November 2026): 262,176 bytes of image came before the `06`.
fn cancel(t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<()> {
    std::thread::sleep(Duration::from_millis(500));
    stop(t, log)?;
    wait_ready(t, log)?;
    lamp_is_the_glass(t)?;
    log("cancelled (1E 18): the carriage went home".into());
    Ok(())
}

/// After a read failed (its `1E 47` sent, its data not all taken): stops it as a cancel does,
/// so that the scanner is not left sending its image to the next scan. The read's error is
/// kept; what the stop did goes to `log`.
fn stop_after<T: Transport>(g: &mut Guarded<T>, log: &mut dyn FnMut(String), e: Error) -> Error {
    // Where it failed, before the stop's own commands.
    let e = after(e, g.last_command());
    log(format!("the read failed ({e}): stopping it with 1E 18"));
    if let Err(also) = cancel(g, log) {
        log(format!("stopping it failed too: {also}"));
    }
    e
}

/// Stops the scan a scanner was left in by a program that went away half-way through its
/// read (closed, killed): `1E 18` written while reading ([`Transport::write_reading`]: such a
/// scanner may take a command only while what it sends is read, capture 52), the image it
/// still had and empty packets left out until its `06`, then `1B 03` until it is idle.
/// **Tried on 12 November 2026** (captures 44-53): the transport must be opened without the
/// STI lock (Epson's driver, given the lock, asks the state and takes image bytes for "no
/// firmware"), and the guard told ([`crate::guard::Guarded::permit_recovery`]).
pub fn recover(t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(60);
    let (mut left_out, mut reads, mut empty) = (0usize, 0usize, 0usize);
    let mut answered = false;
    t.write_reading(&CANCEL, &mut |data: &[u8]| {
        reads += 1;
        if data == [ACK] {
            answered = true;
            return false;
        }
        left_out += data.len();
        empty += usize::from(data.is_empty());
        empty <= MAX_EMPTY && Instant::now() < until && left_out <= CANCEL_MAX_LEFT_OUT
    })?;
    if !answered {
        return Err(other(format!(
            "after 1E 18 the scanner sent {left_out} bytes and no 06"
        )));
    }
    log(format!(
        "1E 18: {left_out} bytes of image left out in {reads} reads before its 06"
    ));
    wait_ready(t, log)?;
    Ok(())
}

/// `1E 18` and its answer: the image still on its way left out, up to a lone `06`.
///
/// A scanner left half-way by a program gone answered `1E 18` first with a packet of no
/// bytes (the end of the image transfer it had begun), then `06` (captures 48, 49): empty
/// reads are not an answer, the `06` is still waited for.
fn stop(t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<()> {
    t.bulk_out(&CANCEL)?;
    let until = Instant::now() + Duration::from_secs(60);
    let mut left_out = 0usize;
    let mut empty = 0usize;
    loop {
        let mut packet = [0u8; 512];
        let n = t.bulk_in(&mut packet)?;
        if packet[..n] == [ACK] {
            break;
        }
        left_out += n;
        if n == 0 {
            empty += 1;
        }
        if empty > MAX_EMPTY || Instant::now() > until || left_out > CANCEL_MAX_LEFT_OUT {
            return Err(other(format!(
                "after 1E 18 the scanner sent {left_out} bytes and no 06"
            )));
        }
    }
    if left_out > 0 {
        log(format!(
            "1E 18: {left_out} bytes of image left out before its 06"
        ));
    }
    Ok(())
}

/// Realigns the colours (green `offset` lines below blue, red twice) and crops
/// to the request: exactly what WIA returns from the same data (captures 03,
/// 04, 30). At 4800 dpi the pixels on even sensor columns come [`Plan::stagger`] lines later
/// as well (captures 59 and 61 against Epson Scan's TIFFs).
pub fn assemble(raw: &[u8], p: &Plan) -> Image {
    let r = p.request;
    let s = usize::from(r.bits / 8);
    let line = p.bytes_per_line as usize;
    let k = p.colour_offset() as usize;
    let offsets = [2 * k, k, 0];
    let stagger = p.stagger() as usize;
    let (w, h) = (r.width as usize, r.height as usize);
    let mut data = Vec::with_capacity(w * h * 3 * s);
    for y in 0..h {
        for x in 0..w {
            let late = if stagger > 0 && p.late(x as u32) {
                stagger
            } else {
                0
            };
            for (c, off) in offsets.iter().enumerate() {
                let at = (y + off + late) * line + (x * 3 + c) * s;
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

/// An image read along at `r.ydpi` brought to `r.dpi` lines an inch ([`Request::image_height`]
/// lines): each line the average of the lines read that it covers, weighted by how much of each
/// (at 3200 to 2400: 3/4 + 1/4, 1/2 + 1/2, 1/4 + 3/4 of two lines in turn). Epson Scan reads
/// 2400 dpi so and reduces on the computer (captures 13, 57, 58); how it reduces is not known,
/// this is an average with no sharpening. The same image when both are equal.
pub fn reduce_along(image: Image, r: &Request) -> Image {
    if r.ydpi == r.dpi {
        return image;
    }
    let s = usize::from(image.bits / 8);
    let row = image.width as usize * 3 * s;
    let samples = image.width as usize * 3;
    let h_in = image.height as usize;
    let h_out = r.image_height() as usize;
    // Input line i spans [i * dpi, (i + 1) * dpi), output line j [j * ydpi, (j + 1) * ydpi).
    let (dpi, ydpi) = (u64::from(r.dpi), u64::from(r.ydpi));
    let read = |line: &[u8], k: usize| -> u64 {
        if s == 2 {
            u64::from(u16::from_le_bytes([line[2 * k], line[2 * k + 1]]))
        } else {
            u64::from(line[k])
        }
    };
    let mut data = Vec::with_capacity(h_out * row);
    let mut sum = vec![0u64; samples];
    for j in 0..h_out as u64 {
        let (start, end) = (j * ydpi, (j + 1) * ydpi);
        sum.fill(0);
        let mut total = 0;
        for i in start / dpi..end.div_ceil(dpi) {
            let w = end.min((i + 1) * dpi) - start.max(i * dpi);
            let i = (i as usize).min(h_in - 1);
            let line = &image.data[i * row..(i + 1) * row];
            for (k, acc) in sum.iter_mut().enumerate() {
                *acc += w * read(line, k);
            }
            total += w;
        }
        for v in &sum {
            let v = (v + total / 2) / total;
            if s == 2 {
                data.extend_from_slice(&(v as u16).to_le_bytes());
            } else {
                data.push(v as u8);
            }
        }
    }
    Image {
        height: h_out as u32,
        data,
        ..image
    }
}

/// Whether the lamp must be given [`WARM_UP`] before the calibration: the scanner flagged it
/// warming, no scan of ours ended in the last [`LAMP_OFF_AFTER`] (`last_used`; none known
/// counts as long ago, as does a clock gone back), or the scanner was switched off and on since
/// it (`power_on` not the last scan's, where that is known).
pub fn lamp_is_cold(
    warming_seen: bool,
    last_used: Option<LampUse>,
    power_on: PowerOn,
    now: SystemTime,
) -> bool {
    let Some(last) = last_used else {
        return true;
    };
    let idle = now.duration_since(last.ended).ok();
    warming_seen
        || idle.is_none_or(|idle| idle >= LAMP_OFF_AFTER)
        || last.power_on.is_some_and(|p| p != power_on)
}

/// Waits until [`WARM_UP`] after `lit`, polling `1B 03` every second. `events` may stop it:
/// nothing has moved yet.
fn warm_up(
    t: &mut dyn Transport,
    lit: Instant,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<()> {
    let until = lit + WARM_UP;
    let mut said = None;
    while Instant::now() < until {
        let left = (until - Instant::now()).as_secs();
        if said.is_none_or(|s: u64| s >= left + 30) {
            log(format!("the lamp was off: warming up, {left} s left"));
            said = Some(left);
        }
        if !events(Event::WarmingUp { left }) {
            return Err(Error::Cancelled);
        }
        ask(t, GET_STATE, 2)?;
        std::thread::sleep(
            Duration::from_secs(1).min(until.saturating_duration_since(Instant::now())),
        );
    }
    Ok(())
}

/// How a scan is made, besides its request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// When a scan of ours last ended (the lamp was lit then) and in which power-on, if known:
    /// a lamp taken to be cold ([`lamp_is_cold`]) gets [`WARM_UP`] first.
    pub last_used: Option<LampUse>,
    /// The tables loaded into the scanner.
    pub curves: Curves,
}

/// Where a scan is, for whoever shows it; the answer `false` stops it where that is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A cold lamp being given its time, seconds left (stopping: nothing moved yet).
    WarmingUp { left: u64 },
    /// The calibration starts (stopping: nothing moved yet).
    Calibrating,
    /// Image data, bytes read so far and their total (stopping: `1E 18`, as Epson's driver).
    Reading { done: usize, total: usize },
}

/// Scans `request` on the glass. `log` gets a line for each phase, `events` where the scan
/// is; when `events` answers `false` the scan stops at the next safe place and ends with
/// [`Error::Cancelled`].
pub fn scan_glass<T: Transport>(
    g: &mut Guarded<T>,
    request: Request,
    options: Options,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<Scanned> {
    g.permit_request(&request)?;
    let result = run(g, request, options, log, events).map_err(|e| after(e, g.last_command()));
    g.end_scan();
    result
}

/// A scan's error with the last command sent before it, to tell where it stopped (the
/// scanner's own error came once before the lamp was lit, with a dozen commands there).
fn after(e: Error, last: Option<[u8; 2]>) -> Error {
    if e.to_string().contains("(after ") {
        return e;
    }
    match (e, last) {
        (Error::Other(what), Some(cmd)) => Error::Other(format!("{what} (after {})", name(cmd))),
        // A transfer that failed or timed out (`nusb` says a timeout as "cancelled").
        (Error::Disconnected(what), Some(cmd)) => {
            Error::Disconnected(format!("{what} (after {})", name(cmd)))
        }
        (e, _) => e,
    }
}

fn run<T: Transport>(
    g: &mut Guarded<T>,
    request: Request,
    options: Options,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<Scanned> {
    let Options { last_used, curves } = options;
    let e57 = setup::request_block(&request)?;
    // The opening, as WIA's driver does it; the scanner's own error cleared first, as Epson
    // Scan does, its lamp then warmed again.
    let (warming, reset) = match wait_state(g, log)? {
        Waited::Ready(warming) => (warming, false),
        Waited::Error(s) => {
            clear_error(g, s, log, events)?;
            (wait_ready(g, log)?, true)
        }
    };
    ask(g, [0x1E, 0x53], 42)?;
    wait_ready(g, log)?;
    command(g, [0x1B, 0x40])?;
    ask(g, [0x1E, 0x53], 42)?;
    wait_ready(g, log)?;
    let id = ask(g, [0x1B, 0x13], IDENTITY.len())?;
    if id != IDENTITY {
        return Err(other(format!(
            "1B 13 answered {:?}, not {:?}: a model or firmware never seen",
            String::from_utf8_lossy(&id),
            String::from_utf8_lossy(IDENTITY)
        )));
    }
    // Any lamp but the glass's, or none: lit with `1E A2 01` below and waited for.
    let mut lamp_was_off = reset;
    for _ in 0..2 {
        wait_ready(g, log)?;
        lamp_was_off |= lamp_at_opening(g)? != 0x01;
    }
    wait_ready(g, log)?;
    command(g, [0x1B, 0x40])?;
    ask(g, [0x1E, 0x53], 42)?;
    wait_ready(g, log)?;
    let lamp = lamp_at_opening(g)?;
    lamp_was_off |= lamp != 0x01;
    if lamp == 0x02 {
        // The lid's lamp in use: WIA's driver sends 1E 77 here, then lights the glass's with
        // 1E A2 01 and asks 1E A1 until 01 (capture 25, a glass scan after films).
        command(g, [0x1E, 0x77])?;
        log("the lid's lamp in use: 1E 77, as WIA's driver".into());
        // Then the state and the lamp asked again (still 02 until 1E A2 01).
        wait_ready(g, log)?;
        lamp_at_opening(g)?;
    }
    let gamma = curves.gamma();
    for channel in 0..3u8 {
        command(g, [0x1E, 0xB3])?;
        g.bulk_out(&[channel])?;
        g.bulk_out(&gamma)?;
    }
    wait_ready(g, log)?;
    lamp_was_off |= lamp_at_opening(g)? != 0x01;
    ask(g, [0x1E, 0x53], 42)?;
    send(g, [0x1E, 0x57], &e57)?;
    wait_ready(g, log)?;
    lamp_was_off |= lamp_at_opening(g)? != 0x01;
    send(g, [0x1E, 0xA2], &[0x01])?;
    let lit = Instant::now();
    log("lamp on (1E A2 01)".into());
    if lamp_was_off {
        wait_lamp_lit(g, log)?;
    }
    let warming = warming.or(wait_ready(g, log)?);

    // The x origin's correction, stored in the scanner since it was switched on; then the
    // plan. Asked before the warm-up (Epson's drivers ask it once the lamp answers, with no
    // warm-up of their own): its answer tells a power-on since the last scan.
    command(g, [0x1E, 0x87])?;
    g.bulk_out(&[0x00])?;
    let a87 = read_exact(g, 10, "1E 87")?;
    let xc = XCorrection::from_1e87(&a87)?;
    let power_on = PowerOn(a87.as_slice().try_into().expect("10 bytes"));
    let p = Plan::new(request, xc)?;
    g.permit_plan(&p)?;
    if last_used
        .and_then(|l| l.power_on)
        .is_some_and(|p| p != power_on)
    {
        log("1E 87 answered otherwise than at the last scan: the scanner was switched off and on since"
            .into());
    }
    if lamp_was_off || lamp_is_cold(warming.is_some(), last_used, power_on, SystemTime::now()) {
        // Lit when the scanner was switched on (flagged warming) or just now.
        warm_up(g, warming.unwrap_or(lit).min(lit), log, events)?;
    }
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
    // The carriage at home (0 in every capture): the calibration's move is counted from there.
    let home = ask(g, [0x1E, 0x86], 4)?;
    let home = u16::from_le_bytes([home[0], home[1]]);
    if home != 0 {
        return Err(other(format!(
            "before the calibration the carriage is {home} steps from home: switch the scanner \
             off and on"
        )));
    }
    send(g, [0x1E, 0x46], &p.e46[0])?;
    wait_04(g)?;
    send(g, [0x1E, 0x46], &p.e46[1])?;
    wait_04(g)?;
    let back = ask(g, [0x1E, 0x92], 12)?;
    ask(g, [0x1E, 0x8C], 1)?;

    // The sensor's black pixels: 4 lines, still.
    if !events(Event::Calibrating) {
        return Err(Error::Cancelled);
    }
    let zeros = [0u8; 18];
    setup_read(
        g,
        &p,
        &back,
        &p.e21_black,
        &p.e41_black,
        &zeros,
        &setup::GAIN_CAL,
    )?;
    wait_ready(g, log)?;
    let black_cols = p.black_column_count();
    let black = read_all(g, 4, 4, black_cols * 6).map_err(|e| stop_after(g, log, e))?;
    let e42_black = black_block(&black);

    // The white strip: 16 lines at 16 bits, the carriage moving 160 steps.
    wait_ready(g, log)?;
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
    wait_ready(g, log)?;
    let cols = p.column_count();
    let white = read_all(g, 16, 16, cols * 6).map_err(|e| stop_after(g, log, e))?;

    // The lamp off, the dark, still.
    wait_ready(g, log)?;
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
    wait_ready(g, log)?;
    let dark = read_all(g, 16, 16, cols * 6).map_err(|e| stop_after(g, log, e))?;
    wait_ready(g, log)?;
    send(g, [0x1E, 0xA2], &[0x01])?;
    log("calibration read: black pixels, white, dark; lamp on again".into());

    if p.asks_table07() {
        // 1E 8B: 06, the scanner holds its 1200 dpi table (it keeps it until switched off),
        // read back as Epson's driver does (captures 33, 37); 15, it does not: computed
        // from a raw line of the sensor and loaded (captures 05, 42; `table07`).
        let a = ask(g, [0x1E, 0x8B], 1)?;
        wait_ready(g, log)?;
        match a[0] {
            ACK => {
                command(g, [0x1E, 0x83])?;
                g.bulk_out(&READ_83)?;
                read_exact(g, 600, "1E 83")?;
            }
            0x15 => {
                command(g, [0x1E, 0x83])?;
                g.bulk_out(&table07::READ_RAW)?;
                let raw = read_exact(g, table07::RAW_LEN, "1E 83 (the raw line)")?;
                command(g, [0x1E, 0x87])?;
                g.bulk_out(&[0x00])?;
                let xc = XCorrection::from_1e87(&read_exact(g, 10, "1E 87")?)?;
                table(g, &table07::HEADER, &table07::compute(&raw, xc))?;
                log("1E 8B answered 15: the 1200 dpi table computed and loaded".into());
            }
            other_ => {
                return Err(other(format!(
                    "1E 8B answered {other_:02X}, neither 06 nor 15"
                )))
            }
        }
    }
    let at = ask(g, [0x1E, 0x86], 4)?;
    let at = u16::from_le_bytes([at[0], at[1]]);
    if u32::from(at) != p.after_calibration {
        return Err(other(format!(
            "after the calibration the carriage is at {at}, not at {}",
            p.after_calibration
        )));
    }
    let tone = curves.tone();
    for header in &tables::TONE_HEADERS {
        table(g, header, &tone)?;
    }

    // The move to the lead.
    if let Some(m) = &p.start_move {
        table(g, &setup::ramp_header(m.ramp.len()), &ramp_bytes(&m.ramp))?;
        send(g, [0x1E, 0x01], &m.block)?;
        command(g, [0x1E, 0x05])?;
        wait_ready(g, log)?;
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
    wait_ready(g, log)?;
    log(format!("reading {} lines", p.lines));
    let raw = read_lines(
        g,
        p.lines,
        p.lines_per_block,
        p.bytes_per_line,
        &mut |done, total| events(Event::Reading { done, total }),
    )
    .map_err(|e| stop_after(g, log, e))?;
    let Some(raw) = raw else {
        cancel(g, log)?;
        return Err(Error::Cancelled);
    };
    wait_ready(g, log)?;
    lamp_is_the_glass(g)?;
    let image = reduce_along(assemble(&raw, &p), &p.request);
    Ok(Scanned {
        image,
        plan: p,
        shading,
        raw,
        power_on,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::setup::tests::{parse, request_of, Line, CAPTURES};
    use crate::table07;
    use std::collections::VecDeque;

    /// A scanner that answers as in a capture and checks every write against
    /// Epson's driver's, in order.
    #[derive(Debug)]
    pub(crate) struct Follow {
        name: &'static str,
        pub(crate) lines: Vec<Line>,
        pub(crate) at: usize,
        answer: VecDeque<u8>,
        last_cmd: [u8; 2],
        pub(crate) writes: usize,
        /// The tables the scan is expected to load where the capture loaded WIA's.
        pub(crate) curves: Curves,
    }

    impl Follow {
        pub(crate) fn new(name: &'static str, text: &str) -> Self {
            Self {
                name,
                lines: parse(text),
                at: 0,
                answer: VecDeque::new(),
                last_cmd: [0; 2],
                writes: 0,
                curves: Curves::Wia,
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
                    ["gamma"] => data == self.curves.gamma(),
                    ["tone"] => data == self.curves.tone(),
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

    /// The lamp used by a scan that just ended, in a power-on not known.
    pub(crate) fn just_used() -> Option<LampUse> {
        Some(LampUse {
            ended: SystemTime::now(),
            power_on: None,
        })
    }

    /// Scans a capture's request on a scanner answering as in it: the result, then
    /// how far into the capture it got and how many writes it made.
    fn scan(name: &'static str, text: &str) -> (Result<Scanned>, usize, usize) {
        let (request, _, _) = request_of(&parse(text));
        let mut g = Guarded::new(Follow::new(name, text));
        let r = scan_glass(
            &mut g,
            request,
            Options {
                last_used: just_used(),
                curves: Curves::Wia,
            },
            &mut |_| {},
            &mut |_| true,
        );
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

    /// At 1200 dpi, a scanner without its table (`1E 8B` answering `15`, capture 05): the
    /// raw line read, `1E 87`, the table computed and written as Epson's driver wrote it,
    /// then the scan as captured.
    #[test]
    fn without_its_1200_dpi_table_the_scanner_gets_it_computed() {
        let (_, text) = CAPTURES.iter().find(|(n, _)| *n == "05").unwrap();
        let raw = include_bytes!("../testdata/05_1e83_7c3d60.bin");
        assert_eq!(raw.len(), table07::RAW_LEN);
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        // The raw line's reads, from after its header up to 1E 87, become one answer.
        let mut out = Vec::new();
        let mut skipping = false;
        for line in text.lines() {
            if skipping && line == "W 1e87" {
                out.push(format!("R {hex}"));
                skipping = false;
            }
            if !skipping {
                out.push(line.to_string());
            }
            if line == "W 07603d7c0040bc03" {
                skipping = true;
            }
        }
        let text = out.join(
            "
",
        );
        let text: &'static str = Box::leak(text.into_boxed_str());
        let (r, _, writes) = scan("05", text);
        let s = r.unwrap_or_else(|e| panic!("{e}"));
        assert!(writes > 100);
        assert_eq!((s.image.width, s.image.height), (945, 945));
    }

    #[test]
    fn a_lamp_is_cold_when_flagged_or_idle_long_enough() {
        let now = SystemTime::now();
        let this = PowerOn([1, 0, 0xA3, 0xFF, 0x1B, 0, 0x32, 1, 0xCC, 0]);
        let other = PowerOn([1, 0, 0x6A, 0xFF, 0x45, 0, 0x08, 1, 0xC0, 0]);
        let used = |ended: SystemTime, power_on| Some(LampUse { ended, power_on });
        let ago = |s: u64| used(now - Duration::from_secs(s), None);
        assert!(!lamp_is_cold(false, ago(60), this, now));
        assert!(!lamp_is_cold(false, ago(13 * 60), this, now));
        assert!(lamp_is_cold(false, ago(14 * 60), this, now));
        assert!(lamp_is_cold(true, ago(60), this, now));
        // Never used by us, or a clock gone back: cold.
        assert!(lamp_is_cold(false, None, this, now));
        assert!(lamp_is_cold(
            false,
            used(now + Duration::from_secs(60), None),
            this,
            now
        ));
        // A minute ago in this power-on: warm; in another one (switched off and on since): cold.
        let minute = now - Duration::from_secs(60);
        assert!(!lamp_is_cold(false, used(minute, Some(this)), this, now));
        assert!(lamp_is_cold(false, used(minute, Some(other)), this, now));
    }

    /// A scan a minute after one of ours, the scanner not flagging its lamp warming: no warm-up
    /// in the same power-on (`1E 87` answering as at the last scan), the warm-up after a power
    /// cycle (`1E 87` answering otherwise), said in the log.
    #[test]
    fn a_power_cycle_since_the_last_scan_warms_the_lamp() {
        let (_, text) = CAPTURES.iter().find(|(n, _)| *n == "29").unwrap();
        let (request, _, _) = request_of(&parse(text));
        let (r, _, _) = scan("29", text);
        let this = r.unwrap().power_on;
        assert_eq!(this.0[2..4], [0xA3, 0xFF], "-93, capture 29's x correction");
        let mut other = this;
        other.0[4] ^= 0x40;
        let minute_ago = SystemTime::now() - Duration::from_secs(60);
        for (power_on, warms) in [(this, false), (other, true)] {
            let mut g = Guarded::new(Follow::new("29", text));
            let mut said = Vec::new();
            let mut warmed = false;
            let r = scan_glass(
                &mut g,
                request,
                Options {
                    last_used: Some(LampUse {
                        ended: minute_ago,
                        power_on: Some(power_on),
                    }),
                    curves: Curves::Wia,
                },
                &mut |l| said.push(l),
                // Stopped at the warm-up, if there is one.
                &mut |e| {
                    warmed |= matches!(e, Event::WarmingUp { .. });
                    !warmed
                },
            );
            assert_eq!(warmed, warms);
            assert_eq!(r.is_ok(), !warms);
            assert_eq!(
                said.iter().any(|l| l.contains("switched off and on")),
                warms,
                "{said:?}"
            );
        }
    }

    /// A capture's scanner until `1E 18`, then the scanner of capture 11 after it: `06`,
    /// the state at rest, the glass's lamp.
    #[derive(Debug)]
    struct Cancelling {
        follow: Follow,
        after: Vec<Vec<u8>>,
        answers: VecDeque<Vec<u8>>,
        /// Packets of image the scanner puts out before `1E 18`'s answer.
        leftover: usize,
    }

    impl Transport for Cancelling {
        fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
            if self.after.is_empty() && data != CANCEL {
                return self.follow.bulk_out(data);
            }
            self.after.push(data.to_vec());
            if data == CANCEL {
                for _ in 0..self.leftover {
                    self.answers.push_back(vec![0x5A; 512]);
                }
            }
            self.answers.push_back(match data {
                [0x1E, 0x18] => vec![ACK],
                [0x1B, 0x03] => vec![0x18, 0x81],
                [0x1E, 0xA1] => vec![0x01],
                _ => return Err(Error::Other(format!("after 1E 18: {data:02x?}"))),
            });
            Ok(())
        }

        fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
            if self.after.is_empty() {
                return self.follow.bulk_in(buf);
            }
            // One USB packet a read, as the scanner gives them.
            let Some(mut a) = self.answers.pop_front() else {
                return Err(Error::Other("no answer".into()));
            };
            let n = buf.len().min(a.len());
            buf[..n].copy_from_slice(&a[..n]);
            if n < a.len() {
                a.drain(..n);
                self.answers.push_front(a);
            }
            Ok(n)
        }
    }

    /// Stopped during the read: `1E 18`, the state until the carriage is home, `1E A1`, as
    /// Epson's driver (capture 11); stopped before the calibration: nothing more sent.
    #[test]
    fn a_scan_is_cancelled_as_epsons_driver_does() {
        let (_, text) = CAPTURES.iter().find(|(n, _)| *n == "29").unwrap();
        let (request, _, _) = request_of(&parse(text));
        let options = Options {
            last_used: just_used(),
            curves: Curves::Wia,
        };
        // The answer right away, or after two packets of image already put out.
        for leftover in [0, 2] {
            let mut g = Guarded::new(Cancelling {
                follow: Follow::new("29", text),
                after: Vec::new(),
                answers: VecDeque::new(),
                leftover,
            });
            let mut blocks = 0;
            let mut said = Vec::new();
            let r = scan_glass(
                &mut g,
                request,
                options,
                &mut |l| said.push(l),
                &mut |e| match e {
                    Event::Reading { .. } => {
                        blocks += 1;
                        blocks < 3
                    }
                    _ => true,
                },
            );
            assert_eq!(r.unwrap_err(), Error::Cancelled);
            assert_eq!(blocks, 3);
            assert_eq!(
                g.inner().after,
                [CANCEL.to_vec(), GET_STATE.to_vec(), vec![0x1E, 0xA1]]
            );
            assert_eq!(
                said.iter()
                    .any(|l| l.contains("1024 bytes of image left out")),
                leftover == 2
            );
            // The guard is back to what moves nothing.
            assert!(g.bulk_out(&[0x1E, 0x47]).is_err());
        }

        let mut g = Guarded::new(Follow::new("29", text));
        let r = scan_glass(&mut g, request, options, &mut |_| {}, &mut |e| {
            e != Event::Calibrating
        });
        assert_eq!(r.unwrap_err(), Error::Cancelled);
        // The last write was 1E 8C's, before the calibration's set-up.
        let lines = &g.inner().lines;
        let at = g.inner().at;
        assert!(
            matches!(&lines[at - 2], Line::W(b, None) if b == &[0x1E, 0x8C]),
            "{:?}",
            &lines[at - 3..at]
        );
    }

    /// A capture's scanner that answers its `at`-th `1B 03` with its own error (`98 81`, seen
    /// on 13 November 2026); what is written after it is kept.
    #[derive(Debug)]
    struct Erring {
        follow: Follow,
        states: usize,
        at: usize,
        answer: Option<Vec<u8>>,
        after: Option<Vec<Vec<u8>>>,
    }

    impl Transport for Erring {
        fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
            if let Some(after) = &mut self.after {
                after.push(data.to_vec());
                return Ok(());
            }
            if data == GET_STATE {
                self.states += 1;
                if self.states == self.at {
                    self.answer = Some(vec![0x98, 0x81]);
                    self.after = Some(Vec::new());
                    return Ok(());
                }
            }
            self.follow.bulk_out(data)
        }

        fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
            match self.answer.take() {
                Some(a) => {
                    buf[..a.len()].copy_from_slice(&a);
                    Ok(a.len())
                }
                None => self.follow.bulk_in(buf),
            }
        }
    }

    /// With the lid's lamp in use (after films), the opening writes what WIA's driver wrote in
    /// capture 25 as far as the glass's lamp lit: `1E 77`, the state and the lamp asked again,
    /// the tables, `1E A2 01`; WIA then waits for the lamp asking `1B 03` and `1E A1` in turn,
    /// ours with its own warm-up.
    #[test]
    fn the_lids_lamp_is_switched_back_as_wias_driver_does() {
        let (_, text) = CAPTURES.iter().find(|(n, _)| *n == "25").unwrap();
        let (r, at, _) = scan("25", text);
        let e = r.unwrap_err().to_string();
        assert!(at >= 59, "stopped at line {at}: {e}");
        assert!(e.contains("(after 1E A2)"), "{e}");
    }

    /// The lid's lamp is lit writing what Epson Scan wrote in capture 72, write by write, as far
    /// as the scanner flagged it warm.
    #[test]
    fn the_lids_lamp_is_lit_as_epson_scan_lights_it() {
        let text = include_str!("../testdata/72_lid_lamp.txt");
        let preview = crate::film::tests::CAPTURED[0].1;
        let mut g = Guarded::new(Follow::new("72", text));
        let mut warming = 0;
        let waited = light_lid_lamp(&mut g, &preview, &mut |_| {}, &mut |e| {
            warming += usize::from(matches!(e, Event::WarmingUp { .. }));
            true
        })
        .unwrap_or_else(|e| panic!("{e}"));
        let f = g.inner();
        assert_eq!(f.at, f.lines.len(), "lines of the capture left");
        // One for each 19 81 the capture answered.
        assert_eq!(warming, 20);
        assert!(waited < LID_LAMP_WAIT);
        // Over: the guard lets out no film's command any more.
        assert!(g.bulk_out(&[0x1E, 0x57]).is_err());
    }

    /// Stopped while the lid's lamp warms: nothing more is sent.
    #[test]
    fn the_lids_lamp_wait_can_be_stopped() {
        let text = include_str!("../testdata/72_lid_lamp.txt");
        let preview = crate::film::tests::CAPTURED[0].1;
        let mut g = Guarded::new(Follow::new("72", text));
        let r = light_lid_lamp(&mut g, &preview, &mut |_| {}, &mut |_| false);
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
        // The first 1B 03 and 1E A1 after 1E A2 02 (19 81, 02), then nothing.
        let f = g.inner();
        let lines = parse(text);
        assert!(matches!(&lines[f.at - 1], Line::R(r) if r == &[0x02]));
        assert!(matches!(&lines[f.at - 3], Line::P(_, s) if s == &[0x19, 0x81]));
    }

    /// A film request never captured lights nothing.
    #[test]
    fn the_lids_lamp_only_for_a_film_request_captured() {
        use crate::protocol::tests::Mock;
        let mut preview = crate::film::tests::CAPTURED[0].1;
        preview.bits = 16;
        let mut g = Guarded::new(Mock::answering(&[]));
        assert!(light_lid_lamp(&mut g, &preview, &mut |_| {}, &mut |_| true).is_err());
        assert!(g.inner().writes.is_empty());
    }

    /// The scanner's own error, found at the opening, is cleared with `1B 40` as Epson Scan
    /// clears it (captures 66, 68), the state asked until at rest; an error again ends it.
    #[test]
    fn the_scanners_own_error_at_the_opening_is_cleared_with_1b_40() {
        use crate::protocol::tests::Mock;
        let error = State([0x98, 0x81]);
        let mut t = Mock::answering(&[&[ACK], &[0x19, 0x81], &[0x18, 0x81]]);
        clear_error(&mut t, error, &mut |_| {}, &mut |_| true).unwrap();
        assert_eq!(
            t.writes,
            [vec![0x1B, 0x40], GET_STATE.to_vec(), GET_STATE.to_vec()]
        );
        let mut t = Mock::answering(&[&[ACK], &[0x19, 0x81], &[0x98, 0x81]]);
        let e = clear_error(&mut t, error, &mut |_| {}, &mut |_| true)
            .unwrap_err()
            .to_string();
        assert!(e.contains("came back after 1B 40"), "{e}");
        // Stopped while it clears: nothing more asked.
        let mut t = Mock::answering(&[&[ACK]]);
        let r = clear_error(&mut t, error, &mut |_| {}, &mut |_| false);
        assert!(matches!(r, Err(Error::Cancelled)));
        assert_eq!(t.writes, [vec![0x1B, 0x40]]);
    }

    /// The scanner's own error flag stops the scan where it is seen after the opening, with
    /// nothing more sent: before the lamp, in the calibration.
    #[test]
    fn the_scanners_own_error_stops_the_scan() {
        let (_, text) = CAPTURES.iter().find(|(n, _)| *n == "29").unwrap();
        let (request, _, _) = request_of(&parse(text));
        let options = Options {
            last_used: just_used(),
            curves: Curves::Wia,
        };
        for at in [4, 8, 12] {
            let mut g = Guarded::new(Erring {
                follow: Follow::new("29", text),
                states: 0,
                at,
                answer: None,
                after: None,
            });
            let r = scan_glass(&mut g, request, options, &mut |_| {}, &mut |_| true);
            let e = r.unwrap_err().to_string();
            assert!(e.contains("98 81"), "at {at}: {e}");
            assert_eq!(g.inner().after, Some(Vec::new()), "at {at}");
            // Where it stopped: 1B 13 before the fourth state.
            match at {
                4 => assert!(e.ends_with("(after 1B 13)"), "{e}"),
                _ => assert!(e.contains("(after 1E "), "at {at}: {e}"),
            }
        }
    }

    /// A capture's scanner whose `n`-th read (`1E 47`) fails at its first block.
    #[derive(Debug)]
    struct FailingRead {
        inner: Cancelling,
        e47: usize,
        fail_at: usize,
    }

    impl Transport for FailingRead {
        fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
            if data == [0x1E, 0x47] {
                self.e47 += 1;
            }
            self.inner.bulk_out(data)
        }

        fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
            if self.e47 == self.fail_at {
                self.fail_at = 0;
                return Err(Error::Disconnected(
                    "bulk IN: transfer was cancelled".into(),
                ));
            }
            self.inner.bulk_in(buf)
        }
    }

    /// A read that fails is stopped with 1E 18, as a cancel, in the calibration (the white
    /// strip, the second read) and in the image (the fourth); the error is the read's.
    #[test]
    fn a_failed_read_is_stopped_with_1e_18() {
        let (_, text) = CAPTURES.iter().find(|(n, _)| *n == "29").unwrap();
        let (request, _, _) = request_of(&parse(text));
        for fail_at in [2, 4] {
            let mut g = Guarded::new(FailingRead {
                inner: Cancelling {
                    follow: Follow::new("29", text),
                    after: Vec::new(),
                    answers: VecDeque::new(),
                    leftover: 0,
                },
                e47: 0,
                fail_at,
            });
            let mut said = Vec::new();
            let r = scan_glass(
                &mut g,
                request,
                Options {
                    last_used: just_used(),
                    curves: Curves::Wia,
                },
                &mut |l| said.push(l),
                &mut |_| true,
            );
            let e = r.unwrap_err().to_string();
            // Where it failed, not the stop's last command (1E A1).
            assert!(
                e.starts_with("bulk IN: transfer was cancelled (after 1E "),
                "{e}"
            );
            assert!(!e.contains("(after 1E A1)"), "{e}");
            assert_eq!(
                g.inner().inner.after,
                [CANCEL.to_vec(), GET_STATE.to_vec(), vec![0x1E, 0xA1]],
                "read {fail_at}"
            );
            assert!(said.iter().any(|l| l.contains("stopping it with 1E 18")));
        }
    }

    /// A carriage not at home before the calibration: nothing moves, the scan stops there.
    #[test]
    fn a_carriage_away_from_home_is_not_calibrated() {
        let (_, text) = CAPTURES.iter().find(|(n, _)| *n == "29").unwrap();
        // The first 1E 86 answers 160 steps out instead of 0.
        let at = text.find("W 1e86").expect("a 1E 86");
        let moved = format!(
            "{}{}",
            &text[..at],
            text[at..].replacen("A 00000000", "A a0000000", 1)
        );
        assert_ne!(moved.as_str(), *text);
        let text: &'static str = Box::leak(moved.into_boxed_str());
        let (r, _, _) = scan("29", text);
        let e = r.unwrap_err().to_string();
        assert!(e.contains("160 steps from home"), "{e}");
    }

    #[test]
    fn at_4800_dpi_the_even_sensor_columns_come_later() {
        // Line i of the read worth i in every sample: a pixel on an even sensor column comes
        // from 8 lines further down (blue at 0, green 96, red 192 as at any resolution).
        let p = Plan::new(Request::new(4800, 0, 3747, 400, 4, 8), XCorrection(-160)).unwrap();
        assert_eq!((p.columns.0 % 2, p.stagger()), (0, 8));
        let line = p.bytes_per_line as usize;
        let raw: Vec<u8> = (0..p.lines as usize * line)
            .map(|i| (i / line) as u8)
            .collect();
        let im = assemble(&raw, &p);
        // Pixel 0 (even sensor column) and 1 (odd) of line 0: red, green, blue.
        assert_eq!(im.data[..6], [200, 104, 8, 192, 96, 0]);
    }

    #[test]
    fn the_2400_dpi_are_read_at_3200_along_and_reduced() {
        // As Epson Scan asked for capture 57's 1889 x 2834 and 13's 944 x 944.
        let r = Request::glass(2400, 10, 100, 1889, 2834, 16);
        assert_eq!((r.dpi, r.ydpi, r.y, r.height), (2400, 3200, 133, 3779));
        assert_eq!(r.image_height(), 2834);
        assert_eq!(Request::glass(2400, 0, 0, 944, 944, 16).height, 1259);
        assert_eq!(
            Request::glass(600, 0, 9, 10, 10, 8),
            Request::new(600, 0, 9, 10, 10, 8)
        );
        // Eight lines read, line i worth 30 i: six lines out, each the lines it covers
        // weighted (3/4 + 1/4, 1/2 + 1/2, 1/4 + 3/4), rounded.
        let r = Request::glass(2400, 0, 0, 1, 6, 8);
        assert_eq!((r.height, r.image_height()), (8, 6));
        let read = |bits: u8, unit: u32| Image {
            width: 1,
            height: 8,
            bits,
            data: (0..8u32)
                .flat_map(|i| {
                    let v = 30 * i * unit;
                    if bits == 16 {
                        [v as u16; 3].iter().flat_map(|s| s.to_le_bytes()).collect()
                    } else {
                        vec![v as u8; 3]
                    }
                })
                .collect(),
        };
        let out = reduce_along(read(8, 1), &r);
        assert_eq!(out.height, 6);
        let firsts: Vec<u8> = out.data.chunks(3).map(|p| p[0]).collect();
        assert_eq!(firsts, [8, 45, 83, 128, 165, 203]);
        let r16 = Request { bits: 16, ..r };
        let out = reduce_along(read(16, 100), &r16);
        let firsts: Vec<u16> = out
            .data
            .chunks(6)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        assert_eq!(firsts, [750, 4500, 8250, 12750, 16500, 20250]);
        // The same resolution both ways: nothing done.
        let same = read(8, 1);
        let r = Request::new(600, 0, 0, 1, 8, 8);
        assert_eq!(reduce_along(same.clone(), &r).data, same.data);
    }

    #[test]
    fn a_long_table_goes_in_pieces_with_a_pause_between() {
        use crate::protocol::tests::Mock;
        /// Records when each write came.
        #[derive(Debug)]
        struct Timed(Mock, Vec<Instant>);
        impl Transport for Timed {
            fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
                self.1.push(Instant::now());
                self.0.bulk_out(data)
            }
            fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
                self.0.bulk_in(buf)
            }
        }
        // 1200 dpi, 150 mm wide: 85,056 bytes of shading (13 November 2026).
        let values = vec![0u8; 85_056];
        let mut t = Timed(Mock::answering(&[&[ACK], &[ACK]]), Vec::new());
        table(
            &mut t,
            &[0x05, 0, 0x04, 0x01, 0x02, 0x40, 0x4C, 0x01],
            &values,
        )
        .unwrap();
        let lens: Vec<usize> = t.0.writes.iter().map(Vec::len).collect();
        assert_eq!(lens, [2, 8, TABLE_CHUNK, 85_056 - TABLE_CHUNK]);
        // No pause before the first piece; one before the second.
        assert!(t.1[3] - t.1[2] >= TABLE_PAUSE);
        assert!(t.0.answers.is_empty(), "the 06 after the table was read");
    }

    #[test]
    fn what_is_not_a_state_is_not_waited_for() {
        use crate::protocol::tests::Mock;
        // Two bytes of an image whose first has the firmware's bit: not taken for idle.
        let mut t = Mock::answering(&[&[0x10, 0x00]]);
        let e = wait_ready(&mut t, &mut |_| {}).unwrap_err().to_string();
        assert!(e.contains("not a state"), "{e}");
        let mut t = Mock::answering(&[&[0x58, 0x81], &[0x18, 0x81]]);
        assert!(wait_ready(&mut t, &mut |_| {}).is_ok());
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

    #[test]
    fn a_scan_left_half_way_is_stopped_with_1e_18() {
        use crate::guard::Guarded;
        use crate::protocol::tests::Mock;
        // As on the scanner: image packets, then the lone 06, then the state while the
        // carriage goes home, then at rest.
        let packet = [0x0Cu8; 512];
        let mut t = Guarded::new(Mock::answering(&[
            &packet,
            &packet[..308],
            &[],
            &[ACK],
            &[0x58, 0x81],
            &[0x18, 0x81],
        ]));
        // Not without the guard told.
        assert!(recover(&mut t, &mut |_| {}).is_err());
        assert!(t.inner().writes.is_empty());
        t.permit_recovery();
        let mut log = Vec::new();
        recover(&mut t, &mut |l| log.push(l)).unwrap();
        assert_eq!(
            t.inner().writes,
            vec![CANCEL.to_vec(), GET_STATE.to_vec(), GET_STATE.to_vec()]
        );
        assert!(log[0].contains("820 bytes"), "{log:?}");
        assert!(log[0].contains("in 4 reads"), "{log:?}");
    }
}
