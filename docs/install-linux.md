# Installing the scanner on a new computer with Omarchy

A step-by-step guide, also for people who have never used the terminal. At the end the
**HP Scanjet 3770** works with its app, **VetroScan**, which opens from the applications
menu like any other program.

Time: about 15 minutes, almost all of it waiting while the computer compiles.

No HP drivers, SANE or other scanning programs are needed: the app talks to the scanner
directly.

> **There is a shorter way.** This guide builds the app from source, which is why Rust and a
> few minutes of waiting are needed. Since the first published version, the
> [Releases](https://github.com/MatteoDM/scanner/releases/latest) page has the ready-made
> package for Omarchy/Arch (`vetroscan-vX.Y.Z-linux-x86_64.pkg.tar.zst`): download it and
> install it with `sudo pacman -U` followed by the file's name. It does steps 3, 4 and 6 by
> itself (udev rule included); then skip to step 5 to check. The instructions for every
> system, packages and AppImage included, are in [install.md](install.md).

---

## Before starting

- A computer with **Omarchy** installed and connected to the internet.
- The **scanner plugged in** to the mains and, with the USB cable, to the computer. A USB hub
  is fine, as long as it is powered.
- Knowing how to **open a terminal**: in Omarchy it opens with **Super + Enter** (Super is
  the key with the Windows logo).

**How the terminal is used in this guide.** Every grey box holds commands. Copy them (select
and Ctrl+C), paste them in the terminal with **Ctrl+Shift+V** and press **Enter**. When a
command asks for the **password**, it is the one you log in to the computer with: nothing
shows on the screen while you type it, that is normal. Type it and press Enter.

The lines starting with `#` are explanations, not commands: if you paste them they do
nothing.

---

## Step 1 — Installing Rust (what is needed to compile)

The app is written in Rust and has to be compiled on the computer. Omarchy installs Rust with
a single command:

```sh
omarchy install dev-env rust
```

It asks for the password and then downloads for a couple of minutes. When it has finished,
**close the terminal and open a new one** (Super + Enter): the new terminal finds the
commands just installed.

To check:

```sh
cargo --version
```

It must answer with something like `cargo 1.9x.0`. If it says `command not found`, close and
open the terminal once more; if that is not enough, log out of the session and back in.

> It can also be done from Omarchy's menu: **Super + Space → Install → Development → Rust**.
> If the Rust entry is grey, Rust is already there.

---

## Step 2 — Downloading the program

```sh
mkdir -p ~/Work
cd ~/Work
git clone https://github.com/MatteoDM/scanner.git
cd scanner
```

This way the program ends up in the `Work/scanner` folder in your home folder.

---

## Step 3 — Letting your user use the scanner

By default Linux lets only the administrator use USB devices. This rule gives access to the
scanner to whoever sits at the computer, without a password:

```sh
sudo install -m 644 packaging/linux/70-hp3770.rules packaging/linux/70-epson-4490.rules /etc/udev/rules.d/
sudo udevadm control --reload
```

Then **unplug the scanner's USB cable and plug it back in**: the rule applies from then on.

---

## Step 4 — Compiling and installing

```sh
cargo install --path crates/vetroscan --locked
cargo install --path crates/hp3770-cli --locked
```

The first command builds the app and takes **a few minutes** (5 or more on a laptop): many
`Compiling …` lines scroll by, all is well. It ends with `Installed package …`. The second
command, faster, installs `hp3770`, a small command to check the scanner from the terminal.

The programs go to `~/.cargo/bin`, which the terminal already knows after step 1.

---

## Step 5 — Checking that the computer sees the scanner

```sh
hp3770 status
```

If all is well the first line says the scanner was opened, and then some lines appear,
among them:

```text
Opened the scanner with nusb at bus 003 address 13
USB speed          2.0
carriage at home   yes
```

The "bus" and "address" numbers may differ: it does not matter.

If a **permission** error appears instead (`Permission denied`, `Access denied`), the rule
of step 3 is not active: redo step 3 and **plug the USB cable back in**. If it says it
**cannot find the scanner**, see [Problems](#problems).

---

## Step 6 — Putting the app in the applications menu

```sh
install -Dm 644 packaging/linux/vetroscan.desktop ~/.local/share/applications/vetroscan.desktop
install -Dm 644 packaging/linux/vetroscan.svg ~/.local/share/icons/hicolor/scalable/apps/vetroscan.svg
```

The first line puts the app in the menu, the second gives it its icon.

Now open the app launcher with **Super + Alt + Space**, type **scanner** (or **HP**) and
**VetroScan** appears. Enter, and the app opens.

Done: from here on the terminal is no longer needed.

---

## Using the app, briefly

The app follows the system's language (Italian or English); it can be changed in the
**Preferences** under *Language*.

1. At the top choose **Document** (sheets, also several pages in one PDF) or **Photo**.
2. Put the original on the glass, **face down**, and close the lid.
3. Press **Preview**: after a few seconds you see the glass. The first time the lamp has to
   warm up and it takes a little longer.
4. **Drag on the preview** to choose the area to acquire (if you choose nothing it acquires
   an A4 sheet). With **Automatic crop** the app finds by itself the photos lying on the
   glass, even more than one.
5. Press **Scan page 1** (in Photo **Scan photo**). In the Document mode every scan adds a
   page.
6. In the **Output** tab choose the format, then press the button at the bottom right
   (**Save PDF…**, **Save JPEG…**) and choose where to save.

The **scanner buttons** work with the app open in the background too:

| Button | In Document | In Photo |
| --- | --- | --- |
| Scan (left) | adds a page | scans the photo |
| Copy (centre) | redoes the last page | photocopy: scans and prints |
| Instant Share (right) | saves the document and opens the folder | opens the folder |

What each button does is changed in the **Preferences**.

**For Lightroom or Photoshop** choose the **RAW · DNG** format (Lightroom, Camera Raw) or
**RAW · linear 16-bit TIFF** in **Output**. They are the scanner's data without retouching,
highlights included, to be developed in the photo editor.

Everything else (adjustments, the scan editor, formats, buttons, preferences, common
problems) is explained in the **[usage guide](usage.md)**.

---

## Updating to a new version

```sh
cd ~/Work/scanner
git pull
cargo install --path crates/vetroscan --locked
cargo install --path crates/hp3770-cli --locked
```

The app's settings stay: they are saved in `~/.config/vetroscan`.

Up to v0.2 the app was called "HP Scanjet 3770" and its program `hp3770-app`. Updating from
one of those versions, after the commands above do step 6 again and remove the old program
and its menu entry, so that the menu does not show two apps:

```sh
cargo uninstall hp3770-app
rm ~/.local/share/applications/hp3770-app.desktop
rm ~/.local/share/icons/hicolor/scalable/apps/hp3770-app.svg
```

The settings of the old version are copied at the first start of the new one.

---

## Problems

**The app says "The scanner is plugged in, but Linux does not let this user use it".**
The rule of step 3 is missing. Press **Allow access to the scanner** in the same tab and type
the password: the app installs the rule by itself and connects to the scanner again, without
unplugging the cable.

**The app says "Scanner not found".**
- Is the scanner switched on and the cable plugged in? Try another USB port, preferably
  directly on the computer.
- In the terminal, `lsusb | grep 03f0:2505` must show a line. If it shows nothing, the
  computer does not see the scanner at all: cable, socket or port.
- If the line is there but `hp3770 status` gives a permission error, redo step 3 and plug
  the cable back in.
- Only one program at a time can use the scanner: close other copies of the app or an
  `hp3770` left open in a terminal.

**Pressing "Save" does not open the window to choose the file.**
The desktop "portal" is needed, which Omarchy already has. If it was removed:

```sh
sudo pacman -S --needed xdg-desktop-portal-gtk
```

then log out of the session and back in.

**The photocopy (Copy button) does not print.**
Printing goes through the printing system (CUPS): the printer must be set up and made the
default, because the app always prints on that one. Try `lp` followed by the name of an
image; the printer is set up from Omarchy's menu (**Super + Space**).

**`cargo install` stops with an error.**
Usually something from step 1 is missing. Redo step 1, open a new terminal and try again. If
the error stays, copy the last 20 lines and ask for help.

**The launcher (Super + Alt + Space) does not find the app.**
Redo step 6. The launcher sometimes notices the new app only after a minute, or after
logging out of the session and back in.

---

## Uninstalling

```sh
cargo uninstall vetroscan
cargo uninstall hp3770-cli
rm ~/.local/share/applications/vetroscan.desktop
rm ~/.local/share/icons/hicolor/scalable/apps/vetroscan.svg
sudo rm /etc/udev/rules.d/70-hp3770.rules /etc/udev/rules.d/70-epson-4490.rules
rm -r ~/Work/scanner          # the downloaded code
rm -r ~/.config/vetroscan     # the settings (optional)
```

The saved scans are not touched.
