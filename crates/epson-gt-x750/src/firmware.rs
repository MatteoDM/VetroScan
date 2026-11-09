// SPDX-License-Identifier: GPL-2.0-or-later
//! The scanner's firmware, `esfw54.bin`: sent at every power-on, before
//! anything else.
//!
//! It is never in the repository. A build may embed it (`build.rs`, from
//! the file named by `VETROSCAN_ESFW54`); otherwise, or when the user names
//! another copy, it is read at run time from the places Epson's drivers
//! install it ([`search_paths`]). Either way only the two known copies are
//! accepted, by SHA-256 (`known_firmware.rs`), and the bytes are sent as
//! they are, but for the checksum (see [`crate::protocol::upload_firmware`]).

use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

mod known {
    include!("known_firmware.rs");
}

pub use known::{AVASYS_SHA256, FILE_LEN, WINDOWS_DRIVER_SHA256};

/// The file's name, as Epson's drivers install it.
pub const FILE_NAME: &str = "esfw54.bin";
/// The header before what the scanner gets: [`MAGIC`] and zeros.
pub const HEADER_LEN: usize = 256;
/// The first bytes of the header.
pub const MAGIC: &[u8; 4] = b"EDLA";
/// What the scanner gets: the file after its header (`0x10001` bytes, the
/// last one replaced by the checksum).
pub const PAYLOAD_LEN: usize = FILE_LEN - HEADER_LEN;

#[cfg(esfw54_embedded)]
const EMBEDDED: Option<&[u8]> = Some(include_bytes!(concat!(env!("OUT_DIR"), "/esfw54.bin")));
#[cfg(not(esfw54_embedded))]
const EMBEDDED: Option<&[u8]> = None;

/// Whether this build carries the firmware inside.
pub fn embedded() -> bool {
    EMBEDDED.is_some()
}

/// Which of the known copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    /// From Epson's Windows driver (EPSON EULA: never distributed by us).
    WindowsDriver,
    /// From Epson's Linux plugin (AVASYS Public License).
    Avasys,
}

impl fmt::Display for Build {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WindowsDriver => "Windows driver copy",
            Self::Avasys => "AVASYS copy",
        })
    }
}

/// Where the firmware came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Inside the program (`VETROSCAN_ESFW54` at build time).
    Embedded,
    /// Read from this file.
    File(PathBuf),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Embedded => f.write_str("embedded"),
            Self::File(path) => write!(f, "{}", path.display()),
        }
    }
}

/// A known copy of `esfw54.bin`, checked.
#[derive(Clone, PartialEq, Eq)]
pub struct Firmware {
    image: Cow<'static, [u8]>,
    build: Build,
    origin: Origin,
}

impl fmt::Debug for Firmware {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Firmware")
            .field("build", &self.build)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl Firmware {
    /// Checks that `image` is one of the known copies.
    pub fn new(image: Cow<'static, [u8]>, origin: Origin) -> Result<Self, String> {
        let sha = sha256_hex(&image);
        let build = if sha == WINDOWS_DRIVER_SHA256 {
            Build::WindowsDriver
        } else if sha == AVASYS_SHA256 {
            Build::Avasys
        } else {
            return Err(format!(
                "{origin} is not a known {FILE_NAME} ({} bytes, SHA-256 {sha})",
                image.len()
            ));
        };
        check_image(&image).map_err(|e| format!("{origin}: {e}"))?;
        Ok(Self {
            image,
            build,
            origin,
        })
    }

    /// The copy inside the program, if this build has one.
    pub fn embedded() -> Option<Result<Self, String>> {
        EMBEDDED.map(|bytes| Self::new(Cow::Borrowed(bytes), Origin::Embedded))
    }

    /// Reads and checks a file.
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::new(Cow::Owned(bytes), Origin::File(path.to_path_buf()))
    }

    /// A made-up image taken as it is, for tests of the upload (the real
    /// files are not in the repository).
    #[cfg(test)]
    pub(crate) fn unchecked(image: Vec<u8>) -> Self {
        Self {
            image: Cow::Owned(image),
            build: Build::Avasys,
            origin: Origin::File(PathBuf::from("made-up")),
        }
    }

    /// The whole file, header included.
    pub fn image(&self) -> &[u8] {
        &self.image
    }

    /// Which copy it is.
    pub fn build(&self) -> Build {
        self.build
    }

    /// Where it came from.
    pub fn origin(&self) -> &Origin {
        &self.origin
    }
}

/// Checks the container: size and header (not which copy it is).
pub fn check_image(image: &[u8]) -> Result<(), String> {
    if image.len() != FILE_LEN {
        return Err(format!(
            "{FILE_NAME} has {} bytes, not {FILE_LEN}",
            image.len()
        ));
    }
    if !image.starts_with(MAGIC) {
        return Err(format!("{FILE_NAME} does not start with \"EDLA\""));
    }
    Ok(())
}

/// The SHA-256 of `bytes`, upper-case hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

/// Where Epson's drivers install the file on this system, in the order
/// they are tried after a path the user named and the embedded copy:
/// on Windows, where Epson's driver puts it.
#[cfg(windows)]
pub fn search_paths() -> Vec<PathBuf> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    vec![PathBuf::from(root).join("System32").join(FILE_NAME)]
}

/// On Linux: Fedora's `iscan-firmware`, then Epson's own `iscan` plugin.
#[cfg(target_os = "linux")]
pub fn search_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/lib/firmware/epson").join(FILE_NAME),
        PathBuf::from("/usr/share/iscan").join(FILE_NAME),
    ]
}

/// Elsewhere (macOS): no place is known where a driver installs it; the
/// firmware comes from the program (embedded) or from a path the user
/// named (the application also looks in its own settings folder).
#[cfg(not(any(windows, target_os = "linux")))]
pub fn search_paths() -> Vec<PathBuf> {
    Vec::new()
}

/// The firmware to send: from `preferred` (a path the user chose), else
/// the embedded copy, else the first known copy in [`search_paths`]. The
/// error says what was tried and why each one was refused.
pub fn find(preferred: Option<&Path>) -> Result<Firmware, String> {
    find_in(preferred, Firmware::embedded(), &search_paths())
}

fn find_in(
    preferred: Option<&Path>,
    embedded: Option<Result<Firmware, String>>,
    paths: &[PathBuf],
) -> Result<Firmware, String> {
    let mut tried = Vec::new();
    if let Some(path) = preferred {
        match Firmware::read(path) {
            Ok(fw) => return Ok(fw),
            Err(e) => tried.push(e),
        }
    }
    match embedded {
        Some(Ok(fw)) => return Ok(fw),
        Some(Err(e)) => tried.push(e),
        None => {}
    }
    for path in paths {
        if !path.exists() {
            tried.push(format!("{}: not there", path.display()));
            continue;
        }
        match Firmware::read(path) {
            Ok(fw) => return Ok(fw),
            Err(e) => tried.push(e),
        }
    }
    Err(format!(
        "the scanner needs its firmware ({FILE_NAME}, from Epson's driver) and none was found: {}",
        tried.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("epson-gt-x750-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn the_known_hashes_are_sha256_in_upper_case() {
        for h in [WINDOWS_DRIVER_SHA256, AVASYS_SHA256] {
            assert_eq!(h.len(), 64);
            assert!(h
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()));
        }
        assert_eq!(PAYLOAD_LEN, 0x10001);
    }

    #[test]
    fn an_unknown_file_is_refused_even_with_the_right_shape() {
        let mut image = vec![0u8; FILE_LEN];
        image[..4].copy_from_slice(MAGIC);
        let e = Firmware::new(Cow::Owned(image), Origin::Embedded).unwrap_err();
        assert!(e.contains("not a known esfw54.bin"), "{e}");
    }

    #[test]
    fn the_container_is_checked() {
        assert!(check_image(&[0; 10]).unwrap_err().contains("10 bytes"));
        assert!(check_image(&vec![0; FILE_LEN])
            .unwrap_err()
            .contains("EDLA"));
        let mut image = vec![0u8; FILE_LEN];
        image[..4].copy_from_slice(MAGIC);
        assert_eq!(check_image(&image), Ok(()));
    }

    #[test]
    fn find_says_what_it_tried() {
        let bad = scratch("not-firmware.bin");
        std::fs::write(&bad, b"hello").unwrap();
        let missing = scratch("missing.bin");
        let e = find_in(Some(&bad), None, std::slice::from_ref(&missing)).unwrap_err();
        assert!(e.contains("not-firmware.bin"), "{e}");
        assert!(e.contains("missing.bin: not there"), "{e}");
        // A bad embedded copy (impossible after build.rs, but checked) is
        // named too.
        let e = find_in(None, Some(Err("embedded: broken".into())), &[]).unwrap_err();
        assert!(e.contains("embedded: broken"), "{e}");
    }

    /// Only in builds made with `VETROSCAN_ESFW54`.
    #[test]
    fn the_embedded_copy_is_a_known_one() {
        assert_eq!(embedded(), Firmware::embedded().is_some());
        if let Some(fw) = Firmware::embedded() {
            assert_eq!(fw.unwrap().origin(), &Origin::Embedded);
        }
    }

    /// Only where Epson's Windows driver is installed (Matteo's PC): the
    /// facts of `captures/epson4490/01-firmware-upload.md` about that copy.
    #[test]
    fn the_windows_driver_copy_when_it_is_installed() {
        let Some(path) = search_paths().into_iter().find(|p| p.exists()) else {
            return;
        };
        let fw = Firmware::read(&path).unwrap();
        let payload = &fw.image()[HEADER_LEN..];
        let sum = payload[..PAYLOAD_LEN - 1]
            .iter()
            .fold(0u8, |s, &b| s.wrapping_add(b));
        match fw.build() {
            Build::WindowsDriver => assert_eq!((sum, payload[PAYLOAD_LEN - 1]), (0xD9, 0xF8)),
            Build::Avasys => assert_eq!((sum, payload[PAYLOAD_LEN - 1]), (0x1E, 0x3D)),
        }
    }
}
