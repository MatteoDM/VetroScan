// SPDX-License-Identifier: GPL-2.0-or-later
//! A fake scanner that answers from a capture.
//!
//! [`Replay`] implements [`Transport`]: every request the driver makes is
//! compared with the next request the HP driver made in the capture, and
//! IN requests get the answer the real scanner gave. The first difference
//! stops the replay with an error that says where the two drivers diverged.
//! This is how translated `hp3900` code is checked against the HP driver
//! without the hardware.

use crate::trace::{Event, Trace};
use hp3770_core::protocol::{Command, Hex, SetupPacket};
use hp3770_core::transport::Transport;
use hp3770_core::{Error, Result};
use std::collections::VecDeque;
use std::time::Duration;

/// Replays the control and bulk traffic of a capture.
#[derive(Debug, Clone)]
pub struct Replay {
    /// Control and bulk events, in order.
    events: Vec<Event>,
    /// Next event to match.
    pos: usize,
    /// Bytes of the current bulk IN event not yet handed out.
    bulk_offset: usize,
    /// Button codes, delivered in order whenever asked.
    buttons: VecDeque<u8>,
}

impl Replay {
    /// Replays `events`. Standard requests (descriptors, configuration) are
    /// dropped: they belong to the operating system, not to the driver.
    pub fn new(events: impl IntoIterator<Item = Event>) -> Self {
        let mut buttons = VecDeque::new();
        let events = events
            .into_iter()
            .filter(|e| match e {
                Event::Interrupt { data, .. } => {
                    buttons.extend(data.first().copied());
                    false
                }
                Event::Control {
                    command: Command::Standard(_),
                    ..
                } => false,
                _ => true,
            })
            .collect();
        Self {
            events,
            pos: 0,
            bulk_offset: 0,
            buttons,
        }
    }

    /// Replays a whole trace.
    pub fn from_trace(trace: &Trace) -> Self {
        Self::new(trace.events.iter().cloned())
    }

    /// Replays the events whose time is in `[from, to)` seconds.
    pub fn window(trace: &Trace, from: f64, to: f64) -> Self {
        Self::new(
            trace
                .events
                .iter()
                .filter(|e| (from..to).contains(&e.time()))
                .cloned(),
        )
    }

    /// Events matched so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Events not matched yet.
    pub fn remaining(&self) -> usize {
        self.events.len() - self.pos
    }

    /// Succeeds if the driver made every request of the capture.
    pub fn finish(&self) -> Result<()> {
        match self.events.get(self.pos) {
            None => Ok(()),
            Some(e) => Err(self.diverged(
                &format!("the driver stopped, {} event(s) left", self.remaining()),
                e,
            )),
        }
    }

    fn diverged(&self, actual: &str, expected: &Event) -> Error {
        Error::Transport(format!(
            "replay diverged at event {} ({:.3} s): expected `{}`, got {actual}",
            self.pos,
            expected.time(),
            describe(expected)
        ))
    }

    fn next(&self) -> Result<&Event> {
        self.events
            .get(self.pos)
            .ok_or_else(|| Error::Transport(format!("replay ended after {} events", self.pos)))
    }

    fn control(&mut self, setup: SetupPacket, data: &[u8]) -> Result<Vec<u8>> {
        let actual = Command::decode(setup, data);
        let expected = self.next()?;
        let Event::Control {
            command, response, ..
        } = expected
        else {
            return Err(self.diverged(&format!("`{actual}`"), expected));
        };
        if *command != actual {
            return Err(self.diverged(&format!("`{actual}`"), expected));
        }
        let response = response.clone();
        self.pos += 1;
        Ok(response)
    }
}

fn describe(event: &Event) -> String {
    match event {
        Event::Control { command, .. } => command.to_string(),
        Event::BulkIn { data, .. } => format!("bulk IN of {} bytes", data.len()),
        Event::BulkOut { data, .. } => format!("bulk OUT of {} bytes", data.len()),
        Event::Interrupt { data, .. } => format!("interrupt {}", Hex(data)),
    }
}

impl Transport for Replay {
    fn control_in(&mut self, setup: SetupPacket) -> Result<Vec<u8>> {
        self.control(setup, &[])
    }

    fn control_out(&mut self, setup: SetupPacket, data: &[u8]) -> Result<()> {
        self.control(setup, data).map(drop)
    }

    /// Hands out the bytes of consecutive bulk IN events as one stream, so
    /// the driver may read in chunks of a different size than the HP driver.
    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            let Some(Event::BulkIn { data, .. }) = self.events.get(self.pos) else {
                break;
            };
            let take = (data.len() - self.bulk_offset).min(buf.len() - n);
            buf[n..n + take].copy_from_slice(&data[self.bulk_offset..self.bulk_offset + take]);
            n += take;
            self.bulk_offset += take;
            if self.bulk_offset == data.len() {
                self.pos += 1;
                self.bulk_offset = 0;
            }
        }
        if n == 0 && !buf.is_empty() {
            let expected = self.next()?;
            return Err(self.diverged(&format!("bulk IN of {} bytes", buf.len()), expected));
        }
        Ok(n)
    }

    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        let expected = self.next()?;
        match expected {
            Event::BulkOut { data: d, .. } if d == data => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(self.diverged(
                &format!("bulk OUT of {} bytes: {}", data.len(), Hex(data)),
                expected,
            )),
        }
    }

    fn interrupt_in(&mut self, _timeout: Duration) -> Result<Option<u8>> {
        Ok(self.buttons.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hp3770_core::transport::Chip;

    fn ev(command: Command, response: &[u8]) -> Event {
        Event::Control {
            time: 1.0,
            command,
            response: response.to_vec(),
            status: 0,
        }
    }

    #[test]
    fn answers_and_checks_requests() {
        let mut chip = Chip::new(Replay::new([
            ev(
                Command::ReadRegisters {
                    address: 0xfe3a,
                    length: 2,
                },
                &[0x13, 0x02],
            ),
            ev(
                Command::WriteRegisters {
                    address: 0xfe39,
                    data: vec![0xa3, 0x13],
                },
                &[],
            ),
        ]));
        chip.write_byte(0xfe39, 0xa3).unwrap();
        chip.transport_mut().finish().unwrap();
    }

    #[test]
    fn reports_the_divergence() {
        let mut chip = Chip::new(Replay::new([ev(
            Command::WriteRegisters {
                address: 0xe800,
                data: vec![0x84, 0x40],
            },
            &[],
        )]));
        let err = chip.write_registers(0xe800, &[0x04, 0x40]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("event 0"), "{msg}");
        assert!(msg.contains("84 40") && msg.contains("04 40"), "{msg}");
        assert!(chip.transport_mut().finish().is_err());
    }

    #[test]
    fn bulk_in_is_a_stream() {
        let bulk = |data: &[u8]| Event::BulkIn {
            time: 0.0,
            endpoint: 0x81,
            data: data.to_vec(),
            status: 0,
        };
        let mut r = Replay::new([bulk(&[1, 2, 3]), bulk(&[4, 5])]);
        let mut buf = [0u8; 4];
        assert_eq!(r.bulk_in(&mut buf).unwrap(), 4);
        assert_eq!(buf, [1, 2, 3, 4]);
        assert_eq!(r.bulk_in(&mut buf).unwrap(), 1);
        assert_eq!(buf[0], 5);
        r.finish().unwrap();
        assert!(r.bulk_in(&mut buf).is_err());
    }
}
