// SPDX-License-Identifier: GPL-2.0-or-later
//! Lamp control: on/off, PWM duty cycle and the `E950` mode.
//!
//! Translated from `hp3900` and checked against the HP driver: the start-up
//! in `00_collegamento` and the lamp switched off and on again during the
//! calibration of a 300 dpi scan (`04_colore_300dpi_A4`). The transparency
//! adapter's lamp was found by hand on 8 November 2026 (see
//! [`Lamp::from_registers`]): the HP driver on Windows 11 never lights it.

use crate::error::Result;
use crate::init::Registers;
use crate::status::Lamp;
use crate::transport::{Chip, Transport};

impl<T: Transport> Chip<T> {
    /// Which lamp `E946` asks for (`Lamp_Status_Get`): reads `E946` and
    /// `E954`-`E955` as the HP driver does, decodes `E946` alone
    /// ([`Lamp::from_e946`]).
    pub fn lamp_status(&mut self) -> Result<Lamp> {
        let e946 = self.read_byte(0xe946)?;
        self.read_word(0xe954)?;
        Ok(Lamp::from_e946(e946))
    }

    /// Switches the flatbed lamp on or off (`Lamp_Status_Set` with
    /// `FLB_LAMP`): reads the whole block, sets bit 6 of `E946`, clears bit 5
    /// (the transparency adapter's lamp: one lamp at a time; it is clear in
    /// every capture of the glass) and, when switching on, clears bit 4 of
    /// `E955` as hp3900 does. Updates the lamp bits of
    /// `E946` in `init` like hp3900; unlike hp3900, `E955` is not copied to
    /// `init`: after the lamp was off the chip reads 09 (capture 09) or 08
    /// (capture 06) there, and the HP driver's next blocks still carry the
    /// initial block's 01 (white strip, park and scan).
    ///
    /// hp3900 waits 200 ms between the two writes; the HP driver does not
    /// (3 ms apart in every capture), and neither does this.
    pub fn lamp_flatbed(&mut self, init: &mut Registers, on: bool) -> Result<()> {
        let mut regs = self.read_block()?;
        regs.set_bits(0x146, 0x40, u8::from(on));
        regs.set_bits(0x146, 0x20, 0);
        if regs.get(0x146) & 0x40 != 0 {
            regs.set_bits(0x155, 0x10, 0);
        }
        init.0[0x146] = (init.get(0x146) & 0x9f) | (regs.get(0x146) & 0x60);
        self.write_byte(0xe946, regs.get(0x146))?;
        self.write_registers(0xe954, &regs.0[0x154..0x156])
    }

    /// Switches the transparency adapter's lamp on or off: bit 5 of `E946`
    /// and bit 6 of `E965` together (see [`Lamp::from_registers`]); on, the
    /// flatbed lamp goes off (bit 6 of `E946`). Reads the whole block first
    /// as for the flatbed lamp, writes `E965` then `E946`, and keeps both
    /// bits in `init`, so that the blocks a scan writes keep the lamp lit.
    /// It needs about 30 s to warm up (HP's user's manual).
    pub fn lamp_transparency(&mut self, init: &mut Registers, on: bool) -> Result<()> {
        let mut regs = self.read_block()?;
        regs.set_bits(0x165, 0x40, u8::from(on));
        regs.set_bits(0x146, 0x20, u8::from(on));
        if on {
            regs.set_bits(0x146, 0x40, 0);
        }
        init.0[0x146] = (init.get(0x146) & 0x9f) | (regs.get(0x146) & 0x60);
        init.0[0x165] = (init.get(0x165) & !0x40) | (regs.get(0x165) & 0x40);
        self.write_byte(0xe965, regs.get(0x165))?;
        self.write_byte(0xe946, regs.get(0x146))
    }

    /// Lets the chip drive the lamp with its PWM (`Lamp_PWM_use`, with
    /// `PwmLampLevel=0` as in the HP configuration): bit 6 of `E948` on,
    /// bits 6-7 of `E9E0` off; `enable = false` clears both.
    pub fn lamp_pwm_use(&mut self, init: &mut Registers, enable: bool) -> Result<()> {
        let mut a = self.read_byte(0xe948)?;
        let mut b = self.read_byte(0xe9e0)?;
        if enable {
            a |= 0x40;
            b &= 0x3f;
            init.0[0x148] |= 0x40;
            init.0[0x1e0] &= 0x3f;
        } else {
            b &= 0x7f;
            a &= 0xbf;
        }
        self.write_byte(0xe948, a)?;
        self.write_byte(0xe9e0, b)
    }

    /// Duty cycle of the lamp PWM, 0-63 (`Lamp_PWM_DutyCycle_Get`, low 6
    /// bits of `E948`).
    pub fn lamp_pwm_duty_cycle(&mut self) -> Result<u8> {
        Ok(self.read_byte(0xe948)? & 0x3f)
    }

    /// Sets the duty cycle of the lamp PWM (`Lamp_PWM_DutyCycle_Set`, with
    /// `PwmLampLevel=0`). Updates `init` like hp3900.
    pub fn set_lamp_pwm_duty_cycle(&mut self, init: &mut Registers, duty: u8) -> Result<()> {
        let mut regs = self.read_block()?;
        regs.set_bits(0x148, 0x3f, duty);
        regs.set_bits(0x148, 0x40, 0);
        regs.0[0x1e0] |= (duty >> 1) & 0x40;
        init.set_bits(0x148, 0x7f, regs.get(0x148));
        init.set_bits(0x1e0, 0x3f, regs.get(0x1e0));
        self.write_byte(0xe948, regs.get(0x148))?;
        self.write_byte(0xe9e0, regs.get(0x1e0))
    }

    /// Prepares the lamp PWM with a fixed duty cycle (`Lamp_PWM_Setup`):
    /// enables the PWM and changes the duty cycle only if it differs.
    pub fn lamp_pwm_setup(&mut self, init: &mut Registers, fixed_pwm: u8) -> Result<()> {
        self.lamp_pwm_use(init, true)?;
        if self.lamp_pwm_duty_cycle()? != fixed_pwm {
            self.set_lamp_pwm_duty_cycle(init, fixed_pwm)?;
        }
        Ok(())
    }

    /// Sets or clears bit 6 of `E950` (`Set_E950_Mode`). hp3900 clears it
    /// at the end of `RTS_Scanner_Init`; its meaning is unknown.
    pub fn set_e950_mode(&mut self, set: bool) -> Result<()> {
        let data = self.read_word(0xe950)?;
        let data = if set { data | 0x40 } else { data & 0xffbf };
        self.write_word(0xe950, data)
    }
}
