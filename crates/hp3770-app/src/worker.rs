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
    /// Switch the lamp off after this long without scanning (the chip's
    /// own timer, 13 minutes, stays as a safety net).
    LampTimer(Duration),
    /// Switch the lamp off now (the app is closing); says when done.
    LampOff(Option<std_mpsc::Sender<()>>),
}

/// "Appena finita la scansione": a short pause all the same, so that the
/// passes of one scan (several originals, one after the other) do not each
/// wait for the lamp to warm up again.
pub const LAMP_SOON: Duration = Duration::from_secs(15);

/// Whether a lamp last used at `used` (none: off) is due to go off after
/// `after` without scanning.
pub fn lamp_due(used: Option<Instant>, after: Duration, now: Instant) -> bool {
    used.is_some_and(|t| now.saturating_duration_since(t) >= after.max(LAMP_SOON))
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
        let mut sender = output.clone();
        let thread = std::thread::Builder::new().name("scanner".into());
        if std::env::var_os("HP3770_FAKE_SCANNER").is_some() {
            // A made-up scanner, to try the interface with no scanner.
            thread.spawn(move || {
                fake::run(&receiver, &cancel, Duration::from_millis(400), |event| {
                    let _ = iced::futures::executor::block_on(sender.send(event));
                });
            })
        } else {
            thread.spawn(move || Worker::new(sender, cancel).run(receiver))
        }
        .expect("scanner thread");
        // Keep the stream alive for as long as the application runs.
        std::future::pending::<()>().await;
    })
}

/// What a scan stopped with "Annulla" ends with, instead of an error.
pub const CANCELLED: &str = "annullata";

/// The text of a failed scan for the interface: [`CANCELLED`] when it was
/// the user who stopped it.
fn error_text(e: hp3770_core::Error) -> String {
    match e {
        hp3770_core::Error::Cancelled(_) => CANCELLED.into(),
        e => e.to_string(),
    }
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
    /// When the lamp was last needed (none: switched off since); it may be
    /// on when the app starts.
    lamp_used: Option<Instant>,
    /// How long without scanning before the lamp goes off.
    lamp_after: Duration,
}

impl Worker {
    fn new(output: mpsc::Sender<Event>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            output,
            cancel,
            chip: None,
            held: 0,
            last_open: None,
            lamp_used: Some(Instant::now()),
            // Until the app says: the default of the preferences.
            lamp_after: Duration::from_secs(120),
        }
    }

    /// Switches the flatbed lamp off (if the scanner is open), and tells
    /// the interface the new state.
    fn lamp_off(&mut self) {
        let Some(chip) = self.chip.as_mut() else {
            return;
        };
        // lamp_flatbed reads the chip's block; the copy kept by a scan is
        // not needed to switch off.
        let mut init = hp3770_core::init::Registers::default();
        match chip.lamp_flatbed(&mut init, false) {
            Ok(()) => {
                self.lamp_used = None;
                log("lampada spenta");
                let status = chip.status().map_err(|e| e.to_string());
                self.emit(Event::Status(status));
            }
            Err(e) => {
                // Not again every 150 ms: the chip's own timer will.
                self.lamp_used = None;
                log(&format!("lampada non spenta: {e}"));
            }
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
                    self.lamp_used = Some(Instant::now());
                }
                Ok(Command::LampTimer(after)) => self.lamp_after = after,
                Ok(Command::LampOff(done)) => {
                    self.lamp_off();
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => {
                    self.poll_buttons();
                    if lamp_due(self.lamp_used, self.lamp_after, Instant::now()) {
                        self.lamp_off();
                    }
                }
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
        .map_err(error_text)
    }

    /// Reads the buttons. If the scanner went away (unplugged, switched
    /// off, taken for a moment by another program) it is opened again,
    /// at most every 300 ms, so the buttons never stop working.
    fn poll_buttons(&mut self) {
        if self.chip.is_none() {
            if self
                .last_open
                .is_some_and(|t| t.elapsed() < Duration::from_millis(300))
            {
                return;
            }
            self.last_open = Some(Instant::now());
            if let Err(e) = self.chip() {
                log(&format!("scanner non aperto: {e}"));
                return;
            }
            log("scanner riaperto");
            let status = self
                .chip()
                .and_then(|c| c.status().map_err(|e| e.to_string()));
            self.emit(Event::Status(status));
            // A button still held when the scanner comes back is the press
            // that made it go away (Windows reacting to the same press).
            self.held = 0;
        }
        let Some(chip) = self.chip.as_mut() else {
            return;
        };
        let held = match chip.buttons_held() {
            Ok(held) => held,
            Err(e) => {
                log(&format!("lettura tasti non riuscita: {e}"));
                self.chip = None;
                return;
            }
        };
        let pressed = held & !self.held;
        self.held = held;
        if pressed == 0 {
            return;
        }
        // One button at a time: several at once is a bad read.
        if pressed.count_ones() != 1 {
            log(&format!("lettura scartata: {pressed:02x}"));
            return;
        }
        for button in [Button::Scan, Button::Copy, Button::InstantShare] {
            if pressed & button.mask() != 0 {
                log(&format!("tasto {}", button.name()));
                self.emit(Event::Button(button));
            }
        }
    }
}

/// Appends a line to the log of the buttons (`tasti.log` next to the
/// settings), to tell what happened when a press seems lost.
fn log(line: &str) {
    use std::io::Write;
    let Some(dir) = crate::prefs::config_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("tasti.log");
    // Keep it small: start again past 256 KB.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        let _ = writeln!(f, "{now:.3} {line}");
    }
}

/// A scanner made up in memory, for the tests of the interface and for
/// trying it with no scanner (`HP3770_FAKE_SCANNER=1`): a white lid with
/// three originals on the glass, scanned at any resolution and in any mode.
pub mod fake {
    use super::*;
    use hp3770_core::geometry::Area;
    use hp3770_core::session::{Format, Mode};
    use hp3770_core::status::Lamp;

    /// The originals on the glass: left, top, width, height (inches) and
    /// the colour they are printed in.
    pub const ORIGINALS: [([f64; 4], [u8; 3]); 3] = [
        ([0.8, 0.8, 4.0, 2.8], [190, 90, 40]),
        ([1.0, 5.0, 3.0, 3.0], [40, 90, 170]),
        ([5.3, 4.5, 2.5, 4.0], [60, 140, 70]),
    ];

    /// The colour of the glass at a point (inches from the top left).
    pub fn glass(x: f64, y: f64) -> [u8; 3] {
        for ([left, top, width, height], colour) in ORIGINALS {
            let (u, v) = ((x - left) / width, (y - top) / height);
            if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                // Shaded across, with stripes: something for the
                // adjustments to change and the crop to find.
                let stripe = if ((u * 12.0) as i32 + (v * 8.0) as i32) % 2 == 0 {
                    1.0
                } else {
                    0.8
                };
                let shade = (0.6 + 0.4 * u) * stripe;
                return colour.map(|c| (f64::from(c) * shade).round() as u8);
            }
        }
        [236, 236, 232]
    }

    /// The scanner state of the made-up scanner, its lamp on or off.
    pub fn status(lamp_on: bool) -> Status {
        Status {
            usb2: true,
            chipset_id: 6,
            init_done: true,
            executing: false,
            head_at_home: true,
            lamp: if lamp_on { Lamp::Flatbed } else { Lamp::Off },
            tma_attached: false,
            buttons_held: 0,
            link_word: 1024,
        }
    }

    /// The scan of `request`, as the real scanner would give it back.
    pub fn scan(request: &Request) -> (Image, Report) {
        let Area {
            x,
            y,
            width,
            height,
        } = request.area;
        let (width, height) = (width as usize, height as usize);
        let dpi = f64::from(request.dpi);
        let mut rgb = Vec::with_capacity(width * height * 3);
        for row in 0..height {
            let gy = (f64::from(y) + row as f64 + 0.5) / dpi;
            for col in 0..width {
                let gx = (f64::from(x) + col as f64 + 0.5) / dpi;
                rgb.extend_from_slice(&glass(gx, gy));
            }
        }
        let grey = |p: &[u8]| {
            ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000) as u8
        };
        let (format, data) = match request.mode {
            Mode::Colour => (Format::Rgb, rgb.clone()),
            Mode::Grey => (Format::Grey, rgb.chunks_exact(3).map(grey).collect()),
            Mode::Lineart { threshold } => {
                let stride = width.div_ceil(8);
                let mut data = vec![0u8; stride * height];
                for (i, p) in rgb.chunks_exact(3).enumerate() {
                    if grey(p) >= 255 - threshold {
                        let (row, col) = (i / width, i % width);
                        data[row * stride + col / 8] |= 0x80 >> (col % 8);
                    }
                }
                (Format::Lineart, data)
            }
        };
        let samples16 = (request.depth == 16 && request.mode == Mode::Colour)
            .then(|| rgb.iter().map(|&v| u16::from(v) * 257).collect());
        (
            Image {
                width,
                height,
                dpi: request.dpi,
                format,
                data,
            },
            Report {
                warmed_up: false,
                gains: [16, 14, 21],
                offsets: [300, 300, 310],
                peak_buffered: 0,
                white: [Vec::new(), Vec::new(), Vec::new()],
                samples16,
            },
        )
    }

    /// Answers the interface's commands like the scanner thread, taking
    /// `pace` for each stage of a scan (so that "Annulla" can stop it).
    pub fn run(
        commands: &std_mpsc::Receiver<Command>,
        cancel: &AtomicBool,
        pace: Duration,
        mut emit: impl FnMut(Event),
    ) {
        // The lamp as the real thread handles it: on after a scan, off after
        // the time set without scanning, or when asked.
        let mut lamp_used = Some(Instant::now());
        let mut lamp_after = Duration::from_secs(120);
        loop {
            let command = match commands.recv_timeout(Duration::from_millis(50)) {
                Ok(command) => command,
                Err(std_mpsc::RecvTimeoutError::Timeout) => {
                    if lamp_due(lamp_used, lamp_after, Instant::now()) {
                        lamp_used = None;
                        emit(Event::Status(Ok(status(false))));
                    }
                    continue;
                }
                Err(std_mpsc::RecvTimeoutError::Disconnected) => return,
            };
            match command {
                Command::Status => emit(Event::Status(Ok(status(lamp_used.is_some())))),
                Command::LampTimer(after) => lamp_after = after,
                Command::LampOff(done) => {
                    lamp_used = None;
                    emit(Event::Status(Ok(status(false))));
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                }
                Command::Scan(purpose, request) => {
                    let stages = [
                        Stage::Starting,
                        Stage::Calibrating,
                        Stage::Scanning { done: 1, total: 2 },
                        Stage::Finishing,
                    ];
                    let mut cancelled = false;
                    for stage in stages {
                        emit(Event::Progress(stage));
                        std::thread::sleep(pace);
                        if cancel.load(Ordering::SeqCst) {
                            cancelled = true;
                            break;
                        }
                    }
                    let result = if cancelled {
                        Err(CANCELLED.to_string())
                    } else {
                        Ok(Arc::new(scan(&request)))
                    };
                    lamp_used = Some(Instant::now());
                    emit(Event::Done(purpose, result));
                }
            }
        }
    }

    /// A made-up scanner on a thread of its own, for the tests: the handle
    /// the interface sends to, and where its events come out.
    #[cfg(test)]
    pub fn spawn(pace: Duration) -> (Handle, std_mpsc::Receiver<Event>) {
        let (commands, receiver) = std_mpsc::channel();
        let (events, out) = std_mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = Handle {
            commands,
            cancel: cancel.clone(),
        };
        std::thread::spawn(move || {
            run(&receiver, &cancel, pace, |e| {
                let _ = events.send(e);
            });
        });
        (handle, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lamp_goes_off_after_the_time_chosen_never_mid_job() {
        let now = Instant::now();
        let ago = |s: u64| Some(now - Duration::from_secs(s));
        let two = Duration::from_secs(120);
        assert!(!lamp_due(ago(60), two, now));
        assert!(lamp_due(ago(121), two, now));
        // Already off: nothing to do.
        assert!(!lamp_due(None, two, now));
        // "As soon as the scan is over" still lets the passes of one scan
        // follow each other with the lamp on.
        assert!(!lamp_due(ago(5), Duration::ZERO, now));
        assert!(lamp_due(ago(16), Duration::ZERO, now));
    }

    #[test]
    fn a_scan_stopped_by_the_user_is_not_an_error() {
        assert_eq!(error_text(hp3770_core::Error::Cancelled("scan")), CANCELLED);
        assert_ne!(
            error_text(hp3770_core::Error::Transport("gone".into())),
            CANCELLED
        );
    }
}
