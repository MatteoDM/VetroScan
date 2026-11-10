// SPDX-License-Identifier: GPL-2.0-or-later
// The copies of `esfw54.bin` the backend accepts, by SHA-256. Shared by
// `build.rs` (which `include!`s this file) and the crate, so that a build
// and a run check the same list.
//
// Both are 65,793 bytes: a 256-byte header ("EDLA" and zeros) and 65,537
// bytes for the scanner. They are different builds of the firmware (about
// 49,800 bytes differ); which one is newer is not known, and neither has
// been sent to a scanner by this program yet.

/// The copy in Epson's Windows driver 3.2.4.1 (`es54.inf`,
/// `C:\Windows\System32\esfw54.bin`), under the EPSON End User Software
/// License Agreement: in the private repository and in the builds kept by
/// Matteo, never in public packages.
pub const WINDOWS_DRIVER_SHA256: &str =
    "B01384A9EC89F62183054A2826789498C7666C646BE7E5CFDE16CEDF012EF0F9";

/// The copy in Epson's Linux plugin `iscan-plugin-gt-x750-2.1.2-1` (and in
/// Fedora's `iscan-firmware`), under the AVASYS Public License.
pub const AVASYS_SHA256: &str = "9A4171ABD4869990C85299ACDDF7BE2DDB96C744CF9E44CB4521DCBA26AE7D5A";

/// The size of the file.
pub const FILE_LEN: usize = 65_793;
