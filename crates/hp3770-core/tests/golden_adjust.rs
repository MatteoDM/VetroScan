// SPDX-License-Identifier: GPL-2.0-or-later
//! Golden test of the host adjustments: the same scan, through every way of
//! rendering it, must give the very same bytes as when the expected values
//! were written down (v0.1.2, before the imaging moved out of the driver).
//!
//! A mismatch prints the whole table; `GOLDEN_PRINT=1 cargo test -p
//! hp3770-core --test golden_adjust -- --nocapture` prints it without failing,
//! to write down new values only when a change of the result is wanted.

use hp3770_core::profile::HP3770;
use scan_imaging::adjust::{self, Adjustments, Curve, Samples};
use scan_imaging::Mode;

const WIDTH: usize = 97;
const HEIGHT: usize = 61;

/// The scan: linear RGB samples like the chip's, 16 bits, with gradients
/// over the whole range (most of it in the shadows, as on paper), a paper
/// white near the HP white levels, and noise.
fn scan16() -> Vec<u16> {
    let mut state: u32 = 0x3770_2026;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let mut out = Vec::with_capacity(WIDTH * HEIGHT * 3);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            for c in 0..3 {
                let v = if y < HEIGHT / 4 {
                    // Paper, a little above and below the white levels.
                    (180 + (x + 7 * c) % 30) as f64 * 256.0
                } else {
                    // A ramp across, another down, a different one per channel.
                    let t = (x as f64 / (WIDTH - 1) as f64 + y as f64 / (HEIGHT - 1) as f64) / 2.0;
                    65535.0 * t.powf(1.0 + c as f64 * 0.4)
                };
                let n = f64::from(noise() % 2049) - 1024.0;
                out.push((v + n).clamp(0.0, 65535.0) as u16);
            }
        }
    }
    out
}

/// Every adjustment away from its default.
fn busy() -> Adjustments {
    Adjustments {
        temperature: 23.0,
        tint: -17.0,
        exposure: 0.4,
        highlights: -35.0,
        shadows: 42.0,
        whites: 12.0,
        blacks: -9.0,
        vibrance: 30.0,
        curve: [
            Curve::MEDIUM_CONTRAST,
            Curve::new(&[[0.0, 0.05], [0.5, 0.55], [1.0, 1.0]]),
            Curve::IDENTITY,
            Curve::STRONG_CONTRAST,
        ],
        noise: 30.0,
        descreen: 40.0,
        adaptive: false,
        black: 8.0,
        white: 240.0,
        gamma: 1.3,
        brightness: 6.0,
        contrast: 15.0,
        saturation: 20.0,
        sharpen: 2.0,
    }
}

/// FNV-1a, 64 bits.
fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// (case, bytes, hash) of every rendering of the scan.
fn renderings() -> Vec<(String, usize, u64)> {
    let tone = HP3770.tone();
    let rgb16 = scan16();
    let rgb8: Vec<u8> = rgb16.iter().map(|&v| (v >> 8) as u8).collect();
    let adjustments = [
        ("default", Adjustments::default()),
        ("busy", busy()),
        (
            "adaptive",
            Adjustments {
                adaptive: true,
                ..busy()
            },
        ),
    ];
    let modes = [
        ("colour", Mode::Colour),
        ("grey", Mode::Grey),
        ("lineart", Mode::Lineart { threshold: 55 }),
    ];
    let mut out = Vec::new();
    for (a_name, a) in &adjustments {
        for (m_name, mode) in modes {
            // The adaptive threshold only changes black and white.
            if a.adaptive && !matches!(mode, Mode::Lineart { .. }) {
                continue;
            }
            for linear in [false, true] {
                let case = |f: &str| format!("{f}/{a_name}/{m_name}/linear={linear}");
                let eight = adjust::render(&rgb8, WIDTH, HEIGHT, 300, mode, a, linear, &tone);
                out.push((case("render"), eight.data.len(), fnv(eight.data)));
                let sixteen = adjust::render_samples(
                    Samples::Sixteen(&rgb16),
                    WIDTH,
                    HEIGHT,
                    300,
                    mode,
                    a,
                    linear,
                    &tone,
                );
                out.push((case("samples16"), sixteen.data.len(), fnv(sixteen.data)));
                if let Some(deep) =
                    adjust::render16(&rgb16, WIDTH, HEIGHT, 300, mode, a, linear, &tone)
                {
                    let bytes = deep.iter().flat_map(|v| v.to_le_bytes());
                    out.push((case("render16"), deep.len() * 2, fnv(bytes)));
                }
            }
        }
    }
    out
}

const EXPECTED: &[(&str, usize, u64)] = &[
    (
        "render/default/colour/linear=false",
        17751,
        0x7673a6804f6a270b,
    ),
    (
        "samples16/default/colour/linear=false",
        17751,
        0x6b8ea95a6da67995,
    ),
    (
        "render16/default/colour/linear=false",
        35502,
        0x90248652bfc8d5f7,
    ),
    (
        "render/default/colour/linear=true",
        17751,
        0x902d0fefbda88531,
    ),
    (
        "samples16/default/colour/linear=true",
        17751,
        0x7f186028703326b2,
    ),
    (
        "render16/default/colour/linear=true",
        35502,
        0xda9103a100cb2bf4,
    ),
    ("render/default/grey/linear=false", 5917, 0xa2ac0e033bd2d46c),
    (
        "samples16/default/grey/linear=false",
        5917,
        0xda87f222f7e09aa1,
    ),
    (
        "render16/default/grey/linear=false",
        11834,
        0x4f9c1728817de896,
    ),
    ("render/default/grey/linear=true", 5917, 0x6c5ae35b922b8247),
    (
        "samples16/default/grey/linear=true",
        5917,
        0xf6f251b9d11479fc,
    ),
    (
        "render16/default/grey/linear=true",
        11834,
        0x4f49b325f05fb8e4,
    ),
    (
        "render/default/lineart/linear=false",
        793,
        0x4e7c667b8f6f5a57,
    ),
    (
        "samples16/default/lineart/linear=false",
        793,
        0x7ec43dae5d407206,
    ),
    (
        "render/default/lineart/linear=true",
        793,
        0xe64fb266e1bc73e2,
    ),
    (
        "samples16/default/lineart/linear=true",
        793,
        0x61c1d179e6eca81d,
    ),
    ("render/busy/colour/linear=false", 17751, 0x6fdac9326f8c92a3),
    (
        "samples16/busy/colour/linear=false",
        17751,
        0x7fac02c6d143ba87,
    ),
    (
        "render16/busy/colour/linear=false",
        35502,
        0x102b5a5df9c09378,
    ),
    ("render/busy/colour/linear=true", 17751, 0xb01384dfcf54d5cd),
    (
        "samples16/busy/colour/linear=true",
        17751,
        0x592caac55d8214ff,
    ),
    (
        "render16/busy/colour/linear=true",
        35502,
        0x2d2aeb3a7658e245,
    ),
    ("render/busy/grey/linear=false", 5917, 0xb8ad0b631327e970),
    ("samples16/busy/grey/linear=false", 5917, 0xe3cd6bf05cfe33c2),
    ("render16/busy/grey/linear=false", 11834, 0xb74fe7d467e018d6),
    ("render/busy/grey/linear=true", 5917, 0xdb9e290bc422460c),
    ("samples16/busy/grey/linear=true", 5917, 0xdbb52f7b83934d50),
    ("render16/busy/grey/linear=true", 11834, 0xc2628da920b251e7),
    ("render/busy/lineart/linear=false", 793, 0x67b24606d1b519b2),
    (
        "samples16/busy/lineart/linear=false",
        793,
        0x832dcffa59f824f2,
    ),
    ("render/busy/lineart/linear=true", 793, 0xbe9a471652dbc396),
    (
        "samples16/busy/lineart/linear=true",
        793,
        0xad7baedc5291dcb1,
    ),
    (
        "render/adaptive/lineart/linear=false",
        793,
        0x9ed63a587aa2a1e0,
    ),
    (
        "samples16/adaptive/lineart/linear=false",
        793,
        0xcd7c5b63ea3cee61,
    ),
    (
        "render/adaptive/lineart/linear=true",
        793,
        0xca9220f4a482eaa5,
    ),
    (
        "samples16/adaptive/lineart/linear=true",
        793,
        0xb53931e512088687,
    ),
];

/// Where macOS differs: its maths library rounds a sample of the 16-bit
/// output differently once in a while (`powf` of `f32`, here with gamma
/// 1.3), the same before and after any change of this crate. Each system
/// keeps its own values, so the test stays byte for byte everywhere.
const MACOS: &[(&str, u64)] = &[("render16/busy/colour/linear=true", 0xf644_90a7_4373_5be9)];

/// The values expected on this system.
fn expected() -> Vec<(&'static str, usize, u64)> {
    EXPECTED
        .iter()
        .map(|&(case, len, hash)| {
            let own = MACOS
                .iter()
                .find(|(c, _)| cfg!(target_os = "macos") && *c == case);
            (case, len, own.map_or(hash, |&(_, h)| h))
        })
        .collect()
}

#[test]
fn adjustments_give_the_same_bytes() {
    let got = renderings();
    let table: String = got
        .iter()
        .map(|(case, len, hash)| format!("    (\"{case}\", {len}, 0x{hash:016x}),\n"))
        .collect();
    if std::env::var_os("GOLDEN_PRINT").is_some() {
        println!("{table}");
        return;
    }
    let got: Vec<(&str, usize, u64)> = got.iter().map(|(c, l, h)| (c.as_str(), *l, *h)).collect();
    assert_eq!(got, expected(), "now:\n{table}");
}
