# VetroScan — scanning, with an open driver for the HP Scanjet 3770

**VetroScan** is a scanning application with a userspace driver for the **HP Scanjet 3770**
scanner (USB `03f0:2505`), written entirely in Rust, for Linux, macOS and Windows. The app
talks to scanners through a common contract, so other models can follow (until v0.2 it was
called "HP Scanjet 3770", after the one scanner it knew).

*In italiano: [README.it.md](README.it.md).*

The scanner is built around a **Realtek RTS8822** ASIC, the same one the SANE `hp3900`
backend drives. The driver is a Rust translation of the part of `hp3900` the 3770 needs,
completed with the parameters extracted from the original HP driver and checked against
USB captures of the HP driver.

> Status: scans equal to the HP driver's in colour, grey and black and white, from 200 to
> 2400 dpi, also at 16 bits per channel. Start-up, calibration and scan are checked byte for
> byte against the HP driver's captures. Tried with the scanner on Windows (with the HP
> driver installed) and on Linux (Arch, Wayland). Still missing: a SANE backend and the
> transparency adapter; on macOS it is built and packaged at every version, but not yet
> tried with the scanner.

The app speaks Italian and English (Preferences → *Language*; by default the system's).

## Installing

Download the file for your system from the [latest release](https://github.com/MatteoDM/scanner/releases/latest).
Every package holds the `vetroscan` application and the `hp3770` command (the HP 3770
driver tool). Installing it over an older `hp3770` package or installer replaces it, and the
settings come along. The full
instructions, driver included, system by system (install, check, update, uninstall, common
problems): **[docs/install.md](docs/install.md)**.

| System | File | How |
|---|---|---|
| **Windows** (x64) | `vetroscan-vX.Y.Z-windows-x86_64-setup.exe` | Run it and keep **Install for me only**: it goes to `%LOCALAPPDATA%\Programs\VetroScan` with an entry in the Start menu and in *Installed apps*, **without administrator rights**. It uses the HP driver if installed, WinUSB otherwise. |
| Windows, portable | `vetroscan-vX.Y.Z-windows-x86_64-portable.zip` | Extract anywhere and run `vetroscan.exe`; nothing is installed. |
| **macOS** (Apple Silicon / Intel) | `vetroscan-vX.Y.Z-macos-arm64.dmg` / `-macos-x86_64.dmg` | Open it and drag **VetroScan** to *Applications*. Not yet tried with the scanner. |
| **Arch / Omarchy** | `vetroscan-vX.Y.Z-linux-<arch>.pkg.tar.zst` | `sudo pacman -U vetroscan-vX.Y.Z-linux-x86_64.pkg.tar.zst` |
| **Debian / Ubuntu** | `vetroscan-vX.Y.Z-linux-<arch>.deb` | `sudo apt install ./vetroscan-vX.Y.Z-linux-x86_64.deb` |
| **Fedora / openSUSE** | `vetroscan-vX.Y.Z-linux-<arch>.rpm` | `sudo dnf install ./vetroscan-vX.Y.Z-linux-x86_64.rpm` |
| **Any Linux** | `vetroscan-vX.Y.Z-linux-<arch>.AppImage` | `chmod +x vetroscan-*.AppImage` and run it; nothing is installed. |

The Linux packages (`x86_64` and `aarch64`) also install the udev rule that lets the scanner
be used without root, and apply it at once, even to a scanner already plugged in. The
AppImage cannot: at the first start the app offers the button **Allow access to the
scanner**, which installs it with the computer's password. From source, on Arch:
`yay -S vetroscan-git` (the PKGBUILD is in [`packaging/aur`](packaging/aur)).

**Warnings at the first start.** The Windows installer and the macOS app are not signed with
a paid certificate: Windows SmartScreen may say *Windows protected your PC* (click **More
info → Run anyway**); macOS says it cannot check the app: click **Done**, then *System
Settings → Privacy & Security → Open Anyway*.

## Using the app

How to use the app, for people who are not technical: Document and Photo modes, preview and
automatic crop, adjustments, the scan editor, output formats (RAW and 16 bits included),
the scanner buttons, preferences, shortcuts and common problems:
**[docs/usage.md](docs/usage.md)**.

What is new in each version is in [CHANGELOG.md](CHANGELOG.md).

## Layout

| Crate | What it holds |
| --- | --- |
| [`scan-imaging`](crates/scan-imaging) | Image processing the same for every scanner: pixel formats, grey and black and white, sharpening, crops and rotations, finding the photos on the glass |
| [`scanner-api`](crates/scanner-api) | The contract between the app and the scanners: capabilities, state, scan, buttons, registry of the backends, a made-up scanner for the tests |
| [`hp3770-core`](crates/hp3770-core) | The RTS8822's USB protocol (vendor requests, registers, EEPROM, DMA), device identity, buttons, a `Transport` trait independent of the USB library, and the `rts8822` backend of the contract |
| [`hp3770-capture`](crates/hp3770-capture) | A reader of USBPcap captures and the `hp3770-decode` tool, which turns a capture into a readable sequence of operations on the chip |
| [`hp3770-usb`](crates/hp3770-usb) | USB access: `nusb` (Linux, macOS, Windows with WinUSB) and `usbscan.sys` (Windows with the HP driver installed, no driver change) |
| [`vetroscan-export`](crates/vetroscan-export) | Saving as PNG, JPEG, TIFF (also 16-bit and multi-page), WebP, AVIF, DNG, multi-page PDF and ZIP |
| [`hp3770-cli`](crates/hp3770-cli) | The `hp3770` command |
| [`vetroscan`](crates/vetroscan) | The `vetroscan` desktop application (iced): documents and photos, preview, adjustments, scanner buttons, Italian and English |

The plan for other scanners is in [docs/architecture-plan.md](docs/architecture-plan.md).
How to contribute: [CONTRIBUTING.md](CONTRIBUTING.md).

## Building and testing

Rust 1.85 or newer is needed.

```sh
cargo build --release
cargo test
```

The tests use the small captures in `crates/hp3770-capture/tests/fixtures`. The full
captures of the scans (up to ~27 MB each) are not in the repository: to test those too,
point to the folder that holds them.

```sh
HP3770_CAPTURES=/path/to/captures cargo test
```

**Tests of the app (e2e).** `cargo test -p vetroscan` tries the whole app without a window
and without a scanner: it clicks the real buttons with iced's simulator, and made-up
scanners (in memory, with three originals on the glass) answer like the real one. They
cover the questions before losing scans, Document and Photo, Redo, Ctrl+Z, the editor,
saving in every format, both languages, and scanners that can do more or less than the 3770.
No test touches the settings, the notifications, the printer or the Pictures folder.

The app can be tried by hand with the same made-up scanner:

```sh
VETROSCAN_FAKE_SCANNER=1 cargo run -p vetroscan
```

**Tests on the real scanner.** They move the carriage, so they only run when asked, with the
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
```

`scan` does everything the HP driver does: start-up, lamp warm-up if it was off, calibration
(offset, gain, black and white shading), scan, carriage home. It goes through the same
scanner contract as the app. The area is in millimetres from the top left corner of the
glass (an A4 by default). The image comes out as PNG (or PPM/PGM/PBM) with the same tone
curve as the HP driver; `--linear` saves the chip's raw data. It never writes the EEPROM.

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

## Linux

A step-by-step guide for a new computer with Omarchy, also for people who never used the
terminal: [docs/install-linux.md](docs/install-linux.md).

The driver talks to the scanner directly over USB (`nusb`), without SANE or kernel modules.
The user only needs to be allowed to open the device: the udev rule in
[`packaging/linux`](packaging/linux) grants it to whoever is logged in to the session.

```sh
sudo install -m 644 packaging/linux/70-hp3770.rules /etc/udev/rules.d/
sudo udevadm control --reload
# then unplug and plug the scanner back in
cargo run --release --bin hp3770 -- status   # must answer without sudo
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
- the scanner buttons work with the app in the background too: it reads them from the chip,
  no system service takes them as WIA does on Windows.

## Decoding a capture

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
classic pcap format (not pcapng). The protocol is described in
[docs/usb-protocol.md](docs/usb-protocol.md).

## Licence

GPL-2.0-or-later, like the SANE `hp3900` backend the chip logic comes from. See
[LICENSE](LICENSE).
