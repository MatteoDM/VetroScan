// SPDX-License-Identifier: GPL-2.0-or-later
//! The scanner's native commands that this backend sends: only the ones
//! that move nothing.
//!
//! Everything goes over two bulk endpoints (no control transfers): a
//! command is two bytes, `1B xx` or `1E xx`, often followed by a data
//! block, and most are answered by one byte, [`ACK`]. What is here was read
//! from Epson's Windows driver (`esxwia54.dll`, `esxuin54.dll`; notes in
//! `captures/epson4490/`, outside the repository) under the EU
//! interoperability exception, and **has not been checked on a scanner
//! yet**: the first USB capture must confirm every byte.
//!
//! Nothing that moves the carriage, nor switches a lamp, may be added here
//! before the scan sequence is understood (phase 12 of
//! `docs/architecture-plan.md`).

use crate::firmware::{self, Firmware, HEADER_LEN, PAYLOAD_LEN};
use crate::Transport;
use scanner_api::{Error, Result};

/// The scanner's "yes".
pub const ACK: u8 = 0x06;
/// Asks the state: two bytes come back ([`State`]).
pub const GET_STATE: [u8; 2] = [0x1B, 0x03];
/// Starts a firmware upload; answered by [`ACK`].
pub const FIRMWARE_BEGIN: [u8; 2] = [0x1B, 0x06];
/// Ends a firmware upload; answered by [`ACK`].
pub const FIRMWARE_END: [u8; 2] = [0x1B, 0x16];
/// Sent alone after [`FIRMWARE_END`] (starts the firmware); answered by
/// [`ACK`].
pub const FIRMWARE_RUN: u8 = 0x80;
/// The longest write of the firmware's bytes, as Epson's driver does them.
pub const FIRMWARE_CHUNK: usize = 0xFFF0;

/// The two bytes of [`GET_STATE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State(pub [u8; 2]);

impl State {
    /// In the first byte: the firmware is running.
    pub const FIRMWARE_RUNNING: u8 = 0x10;
    /// In the first byte: busy (Epson's interpreter waits, polling every
    /// 50 ms, until both are clear).
    pub const BUSY: u8 = 0x42;

    /// Whether the firmware is running (nothing to upload).
    pub fn firmware_running(self) -> bool {
        self.0[0] & Self::FIRMWARE_RUNNING != 0
    }

    /// Whether the scanner is busy.
    pub fn busy(self) -> bool {
        self.0[0] & Self::BUSY != 0
    }
}

/// Reads exactly `buf.len()` bytes.
fn read_exact(t: &mut dyn Transport, buf: &mut [u8], what: &str) -> Result<()> {
    let mut done = 0;
    while done < buf.len() {
        let n = t.bulk_in(&mut buf[done..])?;
        if n == 0 {
            return Err(Error::Other(format!(
                "{what}: the scanner answered {done} bytes of {}",
                buf.len()
            )));
        }
        done += n;
    }
    Ok(())
}

/// Reads one byte and wants [`ACK`].
fn expect_ack(t: &mut dyn Transport, after: &str) -> Result<()> {
    let mut answer = [0u8];
    read_exact(t, &mut answer, after)?;
    if answer[0] != ACK {
        return Err(Error::Other(format!(
            "firmware upload refused: after {after} the scanner answered {:02X}, not {ACK:02X}",
            answer[0]
        )));
    }
    Ok(())
}

/// The scanner's state ([`GET_STATE`]).
pub fn state(t: &mut dyn Transport) -> Result<State> {
    t.bulk_out(&GET_STATE)?;
    let mut answer = [0u8; 2];
    read_exact(t, &mut answer, "state (1B 03)")?;
    Ok(State(answer))
}

/// The 8-bit sum the scanner checks: of the payload's bytes but the last,
/// which it replaces.
pub fn checksum(payload: &[u8]) -> u8 {
    payload[..payload.len() - 1]
        .iter()
        .fold(0u8, |sum, &b| sum.wrapping_add(b))
}

/// Sends the firmware `image` (the whole `esfw54.bin`, header included) as
/// Epson's Windows driver does:
///
/// 1. `1B 06`, [`ACK`];
/// 2. the payload's length, 4 bytes little-endian (`01 00 01 00`);
/// 3. the payload but its last byte, in writes of at most
///    [`FIRMWARE_CHUNK`] bytes (65,520 + 16);
/// 4. in place of the last byte, the [`checksum`], then [`ACK`];
/// 5. `1B 16`, [`ACK`]; `80`, [`ACK`].
///
/// Any other answer stops it with an error. Whether the scanner leaves the
/// bus and comes back after the last step is not known yet.
pub fn upload_firmware(t: &mut dyn Transport, image: &[u8]) -> Result<()> {
    firmware::check_image(image).map_err(Error::Other)?;
    let payload = &image[HEADER_LEN..];
    debug_assert_eq!(payload.len(), PAYLOAD_LEN);

    t.bulk_out(&FIRMWARE_BEGIN)?;
    expect_ack(t, "1B 06")?;
    let len = u32::try_from(payload.len()).expect("payload fits u32");
    t.bulk_out(&len.to_le_bytes())?;
    for chunk in payload[..payload.len() - 1].chunks(FIRMWARE_CHUNK) {
        t.bulk_out(chunk)?;
    }
    t.bulk_out(&[checksum(payload)])?;
    expect_ack(t, "the firmware's checksum")?;
    t.bulk_out(&FIRMWARE_END)?;
    expect_ack(t, "1B 16")?;
    t.bulk_out(&[FIRMWARE_RUN])?;
    expect_ack(t, "80")
}

/// What [`ensure_firmware`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirmwareOutcome {
    /// It was running already (sent earlier, by us or by Epson's driver).
    AlreadyRunning,
    /// It was not: this copy was sent.
    Uploaded {
        build: firmware::Build,
        origin: firmware::Origin,
    },
}

/// Sends the firmware if it is not running. `load` is only called when it
/// is needed, so a scanner whose firmware is running needs no file.
pub fn ensure_firmware(
    t: &mut dyn Transport,
    load: impl FnOnce() -> Result<Firmware>,
) -> Result<FirmwareOutcome> {
    if state(t)?.firmware_running() {
        return Ok(FirmwareOutcome::AlreadyRunning);
    }
    let fw = load()?;
    upload_firmware(t, fw.image())?;
    Ok(FirmwareOutcome::Uploaded {
        build: fw.build(),
        origin: fw.origin().clone(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::firmware::{FILE_LEN, MAGIC};
    use std::collections::VecDeque;

    /// A transport that records every write and answers from a script.
    #[derive(Debug, Default)]
    pub(crate) struct Mock {
        pub writes: Vec<Vec<u8>>,
        pub answers: VecDeque<Vec<u8>>,
    }

    impl Mock {
        pub(crate) fn answering(answers: &[&[u8]]) -> Self {
            Self {
                writes: Vec::new(),
                answers: answers.iter().map(|a| a.to_vec()).collect(),
            }
        }
    }

    impl Transport for Mock {
        fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
            self.writes.push(data.to_vec());
            Ok(())
        }

        fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
            let Some(mut answer) = self.answers.pop_front() else {
                return Err(Error::Other("mock: no answer left".into()));
            };
            let n = answer.len().min(buf.len());
            buf[..n].copy_from_slice(&answer[..n]);
            if n < answer.len() {
                answer.drain(..n);
                self.answers.push_front(answer);
            }
            Ok(n)
        }
    }

    /// A made-up image with the real shape: "EDLA", zeros, then bytes that
    /// are not the firmware.
    pub(crate) fn image() -> Vec<u8> {
        let mut image = vec![0u8; FILE_LEN];
        image[..4].copy_from_slice(MAGIC);
        for (i, b) in image[HEADER_LEN..].iter_mut().enumerate() {
            *b = (i * 7 + i / 251) as u8;
        }
        image
    }

    const ACKS: [&[u8]; 4] = [&[ACK], &[ACK], &[ACK], &[ACK]];

    #[test]
    fn the_upload_sends_exactly_the_documented_bytes() {
        let image = image();
        let payload = &image[HEADER_LEN..];
        let mut t = Mock::answering(&ACKS);
        upload_firmware(&mut t, &image).unwrap();

        let sum = payload[..65_536]
            .iter()
            .fold(0u8, |s, &b| s.wrapping_add(b));
        assert_ne!(
            sum, payload[65_536],
            "the test needs the last byte replaced"
        );
        let expected: Vec<Vec<u8>> = vec![
            vec![0x1B, 0x06],
            vec![0x01, 0x00, 0x01, 0x00],
            payload[..65_520].to_vec(),
            payload[65_520..65_536].to_vec(),
            vec![sum],
            vec![0x1B, 0x16],
            vec![0x80],
        ];
        assert_eq!(t.writes.len(), expected.len());
        for (i, (w, e)) in t.writes.iter().zip(&expected).enumerate() {
            assert!(w == e, "write {i}: {} bytes, expected {}", w.len(), e.len());
        }
        assert!(t.answers.is_empty(), "every ACK read");
    }

    #[test]
    fn any_answer_but_ack_stops_the_upload() {
        // A wrong answer at each of the four places, in turn: nothing is
        // sent after it.
        let writes_before = [1, 5, 6, 7];
        for (place, &writes) in writes_before.iter().enumerate() {
            let mut answers: Vec<&[u8]> = ACKS.to_vec();
            answers[place] = &[0x15];
            let mut t = Mock::answering(&answers);
            let e = upload_firmware(&mut t, &image()).unwrap_err();
            assert!(e.to_string().contains("answered 15"), "{e}");
            assert_eq!(t.writes.len(), writes, "place {place}");
        }
    }

    #[test]
    fn a_silent_scanner_stops_the_upload() {
        let mut t = Mock::answering(&[&[]]);
        assert!(upload_firmware(&mut t, &image()).is_err());
        assert_eq!(t.writes, vec![vec![0x1B, 0x06]]);
    }

    #[test]
    fn a_malformed_image_sends_nothing() {
        let mut t = Mock::answering(&ACKS);
        assert!(upload_firmware(&mut t, &image()[..1000]).is_err());
        let mut no_magic = image();
        no_magic[0] = b'X';
        assert!(upload_firmware(&mut t, &no_magic).is_err());
        assert!(t.writes.is_empty());
    }

    #[test]
    fn a_running_firmware_is_not_sent_again() {
        let mut t = Mock::answering(&[&[0x10, 0x00]]);
        let outcome = ensure_firmware(&mut t, || panic!("no firmware needed")).unwrap();
        assert_eq!(outcome, FirmwareOutcome::AlreadyRunning);
        assert_eq!(t.writes, vec![GET_STATE.to_vec()]);
    }

    #[test]
    fn a_stopped_firmware_is_sent_after_the_state() {
        let image = image();
        let mut answers: Vec<&[u8]> = vec![&[0x00, 0x00]];
        answers.extend(ACKS);
        let mut t = Mock::answering(&answers);
        let outcome = ensure_firmware(&mut t, || Ok(Firmware::unchecked(image.clone()))).unwrap();
        assert!(matches!(outcome, FirmwareOutcome::Uploaded { .. }));
        assert_eq!(t.writes.len(), 1 + 7);
        assert_eq!(t.writes[0], GET_STATE);
        assert_eq!(t.writes[1], FIRMWARE_BEGIN);
        assert_eq!(t.writes[7], [FIRMWARE_RUN]);
    }

    #[test]
    fn without_firmware_nothing_but_the_state_is_sent() {
        let mut t = Mock::answering(&[&[0x00, 0x00]]);
        let e = ensure_firmware(&mut t, || Err(Error::Other("no esfw54.bin".into()))).unwrap_err();
        assert_eq!(e, Error::Other("no esfw54.bin".into()));
        assert_eq!(t.writes, vec![GET_STATE.to_vec()]);
    }

    #[test]
    fn the_state_may_come_in_pieces() {
        let mut t = Mock::answering(&[&[0x52], &[0x01]]);
        let s = state(&mut t).unwrap();
        assert_eq!(s, State([0x52, 0x01]));
        assert!(s.firmware_running());
        assert!(s.busy());
        assert!(!State([0x10, 0xFF]).busy());
        assert!(State([0x02, 0x00]).busy());
        assert!(!State([0xEF, 0x00]).firmware_running());
    }
}
