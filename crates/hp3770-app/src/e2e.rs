// SPDX-License-Identifier: GPL-2.0-or-later
//! End-to-end tests of the application: the real `App`, its real view
//! clicked through `iced_test`'s simulator (no window), and the made-up
//! scanner of [`crate::worker::fake`] on its own thread. The tasks the app
//! returns are run, as the iced runtime would; what it does outside its
//! window (notifications, printing, the "Salva" dialog, the folder of the
//! scans) goes through [`crate::testing`].

use crate::testing;
use crate::worker::{fake, Event};
use crate::{App, Kind, Message};
use iced::futures::StreamExt;
use iced::{Size, Task};
use std::sync::mpsc;
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
        let (app, _) = App::new();
        let (handle, events) = fake::spawn(pace);
        let mut ui = Self {
            app,
            events,
            closed: false,
        };
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
            let wait = if self.app.busy.is_some() {
                Duration::from_secs(20)
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

// ------------------------------------- questions before losing scans

#[test]
fn photo_mode_asks_before_dropping_unsaved_pages_and_annulla_keeps_them() {
    let mut ui = document(3);
    ui.click("Foto");
    assert_eq!(ui.question(), Some("Passare a Foto?"));
    assert!(ui.shows(
        "In Foto resta solo l'ultima scansione. \
         Le altre 2 pagine del documento non sono salvate e andranno perse."
    ));
    assert_eq!(ui.app.kind, Kind::Document);
    ui.click("Annulla");
    assert_eq!(ui.question(), None);
    assert_eq!(ui.app.kind, Kind::Document);
    assert_eq!(ui.app.pages.len(), 3);
}

#[test]
fn escape_answers_no() {
    let mut ui = document(2);
    ui.click("Foto");
    assert!(ui.question().is_some());
    ui.key(iced::keyboard::key::Named::Enter);
    assert!(ui.question().is_some(), "Enter must not throw pages away");
    ui.key(iced::keyboard::key::Named::Escape);
    assert_eq!(ui.question(), None);
    assert_eq!(ui.app.pages.len(), 2);
}

#[test]
fn going_on_to_photo_keeps_the_last_page() {
    let mut ui = document(3);
    ui.click("Foto");
    ui.click("Passa a Foto");
    assert_eq!(ui.app.kind, Kind::Photo);
    assert_eq!(ui.app.pages.len(), 1);
}

#[test]
fn salva_in_the_question_saves_then_goes_on() {
    let mut ui = document(2);
    ui.click("Foto");
    ui.click("Salva…");
    assert_eq!(ui.saved().len(), 1, "{:?}", ui.saved());
    assert_eq!(ui.app.kind, Kind::Photo);
    assert!(!ui.app.unsaved);
}

#[test]
fn a_save_dialog_cancelled_changes_nothing() {
    let mut ui = document(2);
    testing::cancel_save_dialog(true);
    ui.click("Foto");
    ui.click("Salva…");
    assert!(ui.saved().is_empty());
    assert_eq!(ui.app.kind, Kind::Document);
    assert_eq!(ui.app.pages.len(), 2);
    assert!(ui.app.unsaved);
}

#[test]
fn saved_pages_are_not_asked_about() {
    let mut ui = document(2);
    ui.send(Message::Save);
    assert!(!ui.app.unsaved);
    ui.click("Foto");
    assert_eq!(ui.question(), None);
    assert_eq!(ui.app.kind, Kind::Photo);
}

#[test]
fn a_change_after_saving_is_asked_about_again() {
    let mut ui = document(2);
    ui.send(Message::Save);
    ui.send(Message::PageRotate(0, true));
    assert!(ui.app.unsaved);
    ui.click("Foto");
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
    ui.send(Message::Worker(Event::Button(
        hp3770_core::device::Button::Scan,
    )));
    assert_eq!(ui.question(), Some("Fare una nuova scansione?"));
    let effects = testing::take_effects();
    assert!(
        effects.contains(&"notifica: Fare una nuova scansione?".to_string()),
        "{effects:?}"
    );
    // A second press while the question is up does nothing more.
    ui.send(Message::Worker(Event::Button(
        hp3770_core::device::Button::Scan,
    )));
    assert!(ui.app.busy.is_none());
}

#[test]
fn photocopy_answered_no_does_not_print_the_next_photo() {
    let mut ui = Ui::new();
    ui.send(Message::Kind(Kind::Photo));
    ui.scan(1);
    ui.send(Message::Worker(Event::Button(
        hp3770_core::device::Button::Copy,
    )));
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
fn closing_with_nothing_to_lose_closes_at_once() {
    let mut ui = Ui::new();
    ui.send(Message::CloseRequested(iced::window::Id::unique()));
    assert_eq!(ui.question(), None);
    assert!(ui.closed);
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

#[test]
fn saving_twice_in_the_same_second_writes_over_nothing() {
    use hp3770_export::ImageFormat;
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
        ui.app.found.len(),
        fake::ORIGINALS.len(),
        "{:?}",
        ui.app.found
    );
    ui.click("Scansiona 3 foto");
    assert_eq!(ui.app.pages.len(), 3);
}
