// SPDX-License-Identifier: GPL-2.0-or-later
//! What the application needs from a scanner, and nothing about how a
//! scanner is reached.
//!
//! A backend (the HP 3770 driver over USB today; eSCL, SANE or WIA later)
//! implements [`Scanner`] for each device it can open and [`Backend`] to
//! find them; the [`Registry`] puts the backends together. The application
//! only sees these types: what the scanner can do ([`Capabilities`]), how
//! it is ([`DeviceStatus`]), a scan of an area ([`ScanRequest`] →
//! [`ScanResult`]) and its front-panel buttons.
//!
//! A scan always gives the scanner's **linear** RGB samples, without tone
//! curve or adjustments: the application draws them with
//! [`scan_imaging::adjust`] through the scanner's [`Tone`], so the preview
//! and every saved file come from the same samples.
//!
//! [`fake`] is a made-up scanner in memory, for tests and for trying the
//! application without a scanner.

pub mod fake;
mod registry;

pub use registry::{Backend, DeviceInfo, Registry};
pub use scan_imaging::adjust::Tone;

use scan_imaging::Image;
use std::fmt;
use std::time::Duration;

/// A scanner the application can use.
///
/// Every call blocks: the application calls it from a thread of its own.
pub trait Scanner: Send {
    /// What the scanner is and can do. It does not change while the
    /// scanner is open.
    fn capabilities(&self) -> &Capabilities;

    /// How the scanner is now.
    fn status(&mut self) -> Result<DeviceStatus>;

    /// Scans `request.area` at `request.dpi`. `progress` hears each stage;
    /// when it returns `false` the scan stops as soon as it safely can and
    /// ends with [`Error::Cancelled`].
    fn scan(
        &mut self,
        request: &ScanRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<ScanResult>;

    /// The front-panel buttons held now: bit `i` is
    /// `capabilities().buttons[i]`. A scanner without buttons answers 0.
    /// The application reads them often while idle and acts on presses
    /// (a bit that was clear the time before).
    fn poll_buttons(&mut self) -> Result<u32>;

    /// Switches the lamp off now. Only for scanners with a lamp
    /// ([`Capabilities::lamp`]); the others answer
    /// [`Error::Unsupported`].
    fn lamp_off(&mut self) -> Result<()> {
        Err(Error::Unsupported("lamp_off"))
    }
}

/// What a scanner is and can do.
#[derive(Debug, Clone, PartialEq)]
pub struct Capabilities {
    /// Who makes it, as shown to the user ("HP").
    pub manufacturer: String,
    /// The model, as shown to the user ("Scanjet 3770").
    pub model: String,
    /// The glass, width and height, inches.
    pub glass_inches: (f64, f64),
    /// The resolutions it scans at, dpi, smallest first.
    pub resolutions: Vec<u16>,
    /// The colour modes it scans in. A scan request is always for linear
    /// colour samples ([`ScanRequest`]); this says which modes the
    /// scanner offers natively (the application makes grey and black and
    /// white from colour in any case).
    pub modes: Vec<ColourMode>,
    /// The bits per sample it gives: 8, and 16 if it can.
    pub depths: Vec<u8>,
    /// Whether it has an adapter for slides and negatives.
    pub transparency: bool,
    /// The front-panel buttons, in the order of the bits of
    /// [`Scanner::poll_buttons`].
    pub buttons: Vec<ButtonInfo>,
    /// The lamp, if it has one the host switches on and off.
    pub lamp: Option<LampInfo>,
    /// The tone curve its linear samples need on the host; its white
    /// levels are also the paper white of the samples.
    pub tone: Tone,
}

impl Capabilities {
    /// What to assume when no backend knows any model: an A4 glass at 300
    /// dpi, no buttons, no lamp, a plain gamma 2.2 up to full white.
    pub fn placeholder() -> Self {
        Self {
            manufacturer: String::new(),
            model: "Nessuno scanner".into(),
            glass_inches: (8.27, 11.69),
            resolutions: vec![300],
            modes: vec![ColourMode::Colour],
            depths: vec![8],
            transparency: false,
            buttons: Vec::new(),
            lamp: None,
            tone: Tone {
                white: [255.0; 3],
                grey_white: 255.0,
                gamma: 2.2,
            },
        }
    }

    /// Manufacturer and model, as one name ("HP Scanjet 3770").
    pub fn name(&self) -> String {
        format!("{} {}", self.manufacturer, self.model)
            .trim()
            .to_string()
    }

    /// The highest resolution.
    pub fn max_dpi(&self) -> u16 {
        self.resolutions.iter().copied().max().unwrap_or(0)
    }

    /// The glass in pixels at `dpi`, width and height.
    pub fn glass_pixels(&self, dpi: u16) -> (u32, u32) {
        let (w, h) = self.glass_inches;
        (
            (w * f64::from(dpi)).round() as u32,
            (h * f64::from(dpi)).round() as u32,
        )
    }
}

/// A colour mode a scanner offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColourMode {
    Colour,
    Grey,
    /// Black and white.
    Lineart,
}

/// A front-panel button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonInfo {
    /// Stable identifier, for saving what the button does ("scan").
    pub id: String,
    /// What is printed on or next to it, as shown to the user ("Scan").
    pub label: String,
    /// What the manufacturer meant it for: the application's default.
    pub action: SuggestedAction,
}

/// What a button is meant for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SuggestedAction {
    /// Scan to the computer.
    Scan,
    /// Scan and print (photocopy).
    Copy,
    /// Scan and send or share.
    Share,
    /// Nothing in particular.
    None,
}

/// A lamp the host switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LampInfo {
    /// How long it takes to warm up from off.
    pub warm_up: Duration,
}

/// How a scanner is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStatus {
    /// Ready for a scan (not busy with something else).
    pub ready: bool,
    /// The lamp, if the scanner has one.
    pub lamp: Option<LampState>,
    /// Whether the transparency adapter is attached, if it can have one.
    pub adapter: Option<bool>,
    /// Technical details the application shows as they are (label,
    /// value): link speed, chip, registers.
    pub details: Vec<(String, String)>,
}

/// What the lamp is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LampState {
    Off,
    /// On, for the glass.
    On,
    /// On, the lamp of the transparency adapter.
    Transparency,
}

/// An area of the glass, in pixels at the resolution of the scan, from its
/// top left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A scan to make: always linear colour samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanRequest {
    /// Resolution, one of [`Capabilities::resolutions`].
    pub dpi: u16,
    /// The area, within the glass.
    pub area: Area,
    /// Bits per sample, one of [`Capabilities::depths`]: with 16 the
    /// samples also come in [`ScanResult::samples16`].
    pub depth: u8,
}

/// A finished scan.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanResult {
    /// The linear samples, 8-bit RGB ([`scan_imaging::Format::Rgb`]).
    pub image: Image,
    /// The same samples at 16 bits, when the request asked for them.
    pub samples16: Option<Vec<u16>>,
    /// What else the scanner says about the scan (label, value), for the
    /// curious: gains, warm-up, buffer.
    pub details: Vec<(String, String)>,
}

/// Where a scan is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Start-up and preparation.
    Starting,
    /// The lamp was off: waiting for it to warm up.
    WarmingUp,
    /// Calibration.
    Calibrating,
    /// Image data: bytes read so far, out of the total.
    Scanning { done: usize, total: usize },
    /// Stop and the last of the processing.
    Finishing,
}

/// Why a call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The user stopped the scan.
    Cancelled,
    /// The scanner is not there (unplugged, switched off, not found).
    Disconnected(String),
    /// Another program is using it, or it is busy.
    Busy(String),
    /// The scanner cannot do this (named).
    Unsupported(&'static str),
    /// The request is outside what the scanner accepts.
    InvalidRequest(String),
    /// Anything else, as the backend tells it.
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::Disconnected(why)
            | Self::Busy(why)
            | Self::InvalidRequest(why)
            | Self::Other(why) => write!(f, "{why}"),
            Self::Unsupported(what) => write!(f, "not supported by this scanner: {what}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Checks a request against what the scanner can do, as every backend
/// should before it moves anything.
pub fn check(caps: &Capabilities, request: &ScanRequest) -> Result<()> {
    if !caps.resolutions.contains(&request.dpi) {
        return Err(Error::InvalidRequest(format!(
            "{} dpi is not one of {:?}",
            request.dpi, caps.resolutions
        )));
    }
    if !caps.depths.contains(&request.depth) {
        return Err(Error::InvalidRequest(format!(
            "{} bits per sample is not one of {:?}",
            request.depth, caps.depths
        )));
    }
    let (w, h) = caps.glass_pixels(request.dpi);
    let a = request.area;
    if a.width == 0
        || a.height == 0
        || u64::from(a.x) + u64::from(a.width) > u64::from(w)
        || u64::from(a.y) + u64::from(a.height) > u64::from(h)
    {
        return Err(Error::InvalidRequest(format!(
            "area {a:?} is not within the glass ({w} x {h} at {} dpi)",
            request.dpi
        )));
    }
    Ok(())
}
