// SPDX-License-Identifier: GPL-2.0-or-later
//! The Epson Perfection 4490 Photo (GT-X750 in Japan, USB `04b8:0119`).
//!
//! **Milestone 1 of phase 12** (`docs/architecture-plan.md`), written
//! before the scanner arrived: it finds the scanner, sends its firmware
//! when it is not running, and reads its state. It does not scan
//! ([`Scanner::scan`] answers an error): nothing that moves the carriage or
//! switches a lamp is sent, because the native scan protocol is not
//! understood yet. Every byte here was read from Epson's Windows driver
//! and is still to be confirmed by a USB capture.
//!
//! The scanner does not speak ESC/I: it runs a firmware the host sends at
//! every power-on ([`firmware`]), then answers two-byte native commands
//! over two bulk endpoints ([`protocol`]). Two transports, as for the HP
//! 3770:
//!
//! * `usbscan.sys` (Windows only), the still-image driver Epson's package
//!   binds to the scanner: tried first there;
//! * `nusb`: Linux (usbfs, with a udev rule), macOS, and Windows when the
//!   scanner is bound to WinUSB.
//!
//! [`EpsonBackend`] offers the scanner to the application through the
//! [`scanner_api::Registry`]; without the scanner it lists nothing and
//! sends nothing.

pub mod firmware;
pub mod guard;
mod nusb_backend;
pub mod protocol;
#[cfg(windows)]
mod sti;
#[cfg(windows)]
mod usbscan;

pub use nusb_backend::NusbTransport;
#[cfg(windows)]
pub use usbscan::UsbscanTransport;

use protocol::FirmwareOutcome;
use scanner_api::{
    ButtonInfo, Capabilities, ColourMode, DeviceStatus, Error, Progress, Result, ScanRequest,
    ScanResult, SuggestedAction, Tone, UsbId,
};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// Epson's USB vendor id.
pub const VENDOR_ID: u16 = 0x04B8;
/// The Perfection 4490 Photo.
pub const PRODUCT_ID: u16 = 0x0119;
/// Name of this backend in the [`scanner_api::Registry`].
pub const BACKEND: &str = "epson-gt-x750";
/// Timeout of a bulk transfer.
pub const BULK_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether a capture of Epson's driver has confirmed the firmware upload's
/// bytes, so that opening a scanner whose firmware is not running sends it.
/// Confirmed on 11 November 2026: switched on, the scanner got from Epson's
/// Windows driver exactly what [`protocol::upload_firmware`] sends (the
/// file after its header, the checksum in place of its last byte), and
/// stayed on the bus. A scanner whose firmware already runs (sent by Epson's
/// driver) opens either way.
pub const UPLOAD_CONFIRMED: bool = true;

/// The two bulk endpoints of the scanner: all the backend needs (Epson's
/// driver sends no control transfers).
pub trait Transport: Send + fmt::Debug {
    /// Writes `data` to the bulk OUT endpoint, all of it.
    fn bulk_out(&mut self, data: &[u8]) -> Result<()>;
    /// Reads from the bulk IN endpoint into `buf`; the bytes read.
    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize>;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        (**self).bulk_out(data)
    }

    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).bulk_in(buf)
    }
}

/// How a scanner is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// Direct USB through `nusb`.
    Nusb,
    /// Windows `usbscan.sys` (Epson's driver installed).
    Usbscan,
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Nusb => "nusb",
            Self::Usbscan => "usbscan.sys",
        })
    }
}

/// A scanner found, before it is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// How it would be reached.
    pub link: Link,
    /// Where: `\\.\UsbscanN`, or bus and address.
    pub location: String,
    /// Why this link cannot open it (`nusb` on Windows without WinUSB).
    pub note: Option<String>,
}

/// The Epson 4490s every link can see: `usbscan.sys` first on Windows.
pub fn list() -> Vec<Found> {
    #[cfg(windows)]
    return usbscan::list()
        .into_iter()
        .chain(nusb_backend::list())
        .collect();
    #[cfg(not(windows))]
    nusb_backend::list()
}

/// Opens the scanner at `location` (from [`list`]).
pub fn open(location: &str) -> Result<Box<dyn Transport>> {
    #[cfg(windows)]
    if location.starts_with(r"\\.\") {
        return Ok(Box::new(UsbscanTransport::open(location)?));
    }
    Ok(Box::new(NusbTransport::open(location)?))
}

/// What the 4490 can do, **provisionally**: from Epson's user guide, none of
/// it tried. Only the glass, until films are driven; no lamp until it is
/// known whether the host switches it.
pub fn capabilities() -> Capabilities {
    let button = |id: &str, label: &str, action| ButtonInfo {
        id: id.into(),
        label: label.into(),
        action,
    };
    Capabilities {
        manufacturer: "Epson".into(),
        model: "Perfection 4490 Photo".into(),
        usb: Some(UsbId {
            vendor: VENDOR_ID,
            product: PRODUCT_ID,
        }),
        // 216 x 297 mm.
        glass_inches: (8.5, 11.7),
        // Provisional: optical 4800 dpi across; which ones the native
        // protocol takes is not known.
        resolutions: vec![150, 300, 600, 1200, 2400, 4800],
        modes: vec![ColourMode::Colour, ColourMode::Grey],
        depths: vec![8, 16],
        // It has an adapter in its lid (slides, negatives, 6 x 12 cm), not
        // driven yet: not declared, so the application offers no films.
        transparency: None,
        // Read with `1E 85` (`poll_buttons`), in this order: the scanner
        // answers 1-4 (captured on 11 November 2026).
        buttons: vec![
            button("start", "Start", SuggestedAction::Scan),
            button("pdf", "Scan to PDF", SuggestedAction::Scan),
            button("email", "Scan to E-mail", SuggestedAction::Share),
            button("copy", "Copy", SuggestedAction::Copy),
        ],
        lamp: None,
        // Provisional: a plain gamma 2.2 up to full white, until measured.
        tone: Tone {
            white: [255.0; 3],
            grey_white: 255.0,
            gamma: 2.2,
        },
        stripes: Vec::new(),
        colour: Vec::new(),
    }
}

/// The Epson 4490 on USB, as a backend of the [`scanner_api::Registry`].
#[derive(Debug, Clone)]
pub struct EpsonBackend {
    /// A copy of `esfw54.bin` the user chose, tried before the others.
    firmware_path: Option<PathBuf>,
    /// Whether opening a scanner may send it the firmware.
    upload: bool,
}

impl Default for EpsonBackend {
    fn default() -> Self {
        Self {
            firmware_path: None,
            upload: UPLOAD_CONFIRMED,
        }
    }
}

impl EpsonBackend {
    /// Finds the firmware as [`firmware::find`] does; sends it as
    /// [`UPLOAD_CONFIRMED`] says.
    pub fn new() -> Self {
        Self::default()
    }

    /// Tries this copy of `esfw54.bin` first (from the preferences).
    pub fn with_firmware_path(mut self, path: Option<PathBuf>) -> Self {
        self.firmware_path = path;
        self
    }

    /// Whether opening a scanner may send it the firmware.
    pub fn with_upload(mut self, upload: bool) -> Self {
        self.upload = upload;
        self
    }
}

/// What gives [`Gt4490::start`] the firmware: none when `upload` is not
/// allowed (an error saying so), else [`firmware::find`].
pub fn firmware_loader(
    upload: bool,
    path: Option<PathBuf>,
) -> impl FnOnce() -> Result<firmware::Firmware> {
    move || {
        if !upload {
            return Err(Error::Other(
                "the Epson 4490's firmware is not running, and sending it is turned off: switch \
                 the scanner on with Epson's driver, or run `epson4490 upload` to send it"
                    .into(),
            ));
        }
        firmware::find(path.as_deref()).map_err(Error::Other)
    }
}

impl scanner_api::Backend for EpsonBackend {
    fn name(&self) -> &'static str {
        BACKEND
    }

    /// One entry for each scanner a link can open. Nothing is sent to it.
    fn list(&self) -> Vec<scanner_api::DeviceInfo> {
        let name = capabilities().name();
        list()
            .into_iter()
            .filter(|f| f.note.is_none())
            .map(|f| scanner_api::DeviceInfo {
                backend: BACKEND,
                id: f.location,
                name: name.clone(),
            })
            .collect()
    }

    fn models(&self) -> Vec<Capabilities> {
        vec![capabilities()]
    }

    /// Opens the scanner and sends its firmware if it is not running (and
    /// that is allowed: [`UPLOAD_CONFIRMED`]).
    fn open(&self, device: &scanner_api::DeviceInfo) -> Result<Box<dyn scanner_api::Scanner>> {
        let transport = open(&device.id)?;
        let load = firmware_loader(self.upload, self.firmware_path.clone());
        Ok(Box::new(Gt4490::start(transport, load)?))
    }
}

/// An open Epson 4490 whose firmware runs. Everything it writes goes
/// through the [`guard`]: nothing that may move the carriage.
#[derive(Debug)]
pub struct Gt4490 {
    transport: guard::Guarded<Box<dyn Transport>>,
    caps: Capabilities,
    firmware: FirmwareOutcome,
}

impl Gt4490 {
    /// Takes the scanner on `transport`, sending it the firmware `load`
    /// gives if it is not running.
    pub fn start(
        transport: Box<dyn Transport>,
        load: impl FnOnce() -> Result<firmware::Firmware>,
    ) -> Result<Self> {
        let mut transport = guard::Guarded::new(transport);
        let firmware = protocol::ensure_firmware(&mut transport, load)?;
        Ok(Self {
            transport,
            caps: capabilities(),
            firmware,
        })
    }

    /// What was done about the firmware when the scanner was opened.
    pub fn firmware(&self) -> &FirmwareOutcome {
        &self.firmware
    }
}

impl scanner_api::Scanner for Gt4490 {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn status(&mut self) -> Result<DeviceStatus> {
        let state = protocol::state(&mut self.transport)?;
        Ok(DeviceStatus {
            ready: state.firmware_running() && !state.busy(),
            lamp: None,
            adapter: None,
            details: vec![
                (
                    "State (1B 03)".into(),
                    format!("{:02X} {:02X}", state.0[0], state.0[1]),
                ),
                (
                    "Firmware".into(),
                    match &self.firmware {
                        FirmwareOutcome::AlreadyRunning => "already running when opened".into(),
                        FirmwareOutcome::Uploaded { build, origin } => {
                            format!("sent when opened ({build}, {origin})")
                        }
                    },
                ),
                ("Backend".into(), "milestone 1: no scanning yet".into()),
            ],
        })
    }

    fn scan(
        &mut self,
        request: &ScanRequest,
        _progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<ScanResult> {
        scanner_api::check(&self.caps, request)?;
        Err(Error::Other(
            "scanning with the Epson Perfection 4490 is not implemented yet".into(),
        ))
    }

    /// The button pressed, as `1E 85` reads it: its number 1-4 is the
    /// place in [`capabilities`]' buttons plus one (Start, PDF, E-mail,
    /// Copy, as captured). Any other answer is none.
    fn poll_buttons(&mut self) -> Result<u32> {
        let n = protocol::button(&mut self.transport)?;
        Ok(match n {
            1..=4 => 1 << (n - 1),
            _ => 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::tests::Mock;
    use scanner_api::{Area, Backend, Scanner, Source};

    #[test]
    fn scanners_can_move_to_the_scanner_thread() {
        fn send<T: Send>() {}
        send::<Gt4490>();
        send::<EpsonBackend>();
        send::<NusbTransport>();
    }

    #[test]
    fn the_model_is_provisional_and_glass_only() {
        let models = EpsonBackend::new().models();
        assert_eq!(models.len(), 1);
        let caps = &models[0];
        assert_eq!(caps.name(), "Epson Perfection 4490 Photo");
        assert_eq!(caps.glass_inches, (8.5, 11.7));
        assert_eq!(caps.depths, vec![8, 16]);
        assert_eq!(caps.buttons.len(), 4);
        assert!(caps.lamp.is_none());
        assert!(!caps.offers(Source::Negative));
    }

    fn running() -> Gt4490 {
        let t = Mock::answering(&[&[0x10, 0x00]]);
        Gt4490::start(Box::new(t), || panic!("no firmware needed")).unwrap()
    }

    #[test]
    fn it_does_not_scan_yet_and_sends_nothing_for_it() {
        let mut s = running();
        let request = ScanRequest {
            dpi: 300,
            source: Source::Glass,
            area: Area {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            depth: 8,
        };
        let e = s.scan(&request, &mut |_| true).unwrap_err();
        assert!(e.to_string().contains("not implemented yet"), "{e}");
        assert_eq!(s.lamp_off(), Err(Error::Unsupported("lamp_off")));
        // Only the 1B 03 of the opening went out (the mock would have
        // failed any read after).
        let e = s.status().unwrap_err();
        assert!(e.to_string().contains("no answer left"), "{e}");
    }

    #[test]
    fn the_buttons_are_read_with_1e_85() {
        // Opening (1B 03), then none, Start, Copy, PDF, E-mail and an
        // unknown answer, as 1E 85 gives them.
        let t = Mock::answering(&[&[0x10, 0x00], &[0], &[1], &[4], &[2], &[3], &[9]]);
        let mut s = Gt4490::start(Box::new(t), || panic!("no firmware needed")).unwrap();
        let bits: Vec<u32> = (0..6).map(|_| s.poll_buttons().unwrap()).collect();
        assert_eq!(bits, [0, 0b0001, 0b1000, 0b0010, 0b0100, 0]);
        // In the order of the capabilities' buttons.
        let ids: Vec<&str> = s
            .capabilities()
            .buttons
            .iter()
            .map(|b| b.id.as_str())
            .collect();
        assert_eq!(ids, ["start", "pdf", "email", "copy"]);
    }

    #[test]
    fn a_button_poll_sends_1e_85_and_reads_one_byte() {
        let mut t = Mock::answering(&[&[2]]);
        assert_eq!(protocol::button(&mut t), Ok(2));
        assert_eq!(t.writes, vec![protocol::GET_BUTTONS.to_vec()]);
    }

    #[test]
    fn opening_sends_the_firmware_through_the_guard() {
        let t = Mock::answering(&[&[0x00, 0x00], &[6], &[6], &[6], &[6], &[0x10, 0x00]]);
        let image = protocol::tests::image();
        let mut s =
            Gt4490::start(Box::new(t), || Ok(firmware::Firmware::unchecked(image))).unwrap();
        assert!(matches!(s.firmware(), FirmwareOutcome::Uploaded { .. }));
        // The answers were read in order: state, four ACKs, then the state.
        assert!(s.status().unwrap().ready);
    }

    #[test]
    fn without_leave_the_firmware_is_not_sent() {
        assert!(EpsonBackend::new().upload);
        assert!(!EpsonBackend::new().with_upload(false).upload);
        // Firmware stopped, upload not allowed: only 1B 03 goes out (an
        // upload would have read an ACK the mock does not have).
        let t = Mock::answering(&[&[0x00, 0x00]]);
        let e = Gt4490::start(Box::new(t), firmware_loader(false, None)).unwrap_err();
        assert!(e.to_string().contains("sending it is turned off"), "{e}");
        // Running: it opens anyway.
        let t = Mock::answering(&[&[0x10, 0x00]]);
        assert!(Gt4490::start(Box::new(t), firmware_loader(false, None)).is_ok());
    }

    #[test]
    fn the_status_reads_the_state() {
        let t = Mock::answering(&[&[0x10, 0x00], &[0x12, 0x00], &[0x10, 0x00]]);
        let mut s = Gt4490::start(Box::new(t), || panic!("no firmware needed")).unwrap();
        let busy = s.status().unwrap();
        assert!(!busy.ready);
        assert_eq!(busy.details[0].1, "12 00");
        assert!(s.status().unwrap().ready);
        assert_eq!(s.firmware(), &FirmwareOutcome::AlreadyRunning);
    }
}
