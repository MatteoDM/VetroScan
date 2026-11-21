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
//!   `1E 31` and `1E 47`;
//! * during a film's opening, once [`Guarded::permit_film`] has accepted a film request
//!   ([`check_film_request`]): the same opening and read-only commands, that request's `1E 57`
//!   (Epson Scan's form, [`crate::film::request_block`]) and the lid's lamp (`1E A2 02`); nothing
//!   that moves;
//! * once [`Guarded::permit_film_path`] has opened the film's path (after its lamp): the moves
//!   of every film capture (the park, the lid unit's origin and its moves back, the white
//!   read's), the calibration's set-ups in the forms captured; once [`Guarded::permit_film_plan`]
//!   has accepted the plan: its reads' `1E 41`, the tone tables (linear), its shading table,
//!   the 07 table and its raw line, and the image's start move then its read move, each once
//!   and in that order;
//! * a Digital ICE scan, once [`Guarded::permit_ice`] has accepted its request
//!   ([`check_ice_request`]): the film's path with the colour pass's `1E 57` (source `04`), its
//!   hole pre-read (`1E 11 01`, the 256-value ramp, the carriage's 131 steps, the read's 512,
//!   the park's 327 from 1155), `1E 88` once with a block of the holes' form; once
//!   [`Guarded::permit_infrared_pass`] has switched to the infrared pass: the infrared `1E 57`
//!   (mode `20`, source `03`), its light (`1E A2 04`, `05`), the lid unit's 705 steps, `1E 87 01`,
//!   the origin read's ramp at T 8064, window line times up to [`ice::IR_MAX_LINE_TIME`], and its
//!   plan only with a start move 35-45 steps after the colour pass's ([`ICE_START_DELTAS`]) and
//!   its columns near the colour read's ([`check_infrared_plan`]).
//!
//! What a scan may ask ([`check_request`], [`check_plan`]): the glass, colour,
//! 100, 200, 300, 400, 600 and 1200 dpi, at 8 bits (WIA) or 16 (Epson Scan,
//! the whole glass at each); an area inside the glass; the line times
//! (so the motor's speeds) each resolution was captured with; a start move
//! of a kind seen; no read further than the furthest captured. Refused, as
//! not captured: 800 (WIA asks 1200), 900, 2400 and 4800 dpi, black and
//! white, Epson Scan's draft mode. Films: only the requests Epson Scan was
//! captured sending ([`FILM_COVERED`], inside [`crate::film::ADAPTER_INCHES`]).
//! Anything else is refused with an error and never reaches the scanner.

use crate::film::{self, Film, FilmPlan, FilmRequest, Pass};
use crate::firmware::{HEADER_LEN, PAYLOAD_LEN};
use crate::ice;
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

/// The film requests a scan may send: resolution across and along, bits, the films, as Epson
/// Scan was captured sending them (captures 16-19 and 72-81, 14 November 2026; 98-101, 15
/// November): its preview of the whole window at 300 dpi, 8 bits; negatives and slides at 600,
/// 1200, 2400 and 4800 dpi (read along at 5400, [`film::READ_ALONG`]), 16 bits; a negative at
/// 2400 at 8 bits too. Digital ICE's second pass (capture 18) is not here.
pub const FILM_COVERED: [(u32, u32, u8, &[Film]); 6] = [
    (300, 300, 8, &[Film::Negative, Film::Positive]),
    (600, 600, 16, &[Film::Negative, Film::Positive]),
    (1200, 1200, 16, &[Film::Negative, Film::Positive]),
    (2400, 2400, 16, &[Film::Negative, Film::Positive]),
    (2400, 2400, 8, &[Film::Negative]),
    (4800, 5400, 16, &[Film::Negative, Film::Positive]),
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

/// Whether a film request may be sent: a resolution, depth and film Epson Scan was captured
/// sending ([`FILM_COVERED`]), an area inside the transparency unit's window.
pub fn check_film_request(r: &FilmRequest) -> Result<()> {
    let refuse = |why: String| Err(Error::InvalidRequest(format!("Epson 4490: {why}")));
    let kind = match r.film {
        Film::Negative => "a negative",
        Film::Positive => "a slide",
    };
    let covered = FILM_COVERED
        .iter()
        .any(|&(d, y, b, f)| d == r.dpi && y == r.ydpi && b == r.bits && f.contains(&r.film));
    if !covered {
        return refuse(format!(
            "{kind} at {} x {} dpi and {} bits was never captured (negatives: 600, 1200, 2400 at \
             16 bits, 2400 at 8, 4800 x 5400 at 16; slides: 600, 1200, 2400, 4800 x 5400 at 16; \
             the preview at 300 and 8 bits)",
            r.dpi, r.ydpi, r.bits
        ));
    }
    let px = |inches: f64, dpi: u32| (inches * f64::from(dpi)).round() as u64;
    let max_w = px(film::ADAPTER_INCHES.0, r.dpi);
    let max_h = px(film::ADAPTER_INCHES.1, r.ydpi);
    let end = |a: u32, b: u32| u64::from(a) + u64::from(b);
    if r.width == 0 || r.height == 0 || end(r.x, r.width) > max_w || end(r.y, r.height) > max_h {
        return refuse(format!(
            "the area {}x{} at {},{} is not inside the film window ({max_w} x {max_h} pixels at \
             {} x {} dpi)",
            r.width, r.height, r.x, r.y, r.dpi, r.ydpi
        ));
    }
    film::request_block(r)?;
    Ok(())
}

/// The Digital ICE scans a scan may send: resolution across and along, bits, the films, as Epson
/// Scan was captured sending them (captures 18 and 108-113, 17 November 2026; 132, 133 and 137,
/// 20 November): negatives and slides at 600, 1200 and 2400 dpi, 16 bits. Epson Scan asked 4800
/// dpi reads 2400 with ICE (110, 113): never above.
pub const ICE_COVERED: [(u32, u32, u8, &[Film]); 3] = [
    (600, 600, 16, &[Film::Negative, Film::Positive]),
    (1200, 1200, 16, &[Film::Negative, Film::Positive]),
    (2400, 2400, 16, &[Film::Negative, Film::Positive]),
];

/// The infrared pass's start move less the colour pass's, as captured (`ice::start_delta`): 39-44
/// in captures 18 and 108-126, 45 by Epson Scan right after a power-on (127, 19 November 2026:
/// the infrared holes read higher then); another one is refused (Matteo's decision: only what a
/// capture showed; our 46 after a power-on, 18 November, was never seen from Epson Scan). Since
/// 21 November 2026 (Matteo's decision) from 35: his app on Linux computed 37 and 38 three times
/// on a negative at 2400 dpi near the strip's bottom (40-45 in all 28 ICE scans captured, Epson
/// Scan's and ours), the infrared holes read a few 4800ths lower; a step is a hundredth of a
/// millimetre, Epson's rule computed it, and the registration measures the shift on the image;
/// two below the lowest seen. The top stays 45: a later start would cut more of an area at the
/// holder's bottom ([`ice_reachable`]).
pub const ICE_START_DELTAS: std::ops::RangeInclusive<i64> = 35..=45;

/// How far the infrared read's first column may lie from the colour read's, 4800ths of an inch:
/// the captures' -53 to -26 (`ice::infrared_plan`), and room for a frame anywhere in the window
/// (the lens's ratio moves it by up to +90 at the window's right edge).
pub const ICE_COLUMN_SHIFT: std::ops::RangeInclusive<i64> = -96..=96;

/// The furthest the lid's unit has gone in a captured read: Epson Scan's Digital ICE on the
/// lowest frame of the right strip, its area drawn as low as Epson Scan lets it go (capture 131,
/// 19 November 2026: 1200 dpi, y 9399, 1784 lines, ending at line 11,183): its infrared read
/// from 2981 after the white read, 18,368 steps and the ramp's 1,024 to the first line, 3,600
/// read. (Before it: 126, the same frame as Epson Scan found it, 25,880; Epson Scan's preview of
/// the whole window, 16 and 98, 24,311.)
pub const MAX_FILM_UNIT_END: u32 = 2981 + 18_368 + 1024 + 3600;

/// Where the lid's unit is after an infrared pass's white read, at most: the infrared light's
/// 1,729 steps, the origin's 1416 less the shortest move back allowed, 64.
const IR_UNIT_AFTER_WHITE_MAX: u32 = ice::IR_UNIT_AT + 1416 - FILM_BACK_MOVES[0] + 64;

/// The most of an area's height a Digital ICE scan cuts from its bottom to keep the lid's unit
/// within [`MAX_FILM_UNIT_END`] ([`ice_reachable`]): a twentieth, a film frame's border below
/// it; an area reaching further down is refused.
const ICE_CUT_MAX: u32 = 20;

/// `r` as a Digital ICE scan can read it: whole, or without the lines at its bottom whose
/// infrared read would take the lid's unit beyond [`MAX_FILM_UNIT_END`], counted for the worst
/// case (the unit as far as it can be after the white read, the start the latest of
/// [`ICE_START_DELTAS`]), so that the infrared pass never stops after the colour one (19 November
/// 2026, Matteo: the lowest frame as the application finds it ends 13 lines below the lowest
/// area Epson Scan lets one draw). With the lines cut; more than a [`ICE_CUT_MAX`]th of the
/// area, and it is refused as [`check_ice_request`] refuses it.
pub fn ice_reachable(r: &FilmRequest) -> Result<(FilmRequest, u32)> {
    let mut cut = *r;
    loop {
        let g = film::geometry(&cut, setup::XCorrection(0))?;
        let ir_lines = cut.height
            + if g.read_dpi == 4800 {
                cut.ydpi / 600
            } else {
                0
            };
        let end = infrared_unit_end(
            IR_UNIT_AFTER_WHITE_MAX,
            g.start_steps + *ICE_START_DELTAS.end() as u32,
            ir_lines * 2400 / cut.ydpi + g.lead,
        );
        if end <= MAX_FILM_UNIT_END {
            break;
        }
        let over = (end - MAX_FILM_UNIT_END)
            .div_ceil((2400 / cut.ydpi).max(1))
            .max(1);
        if (r.height - cut.height + over) * ICE_CUT_MAX > r.height {
            // Too much to cut: refused, with the guard's own reason.
            check_ice_request(r)?;
            return Err(Error::InvalidRequest(format!(
                "Epson 4490: Digital ICE on this area would cut more than a {ICE_CUT_MAX}th of \
                 it at its bottom (the lid's unit's infrared light reads beyond the furthest a \
                 capture showed); choose an area that ends higher in the holder"
            )));
        }
        cut.height -= over;
    }
    check_ice_request(&cut)?;
    Ok((cut, r.height - cut.height))
}

/// Where the lid's unit is after an infrared pass's white read, at least: the infrared light's
/// 1,729 steps, the origin's 1416 less the longest move back allowed, 64.
const IR_UNIT_AFTER_WHITE_MIN: u32 =
    ice::IR_UNIT_AT + 1416 - FILM_BACK_MOVES[FILM_BACK_MOVES.len() - 1] + 64;

/// Where an infrared read takes the lid's unit: from `unit_after_white`, the start move of
/// `start_steps` with its ramp's 1,024 steps, then the read's `read_steps`.
pub fn infrared_unit_end(unit_after_white: u32, start_steps: u32, read_steps: u32) -> u32 {
    unit_after_white + start_steps + film::RAMP_512_STEPS + read_steps
}

/// Whether an infrared read may take the lid's unit to `end`: not beyond
/// [`MAX_FILM_UNIT_END`]. The driver asks it with where the unit is, before the start move.
pub fn check_infrared_reach(end: u32) -> Result<()> {
    if end > MAX_FILM_UNIT_END {
        return Err(Error::InvalidRequest(format!(
            "Epson 4490: Digital ICE on this area would take the lid's unit to step {end}, \
             beyond the furthest a capture showed ({MAX_FILM_UNIT_END}): its infrared light \
             reads 1,741 steps further on than the colours; choose an area that ends higher in \
             the holder"
        )));
    }
    Ok(())
}

/// Whether a Digital ICE scan of `r` may be sent: a film request ([`check_film_request`]) of a
/// resolution, depth and film captured with ICE ([`ICE_COVERED`]), whose infrared read could keep
/// the lid's unit within [`MAX_FILM_UNIT_END`] (from the least far the unit may be, its start the
/// colour one's and at least the [`ICE_START_DELTAS`]' smallest, its read the colour one's less
/// the colours' offsets): what is refused here is refused before anything moves; the driver
/// checks again where the unit really is ([`check_infrared_reach`]).
pub fn check_ice_request(r: &FilmRequest) -> Result<()> {
    check_film_request(r)?;
    let covered = ICE_COVERED
        .iter()
        .any(|&(d, y, b, f)| d == r.dpi && y == r.ydpi && b == r.bits && f.contains(&r.film));
    if !covered {
        let kind = match r.film {
            Film::Negative => "a negative",
            Film::Positive => "a slide",
        };
        return Err(Error::InvalidRequest(format!(
            "Epson 4490: Digital ICE on {kind} at {} dpi and {} bits was never captured \
             (negatives and slides at 600, 1200 and 2400 dpi, 16 bits)",
            r.dpi, r.bits
        )));
    }
    let g = film::geometry(r, setup::XCorrection(0))?;
    let ir_lines = r.height + if g.read_dpi == 4800 { r.ydpi / 600 } else { 0 };
    check_infrared_reach(infrared_unit_end(
        IR_UNIT_AFTER_WHITE_MIN,
        g.start_steps + *ICE_START_DELTAS.start() as u32,
        ir_lines * 2400 / r.ydpi + g.lead,
    ))?;
    Ok(())
}

/// Whether an infrared pass's plan may follow its colour pass's: the same request and resolution,
/// a start move [`ICE_START_DELTAS`] after the colour one's, the first column within
/// [`ICE_COLUMN_SHIFT`] of the colour read's, as many pixels or up to 256 more.
pub fn check_infrared_plan(colour: &FilmPlan, ir: &FilmPlan) -> Result<()> {
    let refuse = |why: String| {
        Err(Error::InvalidRequest(format!(
            "Epson 4490, Digital ICE: {why}"
        )))
    };
    let (c, i) = (&colour.geometry, &ir.geometry);
    if colour.request != ir.request || c.base != i.base || c.read_dpi != i.read_dpi {
        return refuse("an infrared plan for another request".into());
    }
    // Where each read's first line is: its start move and its lead (capture 127: 269 + 128 and
    // 410 + 32, 45 apart).
    let delta = i64::from(i.start_steps + i.lead) - i64::from(c.start_steps + c.lead);
    if !ICE_START_DELTAS.contains(&delta) {
        return refuse(format!(
            "the infrared read would start {delta} steps after the colour one, never captured \
             ({}-{})",
            ICE_START_DELTAS.start(),
            ICE_START_DELTAS.end()
        ));
    }
    let u = i64::from(4800 / c.base);
    let shift = (i64::from(film::logical_of(i.columns.0, i.words, i.base))
        - i64::from(film::logical_of(c.columns.0, c.words, c.base)))
        * u;
    if !ICE_COLUMN_SHIFT.contains(&shift) || i.pixels < c.pixels || i.pixels > c.pixels + 256 {
        return refuse(format!(
            "the infrared read's columns ({} pixels, {shift} 4800ths from the colour read's) are \
             far from the colour read's ({} pixels)",
            i.pixels, c.pixels
        ));
    }
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
    /// One of a film's `1E 01` blocks: the moves `1E 05` starts (first), the reads' (second).
    FilmMove(Vec<Vec<u8>>, Vec<Vec<u8>>),
    /// A film's `1E 21`: a line time a multiple of 384 µs from [`FILM_LINE_TIMES`]'s start up to
    /// this.
    FilmBlock21(u32),
    /// The hole pre-read's `1E 41` (Digital ICE), for an x correction in [`X_CORRECTION_RANGE`],
    /// or the lid unit's origin read's.
    PreRead41,
    /// `1E 88`'s block: the holes ([`ice::Holes`]), each value within a hole's image.
    Block88,
    /// A film's `1E 84` header.
    FilmTable,
    /// A one-value ramp no faster than this (µs / 16).
    MinWord(u16),
}

/// The line times a film's `1E 21` may carry, µs: from the origin read's 6144 to the window
/// reads' 27,264 (captures 17, 72-81, 98-101: a slide at 4800 dpi, 101); every one a multiple
/// of 384.
pub const FILM_LINE_TIMES: std::ops::RangeInclusive<u32> = 6144..=27_264;

/// The moves back the lid unit may make after its origin read (`1E 8A` less 1024): those Epson
/// Scan was seen to make (228, 240, 252) and one line of the read (12 steps) either side, 216
/// and 264 (Matteo's decision, 14 November 2026: the light's edge moved by a line between our
/// scans, 87 asked 264; the unit then stops at 1200 or 1152, inside the travel it has just
/// made from 0 to 2440); and 204, a line short of 216 (Matteo's decision, 15 November 2026: the
/// lid's lamp, brighter as it warms, moved the edge by a line between our scans 70 and 90 s
/// after it was lit, capture 104; the unit then stops at 1212). Since 21 November 2026 (Matteo's
/// decision) every line of the read from 180 to 312: the edge moves by whole lines between
/// scans, both lights alike (capture 146: our colour passes 240 then 264, their infrared 228
/// then 252, nine minutes apart; Matteo's app the same day asked 276, an edge one line further),
/// the rule follows it by design and Epson Scan moves what the rule says in every capture (145
/// too); two lines past the 204-276 seen either side, the unit stopping at 2128-2260, inside the
/// travel it has just made. A computed origin outside them, a read gone wrong, is refused.
pub const FILM_BACK_MOVES: [u32; 12] = [180, 192, 204, 216, 228, 240, 252, 264, 276, 288, 300, 312];

/// The film path's moves before its plan (every film capture): the park from home (1994 less
/// the 512 ramp's 1024), the lid unit back from where the scanner put it when it lit its lamp
/// again (1078, capture 72), the 8 steps before the origin read; then the moves back.
fn film_moves() -> Vec<Vec<u8>> {
    let mut m = vec![
        setup::move_block(film::PARK - film::RAMP_512_STEPS, 0, 0x1FF, 0x10).to_vec(),
        setup::move_block(1078 - film::RAMP_512_STEPS, 0, 0x1FF, 0x30).to_vec(),
        setup::move_block(8, 0, 0, 0x18).to_vec(),
    ];
    m.extend(
        FILM_BACK_MOVES
            .iter()
            .map(|&n| setup::move_block(n, 0, 0x1FF, 0x30).to_vec()),
    );
    m
}

/// The origin read's move (1920 steps, lead 120: every film capture).
pub fn film_origin_read_move() -> [u8; 12] {
    setup::move_block(1920, 120, 0xFF, 0x12)
}

/// `1E 8A`'s block for a move back of `n` steps: n + 1024, u32.
pub fn film_origin_block(n: u32) -> [u8; 4] {
    (n + film::RAMP_512_STEPS).to_le_bytes()
}

/// Digital ICE's moves started with `1E 05` (captures 18, 108-113): the carriage before the
/// hole pre-read (131 steps, the 256-value ramp), the park after it (327 from 1155); in the
/// infrared pass the lid unit's 705 steps (the film ramp).
fn ice_moves(infrared: bool) -> Vec<Vec<u8>> {
    let mut m = vec![
        setup::move_block(ice::PRE_MOVE, 0, 0xFF, 0x10).to_vec(),
        setup::move_block(ice::PRE_PARK, 0, 0xFF, 0x10).to_vec(),
    ];
    if infrared {
        m.push(setup::move_block(ice::IR_UNIT_MOVE, 0, 0x1FF, 0x10).to_vec());
    }
    m
}

/// The hole pre-read's own move: 512 steps, lead 32.
pub fn ice_pre_read_move() -> [u8; 12] {
    setup::move_block(ice::PRE_READ_STEPS, 32, 0, 0x18)
}

/// A ramp's bytes.
fn ramp_le(r: &[u16]) -> Vec<u8> {
    r.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// What a scan was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    /// The glass.
    Glass(Request),
    /// A film in the lid's transparency unit.
    Film(FilmRequest),
}

/// What a scan may send.
#[derive(Debug, Clone)]
struct Permit {
    asked: Asked,
    e57: [u8; 42],
    plan: Option<Plan>,
    /// A film's path is open (after its lamp): its moves before the plan.
    film_path: bool,
    /// A film's plan, once `1E 87` has given the correction.
    film_plan: Option<FilmPlan>,
    /// The film plan's start move went out: not again, and its read move may follow.
    film_started: bool,
    /// The film plan's read move went out: not again.
    film_read: bool,
    /// The line time of the film's last `1E 21`: the image read's lead must be the one it asks
    /// ([`film::lead_for`]).
    film_t: Option<u32>,
    /// A Digital ICE scan: the pass under way and what the colour pass set.
    ice: Option<IceState>,
}

/// What a Digital ICE scan has done.
#[derive(Debug, Clone)]
struct IceState {
    /// The pass under way: [`Pass::IceColour`], then [`Pass::Infrared`].
    pass: Pass,
    /// The infrared pass's `1E 57`.
    e57_infrared: [u8; 42],
    /// The colour pass's plan, once the infrared pass is under way.
    colour: Option<FilmPlan>,
    /// `1E 88` went out.
    written88: bool,
}

impl Permit {
    fn new(asked: Asked, e57: [u8; 42]) -> Self {
        Self {
            asked,
            e57,
            plan: None,
            film_path: false,
            film_plan: None,
            film_started: false,
            film_read: false,
            film_t: None,
            ice: None,
        }
    }

    /// The Digital ICE pass under way, if any.
    fn ice_pass(&self) -> Option<Pass> {
        self.ice.as_ref().map(|i| i.pass)
    }
}

/// A film plan's start move (with the 512-value ramp, the carriage) and its read move.
fn film_image_moves(p: &FilmPlan) -> ([u8; 12], [u8; 12]) {
    let g = &p.geometry;
    (
        setup::move_block(g.start_steps, 0, 0x1FF, 0x10),
        setup::move_block(g.read_steps, g.lead as u16, 0, 0x08),
    )
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
        self.permit = Some(Permit::new(
            Asked::Glass(*request),
            setup::request_block(request)?,
        ));
        self.expect = None;
        self.start_armed = false;
        self.start_done = false;
        Ok(())
    }

    /// Lets out a film's opening for `request`, if [`check_film_request`] accepts it: the
    /// commands of the glass's opening, the film's `1E 57` and the lid's lamp (`1E A2 02`).
    /// Nothing moves until [`Guarded::permit_film_path`].
    pub fn permit_film(&mut self, request: &FilmRequest) -> Result<()> {
        check_film_request(request)?;
        self.permit = Some(Permit::new(
            Asked::Film(*request),
            film::request_block(request)?,
        ));
        self.expect = None;
        self.start_armed = false;
        self.start_done = false;
        Ok(())
    }

    /// Lets out a Digital ICE scan of `request`, if [`check_ice_request`] accepts it: the film's
    /// opening with the colour pass's `1E 57` ([`ice::request_block`]), then its path as a film's
    /// ([`Guarded::permit_film_path`]) with the hole pre-read and `1E 88`.
    pub fn permit_ice(&mut self, request: &FilmRequest) -> Result<()> {
        check_ice_request(request)?;
        let mut p = Permit::new(
            Asked::Film(*request),
            ice::request_block(request, Pass::IceColour)?,
        );
        p.ice = Some(IceState {
            pass: Pass::IceColour,
            e57_infrared: ice::request_block(request, Pass::Infrared)?,
            colour: None,
            written88: false,
        });
        self.permit = Some(p);
        self.expect = None;
        self.start_armed = false;
        self.start_done = false;
        Ok(())
    }

    /// Switches a Digital ICE scan to its infrared pass, once its colour pass's plan was let out:
    /// the infrared `1E 57`, its light, its moves; its own plan next
    /// ([`Guarded::permit_film_plan`]).
    pub fn permit_infrared_pass(&mut self) -> Result<()> {
        let refuse = || {
            Err(Error::Other(
                "Epson 4490: an infrared pass without its colour pass".into(),
            ))
        };
        let Some(p) = self.permit.as_mut() else {
            return refuse();
        };
        let colour_done = p.ice.as_ref().is_some_and(|i| i.pass == Pass::IceColour)
            && p.film_plan
                .as_ref()
                .is_some_and(|f| f.pass == Pass::IceColour);
        if !colour_done {
            return refuse();
        }
        let plan = p.film_plan.take();
        let state = p.ice.as_mut().expect("checked");
        state.pass = Pass::Infrared;
        state.colour = plan;
        p.e57 = state.e57_infrared;
        p.film_started = false;
        p.film_read = false;
        p.film_t = None;
        self.expect = None;
        self.start_armed = false;
        Ok(())
    }

    /// Lets out the blocks of `plan`, computed for the request already
    /// permitted, if [`check_plan`] accepts it.
    pub fn permit_plan(&mut self, plan: &Plan) -> Result<()> {
        let permit = self.permit.as_mut().ok_or_else(|| {
            Error::Other("Epson 4490: a plan without its request permitted".into())
        })?;
        match permit.asked {
            Asked::Glass(r) if r == plan.request => {}
            Asked::Glass(_) => {
                return Err(Error::Other(
                    "Epson 4490: the plan is not for the request permitted".into(),
                ))
            }
            Asked::Film(_) => {
                return Err(Error::Other(
                    "Epson 4490: a glass plan for a film: the film path's moves are not in the                      driver yet"
                        .into(),
                ))
            }
        }
        check_plan(plan)?;
        permit.plan = Some(plan.clone());
        Ok(())
    }

    /// Opens a permitted film's path (after its lamp): the park, the lid unit's moves and its
    /// origin read, `1E 8A`, the calibration's set-ups, all in the forms captured.
    pub fn permit_film_path(&mut self) -> Result<()> {
        match self.permit.as_mut() {
            Some(p) if matches!(p.asked, Asked::Film(_)) => {
                p.film_path = true;
                Ok(())
            }
            _ => Err(Error::Other(
                "Epson 4490: a film's path without its film permitted".into(),
            )),
        }
    }

    /// Lets out a film's `plan` (its reads' `1E 41`, the start and read moves, the shading),
    /// for the film permitted.
    pub fn permit_film_plan(&mut self, plan: &FilmPlan) -> Result<()> {
        match self.permit.as_mut() {
            Some(p) if p.asked == Asked::Film(plan.request) && p.film_path => {
                check_film_request(&plan.request)?;
                let want = p.ice_pass().unwrap_or(Pass::Film);
                if plan.pass != want {
                    return Err(Error::Other(format!(
                        "Epson 4490: a film plan for {:?} in {want:?}",
                        plan.pass
                    )));
                }
                if let Some(colour) = p.ice.as_ref().and_then(|i| i.colour.as_ref()) {
                    check_infrared_plan(colour, plan)?;
                }
                p.film_plan = Some(plan.clone());
                Ok(())
            }
            _ => Err(Error::Other(
                "Epson 4490: a film plan not for the film permitted".into(),
            )),
        }
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
        let film = matches!(permit.asked, Asked::Film(_));
        // The glass's lamp in a glass scan, the lid's for a film (Epson Scan, captures 72-81).
        let lamps: &[&[u8]] = if permit.ice_pass() == Some(Pass::Infrared) {
            // The lid unit's infrared light, and its dark read's (captures 18, 108-113).
            &[&[0x04], &[0x05]]
        } else if film && permit.film_path {
            &[&[0x02], &[0x03]]
        } else if film {
            &[&[0x02]]
        } else {
            &[&[0x00], &[0x01]]
        };
        let request_level = match data {
            [0x1B, 0x40] | [0x1B, 0x13] => Some(None),
            [0x1E, 0x53 | 0xA1 | 0x92 | 0x8C | 0x04 | 0x8B] => Some(None),
            [0x1E, 0x87] if permit.ice_pass() == Some(Pass::Infrared) => {
                Some(one(&[&[0x00], &[0x01]]))
            }
            [0x1E, 0x87] => Some(one(&[&[0x00]])),
            [0x1E, 0xA2] => Some(one(lamps)),
            // The lid's lamp switched back to the glass's (WIA's driver, capture 25), and Epson
            // Scan's opening (captures 72, 73); no block.
            [0x1E, 0x77] => Some(None),
            [0x1E, 0xB3] => Some(Some(Expect::GammaChannel)),
            [0x1E, 0x57] => Some(one(&[&permit.e57])),
            [0x1E, 0x31] => Some(Some(Expect::Len(12))),
            _ => None,
        };
        if let Some(next) = request_level {
            return idle(next);
        }
        if film && permit.film_path {
            return self
                .film_command(permit, data)
                .map(|next| (Stage::Idle, next));
        }
        let Some(p) = &permit.plan else {
            if film {
                return Err(refusal(
                    data,
                    "a film moves nothing yet: the film path is not in the driver",
                ));
            }
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

    /// A command of a film's path: what may follow it.
    fn film_command(&self, permit: &Permit, data: &[u8]) -> Result<Option<Expect>> {
        let p = permit.film_plan.as_ref();
        let plan_only = |what: &str| Err(refusal(data, &format!("{what} before the film's plan")));
        let pass = permit.ice_pass();
        Ok(match data {
            [0x1E, 0x88]
                if pass == Some(Pass::IceColour)
                    && permit.ice.as_ref().is_some_and(|i| !i.written88) =>
            {
                Some(Expect::Block88)
            }
            [0x1E, 0x11] if pass.is_some() => one(&[[0x01], [0x03], [0x08], [0x09]]),
            [0x1E, 0x41] if pass.is_some() && p.is_none() => Some(Expect::PreRead41),
            [0x1E, 0x25] => one(&[[0x00], [0x02], [0x03]]),
            [0x1E, 0x84] => Some(Expect::FilmTable),
            [0x1E, 0x01] => {
                let mut moves = film_moves();
                let mut reads = vec![
                    film_origin_read_move().to_vec(),
                    setup::move_block(64, 32, 0, 0x18).to_vec(),
                ];
                if pass.is_some() {
                    moves.extend(ice_moves(pass == Some(Pass::Infrared)));
                    reads.push(ice_pre_read_move().to_vec());
                }
                // The image's start move once, then its read move once.
                if let Some(p) = p {
                    let (start, read) = film_image_moves(p);
                    if !permit.film_started {
                        moves.push(start.to_vec());
                    } else if !permit.film_read {
                        reads.push(read.to_vec());
                    }
                }
                Some(Expect::FilmMove(moves, reads))
            }
            [0x1E, 0x05] if self.start_armed => None,
            [0x1E, 0x8A] => Some(Expect::OneOf(
                FILM_BACK_MOVES
                    .iter()
                    .map(|&n| film_origin_block(n).to_vec())
                    .collect(),
            )),
            [0x1E, 0x11] => one(&[[0x03], [0x08], [0x09]]),
            [0x1E, 0x21] => Some(Expect::FilmBlock21(if pass == Some(Pass::Infrared) {
                ice::IR_MAX_LINE_TIME
            } else {
                *FILM_LINE_TIMES.end()
            })),
            [0x1E, 0x41] if p.is_some() => Some(Expect::Block41),
            [0x1E, 0x41] => one(&[film::origin_e41()]),
            [0x1E, 0x42] => Some(Expect::Len(18)),
            [0x1E, 0x43] => one(&[
                setup::GAIN_CAL,
                film::E43_READS,
                film::E43_IMAGE,
                film::E43_SLIDE,
            ]),
            [0x1E, 0x47] => None,
            // The 07 table read back, or (`1E 8B` answering 15) its raw line read: as on the
            // glass at 1200 dpi (the latter never captured for films).
            [0x1E, 0x83] if p.is_some() => one(&[READ_83, table07::READ_RAW]),
            [0x1E, 0x83] => return plan_only("1E 83"),
            [0x1E, 0x18] => None,
            _ => return Err(refusal(data, "not in a film's path")),
        })
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
            Expect::Block41 if self.permit.as_ref().is_some_and(|p| p.film_plan.is_some()) => {
                let fp = self
                    .permit
                    .as_ref()
                    .and_then(|p| p.film_plan.as_ref())
                    .expect("film plan");
                let mut image = fp.e41_image;
                if data.len() == 22 {
                    image[15] = data[15];
                }
                if [
                    fp.e41_origin,
                    fp.e41_black,
                    fp.e41_window,
                    fp.e41_white,
                    fp.e41_dark,
                    image,
                ]
                .iter()
                .any(|b| b[..] == *data)
                {
                    Ok(None)
                } else {
                    bad("a film's 1E 41")
                }
            }
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
            Expect::FilmMove(moves, reads) if moves.iter().chain(reads).any(|b| b == data) => {
                // The image's read: the lead its line time asks, as Epson's driver gives it.
                if let Some(p) = self.permit.as_ref() {
                    if let (Some(plan), Some(t)) = (p.film_plan.as_ref(), p.film_t) {
                        let lead = film::lead_for(plan.request.ydpi, t);
                        if film_image_moves(plan).1[..] == *data && plan.geometry.lead != lead {
                            return bad(&format!(
                                "a film read's lead of {} steps where its line time {t} µs asks {lead}",
                                plan.geometry.lead
                            ));
                        }
                    }
                }
                Ok(None)
            }
            Expect::FilmMove(..) => bad("a film's move"),
            Expect::FilmBlock21(max) => {
                let t = match data {
                    [a, b, c, ..] => u32::from_le_bytes([*a, *b, *c, 0]),
                    _ => 0,
                };
                if data.len() == 26
                    && t % 384 == 0
                    && (*FILM_LINE_TIMES.start()..=*max).contains(&t)
                {
                    Ok(None)
                } else {
                    bad("a film's 1E 21: its line time")
                }
            }
            Expect::PreRead41 => {
                let c0 = match data {
                    [a, b, ..] if data.len() == 22 => u32::from(u16::from_le_bytes([*a, *b])),
                    _ => return bad("the hole pre-read's 1E 41"),
                };
                let lo = ice::pre_c0(setup::XCorrection(*X_CORRECTION_RANGE.start()));
                let hi = ice::pre_c0(setup::XCorrection(*X_CORRECTION_RANGE.end()));
                if ((lo..=hi).contains(&c0) && ice::pre_e41_at(c0)[..] == *data)
                    || film::origin_e41()[..] == *data
                {
                    Ok(None)
                } else {
                    bad("the hole pre-read's 1E 41")
                }
            }
            Expect::Block88 => match ice::Holes::from_block88(data) {
                // Each hole's column and width within its image (692 pixels, in halves), its
                // line and height within the read's 480 lines.
                Some(h)
                    if (0..2).all(|k| {
                        let v = &h.values[4 * k..4 * k + 4];
                        u32::from(v[0]) < 2 * ice::HOLE_W as u32
                            && u32::from(v[1]) < 2 * ice::PRE_READ.1
                            && u32::from(v[2]) < 2 * ice::HOLE_W as u32
                            && u32::from(v[3]) < 2 * ice::PRE_READ.1
                    }) =>
                {
                    Ok(None)
                }
                _ => bad("1E 88's holes"),
            },
            Expect::MinWord(min) => match data {
                [lo, hi] if u16::from_le_bytes([*lo, *hi]) >= *min => Ok(None),
                _ => bad("a one-value ramp faster than any captured"),
            },
            Expect::FilmTable => {
                let fp = self.permit.as_ref().and_then(|p| p.film_plan.as_ref());
                let pass = self.permit.as_ref().and_then(Permit::ice_pass);
                if pass.is_some() && setup::ramp_header(256)[..] == *data {
                    return Ok(Some(Expect::OneOf(vec![ramp_le(&setup::fixed_ramp(256))])));
                }
                if setup::ramp_header(512)[..] == *data {
                    // The origin read's ramp: at T / 12 (6144 on films, 8064 in the infrared
                    // pass).
                    let origin = if pass == Some(Pass::Infrared) {
                        ice::IR_ORIGIN_T / 12
                    } else {
                        512
                    };
                    let ramps = [setup::film_ramp(), setup::ramp(origin, setup::Mode::Medium)];
                    return Ok(Some(Expect::OneOf(
                        ramps
                            .iter()
                            .map(|r| r.iter().flat_map(|v| v.to_le_bytes()).collect())
                            .collect(),
                    )));
                }
                if setup::ramp_header(1)[..] == *data {
                    return Ok(Some(Expect::MinWord(156)));
                }
                if let Some(fp) = fp {
                    if tables::TONE_HEADERS.iter().any(|h| h[..] == *data) {
                        return Ok(Some(Expect::OneOf(vec![Curves::Linear.tone()])));
                    }
                    if setup::shading_header(fp.geometry.column_count())[..] == *data {
                        return Ok(Some(Expect::Len(fp.geometry.column_count() as usize * 6)));
                    }
                    if table07::HEADER[..] == *data {
                        return Ok(Some(Expect::Len(table07::LEN)));
                    }
                }
                bad("a film's 1E 84 header")
            }
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
        let film_move = matches!(&self.expect, Some(Expect::FilmMove(moves, _)) if moves.iter().any(|b| b == data));
        self.start_armed =
            (matches!(self.expect, Some(Expect::Move(_))) && start_block) || film_move;
        let film = self
            .permit
            .as_ref()
            .is_some_and(|p| matches!(p.asked, Asked::Film(_)));
        if was_command && data == [0x1E, 0x05] && !film {
            self.start_done = true;
        }
        if matches!(self.expect, Some(Expect::FilmMove(..))) {
            if let Some(p) = self.permit.as_mut() {
                if let Some((start, read)) = p.film_plan.as_ref().map(film_image_moves) {
                    p.film_started |= start[..] == *data;
                    p.film_read |= p.film_started && read[..] == *data;
                }
            }
        }
        if was_command && data.len() == 2 && matches!(data[0], 0x1B | 0x1E) && data != GET_STATE {
            self.last_command = Some([data[0], data[1]]);
        }
        if matches!(self.expect, Some(Expect::FilmBlock21(_))) && data.len() == 26 {
            if let Some(p) = self.permit.as_mut() {
                p.film_t = Some(u32::from_le_bytes([data[0], data[1], data[2], 0]));
            }
        }
        if matches!(self.expect, Some(Expect::Block88)) {
            if let Some(i) = self.permit.as_mut().and_then(|p| p.ice.as_mut()) {
                i.written88 = true;
            }
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

    /// A film request like one captured, with some fields changed.
    fn film_like(i: usize, change: impl FnOnce(&mut FilmRequest)) -> FilmRequest {
        let mut r = crate::film::tests::CAPTURED[i].1;
        change(&mut r);
        r
    }

    #[test]
    fn every_captured_film_request_is_accepted() {
        for (_, r) in crate::film::tests::CAPTURED {
            check_film_request(&r).unwrap_or_else(|e| panic!("{r:?}: {e}"));
        }
    }

    #[test]
    fn film_requests_never_captured_are_refused() {
        let refused = [
            // A slide at 8 bits (only negatives were scanned so, at 2400).
            film_like(6, |r| {
                r.film = Film::Positive;
                r.bits = 8
            }),
            // 1200 dpi at 8 bits.
            film_like(3, |r| r.bits = 8),
            // 4800 read along at 4800 (Epson Scan read 5400).
            film_like(4, |r| r.ydpi = 4800),
            // 2400 read along at 3200, as on the glass.
            film_like(5, |r| r.ydpi = 3200),
            // A slide at 2400 dpi, 8 bits.
            film_like(8, |r| r.bits = 8),
            // The preview at 16 bits, and at 200 dpi.
            film_like(0, |r| r.bits = 16),
            film_like(0, |r| {
                r.dpi = 200;
                r.ydpi = 200;
                r.width = 540;
                r.height = 1866;
            }),
            // Past the window's right edge and its bottom, and empty.
            film_like(0, |r| r.x = 1),
            film_like(6, |r| r.y = 22_392 - r.height + 1),
            film_like(6, |r| r.width = 0),
            film_like(6, |r| r.x = u32::MAX - 10),
        ];
        for r in refused {
            assert!(check_film_request(&r).is_err(), "{r:?}");
        }
        // The window's last line at 2400 dpi (9.33 in) is inside.
        check_film_request(&film_like(6, |r| r.y = 22_392 - r.height)).unwrap();
    }

    #[test]
    fn a_films_opening_goes_out_and_nothing_moves() {
        let r = crate::film::tests::CAPTURED[6].1;
        let mut t = guarded(&[]);
        // No film permitted: its 1E 57 and the lid's lamp are refused.
        t.bulk_out(&[0x1E, 0x57]).unwrap_err();
        t.permit_film(&r).unwrap();
        t.bulk_out(&[0x1B, 0x40]).unwrap();
        t.bulk_out(&[0x1E, 0x77]).unwrap();
        t.bulk_out(&[0x1E, 0x57]).unwrap();
        // The glass's form of the request is not this film's.
        let glass = setup::request_block(&Request::new(r.dpi, r.x, r.y, r.width, r.height, r.bits))
            .unwrap();
        assert!(t.bulk_out(&glass).is_err());
        t.bulk_out(&[0x1E, 0x57]).unwrap();
        t.bulk_out(&film::request_block(&r).unwrap()).unwrap();
        // The lid's lamp, not the glass's, and not off.
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        assert!(t.bulk_out(&[0x01]).is_err());
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        assert!(t.bulk_out(&[0x00]).is_err());
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        t.bulk_out(&[0x02]).unwrap();
        let sent = t.inner().writes.len();
        // Nothing of the film path: what moves, its tables, the lid's other lamp, 1E 8A.
        for m in [
            &[0x1E, 0x25][..],
            &[0x1E, 0x01],
            &[0x1E, 0x05],
            &[0x1E, 0x47],
            &[0x1E, 0x84],
            &[0x1E, 0x8A],
            &[0x1E, 0x11],
            &[0x1E, 0x41],
        ] {
            let e = t.bulk_out(m).unwrap_err().to_string();
            assert!(e.contains("a film moves nothing yet"), "{e}");
        }
        assert_eq!(t.inner().writes.len(), sent);
        // No glass plan for it either.
        let p = plan(300, 472, 709, 591, 591, 8).unwrap();
        assert!(t.permit_plan(&p).is_err());
        // Once over, not even the lid's lamp.
        t.end_scan();
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        assert!(t.bulk_out(&[0x02]).is_err());
    }

    /// Digital ICE only as captured: its requests (negatives at 1200 and 2400 dpi, slides at
    /// 2400, 16 bits), its commands only in an ICE scan and in their pass (`1E 88` once in the
    /// colour pass, the infrared light and `1E 87 01` in the infrared one), the infrared plan
    /// only with the start and columns captured.
    #[test]
    fn digital_ice_goes_out_only_as_captured() {
        use crate::ice::{self, tests::ICE};
        let (neg, neg1200, slide) = (ICE[1].1, ICE[2].1, ICE[4].1);
        for r in [neg, neg1200, slide] {
            check_ice_request(&r).unwrap_or_else(|e| panic!("{r:?}: {e}"));
        }
        // Slides at 1200 and both films at 600 (Epson Scan's 132, 133, 137) let out too.
        let mut slide1200 = neg1200;
        slide1200.film = Film::Positive;
        let mut neg600 = neg1200;
        (neg600.dpi, neg600.ydpi, neg600.width, neg600.height) = (600, 600, 576, 850);
        let mut slide600 = neg600;
        slide600.film = Film::Positive;
        for r in [slide1200, neg600, slide600] {
            check_ice_request(&r).unwrap_or_else(|e| panic!("{r:?}: {e}"));
        }
        // Not at 8 bits, nor at 4800 (read along at 5400: Epson Scan reads 2400 there).
        let mut neg8 = neg;
        neg8.bits = 8;
        let mut neg4800 = neg;
        (neg4800.dpi, neg4800.ydpi) = (4800, 5400);
        for r in [neg8, neg4800] {
            assert!(check_ice_request(&r).is_err(), "{r:?}");
        }
        // The whole window and a frame at its bottom: their infrared read would take the lid's
        // unit past the furthest captured (131: Epson Scan stops an area 13 lines above the
        // window's bottom at 1200 dpi), so their bottom lines are cut, a twentieth at most; a
        // frame higher up is read whole (Matteo's app scan of a strip, 18 November 2026, found
        // the limit missing; his lowest frame, 19 November, found it too strict).
        for dpi in [1200, 2400] {
            let tall = (film::AREA_INCHES.1 * f64::from(dpi)) as u32;
            let whole = film::for_area(dpi, Film::Negative, (0, 0, dpi * 27 / 10, tall), 16);
            let frame = |y: u32| film::for_area(dpi, Film::Negative, (0, y, dpi * 3 / 2, dpi), 16);
            for r in [whole, frame(tall - dpi)] {
                let (_, lines) = ice_reachable(&r).unwrap_or_else(|e| panic!("{dpi}: {e}"));
                assert!(
                    lines > 0 && lines * ICE_CUT_MAX <= r.height,
                    "{dpi}: {lines}"
                );
            }
            let high = frame(dpi * 6);
            check_ice_request(&high).unwrap_or_else(|e| panic!("{dpi}: {e}"));
            assert_eq!(ice_reachable(&high).unwrap(), (high, 0));
        }
        // Capture 131's own infrared read (the lowest area Epson Scan lets one draw) reaches the
        // limit exactly; 126's (the same frame as Epson Scan found it) is let out whole.
        assert_eq!(infrared_unit_end(2981, 18_368, 3600), MAX_FILM_UNIT_END);
        let frame = |x, y, width, height| FilmRequest {
            dpi: 1200,
            ydpi: 1200,
            x,
            y,
            width,
            height,
            bits: 16,
            film: Film::Negative,
        };
        let lowest = frame(1876, 9446, 1212, 1691);
        check_ice_request(&lowest).unwrap();
        assert_eq!(ice_reachable(&lowest).unwrap(), (lowest, 0));
        assert!(check_infrared_reach(MAX_FILM_UNIT_END + 1).is_err());
        // The same frame as the application found it (19 November 2026, refused then): 13 lines
        // lower than 131's; a few lines cut at its bottom, counted for the worst case (the
        // shortest move back let out, 180: 38 and 25 lines, 12 more than with 204), and let
        // out; so is 131's own area, by fewer.
        for (r, most) in [
            (frame(1888, 9420, 1228, 1776), 40),
            (frame(1888, 9399, 1188, 1784), 30),
        ] {
            let (cut, lines) = ice_reachable(&r).unwrap_or_else(|e| panic!("{r:?}: {e}"));
            assert!(lines > 0 && lines <= most, "{r:?}: {lines}");
            assert_eq!((cut.y, cut.height + lines), (r.y, r.height));
            check_ice_request(&cut).unwrap();
            let g = film::geometry(&cut, setup::XCorrection(0)).unwrap();
            let worst = infrared_unit_end(
                IR_UNIT_AFTER_WHITE_MAX,
                g.start_steps + *ICE_START_DELTAS.end() as u32,
                cut.height * 2 + g.lead,
            );
            assert!(worst <= MAX_FILM_UNIT_END, "{worst}");
            // A line more and it would not be.
            let mut more = cut;
            more.height += 1;
            let g = film::geometry(&more, setup::XCorrection(0)).unwrap();
            let worse = infrared_unit_end(
                IR_UNIT_AFTER_WHITE_MAX,
                g.start_steps + *ICE_START_DELTAS.end() as u32,
                more.height * 2 + g.lead,
            );
            assert!(worse > MAX_FILM_UNIT_END, "{worse}");
        }
        // A short area at the window's bottom would lose more than a twentieth: refused.
        let e = ice_reachable(&frame(1888, 11_196 - 200, 1228, 200))
            .unwrap_err()
            .to_string();
        assert!(e.contains("lid's unit"), "{e}");
        // A film scan without ICE: none of ICE's commands.
        let mut t = guarded(&[]);
        t.permit_film(&neg).unwrap();
        t.permit_film_path().unwrap();
        assert!(t.bulk_out(&[0x1E, 0x88]).is_err());
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        assert!(t.bulk_out(&[ice::LAMP_INFRARED]).is_err());
        t.bulk_out(&[0x1E, 0x87]).unwrap();
        assert!(t.bulk_out(&[0x01]).is_err());
        t.bulk_out(&[0x1E, 0x11]).unwrap();
        assert!(t.bulk_out(&[0x01]).is_err());
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        assert!(t
            .bulk_out(&setup::move_block(ice::PRE_MOVE, 0, 0xFF, 0x10))
            .is_err());
        // An ICE scan: its colour 1E 57, not a film's; the pre-read's moves and 1E 41; 1E 88
        // once and of the holes' form; not the infrared light.
        let mut t = guarded(&[]);
        t.permit_ice(&neg).unwrap();
        t.bulk_out(&[0x1E, 0x57]).unwrap();
        assert!(t.bulk_out(&film::request_block(&neg).unwrap()).is_err());
        t.bulk_out(&[0x1E, 0x57]).unwrap();
        t.bulk_out(&ice::request_block(&neg, Pass::IceColour).unwrap())
            .unwrap();
        t.permit_film_path().unwrap();
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        t.bulk_out(&setup::move_block(ice::PRE_MOVE, 0, 0xFF, 0x10))
            .unwrap();
        t.bulk_out(&[0x1E, 0x05]).unwrap();
        t.bulk_out(&[0x1E, 0x41]).unwrap();
        t.bulk_out(&ice::pre_e41(setup::XCorrection(-133))).unwrap();
        t.bulk_out(&[0x1E, 0x41]).unwrap();
        assert!(t.bulk_out(&ice::pre_e41_at(ice::PRE_C0 + 400)).is_err());
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        t.bulk_out(&ice_pre_read_move()).unwrap();
        t.bulk_out(&[0x1E, 0x88]).unwrap();
        let mut bad = ice::Holes {
            found: true,
            values: [702, 538, 144, 140, 630, 512, 150, 140],
        };
        bad.values[1] = 2000;
        assert!(t.bulk_out(&bad.block88()).is_err());
        bad.values[1] = 538;
        t.bulk_out(&[0x1E, 0x88]).unwrap();
        t.bulk_out(&bad.block88()).unwrap();
        assert!(t.bulk_out(&[0x1E, 0x88]).is_err(), "1E 88 twice");
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        assert!(t.bulk_out(&[ice::LAMP_INFRARED]).is_err());
        // The infrared pass only after the colour pass's plan; then its light, its 1E 57, its
        // lid unit's move, 1E 87 01, longer window times.
        assert!(t.permit_infrared_pass().is_err());
        let corr = setup::XCorrection(-133);
        let colour =
            FilmPlan::from_geometry(neg, film::geometry(&neg, corr).unwrap(), Pass::IceColour);
        assert!(t
            .permit_film_plan(&FilmPlan::new(neg, corr).unwrap())
            .is_err());
        t.permit_film_plan(&colour).unwrap();
        t.permit_infrared_pass().unwrap();
        t.bulk_out(&[0x1E, 0x57]).unwrap();
        t.bulk_out(&ice::request_block(&neg, Pass::Infrared).unwrap())
            .unwrap();
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        t.bulk_out(&[ice::LAMP_INFRARED]).unwrap();
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        assert!(t.bulk_out(&[0x02]).is_err());
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        t.bulk_out(&setup::move_block(ice::IR_UNIT_MOVE, 0, 0x1FF, 0x10))
            .unwrap();
        t.bulk_out(&[0x1E, 0x87]).unwrap();
        t.bulk_out(&[0x01]).unwrap();
        assert!(t.bulk_out(&[0x1E, 0x88]).is_err());
        let mut long = [0u8; 26];
        long[..3].copy_from_slice(&ice::IR_MAX_LINE_TIME.to_le_bytes()[..3]);
        t.bulk_out(&[0x1E, 0x21]).unwrap();
        t.bulk_out(&long).unwrap();
        long[..3].copy_from_slice(&(ice::IR_MAX_LINE_TIME + 384).to_le_bytes()[..3]);
        t.bulk_out(&[0x1E, 0x21]).unwrap();
        assert!(t.bulk_out(&long).is_err());
        // The infrared plan: as captured, or refused.
        let h = |v| ice::Holes {
            found: true,
            values: v,
        };
        let ir = ice::infrared_plan(&colour, corr, &h(ICE[1].3), &h(ICE[1].4)).unwrap();
        let mut far = ir.clone();
        far.geometry.start_steps += 10;
        assert!(t.permit_film_plan(&far).is_err());
        // 108's start is 44 after the colour one: 37 (seen on Linux) let out, 34 not.
        let mut low = ir.clone();
        low.geometry.start_steps -= 10;
        assert!(t.permit_film_plan(&low).is_err());
        low.geometry.start_steps += 3;
        check_infrared_plan(&colour, &low).unwrap();
        let mut wide = ir.clone();
        wide.geometry.pixels += 512;
        assert!(t.permit_film_plan(&wide).is_err());
        t.permit_film_plan(&ir).unwrap();
    }

    #[test]
    fn a_glass_scan_cannot_light_the_lids_lamp() {
        let mut t = guarded(&[]);
        t.permit_request(&Request::new(300, 0, 0, 591, 591, 8))
            .unwrap();
        t.bulk_out(&[0x1E, 0xA2]).unwrap();
        assert!(t.bulk_out(&[0x02]).is_err());
    }

    #[test]
    fn a_films_path_lets_out_only_the_moves_and_times_captured() {
        let r = crate::film::tests::CAPTURED[2].1;
        let mut t = guarded(&[]);
        // Not before its path is opened.
        t.permit_film(&r).unwrap();
        assert!(t.bulk_out(&[0x1E, 0x25]).is_err());
        t.permit_film_path().unwrap();
        // The park, then its 1E 05; a park of another length never.
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        t.bulk_out(&setup::move_block(970, 0, 0x1FF, 0x10)).unwrap();
        t.bulk_out(&[0x1E, 0x05]).unwrap();
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        assert!(t.bulk_out(&setup::move_block(971, 0, 0x1FF, 0x10)).is_err());
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
        // Moves back outside those let out, and their 1E 8A: refused (a line beyond either end,
        // or not a whole line of the read); every line from 180 to 312 let out.
        for n in [168, 324, 270] {
            t.bulk_out(&[0x1E, 0x01]).unwrap();
            assert!(t.bulk_out(&setup::move_block(n, 0, 0x1FF, 0x30)).is_err());
            t.bulk_out(&[0x1E, 0x8A]).unwrap();
            assert!(t.bulk_out(&film_origin_block(n)).is_err());
        }
        for n in [180, 204, 216, 240, 264, 276, 312] {
            t.bulk_out(&[0x1E, 0x01]).unwrap();
            t.bulk_out(&setup::move_block(n, 0, 0x1FF, 0x30)).unwrap();
            t.bulk_out(&[0x1E, 0x8A]).unwrap();
            t.bulk_out(&film_origin_block(n)).unwrap();
        }
        // Line times: multiples of 384 within the captured range only.
        let block = |t: u32| {
            let mut b = [0u8; 26];
            b[..3].copy_from_slice(&t.to_le_bytes()[..3]);
            b
        };
        for (lt, ok) in [
            (13_440, true),
            (13_441, false),
            (5760, false),
            (27_264, true),
            (27_648, false),
        ] {
            t.bulk_out(&[0x1E, 0x21]).unwrap();
            assert_eq!(t.bulk_out(&block(lt)).is_ok(), ok, "{lt}");
        }
        // The image's start and read moves only once the plan is permitted.
        let p = FilmPlan::new(r, setup::XCorrection(-122)).unwrap();
        let start = setup::move_block(p.geometry.start_steps, 0, 0x1FF, 0x10);
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        assert!(t.bulk_out(&start).is_err());
        t.permit_film_plan(&p).unwrap();
        let read = setup::move_block(p.geometry.read_steps, p.geometry.lead as u16, 0, 0x08);
        // The read move not before the start move.
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        assert!(t.bulk_out(&read).is_err());
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        t.bulk_out(&start).unwrap();
        t.bulk_out(&[0x1E, 0x05]).unwrap();
        // The read's lead as the image's line time asks (`film::lead_for`): this 600 dpi plan's
        // 128 not after 27,264 µs (32), after 13,440 (capture 74) yes.
        t.bulk_out(&[0x1E, 0x21]).unwrap();
        t.bulk_out(&block(27_264)).unwrap();
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        let e = t.bulk_out(&read).unwrap_err().to_string();
        assert!(e.contains("lead of 128"), "{e}");
        t.bulk_out(&[0x1E, 0x21]).unwrap();
        t.bulk_out(&block(13_440)).unwrap();
        // The start move once, the read move once.
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        assert!(t.bulk_out(&start).is_err());
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        t.bulk_out(&read).unwrap();
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
        t.bulk_out(&[0x1E, 0x01]).unwrap();
        assert!(t.bulk_out(&read).is_err());
        // The 07 table and its raw line, as on the glass.
        t.bulk_out(&[0x1E, 0x83]).unwrap();
        t.bulk_out(&table07::READ_RAW).unwrap();
        t.bulk_out(&[0x1E, 0x84]).unwrap();
        t.bulk_out(&table07::HEADER).unwrap();
        t.bulk_out(&[0u8; table07::LEN]).unwrap();
        // A one-value ramp faster than any captured.
        t.bulk_out(&[0x1E, 0x84]).unwrap();
        t.bulk_out(&setup::ramp_header(1)).unwrap();
        assert!(t.bulk_out(&100u16.to_le_bytes()).is_err());
    }
}
