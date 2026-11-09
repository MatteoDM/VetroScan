// SPDX-License-Identifier: GPL-2.0-or-later
//! A guard on everything the backend writes to the scanner: only the
//! commands of milestone 1 go out, so nothing can move the carriage by
//! mistake.
//!
//! Read from Epson's interpreter (`captures/epson4490/04-carriage.md`,
//! outside the repository), **not confirmed by a capture yet**: a move is
//! `1E 84` (a ramp table) + `1E 01` (12 bytes) + `1E 05` (go), after
//! `1E 25` (what moves); `1E F4` + 16 bits is another move; `1E 47` /
//! `1E 48` start a scan. Rather than a list of what is forbidden, the guard
//! lets through a list of what is known to move nothing:
//!
//! * `1B 03` (state) and `1E 86` (the two positions), read-only;
//! * the firmware upload: `1B 06`, then only its own data (the length
//!   `01 00 01 00`, 65,536 bytes, the checksum), `1B 16`, and `80` right
//!   after `1B 16`.
//!
//! Anything else is refused with an error and never reaches the scanner.
//! A command is let through only when a capture of Epson's driver has
//! shown it, and then added here on purpose.

use crate::firmware::PAYLOAD_LEN;
use crate::protocol::{FIRMWARE_BEGIN, FIRMWARE_END, FIRMWARE_RUN, GET_STATE};
use crate::Transport;
use scanner_api::{Error, Result};

/// Reads the two 16-bit positions (carriage, transparency unit): 4 bytes.
pub const GET_POSITIONS: [u8; 2] = [0x1E, 0x86];

/// The commands known to move something, with what they do, for the
/// error message (all refused anyway: they are not in the allowed list).
const MOVERS: [(u8, &str); 7] = [
    (0x01, "the parameters of a carriage move"),
    (0x05, "starts a carriage move"),
    (0x25, "selects what moves"),
    (0x47, "starts a scan"),
    (0x48, "starts a scan"),
    (0x84, "loads a motor ramp"),
    (0xF4, "moves the carriage"),
];

/// The payload's length, as the upload sends it.
const UPLOAD_LENGTH: [u8; 4] = (PAYLOAD_LEN as u32).to_le_bytes();

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
    /// After `1B 16`: `80` may follow.
    Run,
}

/// A [`Transport`] that only lets the allowed writes through.
#[derive(Debug)]
pub struct Guarded<T: Transport> {
    inner: T,
    stage: Stage,
}

impl<T: Transport> Guarded<T> {
    /// Guards `inner`.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            stage: Stage::Idle,
        }
    }

    /// The guarded transport.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Whether `data` may be written now, and the stage after it.
    fn check(&self, data: &[u8]) -> Result<Stage> {
        match self.stage {
            Stage::Length if data == UPLOAD_LENGTH => return Ok(Stage::Payload(PAYLOAD_LEN - 1)),
            Stage::Payload(left) if !data.is_empty() && data.len() <= left => {
                return Ok(match left - data.len() {
                    0 => Stage::Checksum,
                    left => Stage::Payload(left),
                })
            }
            Stage::Checksum if data.len() == 1 => return Ok(Stage::Idle),
            Stage::Run if data == [FIRMWARE_RUN] => return Ok(Stage::Idle),
            // Anything else ends an upload: judged as a command.
            _ => {}
        }
        match data {
            d if d == GET_STATE || d == GET_POSITIONS => Ok(Stage::Idle),
            d if d == FIRMWARE_BEGIN => Ok(Stage::Length),
            d if d == FIRMWARE_END => Ok(Stage::Run),
            _ => Err(refusal(data)),
        }
    }
}

fn refusal(data: &[u8]) -> Error {
    let head: Vec<String> = data.iter().take(4).map(|b| format!("{b:02X}")).collect();
    let more = if data.len() > 4 { " ..." } else { "" };
    let what = match data {
        [0x1E, code, ..] => MOVERS
            .iter()
            .find(|(c, _)| c == code)
            .map_or("not a command this backend may send yet", |(_, what)| *what),
        _ => "not a command this backend may send yet",
    };
    Error::Other(format!(
        "refused to send {}{more} to the Epson 4490 ({what}): nothing that may move the \
         carriage goes out before a capture of Epson's driver confirms it",
        head.join(" ")
    ))
}

impl<T: Transport> Transport for Guarded<T> {
    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        let next = match self.check(data) {
            Ok(next) => next,
            Err(e) => {
                self.stage = Stage::Idle;
                return Err(e);
            }
        };
        self.inner.bulk_out(data)?;
        self.stage = next;
        Ok(())
    }

    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        // The firmware's bytes go out without an answer in between: a read
        // there means the upload was given up.
        if matches!(self.stage, Stage::Payload(_) | Stage::Checksum) {
            self.stage = Stage::Idle;
        }
        self.inner.bulk_in(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::HEADER_LEN;
    use crate::protocol::tests::{image, Mock};
    use crate::protocol::{self, ACK};

    fn guarded(answers: &[&[u8]]) -> Guarded<Mock> {
        Guarded::new(Mock::answering(answers))
    }

    #[test]
    fn the_commands_that_move_are_refused_and_never_sent() {
        let movers: [&[u8]; 8] = [
            &[0x1E, 0x05],
            &[0x1E, 0xF4, 0x10, 0x00],
            &[0x1E, 0xF4],
            &[0x1E, 0x47],
            &[0x1E, 0x48],
            &[0x1E, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10, 0],
            &[0x1E, 0x84, 0xC4, 0x09],
            &[0x1E, 0x25, 0x00],
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
    fn the_read_only_commands_go_out() {
        let mut t = guarded(&[]);
        t.bulk_out(&GET_STATE).unwrap();
        t.bulk_out(&GET_POSITIONS).unwrap();
        assert_eq!(
            t.inner().writes,
            vec![GET_STATE.to_vec(), GET_POSITIONS.to_vec()]
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
        protocol::upload_firmware(&mut t, &image).unwrap();
        assert_eq!(t.inner().writes, direct.writes);
        // Over: 80 and moves are refused again.
        assert!(t.bulk_out(&[FIRMWARE_RUN]).is_err());
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
    }

    #[test]
    fn a_refused_upload_leaves_nothing_open() {
        // 1B 06 answered with a NAK: the upload stops after its command.
        let mut t = guarded(&[&[0x15]]);
        assert!(protocol::upload_firmware(&mut t, &image()).is_err());
        assert!(t.bulk_out(&[0x1E, 0x05]).is_err());
        // Stopped in the middle of the data, after a read.
        let mut t = guarded(&[&[ACK], &[0x00]]);
        t.bulk_out(&FIRMWARE_BEGIN).unwrap();
        t.bulk_out(&UPLOAD_LENGTH).unwrap();
        t.bulk_out(&[0; 100]).unwrap();
        t.bulk_in(&mut [0]).unwrap();
        assert!(t.bulk_out(&[0x1E, 0x47]).is_err());
        // More data than the firmware has is refused.
        let mut t = guarded(&[]);
        t.bulk_out(&FIRMWARE_BEGIN).unwrap();
        t.bulk_out(&UPLOAD_LENGTH).unwrap();
        assert!(t.bulk_out(&vec![0; PAYLOAD_LEN]).is_err());
    }
}
