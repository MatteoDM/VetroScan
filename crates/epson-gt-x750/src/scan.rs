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
//! Films ([`scan_film`]): the lid's lamp ([`light_lid_lamp`]), the film's calibration
//! ([`calibrate_film`]) and its image read, as Epson Scan's captures 74-81, through the guard's
//! film path ([`Guarded::permit_film_path`], [`Guarded::permit_film_plan`]).

use crate::film::{FilmPlan, FilmRequest, Pass};
use crate::guard::Guarded;
use crate::ice::{self, Holes};
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
/// The lid's lamp gets no more time than the scanner's flag before a film's calibration, unlike
/// the glass's ([`WARM_UP`]), as Epson Scan: measured from cold (15 November 2026, capture 105,
/// eleven scans of a negative over 14 minutes), a scan differs from the steady ones only by a
/// scale per channel (up to 4 %, in the calibration's steps, the same within 0.03 % after about
/// 13 minutes), which a negative developed on its own base takes out (at most a level of 255).
pub fn light_lid_lamp<T: Transport>(
    g: &mut Guarded<T>,
    request: &FilmRequest,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<Duration> {
    g.permit_film(request)?;
    let result = crate::film::request_block(request)
        .and_then(|e57| lid_lamp(g, &e57, log, events))
        .map_err(|e| after(e, g.last_command()));
    g.end_scan();
    result
}

fn lid_lamp<T: Transport>(
    g: &mut Guarded<T>,
    e57: &[u8; 42],
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<Duration> {
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
    send(g, [0x1E, 0x57], e57)?;
    wait_ready(g, log)?;
    lamp_at_opening(g)?;
    let asked = Instant::now();
    send(g, [0x1E, 0xA2], &[0x02])?;
    log(format!(
        "the lid's lamp (1E A2 02; 1E A1 answered {before:02X} before): 06 after {:.1} s",
        asked.elapsed().as_secs_f64()
    ));
    let lit = Instant::now();
    let mut warmed = false;
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
        let ready = s.firmware_running() && s.0[0] & 0x43 == 0;
        if ready && !warmed && before == 0x02 {
            // Lit already (capture 74: the lamp chosen and warm, 1E A1 not asked again).
            log("the lid's lamp lit already".into());
            return Ok(waited);
        }
        warmed = true;
        let lamp = lamp_in_use(g)?;
        if ready && lamp == 0x02 {
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

/// What a film's calibration found and set ([`calibrate_film`]): what its image read needs.
#[derive(Debug, Clone)]
pub struct FilmCalibration {
    /// The plan, with `1E 87`'s correction.
    pub plan: FilmPlan,
    /// The offsets found (`1E 31` bytes 0, 2, 4).
    pub offsets: [u8; 3],
    /// The exposures, µs a channel, after the loop (a negative's final ones).
    pub exposures: [u32; 3],
    /// The calibration's gains (`1E 31` bytes 8-10).
    pub gains: [u8; 3],
    /// The image's gains.
    pub image_gains: [u8; 3],
    /// The image's and the dark read's `1E 21`.
    pub image_e21: [u8; 26],
    /// The black pixels' last read (16 bits), the white and the dark reads.
    pub black: Vec<u8>,
    /// The white read (16 lines, 16 bits).
    pub white: Vec<u8>,
    /// The dark read (64 lines, 16 bits).
    pub dark: Vec<u8>,
    /// The power-on the scan was made in (`1E 87`).
    pub power_on: PowerOn,
    /// The lid unit's move back after its origin read (`1E 8A` less 1024): where it is after.
    pub back: u32,
    /// Where the lid unit is after its origin read (`1E 86`): 1416 less [`Self::back`], and
    /// the infrared pass's 1729 steps more.
    pub unit_at: u32,
    /// Digital ICE: the holes the pass's pre-read found (`crate::ice::holes`).
    pub holes: Option<Holes>,
}

/// Where the white read leaves the carriages (`1E 86`, captures 74, 77, 80): both 64 steps on
/// from the park and the lid unit's origin (`unit_at`), the carriage at 2058.
fn after_white(unit_at: u32) -> (u32, u32) {
    (crate::film::PARK + 64, unit_at + 64)
}

/// A film pass's `1E 43`: a film's ([`crate::film::e43`]), a slide's in the infrared pass
/// (captures 18, 108-113: `0080` throughout).
fn pass_e43(plan: &FilmPlan, image: bool) -> [u8; 12] {
    if plan.pass == Pass::Infrared {
        crate::film::E43_SLIDE
    } else {
        crate::film::e43(plan.request.film, image)
    }
}

/// What a film pass is, besides its request: a film, or one of Digital ICE's passes (the
/// infrared one with the colour pass's plan and holes).
#[derive(Debug, Clone, Copy)]
enum PassOf<'a> {
    /// A film scan.
    Film,
    /// Digital ICE's colour pass.
    IceColour,
    /// Digital ICE's infrared pass.
    Infrared(&'a FilmPlan, Holes),
}

impl PassOf<'_> {
    fn pass(&self) -> Pass {
        match self {
            Self::Film => Pass::Film,
            Self::IceColour => Pass::IceColour,
            Self::Infrared(..) => Pass::Infrared,
        }
    }
}

/// The opening of Digital ICE's infrared pass, right after the colour pass's image (captures
/// 18, 108-113): `1E 53` twice, the gamma tables (the identity), `1E A1`, `1E 53`, the infrared
/// `1E 57`, `1E A1`, the infrared light `1E A2 04` (answered `06` once lit), the state.
fn infrared_opening(
    t: &mut dyn Transport,
    e57: &[u8; 42],
    log: &mut dyn FnMut(String),
) -> Result<()> {
    ask(t, [0x1E, 0x53], 42)?;
    ask(t, [0x1E, 0x53], 42)?;
    let gamma = Curves::Linear.gamma();
    for channel in 0..3u8 {
        command(t, [0x1E, 0xB3])?;
        t.bulk_out(&[channel])?;
        t.bulk_out(&gamma)?;
    }
    wait_ready(t, log)?;
    lamp_at_opening(t)?;
    ask(t, [0x1E, 0x53], 42)?;
    send(t, [0x1E, 0x57], e57)?;
    wait_ready(t, log)?;
    lamp_at_opening(t)?;
    let asked = Instant::now();
    send(t, [0x1E, 0xA2], &[ice::LAMP_INFRARED])?;
    wait_ready(t, log)?;
    log(format!(
        "the infrared light (1E A2 04) ready after {:.1} s",
        asked.elapsed().as_secs_f64()
    ));
    Ok(())
}

/// Digital ICE's hole pre-read from home (captures 18, 108-113): `1E 25 00`, `1E 87 00`, the
/// pre-read's gains, the carriage 131 steps on the 256-value ramp, the read's set-up and its
/// 512 steps, the read (480 lines of 7040 pixels at 8 bits); the holes found in it. Answers the
/// correction and the holes.
fn hole_pre_read(
    t: &mut dyn Transport,
    pass: Pass,
    log: &mut dyn FnMut(String),
) -> Result<(XCorrection, Holes)> {
    send(t, [0x1E, 0x25], &[0x00])?;
    command(t, [0x1E, 0x87])?;
    t.bulk_out(&[0x00])?;
    let xc = XCorrection::from_1e87(&read_exact(t, 10, "1E 87")?)?;
    let e31 = ice::pre_e31(pass);
    send(t, [0x1E, 0x31], &e31)?;
    table(
        t,
        &setup::ramp_header(256),
        &ramp_bytes(&setup::fixed_ramp(256)),
    )?;
    send(
        t,
        [0x1E, 0x01],
        &setup::move_block(ice::PRE_MOVE, 0, 0xFF, 0x10),
    )?;
    command(t, [0x1E, 0x05])?;
    wait_ready(t, log)?;
    send(t, [0x1E, 0x31], &e31)?;
    send(t, [0x1E, 0x11], &[0x01])?;
    send(t, [0x1E, 0x21], &ice::pre_e21())?;
    send(t, [0x1E, 0x41], &ice::pre_e41(xc))?;
    send(t, [0x1E, 0x42], &ZEROS_42)?;
    send(t, [0x1E, 0x43], &setup::GAIN_CAL)?;
    table(
        t,
        &setup::ramp_header(1),
        &((ice::PRE_T >> 4) as u16).to_le_bytes(),
    )?;
    send(t, [0x1E, 0x01], &crate::guard::ice_pre_read_move())?;
    wait_ready(t, log)?;
    let (px, lines) = ice::PRE_READ;
    let read = read_all(t, lines, lines, px * 3)?;
    wait_ready(t, log)?;
    let holes =
        ice::holes(&read).ok_or_else(|| other("the hole pre-read came back short".into()))?;
    log(format!(
        "the holder's holes ({:?} pass): found {}, {:?} (4800ths: x, y, width, height, left then right)",
        pass, holes.found, holes.values
    ));
    Ok((xc, holes))
}

/// The `1E 31` block: offsets and gains.
fn e31(offsets: [u8; 3], gains: [u8; 3]) -> [u8; 12] {
    [
        offsets[0], 0, offsets[1], 0, offsets[2], 0, 0, 0, gains[0], gains[1], gains[2], 0,
    ]
}

const ZEROS_42: [u8; 18] = [0; 18];

/// Calibrates a film as Epson Scan does (captures 72-81; the rules in [`crate::film_cal`], the
/// blocks in [`FilmPlan`]): the lid's lamp; the carriage parked under the window and the lid's
/// unit found its origin; `1E 87`; the offsets on the black pixels; the exposures (and a
/// negative's gains) on the window; the black pixels, the white and the dark read. Stops before
/// the image (`1E 8B` comes next). `slide_factor`: a slide's image exposure in tenths (10 keeps
/// the white's; Epson Scan chose 15 once).
pub fn calibrate_film<T: Transport>(
    g: &mut Guarded<T>,
    request: &FilmRequest,
    slide_factor: u32,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<FilmCalibration> {
    g.permit_film(request)?;
    let result = film_calibration(g, request, slide_factor, PassOf::Film, log, events)
        .map_err(|e| after(e, g.last_command()));
    g.end_scan();
    result
}

fn film_calibration<T: Transport>(
    g: &mut Guarded<T>,
    request: &FilmRequest,
    slide_factor: u32,
    of: PassOf,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<FilmCalibration> {
    use crate::film::{self, Film};
    use crate::film_cal as fc;
    use crate::guard::{film_origin_block, film_origin_read_move, FILM_BACK_MOVES};
    let pass = of.pass();
    let infrared = pass == Pass::Infrared;
    // The infrared pass calibrates as a slide: all three channels' exposures, no gain search.
    let negative = request.film == Film::Negative && !infrared;
    let (lamp_on, lamp_dark) = if infrared {
        (ice::LAMP_INFRARED, ice::LAMP_INFRARED_DARK)
    } else {
        (0x02, 0x03)
    };
    let e57 = ice::request_block(request, pass)?;
    if infrared {
        infrared_opening(g, &e57, log)?;
    } else {
        lid_lamp(g, &e57, log, events)?;
    }
    g.permit_film_path()?;
    let positions = |a: Vec<u8>| {
        (
            u32::from(u16::from_le_bytes([a[0], a[1]])),
            u32::from(u16::from_le_bytes([a[2], a[3]])),
        )
    };
    // The park: the carriage under the window, the lid's unit home.
    let ramp512: Vec<u8> = ramp_bytes(&setup::film_ramp());
    let mut pre = None;
    let mut unit = 0;
    if pass != Pass::Film {
        // Digital ICE: from home only (every capture); the infrared pass first moves the lid
        // unit's light on, then both read the holder's holes and park from there.
        let at = positions(ask(g, [0x1E, 0x86], 4)?);
        let at2 = positions(ask(g, [0x1E, 0x86], 4)?);
        if at != (0, 0) || at2 != (0, 0) {
            return Err(other(format!(
                "the carriages at {at2:?}: a Digital ICE pass starts from home, as captured"
            )));
        }
        if infrared {
            table(g, &setup::ramp_header(512), &ramp512)?;
            send(g, [0x1E, 0x25], &[0x02])?;
            send(
                g,
                [0x1E, 0x01],
                &setup::move_block(ice::IR_UNIT_MOVE, 0, 0x1FF, 0x10),
            )?;
            command(g, [0x1E, 0x05])?;
            wait_ready(g, log)?;
            send(g, [0x1E, 0xA2], &[lamp_on])?;
            unit = ice::IR_UNIT_AT;
        }
        let (xc, holes) = hole_pre_read(g, pass, log)?;
        if !infrared {
            send(g, [0x1E, 0x88], &holes.block88())?;
        } else {
            // The scanner gives back the colour pass's holes.
            command(g, [0x1E, 0x87])?;
            g.bulk_out(&[0x01])?;
            read_exact(g, 18, "1E 87 01")?;
        }
        pre = Some((xc, holes));
        let at = positions(ask(g, [0x1E, 0x86], 4)?);
        if at != (ice::PRE_AT, unit) {
            return Err(other(format!(
                "after the hole pre-read the carriages are at {at:?}, not ({}, {unit})",
                ice::PRE_AT
            )));
        }
        send(g, [0x1E, 0x25], &[0x00])?;
        table(
            g,
            &setup::ramp_header(256),
            &ramp_bytes(&setup::fixed_ramp(256)),
        )?;
        send(
            g,
            [0x1E, 0x01],
            &setup::move_block(ice::PRE_PARK, 0, 0xFF, 0x10),
        )?;
    } else {
        match positions(ask(g, [0x1E, 0x86], 4)?) {
            (0, 0) => {
                send(g, [0x1E, 0x25], &[0x00])?;
                table(g, &setup::ramp_header(512), &ramp512)?;
                send(
                    g,
                    [0x1E, 0x01],
                    &setup::move_block(film::PARK - film::RAMP_512_STEPS, 0, 0x1FF, 0x10),
                )?;
            }
            // Where the scanner puts them when it lights the lid's lamp again (capture 72).
            (film::PARK, 1078) => {
                table(g, &setup::ramp_header(512), &ramp512)?;
                send(g, [0x1E, 0x25], &[0x02])?;
                send(
                    g,
                    [0x1E, 0x01],
                    &setup::move_block(1078 - film::RAMP_512_STEPS, 0, 0x1FF, 0x30),
                )?;
            }
            (c, u) => {
                return Err(other(format!(
                    "the carriage at {c} and the lid's unit at {u}: a film scan starts from home \
                 (0, 0) or from where the lid's lamp puts them (1994, 1078), as captured"
                )))
            }
        }
    }
    command(g, [0x1E, 0x05])?;
    wait_ready(g, log)?;
    if pass == Pass::Film {
        let at = positions(ask(g, [0x1E, 0x86], 4)?);
        if at != (film::PARK, 0) {
            return Err(other(format!(
                "after the park the carriages are at {at:?}, not (1994, 0)"
            )));
        }
    }
    // The lid unit's origin.
    if !events(Event::Calibrating) {
        return Err(Error::Cancelled);
    }
    let origin_e31 = if infrared {
        ice::IR_ORIGIN_E31
    } else {
        film::ORIGIN_E31
    };
    send(g, [0x1E, 0x31], &origin_e31)?;
    send(g, [0x1E, 0xA2], &[lamp_on])?;
    send(g, [0x1E, 0x25], &[0x02])?;
    table(g, &setup::ramp_header(1), &156u16.to_le_bytes())?;
    send(g, [0x1E, 0x01], &setup::move_block(8, 0, 0, 0x18))?;
    command(g, [0x1E, 0x05])?;
    wait_ready(g, log)?;
    send(g, [0x1E, 0x31], &origin_e31)?;
    send(g, [0x1E, 0x11], &[film::ORIGIN_E11])?;
    let origin_t = if infrared { ice::IR_ORIGIN_T } else { 6144 };
    let mut e21o = [0u8; 26];
    e21o[..3].copy_from_slice(&origin_t.to_le_bytes()[..3]);
    e21o[12] = 0x2A;
    send(g, [0x1E, 0x21], &e21o)?;
    send(g, [0x1E, 0x41], &film::origin_e41())?;
    send(g, [0x1E, 0x42], &ZEROS_42)?;
    send(g, [0x1E, 0x43], &setup::GAIN_CAL)?;
    table(
        g,
        &setup::ramp_header(512),
        &ramp_bytes(&setup::ramp(origin_t / 12, setup::Mode::Medium)),
    )?;
    send(g, [0x1E, 0x01], &film_origin_read_move())?;
    wait_ready(g, log)?;
    let (ol, op) = fc::ORIGIN_READ;
    let origin = read_all(g, ol as u32, ol as u32, (op * 3) as u32)?;
    wait_ready(g, log)?;
    if let Some(levels) = fc::origin_levels(&origin) {
        let levels: Vec<String> = levels.iter().map(u32::to_string).collect();
        log(format!(
            "the lid unit's origin read, each line's level: {}",
            levels.join(" ")
        ));
    }
    let n = if infrared {
        fc::infrared_origin(&origin)
    } else {
        fc::origin(&origin)
    }
    .map(|v| v - film::RAMP_512_STEPS)
    .ok_or_else(|| other("the lid unit's origin read found no edge of its light".into()))?;
    if !FILM_BACK_MOVES.contains(&n) {
        return Err(other(format!(
            "the lid unit's origin read asks a move back of {n} steps, none of those let \
             out ({FILM_BACK_MOVES:?})"
        )));
    }
    log(if pass == Pass::Film {
        format!("the lid unit's origin: back {n} steps (1E 8A {})", n + 1024)
    } else {
        format!("the lid unit's origin: back {n} steps (no 1E 8A with Digital ICE)")
    });
    send(g, [0x1E, 0x25], &[0x02])?;
    table(g, &setup::ramp_header(512), &ramp512)?;
    send(g, [0x1E, 0x01], &setup::move_block(n, 0, 0x1FF, 0x30))?;
    command(g, [0x1E, 0x05])?;
    wait_ready(g, log)?;
    match pass {
        // Digital ICE sends no 1E 8A (captures 18, 108-113).
        Pass::Film => send(g, [0x1E, 0x8A], &film_origin_block(n))?,
        Pass::IceColour => {}
        Pass::Infrared => {
            command(g, [0x1E, 0x87])?;
            g.bulk_out(&[0x01])?;
            read_exact(g, 18, "1E 87 01")?;
            command(g, [0x1E, 0x87])?;
            g.bulk_out(&[0x00])?;
            read_exact(g, 10, "1E 87")?;
        }
    }
    send(g, [0x1E, 0x25], &[0x03])?;
    // The correction, the request again, the positions.
    command(g, [0x1E, 0x87])?;
    g.bulk_out(&[0x00])?;
    let a87 = read_exact(g, 10, "1E 87")?;
    let xc = XCorrection::from_1e87(&a87)?;
    let power_on = PowerOn(a87.as_slice().try_into().expect("10 bytes"));
    ask(g, [0x1E, 0x53], 42)?;
    send(g, [0x1E, 0x57], &e57)?;
    let unit_at = unit + 1416 - n;
    let at = positions(ask(g, [0x1E, 0x86], 4)?);
    if at != (film::PARK, unit_at) {
        return Err(other(format!(
            "after the origin the carriages are at {at:?}, not (1994, {unit_at})"
        )));
    }
    if let Some((pre_xc, _)) = pre {
        if pre_xc != xc {
            return Err(other(format!(
                "1E 87 gave {} at the hole pre-read and {} now",
                pre_xc.0, xc.0
            )));
        }
    }
    let plan = match of {
        PassOf::Film => FilmPlan::new(*request, xc)?,
        PassOf::IceColour => FilmPlan::from_geometry(
            *request,
            crate::film::geometry(request, xc)?,
            Pass::IceColour,
        ),
        PassOf::Infrared(colour, colour_holes) => {
            let (_, ir_holes) = pre.expect("the infrared pass's pre-read");
            ice::infrared_plan(colour, xc, &colour_holes, &ir_holes)?
        }
    };
    g.permit_film_plan(&plan)?;
    let e11 = plan.geometry.e11;
    let (black_words, black_margin) = plan.black;
    // The offsets, on the black pixels.
    let mut gains = if infrared {
        ice::IR_GAINS
    } else if negative {
        fc::NEGATIVE_GAINS
    } else {
        fc::SLIDE_GAINS
    };
    let mut offsets = [fc::OFFSET_START; 3];
    let x0 = [fc::EXPOSURE_START; 3];
    send(g, [0x1E, 0x31], &e31(offsets, gains))?;
    send(
        g,
        [0x1E, 0x21],
        &fc::exposure_block21(black_margin, black_words, x0, false, 0),
    )?;
    let back = ask(g, [0x1E, 0x92], 12)?;
    send(g, [0x1E, 0x31], &back)?;
    send(g, [0x1E, 0x11], &[e11])?;
    send(
        g,
        [0x1E, 0x21],
        &fc::exposure_block21(black_margin, black_words, x0, false, 2),
    )?;
    send(g, [0x1E, 0x41], &plan.e41_black)?;
    send(g, [0x1E, 0x42], &ZEROS_42)?;
    send(g, [0x1E, 0x43], &pass_e43(&plan, false))?;
    ask(g, [0x1E, 0x92], 12)?;
    let back = ask(g, [0x1E, 0x92], 12)?;
    send(g, [0x1E, 0x31], &back)?;
    wait_ready(g, log)?;
    let black_px = u32::from(u16::from_le_bytes([plan.e41_black[2], plan.e41_black[3]]))
        - u32::from(u16::from_le_bytes([plan.e41_black[0], plan.e41_black[1]]));
    let black_read = |g: &mut Guarded<T>,
                      offsets: [u8; 3],
                      gains: [u8; 3],
                      log: &mut dyn FnMut(String)|
     -> Result<Vec<u8>> {
        send(g, [0x1E, 0x31], &e31(offsets, gains))?;
        send(g, [0x1E, 0x41], &plan.e41_black)?;
        wait_ready(g, log)?;
        let d = read_all(g, 4, 4, black_px * 6)?;
        wait_ready(g, log)?;
        Ok(d)
    };
    let mut step = fc::offsets_first_step();
    loop {
        let d = black_read(g, offsets, gains, log)?;
        let means = channel_means16(&d);
        let (next, s) = fc::offsets_round(offsets, step, means);
        offsets = next;
        match s {
            Some(s) => step = s,
            None => break,
        }
    }
    log(format!("offsets {offsets:02X?}"));
    // The exposures, on the window.
    let held = if negative {
        [false, true, true]
    } else {
        [false; 3]
    };
    let mut x = x0;
    let base = plan.geometry.base;
    let cap = if infrared {
        ice::IR_MAX_LINE_TIME - fc::margin(base)
    } else {
        fc::exposure_cap(base)
    };
    send(g, [0x1E, 0xA2], &[lamp_on])?;
    let back = ask(g, [0x1E, 0x92], 12)?;
    send(g, [0x1E, 0x31], &back)?;
    send(g, [0x1E, 0x21], &fc::window_block21(base, x, false, 2))?;
    let back = ask(g, [0x1E, 0x92], 12)?;
    send(g, [0x1E, 0x31], &back)?;
    send(g, [0x1E, 0x11], &[e11])?;
    send(g, [0x1E, 0x21], &fc::window_block21(base, x, false, 2))?;
    send(g, [0x1E, 0x41], &plan.e41_window)?;
    send(g, [0x1E, 0x42], &ZEROS_42)?;
    send(g, [0x1E, 0x43], &pass_e43(&plan, false))?;
    wait_ready(g, log)?;
    let window_px = u32::from(u16::from_le_bytes([plan.e41_window[2], plan.e41_window[3]]))
        - u32::from(u16::from_le_bytes([plan.e41_window[0], plan.e41_window[1]]));
    let window_level = |d: &[u8]| {
        let lines: Vec<&[u8]> = d.chunks_exact((window_px * 3) as usize).collect();
        fc::level(&lines)
    };
    loop {
        send(g, [0x1E, 0x21], &fc::window_block21(base, x, false, 2))?;
        send(g, [0x1E, 0x41], &plan.e41_window)?;
        wait_ready(g, log)?;
        let d = read_all(g, 4, 4, window_px * 3)?;
        wait_ready(g, log)?;
        match fc::exposure_round(x, window_level(&d), held, cap) {
            Some(next) => x = next,
            None => break,
        }
    }
    let mut last_window = fc::window_block21(base, x, false, 2);
    let mut image_t = 0;
    if negative {
        let (final_x, t) = fc::negative_exposures(x[0]);
        x = final_x;
        image_t = t;
        send(g, [0x1E, 0x21], &fc::window_block21(base, x, true, 2))?;
        send(g, [0x1E, 0x21], &fc::window_block21(base, x, true, 2))?;
        let back = ask(g, [0x1E, 0x92], 12)?;
        send(g, [0x1E, 0x31], &back)?;
        send(g, [0x1E, 0x11], &[e11])?;
        last_window = fc::window_block21(base, x, true, 6);
        send(g, [0x1E, 0x21], &last_window)?;
        send(g, [0x1E, 0x41], &plan.e41_window)?;
        send(g, [0x1E, 0x42], &ZEROS_42)?;
        send(g, [0x1E, 0x43], &film::e43(request.film, false))?;
        wait_ready(g, log)?;
        // The gains.
        loop {
            send(g, [0x1E, 0x31], &e31(offsets, gains))?;
            send(g, [0x1E, 0x41], &plan.e41_window)?;
            wait_ready(g, log)?;
            let d = read_all(g, 4, 4, window_px * 3)?;
            wait_ready(g, log)?;
            match fc::gains_round(gains, window_level(&d)) {
                Some(next) => gains = next,
                None => break,
            }
        }
    }
    // The infrared pass: every channel the whole line, from the green's exposure (captures 18,
    // 108-113).
    let ir_end = ice::image_end(&plan);
    if infrared {
        last_window = ice::final_block(x[1], ir_end, fc::window_words(base), 2);
    }
    if !negative {
        // A slide's window block once more (captures 80, 81), the infrared's last.
        send(g, [0x1E, 0x21], &last_window)?;
    }
    log(format!("exposures {x:?} µs, gains {gains:02X?}"));
    // The black pixels with all of it: a negative's with the window's margin and its final
    // words, a slide's with the black pixels' own margin (captures 74-78, 80-81).
    ask(g, [0x1E, 0x8C], 1)?;
    send(g, [0x1E, 0x25], &[0x03])?;
    let mut bb = if infrared {
        ice::final_block(x[1], ir_end, black_words, 2)
    } else if negative {
        fc::exposure_block21(fc::margin(base), black_words, x, true, 6)
    } else {
        fc::exposure_block21(black_margin, black_words, x, false, 2)
    };
    send(g, [0x1E, 0x21], &bb)?;
    send(g, [0x1E, 0x31], &e31(offsets, gains))?;
    send(g, [0x1E, 0x11], &[e11])?;
    bb[25] = 6;
    send(g, [0x1E, 0x21], &bb)?;
    send(g, [0x1E, 0x41], &plan.e41_black)?;
    send(g, [0x1E, 0x42], &ZEROS_42)?;
    send(g, [0x1E, 0x43], &pass_e43(&plan, false))?;
    wait_ready(g, log)?;
    let black = read_all(g, 4, 4, black_px * 6)?;
    wait_ready(g, log)?;
    // The white.
    let words = plan.geometry.words;
    let white_e21 = if infrared {
        ice::final_block(x[1], ir_end, words, 6)
    } else {
        fc::white_block21(x, words, plan.image_margin, &last_window, 6)
    };
    send(g, [0x1E, 0x21], &white_e21)?;
    send(g, [0x1E, 0x31], &e31(offsets, gains))?;
    send(g, [0x1E, 0x11], &[e11])?;
    send(g, [0x1E, 0x21], &white_e21)?;
    send(g, [0x1E, 0x41], &plan.e41_white)?;
    send(g, [0x1E, 0x42], &crate::shading::black_block(&black))?;
    send(g, [0x1E, 0x43], &pass_e43(&plan, false))?;
    let t_white = u32::from_le_bytes([white_e21[0], white_e21[1], white_e21[2], 0]);
    table(
        g,
        &setup::ramp_header(1),
        &(((t_white / 2) >> 4) as u16).to_le_bytes(),
    )?;
    send(g, [0x1E, 0x01], &setup::move_block(64, 32, 0, 0x18))?;
    wait_ready(g, log)?;
    let cols = plan.geometry.column_count();
    let white = read_all(g, 16, 16, cols * 6)?;
    wait_ready(g, log)?;
    // The dark, with the image's settings.
    let (image_e21, image_gains) = if infrared {
        (white_e21, gains)
    } else if negative {
        (
            fc::negative_image_block21(image_t, words, 6),
            fc::negative_image_gains(gains),
        )
    } else {
        let xi = fc::slide_image_exposures(x, slide_factor);
        (
            fc::white_block21(xi, words, plan.image_margin, &last_window, 6),
            gains,
        )
    };
    send(g, [0x1E, 0xA2], &[lamp_dark])?;
    send(g, [0x1E, 0x21], &image_e21)?;
    send(g, [0x1E, 0x31], &e31(offsets, image_gains))?;
    send(g, [0x1E, 0x11], &[e11])?;
    let mut dark_e21 = image_e21;
    dark_e21[25] = if negative { 2 } else { 6 };
    send(g, [0x1E, 0x21], &dark_e21)?;
    send(g, [0x1E, 0x41], &plan.e41_dark)?;
    send(g, [0x1E, 0x42], &ZEROS_42)?;
    send(g, [0x1E, 0x43], &pass_e43(&plan, false))?;
    wait_ready(g, log)?;
    let dark = read_all(g, 64, 64, cols * 6)?;
    wait_ready(g, log)?;
    send(g, [0x1E, 0xA2], &[lamp_on])?;
    Ok(FilmCalibration {
        plan,
        offsets,
        exposures: x,
        gains,
        image_gains,
        image_e21: dark_e21,
        black,
        white,
        dark,
        power_on,
        back: n,
        unit_at,
        holes: pre.map(|(_, h)| h),
    })
}

/// A film scanned ([`scan_film`]).
#[derive(Debug, Clone)]
pub struct FilmScanned {
    /// The image put together ([`assemble_film`]): the request's pixels, at its `dpi` both ways.
    pub image: Image,
    /// What the calibration found and set.
    pub calibration: FilmCalibration,
    /// The shading computed from its white and dark reads.
    pub shading: Shading,
    /// The image data as the scanner sent it, trailers removed: [`crate::film::Geometry::lines`]
    /// lines of [`crate::film::Geometry::line_bytes`].
    pub raw: Vec<u8>,
}

/// Scans a film as Epson Scan does (captures 74-81): its calibration ([`calibrate_film`]), then
/// the image: `1E 8B` and the 07 table, the carriages' positions, the image's `1E 21`, the tone
/// tables (linear), the start move with the film path's ramp, the image's set-up with the
/// shading, the read's one-value ramp and move, the read, and the carriages home; the image then
/// put together on the computer ([`assemble_film`]).
///
/// Stopping (`events` answering `false`) is honoured up to the calibration; once the image is
/// being read it goes on to its end, as no film scan was ever captured stopped with `1E 18`
/// (the read then ends with [`Error::Cancelled`]). A read that fails is stopped with `1E 18`,
/// as on the glass, so that the scanner is not left sending (never captured on films either).
pub fn scan_film<T: Transport>(
    g: &mut Guarded<T>,
    request: &FilmRequest,
    slide_factor: u32,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<FilmScanned> {
    g.permit_film(request)?;
    let result = film_calibration(g, request, slide_factor, PassOf::Film, log, events)
        .and_then(|c| {
            let (raw, shading) = film_image(g, &c, log, events)?;
            Ok(FilmScanned {
                image: assemble_film(&raw, &c.plan),
                calibration: c,
                shading,
                raw,
            })
        })
        .map_err(|e| after(e, g.last_command()));
    g.end_scan();
    result
}

/// A Digital ICE scan ([`scan_ice`]).
#[derive(Debug, Clone)]
pub struct IceScanned {
    /// The colour pass, as a film scan's.
    pub colour: FilmScanned,
    /// The infrared laid on [`FilmScanned::image`]: one 16-bit sample a pixel, row by row
    /// ([`ice::register`]).
    pub infrared: Vec<u16>,
    /// Where the infrared read was found on the colours.
    pub registration: ice::Registration,
    /// The infrared pass's calibration.
    pub infrared_calibration: FilmCalibration,
    /// The infrared read as the scanner sent it, trailers removed: three 16-bit samples a pixel
    /// (the CCD's three rows, each seeing the infrared).
    pub infrared_raw: Vec<u8>,
}

/// Scans a film with Digital ICE as Epson Scan does (captures 18, 108-113; `crate::ice`): the
/// colour pass (a film scan with the hole pre-read and `1E 88`), then the infrared pass (its own
/// opening, light, hole pre-read, calibration and read), the infrared laid on the colours
/// ([`ice::register`]). Stopping is honoured as in [`scan_film`]: up to each pass's image; a
/// pass's image goes on to its end, and a stop asked during the colour image skips the infrared
/// pass.
pub fn scan_ice<T: Transport>(
    g: &mut Guarded<T>,
    request: &FilmRequest,
    slide_factor: u32,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<IceScanned> {
    g.permit_ice(request)?;
    let result =
        ice_scan(g, request, slide_factor, log, events).map_err(|e| after(e, g.last_command()));
    g.end_scan();
    result
}

fn ice_scan<T: Transport>(
    g: &mut Guarded<T>,
    request: &FilmRequest,
    slide_factor: u32,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<IceScanned> {
    let c = film_calibration(g, request, slide_factor, PassOf::IceColour, log, events)?;
    let (raw, shading) = film_image(g, &c, log, events)?;
    let image = assemble_film(&raw, &c.plan);
    let holes = c
        .holes
        .ok_or_else(|| other("the colour pass read no holes".into()))?;
    g.permit_infrared_pass()?;
    let ci = film_calibration(
        g,
        request,
        slide_factor,
        PassOf::Infrared(&c.plan, holes),
        log,
        events,
    )?;
    let (ir_raw, _) = film_image(g, &ci, log, events)?;
    let (infrared, registration) = ice::register(&image, &c.plan, &ir_raw, &ci.plan);
    log(format!(
        "the infrared laid on the colours: line {:.2}, column {:.2}, scale {:.5} ({})",
        registration.line,
        registration.column,
        registration.scale,
        if registration.measured {
            format!("shift measured on {} tiles", registration.tiles)
        } else {
            "the prediction kept: no clear peak".into()
        }
    ));
    Ok(IceScanned {
        colour: FilmScanned {
            image,
            calibration: c,
            shading,
            raw,
        },
        infrared,
        registration,
        infrared_calibration: ci,
        infrared_raw: ir_raw,
    })
}

/// `1E 8B` and the 07 table, as Epson's drivers handle it before a read at base 2400 or 4800
/// on the glass and before every film's image: `06`, the scanner holds it (it keeps it until
/// switched off), read back (captures 33, 37, 74-81); `15`, it does not: computed from a raw
/// line of the sensor and loaded (captures 05, 42, 61 on the glass; `table07`; never captured
/// before a film).
fn table07_step(t: &mut dyn Transport, log: &mut dyn FnMut(String)) -> Result<()> {
    let a = ask(t, [0x1E, 0x8B], 1)?;
    wait_ready(t, log)?;
    match a[0] {
        ACK => {
            command(t, [0x1E, 0x83])?;
            t.bulk_out(&READ_83)?;
            read_exact(t, 600, "1E 83")?;
        }
        0x15 => {
            command(t, [0x1E, 0x83])?;
            t.bulk_out(&table07::READ_RAW)?;
            let raw = read_exact(t, table07::RAW_LEN, "1E 83 (the raw line)")?;
            command(t, [0x1E, 0x87])?;
            t.bulk_out(&[0x00])?;
            let xc = XCorrection::from_1e87(&read_exact(t, 10, "1E 87")?)?;
            table(t, &table07::HEADER, &table07::compute(&raw, xc))?;
            log("1E 8B answered 15: the 07 table computed and loaded".into());
        }
        other_ => {
            return Err(other(format!(
                "1E 8B answered {other_:02X}, neither 06 nor 15"
            )))
        }
    }
    Ok(())
}

/// The shading of a film's white and dark reads: by column parity at base 4800 (Epson Scan's
/// two minimum darks, captures 76-81), the glass's at base 2400.
pub fn film_shading(c: &FilmCalibration) -> Shading {
    let cols = c.plan.geometry.column_count() as usize;
    if c.plan.geometry.base == 4800 {
        Shading::compute_by_parity(&c.white, &c.dark, cols, cols, cols * 3)
    } else {
        Shading::compute(&c.white, &c.dark, cols, cols, cols * 3)
    }
}

fn film_image<T: Transport>(
    g: &mut Guarded<T>,
    c: &FilmCalibration,
    log: &mut dyn FnMut(String),
    events: &mut dyn FnMut(Event) -> bool,
) -> Result<(Vec<u8>, Shading)> {
    let plan = &c.plan;
    let request = plan.request;
    let geo = plan.geometry;
    table07_step(g, log)?;
    let at = ask(g, [0x1E, 0x86], 4)?;
    let at = (
        u32::from(u16::from_le_bytes([at[0], at[1]])),
        u32::from(u16::from_le_bytes([at[2], at[3]])),
    );
    if at != after_white(c.unit_at) {
        return Err(other(format!(
            "after the film's calibration the carriages are at {at:?}, not {:?}",
            after_white(c.unit_at)
        )));
    }
    send(g, [0x1E, 0x21], &c.image_e21)?;
    let tone = Curves::Linear.tone();
    for header in &tables::TONE_HEADERS {
        table(g, header, &tone)?;
    }
    // The start move: the carriage to the image's first line.
    table(
        g,
        &setup::ramp_header(512),
        &ramp_bytes(&setup::film_ramp()),
    )?;
    send(
        g,
        [0x1E, 0x01],
        &setup::move_block(geo.start_steps, 0, 0x1FF, 0x10),
    )?;
    command(g, [0x1E, 0x05])?;
    wait_ready(g, log)?;
    // The image.
    let shading = film_shading(c);
    log(format!(
        "shading: K {:?}, minimum dark {:?} / {:?}, scales 2^{} and 2^{}",
        shading.k, shading.dark_min, shading.dark_min_odd, shading.white_shift, shading.dark_shift
    ));
    let mut e41 = plan.e41_image;
    e41[15] = (shading.white_shift << 4) | shading.dark_shift;
    send(g, [0x1E, 0x31], &e31(c.offsets, c.image_gains))?;
    send(g, [0x1E, 0x11], &[geo.e11])?;
    send(g, [0x1E, 0x21], &c.image_e21)?;
    send(g, [0x1E, 0x41], &e41)?;
    send(g, [0x1E, 0x42], &shading.block42())?;
    send(g, [0x1E, 0x43], &pass_e43(plan, true))?;
    table(
        g,
        &setup::shading_header(geo.column_count()),
        &shading.table_bytes(),
    )?;
    let t = u32::from_le_bytes([c.image_e21[0], c.image_e21[1], c.image_e21[2], 0]);
    table(g, &setup::ramp_header(1), &geo.read_ramp(t).to_le_bytes())?;
    send(
        g,
        [0x1E, 0x01],
        &setup::move_block(geo.read_steps, geo.lead as u16, 0, 0x08),
    )?;
    wait_ready(g, log)?;
    let line = geo.line_bytes(request.bits);
    log(format!(
        "reading {} lines of {} pixels (T {t} µs, start move {} steps)",
        geo.lines, geo.pixels, geo.start_steps
    ));
    let mut stopped = false;
    let raw = read_lines(
        g,
        geo.lines,
        u32::from(geo.lines_per_block),
        line,
        &mut |done, total| {
            if !stopped && !events(Event::Reading { done, total }) {
                stopped = true;
                log("stopping asked: a film's read goes on to its end (1E 18 never captured on films)".into());
            }
            true
        },
    )
    .map_err(|e| {
        let e = after(e, g.last_command());
        log(format!("the film's read failed ({e}): stopping it with 1E 18"));
        std::thread::sleep(Duration::from_millis(500));
        if let Err(also) = stop(g, log).and_then(|()| wait_ready(g, log).map(drop)) {
            log(format!("stopping it failed too: {also}"));
        }
        e
    })?
    .expect("never stopped");
    // The carriages go home by themselves (captures 74-81: 1B 03 moving, then at rest).
    wait_ready(g, log)?;
    if stopped {
        return Err(Error::Cancelled);
    }
    Ok((raw, shading))
}

/// Each channel's mean of a read of 16-bit samples, three a pixel.
fn channel_means16(d: &[u8]) -> [f64; 3] {
    let mut sum = [0f64; 3];
    let mut n = 0usize;
    for p in d.chunks_exact(6) {
        for (c, s) in sum.iter_mut().enumerate() {
            *s += f64::from(u16::from_le_bytes([p[2 * c], p[2 * c + 1]]));
        }
        n += 1;
    }
    sum.map(|s| s / n.max(1) as f64)
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

/// The lines a film's sensor columns of odd parity (`c0 + i` odd) come later than the even ones
/// when it is read at 4800 across: `ydpi / 600` (4 at 2400 along, 9 at 5400; the sensor's two
/// staggered rows, 1/600 in apart). Measured against Epson Scan's TIFFs (captures 17, 76, 77,
/// 80, 81; `captures/epson4490/tools/film_image.py`): on films the odd columns are the late
/// ones, with the start column odd (17, 76) or even (77, 80, 81), where on the glass the even
/// are ([`Plan::late`]). 0 at base 2400.
pub fn film_stagger(plan: &FilmPlan) -> u32 {
    if plan.geometry.read_dpi == 4800 {
        plan.request.ydpi / 600
    } else {
        0
    }
}

/// A film's image put together from its read ([`FilmScanned::raw`]) as Epson Scan's TIFFs show
/// it (captures 17 and 74-81 against their raw data, `captures/epson4490/tools/film_image.py`;
/// rank correlations 0.997-1.000 on the whole image): the colours realigned as on the glass
/// (green `ydpi / 50` lines below blue, red twice as many; the TIFF's first line is the blue's
/// first), the odd sensor columns [`film_stagger`] lines later; a 2400 dpi film, read at 4800
/// across, halved with weights 1/4, 1/2, 1/4 around the read's pixel `2j + 1` (the pair's mean
/// does worse on a negative, 0.97); cropped to the request from the read's first pixel; a 4800
/// dpi film, read at 5400 along, reduced to 4800 lines an inch as the glass's 3200 to 2400
/// ([`reduce_along`]: Epson Scan's own rule not found, a linear interpolation does as well).
pub fn assemble_film(raw: &[u8], plan: &FilmPlan) -> Image {
    let r = plan.request;
    let geo = &plan.geometry;
    let s = usize::from(r.bits / 8);
    let line = geo.line_bytes(r.bits) as usize;
    let px = geo.pixels as usize;
    let k = (r.ydpi / 50) as usize;
    let offsets = [2 * k, k, 0];
    let stagger = film_stagger(plan) as usize;
    let c0 = geo.columns.0 as usize;
    let sample = |y: usize, i: usize, c: usize| -> u32 {
        let i = i.min(px - 1);
        let late = if (c0 + i) % 2 == 1 { stagger } else { 0 };
        let at = (y + offsets[c] + late) * line + (i * 3 + c) * s;
        if s == 2 {
            u32::from(u16::from_le_bytes([raw[at], raw[at + 1]]))
        } else {
            u32::from(raw[at])
        }
    };
    let halved = geo.read_dpi != r.dpi;
    let (w, h) = (r.width as usize, r.height as usize);
    let mut data = Vec::with_capacity(w * h * 3 * s);
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let v = if halved {
                    let i = 2 * x + 1;
                    (sample(y, i - 1, c) + 2 * sample(y, i, c) + sample(y, i + 1, c) + 2) / 4
                } else {
                    sample(y, x, c)
                };
                if s == 2 {
                    data.extend_from_slice(&(v as u16).to_le_bytes());
                } else {
                    data.push(v as u8);
                }
            }
        }
    }
    let image = Image {
        width: r.width,
        height: r.height,
        bits: r.bits,
        data,
    };
    let h_out = u64::from(r.height) * u64::from(r.dpi) / u64::from(r.ydpi);
    reduce_lines(image, r.dpi, r.ydpi, h_out as u32)
}

/// An image read along at `r.ydpi` brought to `r.dpi` lines an inch ([`Request::image_height`]
/// lines): each line the average of the lines read that it covers, weighted by how much of each
/// (at 3200 to 2400: 3/4 + 1/4, 1/2 + 1/2, 1/4 + 3/4 of two lines in turn). Epson Scan reads
/// 2400 dpi so and reduces on the computer (captures 13, 57, 58); how it reduces is not known,
/// this is an average with no sharpening. The same image when both are equal.
pub fn reduce_along(image: Image, r: &Request) -> Image {
    reduce_lines(image, r.dpi, r.ydpi, r.image_height())
}

/// [`reduce_along`] from `ydpi` lines an inch to `dpi`, `h_out` lines.
fn reduce_lines(image: Image, dpi: u32, ydpi: u32, h_out: u32) -> Image {
    if ydpi == dpi {
        return image;
    }
    let s = usize::from(image.bits / 8);
    let row = image.width as usize * 3 * s;
    let samples = image.width as usize * 3;
    let h_in = image.height as usize;
    let h_out = h_out as usize;
    // Input line i spans [i * dpi, (i + 1) * dpi), output line j [j * ydpi, (j + 1) * ydpi).
    let (dpi, ydpi) = (u64::from(dpi), u64::from(ydpi));
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
        table07_step(g, log)?;
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
        /// A film capture's reads, made up from what its rules take from them (one line of
        /// `captures/epson4490/tools/film_testdata.py`'s each), and the read under way.
        film: Option<VecDeque<String>>,
        run: VecDeque<u8>,
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
                film: None,
                run: VecDeque::new(),
            }
        }

        /// A film capture with its reads' summary.
        pub(crate) fn film(name: &'static str, text: &str, reads: &str) -> Self {
            Self {
                film: Some(reads.lines().map(str::to_owned).collect()),
                ..Self::new(name, text)
            }
        }

        /// The bytes of the read starting at `self.at` (its `D` lines, up to the next write),
        /// made up from the next summary line: the same decisions as the real one.
        fn made_up_read(&mut self) -> VecDeque<u8> {
            let total: usize = self.lines[self.at..]
                .iter()
                .take_while(|l| !matches!(l, Line::W(..) | Line::P(..)))
                .filter_map(|l| match l {
                    Line::D(n) => Some(*n),
                    _ => None,
                })
                .sum();
            let spec = self
                .film
                .as_mut()
                .and_then(|f| f.pop_front())
                .unwrap_or_default();
            let f: Vec<&str> = spec.split(' ').collect();
            let num = |i: usize| f[i].parse::<u64>().unwrap();
            let mut out = vec![0u8; total];
            match f[0] {
                "origin" => {
                    for (l, line) in out.chunks_exact_mut(384 * 3).enumerate().take(150) {
                        for px in line.chunks_exact_mut(3) {
                            px[1] = num(1 + l) as u8;
                        }
                    }
                }
                "black" => {
                    let n = num(1) as usize;
                    for c in 0..3 {
                        let sum = num(2 + c);
                        let (q, r) = (sum / n as u64, (sum % n as u64) as usize);
                        for i in 0..n {
                            let v = (q + u64::from(i < r)) as u16;
                            out[6 * i + 2 * c..6 * i + 2 * c + 2].copy_from_slice(&v.to_le_bytes());
                        }
                    }
                }
                "window" => {
                    let line = total / 4;
                    for l in 0..4 {
                        for c in 0..3 {
                            out[l * line + c] = num(1 + 3 * l + c) as u8;
                        }
                    }
                }
                // Digital ICE's hole pre-read: green 255 on the runs, 2 elsewhere.
                "holes" => {
                    out.fill(2);
                    let px = ice::PRE_READ.0 as usize;
                    for (part, x0) in [(f[2], 0), (f[3], ice::RIGHT_HOLE as usize)] {
                        for run in part.split(',').filter(|r| *r != "-") {
                            let v: Vec<usize> =
                                run.split('.').map(|n| n.parse().unwrap()).collect();
                            for x in v[1]..=v[2] {
                                out[(v[0] * px + x0 + x) * 3 + 1] = 255;
                            }
                        }
                    }
                }
                _ => {}
            }
            out.into()
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
                // A film's gamma tables: Epson Scan's inverted curve, ours linear.
                Some(Line::W(want, None))
                    if self.film.is_some()
                        && self.last_cmd == [0x1E, 0xB3]
                        && want.len() == 256 =>
                {
                    data.len() == 256
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
                    // A film's tone tables: Epson Scan's own curve, ours linear.
                    ["other", "8192"] if self.film.is_some() && self.last_cmd == [0x1E, 0x84] => {
                        data == Curves::Linear.tone()
                    }
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
            if self.film.is_some() && data == [0x1E, 0x47] {
                self.run = self.made_up_read();
            }
            Ok(())
        }

        fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
            if self.answer.is_empty() {
                let line = self.lines.get(self.at).cloned();
                self.at += 1;
                match line {
                    Some(Line::R(b) | Line::A(b)) => self.answer.extend(b),
                    Some(Line::D(n)) if self.film.is_some() => {
                        let k = n.min(self.run.len());
                        self.answer.extend(self.run.drain(..k));
                        self.answer.extend(std::iter::repeat_n(0, n - k));
                    }
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

    /// A film scanned on a scanner answering as in a capture, to its end: the scan, and the
    /// capture's lines left (none, when it wrote all of it).
    fn film_scan(
        name: &'static str,
        text: &str,
        reads: &str,
        request: &FilmRequest,
        factor: u32,
    ) -> (FilmScanned, usize) {
        let mut g = Guarded::new(Follow::film(name, text, reads));
        let s = scan_film(&mut g, request, factor, &mut |_| {}, &mut |_| true)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let f = g.inner();
        let left = f.lines.len() - f.at;
        // Over: the guard lets out no film's command any more.
        assert!(g.bulk_out(&[0x1E, 0x25]).is_err());
        (s, left)
    }

    /// A film's scan writes what Epson Scan wrote in capture 74, write by write, from its lamp
    /// to the image read and the carriages home (the gamma and tone tables linear, the computed
    /// `1E 42` and shading any), its calibration reads made up from what the rules take from
    /// the real ones. The calibration alone stops before `1E 8B`, the image's.
    #[test]
    fn a_films_scan_writes_what_epson_scan_wrote() {
        let text = include_str!("../testdata/74_film.txt");
        let reads = include_str!("../testdata/74_film_reads.txt");
        let request = crate::film::tests::CAPTURED[2].1;
        let (s, left) = film_scan("74", text, reads, &request, 10);
        assert_eq!(left, 0, "lines of the capture left");
        let c = &s.calibration;
        assert_eq!(c.offsets, [0x8C, 0x8C, 0x90]);
        assert_eq!(c.exposures, [13_550, 9070, 5953]);
        assert_eq!(c.gains, [0x21, 0x09, 0x16]);
        assert_eq!(c.image_gains, [0x21, 0x14, 0x1F]);
        assert_eq!(c.back, 240);
        let geo = c.plan.geometry;
        assert_eq!(s.raw.len() as u32, geo.lines * geo.line_bytes(16));
        assert_eq!(s.shading.table.len() as u32, geo.column_count() * 3);
        // As Epson Scan's TIFF: 576 x 874.
        assert_eq!((s.image.width, s.image.height), (576, 874));

        let mut g = Guarded::new(Follow::film("74", text, reads));
        calibrate_film(&mut g, &request, 10, &mut |_| {}, &mut |_| true)
            .unwrap_or_else(|e| panic!("{e}"));
        let f = g.inner();
        assert!(
            matches!(&f.lines[f.at], Line::W(w, None) if w == &[0x1E, 0x8B]),
            "{:?}",
            f.lines.get(f.at)
        );
    }

    /// A Digital ICE scan writes what Epson Scan wrote in captures 108 (a negative at 2400 dpi),
    /// 109 (at 1200) and 112 (a slide at 2400), write by write through both passes: the hole
    /// pre-reads and `1E 88` from the holes found, the infrared pass's opening, light, lid unit,
    /// `1E 87 01`, calibration (its exposures' last block), start move and columns from the
    /// holes; the calibration reads made up from what the rules take from the real ones (the
    /// holes as the runs of samples above the threshold).
    #[test]
    fn an_ice_scan_writes_what_epson_scan_wrote() {
        let ice_cases = crate::ice::tests::ICE;
        let cases = [
            (
                "108",
                include_str!("../testdata/108_ice.txt"),
                include_str!("../testdata/108_ice_reads.txt"),
                ice_cases[1],
            ),
            (
                "109",
                include_str!("../testdata/109_ice.txt"),
                include_str!("../testdata/109_ice_reads.txt"),
                ice_cases[2],
            ),
            (
                "112",
                include_str!("../testdata/112_ice.txt"),
                include_str!("../testdata/112_ice_reads.txt"),
                ice_cases[4],
            ),
        ];
        for (name, text, reads, (_, request, _, holes, ir_holes, _, istart, cols, _, lines)) in
            cases
        {
            let mut g = Guarded::new(Follow::film(name, text, reads));
            let s = scan_ice(&mut g, &request, 10, &mut |_| {}, &mut |_| true)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let f = g.inner();
            assert_eq!(f.lines.len() - f.at, 0, "{name}: lines of the capture left");
            // Over: the guard lets out no film's command any more.
            assert!(g.bulk_out(&[0x1E, 0x25]).is_err());
            assert_eq!(s.colour.calibration.holes.map(|h| h.values), Some(holes));
            assert_eq!(
                s.infrared_calibration.holes.map(|h| h.values),
                Some(ir_holes)
            );
            let gi = s.infrared_calibration.plan.geometry;
            assert_eq!(
                (gi.start_steps, gi.columns, gi.lines),
                (istart, cols, lines)
            );
            assert_eq!(
                s.infrared_raw.len() as u32,
                gi.lines * gi.line_bytes(16),
                "{name}"
            );
            let img = &s.colour.image;
            assert_eq!((img.width, img.height), (request.width, request.height));
            assert_eq!(s.infrared.len() as u32, img.width * img.height);
        }
    }

    /// The same for capture 77 (base 4800, a third exposure round), 80 (a slide, Epson Scan's
    /// image factor 15) and slide 1 at 600, 1200 and 4800 dpi (99, 100, 101: factor 10; at 4800 a
    /// line time of 27,264 µs).
    #[test]
    #[allow(clippy::type_complexity)]
    fn films_scans_at_4800_and_of_a_slide_are_epson_scans() {
        let cases: [(
            &str,
            &str,
            &str,
            usize,
            u32,
            [u8; 3],
            [u32; 3],
            [u8; 3],
            [u8; 3],
        ); 5] = [
            (
                "77",
                include_str!("../testdata/77_film.txt"),
                include_str!("../testdata/77_film_reads.txt"),
                6,
                10,
                [0x84, 0x80, 0x88],
                [13_934, 9326, 6120],
                [0x20, 0x09, 0x14],
                [0x20, 0x14, 0x1E],
            ),
            (
                "80",
                include_str!("../testdata/80_film.txt"),
                include_str!("../testdata/80_film_reads.txt"),
                9,
                15,
                [0x88, 0x80, 0x88],
                [16_213, 5520, 5075],
                [0x18, 0x1B, 0x18],
                [0x18, 0x1B, 0x18],
            ),
            (
                "99",
                include_str!("../testdata/99_film.txt"),
                include_str!("../testdata/99_film_reads.txt"),
                11,
                10,
                [0x90, 0x8C, 0x90],
                [18_168, 6219, 5927],
                [0x18, 0x1B, 0x18],
                [0x18, 0x1B, 0x18],
            ),
            (
                "100",
                include_str!("../testdata/100_film.txt"),
                include_str!("../testdata/100_film_reads.txt"),
                12,
                10,
                [0x90, 0x8C, 0x90],
                [18_084, 6176, 5967],
                [0x18, 0x1B, 0x18],
                [0x18, 0x1B, 0x18],
            ),
            (
                "101",
                include_str!("../testdata/101_film.txt"),
                include_str!("../testdata/101_film_reads.txt"),
                13,
                10,
                [0x88, 0x80, 0x88],
                [18_078, 6133, 5772],
                [0x18, 0x1B, 0x18],
                [0x18, 0x1B, 0x18],
            ),
        ];
        for (name, text, reads, i, factor, offsets, exposures, gains, image_gains) in cases {
            let request = crate::film::tests::CAPTURED[i].1;
            let (s, left) = film_scan(name, text, reads, &request, factor);
            assert_eq!(left, 0, "{name}: lines of the capture left");
            let c = &s.calibration;
            assert_eq!(
                (c.offsets, c.exposures, c.gains, c.image_gains),
                (offsets, exposures, gains, image_gains),
                "{name}"
            );
            let geo = c.plan.geometry;
            assert_eq!(s.raw.len() as u32, geo.lines * geo.line_bytes(16), "{name}");
            // At 4800 dpi the lines read at 5400 brought to 4800 (101: 5102 to 4535, as Epson
            // Scan's TIFF).
            assert_eq!(
                (s.image.width, s.image.height),
                (request.width, request.height * request.dpi / request.ydpi),
                "{name}"
            );
        }
    }

    /// A film's image read stopped by the user goes on to its end (no `1E 18`, never captured
    /// on films), then ends as cancelled with the capture written whole.
    #[test]
    fn a_films_read_is_not_cut_short() {
        let text = include_str!("../testdata/74_film.txt");
        let reads = include_str!("../testdata/74_film_reads.txt");
        let request = crate::film::tests::CAPTURED[2].1;
        let mut g = Guarded::new(Follow::film("74", text, reads));
        let r = scan_film(&mut g, &request, 10, &mut |_| {}, &mut |e| {
            !matches!(e, Event::Reading { .. })
        });
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
        let f = g.inner();
        assert_eq!(f.at, f.lines.len());
    }

    /// The carriages found elsewhere than the white read leaves them: nothing more moves.
    #[test]
    fn a_film_image_starts_only_from_where_the_white_read_left_the_carriages() {
        let text = include_str!("../testdata/74_film.txt").replace("A 0a08d804", "A 0b08d804");
        let reads = include_str!("../testdata/74_film_reads.txt");
        let request = crate::film::tests::CAPTURED[2].1;
        let mut g = Guarded::new(Follow::film("74", &text, reads));
        let e = scan_film(&mut g, &request, 10, &mut |_| {}, &mut |_| true)
            .unwrap_err()
            .to_string();
        assert!(e.contains("(2059, 1240)"), "{e}");
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

    /// A film's read made up: every 16-bit sample tells its line, pixel and channel.
    fn film_read(p: &FilmPlan) -> (Vec<u8>, impl Fn(usize, usize, usize) -> u32) {
        let f = |l: usize, i: usize, c: usize| ((l * 131 + i * 17 + c * 7) % 60_000) as u32;
        let g = p.geometry;
        let mut raw = Vec::new();
        for l in 0..g.lines as usize {
            for i in 0..g.pixels as usize {
                for c in 0..3 {
                    raw.extend_from_slice(&(f(l, i, c) as u16).to_le_bytes());
                }
            }
        }
        (raw, f)
    }

    fn px16(img: &Image, x: usize, y: usize, c: usize) -> u32 {
        let at = ((y * img.width as usize + x) * 3 + c) * 2;
        u32::from(u16::from_le_bytes([img.data[at], img.data[at + 1]]))
    }

    /// A film's image as Epson Scan's TIFFs show it: at 600 dpi the colours alone; at 2400 the
    /// odd sensor columns 4 lines later and two pixels read a pixel, 1/4, 1/2, 1/4 around the
    /// second; at 4800 x 5400 the odd columns 9 lines later and the lines reduced to 4800.
    #[test]
    fn films_are_put_together_as_epson_scans_tiffs() {
        let small = |i: usize, w: u32, h: u32| {
            let mut r = crate::film::tests::CAPTURED[i].1;
            (r.width, r.height) = (w, h);
            FilmPlan::new(r, XCorrection(-122)).unwrap()
        };
        // 600 dpi (capture 74's): red 24 lines below blue, green 12.
        let p = small(2, 10, 5);
        let (raw, f) = film_read(&p);
        let img = assemble_film(&raw, &p);
        assert_eq!((img.width, img.height, img.bits), (10, 5, 16));
        for (x, y) in [(0, 0), (9, 4), (3, 2)] {
            assert_eq!(px16(&img, x, y, 0), f(y + 24, x, 0));
            assert_eq!(px16(&img, x, y, 1), f(y + 12, x, 1));
            assert_eq!(px16(&img, x, y, 2), f(y, x, 2));
        }
        // 2400 dpi (capture 77's, start column 2410, even): read pixel i is late when i is odd.
        let p = small(6, 20, 10);
        assert_eq!((p.geometry.columns.0, film_stagger(&p)), (2410, 4));
        let (raw, f) = film_read(&p);
        let img = assemble_film(&raw, &p);
        assert_eq!((img.width, img.height), (20, 10));
        for (x, y, c, off) in [(0, 0, 2, 0), (5, 3, 1, 48), (19, 9, 0, 96)] {
            let want = (f(y + off, 2 * x, c)
                + 2 * f(y + off + 4, 2 * x + 1, c)
                + f(y + off, 2 * x + 2, c)
                + 2)
                / 4;
            assert_eq!(px16(&img, x, y, c), want, "{x}, {y}, {c}");
        }
        // 4800 x 5400 (capture 76's, start column 2423, odd): even pixels late, 9 lines.
        let p = small(4, 16, 90);
        assert_eq!((p.geometry.columns.0, film_stagger(&p)), (2423, 9));
        let (raw, f) = film_read(&p);
        let img = assemble_film(&raw, &p);
        assert_eq!((img.width, img.height), (16, 80));
        // Line 0 is 8/9 of line 0 and 1/9 of line 1 (5400 to 4800), blue at 0.
        let mix = |a: u32, b: u32| (8 * a + b + 4) / 9;
        assert_eq!(px16(&img, 1, 0, 2), mix(f(0, 1, 2), f(1, 1, 2)));
        assert_eq!(px16(&img, 0, 0, 2), mix(f(9, 0, 2), f(10, 0, 2)));
        assert_eq!(px16(&img, 0, 0, 0), mix(f(225, 0, 0), f(226, 0, 0)));
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
