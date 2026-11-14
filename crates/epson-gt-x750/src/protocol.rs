// SPDX-License-Identifier: GPL-2.0-or-later
//! The scanner's native commands outside a scan: the state, the buttons, the
//! firmware upload and the lamp switched off. Nothing here moves the carriage
//! (the scan's commands are in [`crate::scan`]).
//!
//! Everything goes over two bulk endpoints (no control transfers): a
//! command is two bytes, `1B xx` or `1E xx`, often followed by a data
//! block, and most are answered by one byte, [`ACK`]. What is here was read
//! from Epson's Windows driver (`esxwia54.dll`, `esxuin54.dll`; notes in
//! `captures/epson4490/`, outside the repository) under the EU
//! interoperability exception, then checked byte for byte against USB
//! captures of Epson's driver and tried on the scanner (11-12 November 2026).

use crate::firmware::{self, Firmware, HEADER_LEN, PAYLOAD_LEN};
use crate::Transport;
use scanner_api::{Error, Result};
use std::time::Duration;

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
/// Asks which button is pressed: one byte comes back, `00` none, else the
/// button's number (captured on 11 November 2026: `01` Start, `02` Scan to
/// PDF, `03` Scan to E-mail, `04` Copy, each read for about 2 s after a
/// press). Epson's driver asks it every second.
pub const GET_BUTTONS: [u8; 2] = [0x1E, 0x85];
/// The longest write of the firmware's bytes, as Epson's driver does them.
pub const FIRMWARE_CHUNK: usize = 0xFFF0;
/// The longest write of a `1E 84` table: a longer one is written in pieces of this size, as
/// Epson Scan did with capture 42's shading (122,496 bytes, at 1200 dpi on the whole glass):
/// after the 8-byte header, a write of 65,520 bytes (its USBPcap length; the capture's snaplen
/// keeps 65,508 of them) and one of 56,976, back to back, then the one `06`. Sent whole,
/// 85,056 bytes of shading (1200 dpi, 150 mm wide) went unanswered and left the scanner taking
/// whatever came next as more of the table (13 November 2026, the scanner switched off and on).
pub const TABLE_CHUNK: usize = FIRMWARE_CHUNK;

/// The pause before each piece of a `1E 84` table but the first. Epson Scan wrote capture
/// 42's second piece 1.2 ms after its first was done. Ours, with no pause, went unanswered
/// through `usbscan.sys` three times out of three (1200 dpi, 150 mm wide, 13 November 2026),
/// and was answered seven times out of seven with USBPcap capturing (capture 55: 1.7 ms
/// between the pieces, the capture's own delay): the scanner seems to need a moment to take
/// a piece before the next.
pub const TABLE_PAUSE: Duration = Duration::from_millis(5);

/// The two bytes of [`GET_STATE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State(pub [u8; 2]);

impl State {
    /// In the first byte: the firmware is running.
    pub const FIRMWARE_RUNNING: u8 = 0x10;
    /// The only state seen without the firmware: switched on, before Epson's driver sent it
    /// (capture 01).
    pub const NO_FIRMWARE: [u8; 2] = [0x28, 0x00];
    /// Without the firmware, the carriage left away from home: seen twice on 13 November 2026
    /// after a scan hung half-way and the scanner was switched off and on (the carriage stayed
    /// where it was, the front light green). Bit `0x20` is then taken to be "the carriage at
    /// home" (a guess, twice consistent). The firmware sent then brought the carriage home by
    /// itself both times (`5A 81` about 13 s in, `18 81` at 20-22 s), as after every upload.
    pub const NO_FIRMWARE_CARRIAGE_AWAY: [u8; 2] = [0x08, 0x00];
    /// In the first byte: busy (Epson's interpreter waits, polling every
    /// 50 ms, until both are clear).
    pub const BUSY: u8 = 0x42;
    /// In the first byte: the scanner's own error. Seen on 13 November 2026 as `98 81`, its
    /// front light flashing red, after a preview with the lid open (the lamp did not light and
    /// nothing moved); Epson Scan, opened then, cleared it. Never in a capture.
    pub const ERROR: u8 = 0x80;

    /// Whether the firmware is running (nothing to upload).
    pub fn firmware_running(self) -> bool {
        self.0[0] & Self::FIRMWARE_RUNNING != 0
    }

    /// Whether the scanner is busy.
    pub fn busy(self) -> bool {
        self.0[0] & Self::BUSY != 0
    }

    /// Whether the scanner flags its own error ([`State::ERROR`]).
    pub fn error(self) -> bool {
        self.0[0] & Self::ERROR != 0
    }

    /// What to tell the user about [`State::error`].
    pub fn error_text(self) -> String {
        format!(
            "the scanner reports an error of its own (1B 03 answered {:02X} {:02X}, its light \
             flashing red): nothing more was sent to it; the next scan clears it as Epson Scan \
             does (1B 40), and if it comes back, switch it off and on",
            self.0[0], self.0[1]
        )
    }
}

/// Whether `answer` is a state `1B 03` gives a scanner at rest: two bytes, the firmware running
/// with `81` after it (`18 81` idle, `19 81` / `1A 81` the lamp warming, `58 81` the carriage
/// moving), or [`State::NO_FIRMWARE`]. A scanner left half-way through a scan answers with a
/// packet of its image (512 bytes, capture 46).
pub fn is_state(answer: &[u8]) -> bool {
    match answer {
        [a, 0x81] => a & State::FIRMWARE_RUNNING != 0,
        a => a == State::NO_FIRMWARE || a == State::NO_FIRMWARE_CARRIAGE_AWAY,
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

/// The button pressed ([`GET_BUTTONS`]): 0 for none, else 1-4.
pub fn button(t: &mut dyn Transport) -> Result<u8> {
    t.bulk_out(&GET_BUTTONS)?;
    let mut answer = [0u8];
    read_exact(t, &mut answer, "buttons (1E 85)")?;
    Ok(answer[0])
}

/// Switches the lamp: `1E A2`, `06`, the lamp (`00` off, `01` the glass's), `06`.
pub const LAMP: [u8; 2] = [0x1E, 0xA2];

/// Switches the lamp off, as every scan does before its dark calibration (`1E A2 00`). Sent
/// alone (the application's lamp timer) it was first tried on 12 November 2026.
pub fn lamp_off(t: &mut dyn Transport) -> Result<()> {
    for (data, after) in [(&LAMP[..], "1E A2"), (&[0x00][..], "1E A2 00")] {
        t.bulk_out(data)?;
        let mut answer = [0u8];
        read_exact(t, &mut answer, after)?;
        if answer[0] != ACK {
            return Err(Error::Other(format!(
                "after {after} the scanner answered {:02X}, not 06",
                answer[0]
            )));
        }
    }
    Ok(())
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
/// Any other answer stops it with an error. Captured from Epson's driver on
/// 11 November 2026, byte for byte the same; the scanner stays on the bus
/// after `80`, and answers its next command only after about 9 s (it sets
/// itself up), within [`crate::BULK_TIMEOUT`].
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
    let s = state(t)?;
    if s.firmware_running() {
        return Ok(FirmwareOutcome::AlreadyRunning);
    }
    // Only on the states seen before the firmware (the carriage at home or away): two bytes
    // of something else (a scan left half-way, capture 44) must never start an upload.
    if s.0 != State::NO_FIRMWARE && s.0 != State::NO_FIRMWARE_CARRIAGE_AWAY {
        return Err(Error::Other(format!(
            "the Epson 4490 answered {:02X} {:02X} to 1B 03, a state never seen: switch it off \
             and on",
            s.0[0], s.0[1]
        )));
    }
    let fw = load()?;
    t.permit_upload(fw.image());
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
        let mut answers: Vec<&[u8]> = vec![&State::NO_FIRMWARE];
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
        let mut t = Mock::answering(&[&State::NO_FIRMWARE]);
        let e = ensure_firmware(&mut t, || Err(Error::Other("no esfw54.bin".into()))).unwrap_err();
        assert_eq!(e, Error::Other("no esfw54.bin".into()));
        assert_eq!(t.writes, vec![GET_STATE.to_vec()]);
    }

    #[test]
    fn a_state_never_seen_never_starts_an_upload() {
        // Two bytes of the image of a scan left half-way (capture 44's 00 00, 81 00).
        for answer in [[0x00, 0x00], [0x81, 0x00], [0x06, 0x04]] {
            let mut t = Mock::answering(&[&answer]);
            let e = ensure_firmware(&mut t, || panic!("never loaded")).unwrap_err();
            assert!(e.to_string().contains("never seen"), "{e}");
            assert_eq!(t.writes, vec![GET_STATE.to_vec()]);
        }
    }

    /// The carriage left away (`08 00`) is a state without the firmware: it is sent; no other
    /// unknown state is taken.
    #[test]
    fn the_carriage_away_gets_the_firmware() {
        let away = State::NO_FIRMWARE_CARRIAGE_AWAY;
        assert!(is_state(&away));
        let image = image();
        let mut answers: Vec<&[u8]> = vec![&away];
        answers.extend(ACKS);
        let mut t = Mock::answering(&answers);
        let r = ensure_firmware(&mut t, || Ok(Firmware::unchecked(image.clone())));
        assert!(matches!(r, Ok(FirmwareOutcome::Uploaded { .. })), "{r:?}");
        assert_eq!(t.writes[1], FIRMWARE_BEGIN);
        for answer in [[0x00, 0x00], [0x81, 0x00], [0x48, 0x00]] {
            assert!(!is_state(&answer), "{answer:02X?}");
            let mut t = Mock::answering(&[&answer]);
            let e = ensure_firmware(&mut t, || panic!("never loaded")).unwrap_err();
            assert!(e.to_string().contains("never seen"), "{e}");
        }
    }

    #[test]
    fn only_a_scanner_at_rest_answers_a_state() {
        for s in [
            [0x18, 0x81],
            [0x19, 0x81],
            [0x1A, 0x81],
            [0x58, 0x81],
            [0x28, 0x00],
        ] {
            assert!(is_state(&s), "{s:02X?}");
        }
        assert!(!is_state(&[0x06, 0x04]));
        assert!(!is_state(&[0x00, 0x00]));
        assert!(!is_state(&[0x81, 0x00]));
        assert!(!is_state(&[]));
        assert!(!is_state(&[0x18, 0x81, 0x00]));
        assert!(!is_state(&[0x0C; 512]));
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

    #[test]
    fn the_error_seen_on_13_november_is_an_error_not_busy() {
        let s = State([0x98, 0x81]);
        assert!(is_state(&s.0));
        assert!(s.firmware_running());
        assert!(!s.busy());
        assert!(s.error());
        assert!(s.error_text().contains("98 81"));
        for ok in [[0x18, 0x81], [0x19, 0x81], [0x58, 0x81], [0x28, 0x00]] {
            assert!(!State(ok).error(), "{ok:02X?}");
        }
    }
}
