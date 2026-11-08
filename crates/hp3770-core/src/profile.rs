// SPDX-License-Identifier: GPL-2.0-or-later
//! Parameters of the HP Scanjet 3770.
//!
//! The values come from the configuration embedded in the HP driver
//! (`hpgt3770.dll`, same key names as the tables of `hp3900_config.c`) and
//! are checked against the USB captures of the HP driver. Only the numbers
//! are copied, with their source, as `hp3900` does for its models.
//!
//! The DLL holds six configurations (one `[SCAN_PARAM]` and `[PlatForm]`
//! each). The one this scanner uses is the pair whose `[SCAN_PARAM]` starts
//! with `PID&VID=0x05DA5004` (the two differ only in values hp3900 does not
//! read): its reference voltages (2, 3, 2), binary threshold (99, 100) and
//! park motor timing (`CTPC=1991`) are what the HP driver writes in the
//! captures, where the first configuration of the file says 0, 0, 0 and
//! 127, 128. "Configuration" below means that one.

use crate::calibration::CalibrationConfig;
use crate::device::{self, Button};
use crate::geometry;
use crate::motor::{MotorMove, StepType};
use crate::motor_curves;
use scan_imaging::adjust::Tone;

/// Spread-spectrum clock generator settings (`SSCGEnable`, `SSCGMode`,
/// `SSCGClockControl` in `[SCAN_PARAM]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sscg {
    /// Spread spectrum on.
    pub enable: bool,
    /// Modulation mode (0 or 1).
    pub mode: u8,
    /// Spread amount: 0 = 0.25 %, 1 = 0.50 %, 2 = 0.75 %, 3 = 1.00 %.
    pub clock: u8,
}

/// Sensor and motor of the flatbed (`[PlatForm]` and `[MoveMotorSetting]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    /// Optical resolution of the CCD (`RealSensorResolution`).
    pub sensor_resolution: u16,
    /// Distance between the colour lines of the CCD, in lines at the
    /// sensor resolution (`SensorLineDistance`).
    pub line_distance: u8,
    /// Distance between the even and odd pixel rows
    /// (`SensorEvenOddDistance`).
    pub even_odd_distance: u8,
    /// Steps of the motor per inch (`MotorResolution`).
    pub motor_resolution: u16,
    /// Speed of the motor without curves, in steps per second
    /// (`BaseSpeedPPS`).
    pub base_speed_pps: u16,
    /// Order of the colours in the sensor (`SensorChannelColor`, from 0).
    pub channel_color: [u8; 3],
    /// Order of the colours sent to the host (`RGBChannelOrder`, from 0).
    pub rgb_order: [u8; 3],
    /// Second motor resource table, for the modes with `MotorPhase=1`.
    pub motor_resource_phase: [u8; 32],
}

/// What a scanner is, as the user and the application see it: name, USB
/// identity, glass, resolutions, buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceModel {
    /// Who makes it ("HP").
    pub manufacturer: &'static str,
    /// The model ("Scanjet 3770").
    pub name: &'static str,
    /// USB vendor id.
    pub vendor_id: u16,
    /// USB product id.
    pub product_id: u16,
    /// The glass, width and height, hundredths of an inch.
    pub glass: (u16, u16),
    /// The resolutions offered for a scan, dpi, smallest first.
    pub resolutions: &'static [u16],
    /// The front-panel buttons, left to right.
    pub buttons: &'static [Button],
}

impl DeviceModel {
    /// The glass, inches.
    pub fn glass_inches(&self) -> (f64, f64) {
        (
            f64::from(self.glass.0) / 100.0,
            f64::from(self.glass.1) / 100.0,
        )
    }
}

/// Parameters of one scanner model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// What it is.
    pub device: DeviceModel,
    /// Spread-spectrum clock.
    pub sscg: Sscg,
    /// Type of the external RAM, as `RTS_DMA_SetType` programs it.
    pub ram_type: u8,
    /// Motor current step programmed by `Motor_Change` at start-up
    /// (`ChangeMotorCurrent` in `[MoveMotorSetting]`; `None` if disabled).
    pub motor_current: Option<u8>,
    /// Analog front-end voltage setting (`CRVS`, first value).
    pub crvs: u8,
    /// Top, middle and bottom reference voltages of the analog front end
    /// (`RTS_Setup_RefVoltages`), 3 bits each.
    pub ref_voltages: [u8; 3],
    /// SDRAM clock setting (`MCLKIOC` in `[SCAN_PARAM]`).
    pub mclkioc: u8,
    /// Binary threshold, low and high (`BinaryThresholdL/H` in
    /// `[PlatForm]`), written to `E99E` and `E9A0` by `Init_Registers`.
    pub binary_threshold: (u16, u16),
    /// Value of register `E951` in the initial block.
    pub e951: u8,
    /// Motor resource table copied to `E904`-`E923` at start-up
    /// (`cfg_motor_resource_get`).
    pub motor_resource: [u8; 32],
    /// Minutes of inactivity before the chip switches the lamp off
    /// (`Lamp_Status_Timer_Set`).
    pub lamp_timer_minutes: u8,
    /// Fixed duty cycle of the lamp PWM set at start-up
    /// (`cfg_fixedpwm_get`, flatbed).
    pub fixed_pwm: u8,
    /// How the motor runs when the carriage goes home
    /// (`ParkHomeMotorMove` in `[MoveMotorSetting]`).
    pub park: MotorMove,
    /// How the motor runs when the carriage goes home after the
    /// calibration (`BaseSpeedMotorMove` in `[MoveMotorSetting]`).
    pub base_speed: MotorMove,
    /// Calibration parameters (flatbed).
    pub calibration: CalibrationConfig,
    /// Sensor and motor.
    pub platform: Platform,
}

impl Profile {
    /// The tone curve the host applies to this scanner's linear samples:
    /// the HP driver's (see [`geometry::HOST_GAMMA`]), white where the gain
    /// calibration puts the white strip for grey images.
    pub fn tone(&self) -> Tone {
        Tone {
            white: geometry::HP_WHITE,
            grey_white: geometry::host_white(self),
            gamma: geometry::HOST_GAMMA,
        }
    }
}

/// Every model the driver knows.
pub const PROFILES: [&Profile; 1] = [&HP3770];

/// The profile of the scanner with this USB identity.
pub fn for_device(vendor_id: u16, product_id: u16) -> Option<&'static Profile> {
    PROFILES
        .into_iter()
        .find(|p| p.device.vendor_id == vendor_id && p.device.product_id == product_id)
}

/// HP Scanjet 3770.
pub const HP3770: Profile = Profile {
    device: DeviceModel {
        manufacturer: "HP",
        name: "Scanjet 3770",
        vendor_id: device::VENDOR_ID,
        product_id: device::PRODUCT_ID,
        // geometry::GLASS_INCHES: the largest area the HP driver accepts.
        glass: (850, 1170),
        resolutions: &geometry::RESOLUTIONS,
        buttons: &[Button::Scan, Button::Copy, Button::InstantShare],
    },
    // [SCAN_PARAM] SSCGEnable=1, SSCGMode=1, SSCGClockControl=0 (same in
    // every configuration of the file).
    // Capture 00_collegamento, 45.882 s: FE39 written as A3 13.
    sscg: Sscg {
        enable: true,
        mode: 1,
        clock: 0,
    },
    // Capture 00_collegamento, 45.952-45.963 s: EF08 = 00, 00, 08 (type 0).
    ram_type: 0,
    // [MoveMotorSetting] ChangeMotorCurrent=1; hp3900 calls
    // Motor_Change(init_regs, 3). Capture 00, 45.972 s: E954 = B0.
    motor_current: Some(3),
    // [SCAN_PARAM] CRVS=12 31 12; hp3900 reads only the first value.
    // Initial block of the HP driver (capture 04, 2.156 s): E810 = 0C.
    crvs: 12,
    // [SCAN_PARAM] VRTS=2 4 2 0, VRMS=3 0, VRBS=2 4 2 0: first values. The
    // hp3970 table of hp3900 has 0, 0, 0. The initial block of the HP driver
    // and every execution block have E814-E816 = 40 60 40: 2, 3, 2.
    ref_voltages: [2, 3, 2],
    // No MCLKIOC key in any configuration of the file: hp3900 default 8.
    // Initial block of the HP driver: E987 = 88.
    mclkioc: 8,
    // hp3900 writes 0x13. The initial block of the HP driver has 03.
    e951: 0x03,
    // [PlatForm] BinaryThresholdL=99, BinaryThresholdH=100. Initial block of
    // the HP driver: E99E-E9A1 = 63 00 64 00.
    binary_threshold: (99, 100),
    // Default table of hp3900 (all models but BQ5550). Same bytes in the
    // initial block of the HP driver at E904-E923; the chip starts with the
    // BQ5550-like table (B4 at E905, 34 at E90D, 24 at E915, A4 at E91D).
    motor_resource: [
        0xff, 0x90, 0xb0, 0xd4, 0xd0, 0x70, 0x50, 0x54, 0x30, 0x10, 0x14, 0x38, 0x18, 0x0c, 0x08,
        0x28, 0x04, 0x00, 0x20, 0x44, 0x40, 0xe0, 0xc0, 0xc4, 0xa0, 0x80, 0x84, 0xa8, 0x88, 0x9c,
        0x98, 0xb8,
    ],
    // hp3900 passes 13. Capture 00, 48.669 s: E947 written as 22, which is
    // 13 minutes (13 * 2.68 = 34.9). [PlatForm] LampOffTimer=15 would give 40.
    lamp_timer_minutes: 13,
    // Capture 00, 48.718 s: the HP driver reads E948 & 3F = 0 after enabling
    // the PWM and leaves it, so its fixed value is 0. The [PLATFORM] header
    // of the file has T6_USB2FixedPwm=0 (T_ and S_ variants: 22), and the
    // HP driver writes EEPROM 0x7B = 80, which is Lamp_PWM_Save(0).
    fixed_pwm: 0,
    // [MoveMotorSetting] ParkHomeMotorMove=1, counted from 1: [MotorMove1]
    // SystemClock=0x4, CTPC=1991 2999 3983 (first value), ScanMotorStepType=1,
    // MotorCurve=9 3 10 3 2 (first value). The same as the motormove of the
    // hp3800 table in hp3900 but for the curves. Captures 05, 06 and 11:
    // E800 low bits 4, E830 = 1991, E8D9 = 10, steps of setting 9.
    park: MotorMove {
        system_clock: 4,
        ctpc: 1991,
        step_type: StepType::Half,
        setting: Some(&motor_curves::SETTING_9),
    },
    // [MoveMotorSetting] BaseSpeedMotorMove=2: [MotorMove2] SystemClock=0x2,
    // CTPC=1991, ScanMotorStepType=1, MotorCurve=0 (no curve). Captures 04,
    // 05, 06, 09: after the white strip the HP driver parks with E800 low
    // bits 2, E830 = E8E1 = 251 (constant speed: 18 MHz / 36 / 1991) and
    // no step tables.
    base_speed: MotorMove {
        system_clock: 2,
        ctpc: 1991,
        step_type: StepType::Half,
        setting: None,
    },
    // [CalibReflective] of the configuration file (lines ~140-260 of the
    // extracted INI). Checked against captures 01, 02, 04-11.
    calibration: CalibrationConfig {
        // Not in the file: every capture probes at 300 then 350 (E81A-E825),
        // which are AdcOffPredictStart=300 and 300 + 50.
        offset_probes: (300, 350),
        // OffsetAvgTargetR/G/B=10 20.
        offset_target: 10,
        // OffsetTuneStep1=5. Captures 01, 02, 05, 07, 08, 10, 11: +5.
        offset_step: 5,
        // Not in the file: captures 06 and 06b (1200 dpi) raise odd pixels
        // by 3.
        offset_step_even_odd: 3,
        // GainHeight=10 30.
        gain_lines: 10,
        // WRefR=245 233, WRefG=245 238 230, WRefB=245 233 222.
        white_ref: [245, 245, 245],
        // GainTargetFactor=80 90.
        gain_target_percent: 80,
        // BShadingHeight=10 20 30.
        black_lines: 10,
        // U2_S_8B_24D_Cut_R/G/B=-6 (USB 2, 8 bits per sample, 24-bit
        // colour).
        black_cut_hundredths: [-6, -6, -6],
        // WShadingHeight=24.
        white_lines: 24,
        // Every capture reads the white strip at 200 dpi (E8C0 and the
        // motor step of the block).
        white_resolution: 200,
        // hp3900 starts from vgag1 = vgag2 = 4; every block of the HP
        // driver before the gain is set has E814-E819 low bits 4.
        start_gain: 4,
        gain2: 4,
        // Every block of the HP driver: E813 low bits 3F.
        pga: 3,
    },
    // [PlatForm] of the configuration (line 14684 of the extracted file);
    // multi-value keys take the first value.
    platform: Platform {
        // RealSensorResolution=1200.
        sensor_resolution: 1200,
        // SensorLineDistance=24 0 6 12 24 0 12. Scan at 300 dpi (capture
        // 04): E94A-E94D = 6, 6, 12, 12.
        line_distance: 24,
        // SensorEvenOddDistance=4 0 12 24 0 4 2.
        even_odd_distance: 4,
        // MotorResolution=1200 2400.
        motor_resolution: 1200,
        // [MoveMotorSetting] BaseSpeedPPS=800 450 800.
        base_speed_pps: 800,
        // SensorChannelColor=123 321. Every block: EE0A low bits = 06.
        channel_color: [0, 1, 2],
        // RGBChannelOrder=123 321. Every block: E812 low bits = 24.
        rgb_order: [0, 1, 2],
        // Table of the HP driver (function 0x100366d0) for MotorPhase=1;
        // the blocks at 600 and 1200 dpi have it at E904-E923.
        motor_resource_phase: [
            0xff, 0xb4, 0xb0, 0xd4, 0xd0, 0x70, 0x50, 0x54, 0x30, 0x34, 0x14, 0x38, 0x18, 0x0c,
            0x08, 0x28, 0x04, 0x24, 0x20, 0x44, 0x40, 0xe0, 0xc0, 0xc4, 0xa0, 0xa4, 0x84, 0xa8,
            0x88, 0x9c, 0x98, 0xb8,
        ],
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_3770_is_found_by_its_usb_identity() {
        let p = for_device(0x03f0, 0x2505).unwrap();
        assert_eq!(p, &HP3770);
        assert!(for_device(0x03f0, 0x2605).is_none());
        assert_eq!(p.device.glass_inches(), geometry::GLASS_INCHES);
        assert_eq!(p.device.resolutions, geometry::RESOLUTIONS);
    }

    #[test]
    fn the_tone_is_the_hp_drivers() {
        let t = HP3770.tone();
        assert_eq!(t.white, [196.0, 186.0, 184.0]);
        assert_eq!(t.grey_white, 196.0);
        assert_eq!(t.gamma, 2.2);
    }
}
