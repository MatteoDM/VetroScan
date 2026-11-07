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
};
