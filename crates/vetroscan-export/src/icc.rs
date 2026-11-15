// SPDX-License-Identifier: GPL-2.0-or-later
//! ICC profiles that say what a file's numbers are.
//!
//! A finished page is the scanner's linear light through a tone curve,
//! `value = light^(1/gamma)`, with the gamma of the curve it was drawn
//! with (each driver declares its own; films have theirs). The colours are
//! taken as sRGB's primaries and white. A file without a profile is read
//! as sRGB (a curve of about 2.2): right for a page drawn at 2.2, darker
//! midtones than meant for one drawn at 1.8. So every file gets a profile
//! made here from those numbers: a monitor-class ICC v2 profile, sRGB
//! primaries adapted to D50 as ICC asks, white D65, one gamma for the three
//! channels ([`rgb`]), or a grey profile with the same gamma ([`grey`]).
//! The profile describes the numbers; it never changes them.

use scan_imaging::Format;

/// D50, the illuminant of ICC's connection space.
const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

/// D65, sRGB's white (the media white of a v2 display profile).
const D65: [f64; 3] = [0.9505, 1.0, 1.0891];

/// sRGB's primaries in XYZ, adapted to D50 (Bradford): red, green, blue.
pub(crate) const SRGB_D50: [[f64; 3]; 3] = [
    [0.4361, 0.2225, 0.0139],
    [0.3851, 0.7169, 0.0971],
    [0.1431, 0.0606, 0.7141],
];

/// The date written in the profiles made here (they do not change with the
/// day they are written): 15 November 2026.
const DATE: [u16; 6] = [2026, 11, 15, 0, 0, 0];

/// An RGB profile: sRGB primaries and white, the tone curve `gamma` on the
/// three channels.
pub fn rgb(gamma: f64) -> Vec<u8> {
    matrix(
        &format!("RGB, sRGB primaries, gamma {}", shown(gamma)),
        SRGB_D50,
        gamma,
    )
}

/// An RGB profile of other primaries (red, green, blue in XYZ, adapted to
/// D50), white D65, the tone curve `gamma`: a measured scanner's, or a
/// wide-gamut monitor's in the tests.
pub fn matrix(description: &str, primaries: [[f64; 3]; 3], gamma: f64) -> Vec<u8> {
    let curve = curve(gamma);
    build(
        b"RGB ",
        &[
            (b"desc", desc(description)),
            (b"cprt", text(COPYRIGHT)),
            (b"wtpt", xyz(D65)),
            (b"rXYZ", xyz(primaries[0])),
            (b"gXYZ", xyz(primaries[1])),
            (b"bXYZ", xyz(primaries[2])),
            (b"rTRC", curve.clone()),
            (b"gTRC", curve.clone()),
            (b"bTRC", curve),
        ],
        Some(DATE),
    )
}

/// A grey profile: white D65, the tone curve `gamma`.
pub fn grey(gamma: f64) -> Vec<u8> {
    build(
        b"GRAY",
        &[
            (b"desc", desc(&format!("Grey, gamma {}", shown(gamma)))),
            (b"cprt", text(COPYRIGHT)),
            (b"wtpt", xyz(D65)),
            (b"kTRC", curve(gamma)),
        ],
        Some(DATE),
    )
}

/// The profile of an image of `format` drawn at `gamma`: RGB or grey; none
/// for black and white (two levels, nothing to describe).
pub fn of(format: Format, gamma: f64) -> Option<Vec<u8>> {
    match format {
        Format::Rgb => Some(rgb(gamma)),
        Format::Grey => Some(grey(gamma)),
        Format::Lineart => None,
    }
}

/// The profile of the linear 16-bit TIFF ("sRGB lineare": sRGB primaries,
/// white D50, gamma 1), as it has been since that file was written.
pub(crate) fn linear_srgb() -> Vec<u8> {
    let curve = b"curv\0\0\0\0\0\0\0\x01\x01\x00".to_vec();
    build(
        b"RGB ",
        &[
            (b"desc", desc("sRGB lineare")),
            (b"cprt", text("Nessun copyright")),
            (b"wtpt", xyz(D50)),
            (b"rXYZ", xyz(SRGB_D50[0])),
            (b"gXYZ", xyz(SRGB_D50[1])),
            (b"bXYZ", xyz(SRGB_D50[2])),
            (b"rTRC", curve.clone()),
            (b"gTRC", curve.clone()),
            (b"bTRC", curve),
        ],
        None,
    )
}

const COPYRIGHT: &str = "No copyright, use freely";

/// The gamma as the description says it: "1.8", "2.2", "2.25".
fn shown(gamma: f64) -> String {
    let s = format!("{gamma:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A number as ICC's s15Fixed16.
fn s15(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn xyz(v: [f64; 3]) -> Vec<u8> {
    let mut out = b"XYZ \0\0\0\0".to_vec();
    for c in v {
        out.extend_from_slice(&s15(c));
    }
    out
}

fn text(s: &str) -> Vec<u8> {
    let mut out = b"text\0\0\0\0".to_vec();
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    out
}

/// A v2 description: the ASCII text, no Unicode nor ScriptCode one.
fn desc(s: &str) -> Vec<u8> {
    let mut out = b"desc\0\0\0\0".to_vec();
    out.extend_from_slice(&(s.len() as u32 + 1).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    out.extend_from_slice(&[0; 4 + 4 + 2 + 1 + 67]);
    out
}

/// A curve of one gamma (u8Fixed8: 1.8 is 461/256, 1.80078125).
fn curve(gamma: f64) -> Vec<u8> {
    let g = (gamma * 256.0).round().clamp(1.0, 65535.0) as u16;
    let mut out = b"curv\0\0\0\0\0\0\0\x01".to_vec();
    out.extend_from_slice(&g.to_be_bytes());
    out
}

/// A monitor-class ICC v2.1 profile of colour space `space` (PCS XYZ) with
/// `elements` in the order given, each at a multiple of 4.
fn build(space: &[u8; 4], elements: &[(&[u8; 4], Vec<u8>)], date: Option<[u16; 6]>) -> Vec<u8> {
    let table_len = 4 + 12 * elements.len();
    let mut offset = 128 + table_len;
    let mut table = (elements.len() as u32).to_be_bytes().to_vec();
    let mut body = Vec::new();
    for (sig, data) in elements {
        while (offset + body.len()) % 4 != 0 {
            body.push(0);
        }
        let at = offset + body.len();
        table.extend_from_slice(*sig);
        table.extend_from_slice(&(at as u32).to_be_bytes());
        table.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
    }
    offset += body.len();
    let mut header = vec![0u8; 128];
    header[0..4].copy_from_slice(&(offset as u32).to_be_bytes());
    header[8..12].copy_from_slice(&[0x02, 0x10, 0, 0]); // version 2.1
    header[12..16].copy_from_slice(b"mntr");
    header[16..20].copy_from_slice(space);
    header[20..24].copy_from_slice(b"XYZ ");
    if let Some(date) = date {
        for (i, v) in date.iter().enumerate() {
            header[24 + 2 * i..26 + 2 * i].copy_from_slice(&v.to_be_bytes());
        }
    }
    header[36..40].copy_from_slice(b"acsp");
    // Rendering intent 0 (perceptual), illuminant D50.
    header[68..72].copy_from_slice(&s15(D50[0]));
    header[72..76].copy_from_slice(&s15(D50[1]));
    header[76..80].copy_from_slice(&s15(D50[2]));
    let mut out = header;
    out.extend_from_slice(&table);
    out.extend_from_slice(&body);
    out
}

/// What a profile says, as [`read`] finds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Described {
    /// The class (`mntr`).
    pub class: [u8; 4],
    /// The colour space (`RGB ` or `GRAY`).
    pub space: [u8; 4],
    /// The description.
    pub description: String,
    /// The media white, XYZ.
    pub white: [f64; 3],
    /// The primaries, red, green and blue, XYZ (D50); none for grey.
    pub primaries: Option<[[f64; 3]; 3]>,
    /// The gamma of every tone curve (they must all be the same, one
    /// number each).
    pub gamma: f64,
}

/// Reads a profile of one gamma, as this module writes them: `None` for
/// anything else (several gammas, curves of points, a size that does not
/// match).
pub fn read(bytes: &[u8]) -> Option<Described> {
    let be32 = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    if be32(0)? as usize != bytes.len() || bytes.get(36..40)? != b"acsp" {
        return None;
    }
    let sig = |at: usize| -> Option<[u8; 4]> { bytes.get(at..at + 4)?.try_into().ok() };
    let count = be32(128)? as usize;
    let mut tags = Vec::with_capacity(count);
    for i in 0..count {
        let at = 132 + 12 * i;
        let (start, len) = (be32(at + 4)? as usize, be32(at + 8)? as usize);
        tags.push((sig(at)?, bytes.get(start..start + len)?));
    }
    let tag = |name: &[u8; 4]| tags.iter().find(|(s, _)| s == name).map(|(_, d)| *d);
    let xyz = |name: &[u8; 4]| -> Option<[f64; 3]> {
        let d = tag(name)?;
        if d.get(0..4)? != b"XYZ " {
            return None;
        }
        let mut v = [0.0; 3];
        for (c, out) in v.iter_mut().enumerate() {
            let raw = i32::from_be_bytes(d.get(8 + 4 * c..12 + 4 * c)?.try_into().ok()?);
            *out = f64::from(raw) / 65536.0;
        }
        Some(v)
    };
    let gamma_of = |name: &[u8; 4]| -> Option<f64> {
        let d = tag(name)?;
        if d.get(0..4)? != b"curv" || be32_of(d, 8)? != 1 {
            return None;
        }
        Some(f64::from(u16::from_be_bytes(d.get(12..14)?.try_into().ok()?)) / 256.0)
    };
    let space = sig(16)?;
    let (primaries, gamma) = if &space == b"GRAY" {
        (None, gamma_of(b"kTRC")?)
    } else {
        let gammas = [gamma_of(b"rTRC")?, gamma_of(b"gTRC")?, gamma_of(b"bTRC")?];
        if gammas.iter().any(|&g| g != gammas[0]) {
            return None;
        }
        (
            Some([xyz(b"rXYZ")?, xyz(b"gXYZ")?, xyz(b"bXYZ")?]),
            gammas[0],
        )
    };
    let description = tag(b"desc").and_then(|d| {
        let n = be32_of(d, 8)? as usize;
        let text = d.get(12..12 + n)?;
        Some(String::from_utf8_lossy(text.strip_suffix(&[0]).unwrap_or(text)).into_owned())
    })?;
    Some(Described {
        class: sig(12)?,
        space,
        description,
        white: xyz(b"wtpt")?,
        primaries,
        gamma,
    })
}

fn be32_of(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn an_rgb_profile_says_its_primaries_white_and_gamma() {
        for (gamma, written) in [(1.8, 1.80078125), (2.2, 2.19921875)] {
            let bytes = rgb(gamma);
            let d = read(&bytes).unwrap();
            assert_eq!(&d.class, b"mntr");
            assert_eq!(&d.space, b"RGB ");
            assert!(near(d.white, [0.9505, 1.0, 1.0891]), "{:?}", d.white);
            let p = d.primaries.unwrap();
            assert!(near(p[0], [0.4361, 0.2225, 0.0139]));
            assert!(near(p[1], [0.3851, 0.7169, 0.0971]));
            assert!(near(p[2], [0.1431, 0.0606, 0.7141]));
            assert_eq!(d.gamma, written);
            assert_eq!(
                d.description,
                format!("RGB, sRGB primaries, gamma {}", shown(gamma))
            );
            // Version 2.1.
            assert_eq!(&bytes[8..10], &[2, 0x10]);
        }
        assert_eq!(
            read(&rgb(1.8)).unwrap().description,
            "RGB, sRGB primaries, gamma 1.8"
        );
    }

    #[test]
    fn a_grey_profile_has_one_curve() {
        let d = read(&grey(1.8)).unwrap();
        assert_eq!(&d.space, b"GRAY");
        assert_eq!(d.primaries, None);
        assert_eq!(d.gamma, 1.80078125);
        assert!(near(d.white, D65));
        assert_eq!(d.description, "Grey, gamma 1.8");
    }

    #[test]
    fn black_and_white_has_no_profile() {
        assert!(of(Format::Lineart, 2.2).is_none());
        assert_eq!(
            read(&of(Format::Grey, 2.2).unwrap()).unwrap().space,
            *b"GRAY"
        );
        assert_eq!(
            read(&of(Format::Rgb, 2.2).unwrap()).unwrap().space,
            *b"RGB "
        );
    }

    #[test]
    fn the_linear_tiffs_profile_is_the_same_as_before() {
        // The bytes written by the RAW TIFF since it was made.
        let bytes = linear_srgb();
        let d = read(&bytes).unwrap();
        assert_eq!(d.gamma, 1.0);
        assert_eq!(d.description, "sRGB lineare");
        assert!(near(d.white, D50));
        assert_eq!(bytes.len(), 498);
        assert_eq!(&bytes[24..36], &[0; 12], "no date");
    }
}
