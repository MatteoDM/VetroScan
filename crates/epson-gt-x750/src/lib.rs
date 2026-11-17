// SPDX-License-Identifier: GPL-2.0-or-later
//! The Epson Perfection 4490 Photo (GT-X750 in Japan, USB `04b8:0119`).
//!
//! **Phase 12** (`docs/architecture-plan.md`): it finds the scanner, sends
//! its firmware when it is not running, reads its state and its buttons
//! (all confirmed by USB captures of Epson's driver), and scans an area of
//! the glass in colour ([`scan`]), the whole sequence computed from the
//! request ([`setup`]) with the shading from its own calibration
//! ([`shading`]); the application scans with it ([`Gt4490`]), as does the
//! `epson4490` command. Every write goes through the [`guard`] (but the
//! command's `replay`), which lets out a scan only inside what the captures
//! cover.
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

pub mod film;
pub mod film_cal;
pub mod firmware;
pub mod guard;
pub mod mark;
mod nusb_backend;
pub mod protocol;
pub mod scan;
pub mod setup;
pub mod shading;
#[cfg(windows)]
mod sti;
pub mod table07;
pub mod tables;
#[cfg(windows)]
mod usbscan;

pub use nusb_backend::NusbTransport;
#[cfg(windows)]
pub use usbscan::UsbscanTransport;

use protocol::FirmwareOutcome;
use scanner_api::{
    Adapter, ButtonInfo, Capabilities, ColourCorrection, ColourMatrix, ColourMode, DeviceStatus,
    Error, LampInfo, Progress, Result, ScanMemory, ScanRequest, ScanResult, Source,
    SuggestedAction, Tone, UsbId,
};
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Epson's USB vendor id.
pub const VENDOR_ID: u16 = 0x04B8;
/// The Perfection 4490 Photo.
pub const PRODUCT_ID: u16 = 0x0119;
/// Name of this backend in the [`scanner_api::Registry`].
pub const BACKEND: &str = "epson-gt-x750";
/// The glass, width and height in inches: 8.5 in (216 mm, Epson's user guide) by 11.69 in
/// (296.9 mm), Epson Scan's own height (1,169 lines at 100 dpi, capture 38; a line or less more
/// at the other resolutions). The user guide's 11.7 in would let a scan of the whole glass end a
/// few motor steps further than any capture did (the guard's `MAX_READ_END`); 11.69 in keeps the
/// whole height inside them at every resolution (a test).
pub const GLASS_INCHES: (f64, f64) = (8.5, 11.69);
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
    /// Takes the scanner for this program until the transport is dropped: on Windows with
    /// Epson's driver its STI lock (`usbscan.rs`), nothing elsewhere.
    fn hold(&mut self) -> Result<()> {
        Ok(())
    }
    /// Whether [`Transport::hold`] takes a lock other programs take too (Windows' STI lock):
    /// opening then takes it before writing anything, as a program holding it may be scanning
    /// ([`Gt4490::start`]). Elsewhere a program scanning keeps the device to itself.
    fn locks(&self) -> bool {
        false
    }
    /// Lets the upload of this firmware `image` (the whole checked `esfw54.bin`) out: only the
    /// [`guard`] keeps it, to let through that payload and nothing else after `1B 06`.
    fn permit_upload(&mut self, _image: &[u8]) {}
    /// Writes `data` while reading the IN endpoint at the same time, each read handed to
    /// `on_read` until it answers `false`: a scanner left half-way through a scan takes a
    /// command only while what it sends is read (capture 52, 12 November 2026). The default,
    /// for a transport that cannot do both, writes and then reads.
    fn write_reading(
        &mut self,
        data: &[u8],
        on_read: &mut (dyn FnMut(&[u8]) -> bool + Send),
    ) -> Result<()> {
        self.bulk_out(data)?;
        let mut buf = vec![0u8; WRITE_READING_CHUNK];
        loop {
            let n = self.bulk_in(&mut buf)?;
            if !on_read(&buf[..n]) {
                return Ok(());
            }
        }
    }
}

/// The longest read of [`Transport::write_reading`].
pub const WRITE_READING_CHUNK: usize = 65_536;

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        (**self).bulk_out(data)
    }

    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).bulk_in(buf)
    }

    fn hold(&mut self) -> Result<()> {
        (**self).hold()
    }

    fn locks(&self) -> bool {
        (**self).locks()
    }

    fn permit_upload(&mut self, image: &[u8]) {
        (**self).permit_upload(image)
    }

    fn write_reading(
        &mut self,
        data: &[u8],
        on_read: &mut (dyn FnMut(&[u8]) -> bool + Send),
    ) -> Result<()> {
        (**self).write_reading(data, on_read)
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

/// Opens the scanner at `location` without Windows' STI lock (`usbscan.sys` only; elsewhere
/// as [`open`]), only to read what a scanner left half-way through a scan still sends
/// (`UsbscanTransport::open_unlocked`).
pub fn open_unlocked(location: &str) -> Result<Box<dyn Transport>> {
    #[cfg(windows)]
    if location.starts_with(r"\\.\") {
        return Ok(Box::new(UsbscanTransport::open_unlocked(location)?));
    }
    open(location)
}

/// What the 4490 can do: what the guard lets out, as captured and tried on the scanner
/// (the glass 11-14 November 2026, films 14-15 November).
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
        glass_inches: GLASS_INCHES,
        // The resolutions the guard lets out: every one captured on the glass, at 8 bits
        // (WIA, Epson Scan at 2400) and 16 (Epson Scan); 2400 read along at 3200 and reduced,
        // as Epson Scan does; 4800 with its own path. 800 and 900 dpi are not (guard.rs).
        resolutions: vec![100, 200, 300, 400, 600, 1200, 2400, 4800],
        // The scanner always sends colour; grey and black and white are made by the
        // application, as for the HP 3770.
        modes: vec![ColourMode::Colour, ColourMode::Grey, ColourMode::Lineart],
        depths: vec![8, 16],
        // The transparency unit in its lid: strips of negatives and slides, at the resolutions
        // Epson Scan was captured scanning both (captures 74-81, 98-101) and ours tried (87-97),
        // read at 16 bits; the window less its top 6 mm (`film::AREA_INCHES`). Its 6 x 12 cm
        // holder waits for a capture.
        transparency: Some(Adapter {
            area_inches: film::AREA_INCHES,
            sources: vec![Source::Slide, Source::Negative],
            resolutions: FILM_RESOLUTIONS.to_vec(),
        }),
        // Read with `1E 85` (`poll_buttons`), in this order: the scanner
        // answers 1-4 (captured on 11 November 2026).
        buttons: vec![
            button("start", "Start", SuggestedAction::Scan),
            button("pdf", "Scan to PDF", SuggestedAction::Scan),
            button("email", "Scan to E-mail", SuggestedAction::Share),
            button("copy", "Copy", SuggestedAction::Copy),
        ],
        // The host switches it off (`1E A2 00`) and the scanner does by itself after about
        // 15 minutes idle (capture 21); a cold one gets two and a half minutes (measured, `scan::WARM_UP`).
        lamp: Some(LampInfo {
            warm_up: scan::WARM_UP,
            switches_off_after: Some(Duration::from_secs(15 * 60)),
        }),
        // The scan gives linear samples (identity tables, `tables::Curves::Linear`); WIA's
        // image is them through a gamma of 1/1.8 up to full white (measured 12 November
        // 2026, 1.3 levels apart): the same here, until the tone is measured as the 3770's.
        tone: Tone {
            white: [255.0; 3],
            grey_white: 255.0,
            gamma: 1.8,
        },
        stripes: Vec::new(),
        colour: vec![ColourCorrection {
            source: Source::Glass,
            matrix: GLASS_COLOUR,
        }],
        scan_memory: scan_memory(),
        // Digital ICE (capture 18, `docs/architecture-plan.md` phase 12 step 17): declared once
        // the driver reads the infrared pass and lays it on the colours; not yet.
        infrared: Vec::new(),
    }
}

/// What a scan read along at another resolution ([`setup::READ_ALONG`], 2400 dpi at 3200)
/// holds while it is put together, against one read at its own: the data read and the image
/// realigned from it are `along / dpi` times the image each, and the image reduced from them is
/// held beside both (`scan::scan_glass`: `reduce_along(assemble(&raw))` while `raw` lives),
/// where a scan at its own resolution holds the data and one image. A film at 4800 dpi is read
/// along at 5400 the same way (`film::READ_ALONG`); one at 2400 is read twice as wide, the data
/// twice the image and the image once (1.5, under the glass's 2400). The larger of glass and
/// film at each resolution. Computed from the code, not measured.
fn scan_memory() -> Vec<ScanMemory> {
    let mut out: Vec<ScanMemory> = Vec::new();
    for &(dpi, along) in setup::READ_ALONG.iter().chain(film::READ_ALONG.iter()) {
        let r = f64::from(along) / f64::from(dpi);
        let times = (2.0 * r + 1.0) / 2.0;
        match out.iter_mut().find(|m| u32::from(m.dpi) == dpi) {
            Some(m) => m.times = m.times.max(times),
            None => out.push(ScanMemory {
                dpi: dpi as u16,
                times,
            }),
        }
    }
    out
}

/// The resolutions films are scanned at: those Epson Scan was captured scanning negatives and
/// slides at, 16 bits (`guard::FILM_COVERED`).
pub const FILM_RESOLUTIONS: [u16; 4] = [600, 1200, 2400, 4800];

/// The film request for an application's request from the adapter ([`film::for_area`]): a
/// negative or a slide, always read at 16 bits (the depth captured at every film resolution).
fn film_request(request: &ScanRequest) -> film::FilmRequest {
    let kind = if request.source == Source::Slide {
        film::Film::Positive
    } else {
        film::Film::Negative
    };
    let a = request.area;
    film::for_area(
        u32::from(request.dpi),
        kind,
        (a.x, a.y, a.width, a.height),
        16,
    )
}

/// A film's [`ScanResult`]: its image cut to `height` lines (read along at another resolution
/// it may have a line more, [`film::for_area`]), the 16-bit samples too when `deep`.
pub fn film_result_of(s: scan::FilmScanned, dpi: u16, height: u32, deep: bool) -> ScanResult {
    let im = s.image;
    let height = im.height.min(height);
    let wide: Vec<u16> = im.data[..height as usize * im.width as usize * 6]
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let c = &s.calibration;
    ScanResult {
        image: scan_imaging::Image {
            width: im.width as usize,
            height: height as usize,
            dpi,
            format: scan_imaging::Format::Rgb,
            data: wide.iter().map(|&v| (v >> 8) as u8).collect(),
        },
        samples16: deep.then_some(wide),
        infrared: None,
        details: vec![
            (
                "columns read".into(),
                format!(
                    "{}-{}",
                    c.plan.geometry.columns.0, c.plan.geometry.columns.1
                ),
            ),
            (
                "film exposures".into(),
                format!("{:?} µs, gains {:02x?}", c.exposures, c.image_gains),
            ),
            ("lid unit's move back".into(), format!("{} steps", c.back)),
        ],
    }
}

/// Epson Scan's correction of the glass's colours: it sends the scanner no colour correction
/// (`1E 57` byte 33 is 0 in every capture) and applies this matrix on the computer to what the
/// scanner sends, values of its tone tables (gamma about 1.8: 1.76-1.87 by channel, captures
/// 15 and 59). In thirty-seconds, rows adding up to 1; blue the most changed (48/32, less 14/32
/// of green). Measured on Epson Scan's own captures, the data sent against its TIFF
/// (`captures/epson4490/tools/colour_fit.py`, 14 November 2026): 15 (600 dpi) every pixel
/// exactly, 13 and 57 (2400 dpi, two power-ons) within 0.003. At 4800 dpi Epson Scan does more
/// of its own and the fit is looser (59, 14); the matrix is taken as the same.
pub const GLASS_COLOUR: ColourMatrix = ColourMatrix {
    rows: [
        35.0 / 32.0,
        -2.0 / 32.0,
        -1.0 / 32.0,
        -3.0 / 32.0,
        38.0 / 32.0,
        -3.0 / 32.0,
        -2.0 / 32.0,
        -14.0 / 32.0,
        48.0 / 32.0,
    ],
    gamma: 1.8,
};

/// A scan as the contract wants it: 8-bit linear RGB (the top byte of 16-bit samples), and
/// the 16-bit samples when the scan read them.
pub fn result_of(s: scan::Scanned, dpi: u16) -> ScanResult {
    let im = s.image;
    let (data, samples16) = if im.bits == 16 {
        let deep: Vec<u16> = im
            .data
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        (deep.iter().map(|&v| (v >> 8) as u8).collect(), Some(deep))
    } else {
        (im.data, None)
    };
    ScanResult {
        image: scan_imaging::Image {
            width: im.width as usize,
            height: im.height as usize,
            dpi,
            format: scan_imaging::Format::Rgb,
            data,
        },
        samples16,
        infrared: None,
        details: vec![
            (
                "x origin correction (1E 87)".into(),
                format!("{} / 4800 in", s.plan.x_correction.0),
            ),
            (
                "columns read".into(),
                format!("{}-{}", s.plan.columns.0, s.plan.columns.1),
            ),
            ("white calibration K".into(), format!("{:?}", s.shading.k)),
        ],
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
        // Unlocked: a scanner left half-way through a scan of ours is settled before Epson's
        // driver (given the lock) asks it anything; otherwise the lock is taken first
        // ([`Gt4490::start`]).
        let transport = open_unlocked(&device.id)?;
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
    /// When a scan of ours last ended (the lamp was lit then) and in which power-on
    /// ([`scan::lamp_is_cold`]).
    lamp_used: Option<scan::LampUse>,
    /// What opening found and did about a scan left half-way, if there was one.
    recovered: Vec<String>,
    /// Where a scan in progress is marked ([`mark`]).
    mark: PathBuf,
    /// A film scan of ours chose the lid's lamp, and no glass scan has switched back since:
    /// `1E A2 00` with it chosen was never captured, so [`scanner_api::Scanner::lamp_off`]
    /// leaves it to the scanner (it switches off by itself after about 15 minutes).
    lid_lamp: bool,
}

impl Gt4490 {
    /// Takes the scanner on `transport`, sending it the firmware `load`
    /// gives if it is not running.
    ///
    /// It asks the state with room for a packet of image: a scanner a program left half-way
    /// through a scan (closed or killed while reading) answers with its image, not a state, and
    /// is stopped with `1E 18` ([`scan::recover`]; captures 44-51, 12 November 2026).
    ///
    /// With a lock other programs take ([`Transport::locks`], Windows' STI lock) the lock comes
    /// first, before anything is written: a program holding it may be scanning, and a state
    /// asked then takes a packet of its image and stops its scan (capture 71, 14 November 2026:
    /// Epson Scan stuck, then its error; with the lock first nothing went out, capture 70). Only
    /// a scan of ours left by a process gone ([`mark`]) gets the state asked before the lock, as
    /// given the lock Epson's driver asks it itself, takes the image for "no firmware" and tries
    /// to send it (capture 44); a scan of another program left half-way then stays to Epson's
    /// driver, as with Epson Scan.
    pub fn start(
        transport: Box<dyn Transport>,
        load: impl FnOnce() -> Result<firmware::Firmware>,
    ) -> Result<Self> {
        Self::start_marked(transport, load, mark::default_path())
    }

    /// [`Gt4490::start`] with scans marked at `mark`.
    fn start_marked(
        transport: Box<dyn Transport>,
        load: impl FnOnce() -> Result<firmware::Firmware>,
        mark: PathBuf,
    ) -> Result<Self> {
        let mut transport = guard::Guarded::new(transport);
        let recovered = if transport.locks() && !mark::left_by_a_process_gone(&mark) {
            transport.hold()?;
            settle(&mut transport)?
        } else {
            let recovered = settle(&mut transport)?;
            hold(&mut transport, !recovered.is_empty())?;
            recovered
        };
        // Settled: whatever the mark said is over.
        if transport.locks() {
            mark::clear(&mark);
        }
        let firmware = protocol::ensure_firmware(&mut transport, load)?;
        Ok(Self {
            transport,
            caps: capabilities(),
            firmware,
            lamp_used: None,
            recovered,
            mark,
            lid_lamp: false,
        })
    }

    /// What was done about the firmware when the scanner was opened.
    pub fn firmware(&self) -> &FirmwareOutcome {
        &self.firmware
    }
}

/// How long the lock of a program gone is waited for after its scan was stopped: Windows
/// let it go 27 s after the program was killed (capture 51, 12 November 2026).
const LOCK_AFTER_RECOVERY: Duration = Duration::from_secs(45);

/// [`Transport::hold`]; after a scan left half-way was stopped, the lock the program gone
/// still held ([`Error::Busy`]) is waited for.
fn hold(t: &mut guard::Guarded<Box<dyn Transport>>, recovered: bool) -> Result<()> {
    let until = std::time::Instant::now() + LOCK_AFTER_RECOVERY;
    loop {
        match t.hold() {
            Err(Error::Busy(_)) if recovered && std::time::Instant::now() < until => {
                std::thread::sleep(Duration::from_secs(1));
            }
            other => return other,
        }
    }
}

/// Asks the state with room for a packet of image: a state is the answer of a scanner at rest
/// ([`protocol::is_state`]); anything else is a scan left half-way, stopped
/// ([`scan::recover`]). What was done, empty when nothing.
fn settle(t: &mut guard::Guarded<Box<dyn Transport>>) -> Result<Vec<String>> {
    // Written alone: reading first could take the answer of the image service's button poll
    // (`1E 85` every second while the lock is free). A scanner left half-way took this write
    // every time (captures 44-52); if it does not, or nothing comes back, it is stopped all the
    // same.
    let asked = t.bulk_out(&protocol::GET_STATE).and_then(|()| {
        let mut answer = [0u8; 512];
        let n = t.bulk_in(&mut answer)?;
        Ok(answer[..n].to_vec())
    });
    let first = match asked {
        Ok(answer) if protocol::is_state(&answer) => return Ok(Vec::new()),
        Ok(answer) => format!(
            "1B 03 answered {} bytes, not a state: a scan left half-way",
            answer.len()
        ),
        Err(e) => format!("1B 03: {e}: a scan left half-way?"),
    };
    let mut log = vec![first];
    t.permit_recovery();
    let done = scan::recover(t, &mut |line| log.push(line));
    t.end_scan();
    done?;
    log.push("stopped with 1E 18".into());
    Ok(log)
}

impl Gt4490 {
    /// A film's scan for the application ([`scanner_api::Scanner::scan`]): Epson Scan's
    /// sequence ([`scan::scan_film`], a slide's image exposure as its white's).
    fn scan_film(
        &mut self,
        request: &ScanRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<ScanResult> {
        let r = film_request(request);
        // Marked until it ends, as a glass scan.
        let _mark = mark::ScanMark::begin(&self.mark);
        self.lid_lamp = true;
        let scanned =
            scan::scan_film(
                &mut self.transport,
                &r,
                10,
                &mut |_| {},
                &mut |event| match event {
                    scan::Event::WarmingUp { .. } => progress(Progress::WarmingUp),
                    scan::Event::Calibrating => progress(Progress::Calibrating),
                    scan::Event::Reading { done, total } => {
                        progress(Progress::Scanning { done, total })
                    }
                },
            )?;
        progress(Progress::Finishing);
        Ok(film_result_of(
            scanned,
            request.dpi,
            request.area.height,
            request.depth == 16,
        ))
    }
}

impl scanner_api::Scanner for Gt4490 {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn status(&mut self) -> Result<DeviceStatus> {
        let state = protocol::state(&mut self.transport)?;
        let mut details = vec![
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
        ];
        if state.error() {
            details.push(("Error".into(), state.error_text()));
        }
        if !self.recovered.is_empty() {
            details.push(("Scan left half-way".into(), self.recovered.join("; ")));
        }
        details.push((
            "Backend".into(),
            "glass and film scans computed from the request".into(),
        ));
        Ok(DeviceStatus {
            ready: state.firmware_running() && !state.busy() && !state.error(),
            lamp: None,
            // The transparency unit is part of the lid: always there (whether its cable is
            // plugged in is not asked: its lamp would not light).
            adapter: Some(true),
            details,
        })
    }

    /// Scans the glass ([`scan::scan_glass`]) or a film in the lid ([`scan::scan_film`]):
    /// linear samples, 8 bits, and the 16-bit ones too when asked. A cold lamp gets its warm-up
    /// first (`Progress::WarmingUp`); the lid's lamp is waited for as long as the scanner says.
    fn scan(
        &mut self,
        request: &ScanRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<ScanResult> {
        scanner_api::check(&self.caps, request)?;
        if !progress(Progress::Starting) {
            return Err(Error::Cancelled);
        }
        if request.source.is_film() {
            return self.scan_film(request, progress);
        }
        let a = request.area;
        let r = setup::Request::glass(
            u32::from(request.dpi),
            a.x,
            a.y,
            a.width,
            a.height,
            request.depth,
        );
        // Marked until it ends: a process gone half-way through it leaves the mark behind.
        let _mark = mark::ScanMark::begin(&self.mark);
        let options = scan::Options {
            last_used: self.lamp_used,
            curves: tables::Curves::Linear,
        };
        let scanned = scan::scan_glass(
            &mut self.transport,
            r,
            options,
            &mut |_| {},
            &mut |event| match event {
                // Every second of the warm-up, so that Cancel is heard during it.
                scan::Event::WarmingUp { .. } => progress(Progress::WarmingUp),
                scan::Event::Calibrating => progress(Progress::Calibrating),
                scan::Event::Reading { done, total } => {
                    progress(Progress::Scanning { done, total })
                }
            },
        );
        // The lamp is lit once a scan got as far as switching it on: taken so, cancelled or not.
        // A cancelled scan keeps the power-on known before it: if it was another, the next scan
        // warms the lamp up once more.
        let power_on = match &scanned {
            Ok(s) => Some(s.power_on),
            _ => self.lamp_used.and_then(|l| l.power_on),
        };
        if matches!(&scanned, Ok(_) | Err(Error::Cancelled)) {
            self.lamp_used = Some(scan::LampUse {
                ended: SystemTime::now(),
                power_on,
            });
        }
        let s = scanned?;
        // Back to the glass's lamp (`1E 77` at the opening when the lid's was chosen).
        self.lid_lamp = false;
        progress(Progress::Finishing);
        Ok(result_of(s, request.dpi))
    }

    /// `1E A2 00`; the next scan takes the lamp to be cold. After a film, with the lid's lamp
    /// chosen, nothing is sent (never captured): the scanner switches it off by itself.
    fn lamp_off(&mut self) -> Result<()> {
        if self.lid_lamp {
            return Ok(());
        }
        protocol::lamp_off(&mut self.transport)?;
        self.lamp_used = None;
        Ok(())
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

    /// `1B 03` at rest, and before the firmware (captures).
    const IDLE: [u8; 2] = [0x18, 0x81];
    const NO_FW: [u8; 2] = protocol::State::NO_FIRMWARE;

    #[test]
    fn scanners_can_move_to_the_scanner_thread() {
        fn send<T: Send>() {}
        send::<Gt4490>();
        send::<EpsonBackend>();
        send::<NusbTransport>();
    }

    /// 2400 dpi is read along at 3200 and reduced: the data and the image realigned from it
    /// are 4/3 of the image each, beside the image reduced; a film at 4800 is read along at 5400
    /// the same way (9/8); every other resolution holds the data and one image, as the
    /// application's estimate counts.
    #[test]
    fn the_memory_declared_is_the_reduction_s() {
        let c = capabilities();
        assert_eq!(c.scan_memory.len(), 2);
        assert!((c.scan_memory_at(2400) - 11.0 / 6.0).abs() < 1e-12);
        assert!((c.scan_memory_at(4800) - 13.0 / 8.0).abs() < 1e-12);
        for dpi in [100, 200, 300, 400, 600, 1200] {
            assert_eq!(c.scan_memory_at(dpi), 1.0, "{dpi}");
        }
    }

    #[test]
    fn the_model_is_the_glass_as_captured() {
        let models = EpsonBackend::new().models();
        assert_eq!(models.len(), 1);
        let caps = &models[0];
        assert_eq!(caps.name(), "Epson Perfection 4490 Photo");
        assert_eq!(caps.glass_inches, GLASS_INCHES);
        // A line less than WIA's A4 at 300 dpi (capture 07).
        assert_eq!(caps.glass_pixels(300).1, 3507);
        assert_eq!(caps.depths, vec![8, 16]);
        assert_eq!(
            caps.resolutions,
            vec![100, 200, 300, 400, 600, 1200, 2400, 4800]
        );
        assert_eq!(caps.buttons.len(), 4);
        // Epson Scan's colours on the glass, grey left grey; none for films yet.
        let m = caps
            .colour_at(Source::Glass)
            .expect("the glass's correction");
        for row in m.rows.chunks(3) {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-6, "{row:?}");
        }
        assert_eq!(caps.colour_at(Source::Negative), None);
        let lamp = caps.lamp.expect("a lamp the host switches off");
        assert_eq!(lamp.warm_up, Duration::from_secs(150));
        assert_eq!(lamp.switches_off_after, Some(Duration::from_secs(900)));
        // Films in the lid: negatives and slides at the resolutions captured, the window less
        // its top 6 mm.
        for source in [Source::Negative, Source::Slide] {
            assert!(caps.offers(source));
            assert_eq!(caps.resolutions_for(source), [600, 1200, 2400, 4800]);
            assert_eq!(caps.bed_inches(source), Some(film::AREA_INCHES));
        }
        assert_eq!(caps.bed_pixels(Source::Negative, 600), Some((1620, 5454)));
    }

    /// A strip's frame scanned through the application's contract writes what Epson Scan wrote
    /// in capture 74, to its last line: capture 74's frame in the application's window (its
    /// top 6 mm left out), the 16-bit samples and the 8-bit ones.
    #[test]
    fn the_application_scans_a_film() {
        let text = format!(
            "P 1b03 1881
P 1b03 1881
{}",
            include_str!("../testdata/74_film.txt")
        );
        let f =
            scan::tests::Follow::film("74", &text, include_str!("../testdata/74_film_reads.txt"));
        let mut s = Gt4490::start(Box::new(f), || panic!("no firmware needed")).unwrap();
        let request = ScanRequest {
            dpi: 600,
            source: Source::Negative,
            area: Area {
                x: 93,
                y: 65,
                width: 576,
                height: 874,
            },
            depth: 16,
            infrared: false,
        };
        let mut seen = Vec::new();
        let result = s
            .scan(&request, &mut |p| {
                if !matches!(seen.last(), Some(Progress::Scanning { .. }))
                    || !matches!(p, Progress::Scanning { .. })
                {
                    seen.push(p);
                }
                true
            })
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((result.image.width, result.image.height), (576, 874));
        assert_eq!(result.image.data.len(), 576 * 874 * 3);
        assert_eq!(result.samples16.as_ref().map(Vec::len), Some(576 * 874 * 3));
        assert_eq!(seen[..2], [Progress::Starting, Progress::Calibrating]);
        assert!(matches!(seen[2], Progress::Scanning { .. }), "{seen:?}");
        assert_eq!(seen.last(), Some(&Progress::Finishing));
        // The lid's lamp is left to switch off by itself: nothing is sent.
        s.lamp_off().unwrap();
        // The whole capture was written: nothing is left to answer the next command.
        let e = s.status().unwrap_err().to_string();
        assert!(e.contains("the capture None"), "{e}");
    }

    #[test]
    fn the_lamp_is_switched_off_with_1e_a2_00() {
        // The opening's 1B 03, then 06 twice.
        let t = Mock::answering(&[&IDLE, &IDLE, &[6], &[6]]);
        let mut s = Gt4490::start(Box::new(t), || panic!("no firmware needed")).unwrap();
        s.lamp_used = scan::tests::just_used();
        s.lamp_off().unwrap();
        assert_eq!(s.lamp_used, None);
        // A NAK is an error.
        let t = Mock::answering(&[&IDLE, &IDLE, &[0x15]]);
        let mut s = Gt4490::start(Box::new(t), || panic!("no firmware needed")).unwrap();
        assert!(s.lamp_off().is_err());
    }

    fn running() -> Gt4490 {
        let t = Mock::answering(&[&IDLE, &IDLE]);
        Gt4490::start(Box::new(t), || panic!("no firmware needed")).unwrap()
    }

    /// A scanner answering as WIA's capture 29 (300 dpi, 5 x 5 cm), after the two `1B 03` of
    /// the opening (a scan left half-way?, the firmware), its lamp just used.
    fn follow_29() -> Gt4490 {
        let (_, text) = setup::tests::CAPTURES
            .iter()
            .find(|(n, _)| *n == "29")
            .unwrap();
        let mut f = scan::tests::Follow::new("29", &format!("P 1b03 1881\nP 1b03 1881\n{text}"));
        f.curves = tables::Curves::Linear;
        let mut s = Gt4490::start(Box::new(f), || panic!("no firmware needed")).unwrap();
        s.lamp_used = scan::tests::just_used();
        s
    }

    fn request_29() -> ScanRequest {
        let (_, text) = setup::tests::CAPTURES
            .iter()
            .find(|(n, _)| *n == "29")
            .unwrap();
        let (r, _, _) = setup::tests::request_of(&setup::tests::parse(text));
        ScanRequest {
            dpi: r.dpi as u16,
            source: Source::Glass,
            area: Area {
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
            },
            depth: 8,
            infrared: false,
        }
    }

    #[test]
    fn the_application_scans_the_glass() {
        let mut s = follow_29();
        let mut seen = Vec::new();
        let result = s
            .scan(&request_29(), &mut |p| {
                if !matches!(seen.last(), Some(Progress::Scanning { .. }))
                    || !matches!(p, Progress::Scanning { .. })
                {
                    seen.push(p);
                }
                true
            })
            .unwrap();
        assert_eq!((result.image.width, result.image.height), (591, 591));
        assert_eq!(result.image.dpi, 300);
        assert_eq!(result.image.data.len(), 591 * 591 * 3);
        assert!(result.samples16.is_none());
        assert!(result.details[0].1.contains("-93"), "{:?}", result.details);
        // The lamp was used a moment ago: no warming up.
        assert_eq!(seen[..2], [Progress::Starting, Progress::Calibrating]);
        assert!(matches!(seen[2], Progress::Scanning { .. }));
        assert_eq!(seen.last(), Some(&Progress::Finishing));
    }

    #[test]
    fn what_the_guard_refuses_never_reaches_the_scanner() {
        let mut s = running();
        let mut request = ScanRequest {
            dpi: 800,
            source: Source::Glass,
            area: Area {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            depth: 8,
            infrared: false,
        };
        // Not a resolution of the capabilities.
        assert!(s.scan(&request, &mut |_| true).is_err());
        // Films: not at a resolution of the adapter's.
        request.dpi = 300;
        request.source = Source::Negative;
        assert!(s.scan(&request, &mut |_| true).is_err());
        // Stopped before anything is sent.
        request.source = Source::Glass;
        assert_eq!(s.scan(&request, &mut |_| false), Err(Error::Cancelled));
        // Only the 1B 03 of the opening went out (the mock would have
        // failed any read after).
        let e = s.status().unwrap_err();
        assert!(e.to_string().contains("no answer left"), "{e}");
    }

    #[test]
    fn the_buttons_are_read_with_1e_85() {
        // Opening (1B 03), then none, Start, Copy, PDF, E-mail and an
        // unknown answer, as 1E 85 gives them.
        let t = Mock::answering(&[&IDLE, &IDLE, &[0], &[1], &[4], &[2], &[3], &[9]]);
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
        let t = Mock::answering(&[&NO_FW, &NO_FW, &[6], &[6], &[6], &[6], &IDLE]);
        let image = protocol::tests::image();
        let mut s =
            Gt4490::start(Box::new(t), || Ok(firmware::Firmware::unchecked(image))).unwrap();
        assert!(matches!(s.firmware(), FirmwareOutcome::Uploaded { .. }));
        // The answers were read in order: state, four ACKs, then the state.
        assert!(s.status().unwrap().ready);
    }

    /// A scanner switched off and on with its carriage away (`08 00`, 13 November 2026): a
    /// state, not a scan left half-way (no `1E 18` to a scanner without its firmware); the
    /// firmware is sent, and brings the carriage home by itself.
    #[test]
    fn opening_with_the_carriage_away_sends_the_firmware() {
        const AWAY: [u8; 2] = protocol::State::NO_FIRMWARE_CARRIAGE_AWAY;
        let t = Mock::answering(&[&AWAY, &AWAY, &[6], &[6], &[6], &[6], &IDLE]);
        let image = protocol::tests::image();
        let mut s =
            Gt4490::start(Box::new(t), || Ok(firmware::Firmware::unchecked(image))).unwrap();
        assert!(matches!(s.firmware(), FirmwareOutcome::Uploaded { .. }));
        let status = s.status().unwrap();
        assert!(status.ready);
        // Nothing stopped (a stop would have read answers the mock does not have).
        assert!(!status
            .details
            .iter()
            .any(|(k, _)| k == "Scan left half-way"));
    }

    #[test]
    fn without_leave_the_firmware_is_not_sent() {
        assert!(EpsonBackend::new().upload);
        assert!(!EpsonBackend::new().with_upload(false).upload);
        // Firmware stopped, upload not allowed: only 1B 03 goes out (an
        // upload would have read an ACK the mock does not have).
        let t = Mock::answering(&[&NO_FW, &NO_FW]);
        let e = Gt4490::start(Box::new(t), firmware_loader(false, None)).unwrap_err();
        assert!(e.to_string().contains("sending it is turned off"), "{e}");
        // Running: it opens anyway.
        let t = Mock::answering(&[&IDLE, &IDLE]);
        assert!(Gt4490::start(Box::new(t), firmware_loader(false, None)).is_ok());
    }

    #[test]
    fn opening_stops_a_scan_left_half_way() {
        // As on the scanner (captures 46-51): 1B 03 brings a packet of image; 1E 18 an empty
        // packet, then 06; the carriage goes home; then the firmware's 1B 03 and the status.
        let packet = [0x0Cu8; 512];
        let t = Mock::answering(&[
            &packet,
            &[],
            &[protocol::ACK],
            &[0x58, 0x81],
            &IDLE,
            &IDLE,
            &IDLE,
        ]);
        let mut s = Gt4490::start(Box::new(t), || panic!("no firmware needed")).unwrap();
        assert_eq!(s.firmware(), &FirmwareOutcome::AlreadyRunning);
        let status = s.status().unwrap();
        assert!(status.ready);
        let left = status
            .details
            .iter()
            .find(|(k, _)| k == "Scan left half-way")
            .expect("told in the status");
        assert!(left.1.contains("512 bytes"), "{left:?}");
        // Every answer read, in order: nothing more was sent.
        let e = s.status().unwrap_err();
        assert!(e.to_string().contains("no answer left"), "{e}");
        // The guard is back to the commands that move nothing.
        assert!(s.transport.bulk_out(&scan::CANCEL).is_err());
    }

    /// What a [`Locking`] transport saw: the writes, and how many had gone out when the lock
    /// was taken.
    #[derive(Debug, Default)]
    struct Seen {
        writes: Vec<Vec<u8>>,
        held_after: Option<usize>,
    }

    /// A transport with a lock other programs take, as Windows' STI lock; `busy` when another
    /// program holds it.
    #[derive(Debug)]
    struct Locking {
        mock: Mock,
        busy: bool,
        seen: std::sync::Arc<std::sync::Mutex<Seen>>,
    }

    impl Locking {
        fn new(answers: &[&[u8]], busy: bool) -> (Self, std::sync::Arc<std::sync::Mutex<Seen>>) {
            let seen = std::sync::Arc::default();
            let t = Self {
                mock: Mock::answering(answers),
                busy,
                seen: std::sync::Arc::clone(&seen),
            };
            (t, seen)
        }
    }

    impl Transport for Locking {
        fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
            self.seen.lock().unwrap().writes.push(data.to_vec());
            self.mock.bulk_out(data)
        }

        fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
            self.mock.bulk_in(buf)
        }

        fn hold(&mut self) -> Result<()> {
            if self.busy {
                return Err(Error::Busy("another program scans".into()));
            }
            let mut seen = self.seen.lock().unwrap();
            let n = seen.writes.len();
            seen.held_after.get_or_insert(n);
            Ok(())
        }

        fn locks(&self) -> bool {
            true
        }
    }

    fn mark_path(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "epson4490-start-test-{name}-{}",
            std::process::id()
        ));
        mark::clear(&p);
        p
    }

    /// With a lock other programs take, the lock comes before anything is written (capture
    /// 70: Epson Scan scanning, nothing went out), and a lock held is an error with nothing
    /// written (capture 71: a state asked then stopped Epson Scan's scan).
    #[test]
    fn with_a_shared_lock_the_lock_comes_first() {
        let p = mark_path("first");
        let (t, seen) = Locking::new(&[&IDLE, &IDLE], false);
        let s =
            Gt4490::start_marked(Box::new(t), || panic!("no firmware needed"), p.clone()).unwrap();
        assert_eq!(seen.lock().unwrap().held_after, Some(0));
        assert!(s.recovered.is_empty());
        // Another program scanning: busy, nothing written.
        let (t, seen) = Locking::new(&[], true);
        let e = Gt4490::start_marked(Box::new(t), || panic!("no firmware needed"), p).unwrap_err();
        assert!(matches!(e, Error::Busy(_)), "{e}");
        assert!(seen.lock().unwrap().writes.is_empty());
    }

    /// A scan of ours left by a process gone (its mark left behind): the state is asked before
    /// the lock and the scan stopped, as on the scanner (captures 52-54); the mark then goes.
    #[test]
    fn a_scan_of_ours_left_half_way_is_stopped_before_the_lock() {
        let p = mark_path("left");
        std::fs::write(&p, b"").unwrap();
        let packet = [0x0Cu8; 512];
        let (t, seen) = Locking::new(
            &[&packet, &[], &[protocol::ACK], &[0x58, 0x81], &IDLE, &IDLE],
            false,
        );
        let s =
            Gt4490::start_marked(Box::new(t), || panic!("no firmware needed"), p.clone()).unwrap();
        let seen = seen.lock().unwrap();
        // 1B 03, 1E 18, 1B 03 until at rest, all before the lock.
        assert!(seen.held_after.unwrap() >= 3, "{:02x?}", seen.writes);
        assert_eq!(seen.writes[0], protocol::GET_STATE);
        assert_eq!(seen.writes[1], scan::CANCEL);
        assert!(!s.recovered.is_empty());
        assert!(!p.exists());
    }

    /// A scan marks itself while it lasts, and the mark goes when it ends.
    #[test]
    fn a_scan_is_marked_while_it_lasts() {
        let p = mark_path("scan");
        let mut s = follow_29();
        s.mark = p.clone();
        let mut seen = false;
        s.scan(&request_29(), &mut |_| {
            seen |= p.exists();
            true
        })
        .unwrap();
        assert!(seen);
        assert!(!p.exists());
    }

    #[test]
    fn a_scanner_at_rest_is_left_alone() {
        let t = Mock::answering(&[&IDLE, &IDLE, &IDLE]);
        let mut s = Gt4490::start(Box::new(t), || panic!("no firmware needed")).unwrap();
        assert!(s
            .status()
            .unwrap()
            .details
            .iter()
            .all(|(k, _)| k != "Scan left half-way"));
    }

    #[test]
    fn the_status_reads_the_state() {
        let t = Mock::answering(&[&IDLE, &IDLE, &[0x12, 0x00], &[0x10, 0x00]]);
        let mut s = Gt4490::start(Box::new(t), || panic!("no firmware needed")).unwrap();
        let busy = s.status().unwrap();
        assert!(!busy.ready);
        assert_eq!(busy.details[0].1, "12 00");
        assert!(s.status().unwrap().ready);
        assert_eq!(s.firmware(), &FirmwareOutcome::AlreadyRunning);
    }
}
