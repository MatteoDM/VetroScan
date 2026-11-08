# Installing the driver and the app, system by system

The **HP Scanjet 3770** (USB `03f0:2505`) works with an app, **VetroScan** (`vetroscan`),
and a command (`hp3770`) that talk to the scanner directly over USB. **No SANE, no kernel
modules, no HP software are needed**: the "driver" is inside the app. What changes from one
system to another is only **how the system lets a normal program open the scanner**.

| System | What the scanner needs | Package to download |
|---|---|---|
| Arch, Omarchy, Manjaro | udev rule (in the package) | `.pkg.tar.zst` |
| Debian, Ubuntu, Mint | udev rule (in the package) | `.deb` |
| Fedora, openSUSE | udev rule (in the package) | `.rpm` |
| Any Linux | udev rule (the app installs it with a button) | `.AppImage` |
| Windows 10 and 11 (x64) | the HP driver already installed, **or** WinUSB | `-setup.exe` or `-portable.zip` |
| macOS 11 or newer | nothing | `.dmg` |

All the files are on the [Releases](https://github.com/MatteoDM/scanner/releases/latest)
page, with names like `vetroscan-v0.3.0-linux-x86_64.deb`. `SHA256SUMS.txt` has the checksums
to check them.

How to use the app, once installed: **[usage guide](usage.md)**.

**First of all, on every system:** plug the scanner into the mains and, with the USB cable,
directly into the computer (a hub is fine only if it has its own power supply). Only one
program at a time can use the scanner.

- [Linux](#linux)
  - [Arch / Omarchy](#arch--omarchy)
  - [Debian / Ubuntu](#debian--ubuntu)
  - [Fedora / openSUSE](#fedora--opensuse)
  - [AppImage (any Linux)](#appimage-any-linux)
  - [From source](#from-source)
- [Windows](#windows)
- [macOS](#macos)
- [Where the settings are](#where-the-settings-are)

---

## Linux

On Linux a USB device just plugged in belongs to root. The **udev rule** `70-hp3770.rules`
gives it to the user sitting at the computer (the one of the graphical session, not one
connected over ssh):

```text
SUBSYSTEM=="usb", ATTR{idVendor}=="03f0", ATTR{idProduct}=="2505", MODE="0660", TAG+="uaccess"
```

The `.pkg.tar.zst`, `.deb` and `.rpm` packages put it in `/usr/lib/udev/rules.d/` and apply it
at once, even to the scanner already plugged in: **there is nothing else to do**.

Before downloading, check the computer's architecture:

```bash
uname -m
```

`x86_64` → file `…-linux-x86_64…`; `aarch64` → file `…-linux-aarch64…`.

What the packages install:

| File | What it is |
|---|---|
| `/usr/bin/vetroscan` | the app |
| `/usr/bin/hp3770` | the command |
| `/usr/lib/udev/rules.d/70-hp3770.rules` | the permission to use the scanner |
| `/usr/share/applications/vetroscan.desktop` | the entry in the applications menu |
| `/usr/share/icons/hicolor/…/vetroscan.*` | the icon |
| `/usr/share/licenses/vetroscan/` | licence and the libraries' licences |

### Arch / Omarchy

1. Download `vetroscan-vX.Y.Z-linux-x86_64.pkg.tar.zst` to the Downloads folder.
2. Open a terminal (in Omarchy **Super + Enter**) and install:
   ```bash
   cd ~/Downloads
   ```
   ```bash
   sudo pacman -U vetroscan-v*-linux-x86_64.pkg.tar.zst
   ```
   Type the computer's password (nothing shows while you type it) and confirm with
   **Enter**.
3. Check it (see [Checking that it works](#checking-that-it-works-linux)).
4. Open the app from the application launcher (in Omarchy **Super + Alt + Space**, type
   "VetroScan").

If you had installed from source with `cargo install` or the AUR package `vetroscan-git`,
remove them first: `cargo uninstall vetroscan hp3770-cli` or `sudo pacman -R vetroscan-git`
(up to v0.2 they were called `hp3770-app` and `hp3770-git`). A package of v0.2 or older
(`hp3770`) is replaced by the new one by itself.

**Updating:** download the new package and repeat step 2.
**Uninstalling:** `sudo pacman -R vetroscan`

### Debian / Ubuntu

1. Download `vetroscan-vX.Y.Z-linux-x86_64.deb`.
2. In the terminal:
   ```bash
   cd ~/Downloads
   ```
   ```bash
   sudo apt install ./vetroscan-v*-linux-x86_64.deb
   ```
   The `./` before the name is needed: it tells `apt` that it is a file, not a package to
   look for on the network. `apt` downloads by itself what is missing (graphics libraries,
   the portal for "Save", CUPS to print).
3. Check it and open the app from the applications menu.

**Updating:** repeat step 2 with the new file.
**Uninstalling:** `sudo apt remove vetroscan`

### Fedora / openSUSE

1. Download `vetroscan-vX.Y.Z-linux-x86_64.rpm`.
2. In the terminal, from the folder you downloaded it to:
   - Fedora:
     ```bash
     sudo dnf install ./vetroscan-v*-linux-x86_64.rpm
     ```
   - openSUSE:
     ```bash
     sudo zypper install ./vetroscan-v*-linux-x86_64.rpm
     ```
     `zypper` warns that the package is not signed: answer **i** (ignore) to install it.
3. Check it and open the app from the applications menu.

**Updating:** repeat step 2 with the new file.
**Uninstalling:** `sudo dnf remove vetroscan` or `sudo zypper remove vetroscan`

### AppImage (any Linux)

A single file that runs without installing anything. It cannot install the udev rule by
itself: the app does it, once, with a button.

1. Download `vetroscan-vX.Y.Z-linux-x86_64.AppImage`, for instance to `~/Applications`.
2. Make it executable and run it:
   ```bash
   chmod +x ~/Applications/vetroscan-v*-linux-x86_64.AppImage
   ```
   ```bash
   ~/Applications/vetroscan-v*-linux-x86_64.AppImage
   ```
   From the file manager: right click → *Properties* → *Permissions* → *Allow executing*,
   then double-click.
3. With the scanner plugged in, the **Scan** tab says "The scanner is plugged in, but Linux
   does not let this user use it" and shows **Allow access to the scanner**. Press it and
   type the password: the app installs the rule in `/etc/udev/rules.d/70-hp3770.rules`,
   applies it and connects to the scanner again, without unplugging the cable. It is only
   needed the first time.

The `hp3770` command is inside the AppImage: to use it,
`~/Applications/vetroscan-v*-linux-x86_64.AppImage --appimage-extract` makes a
`squashfs-root` folder with `squashfs-root/usr/bin/hp3770`.

If the AppImage does not start and talks about **FUSE**, run it like this:
`./vetroscan-v*-linux-x86_64.AppImage --appimage-extract-and-run`.

**Updating:** download the new file and delete the old one.
**Uninstalling:** delete the file, then the rule: `sudo rm /etc/udev/rules.d/70-hp3770.rules`

### From source

To build the app on your own computer, step by step on Omarchy:
[install-linux.md](install-linux.md). On Arch there is also the AUR package that builds the
latest version on GitHub: `yay -S vetroscan-git` ([PKGBUILD](../packaging/aur/PKGBUILD)).

### Checking that it works (Linux)

With the scanner switched on and plugged in:

```bash
lsusb | grep 03f0:2505
```

A line with HP must appear. Then, **without sudo**:

```bash
hp3770 status
```

It must answer with the state of lamp, carriage and buttons. If it says the permission is
missing:

- with a package: unplug and plug the USB cable back in; if that is not enough,
  `sudo udevadm control --reload && sudo udevadm trigger` and plug it back in;
- if you are connected over ssh, the rule does not apply (it only gives the scanner to
  whoever is at the keyboard): try from a terminal of the graphical session.

### Common problems (Linux)

- **"Save…" does not open the window to choose the file.** The desktop portal is missing.
  Arch/Omarchy: `sudo pacman -S --needed xdg-desktop-portal-gtk`; Debian/Ubuntu:
  `sudo apt install xdg-desktop-portal-gtk`; Fedora: `sudo dnf install xdg-desktop-portal-gtk`.
  Then log out of the session and back in.
- **The photocopy (Copy button) does not print.** Printing goes through CUPS (`lp`): set up
  a printer and make it the default. Try `lp` followed by the name of an image.
- **The app does not open and talks about Vulkan or OpenGL.** The graphics drivers are
  missing: Debian/Ubuntu `sudo apt install mesa-vulkan-drivers`, Fedora
  `sudo dnf install mesa-vulkan-drivers`, Arch `sudo pacman -S vulkan-icd-loader` plus the
  card's driver (`vulkan-radeon`, `vulkan-intel` or the NVIDIA one).

---

## Windows

Windows 10 or 11, 64-bit. The app talks to the scanner in one of two ways, and tries them by
itself in this order:

1. **HP driver** (`usbscan.sys`): if the scanner is already installed with the HP driver, the
   app uses it. Nothing changes in the system and the other scanning apps keep working.
2. **WinUSB**: if the HP driver is not there, the scanner is given Microsoft's generic WinUSB
   driver with **Zadig**. From then on only this app (and the `hp3770` command) uses the
   scanner, no longer Windows' scanning apps.

### 1. The driver: check what is already there

1. Plug in and switch on the scanner.
2. Right click on **Start** → **Device Manager**.
3. Look for the scanner:
   - under **Imaging devices** (or **Cameras and scanners**) there is **HP Scanjet 3770**
     without a yellow triangle → **the HP driver is installed, nothing else is needed**. Go
     to step 2.
   - under **Other devices** there is an unknown device with a yellow triangle, or nothing
     from HP → **the driver is missing**: follow *WinUSB with Zadig* below.

#### WinUSB with Zadig (only if the HP driver is missing)

1. Download Zadig from [zadig.akeo.ie](https://zadig.akeo.ie) and run it (it asks for the
   administrator's confirmation).
2. Menu **Options** → tick **List All Devices**.
3. In the drop-down choose the scanner: check that **USB ID** shows **`03F0 2505`**. Take
   care not to choose another device (keyboard, mouse, disk): you would change its driver.
4. To the right of the green arrow choose **WinUSB**.
5. Press **Install Driver** (or **Replace Driver**) and wait for the message that it is done.

To go back: **Device Manager** → right click on the scanner → **Uninstall device** → tick
*Attempt to remove the driver* → unplug and plug the cable back in.

### 2. The app

**Installer (recommended)**

1. Download `vetroscan-vX.Y.Z-windows-x86_64-setup.exe` and run it.
2. If **Windows protected your PC** appears (SmartScreen: the installer is not signed with a
   paid certificate), press **More info** → **Run anyway**.
3. Choose **Install for me only**: it does not ask for administrator rights and puts the app
   in `%LOCALAPPDATA%\Programs\VetroScan`. *Install for all users* puts it in
   `C:\Program Files\VetroScan` and asks for them.
4. Next until the end; if you like, tick the desktop icon. The app opens from the **Start**
   menu → **VetroScan**.

Silent install (for whoever deploys it):
`vetroscan-vX.Y.Z-windows-x86_64-setup.exe /VERYSILENT /CURRENTUSER` (or `/ALLUSERS`).

**Portable (without installing)**

1. Download `vetroscan-vX.Y.Z-windows-x86_64-portable.zip`.
2. Right click → **Extract All** into a folder of your choice.
3. Double-click `vetroscan.exe`.

### 3. The scanner buttons (with the HP driver only)

With the HP driver, at every button press Windows also opens its own scanning app, which
takes the scanner away from this one. In the app, **Preferences** tab, press **Use the
scanner buttons with this app only** and confirm as administrator once: Windows no longer
does anything on the buttons. **Give the buttons back to Windows** puts it back as it was.
With WinUSB the problem does not arise.

### Checking that it works (Windows)

In **PowerShell** (Start → type "PowerShell"), with the scanner plugged in:

```powershell
& "$env:LOCALAPPDATA\Programs\VetroScan\hp3770.exe" list
```

(With the install for all users: `& "C:\Program Files\VetroScan\hp3770.exe" list`; with the
portable zip, the path of the extracted folder.) It lists the scanner with the backend that
opens it: `usbscan.sys` with the HP driver, `nusb` with WinUSB. If `nusb` says
`bound to …, nusb needs WinUSB`, the scanner has another driver: run Zadig again. Then:

```powershell
& "$env:LOCALAPPDATA\Programs\VetroScan\hp3770.exe" status
```

### Updating and uninstalling (Windows)

- **Updating:** run the `-setup.exe` of the new version; it replaces the old one and keeps
  the settings. Over a version up to v0.2 ("HP Scanjet 3770") it stays in the same folder
  and replaces the old Start menu entry.
- **Uninstalling:** **Settings** → **Apps** → **Installed apps** → **VetroScan** →
  **⋯** → **Uninstall**. The portable zip is simply deleted. The driver (HP or WinUSB)
  stays: to remove it, see *To go back* above.
- **The photocopy** (Copy button) prints on Windows' default printer through Paint.

---

## macOS

macOS 11 or newer, Apple Silicon (M1 and later) or Intel. No driver is needed: the app opens
the scanner directly.

> **Still to be tried.** The macOS app is built at every version, but it has not yet been
> tried with a real scanner. If something does not work, open an issue on GitHub with the
> output of `hp3770 list` and `hp3770 status`.

1. Find out the processor:  → **About This Mac**. **Chip Apple M…** → download
   `vetroscan-vX.Y.Z-macos-arm64.dmg`; **Intel processor** → `vetroscan-vX.Y.Z-macos-x86_64.dmg`.
2. Open the `.dmg` and drag **VetroScan** onto the **Applications** folder (or into the
   `Applications` of your home folder, which does not ask for the password).
3. **First opening.** The app is not signed with a paid Apple certificate, so macOS says it
   cannot check it:
   - macOS 15 and later: press **Done**, open **System Settings** → **Privacy & Security**,
     scroll down to *Security* and press **Open Anyway** next to VetroScan, then
     confirm with the password. It is needed only once.
   - macOS 14 and earlier: in **Applications** right click (or Ctrl+click) on the app →
     **Open** → **Open**.
   - From the Terminal, the same effect:
     ```bash
     xattr -dr com.apple.quarantine "/Applications/VetroScan.app"
     ```
4. If macOS asks to let the app use a **USB accessory**, press **Allow**.

**The `hp3770` command** is inside the app. To have it in the Terminal:

```bash
sudo mkdir -p /usr/local/bin
```
```bash
sudo ln -sf "/Applications/VetroScan.app/Contents/MacOS/hp3770" /usr/local/bin/hp3770
```

Then, with the scanner plugged in, `hp3770 list` and `hp3770 status`.

- **Updating:** download the new `.dmg` and drag the app over the old one. Up to v0.2 the
  app was called **HP Scanjet 3770**: drag that one to the Bin, and if you had made the link
  to the `hp3770` command, make it again as above (it pointed inside the old app). The
  settings come along.
- **Uninstalling:** drag **VetroScan** from Applications to the Bin, and
  `sudo rm /usr/local/bin/hp3770` if you had made the link.
- **The photocopy** prints on the Mac's default printer.

---

## Where the settings are

Preferences and presets stay from one update to the next, and are not deleted by
uninstalling:

| System | Folder |
|---|---|
| Linux | `~/.config/vetroscan` |
| Windows | `%APPDATA%\vetroscan` |
| macOS | `~/Library/Application Support/vetroscan` |

Versions up to v0.2 kept them in a folder called `hp3770` (in the same places): at the
first start the new version copies them into its own folder, and leaves the old one as it is.

To start from scratch, close the app and delete that folder.
