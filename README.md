# VetroScan — scanning documents, photos and films, with open drivers

**VetroScan** is a scanning application written entirely in Rust, for Linux, Windows and macOS,
with its own userspace drivers: it talks to the scanner directly over USB, without SANE, kernel
modules or the maker's software. The app talks to every scanner through one contract and shows
only what the scanner in use declares it can do, so other models can be added as drivers.
It was born as a free driver for the HP Scanjet 3770 (until v0.2 the app was called
"HP Scanjet 3770").

*In italiano: [README.it.md](README.it.md).*

## Scanners

| Scanner | Glass | Films (adapter in the lid) | Tried on |
|---|---|---|---|
| **HP Scanjet 3770** (G3110 family, USB `03f0:2505`, Realtek RTS8822) | 200–2400 dpi, colour, grey, black and white, 8 or 16 bits; scans equal to HP's driver | negatives, black and white negatives and slides at 600 and 1200 dpi | Windows, Linux |
| **Epson Perfection 4490 Photo** (GT-X750, USB `04b8:0119`) | 100–4800 dpi (not 800 and 900), colour, grey, black and white, 8 or 16 bits | negatives, black and white negatives and slides at 600–4800 dpi; **Digital ICE** (infrared dust removal) at 600, 1200 and 2400 dpi | Windows; the glass on Linux too |

On macOS the app is built and packaged, but it has not yet been tried with a scanner.

- The **HP 3770** driver is a Rust translation of the part of SANE's `hp3900` backend the 3770
  needs, completed with the parameters of HP's own driver and checked byte for byte against
  USB captures of it (start-up, calibration, scan, motor).
- The **Epson 4490** driver was written from USB captures of Epson Scan used normally: the
  protocol is documented by nothing else. The scanner needs its firmware, `esfw54.bin`, sent at
  every power-on; VetroScan sends it by itself (see [Licence](#licence)).

## What the app does

- **Document**, **Photo** and **Film** (the last with a scanner that has a film adapter), each
  with its own pages, settings and undo history.
- **Preview** with several areas, each its own page or image; the **automatic crop** finds the
  photos on the glass and the frames or slides in the film holder, editable by hand; a film
  gets one scan per frame chosen, which is what makes 2400 and 4800 dpi usable.
- Negatives **developed** as a lab does: the orange base measured on each strip (or locked for
  a roll), the white and black of each frame; colour noise reduction, the scanner's stripes
  removed, the scanner's colours corrected where the driver declares a correction.
- **No processing**, for those who process their scans elsewhere: the file as the scanner gives
  it, with every correction off.
- **Adjustments** (levels, curves, tone, colour, sharpness, descreening, black and white
  thresholds), presets, an **editor** (rotate, straighten, crop, resize), all without touching
  the scan until it is saved; pick several photos to save or adjust only those.
- **Saving** as PDF (on a sheet, with a size limit), TIFF (also 16-bit and multi-page), PNG,
  JPEG, WebP, AVIF, ZIP, and **RAW** (linear 16-bit TIFF, DNG for Lightroom). Every file
  carries an **ICC colour profile**, and the window shows pictures through the monitor's
  profile (found by itself on Windows; sRGB elsewhere for now).
- The **scanner's buttons** work with the app in the background; the lamp is switched off
  after a time chosen.
- Italian and English (Preferences → *Language*; by default the system's).

How to use it, for people who are not technical: **[docs/usage.md](docs/usage.md)**. What is
new in each version: [CHANGELOG.md](CHANGELOG.md).

## Installing

Download the file for your system from the
[latest release](https://github.com/MatteoDM/scanner/releases/latest). Every package holds the
`vetroscan` application and the `hp3770` command (the HP 3770 driver's tool); the 4490 needs
nothing else. Installing it over an older `hp3770` package or installer replaces it, and the
settings come along. The full instructions, driver included, system by system (install, check,
update, uninstall, common problems): **[docs/install.md](docs/install.md)**.

| System | File | How |
|---|---|---|
| **Windows** (x64) | `vetroscan-vX.Y.Z-windows-x86_64-setup.exe` | Run it and keep **Install for me only**: it goes to `%LOCALAPPDATA%\Programs\VetroScan` with an entry in the Start menu and in *Installed apps*, **without administrator rights**. It uses the scanner maker's driver if installed, WinUSB otherwise. |
| Windows, portable | `vetroscan-vX.Y.Z-windows-x86_64-portable.zip` | Extract anywhere and run `vetroscan.exe`; nothing is installed. |
| **macOS** (Apple Silicon / Intel) | `vetroscan-vX.Y.Z-macos-arm64.dmg` / `-macos-x86_64.dmg` | Open it and drag **VetroScan** to *Applications*. Built only by the release workflow on GitHub: a release built elsewhere has none. Not yet tried with a scanner. |
| **Arch / Omarchy** | `vetroscan-vX.Y.Z-linux-<arch>.pkg.tar.zst` | `sudo pacman -U vetroscan-vX.Y.Z-linux-x86_64.pkg.tar.zst` |
| **Debian / Ubuntu** | `vetroscan-vX.Y.Z-linux-<arch>.deb` | `sudo apt install ./vetroscan-vX.Y.Z-linux-x86_64.deb` |
| **Fedora / openSUSE** | `vetroscan-vX.Y.Z-linux-<arch>.rpm` | `sudo dnf install ./vetroscan-vX.Y.Z-linux-x86_64.rpm` |
| **Any Linux** | `vetroscan-vX.Y.Z-linux-<arch>.AppImage` | `chmod +x vetroscan-*.AppImage` and run it; nothing is installed. |

The Linux packages also install the udev rules that let the scanners be used without root
(`70-hp3770.rules`, `70-epson-4490.rules`), and apply them at once, even to a scanner already
plugged in. The AppImage cannot: at the first start the app offers the button **Allow access
to the scanner**, which installs them with the computer's password. The `aarch64` packages,
like the macOS ones, come only from the release workflow. From source, on Arch:
`yay -S vetroscan-git` (the PKGBUILD is in [`packaging/aur`](packaging/aur)).

**Warnings at the first start.** The Windows installer and the macOS app are not signed with
a paid certificate: Windows SmartScreen may say *Windows protected your PC* (click **More
info → Run anyway**); macOS says it cannot check the app: click **Done**, then *System
Settings → Privacy & Security → Open Anyway*.

## Layout

| Crate | What it holds |
| --- | --- |
| [`scanner-api`](crates/scanner-api) | The contract between the app and the scanners: `trait Scanner`, `Capabilities`, state, scan requests, buttons, the registry of the backends, made-up scanners for the tests (`fake.rs`) |
| [`scan-imaging`](crates/scan-imaging) | Image processing that sets itself from what it measures, the same for every scanner: pixel formats, grey and black and white, adjustments, crops and rotations, finding the originals and the frames, films (`film`), stripes, colour noise, colour corrections, dust from the infrared (`dust`) |
| [`hp3770-core`](crates/hp3770-core) | The HP 3770's driver: the RTS8822's USB protocol (registers, EEPROM, DMA), calibration, motor, the film adapter, the device profile and the `rts8822` backend of the contract |
| [`hp3770-usb`](crates/hp3770-usb) | USB access for the HP 3770: `nusb` (Linux, macOS, Windows with WinUSB) and `usbscan.sys` (Windows with HP's driver installed, no driver change) |
| [`hp3770-capture`](crates/hp3770-capture) | A reader of USBPcap captures and the `hp3770-decode` tool; replays HP's captures as tests |
| [`hp3770-cli`](crates/hp3770-cli) | The `hp3770` command |
| [`epson-gt-x750`](crates/epson-gt-x750) | The Epson 4490's driver: firmware upload, state, buttons, glass and film scans, the infrared pass, a guard that lets out only what the captures cover; the `epson4490` command |
| [`vetroscan-export`](crates/vetroscan-export) | Saving as PNG, JPEG, TIFF (also 16-bit and multi-page), WebP, AVIF, DNG, multi-page PDF and ZIP, with the ICC profiles |
| [`vetroscan`](crates/vetroscan) | The `vetroscan` desktop application (iced) |

The plan, the decisions and what was measured on the hardware are in
[docs/architecture-plan.md](docs/architecture-plan.md); the HP 3770's USB protocol in
[docs/usb-protocol.md](docs/usb-protocol.md). How to contribute:
[CONTRIBUTING.md](CONTRIBUTING.md).

## Building and testing

Rust 1.85 or newer is needed.

```sh
cargo build --release
cargo test --workspace --locked
```

The tests use the small captures in `crates/hp3770-capture/tests/fixtures` and
`crates/epson-gt-x750/testdata`. The full captures of the HP 3770's scans (up to ~27 MB each)
are not in the repository: to test those too, point to the folder that holds them.

```sh
HP3770_CAPTURES=/path/to/captures cargo test
```

**Tests of the app (e2e).** `cargo test -p vetroscan` tries the whole app without a window
and without a scanner: it clicks the real buttons with iced's simulator, and made-up scanners
(in memory: originals on the glass, film strips in a holder) answer like real ones. They cover
Document, Photo and Film, the areas, Redo, Ctrl+Z, the editor, saving in every format, both
languages, and scanners that can do more or less than the real ones. No test touches the
settings, the notifications, the printer or the Pictures folder.

The app can be tried by hand with a made-up scanner:

```sh
VETROSCAN_FAKE_SCANNER=1 cargo run -p vetroscan
```

**Tests on the real HP 3770.** They move the carriage, so they only run when asked, with the
scanner plugged in and no other program using it:

```sh
cargo test -p hp3770-cli --test hardware -- --ignored --test-threads 1
```

## The `hp3770` command

```sh
cargo run --release --bin hp3770 -- list                     # scanners seen by each backend
cargo run --release --bin hp3770 -- status                   # lamp, carriage, transparency adapter, buttons
cargo run --release --bin hp3770 -- probe -o probe.txt       # registers and EEPROM, read only
cargo run --release --bin hp3770 -- buttons                  # prints the buttons pressed (read from the chip's register)
cargo run --release --bin hp3770 -- buttons -m interrupt     # or from the button's USB events
cargo run --release --bin hp3770 -- init                     # initialises the chip like the HP driver
cargo run --release --bin hp3770 -- lamp off                 # switches the flatbed lamp off (or on, `on`)
cargo run --release --bin hp3770 -- park                     # brings the carriage home, if it is not there
cargo run --release --bin hp3770 -- scan sheet.png           # colour A4, 300 dpi
cargo run --release --bin hp3770 -- scan photo.png --dpi 1200 --left 80 --top 120 --width 50 --height 50
cargo run --release --bin hp3770 -- scan text.png --mode lineart   # also --mode grey
cargo run --release --bin hp3770 -- tma-scan strip.png --dpi 1200 --film negative   # the film adapter
cargo run --release --bin hp3770 -- develop strip-16bit.png positive.png --film negative # again, without the scanner
```

`scan` does everything the HP driver does: start-up, lamp warm-up if it was off, calibration
(offset, gain, black and white shading), scan, carriage home. It goes through the same
scanner contract as the app. The area is in millimetres from the top left corner of the
glass (an A4 by default). The image comes out as PNG (or PPM/PGM/PBM) with the same tone
curve as the HP driver; `--linear` saves the chip's raw data. It never writes the EEPROM.
`hp3770 help` lists the other commands (`detect`, `stripes`, `forward`).

`probe` goes through a filter that refuses every write before it reaches the scanner: it
does not switch the lamp on, does not move the carriage and does not touch the EEPROM. With
`--backend nusb` or `--backend usbscan` the backend is chosen; by default Windows tries
`usbscan.sys` first.

`init` writes to the chip's registers the same values the HP driver writes at every button
press (USB clock, RAM type, motor current, reset, buttons, lamp timer, lamp PWM). It never
writes the EEPROM and never moves the carriage.

`park` runs `init` and then, only if the carriage is not home, brings it back with the same
step table and register block the HP driver uses in captures 05, 06 and 11 (the tests
compare them byte for byte). If the carriage is already home it does not move it.

`buttons` reads the chip's button register every 50 ms by default: on Windows the imaging
service (WIA) listens to the buttons' USB events and takes some of them, while the
register answers whoever reads it.

## The `epson4490` command

Built from source only (the packages do not carry it), for working on the 4490's driver:

```sh
cargo run --release -p epson-gt-x750 --bin epson4490 -- list       # the scanners found, nothing sent
cargo run --release -p epson-gt-x750 --bin epson4490 -- firmware   # which esfw54.bin would be sent
cargo run --release -p epson-gt-x750 --bin epson4490 -- probe      # the state and the carriages' positions
```

Every write goes through the driver's guard: nothing moves the carriage outside what the
captures cover. The other commands are listed at the top of
[`epson4490.rs`](crates/epson-gt-x750/src/bin/epson4490.rs).

## Linux

A step-by-step guide for a new computer with Omarchy, also for people who never used the
terminal: [docs/install-linux.md](docs/install-linux.md).

The drivers talk to the scanners directly over USB (`nusb`), without SANE or kernel modules.
The user only needs to be allowed to open the device: the udev rules in
[`packaging/linux`](packaging/linux) grant it to whoever is logged in to the session.

```sh
sudo install -m 644 packaging/linux/70-hp3770.rules packaging/linux/70-epson-4490.rules /etc/udev/rules.d/
sudo udevadm control --reload
# then unplug and plug the scanner back in
cargo run --release --bin hp3770 -- status   # (HP 3770) must answer without sudo
```

To have the app in the applications menu:

```sh
cargo install --path crates/vetroscan --locked     # into ~/.cargo/bin, which must be in the PATH
install -Dm 644 packaging/linux/vetroscan.desktop ~/.local/share/applications/vetroscan.desktop
install -Dm 644 packaging/linux/vetroscan.svg ~/.local/share/icons/hicolor/scalable/apps/vetroscan.svg
```

The app on Linux:

- runs on Wayland and on X11; the window presents itself as `vetroscan`, the same name as
  the `.desktop` file;
- uses the desktop portal to choose where to save (`xdg-desktop-portal` with a backend, for
  instance `xdg-desktop-portal-gtk`);
- opens folders with `xdg-open` and prints photocopies with `lp` (CUPS);
- keeps its preferences in `~/.config/vetroscan`;
- the scanner buttons work with the app in the background too: no system service takes them
  as WIA does on Windows.

## Decoding a capture (HP 3770)

```sh
cargo run --release --bin hp3770-decode -- capture.pcap          # every operation
cargo run --release --bin hp3770-decode -- --writes capture.pcap # only what changes the scanner
cargo run --release --bin hp3770-decode -- --summary *.pcap      # totals per file
cargo run --release --bin hp3770-decode -- --executions capture.pcap
    # chip registers at every start (bit 7 of E800): the first in full, then the differences
cargo run --release --bin hp3770-decode -- --slice 2.48 2.623 small.pcap capture.pcap
    # cuts a time span into a smaller capture (for the tests)
```

In the tests, `hp3770_capture::replay::Replay` stands in for the scanner: it answers with
the capture's data and checks that our driver makes exactly the same requests as the HP
driver, stopping at the first difference.

Example of output:

```text
    16.275  INTR  ep83 00  (button Scan)
    17.316  READ  reg FE11 len 2 -> 03 0A
    17.331  WRITE reg F8FF = 01 10
     2.255  DMA   enable read  channel 0004 options 000000 size 512
```

The captures are made on Windows with the HP driver installed, with USBPcap, saved in the
classic pcap format (not pcapng).

## Licence

GPL-2.0-or-later, like the SANE `hp3900` backend the HP 3770's chip logic comes from. See
[LICENSE](LICENSE).

One file is not ours and not under the GPL:
[`crates/epson-gt-x750/firmware/esfw54.bin`](crates/epson-gt-x750/firmware) is Epson's firmware
for the Perfection 4490, taken unchanged from Epson's Windows driver and under Epson's licence.
VetroScan does not link it: it embeds the file at build time and sends it to the scanner as it
is, never examining it. The reasons and the alternatives (the copy Epson published for Linux
under the AVASYS Public License) are in
[the plan](docs/architecture-plan.md#the-firmware-and-its-licence).
