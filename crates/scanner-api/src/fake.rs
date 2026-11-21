// SPDX-License-Identifier: GPL-2.0-or-later
//! A scanner made up in memory: a white lid with three originals on the
//! glass, scanned at any resolution it offers. For the tests of the
//! application and for trying it with no scanner.
//!
//! Two models: [`FakeScanner::flatbed`], shaped like the HP Scanjet 3770
//! (A4 glass, up to 2400 dpi, three buttons, a lamp), and
//! [`FakeScanner::a3`], a different one (A3 glass, up to 1200 dpi, two
//! buttons, no lamp), so tests can see the application adapt.
//! [`FakeScanner::with_adapter`] is the first with an adapter for negatives
//! too, a strip of two frames in its holder ([`film`]);
//! [`FakeScanner::with_slides`] one for slides and negatives, two slides in
//! its holder ([`slides`]) until a test puts the strip in
//! ([`Controls::load`]); [`FakeScanner::with_infrared`] one that reads an
//! infrared channel from negatives, with [`dust`] on its strip.

use crate::{
    check, Adapter, Backend, ButtonInfo, Capabilities, ColourCorrection, ColourMatrix, ColourMode,
    DeviceInfo, DeviceStatus, Error, Infrared, LampInfo, LampState, Progress, Result, ScanMemory,
    ScanRequest, ScanResult, Scanner, Source, Stripes, SuggestedAction, Tone,
};
use scan_imaging::{Format, Image};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The originals on the glass: left, top, width, height (inches) and the
/// colour they are printed in.
pub const ORIGINALS: [([f64; 4], [u8; 3]); 3] = [
    ([0.8, 0.8, 4.0, 2.8], [190, 90, 40]),
    ([1.0, 5.0, 3.0, 3.0], [40, 90, 170]),
    ([5.3, 4.5, 2.5, 4.0], [60, 140, 70]),
];

/// The lid, where nothing lies on the glass.
pub const LID: [u8; 3] = [236, 236, 232];

/// The colour of the glass at a point (inches from the top left).
pub fn glass(x: f64, y: f64) -> [u8; 3] {
    for ([left, top, width, height], colour) in ORIGINALS {
        let (u, v) = ((x - left) / width, (y - top) / height);
        if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
            // Shaded across, with stripes: something for the adjustments to
            // change and the crop to find.
            let stripe = if ((u * 12.0) as i32 + (v * 8.0) as i32) % 2 == 0 {
                1.0
            } else {
                0.8
            };
            let shade = (0.6 + 0.4 * u) * stripe;
            return colour.map(|c| (f64::from(c) * shade).round() as u8);
        }
    }
    LID
}

/// The adapter's area of [`FakeScanner::with_adapter`], inches: the HP
/// 3770's as first measured, with the negatives' holder (44.0 x 95.4 mm;
/// the real one now reaches 106.8 mm, for the slides' holder).
pub const ADAPTER_INCHES: (f64, f64) = (1.732, 3.756);

/// The frames of the strip in the adapter's holder: left, top, width,
/// height (inches from the adapter's top left corner), two 35 mm frames one
/// above the other (24 x 36 mm, 38 mm apart).
pub const FRAMES: [[f64; 4]; 2] = [[0.394, 0.55, 0.945, 1.417], [0.394, 2.046, 0.945, 1.417]];

/// The film base's transmission (orange, as colour negatives).
pub const BASE: [f64; 3] = [0.70, 0.42, 0.22];

/// The transmission of the adapter's area at a point (inches from its top
/// left corner), full scale 1: the lamp through no film in the slot at the
/// top, the strip of film (its base, and in the frames a scene denser
/// across), the dark holder everywhere else.
pub fn film(x: f64, y: f64) -> [f64; 3] {
    if y < 0.33 {
        return [1.0; 3];
    }
    strip(x, y, BASE)
}

/// The strip of [`film`] with its base `base`: the frames' scene, the bare
/// base between them, the dark holder around it.
fn strip(x: f64, y: f64, base: [f64; 3]) -> [f64; 3] {
    if !(0.2..1.53).contains(&x) || !(0.4..3.66).contains(&y) {
        return [0.002; 3];
    }
    for [left, top, width, height] in FRAMES {
        let (u, v) = ((x - left) / width, (y - top) / height);
        if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
            // Density above the base, as a scene's light leaves it.
            let stripe = if (v * 6.0) as i32 % 2 == 0 { 1.0 } else { 0.7 };
            let density = (0.15 + 1.0 * u) * stripe;
            let layer = [1.0, 0.9, 0.8];
            return [0, 1, 2].map(|c| base[c] * 10f64.powf(-density * layer[c]));
        }
    }
    base
}

/// Specks of dust on the first frame of [`film`]'s strip with [`FakeScanner::with_infrared`]:
/// centre x, y and radius, inches from the adapter's top left corner.
pub const SPECKS: [[f64; 3]; 2] = [[0.7, 0.9, 0.006], [1.1, 1.5, 0.004]];

/// A hair on the same frame: from x, y to x, y (inches), and its width.
pub const HAIR: ([f64; 4], f64) = ([0.5, 1.2, 0.9, 1.35], 0.0015);

/// The light the [`SPECKS`] and the [`HAIR`] let through, in every channel and in the infrared.
pub const DUST_LETS: f64 = 0.25;

/// The film's transmission of the infrared of [`FakeScanner::with_infrared`]: nearly clear,
/// whatever the scene (films' dyes let infrared through).
pub const FILM_INFRARED: f64 = 0.85;

/// How much of the light the dust on [`FakeScanner::with_infrared`]'s strip lets through at a
/// point (inches from the adapter's top left corner): [`DUST_LETS`] on the [`SPECKS`] and the
/// [`HAIR`], 1 elsewhere.
pub fn dust(x: f64, y: f64) -> f64 {
    let speck = SPECKS
        .iter()
        .any(|&[cx, cy, r]| (x - cx).hypot(y - cy) <= r);
    let ([x0, y0, x1, y1], width) = HAIR;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let t = (((x - x0) * dx + (y - y0) * dy) / dx.mul_add(dx, dy * dy)).clamp(0.0, 1.0);
    let hair = (x - t.mul_add(dx, x0)).hypot(y - t.mul_add(dy, y0)) <= width / 2.0;
    if speck || hair {
        DUST_LETS
    } else {
        1.0
    }
}

/// [`dust`] at each pixel of a request's area, row by row.
fn dust_map(request: &ScanRequest) -> Vec<f64> {
    let a = request.area;
    let dpi = f64::from(request.dpi);
    (0..a.height)
        .flat_map(|row| {
            let gy = (f64::from(a.y) + f64::from(row) + 0.5) / dpi;
            (0..a.width).map(move |col| dust((f64::from(a.x) + f64::from(col) + 0.5) / dpi, gy))
        })
        .collect()
}

/// The adapter's area of [`FakeScanner::with_two_strips`], inches: wide
/// enough for two strips side by side, as the Epson 4490's holder.
pub const TWO_STRIPS_INCHES: (f64, f64) = (3.2, 3.756);

/// How far right the second strip of [`two_strips`] lies, inches.
pub const SECOND_STRIP: f64 = 1.5;

/// The base of the second strip of [`two_strips`]: another roll, less
/// orange.
pub const OTHER_BASE: [f64; 3] = [0.55, 0.40, 0.30];

/// Two strips side by side (as [`film`]), of two rolls: the same scenes on
/// both, the second on a base of its own ([`OTHER_BASE`]), so that each,
/// measured on its own, comes out as the other.
pub fn two_strips(x: f64, y: f64) -> [f64; 3] {
    if y < 0.33 {
        return [1.0; 3];
    }
    if x < SECOND_STRIP {
        strip(x, y, BASE)
    } else {
        strip(x - SECOND_STRIP, y, OTHER_BASE)
    }
}

/// The slides in the adapter's holder of [`FakeScanner::with_slides`]:
/// left, top, width, height (inches from the adapter's top left corner),
/// two 35 mm pictures across the holder (35 x 23.4 mm), one above the
/// other, each in its opaque mount.
pub const SLIDES: [[f64; 4]; 2] = [[0.175, 0.6, 1.378, 0.921], [0.175, 2.1, 1.378, 0.921]];

/// The correction of the slides' colours of [`FakeScanner::with_slides`]:
/// made up, more saturated colours, grey left grey.
pub const SLIDE_COLOUR: ColourMatrix = ColourMatrix {
    rows: [1.2, -0.1, -0.1, -0.1, 1.2, -0.1, -0.1, -0.1, 1.2],
    gamma: 2.2,
};

/// The correction of the glass's colours of [`FakeScanner::with_glass_colour`]: made up, more
/// saturated colours, grey left grey.
pub const GLASS_COLOUR: ColourMatrix = ColourMatrix {
    rows: [1.3, -0.15, -0.15, -0.15, 1.3, -0.15, -0.15, -0.15, 1.3],
    gamma: 1.8,
};

/// The transmission of the adapter's area with slides in the holder (as
/// [`film`]): the lamp in the slot at the top, the slides' pictures (a
/// scene lighter across, in a warm colour), the opaque mounts and holder
/// everywhere else.
pub fn slides(x: f64, y: f64) -> [f64; 3] {
    if y < 0.33 {
        return [1.0; 3];
    }
    for [left, top, width, height] in SLIDES {
        let (u, v) = ((x - left) / width, (y - top) / height);
        if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
            let stripe = if (v * 6.0) as i32 % 2 == 0 { 1.0 } else { 0.7 };
            let light = (0.05 + 0.8 * u) * stripe;
            return [0.8, 0.6, 0.4].map(|c| c * light);
        }
    }
    [0.002; 3]
}

/// What lies in the adapter's holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Holder {
    /// A strip of negatives ([`film`]).
    #[default]
    Strip,
    /// Two slides ([`slides`]).
    Slides,
    /// Two strips of two rolls side by side ([`two_strips`]).
    TwoStrips,
}

/// The linear RGB samples of an area of the glass, or of the adapter's area
/// for a film (with `holder` in it), at 16 bits.
pub fn samples16(request: &ScanRequest, holder: Holder) -> Vec<u16> {
    let a = request.area;
    let (width, height) = (a.width as usize, a.height as usize);
    let dpi = f64::from(request.dpi);
    let mut rgb = Vec::with_capacity(width * height * 3);
    for row in 0..height {
        let gy = (f64::from(a.y) + row as f64 + 0.5) / dpi;
        for col in 0..width {
            let gx = (f64::from(a.x) + col as f64 + 0.5) / dpi;
            if request.source.is_film() {
                let t = match holder {
                    Holder::Strip => film(gx, gy),
                    Holder::Slides => slides(gx, gy),
                    Holder::TwoStrips => two_strips(gx, gy),
                };
                rgb.extend(t.map(|t| (t * 65535.0).round() as u16));
            } else {
                rgb.extend(glass(gx, gy).map(|v| u16::from(v) * 257));
            }
        }
    }
    rgb
}

/// The linear RGB samples of an area, 8 bits ([`samples16`]).
pub fn samples(request: &ScanRequest, holder: Holder) -> Vec<u8> {
    samples16(request, holder)
        .iter()
        .map(|&v| (v >> 8) as u8)
        .collect()
}

/// What a test (or the user, through the application) does to the made-up
/// scanner from outside: buttons, lamp, cable. Clones share the state.
#[derive(Debug, Clone, Default)]
pub struct Controls {
    held: Arc<AtomicU32>,
    lamp_on: Arc<AtomicBool>,
    unplugged: Arc<AtomicBool>,
    adapter_out: Arc<AtomicBool>,
    /// Stripes on the scans of the glass, thousandths (0: none).
    stripes: Arc<AtomicU32>,
    /// Slides in the adapter's holder rather than the strip.
    slides: Arc<AtomicBool>,
    /// Two strips side by side in the holder rather than one.
    two_strips: Arc<AtomicBool>,
    /// Every scan asked of it, in order (the scans refused too).
    requests: Arc<Mutex<Vec<ScanRequest>>>,
    /// A scanner that needs its firmware sent, and no copy of it found:
    /// it is listed but does not open ([`FakeScanner::needing_firmware`]).
    firmware_missing: Arc<AtomicBool>,
}

impl Controls {
    /// Holds (or lets go of) button `index`.
    pub fn hold(&self, index: usize, down: bool) {
        let bit = 1 << index;
        if down {
            self.held.fetch_or(bit, Ordering::SeqCst);
        } else {
            self.held.fetch_and(!bit, Ordering::SeqCst);
        }
    }

    /// Unplugs (or plugs back) the scanner.
    pub fn unplug(&self, unplugged: bool) {
        self.unplugged.store(unplugged, Ordering::SeqCst);
    }

    /// Unplugs (or plugs back) the film adapter's cable, for a scanner
    /// with one.
    pub fn unplug_adapter(&self, unplugged: bool) {
        self.adapter_out.store(unplugged, Ordering::SeqCst);
    }

    /// Takes away (or puts in place) the copy of the firmware a scanner
    /// [`FakeScanner::needing_firmware`] needs.
    pub fn firmware_missing(&self, missing: bool) {
        self.firmware_missing.store(missing, Ordering::SeqCst);
    }

    /// Puts stripes on its scans, of the glass and of films (as the HP
    /// 3770's), at the resolutions the scanner declares them ([`Capabilities::stripes`]): each line
    /// lighter or darker by up to `thousandths`, the pattern repeating as
    /// declared. 0 takes them away.
    pub fn stripes(&self, thousandths: u32) {
        self.stripes.store(thousandths, Ordering::SeqCst);
    }

    /// Puts `holder` in the adapter (a strip, two strips, or the slides).
    pub fn load(&self, holder: Holder) {
        self.slides
            .store(holder == Holder::Slides, Ordering::SeqCst);
        self.two_strips
            .store(holder == Holder::TwoStrips, Ordering::SeqCst);
    }

    /// What is in the adapter's holder.
    pub fn holder(&self) -> Holder {
        if self.slides.load(Ordering::SeqCst) {
            Holder::Slides
        } else if self.two_strips.load(Ordering::SeqCst) {
            Holder::TwoStrips
        } else {
            Holder::Strip
        }
    }

    /// Whether the lamp is on.
    pub fn lamp_on(&self) -> bool {
        self.lamp_on.load(Ordering::SeqCst)
    }

    /// Every scan asked of the scanner so far, in order.
    pub fn requests(&self) -> Vec<ScanRequest> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

/// The made-up scanner.
#[derive(Debug, Clone)]
pub struct FakeScanner {
    caps: Capabilities,
    /// How long each stage of a scan takes (so that a scan can be stopped).
    pace: Duration,
    controls: Controls,
}

impl FakeScanner {
    /// A flatbed like the HP Scanjet 3770: A4 glass, 200 to 2400 dpi, 8
    /// and 16 bits, Scan, Copy and Instant Share, a lamp, the HP driver's
    /// tone curve.
    pub fn flatbed() -> Self {
        Self::with(Capabilities {
            manufacturer: "Test".into(),
            model: "Flatbed A4".into(),
            usb: None,
            glass_inches: (8.5, 11.7),
            resolutions: vec![200, 300, 600, 1200, 2400],
            modes: vec![ColourMode::Colour, ColourMode::Grey, ColourMode::Lineart],
            depths: vec![8, 16],
            transparency: None,
            buttons: vec![
                button("scan", "Scan", SuggestedAction::Scan),
                button("copy", "Copy", SuggestedAction::Copy),
                button("share", "Instant Share", SuggestedAction::Share),
            ],
            lamp: Some(LampInfo {
                warm_up: Duration::from_secs(30),
                // Made up: the app offers it as its longest choice.
                switches_off_after: Some(Duration::from_secs(15 * 60)),
            }),
            tone: Tone {
                white: [196.0, 186.0, 184.0],
                grey_white: 196.0,
                gamma: 2.2,
            },
            // As the HP 3770's; the glass has them only when a test asks
            // (Controls::stripes).
            stripes: [(200, 5), (300, 10), (600, 5), (1200, 5)]
                .map(|(dpi, repeat)| Stripes {
                    source: Source::Glass,
                    dpi,
                    repeat,
                })
                .to_vec(),
            colour: Vec::new(),
            // As the HP 3770's at 2400 dpi.
            scan_memory: vec![ScanMemory {
                dpi: 2400,
                times: 2.0,
            }],
            infrared: Vec::new(),
        })
    }

    /// A different one: A3 glass, 150 to 1200 dpi, 8 bits only, two
    /// buttons, no lamp to switch.
    pub fn a3() -> Self {
        Self::with(Capabilities {
            manufacturer: "Test".into(),
            model: "Flatbed A3".into(),
            usb: None,
            glass_inches: (11.7, 16.5),
            resolutions: vec![150, 300, 600, 1200],
            modes: vec![ColourMode::Colour, ColourMode::Grey],
            depths: vec![8],
            transparency: None,
            buttons: vec![
                button("start", "Start", SuggestedAction::Scan),
                button("send", "Send", SuggestedAction::Share),
            ],
            lamp: None,
            tone: Tone {
                white: [230.0; 3],
                grey_white: 230.0,
                gamma: 2.2,
            },
            stripes: Vec::new(),
            colour: Vec::new(),
            scan_memory: Vec::new(),
            infrared: Vec::new(),
        })
    }

    /// [`Self::flatbed`] with an adapter for negatives ([`ADAPTER_INCHES`],
    /// a strip in its holder: [`film`]), plugged in.
    pub fn with_adapter() -> Self {
        let mut s = Self::flatbed();
        s.caps.model = "Flatbed A4 film".into();
        s.caps.transparency = Some(Adapter {
            area_inches: ADAPTER_INCHES,
            sources: vec![Source::Negative],
            resolutions: vec![600, 1200],
        });
        // The same sensor sees films: the same stripes (as the HP 3770's).
        s.caps.stripes.extend([600, 1200].map(|dpi| Stripes {
            source: Source::Negative,
            dpi,
            repeat: 5,
        }));
        s
    }

    /// [`Self::with_adapter`] taking slides as well as negatives, as the HP
    /// 3770's (the same stripes on slides as on negatives), with
    /// two slides in its holder ([`slides`]) and a correction of their
    /// colours ([`SLIDE_COLOUR`]).
    pub fn with_slides() -> Self {
        let mut s = Self::with_adapter();
        s.caps.model = "Flatbed A4 slides".into();
        if let Some(a) = s.caps.transparency.as_mut() {
            a.sources = vec![Source::Slide, Source::Negative];
        }
        s.caps.stripes.extend([600, 1200].map(|dpi| Stripes {
            source: Source::Slide,
            dpi,
            repeat: 5,
        }));
        s.caps.colour.push(ColourCorrection {
            source: Source::Slide,
            matrix: SLIDE_COLOUR,
        });
        s.controls.load(Holder::Slides);
        s
    }

    /// [`Self::with_adapter`] with a holder wide enough for two strips side
    /// by side ([`TWO_STRIPS_INCHES`]), as the Epson 4490's, holding two
    /// strips of two rolls ([`two_strips`]).
    pub fn with_two_strips() -> Self {
        let mut s = Self::with_adapter();
        s.caps.model = "Flatbed A4 two strips".into();
        if let Some(a) = s.caps.transparency.as_mut() {
            a.area_inches = TWO_STRIPS_INCHES;
        }
        s.controls.load(Holder::TwoStrips);
        s
    }

    /// [`Self::with_adapter`] reading an infrared channel from negatives at both its film
    /// resolutions, as the Epson 4490 with Digital ICE; its strip has [`dust`] on it, which the
    /// infrared shows dark on a nearly clear film ([`FILM_INFRARED`]).
    pub fn with_infrared() -> Self {
        let mut s = Self::with_adapter();
        s.caps.model = "Flatbed A4 infrared".into();
        s.caps.infrared = [600, 1200]
            .map(|dpi| Infrared {
                source: Source::Negative,
                dpi,
            })
            .to_vec();
        s
    }

    /// [`Self::flatbed`] declaring a correction of the glass's colours ([`GLASS_COLOUR`]), as
    /// the Epson 4490 declares Epson Scan's.
    pub fn with_glass_colour() -> Self {
        let mut s = Self::flatbed();
        s.caps.model = "Flatbed A4 colour".into();
        s.caps.colour.push(ColourCorrection {
            source: Source::Glass,
            matrix: GLASS_COLOUR,
        });
        s
    }

    /// A made-up scanner that can do what `caps` says.
    pub fn with(caps: Capabilities) -> Self {
        let controls = Controls::default();
        controls
            .lamp_on
            .store(caps.lamp.is_some(), Ordering::SeqCst);
        Self {
            caps,
            pace: Duration::ZERO,
            controls,
        }
    }

    /// Drawing its pages through a tone curve of `gamma` (the Epson 4490's
    /// glass declares 1.8).
    pub fn with_gamma(mut self, gamma: f64) -> Self {
        self.caps.tone.gamma = gamma;
        self
    }

    /// Each stage of a scan takes `pace`.
    pub fn paced(mut self, pace: Duration) -> Self {
        self.pace = pace;
        self
    }

    /// A scanner that needs its firmware sent at every power-on, as the
    /// Epson 4490 does, with no copy of it found: listed, but opening it
    /// says [`Error::FirmwareMissing`] until
    /// [`Controls::firmware_missing`]`(false)` puts one in place.
    pub fn needing_firmware(self) -> Self {
        self.controls.firmware_missing(true);
        self
    }

    /// The handle on its buttons, lamp and cable.
    pub fn controls(&self) -> Controls {
        self.controls.clone()
    }

    fn plugged(&self) -> Result<()> {
        if self.controls.unplugged.load(Ordering::SeqCst) {
            Err(Error::Disconnected(
                "the made-up scanner is unplugged".into(),
            ))
        } else {
            Ok(())
        }
    }
}

fn button(id: &str, label: &str, action: SuggestedAction) -> ButtonInfo {
    ButtonInfo {
        id: id.into(),
        label: label.into(),
        action,
    }
}

impl Scanner for FakeScanner {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn status(&mut self) -> Result<DeviceStatus> {
        self.plugged()?;
        let lamp = self.caps.lamp.map(|_| {
            if self.controls.lamp_on() {
                LampState::On
            } else {
                LampState::Off
            }
        });
        Ok(DeviceStatus {
            ready: true,
            lamp,
            adapter: self
                .caps
                .transparency
                .as_ref()
                .map(|_| !self.controls.adapter_out.load(Ordering::SeqCst)),
            details: vec![("Link".into(), "in memory".into())],
        })
    }

    fn scan(
        &mut self,
        request: &ScanRequest,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<ScanResult> {
        if let Ok(mut asked) = self.controls.requests.lock() {
            asked.push(*request);
        }
        self.plugged()?;
        check(&self.caps, request)?;
        if request.source.is_film() && self.controls.adapter_out.load(Ordering::SeqCst) {
            return Err(Error::Other("the film adapter is not plugged in".into()));
        }
        let stages = [
            Progress::Starting,
            Progress::Calibrating,
            Progress::Scanning { done: 1, total: 2 },
            Progress::Finishing,
        ];
        for stage in stages {
            if !progress(stage) {
                return Err(Error::Cancelled);
            }
            std::thread::sleep(self.pace);
        }
        if self.caps.lamp.is_some() {
            self.controls.lamp_on.store(true, Ordering::SeqCst);
        }
        let mut deep = samples16(request, self.controls.holder());
        // The dust lies on the strip of the scanner that reads infrared, so that the infrared has
        // something to show.
        let dusty = !self.caps.infrared.is_empty()
            && request.source.is_film()
            && self.controls.holder() == Holder::Strip;
        let dust = dusty.then(|| dust_map(request));
        if let Some(dust) = &dust {
            for (pixel, &k) in deep.chunks_mut(3).zip(dust) {
                for v in pixel {
                    *v = (f64::from(*v) * k).round() as u16;
                }
            }
        }
        let infrared = request.infrared.then(|| match &dust {
            Some(dust) => dust
                .iter()
                .map(|&k| (FILM_INFRARED * k * 65535.0).round() as u16)
                .collect(),
            None => vec![
                (FILM_INFRARED * 65535.0).round() as u16;
                request.area.width as usize * request.area.height as usize
            ],
        });
        let amplitude = f64::from(self.controls.stripes.load(Ordering::SeqCst)) / 1000.0;
        if let Some(repeat) = self.caps.stripes_at(request.source, request.dpi) {
            if amplitude > 0.0 {
                let stride = request.area.width as usize * 3;
                for (y, line) in deep.chunks_mut(stride.max(1)).enumerate() {
                    let phase = std::f64::consts::TAU * (y % repeat) as f64 / repeat as f64;
                    let k = 1.0 + amplitude * phase.sin();
                    for v in line {
                        *v = (f64::from(*v) * k).round().min(65535.0) as u16;
                    }
                }
            }
        }
        let rgb = deep.iter().map(|&v| (v >> 8) as u8).collect();
        let samples16 = (request.depth == 16).then_some(deep);
        Ok(ScanResult {
            image: Image {
                width: request.area.width as usize,
                height: request.area.height as usize,
                dpi: request.dpi,
                format: Format::Rgb,
                data: rgb,
            },
            samples16,
            infrared,
            details: Vec::new(),
        })
    }

    fn poll_buttons(&mut self) -> Result<u32> {
        self.plugged()?;
        let mask = crate::button_mask(self.caps.buttons.len());
        Ok(self.controls.held.load(Ordering::SeqCst) & mask)
    }

    fn lamp_off(&mut self) -> Result<()> {
        self.plugged()?;
        if self.caps.lamp.is_none() {
            return Err(Error::Unsupported("lamp_off"));
        }
        self.controls.lamp_on.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// The file a scanner [`FakeScanner::needing_firmware`] needs.
pub const FIRMWARE_FILE: &str = "made-up-firmware.bin";

/// The made-up scanners as a backend: one device each.
#[derive(Debug, Clone)]
pub struct FakeBackend {
    scanners: Vec<FakeScanner>,
}

impl FakeBackend {
    /// A backend with these scanners.
    pub fn new(scanners: Vec<FakeScanner>) -> Self {
        Self { scanners }
    }
}

impl Backend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn list(&self) -> Vec<DeviceInfo> {
        self.scanners
            .iter()
            .enumerate()
            .map(|(i, s)| DeviceInfo {
                backend: "fake",
                id: i.to_string(),
                name: s.caps.name(),
            })
            .collect()
    }

    fn models(&self) -> Vec<Capabilities> {
        self.scanners.iter().map(|s| s.caps.clone()).collect()
    }

    fn open(&self, device: &DeviceInfo) -> Result<Box<dyn Scanner>> {
        let scanner = device
            .id
            .parse::<usize>()
            .ok()
            .and_then(|i| self.scanners.get(i))
            .ok_or_else(|| Error::Disconnected(format!("no made-up scanner {}", device.id)))?;
        if scanner.controls.firmware_missing.load(Ordering::SeqCst) {
            return Err(Error::FirmwareMissing(crate::FirmwareMissing {
                file: FIRMWARE_FILE.into(),
                looked_in: vec![std::path::PathBuf::from("/made-up/firmware")],
                detail: format!("/made-up/firmware/{FIRMWARE_FILE}: not there"),
            }));
        }
        Ok(Box::new(scanner.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Area, Registry};

    fn request(dpi: u16, depth: u8) -> ScanRequest {
        ScanRequest {
            dpi,
            source: Source::Glass,
            area: Area {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            },
            depth,
            infrared: false,
        }
    }

    #[test]
    fn a_scan_gives_linear_rgb_of_the_area() {
        let mut s = FakeScanner::flatbed();
        let mut stages = Vec::new();
        let r = s
            .scan(&request(300, 16), &mut |p| {
                stages.push(p);
                true
            })
            .unwrap();
        assert_eq!((r.image.width, r.image.height), (400, 300));
        assert_eq!(r.image.format, Format::Rgb);
        assert_eq!(r.image.data.len(), 400 * 300 * 3);
        assert_eq!(r.samples16.unwrap().len(), 400 * 300 * 3);
        assert_eq!(stages.first(), Some(&Progress::Starting));
        assert_eq!(stages.last(), Some(&Progress::Finishing));
        // The first original starts 0.8 inches in: lid at the corner.
        assert_eq!(&r.image.data[..3], &LID);
        let at = |x: usize, y: usize| &r.image.data[(y * 400 + x) * 3..][..3];
        assert_ne!(at(350, 280), &LID[..], "an original");
    }

    #[test]
    fn a_scanner_without_its_firmware_is_listed_but_does_not_open() {
        let scanner = FakeScanner::flatbed().needing_firmware();
        let controls = scanner.controls();
        let registry = Registry::new().with(FakeBackend::new(vec![scanner]));
        let devices = registry.list();
        assert_eq!(devices.len(), 1, "it is there");
        match registry.open(&devices[0]) {
            Err(Error::FirmwareMissing(m)) => {
                assert_eq!(m.file, FIRMWARE_FILE);
                assert!(!m.looked_in.is_empty());
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        // The first scanner there is says the same, not "not there".
        assert!(matches!(
            registry.open_first().map(|_| ()),
            Err(Error::FirmwareMissing(_))
        ));
        controls.firmware_missing(false);
        assert!(registry.open(&devices[0]).is_ok(), "a copy put in place");
    }

    #[test]
    fn a_scan_stops_when_asked() {
        let mut s = FakeScanner::flatbed();
        let mut seen = 0;
        let r = s.scan(&request(300, 8), &mut |_| {
            seen += 1;
            seen < 2
        });
        assert_eq!(r.unwrap_err(), Error::Cancelled);
    }

    #[test]
    fn requests_outside_the_capabilities_are_refused() {
        let mut s = FakeScanner::a3();
        let go = &mut |_| true;
        assert!(matches!(
            s.scan(&request(2400, 8), go),
            Err(Error::InvalidRequest(_))
        ));
        assert!(matches!(
            s.scan(&request(300, 16), go),
            Err(Error::InvalidRequest(_))
        ));
        let mut too_big = request(300, 8);
        too_big.area.width = 11.7f64.mul_add(300.0, 1.0) as u32;
        assert!(matches!(
            s.scan(&too_big, go),
            Err(Error::InvalidRequest(_))
        ));
        assert!(s.scan(&request(300, 8), go).is_ok());
    }

    #[test]
    fn buttons_lamp_and_cable() {
        let mut s = FakeScanner::flatbed();
        let c = s.controls();
        assert_eq!(s.poll_buttons().unwrap(), 0);
        c.hold(1, true);
        assert_eq!(s.poll_buttons().unwrap(), 0b10);
        c.hold(1, false);
        assert_eq!(s.status().unwrap().lamp, Some(LampState::On));
        s.lamp_off().unwrap();
        assert_eq!(s.status().unwrap().lamp, Some(LampState::Off));
        c.unplug(true);
        assert!(matches!(s.status(), Err(Error::Disconnected(_))));
        c.unplug(false);
        // No lamp: nothing to switch off, and none in the status.
        let mut a3 = FakeScanner::a3();
        assert_eq!(a3.lamp_off(), Err(Error::Unsupported("lamp_off")));
        assert_eq!(a3.status().unwrap().lamp, None);
        // Only the bits of its two buttons.
        a3.controls().hold(5, true);
        assert_eq!(a3.poll_buttons().unwrap(), 0);
    }

    #[test]
    fn the_adapter_scans_its_own_area_and_says_when_it_is_unplugged() {
        let mut s = FakeScanner::with_adapter();
        let go = &mut |_| true;
        assert_eq!(s.status().unwrap().adapter, Some(true));
        assert_eq!(FakeScanner::flatbed().status().unwrap().adapter, None);
        let (w, h) = s.caps.bed_pixels(Source::Negative, 600).unwrap();
        assert_eq!((w, h), (1039, 2254));
        // Films only at the adapter's own resolutions.
        let low = ScanRequest {
            source: Source::Negative,
            area: Area {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            ..request(200, 16)
        };
        assert!(matches!(s.scan(&low, go), Err(Error::InvalidRequest(_))));
        let film = ScanRequest {
            source: Source::Negative,
            area: Area {
                x: 0,
                y: 0,
                width: w,
                height: h,
            },
            ..request(600, 16)
        };
        let r = s.scan(&film, go).unwrap();
        let deep = r.samples16.unwrap();
        let at = |x: usize, y: usize| deep[(y * w as usize + x) * 3..][..3].to_vec();
        assert_eq!(at(510, 60), [65535; 3], "the slot: the lamp");
        assert!(at(15, 1200).iter().all(|&v| v < 200), "the holder");
        let base = BASE.map(|t| (t * 65535.0).round() as u16);
        assert_eq!(at(510, 285), base, "the base, above the first frame");
        // Wider than the adapter's area: refused, though it is within the
        // glass.
        let mut wide = film;
        wide.area.width = w + 1;
        assert!(matches!(s.scan(&wide, go), Err(Error::InvalidRequest(_))));
        // Without an adapter, or with it unplugged, no film.
        assert!(matches!(
            FakeScanner::flatbed().scan(&film, go),
            Err(Error::InvalidRequest(_))
        ));
        s.controls().unplug_adapter(true);
        assert_eq!(s.status().unwrap().adapter, Some(false));
        assert!(matches!(s.scan(&film, go), Err(Error::Other(_))));
        // Slides: not taken by this adapter.
        let slide = ScanRequest {
            source: Source::Slide,
            ..film
        };
        s.controls().unplug_adapter(false);
        assert!(matches!(s.scan(&slide, go), Err(Error::InvalidRequest(_))));
    }

    /// An adapter for slides and negatives: what it scans is what lies in
    /// its holder, whichever film the request says.
    #[test]
    fn slides_in_the_holder_until_the_strip_goes_in() {
        let mut s = FakeScanner::with_slides();
        let go = &mut |_| true;
        assert!(s.caps.offers(Source::Slide) && s.caps.offers(Source::Negative));
        assert_eq!(s.caps.stripes_at(Source::Slide, 600), Some(5));
        assert_eq!(s.caps.colour_at(Source::Slide), Some(SLIDE_COLOUR));
        assert_eq!(s.caps.colour_at(Source::Negative), None);
        let (w, h) = s.caps.bed_pixels(Source::Slide, 600).unwrap();
        let slide = ScanRequest {
            source: Source::Slide,
            area: Area {
                x: 0,
                y: 0,
                width: w,
                height: h,
            },
            ..request(600, 16)
        };
        let deep = s.scan(&slide, go).unwrap().samples16.unwrap();
        let at = |deep: &[u16], x: usize, y: usize| deep[(y * w as usize + x) * 3..][..3].to_vec();
        assert_eq!(at(&deep, 510, 60), [65535; 3], "the slot: the lamp");
        // In the first slide, at a quarter across: its warm picture.
        let p = at(&deep, 450, 600);
        assert!(p[0] > p[1] && p[1] > p[2] && p[2] > 1000, "{p:?}");
        assert!(at(&deep, 510, 285).iter().all(|&v| v < 200), "the mount");
        s.controls().load(Holder::Strip);
        let deep = s.scan(&slide, go).unwrap().samples16.unwrap();
        let base = BASE.map(|t| (t * 65535.0).round() as u16);
        assert_eq!(at(&deep, 510, 285), base, "the strip's base");
    }

    #[test]
    fn the_registry_lists_and_opens_every_backend() {
        let registry = Registry::new().with(FakeBackend::new(vec![
            FakeScanner::flatbed(),
            FakeScanner::a3(),
        ]));
        let devices = registry.list();
        let names: Vec<_> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Test Flatbed A4", "Test Flatbed A3"]);
        let second = registry.open(&devices[1]).unwrap();
        assert_eq!(second.capabilities().max_dpi(), 1200);
        assert_eq!(second.capabilities().glass_pixels(100), (1170, 1650));
        assert_eq!(
            registry.open_first().unwrap().capabilities().model,
            "Flatbed A4"
        );
        assert!(Registry::new().open_first().is_err());
        assert_eq!(registry.models().len(), 2);
        assert_eq!(registry.models()[1].model, "Flatbed A3");
    }

    #[test]
    fn a_film_without_an_adapter_is_refused_for_its_source() {
        let mut r = request(1200, 8);
        r.dpi = 2400;
        r.source = Source::Negative;
        let e = check(FakeScanner::a3().capabilities(), &r)
            .unwrap_err()
            .to_string();
        assert!(e.contains("does not scan from Negative"), "{e}");
    }

    #[test]
    fn the_button_mask_covers_up_to_32_buttons() {
        assert_eq!(crate::button_mask(0), 0);
        assert_eq!(crate::button_mask(3), 0b111);
        assert_eq!(crate::button_mask(32), u32::MAX);
        assert_eq!(crate::button_mask(40), u32::MAX);
    }

    #[test]
    fn infrared_only_where_declared_and_dark_on_the_dust() {
        let go = &mut |_| true;
        // The first frame at 600 dpi: the speck at (0.7, 0.9) inches and the hair are in it.
        let film = ScanRequest {
            source: Source::Negative,
            area: Area {
                x: 240,
                y: 330,
                width: 567,
                height: 850,
            },
            infrared: true,
            ..request(600, 16)
        };
        let mut s = FakeScanner::with_infrared();
        assert!(s.caps.infrared_at(Source::Negative, 600));
        assert!(!s.caps.infrared_at(Source::Glass, 600));
        let r = s.scan(&film, go).unwrap();
        let ir = r.infrared.unwrap();
        assert_eq!(ir.len(), 567 * 850, "one sample a pixel");
        let clear = (FILM_INFRARED * 65535.0).round() as u16;
        // The speck's centre, (0.7, 0.9) inches: 420 - 240 across, 540 - 330 down.
        let (x, y) = (420 - 240, 540 - 330);
        assert!(ir[y * 567 + x] < clear / 3, "dark on the speck");
        assert_eq!(ir[y * 567 + x + 20], clear, "clear beside it");
        let deep = r.samples16.unwrap();
        let rgb = |x: usize| deep[(y * 567 + x) * 3..][..3].to_vec();
        assert!(
            (0..3).all(|c| rgb(x)[c] < rgb(x + 20)[c] / 2),
            "the dust in the colours too: {:?} {:?}",
            rgb(x),
            rgb(x + 20)
        );
        // Not asked: none.
        let without = ScanRequest {
            infrared: false,
            ..film
        };
        assert_eq!(s.scan(&without, go).unwrap().infrared, None);
        // Refused where it is not declared: another resolution, the glass, another scanner.
        for (scanner, request) in [
            (
                FakeScanner::with_infrared(),
                ScanRequest { dpi: 1200, ..film },
            ),
            (
                FakeScanner::with_infrared(),
                ScanRequest {
                    source: Source::Glass,
                    ..film
                },
            ),
            (FakeScanner::with_adapter(), film),
        ] {
            let mut scanner = scanner;
            let declared = scanner.caps.infrared_at(request.source, request.dpi);
            let e = scanner.scan(&request, go);
            assert_eq!(e.is_ok(), declared, "{request:?}");
        }
        let plain = FakeScanner::with_adapter()
            .scan(&without, go)
            .unwrap()
            .samples16
            .unwrap();
        assert!(
            plain[(y * 567 + x) * 3] > 2 * rgb(x)[0],
            "no dust on its strip"
        );
    }
}
