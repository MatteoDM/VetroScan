// SPDX-License-Identifier: GPL-2.0-or-later
//! The thread that owns the scanner.
//!
//! Scanner calls block, and a scan takes from seconds to minutes: every
//! request goes through one thread, which the interface talks to with
//! [`Command`]s and hears from through [`Event`]s (an iced subscription).
//! When idle the thread reads the front-panel buttons every 150 ms.
//!
//! The thread only knows the scanner contract ([`scanner_api::Scanner`]):
//! it opens what the [`Registry`] finds, and does with the lamp and the
//! buttons only what the scanner's capabilities say it has.

use crate::i18n::tr;
use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, Stream};
use scanner_api::{
    Capabilities, DeviceInfo, DeviceStatus, Progress, Registry, ScanRequest, ScanResult, Scanner,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc};
use std::time::{Duration, Instant};

/// What the interface asks for.
#[derive(Debug, Clone)]
pub enum Command {
    /// Read the scanner state (opening it if needed).
    Status,
    /// Use this scanner from now on (one of [`Event::Opened`]'s list).
    Choose(DeviceInfo),
    /// Make a scan.
    Scan(Purpose, ScanRequest),
    /// Switch the lamp off after this long without scanning (scanners
    /// with a lamp only).
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
    /// A scanner was opened: which one, what it can do, and all the
    /// scanners there are now (to choose from when more than one).
    Opened(DeviceInfo, Box<Capabilities>, Vec<DeviceInfo>),
    /// The scanner state, or why it could not be read.
    Status(Result<DeviceStatus, String>),
    /// A scan moved on.
    Progress(Progress),
    /// A scan ended.
    Done(Purpose, Result<Arc<ScanResult>, String>),
    /// A front-panel button was pressed: its index in the capabilities.
    Button(usize),
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

/// The scanners the app looks for: on USB, the RTS8822 driver.
pub fn registry() -> Registry {
    if std::env::var_os("HP3770_FAKE_SCANNER").is_some() {
        // A made-up scanner, to try the interface with no scanner.
        return Registry::new().with(scanner_api::fake::FakeBackend::new(vec![
            scanner_api::fake::FakeScanner::flatbed().paced(Duration::from_millis(400)),
        ]));
    }
    Registry::new().with(hp3770_usb::UsbBackend)
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
        let _ = SinkExt::send(&mut output, Event::Ready(handle)).await;
        let sender = output.clone();
        std::thread::Builder::new()
            .name("scanner".into())
            .spawn(move || Worker::new(registry(), Box::new(sender), cancel).run(&receiver))
            .expect("scanner thread");
        // Keep the stream alive for as long as the application runs.
        std::future::pending::<()>().await;
    })
}

/// Where the thread's events go.
pub trait Output: Send {
    /// Sends an event, waiting if the interface is behind.
    fn send(&mut self, event: Event);
    /// Sends an event if there is room now; drops it otherwise (the
    /// progress of a scan, which must never wait).
    fn try_send(&mut self, event: Event);
}

impl Output for mpsc::Sender<Event> {
    fn send(&mut self, event: Event) {
        let _ = iced::futures::executor::block_on(SinkExt::send(self, event));
    }

    fn try_send(&mut self, event: Event) {
        let _ = mpsc::Sender::try_send(self, event);
    }
}

impl Output for std_mpsc::Sender<Event> {
    fn send(&mut self, event: Event) {
        let _ = std_mpsc::Sender::send(self, event);
    }

    fn try_send(&mut self, event: Event) {
        let _ = std_mpsc::Sender::send(self, event);
    }
}

/// What a scan stopped with "Annulla" ends with, instead of an error.
pub const CANCELLED: &str = "annullata";

/// The text of a failed call for the interface: [`CANCELLED`] when it was
/// the user who stopped the scan.
fn error_text(e: scanner_api::Error) -> String {
    match e {
        scanner_api::Error::Cancelled => CANCELLED.into(),
        e => e.to_string(),
    }
}

/// How often the idle thread reads the buttons.
const BUTTON_POLL: Duration = Duration::from_millis(150);

struct Worker {
    registry: Registry,
    output: Box<dyn Output>,
    cancel: Arc<AtomicBool>,
    scanner: Option<Box<dyn Scanner>>,
    /// The scanner chosen in the interface (none: the first found).
    chosen: Option<DeviceInfo>,
    /// Buttons held at the last poll.
    held: u32,
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
    fn new(registry: Registry, output: Box<dyn Output>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            registry,
            output,
            cancel,
            scanner: None,
            chosen: None,
            held: 0,
            last_open: None,
            lamp_used: Some(Instant::now()),
            // Until the app says: the default of the preferences.
            lamp_after: Duration::from_secs(120),
        }
    }

    /// Whether the open scanner has a lamp the host switches.
    fn has_lamp(&self) -> bool {
        self.scanner
            .as_ref()
            .is_some_and(|s| s.capabilities().lamp.is_some())
    }

    /// Switches the lamp off (if the scanner is open and has one), and
    /// tells the interface the new state.
    fn lamp_off(&mut self) {
        if !self.has_lamp() {
            self.lamp_used = None;
            return;
        }
        let Some(scanner) = self.scanner.as_mut() else {
            return;
        };
        // Not again every 150 ms if it fails: the scanner's own timer will.
        self.lamp_used = None;
        match scanner.lamp_off() {
            Ok(()) => {
                log("lampada spenta");
                let status = scanner.status().map_err(error_text);
                self.output.send(Event::Status(status));
            }
            Err(e) => log(&format!("lampada non spenta: {e}")),
        }
    }

    /// The open scanner, opened if needed: the one chosen, or the first.
    fn scanner(&mut self) -> Result<&mut Box<dyn Scanner>, String> {
        if self.scanner.is_none() {
            // For trying the interface without touching the scanner (for
            // instance while another program uses it).
            if std::env::var_os("HP3770_NO_SCANNER").is_some() {
                return Err(tr("scanner disattivato (HP3770_NO_SCANNER)").into());
            }
            let devices = self.registry.list();
            let first = self
                .chosen
                .as_ref()
                .and_then(|c| devices.iter().find(|d| *d == c))
                .or(devices.first())
                .cloned();
            let Some(device) = first else {
                // Nothing listed: open as the backends would anyway, for
                // their own words on why not.
                return Err(match self.registry.open_first() {
                    Ok(_) => tr("scanner non elencato").into(),
                    Err(e) => error_text(e),
                });
            };
            let scanner = self.registry.open(&device).map_err(error_text)?;
            let caps = Box::new(scanner.capabilities().clone());
            self.output.send(Event::Opened(device, caps, devices));
            self.scanner = Some(scanner);
        }
        Ok(self.scanner.as_mut().expect("just opened"))
    }

    fn status(&mut self) -> Result<DeviceStatus, String> {
        self.scanner().and_then(|s| s.status().map_err(error_text))
    }

    /// Reads the state for the interface; a scanner that does not answer
    /// is opened again next time.
    fn send_status(&mut self) {
        let status = self.status();
        if status.is_err() {
            self.scanner = None;
        }
        self.output.send(Event::Status(status));
    }

    fn run(mut self, commands: &std_mpsc::Receiver<Command>) {
        loop {
            match commands.recv_timeout(BUTTON_POLL) {
                Ok(Command::Status) => self.send_status(),
                Ok(Command::Choose(device)) => {
                    self.chosen = Some(device);
                    self.scanner = None;
                    self.held = 0;
                    self.send_status();
                }
                Ok(Command::Scan(purpose, request)) => {
                    let result = self.scan(&request);
                    if result.as_ref().is_err_and(|e| e != CANCELLED) {
                        // Open the scanner again next time.
                        self.scanner = None;
                    }
                    self.output.send(Event::Done(purpose, result.map(Arc::new)));
                    // Presses during the scan are not taken.
                    self.held = u32::MAX;
                    if self.has_lamp() {
                        self.lamp_used = Some(Instant::now());
                    }
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

    fn scan(&mut self, request: &ScanRequest) -> Result<ScanResult, String> {
        let cancel = self.cancel.clone();
        self.scanner()?;
        let mut scanner = self.scanner.take().expect("just opened");
        // Never wait for the interface while the image comes in: if the
        // scanner's buffer fills, the carriage stops and goes back, and the
        // line read across the stop comes out brighter. Only changes of
        // the percentage are sent, and dropped if the queue is full.
        let mut last = None;
        let output = &mut self.output;
        let result = scanner
            .scan(request, &mut |progress| {
                let key = match progress {
                    Progress::Scanning { done, total } => done * 100 / total.max(1),
                    _ => usize::MAX,
                };
                if last != Some(key) || key == usize::MAX {
                    last = Some(key);
                    output.try_send(Event::Progress(progress));
                }
                !cancel.load(Ordering::SeqCst)
            })
            .map_err(error_text);
        self.scanner = Some(scanner);
        result
    }

    /// Reads the buttons of a scanner that has some. If the scanner went
    /// away (unplugged, switched off, taken for a moment by another
    /// program) it is opened again, at most every 300 ms, so the buttons
    /// never stop working.
    fn poll_buttons(&mut self) {
        if self.scanner.is_none() {
            if self
                .last_open
                .is_some_and(|t| t.elapsed() < Duration::from_millis(300))
            {
                return;
            }
            self.last_open = Some(Instant::now());
            if let Err(e) = self.scanner() {
                log(&format!("scanner non aperto: {e}"));
                return;
            }
            log("scanner riaperto");
            let status = self.status();
            self.output.send(Event::Status(status));
            // A button still held when the scanner comes back is the press
            // that made it go away (Windows reacting to the same press).
            self.held = 0;
        }
        let Some(scanner) = self.scanner.as_mut() else {
            return;
        };
        if scanner.capabilities().buttons.is_empty() {
            return;
        }
        let held = match scanner.poll_buttons() {
            Ok(held) => held,
            Err(e) => {
                log(&format!("lettura tasti non riuscita: {e}"));
                self.scanner = None;
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
        let index = pressed.trailing_zeros() as usize;
        let label = scanner
            .capabilities()
            .buttons
            .get(index)
            .map(|b| b.label.clone());
        if let Some(label) = label {
            log(&format!("tasto {label}"));
            self.output.send(Event::Button(index));
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

/// The made-up scanners for the tests of the interface: the real thread,
/// on [`scanner_api::fake`] scanners.
#[cfg(test)]
pub mod fake {
    use super::*;
    use scanner_api::fake::{FakeBackend, FakeScanner};

    /// The thread with these made-up scanners (the first is opened).
    pub fn spawn_with(scanners: Vec<FakeScanner>) -> (Handle, std_mpsc::Receiver<Event>) {
        let (commands, receiver) = std_mpsc::channel();
        let (events, out) = std_mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = Handle {
            commands,
            cancel: cancel.clone(),
        };
        let registry = Registry::new().with(FakeBackend::new(scanners));
        std::thread::spawn(move || {
            Worker::new(registry, Box::new(events), cancel).run(&receiver);
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
        assert_eq!(error_text(scanner_api::Error::Cancelled), CANCELLED);
        assert_ne!(
            error_text(scanner_api::Error::Disconnected("gone".into())),
            CANCELLED
        );
    }

    fn next(events: &std_mpsc::Receiver<Event>) -> Event {
        events
            .recv_timeout(Duration::from_secs(10))
            .expect("an event")
    }

    #[test]
    fn a_press_comes_once_and_only_for_buttons_there_are() {
        use scanner_api::fake::FakeScanner;
        let scanner = FakeScanner::a3();
        let controls = scanner.controls();
        let (handle, events) = fake::spawn_with(vec![scanner]);
        handle.send(Command::Status);
        assert!(matches!(next(&events), Event::Opened(..)));
        assert!(matches!(next(&events), Event::Status(Ok(_))));
        controls.hold(1, true);
        assert!(matches!(next(&events), Event::Button(1)));
        // Held down: not again.
        assert!(events.recv_timeout(Duration::from_millis(500)).is_err());
        controls.hold(1, false);
        // A bit beyond its two buttons is never a press.
        controls.hold(4, true);
        assert!(events.recv_timeout(Duration::from_millis(500)).is_err());
    }

    #[test]
    fn a_scanner_without_a_lamp_is_never_asked_to_switch_it_off() {
        use scanner_api::fake::FakeScanner;
        let (handle, events) = fake::spawn_with(vec![FakeScanner::a3()]);
        handle.send(Command::Status);
        let _ = next(&events);
        let _ = next(&events);
        let (done, wait) = std_mpsc::channel();
        handle.send(Command::LampOff(Some(done)));
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        // No status after it: nothing was switched.
        assert!(events.recv_timeout(Duration::from_millis(400)).is_err());
    }

    #[test]
    fn the_scanner_chosen_is_the_one_opened() {
        use scanner_api::fake::FakeScanner;
        let (handle, events) = fake::spawn_with(vec![FakeScanner::flatbed(), FakeScanner::a3()]);
        handle.send(Command::Status);
        let Event::Opened(first, _, all) = next(&events) else {
            panic!("opened");
        };
        assert_eq!(first.name, "Prova Piano A4");
        assert_eq!(all.len(), 2);
        let _ = next(&events);
        handle.send(Command::Choose(all[1].clone()));
        let Event::Opened(second, caps, _) = next(&events) else {
            panic!("opened");
        };
        assert_eq!(second.name, "Prova Piano A3");
        assert_eq!(caps.max_dpi(), 1200);
    }
}
