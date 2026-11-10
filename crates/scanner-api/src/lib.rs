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
pub use scan_imaging::colour::ColourMatrix;

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
    /// Its USB vendor and product id, for what the system keeps about it
    /// under that id (on Windows, the events of its maker's driver); `None`
    /// for a scanner that is not on USB.
    pub usb: Option<UsbId>,
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
    /// Its adapter for slides and negatives, if it has one.
    pub transparency: Option<Adapter>,
    /// The front-panel buttons, in the order of the bits of
    /// [`Scanner::poll_buttons`].
    pub buttons: Vec<ButtonInfo>,
    /// The lamp, if it has one the host switches on and off.
    pub lamp: Option<LampInfo>,
    /// The tone curve its linear samples need on the host; its white
    /// levels are also the paper white of the samples.
    pub tone: Tone,
    /// The stripes it leaves across its scans, for each source (the glass,
    /// a film in the adapter: a different lamp or sensor may leave others,
    /// or none) at the resolutions where they were measured; the host can
    /// take them away (`scan_imaging::stripes`).
    pub stripes: Vec<Stripes>,
    /// The correction of its colours, for each source that has one (from
    /// its maker's driver, or measured on a target): the host applies it
    /// to the picture, where the user leaves it on
    /// (`scan_imaging::colour`).
    pub colour: Vec<ColourCorrection>,
}

/// The correction of a scanner's colours from one source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourCorrection {
    /// Where the scan comes from.
    pub source: Source,
    /// The matrix, and the values it works on.
    pub matrix: ColourMatrix,
}

/// Stripes across the scans of a scanner from one source at one
/// resolution: a pattern of lighter and darker lines that repeats after
/// the same number of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stripes {
    /// Where the scan comes from.
    pub source: Source,
    /// The resolution, dpi.
    pub dpi: u16,
    /// Lines after which the pattern repeats.
    pub repeat: u16,
}

impl Capabilities {
    /// What to assume when no backend knows any model: an A4 glass at 300
    /// dpi, no buttons, no lamp, a plain gamma 2.2 up to full white.
    pub fn placeholder() -> Self {
        Self {
            manufacturer: String::new(),
            model: "Nessuno scanner".into(),
            usb: None,
            glass_inches: (8.27, 11.69),
            resolutions: vec![300],
            modes: vec![ColourMode::Colour],
            depths: vec![8],
            transparency: None,
            buttons: Vec::new(),
            lamp: None,
            tone: Tone {
                white: [255.0; 3],
                grey_white: 255.0,
                gamma: 2.2,
            },
            stripes: Vec::new(),
            colour: Vec::new(),
        }
    }

    /// Lines after which the stripes of a scan from `source` at `dpi`
    /// repeat, if it leaves some there.
    pub fn stripes_at(&self, source: Source, dpi: u16) -> Option<usize> {
        self.stripes
            .iter()
            .find(|s| s.source == source && s.dpi == dpi)
            .map(|s| usize::from(s.repeat))
    }

    /// The correction of the colours of a scan from `source`, if it has
    /// one.
    pub fn colour_at(&self, source: Source) -> Option<ColourMatrix> {
        self.colour
            .iter()
            .find(|c| c.source == source)
            .map(|c| c.matrix)
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
        pixels(self.glass_inches, dpi)
    }

    /// Whether it scans from `source`: the glass always, films with an
    /// adapter that takes them.
    pub fn offers(&self, source: Source) -> bool {
        source == Source::Glass
            || self
                .transparency
                .as_ref()
                .is_some_and(|a| a.sources.contains(&source))
    }

    /// What a scan from `source` covers, width and height, inches: the
    /// glass, or the adapter's area. `None` for a source it does not offer.
    pub fn bed_inches(&self, source: Source) -> Option<(f64, f64)> {
        if !self.offers(source) {
            return None;
        }
        match (source, &self.transparency) {
            (Source::Glass, _) => Some(self.glass_inches),
            (_, Some(a)) => Some(a.area_inches),
            (_, None) => None,
        }
    }

    /// The resolutions it scans from `source` at: films have their own
    /// ([`Adapter::resolutions`]).
    pub fn resolutions_for(&self, source: Source) -> &[u16] {
        match (&self.transparency, source.is_film()) {
            (Some(a), true) => &a.resolutions,
            _ => &self.resolutions,
        }
    }

    /// [`Self::bed_inches`] in pixels at `dpi`.
    pub fn bed_pixels(&self, source: Source, dpi: u16) -> Option<(u32, u32)> {
        self.bed_inches(source).map(|size| pixels(size, dpi))
    }
}

fn pixels((w, h): (f64, f64), dpi: u16) -> (u32, u32) {
    (
        (w * f64::from(dpi)).round() as u32,
        (h * f64::from(dpi)).round() as u32,
    )
}

/// An adapter for films: a lamp that shines through them, in a holder on
/// the glass (the HP 3770's is in its lid).
#[derive(Debug, Clone, PartialEq)]
pub struct Adapter {
    /// The area its light covers, width and height, inches: a scan with
    /// it is of an area within this one, from its top left corner.
    pub area_inches: (f64, f64),
    /// The films it takes ([`Source::Slide`], [`Source::Negative`]).
    pub sources: Vec<Source>,
    /// The resolutions it scans films at, dpi, smallest first: some of
    /// [`Capabilities::resolutions`] (the HP 3770's motor stalls with
    /// films at the lowest ones).
    pub resolutions: Vec<u16>,
}

/// Where a scan comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Source {
    /// Light reflected by what lies on the glass.
    #[default]
    Glass,
    /// Slides in the adapter: positives in transmitted light.
    Slide,
    /// Negatives in the adapter, colour or black and white.
    Negative,
}

impl Source {
    /// A film, through the adapter.
    pub fn is_film(self) -> bool {
        self != Self::Glass
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

/// A scanner's USB vendor and product id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UsbId {
    pub vendor: u16,
    pub product: u16,
}

/// A lamp the host switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LampInfo {
    /// How long it takes to warm up from off.
    pub warm_up: Duration,
    /// After how long without scans the scanner switches it off by itself,
    /// if it does: waiting longer on the host is pointless.
    pub switches_off_after: Option<Duration>,
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

/// An area of the glass (or of the adapter's area, for a film), in pixels
/// at the resolution of the scan, from its top left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A scan to make: always linear colour samples.
///
/// From a film they are its transmission (full scale: the adapter's lamp
/// through no film), as the scanner measures it: a negative stays a
/// negative, the application turns it into the picture
/// ([`scan_imaging::film`]), better with the 16-bit samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanRequest {
    /// Resolution, one of [`Capabilities::resolutions`].
    pub dpi: u16,
    /// Where from: the glass, or a film in the adapter.
    pub source: Source,
    /// The area, within the glass, or within the adapter's area for a film
    /// ([`Capabilities::bed_inches`]).
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
    let resolutions = caps.resolutions_for(request.source);
    if !resolutions.contains(&request.dpi) {
        return Err(Error::InvalidRequest(format!(
            "{} dpi is not one of {:?}",
            request.dpi, resolutions
        )));
    }
    if !caps.depths.contains(&request.depth) {
        return Err(Error::InvalidRequest(format!(
            "{} bits per sample is not one of {:?}",
            request.depth, caps.depths
        )));
    }
    let Some((w, h)) = caps.bed_pixels(request.source, request.dpi) else {
        return Err(Error::InvalidRequest(format!(
            "this scanner does not scan from {:?}",
            request.source
        )));
    };
    let a = request.area;
    if a.width == 0
        || a.height == 0
        || u64::from(a.x) + u64::from(a.width) > u64::from(w)
        || u64::from(a.y) + u64::from(a.height) > u64::from(h)
    {
        return Err(Error::InvalidRequest(format!(
            "area {a:?} is not within the {} ({w} x {h} at {} dpi)",
            if request.source.is_film() {
                "adapter's area"
            } else {
                "glass"
            },
            request.dpi
        )));
    }
    Ok(())
}
