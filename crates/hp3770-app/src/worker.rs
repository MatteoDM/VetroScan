// SPDX-License-Identifier: GPL-2.0-or-later
//! The thread that owns the scanner.
//!
//! USB requests block, and a scan takes from seconds to minutes: every
//! request goes through one thread, which the interface talks to with
//! [`Command`]s and hears from through [`Event`]s (an iced subscription).
//! When idle the thread reads the front-panel buttons every 150 ms.

use hp3770_core::device::Button;
use hp3770_core::profile::HP3770;
use hp3770_core::session::{self, Image, Report, Request, Stage};
use hp3770_core::status::Status;
use hp3770_core::transport::Chip;
use hp3770_usb::Scanner;
use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, Stream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc};
use std::time::{Duration, Instant};

/// What the interface asks for.
#[derive(Debug, Clone)]
pub enum Command {
    /// Read the scanner state.
    Status,
    /// Make a scan.
    Scan(Purpose, Box<Request>),
}

/// Why a scan is made: the interface puts the image in different places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// The preview of the whole glass.
    Preview,
    /// A page of the document.
    Page,
}

/// What the thread tells the interface.
#[derive(Debug, Clone)]
pub enum Event {
    /// The thread is running: where to send commands, and the flag that
    /// cancels the running scan.
    Ready(Handle),
    /// The scanner state, or why it could not be read.
    Status(Result<Status, String>),
    /// A scan moved on.
    Progress(Stage),
    /// A scan ended.
    Done(Purpose, Result<Arc<(Image, Report)>, String>),
    /// A front-panel button was pressed.
    Button(Button),
}

/// How the interface reaches the thread.
#[derive(Debug, Clone)]
pub struct Handle {
    commands: std_mpsc::Sender<Command>,
    cancel: Arc<AtomicBool>,
}

impl Handle {
    /// Sends a command (ignored if the thread has stopped).
    pub fn send(&self, command: Command) {
        if matches!(command, Command::Scan(..)) {
            self.cancel.store(false, Ordering::SeqCst);
        }
        let _ = self.commands.send(command);
    }

    /// Cancels the running scan, if any.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// The subscription stream of the thread's events.
pub fn events() -> impl Stream<Item = Event> {
    iced::stream::channel(64, async |mut output: mpsc::Sender<Event>| {
        let (commands, receiver) = std_mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = Handle {
            commands,
            cancel: cancel.clone(),
        };
        let _ = output.send(Event::Ready(handle)).await;
        let sender = output.clone();
        std::thread::Builder::new()
            .name("scanner".into())
            .spawn(move || Worker::new(sender, cancel).run(receiver))
            .expect("scanner thread");
        // Keep the stream alive for as long as the application runs.
        std::future::pending::<()>().await;
    })
}

/// How often the idle thread reads the buttons.
const BUTTON_POLL: Duration = Duration::from_millis(150);

struct Worker {
    output: mpsc::Sender<Event>,
    cancel: Arc<AtomicBool>,
    chip: Option<Chip<Scanner>>,
    /// Buttons held at the last poll.
    held: u8,
    /// When the scanner was last opened for the buttons (to retry slowly
    /// after it went away).
    last_open: Option<Instant>,
}

impl Worker {
    fn new(output: mpsc::Sender<Event>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            output,
            cancel,
            chip: None,
            held: 0,
            last_open: None,
        }
    }

    fn emit(&mut self, event: Event) {
        let _ = iced::futures::executor::block_on(self.output.send(event));
    }

    /// The open scanner, opened if needed.
    fn chip(&mut self) -> Result<&mut Chip<Scanner>, String> {
        if self.chip.is_none() {
            // For trying the interface without touching the scanner (for
            // instance while another program uses it).
            if std::env::var_os("HP3770_NO_SCANNER").is_some() {
                return Err("scanner disattivato (HP3770_NO_SCANNER)".into());
            }
            let scanner = hp3770_usb::open().map_err(|e| e.to_string())?;
            self.chip = Some(Chip::new(scanner));
        }
        Ok(self.chip.as_mut().expect("just opened"))
    }

    fn run(mut self, commands: std_mpsc::Receiver<Command>) {
        loop {
            match commands.recv_timeout(BUTTON_POLL) {
                Ok(Command::Status) => {
                    let status = self
                        .chip()
                        .and_then(|c| c.status().map_err(|e| e.to_string()));
                    if status.is_err() {
                        self.chip = None;
                    }
                    self.emit(Event::Status(status));
                }
                Ok(Command::Scan(purpose, request)) => {
                    let result = self.scan(&request);
                    if result.is_err() {
                        // Open the scanner again next time.
                        self.chip = None;
                    }
                    self.emit(Event::Done(purpose, result.map(Arc::new)));
                    // Presses during the scan are not taken.
                    self.held = 0xff;
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => self.poll_buttons(),
                Err(std_mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn scan(&mut self, request: &Request) -> Result<(Image, Report), String> {
        let cancel = self.cancel.clone();
        let mut output = self.output.clone();
        let chip = self.chip()?;
        // Never wait for the interface while the image comes in: if the
        // chip's buffer fills, the carriage stops and goes back, and the
        // line read across the stop comes out brighter. Only changes of
        // the percentage are sent, and dropped if the queue is full.
        let mut last = None;
        session::scan(chip, &HP3770, request, &mut |stage| {
            let key = match stage {
                Stage::Scanning { done, total } => done * 100 / total.max(1),
                _ => usize::MAX,
            };
            if last != Some(key) || key == usize::MAX {
                last = Some(key);
                let _ = output.try_send(Event::Progress(stage));
            }
            !cancel.load(Ordering::SeqCst)
        })
        .map_err(|e| e.to_string())
    }

    /// Reads the buttons. If the scanner went away (unplugged, switched
    /// off, taken for a moment by another program) it is opened again,
    /// every 2 seconds at most, so the buttons never stop working.
    fn poll_buttons(&mut self) {
        if self.chip.is_none() {
            if self
                .last_open
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
            {
                return;
            }
            self.last_open = Some(Instant::now());
            if self.chip().is_err() {
                return;
            }
            let status = self
                .chip()
                .and_then(|c| c.status().map_err(|e| e.to_string()));
            self.emit(Event::Status(status));
            // Not a press: whatever is held when the scanner comes back.
            self.held = 0xff;
        }
        let Some(chip) = self.chip.as_mut() else {
            return;
        };
        let Ok(held) = chip.buttons_held() else {
            self.chip = None;
            return;
        };
        let pressed = held & !self.held;
        self.held = held;
        // One button at a time: several at once is a bad read.
        if pressed.count_ones() != 1 {
            return;
        }
        for button in [Button::Scan, Button::Copy, Button::InstantShare] {
            if pressed & button.mask() != 0 {
                self.emit(Event::Button(button));
            }
        }
    }
}
