// SPDX-License-Identifier: GPL-2.0-or-later
//! Development tool: how steady the transparency adapter's lamp is.
//!
//! `cargo run -p hp3770-cli --example tma_lamp -- [SECONDS] [OFF_AT]`
//!
//! Starts the scanner, switches the adapter's lamp on, takes the carriage
//! to the slot without film where the adapter is calibrated, and reads
//! 10 lines there at 200 dpi every second, with fixed offsets and gains:
//! the mean of the brightest tenth of the columns of each channel. With
//! OFF_AT, the lamp is switched off and on again at that second, as the
//! calibration does for its black. Then the carriage goes home and the
//! flatbed lamp comes back on. Not part of the program.

use hp3770_core::motor::{park_registers, Direction, MotorPosition};
use hp3770_core::profile::HP3770;
use hp3770_core::setup::{ArrangeLine, GainOffset, Hardware, ScanParams};
use hp3770_core::transport::Chip;
use std::process::ExitCode;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let args: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let seconds = args.first().copied().unwrap_or(90);
    let off_at = args.get(1).copied();
    match run(seconds, off_at) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tma_lamp: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(seconds: u64, off_at: Option<u64>) -> Result<(), Box<dyn std::error::Error>> {
    let profile = &HP3770;
    let adapter = profile.transparency.as_ref().ok_or("no adapter")?;
    let mut chip = Chip::new(hp3770_usb::open()?);
    let link = chip.link(profile)?;
    let mut init = link.registers.clone();
    if !link.at_home {
        chip.head_park_home(&park_registers(&init), &profile.park)?;
    }
    chip.prepare_scan(&mut init, profile)?;
    if std::env::var_os("TMA_DARK").is_some() {
        // The dark as the calibration reads it: both lamps off at once,
        // the flatbed's having been on.
        chip.motor_move(
            &park_registers(&init),
            &profile.park,
            &MotorPosition {
                direction: Direction::Forward,
                e801_bit2: false,
                steps: adapter.strip_steps,
            },
        )?;
        chip.lamp_transparency(&mut init, false)?;
        chip.lamp_flatbed(&mut init, false)?;
        println!("dark after both lamps off (flatbed was on):");
        return dark_readings(&mut chip, &mut init, profile);
    }
    chip.lamp_transparency(&mut init, true)?;
    let start = Instant::now();
    chip.motor_move(
        &park_registers(&init),
        &profile.park,
        &MotorPosition {
            direction: Direction::Forward,
            e801_bit2: false,
            steps: adapter.strip_steps,
        },
    )?;
    let params = ScanParams {
        negative: false,
        triple_line: false,
        depth: 8,
        resolution_x: 200,
        resolution_y: 200,
        left: 43 + 693 + 87,
        top: 1,
        width: 172,
        height: 10,
    };
    let still = Hardware {
        black_shading: false,
        white_shading: false,
        gamma: false,
        gamma_size: 0,
        backward: false,
        static_head: true,
        dummy_scan: false,
        compression: false,
        arrange_line: ArrangeLine::None,
        high_resolution: false,
    };
    let go = GainOffset {
        even_offset: [315; 3],
        odd_offset: [315; 3],
        even_fine: [0; 3],
        odd_fine: [0; 3],
        gain: [10; 3],
        gain2: [4; 3],
        pga: [3; 3],
    };
    println!("  s      R      G      B   (brightest tenth of the slot, 8-bit)");
    let mut second = 0;
    let mut switched = false;
    while second <= seconds {
        let due = Duration::from_secs(second);
        let now = start.elapsed();
        if due > now {
            std::thread::sleep(due - now);
        }
        if Some(second) == off_at && !switched {
            chip.lamp_transparency(&mut init, false)?;
            std::thread::sleep(Duration::from_millis(500));
            chip.lamp_transparency(&mut init, true)?;
            switched = true;
            println!("  -- lamp off and on again");
        }
        let mut regs = init.clone();
        let data = chip.get_image(&mut regs, profile, &params, &still, &go, None)?;
        let width = data.len() / 3 / params.height as usize;
        let means: Vec<f64> = (0..3)
            .map(|c| {
                let mut cols: Vec<f64> = (0..width)
                    .map(|x| {
                        (0..params.height as usize)
                            .map(|y| f64::from(data[(y * width + x) * 3 + c]))
                            .sum::<f64>()
                            / f64::from(params.height)
                    })
                    .collect();
                cols.sort_by(|a, b| b.total_cmp(a));
                let n = (cols.len() / 10).max(1);
                cols[..n].iter().sum::<f64>() / n as f64
            })
            .collect();
        println!(
            "{:3}  {:6.1} {:6.1} {:6.1}",
            start.elapsed().as_secs(),
            means[0],
            means[1],
            means[2]
        );
        second += if second < 20 { 1 } else { 5 };
    }
    chip.head_park_home(&park_registers(&init), &profile.park)?;
    chip.lamp_transparency(&mut init, false)?;
    chip.lamp_flatbed(&mut init, true)?;
    Ok(())
}

fn dark_readings(
    chip: &mut Chip<hp3770_usb::Scanner>,
    init: &mut hp3770_core::init::Registers,
    profile: &hp3770_core::profile::Profile,
) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    for _ in 0..12 {
        let b = brightness(chip, init, profile)?;
        let s = chip.status()?;
        let e965 = chip.read_byte(0xe965)?;
        let e946 = chip.read_byte(0xe946)?;
        println!(
            "{:5.1}  {:6.1} {:6.1} {:6.1}   E946 {e946:02X} E965 {e965:02X} lamp {} lit {}",
            start.elapsed().as_secs_f32(),
            b[0],
            b[1],
            b[2],
            s.lamp,
            s.tma_lamp_lit
        );
        std::thread::sleep(Duration::from_millis(700));
    }
    chip.head_park_home(&park_registers(init), &profile.park)?;
    chip.lamp_flatbed(init, true)?;
    Ok(())
}

fn brightness(
    chip: &mut Chip<hp3770_usb::Scanner>,
    init: &hp3770_core::init::Registers,
    profile: &hp3770_core::profile::Profile,
) -> Result<Vec<f64>, Box<dyn std::error::Error>> {
    let params = ScanParams {
        negative: false,
        triple_line: false,
        depth: 8,
        resolution_x: 200,
        resolution_y: 200,
        left: 43 + 693 + 87,
        top: 1,
        width: 172,
        height: 10,
    };
    let still = Hardware {
        black_shading: false,
        white_shading: false,
        gamma: false,
        gamma_size: 0,
        backward: false,
        static_head: true,
        dummy_scan: false,
        compression: false,
        arrange_line: ArrangeLine::None,
        high_resolution: false,
    };
    let go = GainOffset {
        even_offset: [315; 3],
        odd_offset: [315; 3],
        even_fine: [0; 3],
        odd_fine: [0; 3],
        gain: [10; 3],
        gain2: [4; 3],
        pga: [3; 3],
    };
    let mut regs = init.clone();
    let data = chip.get_image(&mut regs, profile, &params, &still, &go, None)?;
    let width = data.len() / 3 / params.height as usize;
    Ok((0..3)
        .map(|c| {
            let mut cols: Vec<f64> = (0..width)
                .map(|x| {
                    (0..params.height as usize)
                        .map(|y| f64::from(data[(y * width + x) * 3 + c]))
                        .sum::<f64>()
                        / f64::from(params.height)
                })
                .collect();
            cols.sort_by(|a, b| b.total_cmp(a));
            let n = (cols.len() / 10).max(1);
            cols[..n].iter().sum::<f64>() / n as f64
        })
        .collect())
}
