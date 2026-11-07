// SPDX-License-Identifier: GPL-2.0-or-later
//! Parameters of the HP Scanjet 3770.
//!
//! The values come from the configuration embedded in the HP driver
//! (`hpgt3770.dll`, same key names as the tables of `hp3900_config.c`) and
//! are checked against the USB captures of the HP driver. Only the numbers
//! are copied, with their source, as `hp3900` does for its models.

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

/// Parameters of one scanner model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
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
}

/// HP Scanjet 3770.
pub const HP3770: Profile = Profile {
    // [SCAN_PARAM] SSCGEnable=1, SSCGMode=1, SSCGClockControl=0.
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
    // [SCAN_PARAM] CRVS=12 6 8; hp3900 reads only the first value.
    // Initial block of the HP driver (capture 04, 2.156 s): E810 = 0C.
    crvs: 12,
    // The HP configuration has VRTS=VRMS=VRBS=0 and so does the hp3970
    // table of hp3900, but the initial block of the HP driver and every
    // execution block in the captures have E814-E816 = 40 60 40: 2, 3, 2.
    ref_voltages: [2, 3, 2],
    // No MCLKIOC key in the HP configuration: hp3900 default 8.
    // Initial block of the HP driver: E987 = 88.
    mclkioc: 8,
    // hp3900 writes 0x13. The initial block of the HP driver has 03.
    e951: 0x03,
    // Default table of hp3900 (all models but BQ5550). Same bytes in the
    // initial block of the HP driver at E904-E923; the chip starts with the
    // BQ5550-like table (B4 at E905, 34 at E90D, 24 at E915, A4 at E91D).
    motor_resource: [
        0xff, 0x90, 0xb0, 0xd4, 0xd0, 0x70, 0x50, 0x54, 0x30, 0x10, 0x14, 0x38, 0x18, 0x0c, 0x08,
        0x28, 0x04, 0x00, 0x20, 0x44, 0x40, 0xe0, 0xc0, 0xc4, 0xa0, 0x80, 0x84, 0xa8, 0x88, 0x9c,
        0x98, 0xb8,
    ],
    // hp3900 passes 13. Capture 00, 48.669 s: E947 written as 22, which is
    // 13 minutes (13 * 2.68 = 34.9). [PlatForm] LampOffTimer=15 would give 28.
    lamp_timer_minutes: 13,
    // Capture 00, 48.718 s: the HP driver reads E948 & 3F = 0 after enabling
    // the PWM and leaves it, so its fixed value is 0. The HP configuration
    // has T6_USB2FixedPwm=0 (T_ and S_ variants: 22), and the HP driver
    // writes EEPROM 0x7B = 80, which is Lamp_PWM_Save(0).
    fixed_pwm: 0,
};
