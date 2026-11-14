// SPDX-License-Identifier: GPL-2.0-or-later
//! A guard on everything the backend writes to the scanner, so nothing can
//! move the carriage or switch the lamp but a scan inside what the captures
//! of Epson's driver have shown.
//!
//! Rather than a list of what is forbidden, the guard lets through a list of
//! what is allowed:
//!
//! * always: `1B 03` (state), `1E 86` (the two positions) and `1E 85` (the
//!   button pressed), read-only; `1E A2 00`, the lamp off;
//! * once [`Transport::permit_upload`] has given it a checked firmware, and
//!   outside a scan: its upload, `1B 06`, then only its own data (the length
//!   `01 00 01 00`, that firmware's 65,536 bytes, their checksum), `1B 16`
//!   right after them, and `80` right after `1B 16`;
//! * during a glass scan ([`crate::scan`]), once [`Guarded::permit_request`]
//!   has accepted the request: the opening and the read-only commands of
//!   Epson's sequence, the gamma tables, that request's `1E 57`, the lamp
//!   (`1E A2 00`/`01`) and `1E 87`;
//! * once [`Guarded::permit_plan`] has accepted the plan computed from it:
//!   exactly the plan's blocks (`1E 11`, `21`, `41`, `43`, `46`, the ramps
//!   and moves `1E 84` type 04 and `1E 01`, `1E 05` only right after the start
//!   move's `1E 01`, once,
//!   the tone tables and a shading table as wide as its columns), `1E 42`,
//!   `1E 31` and `1E 47`.
//!
//! What a scan may ask ([`check_request`], [`check_plan`]): the glass, colour,
//! 100, 200, 300, 400, 600 and 1200 dpi, at 8 bits (WIA) or 16 (Epson Scan,
//! the whole glass at each); an area inside the glass; the line times
//! (so the motor's speeds) each resolution was captured with; a start move
//! of a kind seen; no read further than the furthest captured. Refused, as
//! not captured: 800 (WIA asks 1200), 900, 2400 and 4800 dpi, black and
//! white, films, Epson Scan's draft mode. Anything else is refused with an
//! error and never reaches the scanner.

use crate::firmware::{HEADER_LEN, PAYLOAD_LEN};
use crate::protocol::{
    checksum, FIRMWARE_BEGIN, FIRMWARE_END, FIRMWARE_RUN, GET_BUTTONS, GET_STATE, TABLE_CHUNK,
};
use crate::scan::{CANCEL, READ_83};
use crate::setup::{self, MoveKind, Plan, Request};
use crate::table07;
use crate::tables::{self, Curves};
use crate::Transport;
use scanner_api::{Error, Result};

/// Reads the two 16-bit positions (carriage, transparency unit): 4 bytes.
pub const GET_POSITIONS: [u8; 2] = [0x1E, 0x86];

/// The resolutions and depths a glass scan may use: the ones captured (WIA at 8 bits; Epson
/// Scan at 16, captures 15 and 38-43, 12 November 2026; Epson Scan at 2400 dpi, 8 and 16 bits,
/// read along at 3200, captures 13, 57 and 58, 13 November 2026; Epson Scan at 4800 dpi, 8 and
/// 16 bits, captures 14 and 59-62, 14 November 2026).
pub const COVERED: [(u32, u8); 16] = [
    (100, 8),
    (200, 8),
    (300, 8),
    (400, 8),
    (600, 8),
    (1200, 8),
    (100, 16),
    (200, 16),
    (300, 16),
    (400, 16),
    (600, 16),
    (1200, 16),
    (2400, 8),
    (2400, 16),
    (4800, 8),
    (4800, 16),
];

/// The line times (µs) each resolution was captured with: the motor's speed
/// follows from them, so no other one goes out.
pub const LINE_TIMES: [(u32, u32); 10] = [
    (100, 4224),
    (200, 3840),
    (300, 3840),
    (300, 4224),
    (400, 3840),
    (600, 3840),
    (600, 4224),
    (1200, 10_752),
    (2400, 10_752),
    (4800, 0x41C0),
];

/// The furthest step from home a captured read reached (capture 42, Epson Scan's whole glass
/// at 1200 dpi; WIA's A4 at 300 dpi, capture 07, reached 29,587).
pub const MAX_READ_END: u32 = 29_601;

/// The glass as Epson's user guide gives it (216 x 297 mm, 8.5 x 11.7 in): no area outside it.
/// The application is offered a little less (`crate::GLASS_INCHES`), so that its whole glass
/// stays within the furthest read captured.
pub const PHYSICAL_GLASS_INCHES: (f64, f64) = (8.5, 11.7);

/// The x origin's corrections the scanner may give (4800ths of an inch):
/// Matteo's 4490 has -93; a value far from it is more likely a wrong answer.
pub const X_CORRECTION_RANGE: std::ops::RangeInclusive<i16> = -300..=300;

/// The commands known to move something, with what they do, for the
/// error message.
const MOVERS: [(u8, &str); 9] = [
    (0x01, "the parameters of a carriage move"),
    (0x05, "starts a carriage move"),
    (0x25, "selects what moves"),
    (0x47, "starts a scan"),
    (0x48, "starts a scan"),
    (0x84, "loads a table"),
    (0xF4, "moves the carriage"),
    (0xA2, "switches the lamp"),
    (0x77, "switches the lamp"),
];

/// The payload's length, as the upload sends it.
const UPLOAD_LENGTH: [u8; 4] = (PAYLOAD_LEN as u32).to_le_bytes();

/// Whether a glass scan of `r` may be sent: the resolutions, depth and area
/// the captures cover.
pub fn check_request(r: &Request) -> Result<()> {
    let refuse = |why: String| Err(Error::InvalidRequest(format!("Epson 4490: {why}")));
    // Along: the resolution across, or 2400 at 3200 only (`setup::READ_ALONG`).
    let along = setup::READ_ALONG
        .iter()
        .find(|(d, _)| *d == r.dpi)
        .map_or(r.dpi, |&(_, y)| y);
    if r.ydpi != along {
        return refuse(format!(
            "{} x {} dpi was never captured (only {} x {along})",
            r.dpi, r.ydpi, r.dpi
        ));
    }
    if !COVERED.contains(&(r.dpi, r.bits)) {
        return refuse(format!(
            "{} dpi at {} bits was never captured on the glass (only 100, 200, 300, 400, 600, \
             1200, 2400 and 4800 dpi, at 8 or 16 bits)",
            r.dpi, r.bits
        ));
    }
    // The physical glass, counted as the application counts pixels (along at `ydpi`); how far
    // the carriage may go is `check_plan`'s.
    let px = |inches: f64, dpi: u32| (inches * f64::from(dpi)).round() as u64;
    let max_w = px(PHYSICAL_GLASS_INCHES.0, r.dpi);
    let max_h = px(PHYSICAL_GLASS_INCHES.1, r.ydpi);
    let end = |a: u32, b: u32| u64::from(a) + u64::from(b);
    if r.width == 0 || r.height == 0 || end(r.x, r.width) > max_w || end(r.y, r.height) > max_h {
        return refuse(format!(
            "the area {}x{} at {},{} is not inside the glass ({max_w} x {max_h} pixels at {} x {} \
             dpi)",
            r.width, r.height, r.x, r.y, r.dpi, r.ydpi
        ));
    }
    setup::request_block(r)?;
    Ok(())
}

/// Whether a plan may be sent: its request accepted, and every move and
/// speed one the captures have shown.
pub fn check_plan(p: &Plan) -> Result<()> {
    check_request(&p.request)?;
    let refuse = |why: String| Err(Error::InvalidRequest(format!("Epson 4490: {why}")));
    let r = &p.request;
    if !X_CORRECTION_RANGE.contains(&p.x_correction.0) {
        return refuse(format!(
            "1E 87 gave an x correction of {}, far from the -93 seen: not trusted",
            p.x_correction.0
        ));
    }
    if !LINE_TIMES.contains(&(r.dpi, p.t)) {
        return refuse(format!(
            "this area needs a line time of {} µs at {} dpi, never captured (the motor would \
             run at {} steps/s): make it narrower",
            p.t, r.dpi, p.speed
        ));
    }
    if p.starts_late {
        return refuse("the read would start lower than asked: never seen".into());
    }
    match &p.start_move {
        None => {
            return refuse(format!(
                "this position needs no start move, a sequence never captured: move the area by \
                 a pixel (y {})",
                r.y
            ))
        }
        Some(m) if m.kind == MoveKind::Medium => {
            return refuse(format!(
                "a start move of {} steps uses the 256-value ramp, never captured",
                m.steps
            ))
        }
        Some(m) if m.kind == MoveKind::Short && m.steps >= p.num => {
            return refuse(format!(
                "a short start move of {} steps, never captured",
                m.steps
            ))
        }
        Some(_) => {}
    }
    if p.read_end > MAX_READ_END {
        return refuse(format!(
            "the read would end {} steps from home, further than any capture ({MAX_READ_END})",
            p.read_end
        ));
    }
    Ok(())
}

/// What may come after a command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expect {
    /// One of these blocks.
    OneOf(Vec<Vec<u8>>),
    /// One of the plan's `1E 01` blocks (the start move's arms `1E 05`).
    Move(Vec<Vec<u8>>),
    /// Any block of this length (values computed from the calibration, or
    /// the scanner's own written back).
    Len(usize),
    /// One of the plan's `1E 41`: the image's with any byte 15 (the scales).
    Block41,
    /// `1E B3`: a channel, then the gamma table.
    GammaChannel,
    /// `1E 84`: a header the plan allows.
    TableHeader,
}

/// What a scan may send.
#[derive(Debug, Clone)]
struct Permit {
    request: Request,
    e57: [u8; 42],
    plan: Option<Plan>,
}

/// Where the firmware upload is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// No upload: only the allowed commands.
    Idle,
    /// After `1B 06`: the length comes next.
    Length,
    /// The firmware's bytes, this many left (the scanner is taking data).
    Payload(usize),
    /// The checksum byte comes next.
    Checksum,
    /// After the checksum: `1B 16` may follow.
    Uploaded,
    /// After `1B 16`: `80` may follow.
    Run,
}

/// A [`Transport`] that only lets the allowed writes through.
#[derive(Debug)]
pub struct Guarded<T: Transport> {
    inner: T,
    stage: Stage,
    permit: Option<Permit>,
    /// What the next write must be, after a command that takes a block.
    expect: Option<Expect>,
    /// `1E 18` may go out without a scan: a scanner left half-way through one
    /// ([`Guarded::permit_recovery`]).
    recovering: bool,
    /// The payload of the firmware [`Transport::permit_upload`] gave, until it is run.
    upload: Option<Vec<u8>>,
    /// The last write was the start move's `1E 01` block: `1E 05` may follow.
    start_armed: bool,
    /// The start move's `1E 05` went out: not again in this scan.
    start_done: bool,
    /// The last command that went out, `1B 03` left aside: where a scan stopped.
    last_command: Option<[u8; 2]>,
}

impl<T: Transport> Guarded<T> {
    /// Guards `inner`.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            stage: Stage::Idle,
            permit: None,
            expect: None,
            recovering: false,
            upload: None,
            start_armed: false,
            start_done: false,
            last_command: None,
        }
    }

    /// The last command that went out (`1B 03`, the state, left aside), to say where a scan
    /// stopped.
    pub fn last_command(&self) -> Option<[u8; 2]> {
        self.last_command
    }

    /// The guarded transport.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Lets out a glass scan of `request` (the commands before its plan is
    /// known), if [`check_request`] accepts it.
    pub fn permit_request(&mut self, request: &Request) -> Result<()> {
        check_request(request)?;
        self.permit = Some(Permit {
            request: *request,
            e57: setup::request_block(request)?,
            plan: None,
        });
        self.expect = None;
        self.start_armed = false;
        self.start_done = false;
        Ok(())
    }

    /// Lets out the blocks of `plan`, computed for the request already
    /// permitted, if [`check_plan`] accepts it.
    pub fn permit_plan(&mut self, plan: &Plan) -> Result<()> {
        let permit = self.permit.as_mut().ok_or_else(|| {
            Error::Other("Epson 4490: a plan without its request permitted".into())
        })?;
        if permit.request != plan.request {
            return Err(Error::Other(
                "Epson 4490: the plan is not for the request permitted".into(),
            ));
        }
        check_plan(plan)?;
        permit.plan = Some(plan.clone());
        Ok(())
    }

    /// Back to the commands that move nothing.
    pub fn end_scan(&mut self) {
        self.permit = None;
        self.expect = None;
        self.recovering = false;
        self.start_armed = false;
        self.start_done = false;
    }

    /// Lets `1E 18` out without a scan, to stop the one a scanner was left in by a program
    /// that went away half-way through it (capture 47, 12 November 2026). Until
    /// [`Guarded::end_scan`].
    pub fn permit_recovery(&mut self) {
        self.recovering = true;
    }

    /// Whether `data` may be written now: the stage and what to expect after.
    fn check(&self, data: &[u8]) -> Result<(Stage, Option<Expect>)> {
        let idle = |e| Ok((Stage::Idle, e));
        match (self.stage, self.upload.as_deref()) {
            (Stage::Length, Some(_)) if data == UPLOAD_LENGTH => {
                return Ok((Stage::Payload(PAYLOAD_LEN - 1), None))
            }
            (Stage::Payload(left), Some(payload)) if !data.is_empty() && data.len() <= left => {
                // Only the permitted firmware's own bytes, in their place.
                let at = PAYLOAD_LEN - 1 - left;
                if payload[at..at + data.len()] == *data {
                    return Ok((
                        match left - data.len() {
                            0 => Stage::Checksum,
                            left => Stage::Payload(left),
                        },
                        None,
                    ));
                }
            }
            (Stage::Checksum, Some(payload)) if data == [checksum(payload)] => {
                return Ok((Stage::Uploaded, None))
            }
            (Stage::Uploaded, Some(_)) if data == FIRMWARE_END => return Ok((Stage::Run, None)),
            (Stage::Run, Some(_)) if data == [FIRMWARE_RUN] => return idle(None),
            // Anything else ends an upload: judged as a command.
            _ => {}
        }
        if let Some(expect) = &self.expect {
            return self.block(expect, data).map(|next| (Stage::Idle, next));
        }
        match data {
            d if d == GET_STATE || d == GET_POSITIONS || d == GET_BUTTONS => return idle(None),
            // An upload only of a firmware given to the guard, never during a scan.
            d if d == FIRMWARE_BEGIN && self.upload.is_some() && self.permit.is_none() => {
                return Ok((Stage::Length, None))
            }
            _ => {}
        }
        let Some(permit) = &self.permit else {
            // Outside a scan the lamp may only be switched off (the app's lamp timer).
            if data == crate::protocol::LAMP {
                return idle(one(&[[0x00]]));
            }
            if self.recovering && data == CANCEL {
                return idle(None);
            }
            return Err(refusal(data, "no scan permitted"));
        };
        let request_level = match data {
            [0x1B, 0x40] | [0x1B, 0x13] => Some(None),
            [0x1E, 0x53 | 0xA1 | 0x92 | 0x8C | 0x04 | 0x8B] => Some(None),
            [0x1E, 0x87] => Some(one(&[&[0x00]])),
            [0x1E, 0xA2] => Some(one(&[&[0x00], &[0x01]])),
            // The lid's lamp switched back to the glass's (WIA's driver, capture 25); no block.
            [0x1E, 0x77] => Some(None),
            [0x1E, 0xB3] => Some(Some(Expect::GammaChannel)),
            [0x1E, 0x57] => Some(one(&[&permit.e57])),
            [0x1E, 0x31] => Some(Some(Expect::Len(12))),
            _ => None,
        };
        if let Some(next) = request_level {
            return idle(next);
        }
        let Some(p) = &permit.plan else {
            return Err(refusal(data, "the scan's plan is not permitted yet"));
        };
        let next = match data {
            [0x1E, 0x83] if p.asks_table07() => one(&[READ_83, table07::READ_RAW]),
            [0x1E, 0x11] => one(&[&[p.e11]]),
            [0x1E, 0x46] => one(&[&p.e46[0], &p.e46[1]]),
            [0x1E, 0x21] => one(&[&p.e21_black, &p.e21_cal, &p.e21_data]),
            [0x1E, 0x41] => Some(Expect::Block41),
            [0x1E, 0x42] => Some(Expect::Len(18)),
            [0x1E, 0x43] => one(&[&setup::GAIN_CAL, &setup::GAIN_DATA]),
            [0x1E, 0x84] => Some(Expect::TableHeader),
            [0x1E, 0x01] => {
                let mut moves: Vec<Vec<u8>> = vec![p.move_cal.to_vec(), p.move_data.to_vec()];
                if let Some(m) = &p.start_move {
                    moves.push(m.block.to_vec());
                }
                Some(Expect::Move(moves))
            }
            // Only right after the start move's 1E 01, and once.
            [0x1E, 0x05] if self.start_armed && !self.start_done => None,
            [0x1E, 0x47] => None,
            // Stops the read (capture 11).
            [0x1E, 0x18] => None,
            _ => return Err(refusal(data, "not in the scan's plan")),
        };
        idle(next)
    }

    /// Whether `data` is a block `expect` allows; what comes after it.
    fn block(&self, expect: &Expect, data: &[u8]) -> Result<Option<Expect>> {
        let plan = self.permit.as_ref().and_then(|p| p.plan.as_ref());
        let bad = |what: &str| {
            Err(Error::Other(format!(
                "refused to send a block of {} bytes to the Epson 4490 ({what}): not one the \
                 scan's plan computed",
                data.len()
            )))
        };
        match expect {
            Expect::OneOf(blocks) | Expect::Move(blocks) if blocks.iter().any(|b| b == data) => {
                Ok(None)
            }
            Expect::OneOf(_) | Expect::Move(_) => bad("after its command"),
            Expect::Len(n) if data.len() == *n => Ok(None),
            // A long table in pieces, as Epson Scan wrote one (`protocol::TABLE_CHUNK`).
            Expect::Len(n) if *n > TABLE_CHUNK && data.len() == TABLE_CHUNK => {
                Ok(Some(Expect::Len(n - TABLE_CHUNK)))
            }
            Expect::Len(_) => bad("of the wrong length"),
            Expect::Block41 => {
                let p = plan.expect("1E 41 needs a plan");
                let mut image = p.e41_data;
                if data.len() == 22 {
                    image[15] = data[15];
                }
                if [p.e41_black, p.e41_white, p.e41_dark, image]
                    .iter()
                    .any(|b| b[..] == *data)
                {
                    Ok(None)
                } else {
                    bad("1E 41")
                }
            }
            Expect::GammaChannel => match data {
                [0..=2] => Ok(Some(Expect::OneOf(
                    [Curves::Wia, Curves::Linear]
                        .iter()
                        .map(|c| c.gamma().to_vec())
                        .collect(),
                ))),
                _ => bad("1E B3's channel"),
            },
            Expect::TableHeader => {
                let p = plan.expect("1E 84 needs a plan");
                let ramps = std::iter::once(&p.ramp_cal)
                    .chain(p.start_move.as_ref().map(|m| &m.ramp))
                    .chain(std::iter::once(&p.ramp_data));
                let mut tables_: Vec<Vec<u8>> = Vec::new();
                for r in ramps {
                    if setup::ramp_header(r.len())[..] == *data {
                        tables_.push(r.iter().flat_map(|v| v.to_le_bytes()).collect());
                    }
                }
                if !tables_.is_empty() {
                    return Ok(Some(Expect::OneOf(tables_)));
                }
                if p.asks_table07() && table07::HEADER[..] == *data {
                    return Ok(Some(Expect::Len(table07::LEN)));
                }
                if tables::TONE_HEADERS.iter().any(|h| h[..] == *data) {
                    return Ok(Some(Expect::OneOf(vec![
                        Curves::Wia.tone(),
                        Curves::Linear.tone(),
                    ])));
                }
                if setup::shading_header(p.column_count())[..] == *data {
                    return Ok(Some(Expect::Len(p.column_count() as usize * 6)));
                }
                bad("1E 84's header")
            }
        }
    }
}

/// Any of `blocks` next.
fn one<B: AsRef<[u8]>>(blocks: &[B]) -> Option<Expect> {
    Some(Expect::OneOf(
        blocks.iter().map(|b| b.as_ref().to_vec()).collect(),
    ))
}

impl<T: Transport> Guarded<T> {
    /// Writes `data` with `write` if [`Guarded::check`] lets it out. A refusal or a failed
    /// write leaves nothing awaited (no block, no upload stage, no `1E 05` armed).
    fn pass(&mut self, data: &[u8], write: impl FnOnce(&mut T, &[u8]) -> Result<()>) -> Result<()> {
        let sent = self
            .check(data)
            .and_then(|next| write(&mut self.inner, data).map(|()| next));
        let (stage, expect) = match sent {
            Ok(next) => next,
            Err(e) => {
                self.stage = Stage::Idle;
                self.expect = None;
                self.start_armed = false;
                return Err(e);
            }
        };
        let was_command = self.expect.is_none() && self.stage == Stage::Idle;
        let start_block = self
            .permit
            .as_ref()
            .and_then(|p| p.plan.as_ref())
            .and_then(|p| p.start_move.as_ref())
            .is_some_and(|m| m.block[..] == *data);
        self.start_armed = matches!(self.expect, Some(Expect::Move(_))) && start_block;
        if was_command && data == [0x1E, 0x05] {
            self.start_done = true;
        }
        if was_command && data.len() == 2 && matches!(data[0], 0x1B | 0x1E) && data != GET_STATE {
            self.last_command = Some([data[0], data[1]]);
        }
        if self.stage == Stage::Run && stage == Stage::Idle {
            // 80 went out: the upload is over.
            self.upload = None;
        }
        self.stage = stage;
        self.expect = expect;
        Ok(())
    }
}

fn refusal(data: &[u8], why: &str) -> Error {
    let head: Vec<String> = data.iter().take(4).map(|b| format!("{b:02X}")).collect();
    let more = if data.len() > 4 { " ..." } else { "" };
    let what = match data {
        [0x1E, code, ..] => MOVERS
            .iter()
            .find(|(c, _)| c == code)
            .map_or("not a command this backend may send", |(_, what)| *what),
        _ => "not a command this backend may send",
    };
    Error::Other(format!(
        "refused to send {}{more} to the Epson 4490 ({what}; {why}): nothing that may move the \
         carriage goes out but a scan inside what the captures of Epson's driver have shown",
        head.join(" ")
    ))
}

impl<T: Transport> Transport for Guarded<T> {
    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        self.pass(data, |t, data| t.bulk_out(data))
    }

    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        // The firmware's bytes go out without an answer in between: a read
        // there means the upload was given up.
        if matches!(self.stage, Stage::Payload(_) | Stage::Checksum) {
            self.stage = Stage::Idle;
        }
        let read = self.inner.bulk_in(buf);
        if read.is_err() {
            // The answer to a command never came: its block must not go out after it.
            self.stage = Stage::Idle;
            self.expect = None;
            self.start_armed = false;
        }
        read
    }

    fn hold(&mut self) -> Result<()> {
        self.inner.hold()
    }

    fn locks(&self) -> bool {
        self.inner.locks()
    }

    fn permit_upload(&mut self, image: &[u8]) {
        if crate::firmware::check_image(image).is_ok() {
            self.upload = Some(image[HEADER_LEN..].to_vec());
        }
    }

    fn write_reading(
        &mut self,
        data: &[u8],
        on_read: &mut (dyn FnMut(&[u8]) -> bool + Send),
    ) -> Result<()> {
        self.pass(data, |t, data| t.write_reading(data, on_read))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::HEADER_LEN;
    use crate::protocol::tests::{image, Mock};
    use crate::protocol::{self, ACK};
    use crate::setup::tests::{parse, request_of, CAPTURES};
    use crate::setup::XCorrection;

    fn guarded(answers: &[&[u8]]) -> Guarded<Mock> {
        Guarded::new(Mock::answering(answers))
    }

    #[test]
    fn the_commands_that_move_are_refused_and_never_sent() {
        let movers: [&[u8]; 9] = [
            &[0x1E, 0x05],
            &[0x1E, 0xF4, 0x10, 0x00],
            &[0x1E, 0xF4],
            &[0x1E, 0x47],
            &[0x1E, 0x48],
            &[0x1E, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10, 0],
            &[0x1E, 0x84, 0xC4, 0x09],
            &[0x1E, 0x25, 0x00],
            &[0x1E, 0xA2, 0x01],
        ];
        let mut t = guarded(&[]);
        for m in movers {
            let e = t.bulk_out(m).unwrap_err().to_string();
            assert!(e.contains("refused to send 1E"), "{e}");
        }
        assert!(t
            .bulk_out(&[0x1E, 0x05])
            .unwrap_err()
            .to_string()
            .contains("starts a carriage move"));
        // Unknown commands too, and a lone 80.
        assert!(t.bulk_out(&[0x1E, 0x11, 0x00]).is_err());
        assert!(t.bulk_out(&[0x1B, 0x40]).is_err());
        assert!(t.bulk_out(&[FIRMWARE_RUN]).is_err());
        assert!(t.inner().writes.is_empty());
    }

    #[test]
    fn the_lamp_may_only_be_switched_off_outside_a_scan() {
        let mut t = guarded(&[]);
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        assert!(t.bulk_out(&[0x01]).is_err());
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        t.bulk_out(&[0x00]).unwrap();
        assert_eq!(
            t.inner().writes,
            [vec![0x1E, 0xA2], vec![0x1E, 0xA2], vec![0x00]]
        );
    }

    #[test]
    fn the_read_only_commands_go_out() {
        let mut t = guarded(&[]);
        t.bulk_out(&GET_STATE).unwrap();
        t.bulk_out(&GET_POSITIONS).unwrap();
        t.bulk_out(&GET_BUTTONS).unwrap();
        assert_eq!(
            t.inner().writes,
            vec![
                GET_STATE.to_vec(),
                GET_POSITIONS.to_vec(),
                GET_BUTTONS.to_vec()
            ]
        );
    }

    #[test]
    fn the_upload_goes_through_unchanged() {
        // A payload that starts like a move: it is data, not a command.
        let mut image = image();
        image[HEADER_LEN..HEADER_LEN + 2].copy_from_slice(&[0x1E, 0x05]);
        let mut direct = Mock::answering(&[&[ACK][..]; 4]);
        protocol::upload_firmware(&mut direct, &image).unwrap();
        let mut t = guarded(&[&[ACK][..]; 4]);
        t.permit_upload(&image);
        protocol::upload_firmware(&mut t, &image).unwrap();
        assert_eq!(t.inner().writes, direct.writes);
        // Over: 80, another upload and moves are refused again.
        assert!(t.bulk_out(&[FIRMWARE_RUN]).is_err());
        assert!(t.bulk_out(&FIRMWARE_BEGIN).is_err());
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
    }

    #[test]
    fn only_the_permitted_firmware_goes_out() {
        let image = image();
        let payload = &image[HEADER_LEN..];
        // Without a firmware given, no upload at all; nor 1B 16 and 80 alone.
        let mut t = guarded(&[]);
        assert!(t.bulk_out(&FIRMWARE_BEGIN).is_err());
        assert!(t.bulk_out(&FIRMWARE_END).is_err());
        assert!(t.bulk_out(&[FIRMWARE_RUN]).is_err());
        // Given one: other bytes in its place are refused.
        t.permit_upload(&image);
        t.bulk_out(&FIRMWARE_BEGIN).unwrap();
        t.bulk_out(&UPLOAD_LENGTH).unwrap();
        let mut other = payload[..100].to_vec();
        other[50] ^= 1;
        assert!(t.bulk_out(&other).is_err());
        // Its own bytes, then a wrong checksum: refused, and no 1B 16 after.
        t.bulk_out(&FIRMWARE_BEGIN).unwrap();
        t.bulk_out(&UPLOAD_LENGTH).unwrap();
        t.bulk_out(&payload[..PAYLOAD_LEN - 1]).unwrap();
        assert!(t.bulk_out(&[checksum(payload).wrapping_add(1)]).is_err());
        assert!(t.bulk_out(&FIRMWARE_END).is_err());
        // Never during a scan.
        let p = plan(300, 472, 709, 591, 591, 8).unwrap();
        t.permit_request(&p.request).unwrap();
        assert!(t.bulk_out(&FIRMWARE_BEGIN).is_err());
    }

    #[test]
    fn a_refused_upload_leaves_nothing_open() {
        // 1B 06 answered with a NAK: the upload stops after its command.
        let image = image();
        let mut t = guarded(&[&[0x15]]);
        t.permit_upload(&image);
        assert!(protocol::upload_firmware(&mut t, &image).is_err());
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
        // Stopped in the middle of the data, after a read.
        let mut t = guarded(&[&[ACK], &[0x00]]);
        t.permit_upload(&image);
        t.bulk_out(&FIRMWARE_BEGIN).unwrap();
        t.bulk_out(&UPLOAD_LENGTH).unwrap();
        t.bulk_out(&image[HEADER_LEN..HEADER_LEN + 100]).unwrap();
        t.bulk_in(&mut [0]).unwrap();
        assert!(t.bulk_out(&[0x1E, 0x47]).is_err());
        // More data than the firmware has is refused.
        let mut t = guarded(&[]);
        t.permit_upload(&image);
        t.bulk_out(&FIRMWARE_BEGIN).unwrap();
        t.bulk_out(&UPLOAD_LENGTH).unwrap();
        assert!(t.bulk_out(&image[HEADER_LEN - 1..]).is_err());
    }

    fn plan(dpi: u32, x: u32, y: u32, w: u32, h: u32, bits: u8) -> Result<Plan> {
        Plan::new(Request::glass(dpi, x, y, w, h, bits), XCorrection(-93))
    }

    /// Every capture's request and plan is accepted (WIA's, Epson Scan's at 16 bits and at
    /// 2400 x 3200 dpi), and the limits are the captures' own.
    #[test]
    fn the_captured_scans_are_accepted() {
        let mut furthest = 0;
        for (name, text) in CAPTURES {
            let (r, xc, _) = request_of(&parse(text));
            let p = Plan::new(r, xc).unwrap();
            furthest = furthest.max(p.read_end);
            let ok = check_plan(&p);
            assert!(ok.is_ok(), "{name}: {ok:?}");
        }
        assert_eq!(furthest, MAX_READ_END);
    }

    #[test]
    fn what_was_never_captured_is_refused() {
        // Resolutions and depths; 2400 read along at 2400, not 3200.
        for (dpi, bits) in [(800, 8), (2400, 8), (800, 16), (2400, 16), (150, 8)] {
            let r = Request::new(dpi, 0, 0, 400, 400, bits);
            assert!(check_request(&r).is_err(), "{dpi} {bits}");
        }
        for bits in [8, 16] {
            assert!(check_request(&Request::glass(2400, 0, 0, 400, 400, bits)).is_ok());
        }
        assert!(check_request(&Request {
            ydpi: 600,
            ..Request::new(300, 0, 0, 400, 400, 8)
        })
        .is_err());
        // Outside the glass: 8.5 x 11.7 in.
        assert!(check_request(&Request::new(300, 2000, 0, 551, 100, 8)).is_err());
        assert!(check_request(&Request::new(300, 2000, 0, 550, 100, 8)).is_ok());
        assert!(check_request(&Request::new(300, 0, 3500, 100, 11, 8)).is_err());
        assert!(check_request(&Request::new(300, 0, 3500, 100, 10, 8)).is_ok());
        // A line time never captured at a resolution: none left inside the glass since
        // capture 43 (the whole width at 600 dpi, 4224 µs), refused if one were.
        let mut p = plan(600, 0, 0, 5100, 1000, 8).unwrap();
        assert_eq!(p.t, 4224);
        assert!(check_plan(&p).is_ok());
        p.t = 4608;
        let e = check_plan(&p).unwrap_err().to_string();
        assert!(e.contains("line time"), "{e}");
        // A read further than any capture: refused.
        let mut p = plan(300, 0, 3400, 591, 100, 8).unwrap();
        assert!(check_plan(&p).is_ok());
        p.read_end = MAX_READ_END + 1;
        let e = check_plan(&p).unwrap_err().to_string();
        assert!(e.contains("further"), "{e}");
        // No start move: never needed on the glass at the resolutions covered (the near
        // case leaves (1267 - R) mod num steps, 3 or 11), but refused if it were.
        for (dpi, _) in COVERED {
            for y in 0..200 {
                let p = plan(dpi, 0, y, 400, 400, 8).unwrap();
                let m = p.start_move.as_ref().expect("a start move");
                assert!(m.kind != MoveKind::Medium, "{dpi} dpi y {y}");
                assert!(check_plan(&p).is_ok(), "{dpi} dpi y {y}");
            }
        }
        let mut p = plan(300, 0, 0, 591, 591, 8).unwrap();
        p.start_move = None;
        let e = check_plan(&p).unwrap_err().to_string();
        assert!(e.contains("no start move"), "{e}");
        // A wild x correction.
        let p = Plan::new(Request::new(300, 0, 0, 591, 591, 8), XCorrection(2000)).unwrap();
        assert!(check_plan(&p).is_err());
    }

    /// The whole glass the application is offered, at every resolution and depth covered: its
    /// preview (200 dpi) and the largest scans.
    #[test]
    fn the_cancel_goes_out_without_a_scan_only_to_recover() {
        let mut t = guarded(&[]);
        assert!(t.bulk_out(&CANCEL).is_err());
        t.permit_recovery();
        t.bulk_out(&CANCEL).unwrap();
        // Nothing else that moves.
        assert!(t.bulk_out(&[0x1E, 0x47]).is_err());
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
        t.end_scan();
        assert!(t.bulk_out(&CANCEL).is_err());
        assert_eq!(t.inner().writes, vec![CANCEL.to_vec()]);
    }

    #[test]
    fn the_whole_glass_is_accepted() {
        let caps = crate::capabilities();
        for (dpi, bits) in COVERED {
            let (w, h) = caps.glass_pixels(dpi as u16);
            // A line of the whole width is more than a block: the plan says how wide, and
            // that width is accepted at both edges.
            let narrow = [
                ((2400, 16), 10_912, "115 mm"),
                ((4800, 16), 10_912, "58 mm"),
                ((4800, 8), 21_840, "116 mm"),
            ];
            if let Some(&(_, most, mm)) = narrow.iter().find(|(k, ..)| *k == (dpi, bits)) {
                let e = plan(dpi, 0, 0, w, h, bits).unwrap_err().to_string();
                assert!(e.contains(&format!("at most {most} pixels ({mm})")), "{e}");
                for x in [0, w - most] {
                    let ok = check_plan(&plan(dpi, x, 0, most, h, bits).unwrap());
                    assert!(
                        ok.is_ok(),
                        "{dpi} dpi {bits} bits, {most} x {h} at {x}: {ok:?}"
                    );
                }
                continue;
            }
            let ok = check_plan(&plan(dpi, 0, 0, w, h, bits).unwrap());
            assert!(ok.is_ok(), "{dpi} dpi {bits} bits, {w} x {h}: {ok:?}");
        }
    }

    #[test]
    fn a_permitted_scan_sends_only_its_own_blocks() {
        let p = plan(300, 472, 709, 591, 591, 8).unwrap();
        let mut t = guarded(&[]);
        // Nothing of the scan before the permit.
        assert!(t.bulk_out(&[0x1E, 0x57]).is_err());
        t.permit_request(&p.request).unwrap();
        t.bulk_out(&[0x1B, 0x40]).unwrap();
        t.bulk_out(&[0x1E, 0x57]).unwrap();
        // Another request's block is refused.
        let other = setup::request_block(&Request::new(300, 0, 0, 591, 591, 8)).unwrap();
        assert!(t.bulk_out(&other).is_err());
        t.bulk_out(&[0x1E, 0x57]).unwrap();
        t.bulk_out(&p.e57).unwrap();
        // The plan's blocks need the plan.
        assert!(t.bulk_out(&[0x1E, 0x01]).is_err());
        t.permit_plan(&p).unwrap();
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        assert!(t.bulk_out(&setup::move_block(100, 0, 0, 0x18)).is_err());
        // 1E 05 only right after the start move's 1E 01, and once.
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        t.bulk_out(&p.move_data).unwrap();
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        t.bulk_out(&p.start_move.as_ref().unwrap().block).unwrap();
        t.bulk_out(&[0x1E, 0x05]).unwrap();
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        t.bulk_out(&p.start_move.as_ref().unwrap().block).unwrap();
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
        // A ramp: its header, then only its values.
        t.bulk_out(&[0x1E, 0x84]).unwrap();
        t.bulk_out(&setup::ramp_header(512)).unwrap();
        assert!(t.bulk_out(&vec![0x10; 1024]).is_err());
        t.bulk_out(&[0x1E, 0x84]).unwrap();
        t.bulk_out(&setup::ramp_header(512)).unwrap();
        let ramp: Vec<u8> = p.ramp_data.iter().flat_map(|v| v.to_le_bytes()).collect();
        t.bulk_out(&ramp).unwrap();
        // The image's 1E 41 with any scales; another geometry refused.
        t.bulk_out(&[0x1E, 0x41]).unwrap();
        let mut e41 = p.e41_data;
        e41[15] = 0x72;
        t.bulk_out(&e41).unwrap();
        t.bulk_out(&[0x1E, 0x41]).unwrap();
        e41[0] = 0;
        assert!(t.bulk_out(&e41).is_err());
        // WIA's curves or the identity, nothing else.
        for curves in [Curves::Wia, Curves::Linear] {
            t.bulk_out(&[0x1E, 0xB3]).unwrap();
            t.bulk_out(&[1]).unwrap();
            t.bulk_out(&curves.gamma()).unwrap();
            t.bulk_out(&[0x1E, 0x84]).unwrap();
            t.bulk_out(&tables::TONE_HEADERS[2]).unwrap();
            t.bulk_out(&curves.tone()).unwrap();
        }
        t.bulk_out(&[0x1E, 0xB3]).unwrap();
        t.bulk_out(&[1]).unwrap();
        assert!(t.bulk_out(&[0x80; 256]).is_err());
        t.bulk_out(&[0x1E, 0x84]).unwrap();
        t.bulk_out(&tables::TONE_HEADERS[0]).unwrap();
        assert!(t.bulk_out(&[0; 8192]).is_err());
        // The lamp, but not a third value (off and on during a scan; outside one, off only:
        // `the_lamp_may_only_be_switched_off_outside_a_scan`).
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        assert!(t.bulk_out(&[0x02]).is_err());
        // Over: moves refused again.
        t.end_scan();
        assert!(t.bulk_out(&[0x1E, 0x01]).is_err());
        assert!(t.bulk_out(&[0x1E, 0x47]).is_err());
    }

    #[test]
    fn an_answer_that_never_came_leaves_no_block_awaited() {
        // 1E A2 out, its 06 never read: the next command is judged as a command.
        let mut t = guarded(&[]);
        assert!(protocol::lamp_off(&mut t).is_err());
        t.bulk_out(&GET_STATE).unwrap();
        assert_eq!(t.inner().writes, [vec![0x1E, 0xA2], GET_STATE.to_vec()]);
    }

    /// A shading longer than a piece goes out in pieces of `TABLE_CHUNK` and the rest (as
    /// Epson Scan's capture 42), or whole; no other cut, nothing past its length.
    #[test]
    fn a_long_shading_goes_in_pieces() {
        // 1200 dpi, 150 mm wide: 85,056 bytes of shading.
        let p = plan(1200, 1417, 1890, 7087, 7087, 16).unwrap();
        let len = p.column_count() as usize * 6;
        assert!(len > TABLE_CHUNK);
        let mut t = guarded(&[]);
        t.permit_request(&p.request).unwrap();
        t.permit_plan(&p).unwrap();
        let header = setup::shading_header(p.column_count());
        t.bulk_out(&[0x1E, 0x84]).unwrap();
        t.bulk_out(&header).unwrap();
        t.bulk_out(&vec![0; TABLE_CHUNK]).unwrap();
        t.bulk_out(&vec![0; len - TABLE_CHUNK]).unwrap();
        t.bulk_out(&[0x1E, 0x84]).unwrap();
        t.bulk_out(&header).unwrap();
        t.bulk_out(&vec![0; len]).unwrap();
        t.bulk_out(&[0x1E, 0x84]).unwrap();
        t.bulk_out(&header).unwrap();
        assert!(t.bulk_out(&vec![0; 4096]).is_err());
        t.bulk_out(&[0x1E, 0x84]).unwrap();
        t.bulk_out(&header).unwrap();
        t.bulk_out(&vec![0; TABLE_CHUNK]).unwrap();
        assert!(t.bulk_out(&vec![0; len]).is_err());
    }

    #[test]
    fn an_area_past_the_end_of_u32_is_refused() {
        assert!(check_request(&Request::new(300, u32::MAX - 10, 0, 100, 100, 8)).is_err());
        assert!(check_request(&Request::new(300, 0, u32::MAX - 10, 100, 100, 8)).is_err());
    }

    #[test]
    fn a_plan_is_permitted_only_for_its_request() {
        let mut t = guarded(&[]);
        let p = plan(300, 472, 709, 591, 591, 8).unwrap();
        assert!(t.permit_plan(&p).is_err());
        t.permit_request(&Request::new(300, 0, 0, 591, 591, 8))
            .unwrap();
        assert!(t.permit_plan(&p).is_err());
        assert!(t
            .permit_request(&Request::new(2400, 0, 0, 591, 591, 8))
            .is_err());
    }
}
