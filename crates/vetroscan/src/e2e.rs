// SPDX-License-Identifier: GPL-2.0-or-later
//! End-to-end tests of the application: the real `App`, its real view
//! clicked through `iced_test`'s simulator (no window), and the made-up
//! scanner of [`crate::worker::fake`] on its own thread. The tasks the app
//! returns are run, as the iced runtime would; what it does outside its
//! window (notifications, printing, the "Salva" dialog, the folder of the
//! scans) goes through [`crate::testing`].

use crate::testing;
use crate::worker::{fake, Event};
use crate::{App, Kind, Message, PEAK};
use iced::futures::StreamExt;
use iced::{Size, Task};
use scan_imaging::detect::Found;
use scanner_api::fake::FakeScanner;
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// The application under test.
struct Ui {
    app: App,
    events: mpsc::Receiver<Event>,
    /// The app closed its window.
    closed: bool,
}

impl Ui {
    /// The app as it starts, with the made-up scanner answering at once.
    fn new() -> Self {
        Self::with_pace(Duration::ZERO)
    }

    /// The made-up scanner taking `pace` for each stage of a scan.
    fn with_pace(pace: Duration) -> Self {
        Self::with_scanners(vec![FakeScanner::flatbed().paced(pace)])
    }

    /// The app with these made-up scanners (the first one in use).
    fn with_scanners(scanners: Vec<FakeScanner>) -> Self {
        let (app, _) = App::new();
        let (handle, events) = fake::spawn_with(scanners);
        let mut ui = Self {
            app,
            events,
            closed: false,
        };
        // The answer to what the app asks at the start (the monitor's
        // profile; its other task, the system's theme, never answers here).
        ui.send(Message::MonitorFound(testing::monitor()));
        ui.send(Message::Worker(Event::Ready(handle)));
        // The scanner state, asked for on Ready (however busy the machine).
        while ui.app.status.is_none() {
            let event = ui
                .events
                .recv_timeout(Duration::from_secs(20))
                .expect("the scanner state");
            ui.send(Message::Worker(event));
        }
        ui.settle();
        // Small scans: the tests are about the interface, not the pixels.
        for kind in [Kind::Document, Kind::Photo] {
            ui.send(Message::Kind(kind));
            ui.send(Message::Dpi(200));
        }
        ui.send(Message::Kind(Kind::Document));
        ui
    }

    /// Gives the app a message and runs what it returns.
    fn send(&mut self, message: Message) {
        let task = self.app.update(message);
        self.run(task);
    }

    fn run(&mut self, task: Task<Message>) {
        use iced_runtime::Action;
        let Some(stream) = iced_runtime::task::into_stream(task) else {
            return;
        };
        let actions: Vec<Action<Message>> = iced::futures::executor::block_on(stream.collect());
        for action in actions {
            match action {
                Action::Output(message) => self.send(message),
                Action::Window(iced_runtime::window::Action::Close(_)) => self.closed = true,
                _ => {}
            }
        }
    }

    /// Hands the app what the scanner sent until it has finished.
    fn settle(&mut self) {
        loop {
            // Long enough for a large made-up scan on a slow machine; only a
            // scan that really hangs waits that long.
            let wait = if self.app.busy.is_some() {
                Duration::from_secs(60)
            } else {
                Duration::from_millis(50)
            };
            match self.events.recv_timeout(wait) {
                Ok(event) => self.send(Message::Worker(event)),
                Err(_) => break,
            }
        }
        assert!(self.app.busy.is_none(), "the scan did not end");
    }

    /// The messages a click on the text `label` gives (it must be shown).
    fn click_messages(&mut self, label: &str) -> Vec<Message> {
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            Size::new(1360.0, 900.0),
            self.app.view(),
        );
        if let Err(e) = ui.click(label) {
            panic!("«{label}» is not on the screen: {e:?}");
        }
        ui.into_messages().collect()
    }

    /// The messages a click on each text `label` on the screen gives, in the order they are
    /// drawn (the same label on two buttons).
    fn click_each(&mut self, label: &str) -> Vec<Vec<Message>> {
        use iced_test::selector::{Candidate, Target};
        let mut all = Vec::new();
        for n in 1.. {
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                Size::new(1360.0, 900.0),
                self.app.view(),
            );
            let wanted = label.to_string();
            let mut seen = 0;
            let nth = move |c: Candidate<'_>| match c {
                Candidate::Text { content, .. } if content == wanted => {
                    seen += 1;
                    (seen == n).then(|| Target::from(c))
                }
                _ => None,
            };
            if ui.click(nth).is_err() {
                return all;
            }
            all.push(ui.into_messages().collect());
        }
        all
    }

    /// Clicks the text `label`, then lets any scan it started finish.
    fn click(&mut self, label: &str) {
        for message in self.click_messages(label) {
            self.send(message);
        }
        self.settle();
    }

    /// Whether the text is on the screen.
    fn shows(&mut self, label: &str) -> bool {
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            Size::new(1360.0, 900.0),
            self.app.view(),
        );
        ui.find(label).is_ok()
    }

    /// Hands the app what the scanner sent until its state says the
    /// adapter is `plugged` (or not): on a busy machine the answer to
    /// RefreshStatus can come later than `settle` waits.
    fn adapter_state(&mut self, plugged: bool) {
        while !matches!(&self.app.status, Some(Ok(s)) if s.adapter == Some(plugged)) {
            let event = self
                .events
                .recv_timeout(Duration::from_secs(20))
                .expect("the scanner state");
            self.send(Message::Worker(event));
        }
        self.settle();
    }

    /// Presses a key (as the keyboard subscription would deliver it).
    fn key(&mut self, key: iced::keyboard::key::Named) {
        let iced::Event::Keyboard(event) = iced_test::simulator::press_key(key, None) else {
            unreachable!("a key press is a keyboard event")
        };
        self.send(Message::Key(event));
    }

    /// The title of the question shown, if any.
    fn question(&self) -> Option<&str> {
        self.app.confirm.as_ref().map(|c| c.title.as_str())
    }

    /// Scans `n` pages (or photos) one after the other.
    fn scan(&mut self, n: usize) {
        for _ in 0..n {
            self.send(Message::Scan);
            self.settle();
        }
    }

    /// Hands the app what the scanner sends until the scan's result, given
    /// back unsent.
    fn next_done(&mut self) -> Event {
        loop {
            let event = self
                .events
                .recv_timeout(Duration::from_secs(60))
                .expect("the scan's result");
            if matches!(event, Event::Done(..)) {
                return event;
            }
            self.send(Message::Worker(event));
        }
    }

    /// The files in the folder of the scans.
    fn saved(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.app.folder())
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter(|e| e.metadata().is_ok_and(|m| m.len() > 0))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        testing::clean_up();
    }
}

/// A document of `n` pages scanned and not saved.
fn document(n: usize) -> Ui {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Document));
    ui.scan(n);
    assert_eq!(ui.app.pages.len(), n);
    assert!(ui.app.unsaved);
    ui
}

// ------------------------------------------------------------- start

#[test]
fn the_app_finds_the_scanner_when_it_starts() {
    let mut ui = Ui::new();
    assert!(matches!(ui.app.status, Some(Ok(_))));
    assert!(ui.shows("Anteprima"));
    assert!(
        ui.shows(concat!("v", env!("CARGO_PKG_VERSION"))),
        "the version"
    );
    assert!(ui.app.pages.is_empty());
}

#[test]
fn pages_add_up_in_a_document() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Document));
    ui.click("Scansiona pagina 1");
    ui.click("Scansiona pagina 2");
    assert_eq!(ui.app.pages.len(), 2);
    assert!(ui.shows("Scansiona pagina 3"));
    assert!(
        ui.app.note.contains("Pagina 2 acquisita"),
        "{}",
        ui.app.note
    );
}

/// The memory a scan takes counts what the scanner's driver declares it holds more at a
/// resolution (`Capabilities::scan_memory`): the made-up flatbed, as the HP 3770, twice at 2400
/// dpi; the same scanner declaring nothing, the image alone. At 1200 dpi both the same.
#[test]
fn a_scans_memory_counts_what_its_driver_declares() {
    use scanner_api::Scanner as _;
    let memory = |scanner: FakeScanner, dpi: u16| {
        let mut ui = Ui::with_scanners(vec![scanner]);
        ui.send(Message::Kind(Kind::Document));
        ui.send(Message::Dpi(dpi));
        assert_eq!(ui.app.dpi(), dpi);
        let m = ui.app.memory();
        std::mem::forget(ui);
        testing::clean_up();
        m
    };
    let declaring = FakeScanner::flatbed();
    assert_eq!(declaring.capabilities().scan_memory_at(2400), 2.0);
    let plain = || {
        FakeScanner::with(scanner_api::Capabilities {
            model: "Senza memoria dichiarata".into(),
            scan_memory: Vec::new(),
            ..FakeScanner::flatbed().capabilities().clone()
        })
    };
    let (with, without) = (memory(declaring, 2400), memory(plain(), 2400));
    assert!(without > 0.0);
    assert!((with / without - 2.0).abs() < 1e-9, "{with} {without}");
    let (with, without) = (memory(FakeScanner::flatbed(), 1200), memory(plain(), 1200));
    assert_eq!(with, without);
}

/// A scan that would need more than half the computer's memory at its height is refused, said
/// in the pane and on pressing Scansiona, with nothing sent; on a computer with enough memory,
/// or before its memory is known, the same scan goes.
#[test]
fn a_scan_too_large_for_the_computers_memory_is_refused() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Document));
    let need = ui.app.memory() * PEAK;
    // A computer with less than twice what the scan needs at its height.
    let small = (need * 1.5) as u64;
    testing::set_ram(Some(small));
    let said = ui.app.too_large_text(small);
    assert!(said.contains("troppo grande"), "{said}");
    assert!(ui.shows(&said));
    ui.click("Scansiona pagina 1");
    assert!(ui.app.pages.is_empty());
    assert_eq!(ui.app.note, said);
    // Enough memory: the same scan goes.
    testing::set_ram(Some((need * 2.5) as u64));
    assert_eq!(ui.app.too_large(), None);
    ui.click("Scansiona pagina 1");
    assert_eq!(ui.app.pages.len(), 1);
    assert!(!ui.shows(&said));
    // The memory not known: no limit.
    testing::set_ram(None);
    assert_eq!(ui.app.too_large(), None);
    ui.click("Scansiona pagina 2");
    assert_eq!(ui.app.pages.len(), 2);
}

// ------------------------------------- questions before losing scans

#[test]
fn each_way_of_working_keeps_its_own_pages() {
    let mut ui = document(3);
    let pages = scans(&ui);
    // Foto: nothing asked, nothing of the document there.
    ui.click("Foto");
    assert_eq!(ui.question(), None);
    assert_eq!(ui.app.kind, Kind::Photo);
    assert!(ui.app.pages.is_empty() && !ui.app.unsaved);
    ui.click("Anteprima");
    ui.click("Scansiona 3 foto");
    let photos = scans(&ui);
    assert_eq!(photos.len(), 3);
    // And back, as many times as wanted: each finds its own as it was.
    for _ in 0..2 {
        ui.click("Documento");
        assert_eq!(scans(&ui), pages);
        assert!(ui.app.unsaved);
        assert!(ui.shows("Scansiona pagina 4"));
        ui.click("Foto");
        assert_eq!(scans(&ui), photos);
        assert!(ui.app.unsaved);
    }
}

#[test]
fn photos_look_the_same_after_going_to_document_and_back() {
    use crate::Field;
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::Adjust(Field::Temperature, 40.0));
    ui.scan(1);
    ui.send(Message::Kind(Kind::Document));
    ui.send(Message::Kind(Kind::Photo));
    let shared = *ui.app.settings();
    assert_eq!(ui.app.pages[0].effective(&shared).adjust.temperature, 40.0);
    // Ctrl+Z still takes back what was done in Foto.
    ui.send(Message::Undo);
    assert_eq!(ui.app.settings().adjust.temperature, 0.0);
}

#[test]
fn escape_answers_no() {
    let mut ui = document(2);
    ui.send(Message::NewDocument);
    assert!(ui.question().is_some());
    ui.key(iced::keyboard::key::Named::Enter);
    assert!(ui.question().is_some(), "Enter must not throw pages away");
    ui.key(iced::keyboard::key::Named::Escape);
    assert_eq!(ui.question(), None);
    assert_eq!(ui.app.pages.len(), 2);
}

#[test]
fn salva_in_the_question_saves_then_goes_on() {
    let mut ui = document(2);
    ui.send(Message::NewDocument);
    ui.click("Salva…");
    assert_eq!(ui.saved().len(), 1, "{:?}", ui.saved());
    assert!(ui.app.pages.is_empty(), "the new document");
    assert!(!ui.app.unsaved);
}

#[test]
fn a_save_dialog_cancelled_changes_nothing() {
    let mut ui = document(2);
    testing::cancel_save_dialog(true);
    ui.send(Message::NewDocument);
    ui.click("Salva…");
    assert!(ui.saved().is_empty());
    assert_eq!(ui.app.pages.len(), 2);
    assert!(ui.app.unsaved);
}

#[test]
fn saved_pages_are_not_asked_about() {
    let mut ui = document(2);
    ui.send(Message::Save);
    assert!(!ui.app.unsaved);
    ui.send(Message::NewDocument);
    assert_eq!(ui.question(), None);
    assert!(ui.app.pages.is_empty());
}

#[test]
fn a_change_after_saving_is_asked_about_again() {
    let mut ui = document(2);
    ui.send(Message::Save);
    ui.send(Message::PageRotate(0, true));
    assert!(ui.app.unsaved);
    ui.send(Message::NewDocument);
    assert!(ui.question().is_some());
}

#[test]
fn new_document_asks_first() {
    let mut ui = document(2);
    ui.send(Message::NewDocument);
    assert_eq!(ui.question(), Some("Iniziare un nuovo documento?"));
    assert!(ui.shows("Le 2 pagine acquisite non sono salvate e andranno perse."));
    ui.click("Scarta e ricomincia");
    assert!(ui.app.pages.is_empty());
}

#[test]
fn deleting_an_unsaved_page_asks_first() {
    let mut ui = document(3);
    ui.send(Message::PageDelete(1));
    assert_eq!(ui.question(), Some("Eliminare la pagina 2?"));
    ui.click("Annulla");
    assert_eq!(ui.app.pages.len(), 3);
    ui.send(Message::PageDelete(1));
    ui.click("Elimina la pagina 2");
    assert_eq!(ui.app.pages.len(), 2);
}

#[test]
fn a_new_photo_asks_before_replacing_an_unsaved_one() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.scan(1);
    let first = ui.app.epoch;
    ui.send(Message::Scan);
    assert_eq!(ui.question(), Some("Fare una nuova scansione?"));
    assert!(ui.app.busy.is_none(), "nothing scans before the answer");
    ui.click("Scansiona");
    assert_eq!(ui.app.pages.len(), 1);
    assert_ne!(ui.app.epoch, first, "the photo was replaced");
}

#[test]
fn photos_saved_automatically_are_replaced_without_asking() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::AutoSave(true));
    ui.scan(1);
    assert_eq!(ui.saved().len(), 1);
    assert!(!ui.app.unsaved);
    ui.scan(1);
    assert_eq!(ui.question(), None);
    assert_eq!(ui.saved().len(), 2);
}

#[test]
fn a_scanner_button_with_a_question_pending_tells_the_person_at_the_scanner() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.scan(1);
    testing::take_effects();
    ui.send(Message::Worker(Event::Button(0)));
    assert_eq!(ui.question(), Some("Fare una nuova scansione?"));
    let effects = testing::take_effects();
    assert!(
        effects.contains(&"notifica: Fare una nuova scansione?".to_string()),
        "{effects:?}"
    );
    // A second press while the question is up does nothing more.
    ui.send(Message::Worker(Event::Button(0)));
    assert!(ui.app.busy.is_none());
}

#[test]
fn photocopy_answered_no_does_not_print_the_next_photo() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.scan(1);
    ui.send(Message::Worker(Event::Button(1)));
    if ui.question().is_none() {
        // Copy is not the photocopy in this setup: nothing to check.
        return;
    }
    ui.click("Annulla");
    testing::take_effects();
    ui.scan(1);
    ui.click("Scansiona");
    assert!(!testing::take_effects()
        .iter()
        .any(|e| e.starts_with("stampa")));
}

#[test]
fn closing_with_unsaved_pages_asks_and_closes_on_yes() {
    let mut ui = document(1);
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert_eq!(ui.question(), Some("Chiudere senza salvare?"));
    assert!(!ui.closed);
    ui.click("Chiudi senza salvare");
    assert!(ui.closed);
}

#[test]
fn closing_asks_about_the_work_of_every_way_of_working_and_saves_each() {
    let mut ui = document(1);
    ui.send(Message::Kind(Kind::Photo));
    ui.scan(1);
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert_eq!(ui.question(), Some("Chiudere senza salvare?"));
    assert!(ui.shows(
        "Ancora da salvare: 1 pagina in Documento, 1 foto in Foto. \
         Chiudendo senza salvare, il lavoro va perso."
    ));
    // "Salva…": the photo shown, then the question again for the document.
    ui.click("Salva…");
    assert_eq!(ui.saved().len(), 1, "{:?}", ui.saved());
    assert!(!ui.closed);
    assert!(ui.shows(
        "Ancora da salvare: 1 pagina in Documento. \
         Chiudendo senza salvare, il lavoro va perso."
    ));
    // Saved too, from Documento: nothing left, the window closes.
    ui.click("Salva…");
    assert_eq!(ui.app.kind, Kind::Document);
    assert_eq!(ui.saved().len(), 2, "{:?}", ui.saved());
    assert!(ui.closed);
}

#[test]
fn closing_saves_every_photo_and_the_dialog_says_so() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::AutoCrop(false));
    ui.scan(1);
    ui.send(Message::PageDuplicate(0));
    assert_eq!(ui.app.pages.len(), 2);
    // One name asked, two files written: the dialog says it.
    let (filter, title) = ui.app.save_labels();
    assert_eq!(filter, "Un file per foto");
    assert_eq!(
        title,
        "Salva le 2 foto: un file per foto, con il numero in fondo al nome"
    );
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert!(ui
        .app
        .confirm
        .as_ref()
        .is_some_and(|c| c.body.contains("Le 2 foto acquisite")));
    ui.send(Message::ConfirmSave);
    ui.settle();
    assert_eq!(ui.saved().len(), 2, "{:?} {}", ui.saved(), ui.app.note);
    assert!(ui.closed);
}

#[test]
fn the_save_dialog_says_one_file_or_one_file_per_page() {
    let mut ui = document(3);
    ui.send(Message::Output(crate::Output::Pdf));
    assert_eq!(ui.app.save_labels().1, "Salva le 3 pagine in un solo file");
    ui.send(Message::Output(crate::Output::Image(
        vetroscan_export::ImageFormat::Png,
    )));
    assert_eq!(
        ui.app.save_labels(),
        (
            "Un file per pagina",
            "Salva le 3 pagine: un file per pagina, con il numero in fondo al nome".to_string()
        )
    );
}

#[test]
fn closing_with_nothing_to_lose_closes_at_once() {
    let mut ui = Ui::new();
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert_eq!(ui.question(), None);
    assert!(ui.closed);
}

#[test]
fn closing_switches_the_lamp_off_first() {
    let mut ui = Ui::new();
    ui.scan(1);
    ui.send(Message::Save);
    let lamp = |ui: &Ui| match &ui.app.status {
        Some(Ok(s)) => Some(s.lamp),
        _ => None,
    };
    ui.send(Message::RefreshStatus);
    ui.settle();
    assert_eq!(lamp(&ui), Some(Some(scanner_api::LampState::On)));
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert!(ui.closed);
    ui.settle();
    assert_eq!(lamp(&ui), Some(Some(scanner_api::LampState::Off)));
}

#[test]
fn when_the_lamp_goes_off_is_chosen_and_remembered() {
    use crate::LampAfter;
    {
        let mut ui = Ui::new();
        ui.send(Message::Pane(crate::Pane::Prefs));
        assert!(ui.shows("Lampada"));
        assert_eq!(ui.app.lamp_off, 2, "the default");
        ui.send(Message::LampAfter(LampAfter::app(0)));
        std::mem::forget(ui);
    }
    let (app, _) = App::new();
    assert_eq!(app.lamp_off, 0);
    testing::clean_up();
}

#[test]
fn the_lamps_choices_and_note_come_from_the_scanner() {
    use crate::LampAfter;
    use scanner_api::Scanner as _;
    // The made-up flatbed switches its lamp off by itself after 15 minutes
    // and warms up in 30 seconds: its time closes the choices, and the
    // note says both.
    let mut ui = Ui::new();
    ui.send(Message::Pane(crate::Pane::Prefs));
    let minutes: Vec<u8> = ui.app.lamp_choices().iter().map(|c| c.minutes).collect();
    assert_eq!(minutes, [0, 1, 2, 5, 10, 15]);
    assert_eq!(
        ui.app
            .lamp_choices()
            .last()
            .map(ToString::to_string)
            .as_deref(),
        Some("Dopo 15 minuti (la spegne lo scanner)")
    );
    let note = ui.app.lamp_note();
    assert!(note.contains("circa 30 secondi"), "{note}");
    assert!(note.contains("da solo dopo 15 minuti"), "{note}");
    assert!(ui.shows(&note));
    // No scanner's name in what it says.
    let said = ui.app.lamp_choices().into_iter().map(|c| c.to_string());
    assert!(said.chain([note]).all(|t| !t.contains("HP")));
    std::mem::forget(ui);
    testing::clean_up();

    // One whose lamp has no timer of its own: the app's times only, and
    // nothing said of a timer.
    let flatbed = FakeScanner::flatbed();
    let caps = flatbed.capabilities().clone();
    let mut ui = Ui::with_scanners(vec![FakeScanner::with(scanner_api::Capabilities {
        model: "Lampada senza timer".into(),
        lamp: caps.lamp.map(|l| scanner_api::LampInfo {
            switches_off_after: None,
            ..l
        }),
        ..caps
    })]);
    ui.send(Message::Pane(crate::Pane::Prefs));
    assert!(ui.shows("Lampada"));
    let choices = ui.app.lamp_choices();
    assert_eq!(choices.last(), Some(&LampAfter::app(10)));
    assert!(choices.iter().all(|c| !c.by_scanner));
    let note = ui.app.lamp_note();
    assert!(!note.contains("da solo dopo"), "{note}");
    assert!(ui.shows(&note));
    // A made-up scanner is not on USB: no row about Windows' own app.
    assert!(!ui.shows("Usa i tasti dello scanner solo con questa app"));
    std::mem::forget(ui);
    testing::clean_up();
}

#[test]
fn closing_during_a_scan_asks() {
    let mut ui = Ui::with_pace(Duration::from_millis(300));
    ui.send(Message::Scan);
    assert!(ui.app.busy.is_some());
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert_eq!(ui.question(), Some("Chiudere durante la scansione?"));
    ui.send(Message::Confirm(false));
    ui.settle();
    assert!(!ui.closed);
}

// ------------------------------------------------------------- files

/// What a file starts with, by its extension.
fn looks_like(name: &str, bytes: &[u8]) -> bool {
    let ext = name.rsplit('.').next().unwrap_or_default();
    match ext {
        "pdf" => bytes.starts_with(b"%PDF"),
        "png" => bytes.starts_with(b"\x89PNG"),
        "jpg" | "jpeg" => bytes.starts_with(&[0xff, 0xd8]),
        "tif" | "tiff" | "dng" => bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*"),
        "webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "avif" => bytes.get(4..8) == Some(b"ftyp"),
        "bmp" => bytes.starts_with(b"BM"),
        _ => false,
    }
}

/// Every format offered in this way of working and colour, at 8 and 16
/// bits, is written and is what its name says.
fn every_output_offered_is_saved(kind: Kind, colour: crate::Colour) {
    use crate::{outputs, Depth};
    for depth in [8, 16] {
        let mut ui = Ui::new();
        ui.send(Message::Kind(kind));
        ui.send(Message::Colour(colour));
        ui.send(Message::Depth(Depth(depth)));
        // Two originals in Foto, two pages in Documento.
        if kind == Kind::Photo {
            ui.click("Anteprima");
            ui.send(Message::DropFound(2));
        }
        ui.send(Message::Scan);
        ui.settle();
        if kind == Kind::Document {
            ui.scan(1);
        }
        assert_eq!(ui.app.pages.len(), 2, "{kind:?} {colour:?} {depth}");
        for output in outputs(kind, colour, depth) {
            let before = ui.saved();
            ui.send(Message::Output(output));
            ui.send(Message::Save);
            let new: Vec<String> = ui
                .saved()
                .into_iter()
                .filter(|f| !before.contains(f))
                .collect();
            let what = format!("{kind:?} {colour:?} {depth} bit {output}: {}", ui.app.note);
            assert!(!new.is_empty(), "nothing saved: {what}");
            for name in &new {
                let bytes = std::fs::read(ui.app.folder().join(name)).unwrap();
                assert!(looks_like(name, &bytes), "{name} is not one: {what}");
            }
        }
    }
}

#[test]
fn every_document_format_in_colour() {
    every_output_offered_is_saved(Kind::Document, crate::Colour::Rgb);
}

#[test]
fn every_document_format_in_grey() {
    every_output_offered_is_saved(Kind::Document, crate::Colour::Grey);
}

#[test]
fn every_document_format_in_black_and_white() {
    every_output_offered_is_saved(Kind::Document, crate::Colour::Lineart);
}

#[test]
fn every_photo_format_in_colour() {
    every_output_offered_is_saved(Kind::Photo, crate::Colour::Rgb);
}

#[test]
fn every_photo_format_in_grey() {
    every_output_offered_is_saved(Kind::Photo, crate::Colour::Grey);
}

#[test]
fn every_photo_format_in_black_and_white() {
    every_output_offered_is_saved(Kind::Photo, crate::Colour::Lineart);
}

// ------------------------------------------------- paper

/// The page box and the image matrix of the first page of a PDF saved.
fn pdf_page(ui: &Ui) -> ([f64; 4], [f64; 6], bool) {
    let name = ui
        .saved()
        .into_iter()
        .find(|n| n.ends_with(".pdf"))
        .expect("a PDF");
    let bytes = std::fs::read(ui.app.folder().join(name)).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    let numbers = |s: &str| -> Vec<f64> {
        s.split_whitespace()
            .filter_map(|w| w.parse().ok())
            .collect()
    };
    let media = text.split("/MediaBox [").nth(1).unwrap();
    let media = numbers(media.split(']').next().unwrap());
    let content = text.split("stream\nq ").nth(1).unwrap();
    let content = content.split(" cm").next().unwrap();
    let clipped = content.contains("re W n");
    let matrix = numbers(content.rsplit("re W n").next().unwrap());
    (
        media.try_into().unwrap(),
        matrix.try_into().unwrap(),
        clipped,
    )
}

const MM_PT: f64 = 72.0 / 25.4;

#[test]
fn a_cropped_photo_fitted_to_a4_fills_it_inside_the_margin() {
    use crate::{Margin, PdfFit, PdfSize};
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    // One original only: the first, 4 × 2.8 inches, landscape.
    ui.send(Message::DropFound(2));
    ui.send(Message::DropFound(1));
    ui.scan(1);
    ui.send(Message::Output(crate::Output::Pdf));
    ui.send(Message::Pane(crate::Pane::Output));
    assert!(ui.shows("Foglio"));
    assert!(!ui.shows("Margine"), "no margin at real size");
    ui.send(Message::PdfSize(PdfSize::A4));
    ui.send(Message::PdfFit(PdfFit::Page));
    ui.send(Message::PdfMargin(Margin(10)));
    assert!(ui.shows("Margine"));
    ui.send(Message::Save);
    let (media, m, clipped) = pdf_page(&ui);
    // A4 turned for a landscape photo.
    assert!((media[2] - 297.0 * MM_PT).abs() < 0.1, "{media:?}");
    assert!((media[3] - 210.0 * MM_PT).abs() < 0.1, "{media:?}");
    // As wide or as tall as the sheet less 10 mm each side, centred.
    let (aw, ah) = ((297.0 - 20.0) * MM_PT, (210.0 - 20.0) * MM_PT);
    let (dw, dh) = (m[0], m[3]);
    assert!(
        (dw - aw).abs() < 0.5 || (dh - ah).abs() < 0.5,
        "{m:?} in {aw} × {ah}"
    );
    assert!(dw <= aw + 0.5 && dh <= ah + 0.5);
    assert!((m[4] - (media[2] - dw) / 2.0).abs() < 0.5);
    assert!(!clipped);
}

#[test]
fn a_photo_filling_a_10x15_sheet_is_cut_at_the_margin() {
    use crate::{Margin, PdfFit, PdfSize};
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    // The square original only.
    ui.send(Message::DropFound(2));
    ui.send(Message::DropFound(0));
    ui.scan(1);
    ui.send(Message::Output(crate::Output::Pdf));
    ui.send(Message::PdfSize(PdfSize::Photo10x15));
    ui.send(Message::PdfFit(PdfFit::Fill));
    ui.send(Message::PdfMargin(Margin(0)));
    ui.send(Message::Save);
    let (media, m, clipped) = pdf_page(&ui);
    assert!((media[2] - 100.0 * MM_PT).abs() < 0.1, "{media:?}");
    assert!(clipped, "what overflows is cut");
    assert!(m[3] >= media[3] - 0.5, "covers the height: {m:?}");
}

/// The page boxes of the PDF saved, page after page.
fn pdf_boxes(ui: &Ui) -> Vec<(f64, f64)> {
    let name = ui
        .saved()
        .into_iter()
        .find(|n| n.ends_with(".pdf"))
        .expect("a PDF");
    let bytes = std::fs::read(ui.app.folder().join(name)).unwrap();
    String::from_utf8_lossy(&bytes)
        .split("/MediaBox [0 0 ")
        .skip(1)
        .map(|s| {
            let v: Vec<f64> = s
                .split(']')
                .next()
                .unwrap()
                .split_whitespace()
                .map(|w| w.parse().unwrap())
                .collect();
            (v[0], v[1])
        })
        .collect()
}

#[test]
fn landscape_crops_on_upright_sheets_and_one_page_turned_its_own_way() {
    use crate::{PdfFit, PdfSize, Sheet};
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    // The two landscape-ish originals: 4 × 2.8 and 3 × 3 inches.
    ui.send(Message::DropFound(2));
    ui.scan(1);
    assert_eq!(ui.app.pages.len(), 2);
    ui.send(Message::Output(crate::Output::Pdf));
    ui.send(Message::PdfSize(PdfSize::A4));
    ui.send(Message::PdfFit(PdfFit::Page));
    ui.send(Message::PdfSheet(Sheet::Portrait));
    // The second page sideways, the first as the document.
    ui.send(Message::PageShow(1));
    ui.send(Message::Pane(crate::Pane::Output));
    assert!(ui.shows("Impaginazione propria per la foto 2"));
    ui.send(Message::PdfOwnLayout(true));
    ui.send(Message::PdfSheet(Sheet::Landscape));
    assert!(ui.shows("2 · foglio"));
    assert_eq!(
        ui.app.photo.pdf_sheet,
        Sheet::Portrait,
        "the document stays"
    );
    ui.send(Message::Save);
    let boxes = pdf_boxes(&ui);
    assert_eq!(boxes.len(), 2);
    assert!(boxes[0].0 < boxes[0].1, "page 1 upright: {boxes:?}");
    assert!(boxes[1].0 > boxes[1].1, "page 2 sideways: {boxes:?}");
}

/// Two photos, PDF on A4 fitted with a 10 mm margin, the output panel
/// open on the second.
fn two_photos_on_a4() -> Ui {
    use crate::{Margin, PdfFit, PdfSize};
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    ui.send(Message::DropFound(2));
    ui.scan(1);
    assert_eq!(ui.app.pages.len(), 2);
    ui.send(Message::Output(crate::Output::Pdf));
    ui.send(Message::PdfSize(PdfSize::A4));
    ui.send(Message::PdfFit(PdfFit::Page));
    ui.send(Message::PdfMargin(Margin(10)));
    ui.send(Message::PageShow(1));
    ui.send(Message::Pane(crate::Pane::Output));
    ui
}

/// The image matrix (width, height, left, bottom) of every page of the
/// PDF saved.
fn pdf_matrices(ui: &Ui) -> Vec<[f64; 4]> {
    let name = ui
        .saved()
        .into_iter()
        .find(|n| n.ends_with(".pdf"))
        .expect("a PDF");
    let bytes = std::fs::read(ui.app.folder().join(name)).unwrap();
    String::from_utf8_lossy(&bytes)
        .split(
            "stream
q ",
        )
        .skip(1)
        .map(|s| {
            let s = s.split(" cm").next().unwrap();
            let s = s.rsplit("re W n").next().unwrap();
            let v: Vec<f64> = s
                .split_whitespace()
                .filter_map(|w| w.parse().ok())
                .collect();
            [v[0], v[3], v[4], v[5]]
        })
        .collect()
}

#[test]
fn one_page_has_its_own_margin_and_then_gives_it_to_all() {
    use crate::{Margin, PdfFit};
    let mut ui = two_photos_on_a4();
    assert!(ui.app.print_preview(), "the sheet is in the viewer");
    ui.send(Message::PdfOwnLayout(true));
    ui.send(Message::PdfMargin(Margin(20)));
    assert_eq!(ui.app.photo.pdf_margin, 10, "the document keeps its margin");
    assert!(ui.shows("Applica a tutte le pagine"));
    ui.send(Message::Save);
    let m = pdf_matrices(&ui);
    // Page 2 fitted inside a wider margin: smaller than page 1's fit
    // would make it, and no closer than 20 mm to the edge.
    let mm = 72.0 / 25.4;
    assert!(
        m[1][2] >= 20.0 * mm - 0.5 && m[1][3] >= 20.0 * mm - 0.5,
        "{m:?}"
    );
    // Its layout for every page.
    ui.send(Message::PdfFit(PdfFit::Fill));
    ui.send(Message::PdfApplyToAll);
    assert_eq!(ui.app.photo.pdf_margin, 20);
    assert_eq!(ui.app.photo.pdf_fit, PdfFit::Fill);
    assert!(ui.app.pages.iter().all(|p| p.layout.is_none()));
    // And back: "Come le altre" drops an own layout.
    ui.send(Message::PdfOwnLayout(true));
    ui.send(Message::PdfMargin(Margin(5)));
    ui.click("Come le altre");
    assert!(ui.app.pages[1].layout.is_none());
    assert_eq!(ui.app.photo.pdf_margin, 20);
}

#[test]
fn the_part_kept_of_a_filled_page_is_where_it_was_dragged() {
    use crate::PdfFit;
    let mut ui = two_photos_on_a4();
    ui.send(Message::PageShow(0));
    ui.send(Message::PdfFit(PdfFit::Fill));
    // The first photo (4 x 2.8 in) fills a landscape A4 and overflows it
    // in height: kept, its top, then its bottom.
    let saved_with = |ui: &mut Ui, focus: [f32; 2]| {
        for name in ui.saved() {
            std::fs::remove_file(ui.app.folder().join(name)).unwrap();
        }
        ui.send(Message::PageFocus(focus));
        ui.send(Message::Save);
        pdf_matrices(ui)[0]
    };
    let top = saved_with(&mut ui, [0.5, 0.0]);
    let bottom = saved_with(&mut ui, [0.5, 1.0]);
    // y from the bottom of the sheet: the top part kept puts the image
    // low (its top at the top of the margin), the bottom part high.
    assert!(top[3] < bottom[3] - 1.0, "top {top:?} bottom {bottom:?}");
    assert_eq!(top[0], bottom[0], "same size");
}

#[test]
#[ignore = "writes a screenshot to look at"]
fn screenshot_of_the_print_preview() {
    use crate::PdfFit;
    let mut ui = two_photos_on_a4();
    ui.send(Message::PageShow(0));
    ui.send(Message::PdfFit(PdfFit::Fill));
    ui.send(Message::PageFocus([0.5, 0.2]));
    let mut sim = iced_test::Simulator::with_size(
        iced::Settings::default(),
        Size::new(1360.0, 900.0),
        ui.app.view(),
    );
    let snapshot = sim.snapshot(&iced::Theme::Dark).unwrap();
    let path = std::env::temp_dir().join("vetroscan-print-preview.png");
    let _ = std::fs::remove_file(&path);
    let _ = snapshot.matches_image(&path);
    eprintln!("screenshot: {}", path.display());
}

#[test]
fn dragging_the_page_on_the_sheet_moves_what_is_kept() {
    use crate::PdfFit;
    use iced::mouse;
    let mut ui = two_photos_on_a4();
    ui.send(Message::PdfFit(PdfFit::Fill));
    ui.send(Message::PageFocus([0.5, 0.5]));
    assert!(ui.app.print_preview());
    // Press in the middle of the viewer (the sheet is centred there), drag
    // right: more of the left part is kept.
    let mut sim = iced_test::Simulator::with_size(
        iced::Settings::default(),
        Size::new(1360.0, 900.0),
        ui.app.view(),
    );
    let at = |x: f32, y: f32| iced::Point::new(x, y);
    sim.point_at(at(560.0, 450.0));
    let _ = sim.simulate([iced::Event::Mouse(mouse::Event::ButtonPressed(
        mouse::Button::Left,
    ))]);
    sim.point_at(at(660.0, 450.0));
    let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved {
        position: at(660.0, 450.0),
    })]);
    let messages: Vec<Message> = sim.into_messages().collect();
    for m in messages {
        ui.send(m);
    }
    let focus = ui.app.pages[1].focus;
    assert!(focus[0] < 0.5 || focus[1] < 0.5, "moved: {focus:?}");
}

// ------------------------------------------------- size of the PDF

/// The bytes of the PDF saved last.
fn pdf_bytes(ui: &Ui) -> u64 {
    let mut pdfs: Vec<_> = ui
        .saved()
        .into_iter()
        .filter(|n| n.ends_with(".pdf"))
        .map(|n| std::fs::metadata(ui.app.folder().join(n)).unwrap())
        .collect();
    pdfs.sort_by_key(|m| m.modified().unwrap());
    pdfs.last().expect("a PDF").len()
}

/// The estimate shown now, in bytes (it must be up to date).
fn estimate(ui: &Ui) -> vetroscan_export::Planned {
    let e = ui.app.pdf_estimate.as_ref().expect("an estimate");
    assert_eq!(e.key, ui.app.estimate_key(), "up to date");
    e.result.clone().expect("estimated")
}

/// A document of three pages, PDF, the output panel open.
fn pdf_document() -> Ui {
    let mut ui = document(3);
    ui.send(Message::Output(crate::Output::Pdf));
    ui.send(Message::Pane(crate::Pane::Output));
    ui
}

#[test]
fn the_size_of_the_pdf_is_estimated_before_saving() {
    let mut ui = pdf_document();
    let planned = estimate(&ui);
    assert!(planned.fits);
    assert!(ui.shows(&format!(
        "Dimensione stimata: circa {}",
        crate::human_size(planned.bytes)
    )));
    ui.send(Message::Save);
    let real = pdf_bytes(&ui) as f64;
    let guess = planned.bytes as f64;
    assert!(
        (guess / real - 1.0).abs() < 0.4,
        "estimated {guess}, written {real}"
    );
    assert!(ui.app.note.contains("PDF di"), "{}", ui.app.note);
}

#[test]
fn a_lower_quality_makes_a_smaller_pdf() {
    let mut ui = pdf_document();
    ui.send(Message::Save);
    let full = pdf_bytes(&ui);
    let at_full = estimate(&ui).bytes;
    ui.send(Message::PdfQuality(50));
    assert!(estimate(&ui).bytes < at_full);
    ui.send(Message::Save);
    assert!(pdf_bytes(&ui) < full);
}

#[test]
fn a_limit_is_kept_by_lightening_pages() {
    let mut ui = pdf_document();
    let free = estimate(&ui).bytes;
    // A third of it, in megabytes as typed.
    let mb = free as f64 / 3.0 / 1024.0 / 1024.0;
    let typed = format!("{mb:.3}").replace('.', ",");
    ui.send(Message::PdfLimitText(typed));
    let limit = u64::from(ui.app.settings().pdf_limit_kb.expect("a limit")) * 1024;
    let planned = estimate(&ui);
    assert!(planned.fits && planned.bytes <= limit, "{planned:?}");
    ui.send(Message::Save);
    let written = pdf_bytes(&ui);
    assert!(written <= limit, "{written} over {limit}: {}", ui.app.note);
    assert!(ui.app.note.contains("alleggerit"), "{}", ui.app.note);
}

#[test]
fn a_limit_too_small_says_so_and_still_saves() {
    let mut ui = pdf_document();
    ui.send(Message::PdfLimitText("0,001".into()));
    let planned = estimate(&ui);
    assert!(!planned.fits);
    ui.send(Message::Save);
    assert!(ui.app.note.contains("oltre il limite"), "{}", ui.app.note);
}

#[test]
fn a_limit_that_is_not_a_number_is_pointed_out() {
    let mut ui = pdf_document();
    ui.send(Message::PdfLimitText("5".into()));
    assert_eq!(ui.app.settings().pdf_limit_kb, Some(5 * 1024));
    ui.send(Message::PdfLimitText("cinque".into()));
    assert!(ui.shows("Scrivi un numero di megabyte, per esempio 5 o 2,5"));
    assert_eq!(ui.app.settings().pdf_limit_kb, Some(5 * 1024), "kept");
    ui.send(Message::PdfLimitText(String::new()));
    assert_eq!(ui.app.settings().pdf_limit_kb, None);
}

#[test]
fn colour_adjustments_do_not_come_back_after_closing_scan_settings_do() {
    use crate::Field;
    {
        let mut ui = Ui::new();
        ui.send(Message::Kind(Kind::Photo));
        ui.send(Message::Adjust(Field::Temperature, 40.0));
        ui.send(Message::Adjust(Field::Saturation, 30.0));
        ui.send(Message::Dpi(600));
        ui.send(Message::Colour(crate::Colour::Grey));
        assert_eq!(ui.app.photo.adjust.temperature, 40.0);
        // A look to keep is a preset.
        ui.send(Message::PresetName("Caldo".into()));
        ui.send(Message::PresetSave);
        std::mem::forget(ui);
    }
    let (app, _) = App::new();
    let fresh = scan_imaging::adjust::Adjustments::default();
    assert_eq!(app.photo.adjust, fresh, "the look starts clean");
    assert_eq!(app.document.adjust, fresh);
    assert_eq!(app.photo.dpi, 600, "the scan settings stay");
    assert_eq!(app.photo.colour, crate::Colour::Grey);
    // The preset brings it back.
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::PresetApply("Caldo".into()));
    assert_eq!(ui.app.photo.adjust.temperature, 40.0);
    assert_eq!(ui.app.photo.adjust.saturation, 30.0);
}

#[test]
fn each_page_has_its_own_look_until_it_is_given_to_all() {
    use crate::Field;
    let mut ui = document(2);
    assert!(
        ui.app.pages.iter().all(|p| p.own.is_some()),
        "«Solo questa pagina» from the start"
    );
    // A change on the second page leaves the first and the next scans alone.
    ui.send(Message::PageShow(1));
    ui.send(Message::Pane(crate::Pane::Adjust));
    assert!(ui.shows("Solo questa pagina"));
    ui.send(Message::Adjust(Field::Temperature, 40.0));
    let temperature = |ui: &Ui, i: usize| ui.app.pages[i].own.unwrap().adjust.temperature;
    assert_eq!(temperature(&ui, 1), 40.0);
    assert_eq!(temperature(&ui, 0), 0.0);
    assert_eq!(ui.app.document.adjust.temperature, 0.0);
    // The first page keeps its look when the second goes back to the common one.
    ui.send(Message::OwnLook(false));
    ui.send(Message::Adjust(Field::Saturation, 30.0));
    assert_eq!(ui.app.document.adjust.saturation, 30.0);
    assert_eq!(ui.app.pages[0].own.unwrap().adjust.saturation, 0.0);
    // Given to all: every page, the preview and the next scans.
    ui.send(Message::OwnLook(true));
    ui.send(Message::Adjust(Field::Temperature, 25.0));
    ui.click("Applica a tutte le pagine");
    for i in 0..2 {
        assert_eq!(temperature(&ui, i), 25.0, "page {i}");
    }
    assert_eq!(ui.app.document.adjust.temperature, 25.0);
    ui.scan(1);
    assert_eq!(temperature(&ui, 2), 25.0);
}

#[test]
fn the_sheet_chosen_is_remembered() {
    use crate::{Margin, PdfFit, PdfSize};
    {
        let mut ui = Ui::new();
        ui.send(Message::Kind(Kind::Document));
        ui.send(Message::PdfSize(PdfSize::A5));
        ui.send(Message::PdfFit(PdfFit::Fill));
        ui.send(Message::PdfMargin(Margin(15)));
        ui.send(Message::PdfQuality(70));
        ui.send(Message::PdfLimitText("2,5".into()));
        // Kept for the next start: the test's folder stays until the end.
        std::mem::forget(ui);
    }
    let (app, _) = App::new();
    assert_eq!(app.document.pdf_size, PdfSize::A5);
    assert_eq!(app.document.pdf_fit, PdfFit::Fill);
    assert_eq!(app.document.pdf_margin, 15);
    assert_eq!(app.document.pdf_quality, 70);
    assert_eq!(app.document.pdf_limit_kb, Some(2560));
    testing::clean_up();
}

#[test]
fn saving_twice_in_the_same_second_writes_over_nothing() {
    use vetroscan_export::ImageFormat;
    let mut ui = document(2);
    // One image per page: NAME-001.png and NAME-002.png.
    ui.send(Message::Output(crate::Output::Image(ImageFormat::Png)));
    ui.send(Message::Save);
    ui.send(Message::Save);
    ui.send(Message::Save);
    assert_eq!(ui.saved().len(), 6, "{:?}", ui.saved());
}

// ------------------------------------------------------------- scans

#[test]
fn rifai_offers_the_new_scan_and_the_choice_is_kept() {
    let mut ui = document(2);
    let epoch = ui.app.epoch;
    ui.send(Message::PageRedo(0));
    ui.settle();
    assert!(ui.app.redone.is_some());
    assert!(ui.shows("Tieni la nuova"));
    ui.click("Lascia la vecchia");
    assert!(ui.app.redone.is_none());
    assert_eq!(ui.app.epoch, epoch, "the old page stayed");
    ui.send(Message::PageRedo(1));
    ui.settle();
    ui.click("Tieni la nuova");
    assert_eq!(ui.app.pages.len(), 2);
    assert_ne!(ui.app.epoch, epoch, "page 2 was replaced");
    assert!(ui.app.note.contains("sostituita"), "{}", ui.app.note);
}

#[test]
fn ctrl_z_takes_a_setting_back_and_ctrl_y_puts_it_again() {
    let mut ui = Ui::new();
    let colour = ui.app.settings().colour;
    ui.send(Message::Colour(crate::Colour::Grey));
    assert_eq!(ui.app.settings().colour, crate::Colour::Grey);
    ui.send(Message::Undo);
    assert_eq!(ui.app.settings().colour, colour);
    ui.send(Message::Redo);
    assert_eq!(ui.app.settings().colour, crate::Colour::Grey);
}

#[test]
fn colour_in_the_scan_settings_is_for_the_next_scans_a_page_changes_it_in_regola() {
    use crate::Colour;
    let mut ui = document(1);
    ui.send(Message::Save);
    ui.send(Message::PageShow(0));
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(ui.shows(
        "Valgono per le prossime scansioni: il colore della pagina 1 si cambia in «Regola»."
    ));
    // Chosen with the page shown: the next scans, as the resolution beside it.
    ui.send(Message::Colour(Colour::Grey));
    assert_eq!(ui.app.settings().colour, Colour::Grey);
    assert_eq!(ui.app.pages[0].settings.colour, Colour::Rgb);
    assert!(!ui.app.unsaved, "the page did not change");
    ui.send(Message::Undo);
    assert_eq!(ui.app.settings().colour, Colour::Rgb);
    // The page's own, at the top of "Regola".
    ui.send(Message::Pane(crate::Pane::Adjust));
    assert!(ui.shows("Modo"));
    ui.send(Message::PageColour(Colour::Lineart));
    assert_eq!(ui.app.pages[0].settings.colour, Colour::Lineart);
    assert_eq!(ui.app.settings().colour, Colour::Rgb);
    assert!(ui.app.unsaved);
    // Ctrl+Z gives the page its colour back, Ctrl+Y the new one again.
    ui.send(Message::Undo);
    assert_eq!(ui.app.pages[0].settings.colour, Colour::Rgb);
    ui.send(Message::Redo);
    assert_eq!(ui.app.pages[0].settings.colour, Colour::Lineart);
    ui.scan(1);
    assert_eq!(ui.app.pages[1].settings.colour, Colour::Rgb);
}

#[test]
fn the_editor_changes_only_the_geometry() {
    use crate::EditMsg;
    let mut ui = document(1);
    ui.send(Message::Edit(EditMsg::Open(0)));
    assert!(!ui.shows("Ripristina colori"));
    assert!(!ui.shows("Modo"));
    ui.send(Message::Edit(EditMsg::Discard));
    assert!(ui.app.editor.is_none());
}

#[test]
fn a_photo_takes_a_look_of_its_own_at_its_first_change() {
    use crate::Field;
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::AutoCrop(false));
    ui.scan(1);
    ui.send(Message::PageDuplicate(0));
    assert_eq!(ui.app.pages.len(), 2);
    assert!(ui.app.pages.iter().all(|p| p.own.is_none()));
    ui.send(Message::PageShow(1));
    ui.send(Message::Pane(crate::Pane::Adjust));
    assert!(ui.shows(
        "Segue le regolazioni comuni finché non ne cambi una: da lì valgono solo per questa foto."
    ));
    // Only the photo shown: not the other, the preview or the next scans.
    ui.send(Message::Adjust(Field::Temperature, 40.0));
    assert_eq!(ui.app.pages[1].own.unwrap().adjust.temperature, 40.0);
    assert!(ui.app.pages[0].own.is_none());
    assert_eq!(ui.app.photo.adjust.temperature, 0.0);
    assert!(ui.shows("Regolazioni solo di questa foto."));
    // Ctrl+Z: back to following the shared look.
    ui.send(Message::Undo);
    assert!(ui.app.pages[1].own.is_none());
    ui.send(Message::Redo);
    assert_eq!(ui.app.pages[1].own.unwrap().adjust.temperature, 40.0);
    // Given to all: the shared look, which every photo follows again.
    ui.click("Applica a tutte le foto");
    assert_eq!(ui.app.photo.adjust.temperature, 40.0);
    assert!(ui.app.pages.iter().all(|p| p.own.is_none()));
    let shared = *ui.app.settings();
    assert_eq!(ui.app.pages[0].effective(&shared).adjust.temperature, 40.0);
    // And back to the common look on its own.
    ui.send(Message::Adjust(Field::Saturation, 30.0));
    assert!(ui.app.pages[1].own.is_some());
    ui.click("Torna alle regolazioni comuni");
    assert!(ui.app.pages[1].own.is_none());
    assert_eq!(ui.app.photo.adjust.saturation, 0.0);
}

#[test]
fn the_editor_turns_a_page_and_its_ctrl_z_takes_it_back() {
    use crate::EditMsg;
    let mut ui = document(1);
    let before = format!("{:?}", ui.app.pages[0].transform);
    ui.send(Message::Edit(EditMsg::Open(0)));
    assert!(ui.app.editor.is_some());
    ui.send(Message::Edit(EditMsg::Rotate(true)));
    ui.send(Message::Edit(EditMsg::Undo));
    ui.send(Message::Edit(EditMsg::Rotate(true)));
    ui.send(Message::Edit(EditMsg::Apply));
    assert!(ui.app.editor.is_none());
    let after = format!("{:?}", ui.app.pages[0].transform);
    assert_ne!(after, before, "the page was turned");
    // Turned four times in all: as it was.
    ui.send(Message::Edit(EditMsg::Open(0)));
    for _ in 0..3 {
        ui.send(Message::Edit(EditMsg::Rotate(true)));
    }
    ui.send(Message::Edit(EditMsg::Apply));
    assert_eq!(format!("{:?}", ui.app.pages[0].transform), before);
}

// ------------------------------------------------- duplicate and drag

/// Which scan each page shows (pages scanned apart have their own).
fn scans(ui: &Ui) -> Vec<*const scan_imaging::Image> {
    ui.app
        .pages
        .iter()
        .map(|p| Arc::as_ptr(&p.raw.full))
        .collect()
}

#[test]
fn duplica_copies_a_page_that_is_then_edited_on_its_own() {
    use crate::EditMsg;
    let mut ui = document(2);
    let before = scans(&ui);
    ui.click("Duplica");
    assert_eq!(ui.app.pages.len(), 3);
    let after = scans(&ui);
    assert_eq!(
        after,
        vec![before[0], before[0], before[1]],
        "same scan, no copy"
    );
    assert_eq!(ui.app.current, Some(1));
    assert!(ui.app.unsaved);
    // Turned in the editor, the copy only.
    ui.send(Message::Edit(EditMsg::Open(1)));
    ui.send(Message::Edit(EditMsg::Rotate(true)));
    ui.send(Message::Edit(EditMsg::Apply));
    assert!(ui.app.pages[0].transform.is_identity());
    assert!(!ui.app.pages[1].transform.is_identity());
}

#[test]
fn several_photos_cut_from_one_scan_are_saved_one_file_each() {
    use crate::EditMsg;
    use vetroscan_export::ImageFormat;
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::AutoCrop(false));
    ui.scan(1);
    ui.send(Message::PageDuplicate(0));
    ui.send(Message::PageDuplicate(0));
    assert_eq!(ui.app.pages.len(), 3);
    // A different square of the glass for each copy (pixels of the scan).
    let side = ui.app.pages[0].raw.full.width as f32;
    for (i, x) in [0.1, 0.4, 0.7].into_iter().enumerate() {
        ui.send(Message::Edit(EditMsg::Open(i)));
        ui.send(Message::Edit(EditMsg::Crop(Some(crate::Selection {
            x: x * side,
            y: 0.1 * side,
            width: 0.2 * side,
            height: 0.2 * side,
        }))));
        ui.send(Message::Edit(EditMsg::Apply));
    }
    ui.send(Message::Output(crate::Output::Image(ImageFormat::Png)));
    ui.send(Message::Save);
    assert_eq!(ui.saved().len(), 3, "{:?} {}", ui.saved(), ui.app.note);
}

#[test]
fn ctrl_d_duplicates_the_page_shown() {
    let mut ui = document(1);
    ui.send(Message::PageShow(0));
    let event = iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Character("d".into()),
        modified_key: iced::keyboard::Key::Character("d".into()),
        physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyD),
        location: iced::keyboard::Location::Standard,
        // Ctrl, or Cmd on macOS, as every shortcut of the app.
        modifiers: iced::keyboard::Modifiers::COMMAND,
        text: None,
        repeat: false,
    };
    ui.send(Message::Key(event));
    assert_eq!(ui.app.pages.len(), 2);
}

#[test]
fn a_page_dragged_onto_another_takes_its_place() {
    let mut ui = document(4);
    let s = scans(&ui);
    // The first page down to the third place.
    ui.send(Message::PagePress(0));
    ui.send(Message::PageHover(1));
    ui.send(Message::PageHover(2));
    assert_eq!(ui.app.drag, Some((0, Some(2))));
    ui.send(Message::PageDrop);
    assert_eq!(scans(&ui), vec![s[1], s[2], s[0], s[3]]);
    assert_eq!(ui.app.current, Some(2));
    // The last page up to the first place.
    ui.send(Message::PagePress(3));
    ui.send(Message::PageHover(0));
    ui.send(Message::PageDrop);
    assert_eq!(scans(&ui), vec![s[3], s[1], s[2], s[0]]);
    assert!(ui.app.drag.is_none());
}

#[test]
fn a_click_on_a_page_shows_it_and_moves_nothing() {
    let mut ui = document(3);
    ui.send(Message::Save);
    let s = scans(&ui);
    ui.send(Message::PagePress(1));
    ui.send(Message::PageDrop);
    assert_eq!(scans(&ui), s);
    assert_eq!(ui.app.current, Some(1));
    assert!(!ui.app.unsaved, "only looking is not a change");
}

#[test]
fn a_page_dragged_out_of_the_strip_stays_where_it_was() {
    let mut ui = document(3);
    let s = scans(&ui);
    ui.send(Message::PagePress(0));
    ui.send(Message::PageHover(2));
    // Out of the strip: over its own place again.
    ui.send(Message::PageHover(0));
    ui.send(Message::PageDrop);
    assert_eq!(scans(&ui), s);
}

#[test]
fn a_page_is_dragged_with_the_mouse_in_the_real_strip() {
    use iced::mouse;
    let mut ui = document(3);
    let s = scans(&ui);
    // The thumbnails are 140 high, 10 apart, under the header: the first
    // around y = 110, the third around y = 110 + 2 × (140 + 70).
    let events = |at: [f32; 2], what: Option<mouse::Event>| {
        let mut list = vec![iced::Event::Mouse(mouse::Event::CursorMoved {
            position: iced::Point::new(at[0], at[1]),
        })];
        list.extend(what.map(iced::Event::Mouse));
        list
    };
    let mut messages = Vec::new();
    let sim = |list: Vec<iced::Event>, ui: &mut Ui, messages: &mut Vec<Message>| {
        let mut s = iced_test::Simulator::with_size(
            iced::Settings::default(),
            Size::new(1360.0, 900.0),
            ui.app.view(),
        );
        // The simulator's cursor is where the last move went.
        if let Some(iced::Event::Mouse(mouse::Event::CursorMoved { position })) = list.first() {
            s.point_at(*position);
        }
        let _ = s.simulate(list);
        let new: Vec<Message> = s.into_messages().collect();
        for m in &new {
            ui.send(m.clone());
        }
        messages.extend(new);
    };
    let first = ui.app.pages.len();
    assert_eq!(first, 3);
    sim(
        events(
            [78.0, 120.0],
            Some(mouse::Event::ButtonPressed(mouse::Button::Left)),
        ),
        &mut ui,
        &mut messages,
    );
    assert_eq!(ui.app.drag.map(|d| d.0), Some(0), "{messages:?}");
    // Down the strip, page by page, then let go.
    for y in (140..560).step_by(20) {
        sim(events([78.0, y as f32], None), &mut ui, &mut messages);
    }
    let over = ui.app.drag.and_then(|d| d.1);
    assert!(
        over.is_some(),
        "the drag reached another page: {messages:?}"
    );
    sim(
        events(
            [78.0, 540.0],
            Some(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ),
        &mut ui,
        &mut messages,
    );
    assert!(ui.app.drag.is_none());
    assert_ne!(scans(&ui), s, "the page moved: {messages:?}");
    assert_eq!(scans(&ui)[over.unwrap()], s[0]);
}

#[test]
fn annulla_stops_a_scan_and_keeps_nothing() {
    let mut ui = Ui::with_pace(Duration::from_millis(300));
    ui.send(Message::Kind(Kind::Document));
    ui.send(Message::Scan);
    assert!(ui.app.busy.is_some());
    ui.send(Message::Cancel);
    ui.settle();
    assert!(ui.app.pages.is_empty());
}

#[test]
fn the_preview_finds_the_three_originals_and_foto_scans_each() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    assert!(ui.app.preview.is_some());
    assert_eq!(
        ui.app.shown_regions().len(),
        scanner_api::fake::ORIGINALS.len(),
        "{:?}",
        ui.app.regions
    );
    ui.click("Scansiona 3 foto");
    assert_eq!(ui.app.pages.len(), 3);
}

// ------------------------------------------- what the scanner offers

/// The made-up A3 scanner without buttons.
fn without_buttons() -> FakeScanner {
    use scanner_api::Scanner as _;
    FakeScanner::with(scanner_api::Capabilities {
        model: "Piano senza tasti".into(),
        buttons: Vec::new(),
        ..FakeScanner::a3().capabilities().clone()
    })
}

#[test]
fn the_flatbed_offers_its_lamp_its_buttons_and_16_bits() {
    let mut ui = Ui::new();
    assert_eq!(ui.app.caps.model, "Flatbed A4");
    assert!(ui.shows("Pronto · lampada accesa"));
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(ui.shows("Profondità"));
    assert!(ui.shows("Tasti dello scanner"));
    ui.send(Message::Pane(crate::Pane::Prefs));
    for label in ["Lampada", "Spegnila", "Scan", "Copy", "Instant Share"] {
        assert!(ui.shows(label), "{label}");
    }
    // One scanner: no choice of scanner.
    assert!(!ui.shows("Usa"));
    // No adapter for slides: no film.
    assert!(!ui.shows("Pellicola"));
}

/// The made-up flatbed with its film adapter, and the handle on its cable.
fn with_adapter() -> (Ui, scanner_api::fake::Controls) {
    let scanner = FakeScanner::with_adapter();
    let controls = scanner.controls();
    (Ui::with_scanners(vec![scanner]), controls)
}

#[test]
fn films_need_the_adapter_and_its_cable() {
    let (mut ui, controls) = with_adapter();
    assert!(ui.shows("Pellicola"));
    // Its cable unplugged (and the state read since): a press says so and
    // changes nothing.
    controls.unplug_adapter(true);
    ui.send(Message::RefreshStatus);
    ui.adapter_state(false);
    ui.click("Pellicola");
    assert!(!ui.app.filming());
    assert!(ui.app.note.contains("non è collegato"), "{}", ui.app.note);
    assert_eq!(ui.app.request().source, scanner_api::Source::Glass);
    // Plugged in: the next press finds it and switches.
    controls.unplug_adapter(false);
    ui.click("Pellicola");
    assert!(ui.app.filming());
    assert_eq!(ui.app.kind, Kind::Photo);
    let request = ui.app.request();
    assert_eq!(request.source, scanner_api::Source::Negative);
    assert_eq!(request.depth, 16, "a negative needs the 16-bit samples");
    // Settings of their own: 1200 dpi, not the photos' 600.
    assert_eq!(ui.app.settings().dpi, 1200);
    ui.click("Foto");
    assert!(!ui.app.filming());
    assert_eq!(ui.app.request().source, scanner_api::Source::Glass);
    assert_eq!(
        ui.app.settings().dpi,
        200,
        "the photos' own, set by the tests"
    );
}

#[test]
fn frames_scanned_stay_to_see_and_save_with_the_adapter_unplugged() {
    let (mut ui, controls) = with_adapter();
    ui.click("Pellicola");
    ui.send(Message::Dpi(200));
    ui.click("Scansiona foto");
    let frames = scans(&ui);
    assert_eq!(frames.len(), 2);
    ui.click("Foto");
    assert!(ui.app.pages.is_empty());
    controls.unplug_adapter(true);
    ui.send(Message::RefreshStatus);
    ui.adapter_state(false);
    ui.click("Pellicola");
    assert!(ui.app.filming());
    assert_eq!(scans(&ui), frames);
    assert!(ui.app.note.contains("non è collegato"), "{}", ui.app.note);
}

#[test]
fn a_single_photo_has_its_tools_in_the_column() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    assert!(!ui.shows("Duplica"), "no photo, no column");
    ui.scan(1);
    assert_eq!(ui.app.pages.len(), 1);
    for label in ["Modifica", "Rifai", "Duplica", "Elimina"] {
        assert!(ui.shows(label), "{label}");
    }
    ui.click("Duplica");
    assert_eq!(ui.app.pages.len(), 2);
}

#[test]
fn a_strip_of_negatives_becomes_a_picture_per_frame() {
    let (mut ui, _) = with_adapter();
    ui.click("Pellicola");
    ui.send(Message::Dpi(200));
    // Films have their preview (since 15 November 2026), but a scan made
    // without it still finds the frames itself.
    assert!(ui.shows("Anteprima"));
    // One scan, one picture for each frame, standing up as on the strip.
    ui.click("Scansiona foto");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    for page in &ui.app.pages {
        let full = &page.raw.full;
        assert_eq!(page.raw.tone, scan_imaging::film::TONE);
        assert!(page.raw16.is_some());
        assert!(full.height > full.width, "{} x {}", full.width, full.height);
        // The made-up scene is denser to the right: brighter there in the
        // picture, darker to the left.
        let grey = |x: usize| -> u32 {
            let i = (full.height / 2 * full.width + x) * 3;
            full.data[i..i + 3].iter().map(|&v| u32::from(v)).sum()
        };
        assert!(
            grey(full.width * 4 / 5) > grey(full.width / 5) + 60,
            "{} then {}",
            grey(full.width / 5),
            grey(full.width * 4 / 5)
        );
    }
    // Back to the glass: its preview again.
    ui.click("Foto");
    assert!(ui.shows("Anteprima"));
    assert!(ui.app.regions.is_empty() && ui.app.measures.is_none());
}

/// Whether every original in `a` has one in `b` with its centre within
/// `px` pixels of the preview, and as many.
fn same_places(a: &[Found], b: &[Found], px: f32) -> bool {
    a.len() == b.len()
        && a.iter().all(|f| {
            b.iter()
                .any(|g| (f.cx - g.cx).abs() < px && (f.cy - g.cy).abs() < px)
        })
}

/// Where the preview finds the originals, in the mode the app is in.
fn found_by_the_preview(ui: &mut Ui) -> Vec<Found> {
    ui.send(Message::Preview);
    ui.settle();
    let found: Vec<Found> = ui.app.shown_regions().iter().map(|r| r.at).collect();
    assert!(!found.is_empty(), "{}", ui.app.note);
    found
}

#[test]
fn without_a_preview_the_frames_are_found_in_the_scan() {
    let (mut ui, _) = with_adapter();
    ui.click("Pellicola");
    ui.send(Message::Dpi(200));
    ui.click("Scansiona foto");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    for page in &ui.app.pages {
        assert!(page.whole.is_none());
        assert_eq!(page.raw.tone, scan_imaging::film::TONE);
        assert!(page.raw16.is_some());
        let full = &page.raw.full;
        assert!(full.height > full.width, "{} x {}", full.width, full.height);
    }
    // "Rifai" on a frame scans that frame only.
    ui.send(Message::PageRedo(1));
    ui.settle();
    assert_eq!(ui.app.pages.len(), 2);
}

/// The film's picture brighter to the right, as the made-up scene is
/// denser there: a frame developed the right way round.
fn developed_right(page: &crate::Page) -> bool {
    let full = &page.raw.full;
    grey_at(full, full.width * 4 / 5) > grey_at(full, full.width / 5) + 60
}

#[test]
fn the_preview_of_films_finds_the_frames_and_each_is_scanned_on_its_own() {
    let (mut ui, controls) = with_adapter();
    ui.click("Pellicola");
    assert_eq!(ui.app.tab, crate::Tab::Preview, "the preview first");
    ui.click("Anteprima");
    assert!(ui.app.preview.is_some(), "{}", ui.app.note);
    assert!(
        ui.app.measures.is_some(),
        "the film measured on the preview"
    );
    let frames = ui.app.shown_regions();
    assert_eq!(frames.len(), 2, "{}", ui.app.note);
    // The preview at the lowest resolution the adapter scans at, whole.
    let asked = controls.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].dpi, 600);
    ui.send(Message::Dpi(1200));
    ui.click("Scansiona 2 foto");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    // One scan for each frame: the frame and a millimetre around it, not
    // the strip between.
    let asked = &controls.requests()[1..];
    assert_eq!(asked.len(), 2, "{asked:?}");
    let k = 1200.0 / f32::from(crate::PREVIEW_DPI);
    let margin = f32::from(crate::PREVIEW_DPI) / 25.4;
    for (request, frame) in asked.iter().zip(&frames) {
        assert_eq!(request.dpi, 1200);
        assert_eq!(request.source, scanner_api::Source::Negative);
        let [_, _, w, h] = frame.at.bounds();
        for (got, want) in [(request.area.width, w), (request.area.height, h)] {
            let got = got as f32;
            assert!(
                got >= want * k - 2.0 && got <= (want + 2.0 * margin) * k + 2.0,
                "{got} for {want}"
            );
        }
    }
    // Each developed with what the preview measured.
    for page in &ui.app.pages {
        assert!(page.origin.is_some() && page.whole.is_none());
        assert_eq!(page.raw.tone, scan_imaging::film::TONE);
        assert!(developed_right(page));
    }
}

/// Films, as on the glass: after a scan the preview is still there to go back to, on its own
/// tab, with its frames; another choice of frames is scanned without a new preview, and the
/// "Anteprima" button redoes the preview only when pressed.
#[test]
fn after_a_film_scan_the_preview_is_gone_back_to_without_scanning_it_again() {
    use crate::Tab;
    use iced::keyboard::key::Named;
    let (mut ui, controls) = with_adapter();
    ui.click("Pellicola");
    ui.click("Anteprima");
    assert_eq!(ui.app.shown_regions().len(), 2, "{}", ui.app.note);
    ui.click("Scansiona 2 foto");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    assert_eq!(ui.app.tab, Tab::Result);
    assert_eq!(controls.requests().len(), 3, "the preview, then one each");
    // Two "Anteprima": the button that redoes it, and the tab that shows it.
    let clicks = ui.click_each("Anteprima");
    assert_eq!(clicks.len(), 2, "the preview's tab on films too");
    let tab = clicks
        .iter()
        .find(|m| matches!(m[..], [Message::Tab(Tab::Preview)]))
        .expect("a click on the tab shows the preview")
        .clone();
    for message in tab {
        ui.send(message);
    }
    assert_eq!(ui.app.tab, Tab::Preview);
    assert!(ui.app.preview.is_some());
    assert_eq!(ui.app.shown_regions().len(), 2, "its frames still there");
    assert_eq!(controls.requests().len(), 3, "nothing scanned to show it");
    // The second frame left out, the first scanned again at another resolution: no preview.
    ui.send(Message::Area(crate::areas::AreaMsg::Choose(1)));
    arrow(&mut ui, Named::Delete, NONE);
    ui.send(Message::Save);
    ui.send(Message::Dpi(600));
    ui.click("Scansiona foto");
    let asked = controls.requests();
    assert_eq!(asked.len(), 4, "{asked:?}");
    assert_eq!(asked[3].dpi, 600);
    assert_eq!(ui.app.pages.len(), 1, "{}", ui.app.note);
    // The button above redoes the preview, when pressed.
    ui.send(Message::Save);
    ui.send(Message::Preview);
    ui.settle();
    let asked = controls.requests();
    assert_eq!(asked.len(), 5);
    assert_eq!(asked[4].area, asked[0].area, "the whole holder again");
    assert_eq!(ui.app.tab, Tab::Preview);
}

#[test]
fn on_the_preview_of_films_a_frame_is_left_out_corrected_or_drawn() {
    use crate::areas::AreaMsg;
    use crate::viewer::Selection;
    use iced::keyboard::key::Named;
    let (mut ui, controls) = with_adapter();
    ui.click("Pellicola");
    ui.click("Anteprima");
    assert_eq!(ui.app.shown_regions().len(), 2, "{}", ui.app.note);
    // The second chosen and left out with Delete.
    ui.send(Message::Area(AreaMsg::Choose(1)));
    arrow(&mut ui, Named::Delete, NONE);
    assert_eq!(ui.app.shown_regions().len(), 1);
    assert_eq!(ui.app.chosen, None);
    // The first made smaller by hand: still cut out, at its new size.
    ui.send(Message::Area(AreaMsg::Choose(0)));
    let r = ui.app.chosen_rect().unwrap();
    let smaller = Selection {
        x: r.x + 10.0,
        y: r.y + 10.0,
        width: r.width - 20.0,
        height: r.height - 20.0,
    };
    ui.send(Message::Select(Some(smaller)));
    assert_eq!(ui.app.chosen_rect(), Some(smaller));
    let edited = ui.app.shown_regions()[0];
    assert!(edited.auto && !edited.exact());
    // One drawn on an empty part: a click lets the first go, a drag draws.
    ui.send(Message::Select(None));
    let drawn = Selection {
        x: 20.0,
        y: 20.0,
        width: 100.0,
        height: 60.0,
    };
    ui.send(Message::Select(Some(drawn)));
    assert_eq!(ui.app.shown_regions().len(), 2);
    assert!(ui.app.shown_regions()[1].exact());
    ui.send(Message::Dpi(1200));
    ui.click("Scansiona 2 foto");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    assert_eq!(controls.requests().len(), 3, "the preview, then one each");
    assert_eq!(ui.app.pages[0].origin, Some(edited.at));
    // The area drawn scanned as it is, kept whole.
    let page = &ui.app.pages[1];
    assert!(page.origin.is_none() && page.whole.is_some());
    let k = 1200.0 / f32::from(crate::PREVIEW_DPI);
    let full = &page.raw.full;
    assert!(
        (full.width as f32 - drawn.width * k).abs() <= 2.0,
        "{}",
        full.width
    );
    assert!(
        (full.height as f32 - drawn.height * k).abs() <= 2.0,
        "{}",
        full.height
    );
}

#[test]
fn slides_too_have_their_preview_and_a_scan_each() {
    let (mut ui, controls) = with_slides();
    controls.load(scanner_api::fake::Holder::Slides);
    ui.click("Pellicola");
    ui.send(Message::Holder(crate::Holder::Slides));
    ui.click("Anteprima");
    let n = ui.app.shown_regions().len();
    assert_eq!(n, 2, "{}", ui.app.note);
    ui.click("Scansiona 2 foto");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    let asked = controls.requests();
    assert_eq!(asked.len(), 3, "{asked:?}");
    assert!(asked.iter().all(|r| r.source == scanner_api::Source::Slide));
}

#[test]
fn foto_the_originals_found_are_corrected_and_one_drawn_is_added() {
    use crate::areas::AreaMsg;
    use crate::viewer::Selection;
    let scanner = FakeScanner::flatbed();
    let controls = scanner.controls();
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    assert_eq!(ui.app.shown_regions().len(), 3, "{}", ui.app.note);
    // The first turned by a degree from the panel.
    ui.send(Message::Area(AreaMsg::Choose(0)));
    let before = ui.app.shown_regions()[0].at.angle;
    ui.send(Message::Area(AreaMsg::Tilt(1.0)));
    let turned = ui.app.shown_regions()[0].at;
    assert!(
        (turned.angle - before - 1.0).abs() < 0.06,
        "{}",
        turned.angle
    );
    // One drawn where nothing was found (the lid below the photos).
    ui.send(Message::Select(None));
    let drawn = Selection {
        x: 200.0,
        y: 1800.0,
        width: 600.0,
        height: 400.0,
    };
    ui.send(Message::Select(Some(drawn)));
    assert_eq!(ui.app.chosen, Some(3));
    // With an area drawn, one scan each; the photos found still cut out
    // and straightened, the area drawn as it is.
    ui.click("Scansiona 4 foto");
    assert_eq!(ui.app.pages.len(), 4, "{}", ui.app.note);
    assert_eq!(controls.requests().len(), 5);
    assert_eq!(ui.app.pages[0].origin, Some(turned));
    assert!(ui.app.pages[3].origin.is_none());
    // Without it, the three found in one pass, as before.
    ui.send(Message::DropFound(3));
    ui.app.unsaved = false;
    ui.click("Scansiona 3 foto");
    assert_eq!(ui.app.pages.len(), 3, "{}", ui.app.note);
    assert_eq!(controls.requests().len(), 6, "one pass for the three");
}

#[test]
fn an_original_chosen_in_foto_is_not_moved_from_documento() {
    use crate::viewer::Selection;
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    ui.send(Message::Area(crate::areas::AreaMsg::Choose(0)));
    assert!(ui.app.chosen_rect().is_some());
    // Documento shares the glass but not the automatic crop: the original
    // is hidden, so neither chosen nor moved, and a drag draws a new area.
    ui.send(Message::Kind(Kind::Document));
    assert!(ui.app.shown_regions().is_empty());
    assert_eq!(ui.app.chosen_rect(), None);
    assert!(!ui.app.moving_area());
    let area = Selection {
        x: 100.0,
        y: 100.0,
        width: 300.0,
        height: 200.0,
    };
    ui.send(Message::Select(Some(area)));
    assert_eq!(ui.app.chosen_rect(), Some(area));
    assert_eq!(ui.app.shown_regions().len(), 1);
    // Back in Foto the three found and the area drawn.
    ui.send(Message::Kind(Kind::Photo));
    assert_eq!(ui.app.shown_regions().len(), 4);
}

#[test]
fn photos_picked_are_saved_alone_and_the_others_stay_to_save() {
    use crate::Pick;
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    ui.click("Scansiona 3 foto");
    assert_eq!(ui.app.pages.len(), 3);
    // The first shown; Ctrl+click on the third picks both.
    ui.send(Message::PageShow(0));
    ui.send(Message::PagePick(2, Pick::Toggle));
    let picked = |ui: &Ui| ui.app.pages.iter().map(|p| p.picked).collect::<Vec<_>>();
    assert_eq!(picked(&ui), [true, false, true]);
    assert!(ui.shows("2 scelte"));
    // Ctrl+click again lets it go; Shift+click picks from the one shown.
    ui.send(Message::PagePick(2, Pick::Toggle));
    assert_eq!(picked(&ui), [true, false, false]);
    ui.send(Message::PagePick(1, Pick::Range));
    assert_eq!(picked(&ui), [true, true, false]);
    // Saved: only those two, one file each.
    ui.send(Message::Output(crate::Output::Image(
        vetroscan_export::ImageFormat::Png,
    )));
    ui.send(Message::Save);
    assert_eq!(ui.saved().len(), 2, "{:?} {}", ui.saved(), ui.app.note);
    assert!(ui.app.unsaved, "the third is still to save");
    // Esc lets them go: Save writes all of them again.
    ui.send(Message::Tab(crate::Tab::Result));
    ui.key(iced::keyboard::key::Named::Escape);
    assert_eq!(picked(&ui), [false; 3]);
    // Ctrl+A picks them all.
    ui.send(Message::PickAll(true));
    assert_eq!(picked(&ui), [true; 3]);
    // A plain click lets them go.
    ui.send(Message::PagePress(1));
    ui.send(Message::PageDrop);
    assert_eq!(picked(&ui), [false; 3]);
}

#[test]
fn the_look_of_a_photo_goes_to_the_photos_picked_only() {
    use crate::{Field, Pick};
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    ui.click("Scansiona 3 foto");
    ui.send(Message::PageShow(0));
    ui.send(Message::PagePick(1, Pick::Toggle));
    // The first made brighter in Regola: its own look.
    ui.send(Message::Pane(crate::Pane::Adjust));
    ui.send(Message::Adjust(Field::Brightness, 30.0));
    let shared = *ui.app.settings();
    let brightness = |ui: &Ui, i: usize| ui.app.pages[i].effective(&shared).adjust.brightness;
    assert_eq!(brightness(&ui, 0), 30.0);
    assert!(ui.shows("Applica alle foto scelte (2)"));
    ui.click("Applica alle foto scelte (2)");
    assert_eq!(brightness(&ui, 1), 30.0, "picked");
    assert_eq!(brightness(&ui, 2), shared.adjust.brightness, "not picked");
}
#[test]
fn senza_elaborazioni_on_the_glass_gives_the_scanners_samples_only() {
    use crate::Field;
    let scanner = FakeScanner::with_glass_colour();
    let controls = scanner.controls();
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::AutoCrop(false));
    ui.send(Message::Dpi(300));
    // The scanner's own: its colour correction off, nothing else on.
    ui.send(Message::ScannerColour(false));
    ui.scan(1);
    let bare = ui.app.pages[0].raw.full.data.clone();
    // Senza elaborazioni with the correction and twice the resolution
    // asked: neither is done.
    ui.send(Message::ScannerColour(true));
    ui.send(Message::Double(true));
    ui.send(Message::Untouched(true));
    assert!(ui.shows("Senza elaborazioni"));
    ui.app.unsaved = false;
    ui.scan(1);
    assert_eq!(ui.app.pages.len(), 1, "{}", ui.app.note);
    assert_eq!(controls.requests().last().unwrap().dpi, 300, "not doubled");
    let page = &ui.app.pages[0];
    assert!(
        page.raw.full.data == bare,
        "the scanner's samples, untouched"
    );
    // No sharpening, and Regola changes nothing on it.
    let shared = *ui.app.settings();
    assert_eq!(page.effective(&shared).adjust.sharpen, 0.0);
    let before = page.finished(&shared).data;
    ui.send(Message::Pane(crate::Pane::Adjust));
    assert!(ui.shows(
        "Senza elaborazioni: le regolazioni qui non hanno effetto (si tolgono in Scansione)."
    ));
    ui.send(Message::Adjust(Field::Brightness, 40.0));
    let shared = *ui.app.settings();
    assert!(ui.app.pages[0].finished(&shared).data == before);
}

#[test]
fn senza_elaborazioni_inverts_a_negative_only_or_leaves_it_as_it_is() {
    let (mut ui, _) = with_adapter();
    ui.click("Pellicola");
    ui.send(Message::Dpi(600));
    ui.send(Message::Untouched(true));
    // The choice for negatives shows only "Senza elaborazioni".
    assert!(ui.shows("Lascia i negativi come sono"));
    ui.send(Message::KeepNegative(true));
    // The preview: frames found, nothing measured.
    ui.click("Anteprima");
    assert_eq!(ui.app.shown_regions().len(), 2, "{}", ui.app.note);
    assert!(ui.app.measures.is_none());
    ui.click("Scansiona 2 foto");
    let kept: Vec<Vec<u16>> = ui
        .app
        .pages
        .iter()
        .map(|p| p.raw16.as_ref().unwrap().to_vec())
        .collect();
    // Left as it is: the negative, orange base and all (red the most
    // light, blue the least, as the made-up film's base).
    let mean = |v: &[u16], c: usize| {
        v.iter()
            .skip(c)
            .step_by(3)
            .map(|&x| f64::from(x))
            .sum::<f64>()
            / (v.len() / 3) as f64
    };
    assert!(mean(&kept[0], 0) > mean(&kept[0], 2), "an orange negative");
    // Inverted: each sample one less the negative's at gamma 2.2 (within
    // what straightening the frame mixes between neighbouring pixels).
    ui.send(Message::KeepNegative(false));
    ui.app.unsaved = false;
    ui.click("Scansiona 2 foto");
    let inverted = ui.app.pages[0].raw16.as_ref().unwrap().to_vec();
    assert_eq!(inverted.len(), kept[0].len());
    let close = inverted
        .iter()
        .zip(&kept[0])
        .filter(|&(&got, &negative)| {
            let t = f64::from(negative) / 65535.0;
            let want = (1.0 - t.powf(1.0 / 2.2)).powf(2.2) * 65535.0;
            (f64::from(got) - want).abs() <= 400.0
        })
        .count();
    assert!(
        close * 100 >= inverted.len() * 95,
        "{close} of {}",
        inverted.len()
    );
    assert!(
        mean(&inverted, 2) > mean(&inverted, 0),
        "blue now the most light"
    );
    // Developed as usual once it is off.
    ui.send(Message::Untouched(false));
    ui.click("Anteprima");
    assert!(ui.app.measures.is_some());
}
/// The mean of each channel of a page's 16-bit picture.
fn channel_means(page: &crate::Page) -> [f64; 3] {
    let deep = page.raw16.as_ref().expect("16-bit samples");
    let n = (deep.len() / 3) as f64;
    std::array::from_fn(|c| {
        deep.iter()
            .skip(c)
            .step_by(3)
            .map(|&v| f64::from(v))
            .sum::<f64>()
            / n
    })
}

#[test]
fn two_strips_of_two_rolls_are_each_developed_with_their_own_measures() {
    let scanner = FakeScanner::with_two_strips();
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.click("Pellicola");
    ui.click("Anteprima");
    // Two strips found and measured on their own: their bases as the two
    // rolls' (the made-up film's).
    let mm = 25.4 / f32::from(crate::PREVIEW_DPI);
    let found: Vec<(f32, f32, f32, f32)> = ui
        .app
        .shown_regions()
        .iter()
        .map(|r| {
            (
                r.at.cx * mm,
                r.at.cy * mm,
                r.at.width * mm,
                r.at.height * mm,
            )
        })
        .collect();
    assert_eq!(found.len(), 4, "{found:?} {}", ui.app.note);
    assert_eq!(ui.app.strips.len(), 2);
    let bases: Vec<[f32; 3]> = ui.app.strips.iter().map(|(_, m)| m.base).collect();
    let near = |a: [f32; 3], b: [f64; 3]| (0..3).all(|c| (f64::from(a[c]) - b[c]).abs() < 0.02);
    assert!(
        bases.iter().any(|&b| near(b, scanner_api::fake::BASE))
            && bases
                .iter()
                .any(|&b| near(b, scanner_api::fake::OTHER_BASE)),
        "{bases:?}"
    );
    // Each frame: its strip's base, its own white.
    assert_eq!(ui.app.frames.len(), 4);
    for (b, m) in &ui.app.frames {
        let (cx, cy) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
        let (sb, sm) = ui
            .app
            .strips
            .iter()
            .find(|(s, _)| cx >= s[0] && cx <= s[0] + s[2] && cy >= s[1] && cy <= s[1] + s[3])
            .expect("a frame in a strip");
        assert_eq!(m.base, sm.base, "{b:?} in {sb:?}");
    }
    // Each frame scanned on its own, with its own measures: the same
    // scene on the two rolls comes out the same.
    ui.click("Scansiona 4 foto");
    assert_eq!(ui.app.pages.len(), 4, "{}", ui.app.note);
    let left: Vec<[f64; 3]> = ui
        .app
        .pages
        .iter()
        .filter(|p| p.origin.is_some_and(|o| o.cx < ui.app.glass().width / 2.0))
        .map(channel_means)
        .collect();
    let right: Vec<[f64; 3]> = ui
        .app
        .pages
        .iter()
        .filter(|p| p.origin.is_some_and(|o| o.cx >= ui.app.glass().width / 2.0))
        .map(channel_means)
        .collect();
    assert_eq!((left.len(), right.len()), (2, 2));
    for (l, r) in left.iter().zip(&right) {
        for c in 0..3 {
            assert!(
                (l[c] - r[c]).abs() < 0.03 * 65535.0,
                "channel {c}: {l:?} against {r:?}"
            );
        }
    }
    // Scanned whole without a preview: each strip measured on its own too.
    ui.app.unsaved = false;
    ui.send(Message::Kind(Kind::Document));
    let mut ui2 = Ui::with_scanners(vec![FakeScanner::with_two_strips()]);
    ui2.click("Pellicola");
    ui2.click("Scansiona foto");
    assert_eq!(ui2.app.pages.len(), 4, "{}", ui2.app.note);
    let means: Vec<[f64; 3]> = ui2.app.pages.iter().map(channel_means).collect();
    for c in 0..3 {
        let (lo, hi) = means
            .iter()
            .map(|m| m[c])
            .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
        assert!(hi - lo < 0.03 * 65535.0, "channel {c}: {means:?}");
    }
}
#[test]
fn the_screen_shows_a_film_as_its_file_holds_it() {
    // A frame's shadows drawn from 8-bit linear samples came out darker on
    // the screen than in the file (up to 20 levels on the Epson 4490's
    // frames): the screen is drawn from the 16-bit samples, as the file.
    let (mut ui, _) = with_adapter();
    ui.click("Pellicola");
    ui.click("Anteprima");
    let preview = ui.app.preview.as_ref().expect("a preview");
    assert!(preview.0.proxy16.is_some(), "the preview keeps its 16 bits");
    ui.click("Scansiona 2 foto");
    let same = |ui: &Ui| {
        let shared = *ui.app.settings();
        let page = &ui.app.pages[0];
        let file = page.finished(&shared);
        let shown = &page.shown;
        assert_eq!((shown.width, shown.height), (file.width, file.height));
        let worst = shown
            .data
            .iter()
            .zip(&file.data)
            .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
            .max()
            .unwrap_or(0);
        assert!(
            worst <= 1,
            "the screen against the file: up to {worst} levels"
        );
    };
    same(&ui);
    // Turned: the same.
    ui.send(Message::PageRotate(0, true));
    ui.settle();
    same(&ui);
}

#[test]
fn documento_scans_each_area_drawn_as_a_page() {
    use crate::viewer::Selection;
    let scanner = FakeScanner::flatbed();
    let controls = scanner.controls();
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.send(Message::Kind(Kind::Document));
    ui.click("Anteprima");
    let areas = [
        Selection {
            x: 100.0,
            y: 100.0,
            width: 700.0,
            height: 500.0,
        },
        Selection {
            x: 100.0,
            y: 1200.0,
            width: 900.0,
            height: 700.0,
        },
    ];
    for area in areas {
        ui.send(Message::Select(None));
        ui.send(Message::Select(Some(area)));
    }
    assert_eq!(ui.app.shown_regions().len(), 2);
    ui.click("Scansiona 2 pagine");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    let asked = controls.requests();
    assert_eq!(asked.len(), 3, "the preview, then one each");
    let dpi = asked[1].dpi;
    let k = f32::from(dpi) / f32::from(crate::PREVIEW_DPI);
    for (request, area) in asked[1..].iter().zip(areas) {
        assert!((request.area.x as f32 - area.x * k).abs() <= 2.0);
        assert!((request.area.width as f32 - area.width * k).abs() <= 2.0);
    }
    // "Rifai" scans a page's own area again, whatever is chosen now.
    ui.send(Message::DropFound(0));
    ui.send(Message::PageRedo(0));
    ui.settle();
    let last = *controls.requests().last().unwrap();
    assert_eq!(last.area, asked[1].area);
}

#[test]
fn separa_cuts_the_frames_out_of_a_film_scanned_whole() {
    // Where the scan with the automatic crop finds the frames.
    let frames: Vec<Found> = {
        let (mut ui, _) = with_adapter();
        ui.click("Pellicola");
        ui.send(Message::Dpi(200));
        ui.click("Scansiona foto");
        ui.app.pages.iter().filter_map(|p| p.origin).collect()
    };
    assert_eq!(frames.len(), 2);
    let (mut ui, _) = with_adapter();
    ui.click("Pellicola");
    ui.send(Message::Dpi(200));
    // The automatic crop off: one picture, the whole adapter's area.
    ui.send(Message::AutoCrop(false));
    ui.click("Scansiona foto");
    assert_eq!(ui.app.pages.len(), 1);
    assert!(ui.app.pages[0].whole.is_some());
    // Cut afterwards, without scanning again: first the frames shown on
    // the scan, then the cut.
    ui.click("Separa i fotogrammi");
    assert_eq!(ui.app.pages.len(), 1);
    assert_eq!(ui.app.separating.as_ref().map(|s| s.found.len()), Some(2));
    ui.click("Separa in 2");
    assert!(ui.app.separating.is_none());
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    assert!(ui.app.note.contains("2 immagini"), "{}", ui.app.note);
    let origins: Vec<Found> = ui.app.pages.iter().filter_map(|p| p.origin).collect();
    assert!(
        same_places(&origins, &frames, 3.0),
        "{origins:?} {frames:?}"
    );
    for page in &ui.app.pages {
        assert_eq!(page.raw.tone, scan_imaging::film::TONE);
        assert!(page.raw16.is_some());
        let full = &page.raw.full;
        assert!(full.height > full.width, "{} x {}", full.width, full.height);
    }
    // Cut once: nothing more to separate.
    assert!(!ui.shows("Separa i fotogrammi"));
}

#[test]
fn separa_finds_the_photos_of_an_area_scanned_whole() {
    use crate::MmField;
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    let originals = found_by_the_preview(&mut ui);
    // An area chosen by hand, the originals found left aside: a single
    // picture.
    ui.send(Message::AutoCrop(false));
    for (field, mm) in [(0, "10"), (1, "10"), (2, "200"), (3, "240")] {
        ui.send(Message::MmInput(MmField::Area(field), mm.into()));
    }
    ui.send(Message::MmSubmit);
    assert!(ui.app.chosen_rect().is_some());
    ui.click("Scansiona foto");
    assert_eq!(ui.app.pages.len(), 1);
    ui.click("Separa le foto");
    ui.key(iced::keyboard::key::Named::Enter);
    assert_eq!(ui.app.pages.len(), 3, "{}", ui.app.note);
    // Found where the preview found them on the glass, though the scan
    // started 10 mm in: "Rifai" on each finds its place.
    let origins: Vec<Found> = ui.app.pages.iter().filter_map(|p| p.origin).collect();
    assert!(
        same_places(&origins, &originals, 3.0),
        "{origins:?} {originals:?}"
    );
}

#[test]
fn separa_shows_the_areas_first_to_correct_and_cuts_on_ok() {
    use crate::separa::SplitMsg;
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::AutoCrop(false));
    ui.scan(1);
    assert_eq!(ui.app.pages.len(), 1);
    let whole = scans(&ui);
    // Annulla: the scan stays whole, as Esc.
    ui.click("Separa le foto");
    let found = ui.app.separating.as_ref().map_or(0, |s| s.found.len());
    assert_eq!(found, scanner_api::fake::ORIGINALS.len(), "{}", ui.app.note);
    assert!(ui.shows(&format!("Separa in {found}")));
    ui.key(iced::keyboard::key::Named::Escape);
    assert!(ui.app.separating.is_none());
    assert_eq!(scans(&ui), whole);
    // One left out, one chosen and made smaller and upright, one drawn.
    ui.click("Separa le foto");
    ui.click("Escludi");
    let sel = |x: f32, y: f32, w: f32, h: f32| crate::Selection {
        x,
        y,
        width: w,
        height: h,
    };
    ui.send(SplitMsg::Choose(0).into());
    let side = ui.app.pages[0].raw.full.width as f32;
    ui.send(SplitMsg::Area(Some(sel(0.1 * side, 0.1 * side, 0.15 * side, 0.1 * side))).into());
    ui.send(SplitMsg::Tilt(1.0).into());
    ui.send(SplitMsg::Upright.into());
    ui.send(SplitMsg::Area(None).into());
    ui.send(SplitMsg::Area(Some(sel(0.5 * side, 0.6 * side, 0.2 * side, 0.2 * side))).into());
    let sep = ui.app.separating.as_ref().expect("still separating");
    assert_eq!(sep.found.len(), found, "one out, one in");
    assert_eq!(sep.chosen, Some(found - 1), "the area drawn is chosen");
    assert_eq!(sep.found[0].angle, 0.0);
    // A slip of the mouse is not an area.
    ui.send(SplitMsg::Area(None).into());
    ui.send(SplitMsg::Area(Some(sel(10.0, 10.0, 3.0, 3.0))).into());
    assert_eq!(ui.app.separating.as_ref().unwrap().found.len(), found);
    ui.click(&format!("Separa in {found}"));
    assert_eq!(ui.app.pages.len(), found, "{}", ui.app.note);
    assert!(ui.app.unsaved);
    // The first one as it was made smaller, at the scale of the scan.
    let first = &ui.app.pages[0].raw.full;
    assert!(
        (first.width as f32 - 0.15 * side).abs() < 0.03 * side,
        "{} of {side}",
        first.width
    );
}

#[test]
fn separa_with_nothing_found_lets_an_area_be_drawn() {
    use crate::separa::SplitMsg;
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::AutoCrop(false));
    ui.scan(1);
    ui.click("Separa le foto");
    // Everything found left out: nothing to cut, the button waits.
    while ui
        .app
        .separating
        .as_ref()
        .is_some_and(|s| !s.found.is_empty())
    {
        ui.send(SplitMsg::Drop(0).into());
    }
    assert!(ui.shows("Nessuna area: trascina sull'immagine per aggiungerne una."));
    ui.key(iced::keyboard::key::Named::Enter);
    assert!(ui.app.separating.is_some(), "nothing cut");
    assert_eq!(ui.app.pages.len(), 1);
    let side = ui.app.pages[0].raw.full.width as f32;
    ui.send(
        SplitMsg::Area(Some(crate::Selection {
            x: 0.2 * side,
            y: 0.2 * side,
            width: 0.3 * side,
            height: 0.2 * side,
        }))
        .into(),
    );
    ui.click("Ritaglia");
    assert_eq!(ui.app.pages.len(), 1);
    assert!(ui.app.separating.is_none());
    assert!(ui.app.pages[0].origin.is_some(), "cut out of the scan");
}

/// The made-up flatbed with stripes on its glass (3 %), in Documento at
/// 300 dpi on a stretch of the lid with nothing on it.
fn striped_lid() -> Ui {
    use crate::MmField;
    let scanner = FakeScanner::flatbed();
    scanner.controls().stripes(30);
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.send(Message::Dpi(300));
    for (field, mm) in [(0, "10"), (1, "235"), (2, "190"), (3, "50")] {
        ui.send(Message::MmInput(MmField::Area(field), mm.into()));
    }
    ui.send(Message::MmSubmit);
    ui
}

/// How much the lines of the last page differ from their neighbours, as a
/// fraction (on a uniform lid: the stripes).
fn stripes_of_last_page(ui: &Ui) -> f32 {
    let full = &ui.app.pages.last().expect("a page").raw.full;
    let w = full.width;
    let means: Vec<f32> = (0..full.height)
        .map(|y| {
            let row = &full.data[y * w * 3..(y + 1) * w * 3];
            row.iter().map(|&v| f32::from(v)).sum::<f32>() / row.len() as f32
        })
        .collect();
    let k = 7;
    let n = means.len() - 2 * k;
    let sum: f32 = (k..means.len() - k)
        .map(|y| {
            let smooth = means[y - k..=y + k].iter().sum::<f32>() / (2 * k + 1) as f32;
            (means[y] / smooth - 1.0).powi(2)
        })
        .sum();
    (sum / n as f32).sqrt()
}

/// How strong the part of the last page's lines is that repeats after
/// `repeat` lines: each line against its neighbours, averaged by its place
/// in the pattern (the subject of a picture averages out, stripes do not).
fn repeating_every(ui: &Ui, repeat: usize) -> f32 {
    let full = &ui.app.pages.last().expect("a page").raw.full;
    let w = full.width;
    let means: Vec<f32> = (0..full.height)
        .map(|y| {
            let row = &full.data[y * w * 3..(y + 1) * w * 3];
            row.iter().map(|&v| f32::from(v)).sum::<f32>() / row.len() as f32
        })
        .collect();
    let k = 7;
    let mut bins = vec![(0.0f32, 0usize); repeat];
    for y in k..means.len() - k {
        let smooth = means[y - k..=y + k].iter().sum::<f32>() / (2 * k + 1) as f32;
        let b = &mut bins[y % repeat];
        b.0 += means[y] / smooth.max(1.0) - 1.0;
        b.1 += 1;
    }
    let avg: Vec<f32> = bins.iter().map(|(s, n)| s / (*n).max(1) as f32).collect();
    let mean = avg.iter().sum::<f32>() / repeat as f32;
    (avg.iter().map(|a| (a - mean).powi(2)).sum::<f32>() / repeat as f32).sqrt()
}

#[test]
fn the_scanners_stripes_go_with_a_tick() {
    let mut ui = striped_lid();
    assert!(ui.shows("Togli le righe dello scanner"));
    ui.send(Message::Scan);
    ui.settle();
    let with = stripes_of_last_page(&ui);
    assert!(with > 0.015, "{with}");
    ui.click("Togli le righe dello scanner");
    assert!(ui.app.settings().destripe);
    ui.send(Message::Scan);
    ui.settle();
    let without = stripes_of_last_page(&ui);
    assert!(without < with / 5.0, "{with} then {without}");
    // Remembered with the way of working.
    assert_eq!(ui.app.prefs().document.destripe, Some(true));
    // 2400 dpi: none measured there, no tick.
    ui.send(Message::Dpi(2400));
    assert!(!ui.shows("Togli le righe dello scanner"));
}

#[test]
fn twice_the_resolution_then_halved() {
    let mut ui = striped_lid();
    ui.send(Message::Scan);
    ui.settle();
    let plain = &ui.app.pages[0].raw.full;
    let (w, h) = (plain.width, plain.height);
    assert_eq!(plain.dpi, 300);
    assert!(ui.shows("Scansiona a 600 dpi e riduci"));
    ui.click("Scansiona a 600 dpi e riduci");
    ui.send(Message::Scan);
    ui.settle();
    let page = &ui.app.pages[1];
    let full = &page.raw.full;
    assert_eq!(full.dpi, 300, "brought back to the resolution chosen");
    assert!(
        full.width.abs_diff(w) <= 1 && full.height.abs_diff(h) <= 1,
        "{w} x {h} then {} x {}",
        full.width,
        full.height
    );
    assert!(!ui.app.doubled);
    // The 16-bit samples halved with it (Foto keeps them).
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::Dpi(300));
    ui.send(Message::Depth(crate::Depth(16)));
    ui.send(Message::Double(true));
    ui.send(Message::Scan);
    ui.settle();
    let page = ui.app.pages.last().unwrap();
    let full = &page.raw.full;
    assert_eq!(full.dpi, 300);
    assert_eq!(
        page.raw16.as_ref().map(|s| s.len()),
        Some(full.width * full.height * 3)
    );
    // 2400 dpi: nothing twice as fine to ask for.
    ui.send(Message::Dpi(2400));
    assert!(!ui.shows("e riduci"));
}

#[test]
fn misura_le_righe_finds_them_on_a_white_sheet_and_keeps_them() {
    let scanner = FakeScanner::flatbed();
    let controls = scanner.controls();
    controls.stripes(30);
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.send(Message::Pane(crate::Pane::Prefs));
    assert!(ui.shows("300 dpi: ogni 10 righe (dal modello)"));
    // Asked first (a white sheet on the glass), then a strip at each
    // resolution up to 1200 dpi.
    ui.send(Message::MeasureStripes);
    assert!(ui.app.confirm.is_some() && ui.app.busy.is_none());
    ui.click("Misura");
    let name = ui.app.caps.name();
    let found: Vec<(u16, u16)> = ui.app.measured[&name]
        .iter()
        .map(|s| (s.dpi, s.repeat))
        .collect();
    assert_eq!(found, [(200, 5), (300, 10), (600, 5), (1200, 5)]);
    assert!(ui.app.note.contains("Righe misurate"), "{}", ui.app.note);
    let at300 = &ui.app.measured[&name][1];
    assert!(at300.percent > 1.0, "{at300:?}");
    assert!(ui.shows(&format!(
        "300 dpi: ogni 10 righe, {} % (misurate)",
        crate::i18n::decimal(format!("{:.1}", at300.percent))
    )));
    assert_eq!(ui.app.prefs().stripes[&name].len(), 4);
    // A scanner without stripes: measured, none; the tick goes.
    controls.stripes(0);
    ui.send(Message::MeasureStripes);
    ui.click("Misura");
    assert!(ui.app.measured[&name].iter().all(|s| s.repeat == 0));
    assert!(ui.shows("300 dpi: nessuna (misurate)"));
    ui.send(Message::Kind(Kind::Document));
    ui.send(Message::Dpi(300));
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(!ui.shows("Togli le righe dello scanner"));
    // Back to the model's.
    ui.send(Message::ForgetStripes);
    assert!(!ui.app.measured.contains_key(&name));
    assert!(ui.shows("Togli le righe dello scanner"));
}

#[test]
fn films_have_the_stripes_tick_but_not_twice_the_resolution() {
    let (mut ui, _) = with_adapter();
    ui.click("Pellicola");
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(ui.shows("Togli le righe dello scanner"));
    assert!(!ui.shows("Scansiona a 2400 dpi e riduci"));
    ui.send(Message::Dpi(600));
    assert!(!ui.shows("Scansiona a 1200 dpi e riduci"));
    ui.send(Message::Double(true));
    assert!(!ui.app.doubles());
}

#[test]
fn films_have_the_stripes_tick_only_where_the_scanner_declares_stripes_on_films() {
    use scanner_api::{Scanner, Source};
    // An adapter with a lamp of its own, which leaves no stripes on films
    // (as the Epson 4490's may): the glass's stripes do not count there.
    let mut caps = FakeScanner::with_adapter().capabilities().clone();
    caps.stripes.retain(|s| s.source == Source::Glass);
    let mut ui = Ui::with_scanners(vec![FakeScanner::with(caps)]);
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(ui.shows("Togli le righe dello scanner"), "on the glass");
    ui.click("Pellicola");
    assert!(!ui.shows("Togli le righe dello scanner"), "not on films");
}

#[test]
fn films_reduce_the_colour_noise_by_default_and_remember_the_tick() {
    let (mut ui, _) = with_adapter();
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(!ui.shows("Riduci il rumore di colore"), "not on the glass");
    ui.click("Pellicola");
    assert!(ui.shows("Riduci il rumore di colore"));
    assert!(ui.app.settings().colour_noise, "on by default for films");
    // A film still comes out with it (the made-up one has little noise).
    ui.send(Message::Scan);
    ui.settle();
    assert!(!ui.app.pages.is_empty(), "{}", ui.app.note);
    ui.click("Riduci il rumore di colore");
    assert!(!ui.app.settings().colour_noise);
    assert_eq!(ui.app.prefs().film.colour_noise, Some(false));
}

#[test]
fn the_base_of_a_roll_is_locked_used_and_remembered() {
    let lock = "Blocca la base dell'ultima striscia";
    let (mut ui, _) = with_adapter();
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(!ui.shows("Rullino"), "not on the glass");
    ui.click("Pellicola");
    ui.send(Message::Dpi(200));
    assert!(ui.shows("Rullino"));
    // Nothing scanned yet: nothing to lock.
    assert!(ui.click_messages(lock).is_empty());
    // A strip: its own base, from the gaps between its frames.
    ui.click("Scansiona foto");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    let own = ui.app.strip_base.expect("the base between the frames");
    let first: Vec<Vec<u16>> = ui
        .app
        .pages
        .iter()
        .map(|p| p.raw16.as_deref().expect("16 bits").to_vec())
        .collect();
    ui.click(lock);
    assert_eq!(ui.app.roll_base, Some(own));
    assert!(ui.shows("Nuovo rullino"));
    assert!(!ui.shows(lock));
    assert!(ui.app.note.contains("bloccata"), "{}", ui.app.note);
    assert_eq!(ui.app.prefs().film.roll_base, Some(own));
    // The next strip of the roll: the same base, the same pictures (the
    // made-up strip is the same each time).
    ui.send(Message::Scan);
    assert_eq!(ui.question(), Some("Fare una nuova scansione?"));
    ui.click("Scansiona");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    let again: Vec<Vec<u16>> = ui
        .app
        .pages
        .iter()
        .map(|p| p.raw16.as_deref().expect("16 bits").to_vec())
        .collect();
    assert_eq!(again, first);
    // A base locked on another strip of the roll, a little denser: it is
    // the one used, not this strip's own (which is not measured again).
    let other = own.map(|b| b * 0.85);
    ui.app.roll_base = Some(other);
    ui.send(Message::Scan);
    ui.click("Scansiona");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    assert_ne!(
        ui.app.pages[0].raw16.as_deref().expect("16 bits"),
        &first[0][..]
    );
    assert_eq!(ui.app.strip_base, Some(own));
    // Remembered: the app opened again has it.
    let (mut reopened, _) = with_adapter();
    assert_eq!(reopened.app.roll_base, Some(other));
    reopened.click("Pellicola");
    assert!(reopened.shows("Nuovo rullino"));
    // "Nuovo rullino": each strip its own base again, forgotten too.
    ui.click("Nuovo rullino");
    assert_eq!(ui.app.roll_base, None);
    assert_eq!(ui.app.prefs().film.roll_base, None);
    assert!(ui.shows("Rullino") && ui.shows(lock));
    assert!(
        ui.click_messages(lock).is_empty(),
        "the last strip was the old roll's"
    );
}

/// Colour and black and white negatives are two films, with bases far apart (orange, near
/// grey): switching between them forgets what was measured on the other (its preview, measures,
/// frames and the last strip's base, which kept gave the colour frames a blue cast), and a
/// roll's base locked on one is unlocked, saying so.
#[test]
fn switching_between_colour_and_bw_negatives_forgets_the_other_film() {
    let lock = "Blocca la base dell'ultima striscia";
    let (mut ui, _) = with_adapter();
    ui.send(Message::Pane(crate::Pane::Scan));
    ui.click("Pellicola");
    ui.click("Anteprima");
    assert!(ui.app.measures.is_some(), "{}", ui.app.note);
    assert!(!ui.app.frames.is_empty() && !ui.app.shown_regions().is_empty());
    let base = ui.app.strip_base.expect("the base between the frames");
    ui.click(lock);
    assert_eq!(ui.app.roll_base, Some(base));
    // Black and white negatives in the holder: another film, another roll.
    ui.send(Message::Holder(crate::Holder::BwNegatives));
    assert!(ui.app.measures.is_none() && ui.app.preview.is_none());
    assert!(ui.app.frames.is_empty() && ui.app.strips.is_empty());
    assert!(ui.app.shown_regions().is_empty());
    assert_eq!(ui.app.strip_base, None);
    assert_eq!(ui.app.roll_base, None);
    assert_eq!(ui.app.prefs().film.roll_base, None);
    assert!(ui.app.note.contains("bianco e nero"), "{}", ui.app.note);
    assert!(ui.app.note.contains("sbloccata"), "{}", ui.app.note);
    assert!(ui.click_messages(lock).is_empty(), "nothing of it to lock");
    // Its own preview: its own base.
    ui.click("Anteprima");
    assert!(ui.app.measures.is_some(), "{}", ui.app.note);
    assert!(ui.app.strip_base.is_some());
    // Colour again: forgotten again; nothing was locked, so nothing said of it.
    ui.send(Message::Holder(crate::Holder::Negatives));
    assert!(ui.app.measures.is_none() && ui.app.preview.is_none());
    assert_eq!(ui.app.strip_base, None);
    assert!(!ui.app.note.contains("sbloccata"), "{}", ui.app.note);
}

/// "Blocca la base dell'ultima striscia" locks only the strip just measured: a preview of a
/// strip with no gap between its frames leaves no base to lock, not the one before's.
#[test]
fn a_strip_without_gaps_leaves_no_base_to_lock() {
    let lock = "Blocca la base dell'ultima striscia";
    let (mut ui, controls) = with_slides();
    controls.load(scanner_api::fake::Holder::Strip);
    ui.send(Message::Pane(crate::Pane::Scan));
    ui.click("Pellicola");
    assert_eq!(ui.app.film_kind(), scan_imaging::film::Film::Negative);
    ui.click("Anteprima");
    assert!(ui.app.strip_base.is_some(), "{}", ui.app.note);
    assert!(!ui.click_messages(lock).is_empty());
    // The slides' mounts read as negatives: no bare film between frames.
    controls.load(scanner_api::fake::Holder::Slides);
    ui.click("Anteprima");
    assert_eq!(ui.app.strip_base, None, "{}", ui.app.note);
    assert!(ui.click_messages(lock).is_empty());
    // "Senza elaborazioni": the strip seen, not measured, no base either.
    controls.load(scanner_api::fake::Holder::Strip);
    ui.click("Anteprima");
    assert!(ui.app.strip_base.is_some(), "{}", ui.app.note);
    assert!(ui.app.measures.is_some());
    ui.send(Message::Untouched(true));
    ui.click("Anteprima");
    assert_eq!(ui.app.strip_base, None);
    // Nor the strip before's measures, which a scan developed after would use.
    assert!(ui.app.measures.is_none() && ui.app.frames.is_empty());
    assert!(
        !ui.app.note.contains("nessuna pellicola"),
        "{}",
        ui.app.note
    );
}

/// The made-up flatbed whose adapter takes slides and negatives, two
/// slides in its holder, and the handle on it.
fn with_slides() -> (Ui, scanner_api::fake::Controls) {
    let scanner = FakeScanner::with_slides();
    let controls = scanner.controls();
    (Ui::with_scanners(vec![scanner]), controls)
}

/// The grey of a picture's middle row at `x`.
fn grey_at(full: &scan_imaging::Image, x: usize) -> u32 {
    let i = (full.height / 2 * full.width + x) * 3;
    full.data[i..i + 3].iter().map(|&v| u32::from(v)).sum()
}

#[test]
fn slides_are_chosen_scanned_as_positives_and_remembered() {
    use scanner_api::Source;
    let (mut ui, controls) = with_slides();
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(!ui.shows("Nel supporto"), "not on the glass");
    ui.click("Pellicola");
    ui.send(Message::Dpi(200));
    // Negatives first, as before; the choice is there.
    assert_eq!(ui.app.request().source, Source::Negative);
    assert!(ui.shows("Nel supporto") && ui.shows("Rullino"));
    ui.send(Message::Holder(crate::Holder::Slides));
    assert_eq!(ui.app.film_kind(), scan_imaging::film::Film::Slide);
    let request = ui.app.request();
    assert_eq!(request.source, Source::Slide);
    assert_eq!(request.depth, 16, "a slide is stretched from 16 bits");
    assert!(!ui.shows("Rullino"), "a slide has no base to lock");
    assert!(ui.app.note.contains("Diapositive"), "{}", ui.app.note);
    assert!(ui.app.prefs().film.slides);
    // One scan, a picture for each slide, lying as in the holder.
    ui.click("Scansiona foto");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    for page in &ui.app.pages {
        let full = &page.raw.full;
        assert_eq!(page.raw.tone, scan_imaging::film::TONE);
        assert!(full.width > full.height, "{} x {}", full.width, full.height);
        // The made-up scene lets more light through to the right: a
        // positive, brighter there (a negative's inversion would make it
        // darker).
        let (left, right) = (
            grey_at(full, full.width / 5),
            grey_at(full, full.width * 4 / 5),
        );
        assert!(right > left + 60, "{left} then {right}");
        // Stretched to the full range: the brightest film is near white.
        let deep = page.raw16.as_deref().expect("16 bits");
        assert!(deep.iter().copied().max().unwrap() > 60000);
    }
    assert!(ui.app.measures.is_none(), "no preview for films");
    // Remembered: the app opened again scans slides.
    let (mut reopened, _) = with_slides();
    assert!(reopened.app.slides);
    reopened.click("Pellicola");
    assert_eq!(reopened.app.request().source, Source::Slide);
    // Back to negatives (the strip put in the holder): the slides scanned
    // stay, the next scan is of the strip, in their place when asked.
    ui.send(Message::Holder(crate::Holder::Negatives));
    assert_eq!(ui.app.request().source, Source::Negative);
    assert_eq!(ui.app.pages.len(), 2);
    assert!(ui.shows("Rullino"));
    assert!(!ui.app.prefs().film.slides);
    controls.load(scanner_api::fake::Holder::Strip);
    ui.send(Message::Scan);
    assert_eq!(ui.question(), Some("Fare una nuova scansione?"));
    ui.click("Scansiona");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    for page in &ui.app.pages {
        let full = &page.raw.full;
        assert!(full.height > full.width, "the strip's frames stand up");
    }
    assert!(ui.app.strip_base.is_some(), "the strip's base");
    // Not during a scan: the choice stays.
    ui.app.busy = Some((crate::Purpose::Page, None));
    ui.send(Message::Holder(crate::Holder::Slides));
    assert!(!ui.app.slides);
}

/// The scanner's correction of the slides' colours: offered with slides
/// only, on by default, remembered, and the pictures more colourful with
/// it (the made-up matrix spreads the colours).
#[test]
fn the_scanners_colour_correction_is_offered_for_slides() {
    let tick = "Correggi i colori dello scanner";
    // The warm slides' red above their blue, over both pictures.
    let scan = |on: bool| {
        let (mut ui, _) = with_slides();
        ui.send(Message::Pane(crate::Pane::Scan));
        ui.click("Pellicola");
        assert!(!ui.shows(tick), "none declared for negatives");
        ui.send(Message::Holder(crate::Holder::Slides));
        assert!(ui.shows(tick));
        assert!(ui.shows("Togli le righe dello scanner"), "slides' stripes");
        assert!(ui.app.settings().scanner_colour, "on by default");
        ui.send(Message::ScannerColour(on));
        assert_eq!(ui.app.prefs().film.scanner_colour, Some(on));
        ui.click("Scansiona foto");
        assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
        let (mut spread, mut n) = (0i64, 0i64);
        for page in &ui.app.pages {
            let deep = page.raw16.as_deref().expect("16 bits");
            for p in deep.chunks_exact(3) {
                spread += i64::from(p[0]) - i64::from(p[2]);
                n += 1;
            }
        }
        spread / n
    };
    let (without, with) = (scan(false), scan(true));
    assert!(without > 0, "warm slides: {without}");
    assert!(with > without + 500, "{without} then {with}");
    // An adapter declaring no correction: no tick.
    let (mut ui, _) = with_adapter();
    ui.send(Message::Pane(crate::Pane::Scan));
    ui.click("Pellicola");
    assert!(ui.app.filming() && !ui.shows(tick));
    assert_eq!(ui.app.colour_correction(), None);
}

/// The scanner's correction of the glass's colours: offered where it declares one, on by
/// default, kept per way of working, and the picture more colourful with it, at 8 and 16
/// bits; a scanner declaring none has no tick.
#[test]
fn the_scanners_colour_correction_is_offered_for_the_glass() {
    let tick = "Correggi i colori dello scanner";
    // How far apart the channels are, over the picture.
    let spread = |data: &[u8]| -> f64 {
        let (mut s, mut n) = (0.0, 0.0);
        for p in data.chunks_exact(3) {
            let m = (f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])) / 3.0;
            s += p.iter().map(|&v| (f64::from(v) - m).abs()).sum::<f64>();
            n += 1.0;
        }
        s / n
    };
    let scan = |on: bool, depth: u8| {
        let mut ui = Ui::with_scanners(vec![FakeScanner::with_glass_colour()]);
        ui.click("Foto");
        ui.send(Message::Pane(crate::Pane::Scan));
        assert!(ui.shows(tick));
        assert!(ui.app.settings().scanner_colour, "on by default");
        ui.send(Message::Depth(crate::Depth(depth)));
        ui.send(Message::ScannerColour(on));
        assert_eq!(ui.app.prefs().photo.scanner_colour, Some(on));
        ui.click("Scansiona foto");
        assert_eq!(ui.app.pages.len(), 1, "{}", ui.app.note);
        spread(&ui.app.pages[0].raw.full.data)
    };
    for depth in [8, 16] {
        let (without, with) = (scan(false, depth), scan(true, depth));
        assert!(with > without * 1.1, "{depth} bits: {without} then {with}");
    }
    // A scanner declaring none: no tick, nothing changed.
    let mut ui = Ui::with_scanners(vec![FakeScanner::flatbed()]);
    ui.click("Foto");
    ui.send(Message::Pane(crate::Pane::Scan));
    ui.send(Message::ScannerColour(true));
    assert!(!ui.shows(tick));
    assert_eq!(ui.app.colour_correction(), None);
}

/// Where the adapter takes one film only, its film: for negatives the choice is only colour or
/// black and white.
#[test]
fn an_adapter_for_one_film_has_no_choice() {
    use scanner_api::{Scanner, Source};
    let (mut ui, _) = with_adapter();
    ui.send(Message::Pane(crate::Pane::Scan));
    ui.click("Pellicola");
    // The menu (a pick list: its text is not found by the simulator): black and white or
    // colour negatives, slides not offered.
    assert!(ui.shows("Nel supporto"));
    ui.send(Message::Holder(crate::Holder::BwNegatives));
    assert!(ui.app.bw);
    ui.send(Message::Holder(crate::Holder::Negatives));
    assert!(!ui.app.bw);
    // Slides chosen on another scanner: negatives here all the same.
    ui.app.slides = true;
    assert_eq!(ui.app.request().source, Source::Negative);
    assert!(ui.shows("Rullino"));
    // An adapter for slides only: slides, without asking.
    let mut caps = FakeScanner::with_slides().capabilities().clone();
    if let Some(a) = caps.transparency.as_mut() {
        a.sources = vec![Source::Slide];
    }
    let mut ui = Ui::with_scanners(vec![FakeScanner::with(caps)]);
    ui.send(Message::Pane(crate::Pane::Scan));
    ui.click("Pellicola");
    assert!(ui.app.filming());
    assert_eq!(ui.app.request().source, Source::Slide);
    assert!(!ui.shows("Nel supporto") && !ui.shows("Rullino"));
}

#[test]
fn the_scanners_stripes_go_from_films_too() {
    // A strip scanned with the stripes on the negative, the tick off then
    // on (each in its own run: a second scan would ask to replace).
    let scan = |destripe: bool| {
        let (mut ui, controls) = with_adapter();
        controls.stripes(30);
        ui.click("Pellicola");
        ui.send(Message::Destripe(destripe));
        ui.send(Message::Scan);
        ui.settle();
        assert!(!ui.app.pages.is_empty(), "{}", ui.app.note);
        assert_eq!(ui.app.prefs().film.destripe, Some(destripe));
        repeating_every(&ui, 5)
    };
    let with = scan(false);
    assert!(with > 0.005, "{with}");
    let without = scan(true);
    assert!(without < with / 3.0, "{with} then {without}");
}

#[test]
fn a_scanner_without_a_lamp_and_16_bits_shows_neither() {
    let mut ui = Ui::with_scanners(vec![FakeScanner::a3()]);
    assert_eq!(ui.app.caps.model, "Flatbed A3");
    // Ready, and nothing about a lamp.
    assert!(ui.shows("Pronto"));
    assert!(!ui.shows("Pronto · lampada accesa"));
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(!ui.shows("Profondità"));
    // 16 bits asked for (or kept from another scanner): 8 come.
    ui.send(Message::Depth(crate::Depth(16)));
    assert_eq!(ui.app.request().depth, 8);
    ui.send(Message::Pane(crate::Pane::Prefs));
    assert!(!ui.shows("Lampada"));
    assert!(!ui.shows("Spegnila"));
    // Its own two buttons, not the 3770's.
    assert!(ui.shows("Start") && ui.shows("Send"));
    assert!(!ui.shows("Instant Share"));
    // Closing does not wait for a lamp.
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert!(ui.closed);
    assert!(!ui.app.note.contains("lampada"), "{}", ui.app.note);
}

/// A made-up Epson Perfection 4490, with the capabilities its backend
/// declares (phase 12): a lamp the app switches off (the scanner does by
/// itself after 15 minutes), its four buttons, 16 bits, and films in its
/// lid at the resolutions captured.
#[test]
fn the_epson_4490_shows_what_its_backend_declares() {
    use scanner_api::Source;
    let caps = epson_gt_x750::capabilities();
    let mut ui = Ui::with_scanners(vec![FakeScanner::with(caps.clone())]);
    assert_eq!(ui.app.caps, caps);
    // The made-up scanner tells its lamp lit (the real 4490 does not say).
    assert!(ui.shows("Pronto · lampada accesa"));
    // Films: negatives and slides, at its film resolutions only.
    ui.click("Pellicola");
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(ui.shows("Nel supporto"));
    ui.send(Message::Dpi(200));
    assert_eq!(ui.app.request().source, Source::Negative);
    assert_eq!(ui.app.request().dpi, 600);
    ui.send(Message::Dpi(4800));
    assert_eq!(ui.app.request().dpi, 4800);
    ui.send(Message::Holder(crate::Holder::Slides));
    assert_eq!(ui.app.request().source, Source::Slide);
    ui.click("Foto");
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(ui.shows("Profondità"));
    assert!(ui.shows("Tasti dello scanner"));
    ui.send(Message::Pane(crate::Pane::Prefs));
    assert!(ui.shows("Lampada"));
    assert!(ui.app.lamp_note().contains("da solo dopo 15 minuti"));
    for label in ["Start", "Scan to PDF", "Scan to E-mail", "Copy"] {
        assert!(ui.shows(label), "{label}");
    }
    assert!(!ui.shows("Instant Share"));
}

#[test]
fn resolutions_and_glass_are_the_scanners() {
    let mut ui = Ui::with_scanners(vec![FakeScanner::a3()]);
    // 2400 dpi chosen (on the 3770, say): the nearest the A3 has.
    ui.send(Message::Dpi(2400));
    assert_eq!(ui.app.dpi(), 1200);
    assert_eq!(ui.app.request().dpi, 1200);
    ui.send(Message::Dpi(600));
    assert_eq!(ui.app.request().dpi, 600);
    // The preview covers the A3 glass, in pixels of the preview.
    ui.send(Message::Preview);
    ui.settle();
    let (raw, ..) = ui.app.preview.as_ref().expect("a preview");
    assert_eq!(raw.full.dpi, crate::PREVIEW_DPI);
    assert_eq!(
        (raw.full.width, raw.full.height),
        (2340, 3300),
        "11.7 × 16.5 inches at 200 dpi"
    );
    // A page scanned on it keeps its tone curve (at its lowest resolution:
    // a whole page at 600 dpi made up in a debug build took longer than a
    // slow CI machine waits).
    ui.send(Message::Kind(Kind::Document));
    ui.send(Message::Dpi(150));
    ui.scan(1);
    assert_eq!(ui.app.pages[0].raw.tone.white, [230.0; 3]);
}

#[test]
fn a_scanner_without_buttons_shows_no_button_settings() {
    let mut ui = Ui::with_scanners(vec![without_buttons()]);
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(!ui.shows("Tasti dello scanner"));
    ui.send(Message::Pane(crate::Pane::Prefs));
    assert!(!ui.shows("Tasti dello scanner in Documento"));
    // A press that cannot be is ignored.
    ui.send(Message::Worker(Event::Button(0)));
    assert!(ui.app.busy.is_none());
    assert!(ui.app.pages.is_empty());
}

#[test]
fn button_actions_are_kept_for_each_model() {
    use crate::ButtonAction;
    let mut ui = Ui::with_scanners(vec![FakeScanner::flatbed(), FakeScanner::a3()]);
    ui.send(Message::Pane(crate::Pane::Prefs));
    assert!(ui.shows("Usa"), "two scanners: a choice");
    // On the flatbed, Scan redoes the last page.
    ui.send(Message::ButtonAction(0, ButtonAction::RedoLast));
    // The A3: its Start does its default, adding a page.
    let a3 = ui.app.devices[1].clone();
    ui.send(Message::ChooseScanner(a3));
    while ui.app.caps.model != "Flatbed A3" {
        let event = ui
            .events
            .recv_timeout(Duration::from_secs(10))
            .expect("the A3 opened");
        ui.send(Message::Worker(event));
    }
    ui.settle();
    assert_eq!(
        ui.app.button_action(Kind::Document, 0),
        ButtonAction::AddPage
    );
    ui.send(Message::ButtonAction(0, ButtonAction::SaveDocument));
    // Back on the flatbed: its choice is still there.
    let flatbed = ui.app.devices[0].clone();
    ui.send(Message::ChooseScanner(flatbed));
    while ui.app.caps.model != "Flatbed A4" {
        let event = ui
            .events
            .recv_timeout(Duration::from_secs(10))
            .expect("the flatbed opened");
        ui.send(Message::Worker(event));
    }
    ui.settle();
    assert_eq!(
        ui.app.button_action(Kind::Document, 0),
        ButtonAction::RedoLast
    );
    // Both are saved, by model and button id.
    let stored = ui.app.prefs().buttons;
    assert_eq!(stored["Test Flatbed A4"].document["scan"], "rifai-ultima");
    assert_eq!(
        stored["Test Flatbed A3"].document["start"],
        "salva-documento"
    );
}

// ------------------------------------------------------------ languages

#[test]
fn the_app_speaks_english_when_asked_and_italian_again() {
    use crate::i18n::LangChoice;
    let mut ui = Ui::new();
    assert!(ui.shows("Anteprima"));
    ui.send(Message::Language(LangChoice::En));
    // The note of before was Italian: it gives way to one in English.
    assert_eq!(ui.app.note, "Language changed");
    assert!(ui.shows("Preview"));
    assert!(ui.shows("Scan page 1"));
    assert!(!ui.shows("Anteprima"));
    // A whole job in English: two pages, their notes, the panels.
    ui.scan(2);
    assert!(ui.shows("Scan page 3"));
    assert!(
        ui.app.note.starts_with("Page 2 acquired"),
        "{}",
        ui.app.note
    );
    ui.send(Message::Pane(crate::Pane::Adjust));
    assert!(ui.shows("This page only"));
    ui.send(Message::Pane(crate::Pane::Prefs));
    for label in [
        "Language",
        "Lamp",
        "Switch it off",
        "Scanner buttons in Document",
    ] {
        assert!(ui.shows(label), "{label}");
    }
    // Numbers the English way: in the editor, next to the sliders, in the
    // sizes of the PDF.
    assert_eq!(crate::editor::number(2.5, 1), "2.5");
    assert_eq!(crate::Field::Exposure.format(0.5), "0.50");
    assert_eq!(crate::human_size(4_928_307), "4.7 MB");
    // In the editor "Annulla" is Undo, in the questions Cancel.
    ui.send(Message::Edit(crate::EditMsg::Open(0)));
    assert!(ui.shows("Undo"));
    assert!(!ui.shows("Cancel"));
    ui.send(Message::Edit(crate::EditMsg::Discard));
    ui.send(Message::Pane(crate::Pane::Output));
    ui.send(Message::Save);
    assert_eq!(ui.saved().len(), 1, "{:?} {}", ui.saved(), ui.app.note);
    assert!(ui.app.note.starts_with("Saved: "), "{}", ui.app.note);
    // Remembered, and back to Italian.
    assert_eq!(ui.app.prefs().language, "english");
    ui.send(Message::Language(LangChoice::It));
    assert_eq!(ui.app.note, "Lingua cambiata");
    assert!(ui.shows("Anteprima"));
    assert_eq!(crate::editor::number(2.5, 1), "2,5");
    assert_eq!(crate::Field::Exposure.format(0.5), "0,50");
    assert_eq!(crate::human_size(4_928_307), "4,7 MB");
}

#[test]
fn a_question_and_its_answers_in_english() {
    use crate::i18n::LangChoice;
    let mut ui = document(2);
    ui.send(Message::Language(LangChoice::En));
    ui.send(Message::Kind(Kind::Photo));
    ui.scan(1);
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert_eq!(ui.question(), Some("Close without saving?"));
    assert!(ui.shows(
        "Still to save: 2 pages in Document, 1 photo in Photo. \
         Closing without saving, the work is lost."
    ));
    assert!(ui.shows("Close without saving"));
    ui.key(iced::keyboard::key::Named::Escape);
    assert!(ui.question().is_none());
    assert!(!ui.closed);
}

// ---------------------------------------------------------------- the tour

/// A tour of the app in Italian and in English, a screenshot of each place,
/// to look at by hand (texts cut off, mixed or untranslated):
/// `TOUR_DIR=tour cargo test -p vetroscan e2e::tour -- --ignored`.
#[test]
#[ignore = "screenshots to look at, not a check"]
fn tour() {
    use crate::i18n::LangChoice;
    use crate::EditMsg;
    let Some(dir) = std::env::var_os("TOUR_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    for (lang, choice) in [("it", LangChoice::It), ("en", LangChoice::En)] {
        let mut ui = Ui::new();
        ui.send(Message::Language(choice));
        let mut n = 0;
        let mut shot = |ui: &mut Ui, what: &str| {
            n += 1;
            let theme = ui.app.theme();
            let mut sim = iced_test::Simulator::with_size(
                iced::Settings::default(),
                Size::new(1360.0, 900.0),
                ui.app.view(),
            );
            let path = dir.join(format!("{lang}-{n:02}-{what}"));
            sim.snapshot(&theme).unwrap().matches_image(path).unwrap();
        };
        shot(&mut ui, "start");
        ui.send(Message::Preview);
        ui.settle();
        shot(&mut ui, "preview");
        ui.scan(2);
        shot(&mut ui, "pages");
        ui.send(Message::Pane(crate::Pane::Adjust));
        shot(&mut ui, "adjust");
        ui.send(Message::Pane(crate::Pane::Output));
        ui.send(Message::PdfSize(crate::PdfSize::A4));
        shot(&mut ui, "output-pdf");
        ui.send(Message::Pane(crate::Pane::Prefs));
        shot(&mut ui, "prefs");
        ui.send(Message::Edit(EditMsg::Open(0)));
        shot(&mut ui, "editor");
        ui.send(Message::Edit(EditMsg::Section(
            crate::editor::Section::Frame,
        )));
        shot(&mut ui, "editor-crop");
        ui.send(Message::Edit(EditMsg::Section(
            crate::editor::Section::Size,
        )));
        shot(&mut ui, "editor-size");
        ui.send(Message::Edit(EditMsg::Discard));
        ui.send(Message::CloseRequested(iced::window::Id::unique()));
        shot(&mut ui, "question");
        ui.send(Message::Confirm(false));
        // Photo: the originals found on the glass, its formats, its buttons.
        ui.send(Message::Kind(Kind::Photo));
        ui.send(Message::Pane(crate::Pane::Scan));
        ui.send(Message::Preview);
        ui.settle();
        shot(&mut ui, "photo-scan");
        ui.scan(1);
        ui.send(Message::Pane(crate::Pane::Output));
        shot(&mut ui, "photo-output");
        ui.send(Message::Pane(crate::Pane::Prefs));
        shot(&mut ui, "photo-prefs");
        // "Separa" on a photo scanned whole, an area chosen.
        ui.send(Message::AutoCrop(false));
        ui.scan(1);
        if ui.question().is_some() {
            ui.send(Message::Confirm(true));
            ui.settle();
        }
        ui.send(Message::PageSplit(0));
        ui.send(crate::separa::SplitMsg::Choose(0).into());
        shot(&mut ui, "photo-split");
        ui.send(crate::separa::SplitMsg::Cancel.into());
    }
}

// ---------------------------------------------------------- the firmware

/// A scanner that needs its firmware sent at every power-on, with no copy found: shown as
/// there, not "not found", with the file, where to put it and the button that opens that
/// folder; once a copy is in place it opens by itself, and the card goes.
#[test]
fn a_scanner_that_needs_its_firmware_is_shown_there_with_what_to_do() {
    use scanner_api::fake::FIRMWARE_FILE;
    let scanner = FakeScanner::flatbed().needing_firmware();
    let controls = scanner.controls();
    let name = scanner_api::Scanner::capabilities(&scanner).name();
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.settle();
    assert!(matches!(ui.app.status, Some(Err(_))));
    let (device, missing) = ui
        .app
        .firmware_missing
        .clone()
        .expect("the firmware's state");
    assert_eq!(device.name, name);
    assert_eq!(missing.file, FIRMWARE_FILE);
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(ui.shows("Firmware mancante"), "the state, not an error");
    assert!(!ui.shows("Scanner non trovato"));
    let folder = crate::prefs::config_dir().expect("the settings folder");
    let said = crate::i18n::trf(
        "{scanner} è collegato, ma a ogni accensione ha bisogno del suo firmware ({file}), e non \
         l'ho trovato.",
        &[("scanner", &name), ("file", &FIRMWARE_FILE)],
    );
    assert!(ui.shows(&said), "{said}");
    assert!(ui.shows(&format!(
        "Lo cerco anche in: {}",
        std::path::Path::new("/made-up/firmware").display()
    )));
    crate::testing::take_effects();
    ui.click("Apri la cartella delle impostazioni");
    assert!(folder.is_dir());
    assert_eq!(
        crate::testing::take_effects(),
        vec![format!("cartella aperta: {}", folder.display())]
    );
    // A copy put in place: the scanner opens by itself (the idle thread tries again).
    controls.firmware_missing(false);
    while !matches!(ui.app.status, Some(Ok(_))) {
        let event = ui
            .events
            .recv_timeout(Duration::from_secs(20))
            .expect("the scanner opened");
        ui.send(Message::Worker(event));
    }
    ui.settle();
    assert!(ui.app.firmware_missing.is_none());
    assert!(!ui.shows("Firmware mancante"));
    assert!(!ui.shows("Apri la cartella delle impostazioni"));
    ui.scan(1);
    assert_eq!(ui.app.pages.len(), 1, "{}", ui.app.note);
}

/// A scanner that needs no firmware, unplugged: "not found" as before, no firmware card.
#[test]
fn a_scanner_without_firmware_to_send_is_never_shown_missing_it() {
    let scanner = FakeScanner::flatbed();
    let controls = scanner.controls();
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(ui.app.firmware_missing.is_none());
    assert!(!ui.shows("Firmware mancante"));
    controls.unplug(true);
    ui.send(Message::RefreshStatus);
    while !matches!(ui.app.status, Some(Err(_))) {
        let event = ui
            .events
            .recv_timeout(Duration::from_secs(20))
            .expect("the scanner state");
        ui.send(Message::Worker(event));
    }
    ui.settle();
    assert!(ui.shows("Scanner non trovato"));
    assert!(!ui.shows("Firmware mancante"));
    assert!(!ui.shows("Apri la cartella delle impostazioni"));
}

// ---------------------------------------------------------- the folder

#[test]
fn the_folder_of_the_scans_opens_from_the_window_and_with_ctrl_o() {
    let mut ui = Ui::new();
    let folder = ui.app.folder();
    let opened = format!("cartella aperta: {}", folder.display());
    crate::testing::take_effects();
    // Next to "Save", even before the first scan: the folder is made.
    ui.click("Apri la cartella");
    assert!(folder.is_dir());
    assert_eq!(
        crate::testing::take_effects(),
        std::slice::from_ref(&opened)
    );
    // In the Preferences, next to the folder's path.
    ui.send(Message::Pane(crate::Pane::Prefs));
    ui.click("Apri");
    assert_eq!(
        crate::testing::take_effects(),
        std::slice::from_ref(&opened)
    );
    // Ctrl+O.
    let ctrl_o = iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Character("o".into()),
        modified_key: iced::keyboard::Key::Character("o".into()),
        physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyO),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::COMMAND,
        text: None,
        repeat: false,
    };
    ui.send(Message::Key(ctrl_o));
    assert_eq!(crate::testing::take_effects(), [opened]);
}

// ------------------------------------------------- moving from the keyboard

/// Presses `key` with `modifiers`, as the keyboard subscription would
/// deliver it (only what no text field took).
fn press(ui: &mut Ui, key: iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) {
    use iced::keyboard::key::{NativeCode, Physical};
    ui.send(Message::Key(iced::keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: Physical::Unidentified(NativeCode::Unidentified),
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    }));
}

fn arrow(ui: &mut Ui, named: iced::keyboard::key::Named, modifiers: iced::keyboard::Modifiers) {
    press(ui, iced::keyboard::Key::Named(named), modifiers);
}

/// Arrows pressed one right after the other, as a person does: what they
/// start (the page drawn again) runs after the last, so that a busy machine
/// does not space them out beyond the moment that makes them one step of
/// Ctrl+Z.
fn arrows(ui: &mut Ui, keys: &[(iced::keyboard::key::Named, iced::keyboard::Modifiers)]) {
    use iced::keyboard::key::{NativeCode, Physical};
    let tasks: Vec<_> = keys
        .iter()
        .map(|&(named, modifiers)| {
            let key = iced::keyboard::Key::Named(named);
            ui.app
                .update(Message::Key(iced::keyboard::Event::KeyPressed {
                    key: key.clone(),
                    modified_key: key,
                    physical_key: Physical::Unidentified(NativeCode::Unidentified),
                    location: iced::keyboard::Location::Standard,
                    modifiers,
                    text: None,
                    repeat: false,
                }))
        })
        .collect();
    for task in tasks {
        ui.run(task);
    }
}

const NONE: iced::keyboard::Modifiers = iced::keyboard::Modifiers::empty();
const SHIFT: iced::keyboard::Modifiers = iced::keyboard::Modifiers::SHIFT;
const ALT: iced::keyboard::Modifiers = iced::keyboard::Modifiers::ALT;
const CTRL: iced::keyboard::Modifiers = iced::keyboard::Modifiers::COMMAND;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn the_area_to_scan_moves_and_grows_from_the_keyboard() {
    use crate::viewer::Selection;
    use iced::keyboard::key::Named;
    let mut ui = Ui::new();
    ui.click("Anteprima");
    let start = Selection {
        x: 200.0,
        y: 200.0,
        width: 400.0,
        height: 400.0,
    };
    ui.send(Message::Select(Some(start)));
    // The preview is at 200 dpi: 1 mm is 7.87 pixels.
    let mm = 200.0 / 25.4;
    arrow(&mut ui, Named::ArrowRight, NONE);
    let s = ui.app.chosen_rect().unwrap();
    assert!(near(s.x, 200.0 + mm) && near(s.width, 400.0), "{s:?}");
    arrow(&mut ui, Named::ArrowDown, SHIFT);
    assert!(near(ui.app.chosen_rect().unwrap().y, 200.0 + 10.0 * mm));
    // Alt: 0.1 mm, but a pixel of the preview (0.127 mm) is more.
    arrow(&mut ui, Named::ArrowLeft, ALT);
    assert!(near(ui.app.chosen_rect().unwrap().x, 200.0 + mm - 1.0));
    // Ctrl: the size, from the right and bottom edges.
    arrow(&mut ui, Named::ArrowRight, CTRL);
    arrow(&mut ui, Named::ArrowUp, CTRL | SHIFT);
    let s = ui.app.chosen_rect().unwrap();
    assert!(near(s.width, 400.0 + mm) && near(s.height, 400.0 - 10.0 * mm));
    assert!(near(s.x, 200.0 + mm - 1.0), "the left edge stays: {s:?}");
    // At the edge of the glass it stops.
    for _ in 0..10 {
        arrow(&mut ui, Named::ArrowLeft, SHIFT);
    }
    assert_eq!(ui.app.chosen_rect().unwrap().x, 0.0);
    // Home: in the middle of the glass.
    arrow(&mut ui, Named::Home, NONE);
    let (s, glass) = (ui.app.chosen_rect().unwrap(), ui.app.glass());
    assert!(near(s.x * 2.0 + s.width, glass.width), "{s:?} {glass:?}");
    assert!(near(s.y * 2.0 + s.height, glass.height));
    // Never smaller than a few pixels.
    for _ in 0..80 {
        arrow(&mut ui, Named::ArrowLeft, CTRL | SHIFT);
    }
    assert!(ui.app.chosen_rect().unwrap().width >= 4.0);
}

#[test]
fn the_area_to_scan_is_typed_in_millimetres_and_a_field_keeps_its_arrows() {
    use crate::MmField;
    let mut ui = Ui::new();
    ui.click("Anteprima");
    ui.send(Message::Pane(crate::Pane::Scan));
    // No area chosen: the fields show all of A4.
    assert!(ui.shows("210") && ui.shows("297"));
    // In a field the arrows move the text cursor: the field takes them,
    // and the keyboard subscription never gets them.
    let status = {
        let mut sim = iced_test::Simulator::with_size(
            iced::Settings::default(),
            Size::new(1360.0, 900.0),
            ui.app.view(),
        );
        sim.click("210").expect("the width field");
        sim.tap_key(iced::keyboard::key::Named::ArrowLeft)
    };
    assert_eq!(status, iced::event::Status::Captured);
    // Typed, with a comma as Italians write.
    ui.send(Message::MmInput(MmField::Area(0), "12,5".into()));
    ui.send(Message::MmInput(MmField::Area(2), "100".into()));
    let (x, _, w, h) = ui.app.area_mm();
    assert!(
        (x - 12.5).abs() < 0.01 && (w - 100.0).abs() < 0.01,
        "{x} {w}"
    );
    assert!((h - 297.0).abs() < 0.2, "the rest as it was: {h}");
    // What is being typed stays as typed until Enter.
    ui.send(Message::MmInput(MmField::Area(3), "5".into()));
    assert!(ui.shows("5"));
    ui.send(Message::MmSubmit);
    assert!(ui.app.mm_draft.is_none());
    // Beyond the glass it stops at its edge.
    ui.send(Message::MmInput(MmField::Area(2), "900".into()));
    let (x, _, w, _) = ui.app.area_mm();
    let glass_mm = f64::from(ui.app.glass().width) * 25.4 / f64::from(crate::PREVIEW_DPI);
    assert!((x + w - glass_mm).abs() < 0.2, "{x} + {w} on {glass_mm}");
}

/// Where the page shown lies on its sheet: mm from the left and top edges.
fn page_offset(ui: &Ui) -> [f64; 2] {
    let page = &ui.app.pages[ui.app.current.unwrap()];
    let (w, h) = page.inches();
    ui.app
        .settings()
        .layout(page.layout, page.focus)
        .place(w, h)
        .offset_mm()
}

#[test]
fn the_page_moves_on_its_sheet_from_the_keyboard_and_the_pdf_has_it_there() {
    use crate::PdfFit;
    use iced::keyboard::key::Named;
    let mut ui = two_photos_on_a4();
    // At real size: room all round.
    ui.send(Message::PdfFit(PdfFit::Actual));
    assert!(ui.app.print_preview());
    let [l0, t0] = page_offset(&ui);
    arrow(&mut ui, Named::ArrowRight, NONE);
    arrow(&mut ui, Named::ArrowDown, SHIFT);
    let [l, t] = page_offset(&ui);
    assert!(
        (l - l0 - 1.0).abs() < 1e-3 && (t - t0 - 10.0).abs() < 1e-3,
        "{l} {t}"
    );
    // Alt: a pixel of the scan at 200 dpi (0.127 mm) is more than 0.1 mm.
    arrow(&mut ui, Named::ArrowLeft, ALT);
    assert!((page_offset(&ui)[0] - (l - 25.4 / 200.0)).abs() < 1e-3);
    // Ctrl: the page has no size of its own to change.
    let before = ui.app.pages[1].focus;
    arrow(&mut ui, Named::ArrowRight, CTRL);
    assert_eq!(ui.app.pages[1].focus, before);
    // At the edge of the sheet it stops.
    for _ in 0..40 {
        arrow(&mut ui, Named::ArrowLeft, SHIFT);
        arrow(&mut ui, Named::ArrowUp, SHIFT);
    }
    assert_eq!(ui.app.pages[1].focus, [0.0, 0.0]);
    assert!(page_offset(&ui).iter().all(|v| v.abs() < 1e-6));
    // The PDF has it in its top left corner.
    ui.send(Message::Save);
    let m = pdf_matrices(&ui)[1];
    assert!(m[2].abs() < 0.01, "left {m:?}");
    let sheet = pdf_boxes(&ui)[1];
    assert!(
        (m[3] + m[1] - sheet.0.max(sheet.1)).abs() < 0.01
            || (m[3] + m[1] - sheet.0.min(sheet.1)).abs() < 0.01,
        "top {m:?} on {sheet:?}"
    );
    // Home: centred again.
    arrow(&mut ui, Named::Home, NONE);
    assert_eq!(ui.app.pages[1].focus, [0.5, 0.5]);
    assert_eq!(ui.app.pages[0].focus, [0.5, 0.5], "the other page stays");
}

#[test]
fn where_the_page_goes_is_typed_centred_and_given_to_every_page() {
    use crate::{MmField, PdfFit};
    let mut ui = two_photos_on_a4();
    ui.send(Message::PdfFit(PdfFit::Actual));
    assert!(ui.shows("Posizione sul foglio"));
    ui.send(Message::MmInput(MmField::Offset(0), "15".into()));
    ui.send(Message::MmInput(MmField::Offset(1), "20,5".into()));
    let [l, t] = page_offset(&ui);
    assert!(
        (l - 15.0).abs() < 1e-3 && (t - 20.5).abs() < 1e-3,
        "{l} {t}"
    );
    ui.send(Message::MmSubmit);
    ui.click("Uguale per tutte le pagine");
    assert_eq!(ui.app.pages[0].focus, ui.app.pages[1].focus);
    ui.click("Centra");
    assert_eq!(ui.app.pages[1].focus, [0.5, 0.5]);
    // Fitted to the sheet, the page fills it one way inside the margin:
    // it moves only the other way.
    ui.send(Message::PdfFit(PdfFit::Page));
    let [rx, ry] = {
        let page = &ui.app.pages[1];
        let (w, h) = page.inches();
        ui.app
            .settings()
            .layout(page.layout, page.focus)
            .place(w, h)
            .room()
    };
    assert!(rx.abs() < 0.5 || ry.abs() < 0.5, "{rx} {ry}");
}

#[test]
fn the_crop_moves_from_the_keyboard_and_a_series_of_presses_is_one_ctrl_z() {
    use crate::editor::{Aspect, EditMsg, Section};
    use crate::viewer::Selection;
    use iced::keyboard::key::Named;
    let mut ui = document(1);
    ui.send(EditMsg::Open(0).into());
    ui.send(EditMsg::Section(Section::Frame).into());
    ui.send(
        EditMsg::Crop(Some(Selection {
            x: 100.0,
            y: 100.0,
            width: 300.0,
            height: 200.0,
        }))
        .into(),
    );
    ui.send(EditMsg::Submit.into());
    let steps = ui.app.editor.as_ref().unwrap().undo.len();
    let crop = |ui: &Ui| ui.app.crop_selection().unwrap();
    // 200 dpi: 1 mm is 7.87 pixels.
    let mm = 200.0 / 25.4;
    arrows(&mut ui, &[(Named::ArrowRight, NONE); 3]);
    let c = crop(&ui);
    assert!((c.x - (100.0 + 3.0 * mm)).abs() < 0.5, "{c:?}");
    assert!((c.width - 300.0).abs() < 0.5, "{c:?}");
    assert_eq!(
        ui.app.editor.as_ref().unwrap().undo.len(),
        steps + 1,
        "three presses, one step"
    );
    ui.send(Message::Undo);
    assert!((crop(&ui).x - 100.0).abs() < 0.5, "{:?}", crop(&ui));
    // With fixed proportions the size keeps them (the square chosen is as
    // large as the page: a small one first).
    ui.send(EditMsg::Aspect(Aspect::Square).into());
    ui.send(
        EditMsg::Crop(Some(Selection {
            x: 100.0,
            y: 100.0,
            width: 200.0,
            height: 200.0,
        }))
        .into(),
    );
    ui.send(EditMsg::Submit.into());
    let before = crop(&ui);
    arrow(&mut ui, Named::ArrowRight, CTRL | SHIFT);
    let c = crop(&ui);
    assert!(c.width > before.width + 10.0, "{before:?} {c:?}");
    assert!((c.width - c.height).abs() < 1.0, "square: {c:?}");
    // Straightening: [ and ] by 0.1°, with Shift by 1°.
    let angle = |ui: &Ui| ui.app.pages[0].transform.angle;
    press(&mut ui, iced::keyboard::Key::Character("]".into()), NONE);
    assert!((angle(&ui) - 0.1).abs() < 1e-4);
    press(&mut ui, iced::keyboard::Key::Character("}".into()), SHIFT);
    assert!((angle(&ui) - 1.1).abs() < 1e-4);
    // AltGr (Ctrl + Alt) on an Italian keyboard.
    press(
        &mut ui,
        iced::keyboard::Key::Character("[".into()),
        CTRL | ALT,
    );
    assert!((angle(&ui) - 1.0).abs() < 1e-4, "{}", angle(&ui));
}

#[test]
fn the_point_of_the_curve_clicked_last_moves_with_the_arrows() {
    use crate::Group;
    use iced::keyboard::key::Named;
    let mut ui = Ui::new();
    ui.click("Anteprima");
    ui.send(Message::Pane(crate::Pane::Adjust));
    if !ui.app.open.contains(&Group::Curve) {
        ui.send(Message::Toggle(Group::Curve));
    }
    let area = crate::viewer::Selection {
        x: 10.0,
        y: 10.0,
        width: 100.0,
        height: 100.0,
    };
    ui.send(Message::Select(Some(area)));
    ui.send(Message::CurveSet(
        vec![[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]],
        Some(1),
    ));
    ui.app.curve_drag = None;
    let steps = ui.app.undo.len();
    let points = |ui: &Ui| ui.app.settings().adjust.curve[0].points().to_vec();
    let level = 1.0 / 255.0;
    arrows(
        &mut ui,
        &[(Named::ArrowUp, NONE), (Named::ArrowRight, SHIFT)],
    );
    let p = points(&ui)[1];
    assert!(
        near(p[0], 0.5 + 10.0 * level) && near(p[1], 0.5 + level),
        "{p:?}"
    );
    assert_eq!(ui.app.undo.len(), steps + 1, "one step of Ctrl+Z");
    assert_eq!(ui.app.chosen_rect(), Some(area), "the area did not move");
    // Tab: the next point, Shift+Tab the one before.
    arrow(&mut ui, Named::Tab, NONE);
    assert_eq!(ui.app.curve_selected, Some(2));
    arrow(&mut ui, Named::Tab, SHIFT);
    assert_eq!(ui.app.curve_selected, Some(1));
    // Delete takes it away; the ends stay.
    arrow(&mut ui, Named::Delete, NONE);
    assert_eq!(points(&ui).len(), 2);
    assert_eq!(ui.app.curve_selected, None);
    ui.send(Message::CurvePoint(Some(0)));
    arrow(&mut ui, Named::Delete, NONE);
    assert_eq!(points(&ui).len(), 2, "not an end");
    // Esc lets the point go and keeps the area; then the arrows move it.
    arrow(&mut ui, Named::Escape, NONE);
    assert_eq!(ui.app.curve_selected, None);
    assert_eq!(ui.app.chosen_rect(), Some(area));
    arrow(&mut ui, Named::ArrowRight, NONE);
    assert!(ui.app.chosen_rect().unwrap().x > 10.0);
}

#[test]
fn page_up_and_down_go_through_the_pages_and_alt_moves_the_page() {
    use iced::keyboard::key::Named;
    let mut ui = document(3);
    let order = scans(&ui);
    ui.send(Message::PageShow(0));
    arrow(&mut ui, Named::PageDown, NONE);
    assert_eq!(ui.app.current, Some(1));
    arrow(&mut ui, Named::PageUp, NONE);
    arrow(&mut ui, Named::PageUp, NONE);
    assert_eq!(ui.app.current, Some(0), "it stops at the first");
    arrow(&mut ui, Named::PageDown, ALT);
    assert_eq!(scans(&ui), [order[1], order[0], order[2]]);
    assert_eq!(ui.app.current, Some(1), "the page shown is the one moved");
    arrow(&mut ui, Named::PageDown, ALT);
    arrow(&mut ui, Named::PageDown, ALT);
    assert_eq!(
        scans(&ui),
        [order[1], order[2], order[0]],
        "the last stays last"
    );
}

#[test]
fn the_zoomed_view_pans_with_the_arrows() {
    use iced::keyboard::key::Named;
    let mut ui = document(1);
    ui.send(Message::PageShow(0));
    // Fitted to the window: nothing to pan.
    arrow(&mut ui, Named::ArrowRight, NONE);
    assert_eq!(ui.app.result_view, crate::viewer::View::default());
    ui.send(Message::ActualSize);
    let size = ui.app.pages[0].picture.size;
    arrow(&mut ui, Named::ArrowRight, NONE);
    arrow(&mut ui, Named::ArrowDown, SHIFT);
    let c = ui.app.result_view.center.unwrap();
    assert!(near(c.x, size.width / 2.0 + 50.0), "{c:?}");
    assert!(near(c.y, size.height / 2.0 + 250.0), "{c:?}");
    arrow(&mut ui, Named::Home, NONE);
    assert_eq!(ui.app.result_view.center, None);
    assert_eq!(ui.app.result_view.scale, Some(1.0), "the zoom stays");
}

// ------------------------------------------------------------- colour profiles

/// The ICC profiles in a file the app saved, in order: colour space and
/// gamma. TIFF, JPEG and PDF keep them as they are, so they are found by
/// their signature.
fn profiles(name: &str, ui: &Ui) -> Vec<([u8; 4], f64)> {
    let bytes = std::fs::read(ui.app.folder().join(name)).unwrap();
    let mut found = Vec::new();
    let mut at = 36;
    while let Some(i) = bytes[at..].windows(4).position(|w| w == b"acsp") {
        let start = at + i - 36;
        let size = u32::from_be_bytes(bytes[start..start + 4].try_into().unwrap()) as usize;
        let profile = vetroscan_export::icc::read(&bytes[start..start + size])
            .unwrap_or_else(|| panic!("{name}: a profile not understood"));
        found.push((profile.space, profile.gamma));
        at = start + size + 36;
    }
    found
}

/// Saves what the app holds as `output`: the one file written.
fn save_as(ui: &mut Ui, output: crate::Output) -> String {
    let before = ui.saved();
    ui.send(Message::Output(output));
    ui.send(Message::Save);
    let new: Vec<String> = ui
        .saved()
        .into_iter()
        .filter(|n| !before.contains(n))
        .collect();
    assert_eq!(new.len(), 1, "{output:?}: {new:?} {}", ui.app.note);
    new.into_iter().next().unwrap()
}

/// 1.8 and 2.2 as an ICC curve holds them (n / 256).
const G18: f64 = 1.80078125;
const G22: f64 = 2.19921875;

#[test]
fn saved_files_say_the_gamma_their_scanner_draws_with() {
    use crate::{Colour, Output};
    use vetroscan_export::ImageFormat;
    // The flatbed at 2.2, the same at 1.8 (as the Epson 4490's glass).
    for (scanner, gamma) in [
        (FakeScanner::flatbed(), G22),
        (FakeScanner::flatbed().with_gamma(1.8), G18),
    ] {
        let mut ui = Ui::with_scanners(vec![scanner]);
        ui.send(Message::Kind(Kind::Document));
        ui.scan(1);
        for output in [
            Output::Image(ImageFormat::Tiff),
            Output::Image(ImageFormat::Jpeg { quality: 90 }),
            Output::Pdf,
        ] {
            let name = save_as(&mut ui, output);
            assert_eq!(profiles(&name, &ui), [(*b"RGB ", gamma)], "{output:?}");
        }
        // Grey says grey; black and white says nothing.
        ui.send(Message::PageShow(0));
        ui.send(Message::PageColour(Colour::Grey));
        let name = save_as(&mut ui, Output::Image(ImageFormat::Tiff));
        assert_eq!(profiles(&name, &ui), [(*b"GRAY", gamma)]);
        ui.send(Message::PageColour(Colour::Lineart));
        let name = save_as(&mut ui, Output::Image(ImageFormat::Tiff));
        assert_eq!(profiles(&name, &ui), []);
    }
}

#[test]
fn a_scanner_at_1_8_tags_its_glass_1_8_and_its_films_2_2() {
    use crate::{DeepFormat, Output};
    let mut ui = Ui::with_scanners(vec![FakeScanner::with_adapter().with_gamma(1.8)]);
    // A photo on the glass at 16 bits, saved at 16 bits: the scanner's curve.
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::Depth(crate::Depth(16)));
    ui.scan(1);
    let name = save_as(&mut ui, Output::Deep(DeepFormat::Tiff));
    assert_eq!(profiles(&name, &ui), [(*b"RGB ", G18)]);
    // Films: the positives' own curve, whatever the scanner's; each page
    // of a multi-page TIFF with its profile.
    ui.click("Pellicola");
    ui.send(Message::Dpi(200));
    ui.click("Scansiona foto");
    assert_eq!(ui.app.pages.len(), 2);
    let name = save_as(&mut ui, Output::TiffPages);
    assert_eq!(profiles(&name, &ui), [(*b"RGB ", G22), (*b"RGB ", G22)]);
}

// ------------------------------------------------------------- screen colours

/// The texture shown of page `i`, as RGB.
fn texture(ui: &Ui, i: usize) -> Vec<u8> {
    match &ui.app.pages[i].picture.handle {
        iced::widget::image::Handle::Rgba { pixels, .. } => {
            pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect()
        }
        other => panic!("not pixels: {other:?}"),
    }
}

#[test]
fn the_screen_shows_pages_through_the_monitors_profile_files_keep_their_numbers() {
    use crate::screen::{Monitor, Screen};
    use crate::MonitorPick;
    // A scanner drawing at 1.8, as the Epson 4490's glass.
    let mut ui = Ui::with_scanners(vec![FakeScanner::flatbed().with_gamma(1.8)]);
    ui.scan(1);
    let numbers = ui.app.pages[0].shown.to_rgb();
    let through = |screen: &Screen| {
        let mut p: Vec<u8> = numbers
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        screen.show(&mut p, 1.8);
        p.chunks(4)
            .flat_map(|c| [c[0], c[1], c[2]])
            .collect::<Vec<u8>>()
    };
    // No monitor known (yet, or on a system that does not say): sRGB.
    assert_eq!(ui.app.screen.label, "sRGB");
    assert_eq!(texture(&ui, 0), through(&Screen::srgb()));
    assert_ne!(texture(&ui, 0), numbers, "1.8 brought to sRGB");
    // The monitor under the window found, wide gamut: its profile.
    let wide = vetroscan_export::icc::matrix(
        "Adobe RGB-like",
        [
            [0.6097, 0.3111, 0.0195],
            [0.2053, 0.6257, 0.0609],
            [0.1492, 0.0632, 0.7446],
        ],
        2.2,
    );
    testing::set_monitor(Some(Monitor {
        name: "DISPLAY2".into(),
        profile_name: "wide.icc".into(),
        profile: wide.clone(),
    }));
    ui.send(Message::ScreenCheck);
    ui.settle();
    assert_eq!(ui.app.screen.label, "wide.icc (DISPLAY2)");
    assert_eq!(
        texture(&ui, 0),
        through(&Screen::profile(&wide, String::new()).unwrap())
    );
    ui.send(Message::Pane(crate::Pane::Prefs));
    assert!(ui.shows("In uso: wide.icc (DISPLAY2)"));
    // The page's numbers and its file are as they were.
    assert_eq!(ui.app.pages[0].shown.to_rgb(), numbers);
    // "Nessuna conversione": the numbers as they are, as before, remembered.
    ui.send(Message::Monitor(MonitorPick::Off));
    ui.settle();
    assert_eq!(texture(&ui, 0), numbers);
    assert_eq!(ui.app.prefs().monitor, "nessuno");
    assert!(ui.shows("In uso: i numeri come sono, senza conversione"));
    // Back to automatic: the monitor's again.
    ui.send(Message::Monitor(MonitorPick::Auto));
    ui.settle();
    assert_eq!(ui.app.screen.label, "wide.icc (DISPLAY2)");
    testing::set_monitor(None);
}

#[test]
fn a_pdf_of_photos_picked_lays_out_each_with_its_own_sheet() {
    use crate::{Output, PdfFit, Pick};
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.click("Anteprima");
    ui.click("Scansiona 3 foto");
    assert_eq!(ui.app.pages.len(), 3);
    // On A4. The third with its own sheet, filled (what overflows is
    // clipped); the others as the document's, which does not fill.
    ui.app.settings_mut().pdf_size = crate::PdfSize::A4;
    let mut own = ui.app.settings().page_layout();
    assert_ne!(own.fit, PdfFit::Fill);
    own.fit = PdfFit::Fill;
    ui.app.pages[2].layout = Some(own);
    // The first and the third picked, saved as a PDF.
    ui.send(Message::PageShow(0));
    ui.send(Message::PagePick(2, Pick::Toggle));
    let name = save_as(&mut ui, Output::Pdf);
    let pdf = std::fs::read(ui.app.folder().join(&name)).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert_eq!(text.matches("/Type /Page ").count(), 2);
    // The second page of the PDF is the third photo, with its sheet.
    assert_eq!(
        text.matches(" re W n ").count(),
        1,
        "the third photo filled"
    );
}

#[test]
fn senza_elaborazioni_and_raw_files_say_what_their_numbers_are() {
    use crate::{Output, RawFormat};
    use vetroscan_export::ImageFormat;
    // A scanner at 1.8 (as the Epson 4490's glass) with an adapter.
    let mut ui = Ui::with_scanners(vec![FakeScanner::with_adapter().with_gamma(1.8)]);
    ui.send(Message::Kind(Kind::Photo));
    ui.send(Message::AutoCrop(false));
    ui.send(Message::Untouched(true));
    ui.scan(1);
    // The glass without processing: the scanner's curve, as processed.
    let name = save_as(&mut ui, Output::Image(ImageFormat::Tiff));
    assert_eq!(profiles(&name, &ui), [(*b"RGB ", G18)]);
    // The RAW TIFF: linear (gamma 1), whatever the scanner's curve.
    let name = save_as(&mut ui, Output::Raw(RawFormat::Tiff16));
    assert_eq!(profiles(&name, &ui), [(*b"RGB ", 1.0)]);
    // Negatives left as they are, without processing: the films' curve.
    ui.click("Pellicola");
    ui.send(Message::Dpi(600));
    ui.send(Message::Untouched(true));
    ui.send(Message::KeepNegative(true));
    ui.click("Scansiona foto");
    let n = ui.app.pages.len();
    assert!(n > 0, "{}", ui.app.note);
    let name = save_as(&mut ui, Output::TiffPages);
    assert_eq!(profiles(&name, &ui), vec![(*b"RGB ", G22); n]);
}

// ------------------------------------------------------------------ infrared

/// Digital ICE, the scanner that reads an infrared channel from negatives (at 600 and 1200
/// dpi): "Leggi l'infrarosso" offered there only, off by default and remembered; asked for the
/// frames when ticked (never for the preview, nor "Senza elaborazioni"); kept with each frame,
/// one sample a pixel, dark on the strip's dust; saved beside each 16-bit file.
#[test]
fn the_infrared_is_offered_asked_kept_and_saved_where_the_scanner_reads_it() {
    use crate::{DeepFormat, Output};
    use scanner_api::fake::FILM_INFRARED;
    let tick = "Leggi l'infrarosso";
    let scanner = FakeScanner::with_infrared();
    let controls = scanner.controls();
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(!ui.shows(tick), "not on the glass");
    ui.click("Pellicola");
    ui.send(Message::Dpi(600));
    assert!(ui.shows(tick));
    // Said with the tick (general knowledge, no silver film measured): not for silver B&W.
    let silver = "Funziona su pellicole a colori e B/N cromogeniche (XP2, BW400CN); sulle B/N \
                  all'argento può scambiare parti della foto per polvere.";
    assert!(ui.shows(silver));
    assert!(!ui.app.settings().infrared, "off by default");
    assert!(!ui.app.request().infrared);
    ui.send(Message::Infrared(true));
    assert_eq!(ui.app.prefs().film.infrared, Some(true));
    assert!(ui.app.request().infrared);
    // "Senza elaborazioni": the scan as the scanner gives it, no second pass.
    ui.send(Message::Untouched(true));
    assert!(!ui.app.request().infrared);
    ui.send(Message::Untouched(false));
    ui.send(Message::Preview);
    ui.settle();
    ui.scan(1);
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    let asked = controls.requests();
    let (preview, frames) = asked.split_first().expect("the preview");
    assert!(!preview.infrared, "not for the preview");
    assert!(
        !frames.is_empty() && frames.iter().all(|r| r.infrared),
        "{frames:?}"
    );
    let clear = (FILM_INFRARED * 65535.0) as u16;
    let mut darkest = u16::MAX;
    for page in &ui.app.pages {
        let full = &page.raw.full;
        let ir = page.infrared.as_deref().expect("the infrared kept");
        assert_eq!(ir.len(), full.width * full.height);
        let mut sorted = ir.to_vec();
        sorted.sort_unstable();
        assert!(
            sorted[sorted.len() / 2].abs_diff(clear) < 300,
            "the film clear in the infrared: {}",
            sorted[sorted.len() / 2]
        );
        darkest = darkest.min(sorted[0]);
    }
    assert!(darkest < clear / 2, "the dust dark: {darkest}");
    // Saved beside each 16-bit file, with its name.
    let before = ui.saved();
    ui.send(Message::Output(Output::Deep(DeepFormat::Png)));
    ui.send(Message::Save);
    let new: Vec<String> = ui
        .saved()
        .into_iter()
        .filter(|n| !before.contains(n))
        .collect();
    assert_eq!(new.len(), 4, "{new:?} {}", ui.app.note);
    let infrared: Vec<&String> = new.iter().filter(|n| n.contains("-infrarosso")).collect();
    assert_eq!(infrared.len(), 2, "{new:?}");
    for name in infrared {
        let stem = name.trim_end_matches("-infrarosso.png");
        assert!(new.iter().any(|n| n == &format!("{stem}.png")), "{new:?}");
    }
    // Remembered: the app opened again keeps the tick.
    let mut reopened = Ui::with_scanners(vec![FakeScanner::with_infrared()]);
    reopened.click("Pellicola");
    reopened.send(Message::Dpi(600));
    assert!(reopened.app.request().infrared);
}

/// "Leggi l'infrarosso" takes the dust it shows out of the scan: the dusty strip's first frame
/// scanned with the tick comes out as a scanner without the dust gives it, where the infrared is
/// dark; without the tick the dust stays.
#[test]
fn the_dust_the_infrared_shows_is_taken_out_of_the_scan() {
    // The frames, each with its infrared when read, from a scanner after a preview.
    let frames = |scanner: FakeScanner, tick: bool| {
        let mut ui = Ui::with_scanners(vec![scanner]);
        ui.send(Message::Pane(crate::Pane::Scan));
        ui.click("Pellicola");
        ui.send(Message::Dpi(600));
        ui.send(Message::Infrared(tick));
        ui.send(Message::Preview);
        ui.settle();
        ui.scan(1);
        assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
        ui.app
            .pages
            .iter()
            .map(|p| ((*p.raw.full).clone(), p.infrared.as_deref().cloned()))
            .collect::<Vec<_>>()
    };
    let clean = frames(FakeScanner::with_adapter(), false);
    let cleaned = frames(FakeScanner::with_infrared(), true);
    let dusty = frames(FakeScanner::with_infrared(), false);
    let (first, ir) = (&cleaned[0].0, cleaned[0].1.as_ref().expect("the infrared"));
    assert_eq!(
        (first.width, first.height),
        (clean[0].0.width, clean[0].0.height)
    );
    assert!(dusty[0].1.is_none());
    // The dust: where the infrared is dark (the specks and the hair let through a quarter).
    let clear = (scanner_api::fake::FILM_INFRARED * 65535.0) as u16;
    let dust: Vec<usize> = (0..ir.len()).filter(|&i| ir[i] < clear / 2).collect();
    assert!(dust.len() > 30, "{}", dust.len());
    let off = |a: &scan_imaging::Image, b: &scan_imaging::Image| {
        dust.iter()
            .flat_map(|&i| (0..3).map(move |c| i * 3 + c))
            .map(|k| f64::from(a.data[k].abs_diff(b.data[k])))
            .sum::<f64>()
            / (dust.len() * 3) as f64
    };
    let (gone, left) = (off(first, &clean[0].0), off(&dusty[0].0, &clean[0].0));
    assert!(left > 20.0, "the dust in the scan without the tick: {left}");
    assert!(gone < 3.0, "the dust taken out with it: {gone}");
    // Away from the dust, the scan as it was.
    let near: Vec<bool> = {
        let w = first.width;
        let mut near = vec![false; ir.len()];
        for i in (0..ir.len()).filter(|&i| ir[i] < clear) {
            let (y, x) = (i / w, i % w);
            for yy in y.saturating_sub(4)..=(y + 4).min(first.height - 1) {
                for xx in x.saturating_sub(4)..=(x + 4).min(w - 1) {
                    near[yy * w + xx] = true;
                }
            }
        }
        near
    };
    // (the development's rounding moves a few samples by a level)
    let moved: Vec<u8> = (0..ir.len() * 3)
        .filter(|&k| !near[k / 3])
        .map(|k| first.data[k].abs_diff(dusty[0].0.data[k]))
        .filter(|&d| d > 0)
        .collect();
    assert!(
        moved.iter().all(|&d| d <= 1) && moved.len() * 10_000 < ir.len() * 3,
        "the scan changed away from the dust: {} samples, up to {:?}",
        moved.len(),
        moved.iter().max()
    );
    // The second frame has no dust: nothing found, nothing changed.
    assert_eq!(cleaned[1].0, dusty[1].0);
}

/// The film set to scan its frames with "Leggi l'infrarosso", after a preview.
fn dusty_film() -> (Ui, scanner_api::fake::Controls) {
    let scanner = FakeScanner::with_infrared();
    let controls = scanner.controls();
    let mut ui = Ui::with_scanners(vec![scanner]);
    ui.send(Message::Pane(crate::Pane::Scan));
    ui.click("Pellicola");
    ui.send(Message::Dpi(600));
    ui.send(Message::Infrared(true));
    ui.send(Message::Preview);
    ui.settle();
    (ui, controls)
}

/// A scan's cleaning (here the dust the infrared shows, seconds a frame at 2400 dpi) runs away
/// from the app's thread: the scanner's result handed to the app gives back at once, the scan
/// still busy and saying so, nothing added and no other scan taken meanwhile; each frame is added
/// when its cleaning comes back, then the next one scanned, in order, the same as cleaned at once.
#[test]
fn the_scan_is_cleaned_away_from_the_window_and_its_frames_kept_in_order() {
    let (mut at_once, _) = dusty_film();
    at_once.scan(1);
    assert_eq!(at_once.app.pages.len(), 2, "{}", at_once.app.note);
    let (mut ui, controls) = dusty_film();
    ui.send(Message::Scan);
    for frame in 0..2 {
        let done = ui.next_done();
        let task = ui.app.update(Message::Worker(done));
        assert!(ui.app.cleaning.is_some(), "frame {frame}: being cleaned");
        assert!(ui.app.busy.is_some());
        assert!(ui.shows("Pulizia della scansione"));
        assert_eq!(ui.app.pages.len(), frame, "added before it is cleaned");
        let asked = controls.requests().len();
        ui.send(Message::Scan);
        ui.send(Message::Preview);
        assert_eq!(controls.requests().len(), asked, "no scan while cleaning");
        ui.run(task);
        assert!(ui.app.cleaning.is_none());
        assert_eq!(ui.app.pages.len(), frame + 1, "{}", ui.app.note);
    }
    ui.settle();
    assert_eq!(ui.app.pages.len(), 2);
    for (a, b) in ui.app.pages.iter().zip(&at_once.app.pages) {
        assert!(a.raw.full == b.raw.full, "the frames in their order");
    }
}

/// "Annulla" while a scan is cleaned drops it (and the frames still to scan); what the cleaning
/// gives back afterwards goes nowhere, and a new scan works as ever. Closing the window while
/// one is cleaned asks as during a scan, closes, and what comes back is dropped too.
#[test]
fn a_cleaning_cancelled_or_closed_on_is_dropped() {
    let (mut ui, controls) = dusty_film();
    ui.send(Message::Scan);
    let done = ui.next_done();
    let task = ui.app.update(Message::Worker(done));
    assert!(ui.app.cleaning.is_some());
    ui.click("Annulla");
    assert_eq!(ui.app.note, "Scansione annullata");
    assert!(ui.app.cleaning.is_none() && ui.app.jobs.is_empty());
    let asked = controls.requests().len();
    ui.run(task);
    ui.settle();
    assert!(ui.app.pages.is_empty(), "the cancelled frame not added");
    assert_eq!(
        controls.requests().len(),
        asked,
        "the next frame not scanned"
    );
    ui.scan(1);
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);

    let (mut ui, controls) = dusty_film();
    ui.send(Message::Scan);
    let done = ui.next_done();
    let task = ui.app.update(Message::Worker(done));
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert_eq!(ui.question(), Some("Chiudere durante la scansione?"));
    for message in ui.click_messages("Chiudi") {
        ui.send(message);
    }
    assert!(ui.closed);
    let asked = controls.requests().len();
    ui.run(task);
    assert!(ui.app.pages.is_empty());
    assert_eq!(controls.requests().len(), asked);
}

/// The same with a scanner that reads no infrared: no tick, nothing asked, nothing kept, even
/// with the choice left on by another scanner.
#[test]
fn no_infrared_where_the_scanner_reads_none() {
    let (mut ui, controls) = with_adapter();
    ui.send(Message::Pane(crate::Pane::Scan));
    ui.click("Pellicola");
    ui.send(Message::Dpi(600));
    assert!(!ui.shows("Leggi l'infrarosso"));
    ui.send(Message::Infrared(true));
    assert!(!ui.app.request().infrared);
    ui.click("Scansiona foto");
    assert_eq!(ui.app.pages.len(), 2, "{}", ui.app.note);
    assert!(controls.requests().iter().all(|r| !r.infrared));
    assert!(ui.app.pages.iter().all(|p| p.infrared.is_none()));
}

/// "Negativi B/N": black and white negatives are read as negatives (the scanner asked nothing
/// new) and made grey; remembered; colour negatives again in colour. On an adapter for negatives
/// only too (the HP 3770's), and none on the glass.
#[test]
fn black_and_white_negatives_are_scanned_as_negatives_and_made_grey() {
    use scanner_api::Source;
    let (mut ui, _) = with_adapter();
    ui.send(Message::Pane(crate::Pane::Scan));
    assert!(!ui.shows("Negativi B/N"), "not on the glass");
    ui.click("Pellicola");
    ui.send(Message::Dpi(200));
    ui.send(Message::Holder(crate::Holder::BwNegatives));
    let request = ui.app.request();
    assert_eq!(request.source, Source::Negative);
    assert_eq!(ui.app.settings().colour, crate::Colour::Grey);
    assert!(ui.app.note.contains("bianco e nero"), "{}", ui.app.note);
    let prefs = ui.app.prefs();
    assert_eq!(prefs.film.bw_negatives, Some(true));
    assert_eq!(prefs.film.colour, "grigio");
    ui.click("Scansiona foto");
    assert!(!ui.app.pages.is_empty(), "{}", ui.app.note);
    let shared = *ui.app.settings();
    for page in &ui.app.pages {
        let file = page.finished(&shared);
        assert!(
            file.format == scan_imaging::Format::Grey
                || file
                    .data
                    .chunks_exact(3)
                    .all(|p| p[0] == p[1] && p[1] == p[2]),
            "a grey picture: {:?}",
            file.format
        );
    }
    // Remembered: the app opened again has black and white negatives, in grey.
    let (mut reopened, _) = with_adapter();
    assert!(reopened.app.bw);
    reopened.click("Pellicola");
    assert_eq!(reopened.app.settings().colour, crate::Colour::Grey);
    // Colour negatives again: in colour.
    ui.send(Message::Holder(crate::Holder::Negatives));
    assert_eq!(ui.app.settings().colour, crate::Colour::Rgb);
    assert_eq!(ui.app.prefs().film.bw_negatives, None);
    assert_eq!(ui.app.request().source, Source::Negative);
}
