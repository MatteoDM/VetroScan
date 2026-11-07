// SPDX-License-Identifier: GPL-2.0-or-later
//! Register state reconstructed from a capture.
//!
//! Replaying the register reads and writes of a [`Trace`] gives the chip
//! state at any point. The interesting points are the *executions*: the
//! moment the driver sets bit 7 of `E800` (`RTS_Execute` in `hp3900`) and the
//! chip starts moving the carriage and sampling. The register block at that
//! moment is what a translated `RTS_Setup` has to reproduce.

use crate::trace::{Event, Trace};
use hp3770_core::protocol::Command;

/// First register of the main block.
pub const BLOCK_START: u16 = 0xe800;
/// Length of the main block as the HP driver writes it.
pub const BLOCK_LEN: usize = 0x071a;

/// Bit of `E800` that starts an operation.
const EXECUTE: u8 = 0x80;

/// The 64 KiB register space, with unknown bytes left as `None`.
#[derive(Clone, PartialEq, Eq)]
pub struct RegisterFile {
    values: Vec<Option<u8>>,
}

impl std::fmt::Debug for RegisterFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let known = self.values.iter().filter(|v| v.is_some()).count();
        f.debug_struct("RegisterFile")
            .field("known", &known)
            .finish_non_exhaustive()
    }
}

impl Default for RegisterFile {
    fn default() -> Self {
        Self {
            values: vec![None; 0x1_0000],
        }
    }
}

impl RegisterFile {
    /// Value of one register, if the capture showed it.
    pub fn get(&self, address: u16) -> Option<u8> {
        self.values[usize::from(address)]
    }

    /// `len` registers from `start`, or `None` if any of them is unknown.
    pub fn block(&self, start: u16, len: usize) -> Option<Vec<u8>> {
        let start = usize::from(start);
        self.values
            .get(start..start + len)?
            .iter()
            .copied()
            .collect()
    }

    /// Records `data` at `address` (from a write, or from a read answer).
    pub fn set(&mut self, address: u16, data: &[u8]) {
        let start = usize::from(address);
        let end = (start + data.len()).min(self.values.len());
        for (slot, &b) in self.values[start..end].iter_mut().zip(data) {
            *slot = Some(b);
        }
    }

    /// Updates the state with one event. Returns `true` if the event starts
    /// an execution.
    pub fn apply(&mut self, event: &Event) -> bool {
        let Event::Control {
            command, response, ..
        } = event
        else {
            return false;
        };
        match command {
            Command::ReadRegisters { address, .. } => {
                self.set(*address, response);
                false
            }
            Command::WriteRegisters { address, data } => {
                let before = self.get(BLOCK_START).unwrap_or(0);
                self.set(*address, data);
                let after = self.get(BLOCK_START).unwrap_or(0);
                *address <= BLOCK_START
                    && usize::from(BLOCK_START - address) < data.len()
                    && after & EXECUTE != 0
                    && before & EXECUTE == 0
            }
            _ => false,
        }
    }
}

/// The chip state at the start of one execution.
#[derive(Debug, Clone, PartialEq)]
pub struct Execution {
    /// Index of the starting write in [`Trace::events`].
    pub event: usize,
    /// Time of the starting write, seconds from the start of the capture.
    pub time: f64,
    /// The main register block (`E800`, [`BLOCK_LEN`] bytes) at that moment,
    /// with the execute bit set. `None` where the capture never showed a
    /// register.
    pub block: Vec<Option<u8>>,
}

impl Execution {
    /// The block as bytes, if every register is known.
    pub fn bytes(&self) -> Option<Vec<u8>> {
        self.block.iter().copied().collect()
    }

    /// Registers that differ from `other`, as `(address, self, other)`.
    pub fn diff(&self, other: &Execution) -> Vec<(u16, Option<u8>, Option<u8>)> {
        (BLOCK_START..)
            .zip(self.block.iter().zip(&other.block))
            .filter(|(_, (a, b))| a != b)
            .map(|(addr, (a, b))| (addr, *a, *b))
            .collect()
    }
}

impl Trace {
    /// Every execution in the capture, with the register block it ran with.
    pub fn executions(&self) -> Vec<Execution> {
        let mut regs = RegisterFile::default();
        let mut out = Vec::new();
        for (i, event) in self.events.iter().enumerate() {
            if regs.apply(event) {
                let start = usize::from(BLOCK_START);
                out.push(Execution {
                    event: i,
                    time: event.time(),
                    block: regs.values[start..start + BLOCK_LEN].to_vec(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(address: u16, data: &[u8]) -> Event {
        Event::Control {
            time: 0.0,
            command: Command::WriteRegisters {
                address,
                data: data.to_vec(),
            },
            response: Vec::new(),
            status: 0,
        }
    }

    fn read(address: u16, response: &[u8]) -> Event {
        Event::Control {
            time: 0.0,
            command: Command::ReadRegisters {
                address,
                length: response.len() as u16,
            },
            response: response.to_vec(),
            status: 0,
        }
    }

    #[test]
    fn execution_is_the_rising_edge_of_bit_7() {
        let mut block = vec![0u8; BLOCK_LEN];
        block[0] = 0x04;
        block[1] = 0x40;
        let trace = Trace {
            events: vec![
                write(BLOCK_START, &block),
                read(0xe801, &[0x40, 0x00]),
                write(BLOCK_START, &[0x84, 0x40]),
                // Still executing: not a new execution.
                write(BLOCK_START, &[0x84, 0x40]),
                write(BLOCK_START, &[0x04, 0x40]),
                write(0xe813, &[0xff, 0x44]),
                write(BLOCK_START, &[0x84, 0x40]),
            ],
            incomplete: 0,
        };
        let ex = trace.executions();
        assert_eq!(ex.iter().map(|e| e.event).collect::<Vec<_>>(), [2, 6]);
        assert_eq!(ex[0].bytes().unwrap()[0], 0x84);
        assert_eq!(
            ex[1].diff(&ex[0]),
            [
                (0xe813, Some(0xff), Some(0x00)),
                (0xe814, Some(0x44), Some(0x00))
            ]
        );
    }

    #[test]
    fn unknown_registers_stay_unknown() {
        let mut regs = RegisterFile::default();
        regs.apply(&read(0xfe3e, &[0x10, 0x00]));
        assert_eq!(regs.get(0xfe3e), Some(0x10));
        assert_eq!(regs.get(0xfe40), None);
        assert_eq!(regs.block(0xfe3e, 2), Some(vec![0x10, 0x00]));
        assert_eq!(regs.block(0xfe3e, 3), None);
    }
}
