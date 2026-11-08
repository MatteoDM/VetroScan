// SPDX-License-Identifier: GPL-2.0-or-later
//! Scan modes and sensor timings of the HP Scanjet 3770.
//!
//! One scan mode per resolution says how the chip runs: system clock, line
//! period (`CTPC`), sensor timing, motor curve. The values come from the
//! configuration embedded in the HP driver (`hpgt3770.dll`): sections
//! `[ScanColorMode1]`-`[ScanColorMode5]` (line 14754 of the extracted file)
//! and the `[Timing]` sections (line 15082). Only the numbers are copied,
//! with their source, as `hp3900` does in `hp3900_config.c`. Where a key
//! has several values the HP driver uses the first.
//!
//! The scanner always reads colour: grey and black and white are made from
//! the colour image in software, so only the colour modes are here.
//!
//! As with the motor curves, the `[Timing]` sections of the extracted file
//! are not in the order of their numbers: timing 1 is the fourth section,
//! timings 1-9 follow, and the first three sections are timings 10-12 (not
//! used by the flatbed). The captures prove it: the sensor clocks the HP
//! driver writes at 300, 600 and 1200 dpi are those of timings 3, 2 and 1
//! (sensor resolution 300, 600 and 1200).

use crate::motor::{MotorSetting, StepType};
use crate::motor_curves;

/// One clock of the linear image sensor (`CPHnP1`, `CPHnP2`, `CPHnPS`,
/// `CPHnGE`, `CPHnGO`; `st_cph` in hp3900). The patterns are 36 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cph {
    /// First pattern.
    pub p1: u64,
    /// Second pattern.
    pub p2: u64,
    /// `PS` bit.
    pub ps: bool,
    /// `GE` bit.
    pub ge: bool,
    /// `GO` bit.
    pub go: bool,
}

/// Timing of the CCD sensor and of the analog front end (`[Timing]`,
/// `st_timing` in hp3900).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Resolution the sensor is read at (`SensorResolution`).
    pub sensor_resolution: u16,
    /// Pixel clock divider (`CNPP`), written to `E896`.
    pub cnpp: u8,
    /// Transfer gate polarities (`CVTRP1`-`CVTRP3`).
    pub cvtrp: [bool; 3],
    /// Transfer gate width (`CVTRW`).
    pub cvtrw: u8,
    /// Transfer gate front porch (`CVTRFPW`).
    pub cvtrfpw: u8,
    /// Transfer gate back porch (`CVTRBPW`).
    pub cvtrbpw: u8,
    /// Sensor clocks `CPH0`-`CPH5`.
    pub cph: [Cph; 6],
    /// `CPHBP2S`.
    pub cphbp2s: u32,
    /// `CPHBP2E`.
    pub cphbp2e: u32,
    /// Start of the clamp pulse (`CLAMPS`).
    pub clamps: u32,
    /// End of the clamp pulse (`CLAMPE`); `None` for -1, which the HP
    /// driver replaces with the line period.
    pub clampe: Option<u32>,
    /// Correlated double sampling, sample points (`CDSS1`, `CDSS2`).
    pub cdss: [u8; 2],
    /// Correlated double sampling, clamp points (`CDSC1`, `CDSC2`).
    pub cdsc: [u8; 2],
    /// ADC clock patterns (`ADCCLKP1`, `ADCCLKP2`), 36 bits.
    pub adcclkp: [u64; 2],
    /// `ADCCLKP2E`.
    pub adcclkp2e: u8,
}

/// How the chip samples the colours (`SampleRate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRate {
    /// The three colours of a pixel together (`SampleRate=0`, colour).
    Pixel = 0,
    /// One colour per line (`SampleRate=1`, the grey modes).
    Line = 1,
}

/// One scan mode (`[ScanColorModeN]`, `st_scanmode` in hp3900, with the
/// fields the HP driver adds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanMode {
    /// Horizontal resolution the mode is for, in dpi.
    pub resolution: u16,
    /// `SampleRate`.
    pub sample_rate: SampleRate,
    /// `SystemClock`, low 4 bits of `E800`.
    pub system_clock: u8,
    /// `ScanMotorStepType`.
    pub step_type: StepType,
    /// Line period in clock ticks (`CTPC`).
    pub ctpc: u32,
    /// Sensor timing (`Timing`).
    pub timing: &'static Timing,
    /// Motor curves (`MotorCurve`); `None` for 0, constant speed.
    pub motor_curve: Option<&'static MotorSetting>,
    /// Steps the motor goes back when the buffer is full (`MotorBackStep`).
    pub motor_back_step: u16,
    /// Lines the chip reads for each line it keeps (`DummyLine`).
    pub dummy_line: u8,
    /// Use the second motor resource table (`MotorPhase=1`).
    pub motor_phase: bool,
    /// Exposure of red, green and blue (`EXPTn`, `MEXPTn`); `None` for the
    /// whole line (`EXPT` 0, `MEXPT` the line period), as on the glass.
    pub exposure: Option<[Exposure; 3]>,
}

/// The exposure of one colour in a line, in clock ticks: `EXPT` and
/// `MEXPT` of the HP configuration, written to `E836`/`E833` (red),
/// `E83C`/`E839` (green), `E842`/`E83F` (blue). How the chip uses the two is
/// not known; the HP driver's values are copied as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exposure {
    /// `EXPT`.
    pub expt: u32,
    /// `MEXPT`.
    pub mexpt: u32,
}

/// Timing 1, 1200 dpi colour: `[Timing]` section 4 of the file, line 15232.
pub const TIMING_1: Timing = Timing {
    sensor_resolution: 1200,
    cnpp: 35,
    cvtrp: [false, false, false],
    cvtrw: 15,
    cvtrfpw: 4,
    cvtrbpw: 4,
    cph: [
        Cph {
            p1: 0xFE0000000,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0xC00000003,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0x007FFFE00,
            p2: 0x000000000,
            ps: true,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0xFF80001FF,
            p2: 0x000000000,
            ps: true,
            ge: true,
            go: true,
        },
        Cph {
            p1: 0x000000000,
            p2: 0x000000000,
            ps: false,
            ge: false,
            go: false,
        },
        Cph {
            p1: 0x000000000,
            p2: 0x000000000,
            ps: false,
            ge: false,
            go: false,
        },
    ],
    cphbp2s: 0,
    cphbp2e: 0,
    clamps: 26,
    clampe: None,
    cdss: [15, 31],
    cdsc: [18, 34],
    adcclkp: [0x0FC0FC0FC, 0x000000000],
    adcclkp2e: 0,
};

/// Timing 2, 600 dpi colour: `[Timing]` section 5 of the file, line 15282.
pub const TIMING_2: Timing = Timing {
    sensor_resolution: 600,
    cnpp: 35,
    cvtrp: [false, false, false],
    cvtrw: 15,
    cvtrfpw: 4,
    cvtrbpw: 4,
    cph: [
        Cph {
            p1: 0xFE0000000,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0xC00000003,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0x00003FFFF,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0xFFFFC0000,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: true,
        },
        Cph {
            p1: 0xFFFFFFFFF,
            p2: 0x000000000,
            ps: false,
            ge: false,
            go: false,
        },
        Cph {
            p1: 0x000000000,
            p2: 0x000000000,
            ps: false,
            ge: false,
            go: false,
        },
    ],
    cphbp2s: 0,
    cphbp2e: 0,
    clamps: 26,
    clampe: None,
    cdss: [15, 31],
    cdsc: [18, 34],
    adcclkp: [0x0FC0FC0FC, 0x000000000],
    adcclkp2e: 0,
};

/// Timing 3, 300 dpi colour (also 100 dpi): `[Timing]` section 6 of the file, line 15332.
pub const TIMING_3: Timing = Timing {
    sensor_resolution: 300,
    cnpp: 35,
    cvtrp: [false, false, false],
    cvtrw: 15,
    cvtrfpw: 4,
    cvtrbpw: 4,
    cph: [
        Cph {
            p1: 0xF00000000,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0x000000003,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0x007FC01FF,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0xFF803FE00,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: true,
        },
        Cph {
            p1: 0xFFFFFFFFF,
            p2: 0x000000000,
            ps: false,
            ge: false,
            go: false,
        },
        Cph {
            p1: 0x000000000,
            p2: 0x000000000,
            ps: false,
            ge: false,
            go: false,
        },
    ],
    cphbp2s: 0,
    cphbp2e: 0,
    clamps: 26,
    clampe: None,
    cdss: [7, 32],
    cdsc: [9, 34],
    adcclkp: [0x0FC0FC0FC, 0x000000000],
    adcclkp2e: 0,
};

/// Timing 4, 200 dpi colour: `[Timing]` section 7 of the file, line 15382.
pub const TIMING_4: Timing = Timing {
    sensor_resolution: 200,
    cnpp: 35,
    cvtrp: [false, false, false],
    cvtrw: 15,
    cvtrfpw: 4,
    cvtrbpw: 4,
    cph: [
        Cph {
            p1: 0xE00000001,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0x000000003,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0x07E07E07E,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: false,
        },
        Cph {
            p1: 0xF81F81F81,
            p2: 0x000000000,
            ps: false,
            ge: true,
            go: true,
        },
        Cph {
            p1: 0xFFFFFFFFF,
            p2: 0x000000000,
            ps: false,
            ge: false,
            go: false,
        },
        Cph {
            p1: 0x000000000,
            p2: 0x000000000,
            ps: false,
            ge: false,
            go: false,
        },
    ],
    cphbp2s: 0,
    cphbp2e: 0,
    clamps: 26,
    clampe: None,
    cdss: [3, 32],
    cdsc: [5, 34],
    adcclkp: [0x0FC0FC0FC, 0x000000000],
    adcclkp2e: 0,
};

/// Colour scan modes, by resolution (`[ScanColorMode1]`-`[ScanColorMode5]`).
///
/// Keys missing from every section take the value the HP driver writes in
/// the captures: no `MEXPT` (the maximum exposure is the line period), no
/// `EXPT` (0), `MSI` 16 (`E8DA` = 10).
pub const COLOUR_MODES: [ScanMode; 5] = [
    // [ScanColorMode1] SampleRate=0 SystemClock=0x5 ScanMotorStepType=1 0
    // CTPC=11999 15983 10999 Timing=1 MotorCurve=0 1 MotorBackStep=96 0 10
    // MotorPhase=1.
    ScanMode {
        resolution: 1200,
        sample_rate: SampleRate::Pixel,
        system_clock: 5,
        step_type: StepType::Half,
        ctpc: 11999,
        timing: &TIMING_1,
        motor_curve: None,
        motor_back_step: 96,
        dummy_line: 1,
        motor_phase: true,
        exposure: None,
    },
    // [ScanColorMode2] SampleRate=0 SystemClock=0x5 ScanMotorStepType=1 0
    // CTPC=11999 15983 15999 Timing=2 MotorCurve=0 9 MotorBackStep=128 256
    // 0 10 MotorPhase=1.
    ScanMode {
        resolution: 600,
        sample_rate: SampleRate::Pixel,
        system_clock: 5,
        step_type: StepType::Half,
        ctpc: 11999,
        timing: &TIMING_2,
        motor_curve: None,
        motor_back_step: 128,
        dummy_line: 1,
        motor_phase: true,
        exposure: None,
    },
    // [ScanColorMode3] SampleRate=0 SystemClock=0x4 ScanMotorStepType=1 0
    // CTPC=2999 3983 3999 3983 2799 Timing=3 MotorCurve=10
    // MotorBackStep=256 10.
    ScanMode {
        resolution: 300,
        sample_rate: SampleRate::Pixel,
        system_clock: 4,
        step_type: StepType::Half,
        ctpc: 2999,
        timing: &TIMING_3,
        motor_curve: Some(&motor_curves::SETTING_10),
        motor_back_step: 256,
        dummy_line: 1,
        motor_phase: false,
        exposure: None,
    },
    // [ScanColorMode4] SampleRate=0 SystemClock=4 ScanMotorStepType=1
    // CTPC=1991 2999 Timing=4 MotorCurve=11 MotorBackStep=1024 256 928.
    ScanMode {
        resolution: 200,
        sample_rate: SampleRate::Pixel,
        system_clock: 4,
        step_type: StepType::Half,
        ctpc: 1991,
        timing: &TIMING_4,
        motor_curve: Some(&motor_curves::SETTING_11),
        motor_back_step: 1024,
        dummy_line: 1,
        motor_phase: false,
        exposure: None,
    },
    // [ScanColorMode5] SampleRate=0 SystemClock=4 ScanMotorStepType=1
    // CTPC=2999 3983 3007 Timing=3 4 MotorCurve=12 MotorBackStep=1024 256
    // 928 DummyLine=1 XSampleTableLength=3. No capture at 100 dpi yet.
    ScanMode {
        resolution: 100,
        sample_rate: SampleRate::Pixel,
        system_clock: 4,
        step_type: StepType::Half,
        ctpc: 2999,
        timing: &TIMING_3,
        motor_curve: Some(&motor_curves::SETTING_12),
        motor_back_step: 1024,
        dummy_line: 1,
        motor_phase: false,
        exposure: None,
    },
];

/// The exposures of a negative mode: red, green, blue as (`EXPT`, `MEXPT`).
const fn exposures(e: [(u32, u32); 3]) -> Option<[Exposure; 3]> {
    Some([
        Exposure {
            expt: e[0].0,
            mexpt: e[0].1,
        },
        Exposure {
            expt: e[1].0,
            mexpt: e[1].1,
        },
        Exposure {
            expt: e[2].0,
            mexpt: e[2].1,
        },
    ])
}

/// Colour scan modes for negatives with the transparency adapter, by
/// resolution (`[ScanNegColorMode1]`-`[ScanNegColorMode5]`, next to the
/// glass's sections in the file). The line is twice as long as on the
/// glass (the HP configuration has six and three times): with the
/// adapter's white of [`crate::profile::Transparency`] the holder's clear
/// window comes out at the full scale, and twice the light then puts a
/// film's base, which lets through at most about three quarters, near the
/// top without saturating it, the blue far above the glass's exposure.
/// Tried on 8 November 2026: six times saturated the calibration; three
/// times, with the slot calibrated to the glass's white, saturated red on
/// the base. The motor runs at constant speed (`MotorCurve=0`) at every
/// resolution.
///
/// The HP configuration also gives each colour its own exposure, kept here
/// in [`NEGATIVE_EXPOSURES`] but not used: `MEXPT` is how long the colour
/// is exposed and `EXPT` where in the line it starts (`EXPT` + `MEXPT` is
/// the line), red about 0.7 of the line, green 0.4, blue all of it. Tried on
/// 8 November 2026: the calibration's reads (which [`crate::setup`]
/// lengthens with `set_multi_exposure` when the motor runs at constant
/// speed) and the scan then do not expose red and green alike: red came
/// out at its gain's maximum in the calibration and saturated in the scan.
/// Until the chip's use of the two is understood, every colour gets the
/// whole longer line, as on the glass.
pub const NEGATIVE_MODES: [ScanMode; 5] = [
    // [ScanNegColorMode1] SystemClock=5 CTPC=71399 35999 (the second) Timing=1
    // MotorBackStep=96 256 MEXPT1=50999 EXPT1=20399 MEXPT2=30599
    // EXPT2=40799 MEXPT3=71399 EXPT3=0 MotorPhase=1.
    ScanMode {
        ctpc: 23999,
        motor_curve: None,
        motor_back_step: 96,
        motor_phase: true,
        exposure: None,
        ..COLOUR_MODES[0]
    },
    // [ScanNegColorMode2]: as mode 1, Timing=2, MotorBackStep=128.
    ScanMode {
        ctpc: 23999,
        motor_curve: None,
        motor_back_step: 128,
        motor_phase: true,
        exposure: None,
        ..COLOUR_MODES[1]
    },
    // [ScanNegColorMode3] SystemClock=4 CTPC=17807 8999 (the second) Timing=3
    // MotorBackStep=256 MEXPT1=12719 EXPT1=5087 MEXPT2=7631 EXPT2=10175
    // MEXPT3=17807 EXPT3=0 MotorPhase=1.
    ScanMode {
        ctpc: 5999,
        motor_curve: None,
        motor_back_step: 256,
        motor_phase: true,
        exposure: None,
        ..COLOUR_MODES[2]
    },
    // [ScanNegColorMode4] SystemClock=4 CTPC=11759 5975 (the second) Timing=4
    // MotorBackStep=1024 MEXPT1=8399 EXPT1=3359 MEXPT2=5039 EXPT2=6719
    // MEXPT3=11759 EXPT3=0 MotorPhase=1.
    ScanMode {
        ctpc: 3983,
        motor_curve: None,
        motor_back_step: 1024,
        motor_phase: true,
        exposure: None,
        ..COLOUR_MODES[3]
    },
    // [ScanNegColorMode5]: the exposures of mode 3, Timing=3,
    // MotorBackStep=1024.
    ScanMode {
        ctpc: 5999,
        motor_curve: None,
        motor_back_step: 1024,
        motor_phase: true,
        exposure: None,
        ..COLOUR_MODES[4]
    },
];

/// The HP configuration's exposures of red, green and blue (`EXPTn`,
/// `MEXPTn`) for each mode of [`NEGATIVE_MODES`], in the same order. Not
/// used (see [`NEGATIVE_MODES`]).
pub const NEGATIVE_EXPOSURES: [Option<[Exposure; 3]>; 5] = [
    exposures([(20399, 50999), (40799, 30599), (0, 71399)]),
    exposures([(20399, 50999), (40799, 30599), (0, 71399)]),
    exposures([(5087, 12719), (10175, 7631), (0, 17807)]),
    exposures([(3359, 8399), (6719, 5039), (0, 11759)]),
    exposures([(5087, 12719), (10175, 7631), (0, 17807)]),
];

/// The scan mode for negatives at a horizontal resolution.
pub fn negative_mode(resolution: u16) -> Option<&'static ScanMode> {
    NEGATIVE_MODES.iter().find(|m| m.resolution == resolution)
}

/// The colour scan mode for a horizontal resolution (`RTS_GetScanmode`).
pub fn colour_mode(resolution: u16) -> Option<&'static ScanMode> {
    COLOUR_MODES.iter().find(|m| m.resolution == resolution)
}

/// Motor current level of a scan at a horizontal resolution, for
/// `Motor_Change` (`[CurrentLevel]`, first value of each key: the HP driver
/// reads only that one). Checked on the second block of the scans in the
/// captures: bits 4-5 of `E954` are 1 at 200 and 300 dpi (01, 02, 04, 07-11)
/// and 0 at 600 and 1200 dpi (05, 06, 06b). 100 dpi has no capture.
pub fn motor_current_level(resolution: u16) -> u8 {
    match resolution {
        // CurrentLevel_100=3 3 0
        100 => 3,
        // CurrentLevel_200=1 3 0 3 0 3 0 3, CurrentLevel_300=1 3 0 3 0 3 0 3
        200 | 300 => 1,
        // CurrentLevel_600=0 3 3 0, CurrentLevel_1200=0, CurrentLevel_2400=0
        _ => 0,
    }
}
