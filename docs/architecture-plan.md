# Plan: from "an app for the HP 3770" to "an app for scanners"

Status: **phases 0–7 done and published (v0.1.2, v0.2.0, v0.3.0), with the double final
check; phases 8–9 postponed until there is hardware; phases 10–11 (films on the 3770,
precise moves from the keyboard) planned** (plan written on 8 November 2026, after v0.1.1;
updated the same day with the languages, the new name, the postponed phases and phases
10–11; on 9 November phase 12, the Epson Perfection 4490 Photo, researched before it arrives,
and the code of its milestone 1 written without it: see "Where it stands" there).
Each phase ends with a commit, green tests and a version; what is done is marked here.

Order: first the refactoring (phases 1–4), then languages, the project in English and the
new name (phases 5–7), which need no hardware; the other scanners (phases 8–9) are
**postponed** until there is hardware to try them on.

## Goal

1. Every HP scanner with a Realtek RTS88xx chip (the family of the SANE `hp3900` backend)
   works with the same app.
2. The architecture welcomes scanners of other brands (eSCL/AirScan, SANE, WIA) without
   touching the app, the image processing and the export.
3. For whoever uses the HP 3770 today nothing changes: the same scans, byte for byte.

## Where it starts from (measured on the code of v0.1.1)

`hp3770-core` has about 9,000 lines, of two kinds mixed together:

| Generic imaging (~2,700 lines) | RTS8822 driver (~6,300 lines) |
|---|---|
| `transform` (no dependency), `convert` (none), `detect` (only `session::Image`), `adjust` (uses `Profile` only for the reference white, `geometry::host_white`) | `protocol`, `transport`, `setup`, `init`, `lamp`, `motor`, `motor_curves`, `calibration`, `scan`, `scan_modes`, `read`, `arrange`, `session`, `status`, `device`, `profile`, `geometry` |

The app touches the scanner in one place only, `crates/vetroscan/src/worker.rs` (commands
and events), and a made-up scanner (`worker::fake`) used by the e2e tests already exists.
The ties to undo:

| Tie | Where | Problem |
|---|---|---|
| `HP3770`, `HP_WHITE` | `main.rs`, `adjust` | tone curve and white belong to this model |
| `GLASS_INCHES`, `RESOLUTIONS` | `main.rs` (constants) | glass and resolutions change with the model |
| `device::Button` (59 uses), Scan / Copy / Instant Share | `main.rs`, `worker.rs` | other models have other buttons or none |
| `status::Status` with registers (`link_word`, `chipset_id`) | `main.rs`, `worker.rs` | a network scanner has no registers |
| `Chip<Scanner>`, `hp3770_usb::open` | `worker.rs` | only one backend possible |

## Rules for every phase

- No phase changes the HP 3770's behaviour: the tests on the HP driver's captures
  (`hp3770-capture`), the app's e2e tests and the smoke tests on the real scanner
  (`cargo test -p hp3770-cli --test hardware -- --ignored --test-threads 1`) stay green.
- One step at a time, each one building; temporary re-exports are removed at the end of the
  phase that introduced them.
- Preferences already saved by users are migrated, with a test for each migration.
- **Capabilities rule, always** (a rule the user asked for on 8 November 2026). Every part of
  the app tied to a function of the scanner exists only if that scanner's `Capabilities`
  declare it: in the interface (panels, entries, options, shortcuts, texts that name it) and
  in the paths of the backend and of the app (commands, timers, saves). For instance: no lamp
  nor lamp timer without `lamp`; no buttons nor button actions without `buttons`, and only
  those there are; no 16 bits nor RAW without `16` in `depths`; only the resolutions of
  `resolutions`; glass and rulers from `glass_inches`; slides only with `transparency`;
  technical details only those of `DeviceStatus`. The preferences of a missing function stay
  saved (for when the scanner that has it comes back) but are not shown and do not act.
  - Every new function of the contract comes together with: the field in `Capabilities`, the
    branch of the app that depends on it, an e2e with a made-up scanner that has it and one
    with one that does not.
  - In the app's code the scanner's functions are asked in one place only (what the open
    scanner offers: `App::caps` and its helpers), not with scattered checks: this way it is
    easy to see and test that the rule holds everywhere.

## Phase 0 · work in progress → v0.1.2 · done

PDF with layout per page (own margin, position, orientation, "Apply to all pages"), print
preview with the red edge of the cut and a draggable cut with "Fill the sheet". It came first
because it touches the same files as the refactoring (`main.rs`).

## Phase 1 · separating the imaging from the driver → v0.2.0 · done

- New crate **`scan-imaging`**: `Image` and `Format` (then in `session`), `transform`,
  `convert`, `detect`, `adjust`.
- `adjust` receives a **`Tone`** value (reference white, host curve) instead of the whole
  `Profile`; every scanner will give it.
- `hp3770-core` becomes the driver only and for this phase re-exports the types moved.
- **Done when:** all tests green; a "golden" test passes the same scan through `adjust`
  before and after and gets identical images byte for byte; `vetroscan-export` depends only on
  `scan-imaging`.

How it went (4 commits, from the golden test on):

- `Mode` (colour, grey, black and white) also moved to `scan-imaging`: `adjust` needs it.
  `session::Request` uses it from there.
- `Tone` = the colour whites per channel, the grey white, the gamma, with `Tone::table`
  (formerly `geometry::host_tone`). The driver gives it with `Profile::tone()`; the
  explanation of the HP curve is on `geometry::HOST_GAMMA`.
- Golden test: `crates/hp3770-core/tests/golden_adjust.rs`, 36 cases (`render`,
  `render_samples` at 16 bits, `render16`; three modes; default adjustments, all changed,
  adaptive threshold; linear or not). Values fixed on v0.1.2, the same on Windows and Linux;
  on macOS one case (16-bit output, gamma 1.3) rounds one sample differently in the system's
  maths library, so macOS has its own value for it.
- The re-exports were already removed at the end of the phase: app, CLI and export import from
  `scan_imaging`. `hp3770-core` stays only as a development dependency of `vetroscan-export`, for
  the `raw_from_png` example (the HP 3770's paper white).

## Phase 2 · the "scanner" contract → v0.2.0 · done

New crate **`scanner-api`**, which knows nothing about USB or chips:

- `trait Scanner`: `capabilities()`, `status()`, `scan(&ScanRequest, progress) ->
  ScanResult` (cancellable), `poll_buttons()`, and the optional `lamp_off()`.
- `Capabilities`: model and maker, size of the glass, resolutions, modes (colour, grey,
  black and white), depths, transparency adapter yes/no, buttons (id, label, suggested
  action), lamp and warm-up time, `Tone`.
- A generic `DeviceStatus`: ready or busy, lamp (if any), adapter (if any), plus a free list
  of technical details the app shows without understanding them.
- `Registry`: lists the scanners of every backend available and opens them.
- Common errors: cancelled, disconnected, busy, other.
- **Done when:** the trait is documented and the made-up scanner implements it.

How it went:

- `scanner-api` depends only on `scan-imaging` (for `Image` and `Tone`). A scan always gives
  the **linear** colour samples (8 bits, and 16 if asked): the app already does adjustments,
  grey and black and white by itself, so `ScanRequest` is only resolution, area and depth;
  `Capabilities::modes` says what the scanner offers on its own.
- `poll_buttons()` gives the buttons held now (bit `i` = `capabilities().buttons[i]`); the
  reader recognises the presses. `lamp_off()` has a default implementation answering
  `Unsupported`. `check()` checks a request against the capabilities.
- The made-up scanner is `scanner_api::fake`, in two models: `flatbed()` (like the 3770) and
  `a3()` (A3 glass, up to 1200 dpi, 8 bits only, two buttons, no lamp), with `Controls` to
  hold buttons down and unplug the cable.

## Phase 3 · the HP 3770 behind the contract → v0.2.0 · done

- Backend **`rts8822`** in the driver: `impl Scanner` for chip + profile.
- **VID/PID → `Profile`** table (only the 3770 for now); `Capabilities` drawn from the
  profile.
- The CLI goes through the `Registry`; `probe`, `init`, `lamp` stay only where the backend
  has them.
- **Done when:** capture tests identical, smoke tests on the real scanner green.

How it went:

- The `Profile` has a `DeviceModel` (maker, model, VID/PID, glass in hundredths of an inch,
  resolutions, buttons); `profile::PROFILES` and `profile::for_device(vid, pid)` are the
  table. Glass and resolutions are the HP driver's (`geometry::GLASS_INCHES`, `RESOLUTIONS`),
  which cannot be drawn from the `Platform`: a test checks they match.
- `hp3770_core::rts8822::Rts8822<T: Transport>` implements `Scanner`: it asks `session::scan`
  exactly the request the app made (linear colour, no sharpening, fast travel) and puts lamp,
  calibration, buffer and the odd pixels of the white strip in the details. A test proves the
  linear samples come out untouched, so processing them afterwards gives the same images as
  before.
- `hp3770_usb::UsbBackend` is the `Registry`'s backend (it opens like `hp3770_usb::open`,
  with `usbscan.sys` first on Windows); `UsbscanScanner` is now `Send`.
- `hp3770 scan` goes through the `Registry` (or the driver directly with `--backend` or
  `--no-fast-travel`) and processes the samples with the scanner's `Tone`; the other commands
  stay the driver's.
- Smoke tests on the real scanner: 5 out of 5 before and after.

## Phase 4 · the app talks only to the contract → v0.2.0 · done

- `worker.rs` holds a `Box<dyn Scanner>` chosen from the `Registry`; with several scanners, a
  menu.
- No more `GLASS_INCHES`, `RESOLUTIONS`, `HP_WHITE`: everything from the `Capabilities`. The
  buttons become "by id" and their actions are saved per model (with migration).
- **Done when:** today's e2e pass unchanged and **new e2e** with a second, different made-up
  scanner (A3 glass, at most 1200 dpi, two buttons, no lamp) show that the app adapts by
  itself, following the "capabilities rule" rule: for each capability an e2e checks that with
  the scanner that has it the app offers it and with the one that does not it shows nothing
  of it.

How it went:

- The scanner thread (`worker.rs`) holds a `Box<dyn Scanner>` opened from the `Registry`
  (`hp3770_usb::UsbBackend`, or the made-up scanner with `VETROSCAN_FAKE_SCANNER`). The e2e run
  the **same** thread on `scanner_api::fake`: before they had a separate fake. The events
  carry `DeviceStatus`, `Progress`, `ScanResult` and the button as an index; `Event::Opened`
  gives capabilities and the list of scanners.
- Before a scanner answers the app uses the first model the backends know
  (`Backend::models()`); `Capabilities::placeholder()` if they know none.
- The `Tone` travels with the samples (`Raw::tone`): each page is drawn with the curve of the
  scanner that acquired it, and the paper white for straightening and for RAW is its own.
- Resolution and depth chosen stay saved; the app uses the nearest resolution the scanner
  offers (`App::dpi`) and 8 bits if it does not give 16 (`App::depth`). The preview is made at
  the scanner's resolution nearest to 200 dpi and brought back to 200.
- Buttons by id: the actions are saved in `Prefs::buttons` per model and id; the preferences
  of v0.1.x (by position) go to the HP Scanjet 3770 with a migration test, and are still
  written in the old format too for whoever goes back.
- Capabilities rule: lamp (preferences, timer, switching off when closing, state), buttons
  (preferences, summary, hint under the strip, Windows option only with the `rts8822`
  backend), 16 bits, resolutions, glass, film mode and state details all come from
  `Capabilities` / `DeviceStatus`. With several scanners, "Use" in the Preferences.
- New e2e: the A4 flatbed (lamp, buttons, 16 bits), the A3 flatbed (no lamp nor 16 bits, its
  two buttons, its own glass and resolutions, closing without waiting), a scanner without
  buttons, button actions remembered per model when going from one scanner to the other. The
  thread has tests of its own: a press arrives once and only for buttons that exist, no lamp
  switching for whoever has none, the scanner chosen is opened.
- Smoke tests on the real scanner: also `the_registry_opens_the_scanner_like_the_app`
  (registry, state, buttons, 16-bit scan, lamp off).

## Phase 5 · the app in Italian and English → v0.3.0 · done

Italian stays the first language; English is chosen in the Preferences, and if the system is
in English the app starts in English.

- **"Language" preference**: *Automatic* (default), *Italiano*, *English*. Automatic =
  English if the system's language starts with `en` (crate `sys-locale`, MIT/Apache),
  Italian otherwise. Saved in `impostazioni.json`; whoever does not have it yet gets
  *Automatic*. Changing it redraws the interface at once, without restarting.
- **What is translated**: labels, buttons, menus, hints, status notes (`self.note`),
  questions and titles of the dialogs, notifications, worker errors shown to the user.
- **Packages**: Windows installer in Italian and English, `.desktop` in both languages,
  `CFBundleLocalizations` in the macOS `.app`'s `Info.plist`.
- **Done when:** a test checks that every key used in the code exists in the catalog and
  none is left over; a test looks for texts written by hand in the widgets and fails if it
  finds any; today's e2e pass unchanged in Italian and a group of e2e runs in English.

How it went:

- No Fluent (the first plan): with about 440 texts, invented keys would have meant writing
  every text twice and changing types in hundreds of places. Gettext style instead: the key
  is the Italian text, `tr("…")` gives a `&'static str` in the language in use (it replaces a
  literal anywhere without touching the types) and `trf("… {n} …", &[("n", &n)])` fills in
  the places. English is in `crates/vetroscan/i18n/en.json`; Italian has no catalog.
  Sentences with different plurals or agreements are separate texts.
- `crates/vetroscan/src/i18n.rs`: language per thread in the tests (the English e2e run next
  to the Italian ones), `LangChoice` (Automatic, Italiano, English) saved in
  `impostazioni.json` (`language`, default "automatica"), `sys-locale` for the system's
  language (always Italian in the tests). Decimal numbers follow the language.
- Tests of the module: every text asked for has its English with the same places and none is
  left over; no literal "for people" skips `tr`/`trf` (brand, the "Scansioni" folder, system
  commands are marked `// not translated`). Two ignored tools help add texts:
  `i18n::survey` (a list of literals to look at) and `i18n::keys` (writes the keys as JSON).
- Packages: the Windows installer and the `.desktop` were already bilingual; the macOS bundle
  now declares `CFBundleLocalizations` it/en.
- The `hp3770` CLI stays in English: it is a technical tool and already spoke English.

## Phase 6 · the project in English → with v0.3.0 · done

Documentation, commits and release notes go to English; the app stays Italian by default
(phase 5).

- **From when**: from the first commit of this phase every commit message is in English.
  The history already published is not rewritten.
- **Documents**: `README.md` in English, plus a short `README.it.md` for whoever uses the app
  in Italian (installation and first steps). In English too the usage and installation
  guides, the USB protocol, this plan, the `.claude/skills/release` skill.
- **CHANGELOG**: in English from this phase's version; past sections translated too, so the
  file is in one language (the notes of releases already published on GitHub stay as they
  are).
- **Code**: comments and documentation were already in English; the log lines of the
  buttons (`tasti.log`) too now. Errors shown to the user go through the catalog.
- **Written rule**: a `CONTRIBUTING.md` in English (commit language, CHANGELOG, tests to
  pass, how to add a translation).

How it went:

- The guides have English names: `docs/install.md`, `docs/install-linux.md`,
  `docs/usage.md`, `docs/usb-protocol.md`, and this file is `docs/architecture-plan.md`;
  every link was updated. The usage guide also describes what changed meanwhile (language,
  each page's own adjustments, what the app hides for scanners without a function).
- `packaging/` and the workflows were already in English.
- The capture files (`tests/fixtures/*.pcap`) and the folder name `Scansioni` keep their
  Italian names: tests and users' files refer to them.

## Phase 7 · a new name, not tied to the HP 3770 → v0.3.0 · done

With several scanners the name "HP Scanjet 3770" no longer describes the app. The user first
chose "retroscan"; checked before renaming, it turned out to be the name of about ten small
GitHub projects, one of them a macOS scanning app, and the user asked for another name. The
user then chose **VetroScan** (8 November 2026): `vetroscan` in programs, packages, crates
and folders, "VetroScan" where it is shown; free on crates.io and on GitHub. Below,
`retroscan` in the original plan reads as VetroScan.

- **What changes name**: window title and name in the bar, binaries (`retroscan` for the
  app, the CLI also stays as `hp3770` as long as needed), packages
  (`retroscan-vX.Y.Z-<system>.<ext>`), `.desktop` and icon, macOS bundle and `.dmg`, the
  app's and the export's crates (`retroscan-app`, `retroscan-export`; the driver stays
  `hp3770-core`, or becomes `rts88xx` with phase 8), README and documents.
- **What must not break**:
  - Windows: **the installer's `AppId` does not change** (it is how Windows updates and
    uninstalls); the name shown and the folder change, and the new installer removes the old
    menu entry.
  - Linux: the new packages declare `replaces`/`conflicts`/`provides: hp3770`, so the update
    replaces the old one; the udev rule stays valid (only the file's name changes, with the
    old one removed in `postinstall.sh`). AUR: a new package and `hp3770-git` pointing to it.
  - Preferences and presets: the folder goes from `…/hp3770` to `…/retroscan`; at the first
    start the old files are copied (migration with a test), without losing anything.
  - The GitHub repository (`MatteoDM/scanner`) is already generic; the links to old releases
    stay valid.
- **Done when:** an installation of v0.2.0 updates to the new one on Windows and on Linux
  (deb and rpm) keeping preferences and presets, and the final checks below are clean.

How it went:

- The rename is a script with the name as a parameter (crate folders, packages, icons,
  installer, workflow, docs), run on the commit before it: a different final name is one run
  away. In the code the shown name is `APP_NAME` in the app's `main.rs`; the program and the
  settings folder take the package's name (`env!("CARGO_PKG_NAME")`).
- The app crate is `crates/vetroscan` (package and program `vetroscan`, not `-app`), the
  export `crates/vetroscan-export`. The driver crates, the `hp3770` command and the
  `70-hp3770.rules` udev rule keep their names: they are the HP 3770's, not the app's. So no
  rule file is renamed (unlike the first plan).
- Windows: the `AppId` is unchanged, the default folder for new installs is
  `Programs\VetroScan`, an update stays in the old `Programs\HP3770`; `[InstallDelete]`
  removes `hp3770-app.exe` and the old shortcuts.
- Linux: the package `vetroscan` conflicts with, replaces and provides `hp3770` (and the
  `-git` packages); the AUR PKGBUILD is `vetroscan-git`.
- Settings: `prefs::migrate_dir` copies the files of the old `hp3770` folder into the new one
  at the first start, only if the new one does not exist, and leaves the old one (test).
- The status bar names the scanner in use next to its connection.
- New CI steps, on every build: "Update test from v0.2.0" installs the v0.2.0 installer
  (Windows) or `.deb` (Linux, both architectures) and this version over it, and checks that
  the old program and shortcut are gone, the new ones are there, and the old package is
  replaced.

## Final coherence check (double) · after phase 7 · done

Two independent passes, the second only after correcting everything the first finds.

1. **Automatic**:
   - `git grep` of the old name ("Scanjet", "hp3770", "HP3770") outside the driver, the
     captures and the past entries of the CHANGELOG: no result left unexplained;
   - `git grep` of common Italian words in documents and comments ("della", "sono",
     "questo", …): only in `README.it.md`, in the app's Italian texts and in the names kept
     on purpose;
   - catalog completeness and hand-written text tests (phase 5) green;
   - full CI with the packages under the new names; update test from the previous version on
     Windows and Linux; preference migration tests;
   - every link of the README and `docs/` works.
2. **By hand**:
   - a full tour of the app in Italian and English (start, preview, scan, adjustments, editor,
     PDF, saving, Preferences, scanner buttons), with screenshots: no text cut off, mixed or
     untranslated;
   - reading README, `docs/`, CHANGELOG and the release notes from start to end: the same
     name, the same language, the same words for the same things;
   - a clean install on every system and a check of name, icon and menu entry.

How it went:

- First pass: RAW metadata no longer name the HP 3770 for every scanner, the export's and
  backends' texts are English, the thumbnails' notes are translated, the test variables
  follow the new name.
- Second pass: the tour (`e2e::tour`, 13 screenshots per language, Photo and the editor
  included) found two texts cut off, numbers with the other language's decimal separator,
  the last note left in the old language and "Annulla" of the editor translated as Cancel
  (now `tr("undo|Annulla")`). Reading the documents added how to update from v0.2 on macOS
  and from source on Linux; a script found no broken link.
- Clean installs were not tried by hand: the CI installs every package it builds and, on
  Windows and Linux, updates v0.2.0 to the new version. Smoke tests on the real scanner:
  6 out of 6.

## Phase 8 · other HP scanners with an RTS88xx chip · postponed (experimental)

Postponed: without these scanners nothing can be tried. It starts again when a tester or a
borrowed model turns up.

- Profiles for HP 3800, 3970, 4070, 4370, G3010, G3110 (and the others of the family) from the
  data of the SANE `hp3900` backend, with the source cited as for the 3770.
- They come out "experimental": people with those scanners are needed, a `probe`/`status` in
  an issue or, better, a USB capture of the HP driver.
- **High risk:** motor, sensor and calibration change with the model and cannot be checked
  without hardware; that is why after the refactoring and one at a time.

## Phase 9 · other brands · postponed

Postponed like phase 8: eSCL can be tried with any network multifunction, so it is the first
to start again as soon as one is available.

In order of return for cost:

1. **eSCL/AirScan**: HTTP and mDNS discovery, the same on Linux, Windows and macOS; almost
   every modern network scanner and multifunction, without drivers.
2. **SANE** (Linux, macOS): libsane loaded at run time, hundreds of USB models.
3. **WIA** (Windows): a fallback for whoever has only the Windows driver.

Each backend only implements `trait Scanner`: app, imaging and export do not change.

## Phase 10 · slides and negatives with the HP 3770's adapter · done (slides through phase 13)

Added on 8 November 2026. Until then the documents said the HP 3770 has no adapter for
films: it is wrong. The 3770 has one built into the lid (a light panel with a holder for
**two 35 mm slides or one strip of negatives**, and a cable to the "TMA" socket at the back):
HP user's manual `c00208907`, sections "Scan slides and negatives" and "Transparent materials
adapter (TMA) specifications". Matteo's scanner reports it plugged in (`hp3770 probe` of
7 November: `E969` = `ED`, bit 1 clear, the test `status_from_the_hardware_probe`), and the
HP driver has the private WIA property 71685 "TMA Method".

It needs no new hardware, only slides and negatives to put in the holder.

- **Correct the documents first**: `rts8822.rs` (the comment by `transparency: false`),
  `docs/usage.md` (the Film button), the CHANGELOG entry of v0.3.0 (a note in the new
  version, the past entry stays). The README already lists the adapter as missing. Done on
  8 November 2026, with phase 11.
- **Captures of the HP driver**, the same way as the ones of 7 November (`wia1scan` with
  "TMA Method" set, USBPcap): the adapter's start-up and warm-up (the manual asks for at least
  30 s), a preview of the holder, a slide and a strip of negatives at 600, 1200 and 2400 dpi,
  8 and 16 bits. The HP images are the reference for the comparison.
  Tried on 8 November 2026, without an image. What was learnt:
  - the HP driver chooses the adapter with the root item's private property 71684 "TMA
    Select" (the scan item has 71685 "TMA Method", 0 or 1); the adapter's area is 42.3 x
    93.1 mm (333 x 733 pixels at 200 dpi), 600 dpi by default; only 8 bits per channel (71686
    "Source Depth" refuses every value), so the 16-bit captures make no sense;
  - on Windows 11 every scan with the adapter fails (`WIA_ERROR_DEVICE_COMMUNICATION`): the
    HP driver writes the lamp registers, waits a second without asking the scanner anything
    and gives up. Its own errors (`TERR_XPA_LAMP_FAILED`, `TERR_BACKING_NOT_REMOVED`, ...) do
    not reach WIA, and its log (`HKLM\Software\Hewlett-Packard\ScanJet\Tulip\{GUID}`,
    found with Process Monitor) writes no file;
  - with the registers written by hand (`E946`, `E950`-`E951`, `E954`-`E955`, every
    combination; the development tools `examples/poke.rs` and `examples/replay.rs` of
    `hp3770-cli`) the lid's light never came on: bit 6 of `E946` always lights the flatbed lamp,
    bit 5 nothing visible. Matteo saw the lid's light on only during the HP driver's first
    attempts, before a power cycle, with the chip in the state left by `hp3770 init`.
  Found later the same evening, writing back the whole register block read while the light
  was on and halving the differences (witness: bit 4 of `E969`, 1 exactly when the lid's light
  is on): **the adapter's lamp lights with bit 5 of `E946` and bit 6 of `E965` together**,
  with neither alone. Bit 6 of `E946` stays the flatbed lamp; bit 7 of `E965`, the timer bit
  and `E955` play no part. Every start-up (hp3900's, the HP driver's, ours) clears bit 6 of
  `E965`: that is why the HP driver never lights the adapter on Windows 11 (perhaps the value
  is in its missing `hpsj3770.spf`). `lamp_transparency`, `hp3770 lamp on|off --tma`, the
  `tma_lamp_lit` state and the experimental `hp3770 tma-preview` (calibration with the flatbed
  lamp, then the adapter's lamp, 30 s, the whole glass) use it.
  The adapter's light on the glass (`tma-preview` at 200 dpi, Matteo's holder with a strip of
  negatives): 87.6-131.6 mm across, 76.2-171.6 mm down; at the top a slot with no film,
  about 88-132 x 76-84 mm (the place to calibrate the adapter on, as the white strip for the
  glass); below, the film window, about 97-122 x 94-171 mm, two 35 mm frames one above the
  other, the strip along the carriage's travel.
  **The first real negatives, the same night**: `hp3770 tma-scan --dpi 1200 --film negative`
  (adapter's lamp, 30 s, calibration on the slot, the adapter's area at 16 bits, 94 s) gave
  Matteo's two frames as positives, 2080 x 4507 pixels, natural colours. What it took:
  - the calibration of the adapter (`calibrate_transparency`): the carriage to the slot
    (`[CalibTransparent]` `WStripYPos=4130`), the adapter's lamp off for the black shading and
    on for the white, the parameters of `[CalibTransparent]`; the glass's white reference (245)
    instead of the section's (about 80): in 16 bits the reference falls at about half the scale,
    twice the headroom above it;
  - in `scan_imaging::film`: the base measured in the gaps between frames (`base_between`;
    the least dense film inside a frame can be light leaking past its edges), the gaps found
    against the film within 20 mm (the adapter's light is not even along the strip: its
    middle gets about 1.3 times the light of the slot), a positive of at most 2 decades.
  **Exposure for negatives** (the same night): the HP configuration has scan modes of their own
  for the adapter, `[ScanTAColorMode*]` (slides: the glass's line) and `[ScanNegColorMode*]`
  (negatives: a longer line, an exposure per colour). `scan_modes::NEGATIVE_MODES` take the
  second `CTPC` of each, three times the glass's line, every colour exposed for all of it:
  the gains come out at 11-16 (HP's own starting gains for negatives are 6/12/14), the slot
  neutral, the noise halved (slot 3.3-4.5 % to 1.5-1.9 %, the dark of the holder 42-65 % to
  20-30 %); 1200 dpi takes 153 s. Tried and left: the first `CTPC` (six times: gains at their
  minimum and the brightest channel saturated in the calibration's reads) and HP's exposure
  per colour (`EXPT`/`MEXPT`, kept in `NEGATIVE_EXPOSURES`: `MEXPT` how long, `EXPT` where it
  starts; with them the calibration's reads, lengthened by `set_multi_exposure`, and the scan
  did not expose red alike).
  **Colour balance**: the highlights of a negative are measured on the same pixels for the three
  channels (the densest 0.5 % by total density, each channel averaged over them), not on the
  densest of each channel on its own: the cyan cast of the light, neutral parts went away.
  `hp3770 develop` turns a saved scan (NAME-16bit.png) into its picture again in a second,
  without the scanner.
  **The adapter's light, measured with the holder empty** (9 November, `tma-scan
  --calibrate-at`): along the window it varies by a few percent only (red +4 %, blue +2 % over
  75 mm; the "1.3 times" seen before was saturation); the slot gets a third of the window's
  light, and `[CalibTransparent]`'s white, 81/83/70, is exactly 245 over those ratios: with it
  the window's clear light comes out at the full scale (with 245 on the slot it came out three
  times too bright, hence the gains at their maximum and red saturated on the base). The
  negatives' line is now twice the glass's. The calibration waits for the adapter's lamp to be
  steady (it gains about 18 % in its first 15 s): two scans in a row agree within 0.2 %. The
  final negatives at 1200 dpi: no channel saturated, natural colours, 114 s.
  **Corrected on 10 November 2026**: the area above, 76.2-171.6 mm down, is the window of the
  negatives' holder, not the end of the light: with the slides' holder the light reaches about
  188-190 mm and the second slide ends at 176.8 mm (phase 13, step 5.1).
  Still open: the last few percent of light along the strip (a correction measured once on the
  empty holder, if it shows); blue's signal on a negative's base is the lowest (HP gives each
  colour its own exposure for that; not understood yet); slides not tried yet (their mode would be the glass's, the gains at their
  maximum: perhaps the negatives' longer line for them too; **done since**, phase 13 step 5.2,
  with the negatives' longer line); the contract and the app (below).
- **Driver**: the adapter's lamp (done: bit 5 of `E946` and bit 6 of `E965`, see above), its
  calibration (no white strip under the lid: what the HP driver does instead comes from the
  captures), the area of the holder; `hp3900` (`ST_TA`, `ST_NEG` in `hp3900_rts8822.c`) is the
  map, as for the flatbed. `Capabilities::transparency` becomes true for the 3770 and
  `DeviceStatus::adapter` says whether it is plugged in.
- **Contract**: `ScanRequest` gets the source (glass, slide, negative) and `Capabilities` the
  frames of the holder, in the adapter's own coordinates.
- **Imaging** (in `scan-imaging`, it can start before the captures, on made-up samples):
  negatives done properly, on the linear 16-bit samples: the film base measured on the
  unexposed edge, the orange mask removed channel by channel, the inversion in density, then
  levels and grey balance per channel; for slides, the full dynamic range of the 16 bits. The
  frames found by themselves in the holder (`detect`), each one a page. Done on 8 November
  2026, on made-up samples: `scan_imaging::film`. `Negative::measure` takes the base (given,
  `base_in` on a rectangle, or `base_auto`: the least dense film) and per channel the density
  of the highlights, with one black for the three channels; `Negative::positive` gives linear
  16-bit light over the scene's range (gamma of the film 0.6), for the usual rendering with
  `film::TONE`. `Slide` stretches to the 16 bits with one black and one white. The holder,
  the lamp beside the film and the bare base are left out of the levels; the measures are
  taken once (preview) and applied as they are (full scan). `film::frames` finds the slides
  (an empty slot is left out) and the 35 mm frames of each strip (gaps searched every 38 mm,
  each frame anchored to the edge of the gap on its side; with no gap seen, frames centred).
  Still to check on real films, against the HP driver's images.
- **App**: the **Film** button becomes active (Slide, Negative) only with `transparency` and
  the adapter plugged in; e2e with a made-up scanner that has the adapter, one with it
  unplugged and one without it (capabilities rule).
  **Done for negatives on 9 November 2026**, slides left for later (none at hand):
  - contract: `ScanRequest::source` (`Source::Glass`, `Slide`, `Negative`), the area of a
    film within the adapter's area; `Capabilities::transparency` an `Option<Adapter>` (its
    area, the films it takes), `offers`, `bed_inches`, `bed_pixels`; `check` refuses a source
    the scanner does not take. No frames of the holder in the contract: the app finds them on
    the preview (`film::frames`), whatever the holder;
  - driver: the 3770 offers `Source::Negative` with the adapter's area of its profile,
    `DeviceStatus::adapter` is `tma_attached`, a film's area is moved to where the adapter
    lights the glass (the same window as `tma-scan`) and asks for the adapter's lamp, its
    calibration and the negatives' modes; with the cable unplugged the scan is refused;
  - imaging: `film::measure_strip` (frames, base between them, levels inside them) and
    `film::inside`, shared with `tma-scan` and `develop`; the samples stay the film's
    transmission in the contract, the app makes the positive (any backend benefits);
  - app: **Film** is Foto with `Source::Negative` (its own settings, 1200 dpi and 16 bits,
    saved as `film` in the preferences), so the preview, the frames as originals, the cut and
    the pages are Foto's; the preview is developed (`Negative` measured on it, kept for the
    full scans: the same picture), the frames found on the transmission, the pages drawn
    with `film::TONE`. The button shows with an adapter that takes negatives; with its cable
    unplugged a press asks the scanner again and says so. The e2e
    `films_need_the_adapter_and_its_cable`, `a_strip_of_negatives_becomes_a_picture_per_frame`
    with `FakeScanner::with_adapter` (a strip of two frames, orange base);
  - **the first try from the app ground the motor** (9 November 2026): the preview was at
    200 dpi, and the negatives' modes run the motor at constant speed from a standstill where
    the glass's modes at 300 and 200 dpi start it with a ramp (`SETTING_10`, `SETTING_11`);
    the negatives' line, only twice the glass's, asked a step rate the motor cannot start at.
    At 600 and 1200 dpi the glass's modes are at constant speed too, and the negatives' slower:
    the scans of 8 November were all at 1200. So `scan_modes::NEGATIVE_RESOLUTIONS` (600, 1200)
    is checked by `session::scan` before anything moves (the test
    `negatives_at_low_resolutions_are_refused_before_anything_moves`), the contract has
    `Adapter::resolutions` and `Capabilities::resolutions_for`, and the app scans films and
    their preview at those only (the preview at 600 dpi, brought to 200). 2400 dpi left out:
    never tried. Lower resolutions would need the HP configuration's longer lines (three to six
    times) or a ramp of our own;
  - **tried from the app on the real scanner** (9 November 2026, after the fix): Film at
    1200 dpi works, natural colours, a picture per frame. Committed and pushed as `74ac32b`,
    not released yet (CHANGELOG `[Unreleased]`).
  The driver of HP cannot scan with the adapter on Windows 11 (above): the comparison with its
  images in **Done when** cannot be made; the reference are our own scans.
- **Next** (where to start again), in order:
  1. ~~a preview at 600 dpi~~: **no preview for films** since 9 November 2026 (with Matteo:
     the adapter's area is small and full of frames, the scan finds them itself, a preview
     only added a lamp warm-up and the only resolution never tried); **a scan at 600 dpi
     tried by Matteo** the same day, it works. **The preview came back on 15 November 2026**
     (phase 13 step 9): with the Epson 4490's 2400 and 4800 dpi a whole strip is too large;
  2. **blue on a negative's base**, the lowest signal: try HP's exposure per colour again
     (`NEGATIVE_EXPOSURES`, how the chip uses `EXPT`/`MEXPT`) or a longer line for blue only;
     **still open**, now phase 13 step 4 (an exposure per channel, stalled without HP's
     captures);
  3. the **light along the strip**: a correction measured once on the empty holder, only if
     the few percent show in the pictures;
  4. ~~**slides**~~ · **done on 10 November 2026** through phase 13 step 5.2: `Source::Slide`
     in the 3770's `Adapter::sources` (`rts8822.rs`), the negatives' longer line for them,
     `Slide` and `measure_slides` in `film`, e2e with `FakeScanner::with_slides`, tried on the
     scanner;
  5. lower resolutions for films (300, 200 dpi) only with a motor ramp or HP's longer lines,
     and never without a guard in `session::scan` first;
  6. ~~a **release** with the Film mode~~ · **done**: v0.5.0 with the negatives and v0.6.0 with
     the slides (both 10 November 2026; `74ac32b` is in v0.5.0, `73b3047` in v0.6.0).
  Still open of this list: 2, 3 and 5.
- **Done on 9 November 2026, app only** (no new move of the scanner): without a preview the
  frames are found in the scan itself (`film::frames` on the scan brought to the preview's
  resolution), and **Separa** cuts the originals out of a photo scanned whole (glass or
  film), without scanning again. A page scanned whole keeps its area and, for films, the scan
  before it became the positive (`Whole`), so the originals found sit on the glass where the
  preview would put them and "Rifai" on each scans just that one. Three e2e on the made-up
  scanner; **tried by Matteo on the real scanner** the same evening (Film at 1200 dpi,
  without a preview, and "Separa i fotogrammi" on a strip scanned whole): it works.
- **Done when:** the adapter's start-up and calibration match the captures; a slide and a
  negative come out comparable to the HP driver's (or better for negatives); smoke tests on
  the real scanner with a slide in the holder.
- **Limits to say in the guide**: 1200 dpi real across the sensor, about 2 megapixels for a
  35 mm frame (fine for the screen and 10 × 15 prints); no infrared, so no automatic dust
  removal.
- **Later, for quality**: a scanner made for films. Researched on 8 November 2026: the HP
  G4010 / G4050 (Genesys GL843) work in SANE only on the glass, the adapter "not supported
  yet", so it would be reverse engineering again; better a Canon CanoScan 9000F / 9000F Mark II
  (SANE `pixma`, films and infrared complete) or an Epson Perfection V700 / V750 / V800 / V850
  (`epson2`); not the Epson V500 / V550 / V600 / V370, which need a closed Epson plugin. These
  are backends of phase 9.

## Phase 11 · moving things precisely from the keyboard · done

Added on 8 November 2026, at the user's request: in the **Output** pane the page must move on
the sheet in every direction, and be corrected one step at a time from the keyboard; the same
wherever something on the screen is moved with the mouse.

Where it stands today: in Output the page can be dragged only with "Fill the sheet" and only
when it overflows (`Layout::focus` chooses which part is kept); with "Actual size" and "Fit
to the sheet" it is always centred. Nothing moves with the arrows: `App::on_key` has no arrow keys.

- **Output, the page on the sheet**: it moves in every direction with every placement.
  `Layout::focus` also places a page smaller than the room inside the margin (0: left or top,
  0.5: centred, 1: right or bottom), so the default stays centred and the PDFs made so far
  come out identical (golden test). Dragged with the mouse (`sheet.rs`) and with the keys; the
  distance from the sheet's edges in mm next to the preview, typed by hand too, and a
  "Centre" button. "Apply to all pages" carries the position too.
- **The same keys everywhere**, on whatever is active in the pane shown:
  - arrows: move by 1 mm; **Shift** + arrows: 10 mm; **Alt** + arrows: 0.1 mm (or one pixel
    of the scan, whichever is larger);
  - **Ctrl** + arrows (Cmd on macOS): the size instead of the position (right and bottom
    edge), with the same steps;
  - **Home**: centred; a series of presses within a second is one undo step, like a slider
    dragged;
  - never while a text field has the key (as all shortcuts already); at the edge it stops.
- **Other places where a selection moves** (found in the code):
  1. **Scan, the area on the preview** (`viewer::Selection`, eight handles): position and size
     with the keys, kept on the glass; also X, Y, width and height in mm, typed.
  2. **Editor, the crop** (the same `Selection`): position and size with the keys; with fixed
     proportions the size keeps them.
  3. **Editor, straighten**: `[` and `]` turn by 0.1°, with Shift by 1° (the buttons −1°,
     −0.1°, +0.1°, +1° already exist, the keys do not).
  4. **Adjust, the curve** (`curve.rs`): the point clicked last moves with the arrows by one
     level (Shift: 10); Tab goes to the next point, Delete takes it away (not the two ends).
  5. **The strip of pages**: Alt + left and right move the page shown before or after the
     others (`Message::PageMove` already exists for the buttons); Page Up and Page Down show
     the previous and next page.
  6. **The zoomed view**: with nothing to move, the arrows pan the image.
- **Done when:** an e2e for each place (1 mm, 10 mm, 0.1 mm, at the edge it stops, one undo
  step for a series, nothing moves while typing in a field); the PDF golden test unchanged for
  centred pages; the keys in the usage guide's table of shortcuts, in Italian and English.
- Low risk, no hardware: it can come before phase 10.
- **How it was done** (8 November 2026): the keys are read in one place, `keys.rs`
  (`keys::read`, `App::on_move`), which chooses what moves in this order: the point of the
  curve chosen, the crop in the editor, the page on its sheet, the area on the preview, the
  zoomed view. `Layout::focus` now places a page smaller than the room too, and `Placed`
  gives `room`, `offset_mm` and `focus_at`; the golden test `pages_made_before_come_out_the_same`
  checks the PDF numbers of 0.3.1 for centred pages. Two changes from the list above:
  - the strip moves the page with **Alt + Page Up / Page Down**, not Alt + left and right,
    which are already the 0.1 mm step;
  - one step of Ctrl+Z for a series of presses where the mouse has Ctrl+Z (curve, crop,
    straighten); the area on the preview and the page on its sheet have none, with the mouse
    nor with the keys, as before.

## Stripes across the scans of the glass · 9 November 2026 · done

Matteo saw faint horizontal lines on scans of a white sheet, mostly at 300 dpi, and once three
coloured lines (blue, green, red) the same distance apart.

- **Measured** on a white sheet with `hp3770 scan --linear --sharpen 0`, as the mean of each
  line: a pattern of lighter and darker lines that repeats every 5 lines at 200, 600 and 1200
  dpi and every 10 at 300 dpi (3.33 lines a period), 3.2 % at 200 dpi, 2.4 % at 300, 0.2-0.6 %
  at 600, 0.6 % at 1200 (row to row). **The HP driver's own scan at 300 dpi has the same
  period**: it is the scanner, not our driver. It drifts slowly (5.02 lines at 200 dpi, 4.993
  at 1200); from 1200 dpi the even and odd pixels see it on different lines. A white sheet
  does not show the carriage's position, so it comes from the light (the lamp is driven in
  PWM, as the HP driver sets it) or the electronics, not from uneven steps.
- **Taken away on the host** (`scan_imaging::stripes`): each line against the 2 × `repeat`
  lines around it as the median over the columns, then the median over the lines at the same
  place of the pattern within 30 repetitions, for even and odd columns and each colour; only
  what repeats is taken away. On the white sheets 2.4 % → 0.2 % at 300 dpi, 3.2 % → 0.4 % at
  200, 0.6 % → 0.15 % at 1200. The periods are in `Capabilities::stripes` (none at 2400 dpi,
  not measured). In the app a tick, off by default; another tick scans at twice the
  resolution and halves (300 from 600), also off. Neither applies to films.
- **For any scanner**: the filter knows nothing of the 3770 but the repeat at each
  resolution. `stripes::measure` finds it on a scan of something uniform: for each length
  from 2 to 16 lines it works out what the filter would take away, and keeps the shortest
  that takes away nearly as much as the best (on the white sheets: 5, 10, 5 and 5, as
  measured by hand; `hp3770 stripes FILE.png`). In the app **Misura le righe** (Preferenze)
  scans a 25 mm strip low on the glass at each resolution up to 1200 dpi (the glass's usual
  modes, 16 bits where offered) and keeps the result for that scanner in the preferences, in
  place of `Capabilities::stripes`; an e2e on the made-up scanner with stripes switched on.
  **Tried by Matteo on the real scanner** the same evening (the measurement with a white
  sheet, and a scan with the tick): it works.
- **The three coloured lines** are one moment of the scan seen by the three rows of the
  sensor (12 lines apart at 600 dpi): the carriage stopping and starting, or a flicker of the
  lamp. Not seen again in four white sheets; the chip never held more than 0.1 MB waiting,
  so the host kept up. Open: catch it again (which resolution, depth, how long the scan).
- **Right after a power cycle** (the scanner had stopped answering on a USB hub), the
  calibration at 300 dpi failed twice with "offsets still too dark after 20 checks", then
  worked after one scan at 600 dpi, with this version and with the one of 8 November alike.
  Not reproduced since; open.
- **On films too** (10 November 2026, commit `a54013a`): the 3770 leaves the same pattern on
  negatives (every 4.96-4.99 lines at 1200 dpi, about 1 level of 255 in a sky and 2-3.4 in the
  light parts of the picture). **Togli le righe dello scanner** is offered in Pellicola and
  works on the negative's linear samples before it is measured and inverted: on a real strip
  the pattern came out 3-4 times fainter, the colours unchanged. `hp3770 develop --destripe`
  does the same on a saved scan. Declared per source since (phase 13, point 2); at 600 dpi
  measured the same day on a film: every 5.0 lines, 2-3 levels in a blue sky.

## Phase 12 · the Epson Perfection 4490 Photo · the scanner arrived on 11 November 2026

Added on 9 November 2026: Matteo is getting an Epson Perfection 4490 Photo, the first scanner
besides the HP 3770 (phases 8–9 were postponed only for lack of hardware). Researched before
it arrives, on the web only: nothing below has been tried on the device yet.

### What is known, with sources

- **Identity**: USB id `04b8:0119` (`lsusb` in user reports); in Japan the GT-X750; Epson
  product code B11B176011. Epson's open `iscan` sources know it as GT-X750, firmware code
  `0x54` (`backend/get-infofile.c`) and treat product `0x0119` as a special case
  (`backend/epkowa.c`).
- **It does not speak ESC/I**. Olaf Meeuwissen of EPSON AVASYS on sane-devel, 25 and 29 August
  2005: it "does not speak the ESC/I protocol", works neither with the `epson` nor with the
  `snapscan` backend, and its support in `epkowa` "will require a closed source library". The
  SANE `epson`/`epson2` backends do not support it (true still today: sane-backends 1.0.18
  moved it off the unsupported list only as "supported by epkowa").
- **How Epson drives it on Linux** (the open `iscan` sources, e.g. the mirror
  `github.com/ip1981/iscan`): the `epkowa` backend speaks ESC/I to an "Interpreter", the closed
  library `libesint54` of the plugin `iscan-plugin-gt-x750`, through the small API of
  `backend/epkowa_ip_api.h` ("for scanners that don't speak EPSON ESC/I natively"): `int_init`
  / `int_init_with_ctrl` get USB read, write and control callbacks, `int_read`/`int_write`
  carry ESC/I, `function_s_0`/`function_s_1` build and apply a shading table on the host.
  The firmware file's path is passed to the library (`interpreter usb 0x04b8 0x0119
  <plugin> <firmware>` in `epkowa.conf`), and **the library uploads it**: no open code knows
  how the upload works nor the native protocol after it. The header carries a GPL exception
  for modules that talk to `epkowa` only through that interface.
- **No open implementation** of the native protocol was found, for this model or its family
  (the esfw firmwares of `iscan-firmware`: Perfection 2480/2580, 3170, 3490/3590, 4180, 4490,
  V10/V100, V200, V30/V300, V350, V500; note that SANE `snapscan` does drive the 2480/2580 and
  3490/3590 with their esfw41/esfw52 firmware, but Meeuwissen says the 4490 is not like them).
  VueScan drives it with its own closed driver. The controller chip is not known: the
  service manual (SESC05004, main board B81334) or a look at the board would say.
- **Hardware** (Epson's user guide, the PhotoReview review): flatbed, "MatrixCCD" colour CCD
  (six lines, two per colour), 4800 dpi across, 9600 dpi along with "Micro Step Drive"; 48
  bits in colour (16 per channel), 16 in grey; glass 216 × 297 mm (8.5 × 11.7 in); in the lid
  a **transparency unit with its own moving lamp** (it has its own transport lock, besides the
  carriage's at the back) and holders for two 35 mm strips of six frames, four mounted
  slides or one 6 × 12 cm medium-format frame, film area 68 × 236 mm (2.7 × 9.3 in); **Digital
  ICE** for film with an infrared light (the review: an infrared LED), so a fourth channel the
  host uses for dust; four buttons (Start, Scan to PDF, Scan to E-mail, Copy); a port for an
  optional document feeder (B12B813391); about 3.8 ms a line at 600 dpi, 16.9 ms at 4800.
  The lamp's kind and warm-up are not stated.
- **Windows**: Epson's US support page for B11B176011 lists "Scanner Driver and EPSON Scan
  Utility v3.24" (`epson13552.exe`, 18 November 2009, 10.8 MB, TWAIN) as compatible with
  Windows 11, plus Epson Event Manager 3.11.77 for the buttons; the product is "End of
  Support". **Installed by Matteo on 9 November 2026** (scanner not plugged in yet): it is
  `es54.inf` (published as `oem344.inf`, 3.2.4.1 of 21 October 2009, signed by SEIKO EPSON,
  "Legacy"), for `USB\VID_04B8&PID_0119`. On 64-bit Windows it is a **WIA** driver
  (`esxwia54.dll`, `Capabilities=0x13`) over **`usbscan.sys`** (`NTMPDriver`), like HP's for
  the 3770: so `wia1scan` can drive it, and our backend can open the scanner through
  `usbscan.sys` with Epson's driver installed, as `hp3770-usb` does. The interpreter is there
  too, `esint54.dll` (32-bit, in `SysWOW64`, the Windows twin of `libesint54`), with the TWAIN
  source in `TWAIN_32\ESCNDV`. The firmware is **`C:\Windows\System32\esfw54.bin`**, 65,793
  bytes, SHA-256 `B01384A9EC89F62183054A2826789498C7666C646BE7E5CFDE16CEDF012EF0F9` (the same
  in the DriverStore); this copy is under Epson's EULA.

### The firmware and its licence

The scanner needs `esfw54.bin` sent at every power-on, before anything else.

- Fedora/EPEL ship it in **`iscan-firmware`** (`/usr/lib/firmware/epson/esfw54.bin`), taken
  unmodified from Epson's `iscan-plugin-gt-x750-2.1.2-1` (the Fedora spec,
  `src.fedoraproject.org/rpms/iscan-firmware`). That plugin carries the **AVASYS Public
  License** (2008-04-01; `AVASYSPL.en.txt`, which the AUR package installs from the plugin's
  own documentation). Fedora's licence tag is `LicenseRef-Fedora-Firmware` (firmware that may be
  redistributed, not modified; fedora-license-data issues 608 and 610); the package also holds
  `LICENSE.EPSON` (SEIKO EPSON Software License Agreement, 2001) and `EAPL` for the other
  plugins.
- What the AVASYS Public License says: verbatim copies may be reproduced and distributed freely
  in any medium, as long as the recipients get the licence (clause 2); modified versions too,
  under the same licence (3); the parts given only as executable or object code are
  "proprietary and confidential" (4) and may not be reverse engineered, decompiled,
  disassembled "nor otherwise" analysed, except where the LGPL requires it (5); no warranty, no
  liability (6, 7). No restriction to Epson hardware, no termination, no governing law.
- Not the same for every copy: Epson's newer `iscan-gt-x750-bundle` and Epson Scan for Windows
  come under the **EPSON END USER SOFTWARE LICENSE AGREEMENT** (the text is in `iscan`'s
  `non-free/COPYING.EPSON.en.txt`): use only with the Epson hardware, no distribution to third
  parties, no reverse engineering. So a copy taken from the Windows driver must never be
  shipped by us.
- **With our GPL-2.0-or-later code**: the firmware is not linked into our program, it is data
  sent to the device, like the files of `linux-firmware` next to the kernel; a separate,
  unmodified file with its own licence next to it is "mere aggregation" for the GPL.
- **What we do** (decided with Matteo on 9 November 2026: **the firmware goes inside the
  program**, the scanner is twenty years old and supported by nothing else):
  1. **embedded in the binary** at build time: the backend crate's `build.rs` reads the file
     named by `VETROSCAN_ESFW54` (on Matteo's PC `C:\Windows\System32\esfw54.bin`), checks its
     SHA-256 (`B01384A9…EF0F9`, the Windows driver's copy) and includes it
     (`include_bytes!`); the bytes are never changed;
  2. **never in the git repository while it was public** (until step 4, done on 10 November
     2026): the build took it from the disk of whoever builds,
     so no clone and no public CI build carries the copy under Epson's EULA. A build without it
     looks for the file at run time (a path in the Preferences, `System32\esfw54.bin`,
     `/usr/lib/firmware/epson/esfw54.bin` of `iscan-firmware`), with the same SHA-256 check;
     without any, the scanner is listed with a note saying what is missing and nothing is sent
     to it;
  3. **public packages with the firmware inside**, if ever: only with the AVASYS-licensed copy
     (to be fetched from Fedora's `iscan-plugin-gt-x750-2.1.2-1` and compared with the
     Windows one), `AVASYSPL.en.txt` shipped with the package and named in the README's
     licences, and a note of ours in the licence allowing that blob inside the GPL program
     (Matteo is the copyright holder). Builds kept by Matteo alone need none of this: the
     licences bind who distributes.
  4. **Done on 10 November 2026: the repository is private, and the firmware is in it.** Since
     `MatteoDM/scanner` is private, the original
     `esfw54.bin` of Epson's Windows driver (SHA-256 `B01384A9…EF0F9`) is committed in the
     repository, versioned, and embedded in the executable of **every** build, CI's included:
     - first make the repository private on GitHub and check it (`gh repo view --json
       visibility`); the releases go private with it;
     - put the file in the crate (`crates/epson-gt-x750/firmware/esfw54.bin`), take
       `esfw54.bin` / `ESFW54.BIN` out of `.gitignore`;
     - `build.rs` embeds that copy by default (same SHA-256 check; `VETROSCAN_ESFW54` stays only
       to choose another copy, the AVASYS one for a try); the run-time search stays for builds
       without it;
     - this section, "Where it stands" and the CHANGELOG say so.
     The Windows copy's EULA forbids giving it to third parties: the private releases are for
     Matteo; packages for anyone else still need the AVASYS copy (point 3).
- **The captures are not reverse engineering of the firmware**: the file is never disassembled
  nor examined, only sent; what is studied is what passes on the USB cable while Epson's own
  driver is used normally, as for the HP 3770. In the EU the lawful user of a program may
  "observe, study or test" how it works while running it (Directive 2009/24/EC, art. 5(3);
  Italy, law 633/1941 art. 64-ter), and a contract cannot take that away (art. 8). Not legal
  advice: if the project ever ships the file or something derived from the captures goes
  beyond the protocol, read the licences again.
- **If the captures are not enough**, looking inside Epson's interpreter (`esint54.dll`) for the
  protocol is the fallback, under the EU's **decompilation for interoperability** (Directive
  2009/24/EC, art. 6; Italy, art. 64-quater; art. 8: a contract cannot forbid it). Its
  conditions, which Matteo confirmed on 9 November 2026: done by a lawful user (his scanner,
  Epson's driver installed from Epson's site); the information is not available elsewhere (no
  published protocol, no open driver, the model unsupported since 2009); only the parts needed
  for our independent backend to talk to the scanner; the information used only for that,
  never given to others for other ends nor used to make a program like Epson's. Into our code
  goes only the protocol (commands, values, order), written by us: no code, tables or text
  copied from Epson's library. The captures were meant to come first, but with no scanner
  yet Matteo chose to start reading Epson's driver now (9 November 2026).
- **Read so far** (notes in `captures/epson4490/`, outside the repository, to be confirmed by
  the first capture): Epson's WIA driver `esxwia54.dll` talks to the scanner only with bulk
  reads and writes through `usbscan.sys` (no control transfers) and uploads the firmware
  itself: `1B 03` asks the state (bit 4 of the first byte of the 2-byte answer: firmware
  running); otherwise `1B 06` (ACK `06`), the length in 4 bytes, the 65,536 bytes after the
  file's 256-byte header (`EDLA`) in writes of at most 65,520, an 8-bit checksum, ACK, then
  `1B 16` and `80`, each answered by `06`. The interpreter (`esxuin54.dll`, ESC/I to the
  native commands) has no I/O of its own: `INTInit(read, write)` gets two callbacks. First
  pass the same night: the native protocol is two-byte commands `1B xx` / `1E xx` with data
  blocks and an ACK `06`; about forty of them catalogued with their sizes (`1E 47` reads the
  image, `1E 84` writes tables of up to 64 KB, `1E 42` takes 18 bytes of set-up, ...); ESC/I
  arrives in a state machine, `ESC G` runs the native scan sequence. Meanings still to be
  worked out (`captures/epson4490/02-interpreter.md`). The scan sequence for `ESC G` is in
  order (calibration and tables first, then the set-up blocks `1E 11`, `21`, `41`, `01`, `42`,
  `43`, then `1E 48` and `1E 47` for the data); `1E 41` (start line, end line, a step code
  chosen by speed) is the first candidate for the carriage's move.
- **The Linux plugin** (`iscan-plugin-gt-x750-2.1.2-1`, downloaded from Fedora on 9 November
  2026, `captures/epson4490/03-linux-plugin.md`): its function names are deliberately
  obfuscated (`esint54_SYMBOL_157`), only argument types and a few struct names survive. Its
  `esfw54.bin` (the AVASYS copy) is **a different build** from the Windows driver's (same size
  and header, SHA-256 `9A4171AB…`, about 49,800 bytes differ): the backend must accept both,
  and both must be tried on the scanner before public packages carry the AVASYS one.
- **What moves the carriage** (third pass, `captures/epson4490/04-carriage.md`): a move is a
  ramp loaded with `1E 84`, its parameters with `1E 01` (12 bytes: steps at full speed, ramp
  length, direction bit `0x20`) and `1E 05` to start it, then `1B 03` polled while bit `0x40`
  is set; `1E F4` is a simpler move; `1E 47` / `1E 48` start a scan. `1E 86` reads two
  positions (the carriage's and, likely, the transparency unit's), `1E 25` selects what
  moves, and Epson parks the carriage at 1994. These are the commands the backend guards
  first: never sent before a capture shows the same sequence, targets kept within Epson's
  own range (0–7106).

### Two ways to drive it

1. **Through Epson's own drivers**, the backends of phase 9 with a real scanner to try them on:
   SANE (Linux, with Epson's `iscan` and the gt-x750 plugin installed: `epkowa` does the
   firmware and the protocol) and WIA (Windows, if Epson Scan 3.24 offers WIA). Low risk, no
   protocol to learn, but only what those drivers give (likely no infrared channel, perhaps no
   16 bits through WIA, no lamp or buttons through SANE). Linux x86 and x86-64 only for SANE:
   Epson's plugin exists for nothing else.
2. **A native backend** (a new crate, say `epson-gt-x750`, next to `hp3770-core`), from USB
   captures of Epson Scan on Windows, the method of the HP 3770: the firmware upload, the
   commands after it, calibration, the transparency unit and its lamp, the infrared channel,
   the buttons. Full control, on every system, but months of work and the risk of a motor
   moved wrongly.

**The goal is the second** (Matteo, 9 November 2026): the 4490 **plug and play like the 3770**,
with our own driver on Windows, Linux and macOS. So no work goes into the first way for the
4490: the WIA and SANE backends stay in phase 9, for any scanner, and Epson's own software is
the fallback for Matteo if the native backend stops somewhere. A third way, `dlopen` of
Epson's `libesint54` from Rust speaking ESC/I to it, is left out: Linux only.

**Plug and play, as for the 3770** (`hp3770-usb`: `usbscan.sys` first, then `nusb`):

- **Windows**: Epson's driver binds `usbscan.sys` (above), so the backend opens the scanner
  through it, next to Epson Scan, as the 3770 next to HP's driver; without Epson's driver, WinUSB
  with Zadig as in `docs/install.md`. The firmware is inside the program, so nothing else is
  needed;
- **Linux**: `nusb`, a udev rule for `04b8:0119` in the packages (as `70-hp3770.rules`), the
  firmware inside the program: no `iscan`, no SANE;
- **macOS**: `nusb`, as the 3770;
- **at every start**: the backend asks the scanner whether the firmware is already running (on
  Windows Epson's driver may have sent it), sends it if not, waits for the scanner to come back
  (if it re-enumerates, the `Registry` sees it again), and only then reports it ready. The user
  sees only "connecting…".

**Milestones**, each one usable on its own and none moving anything before it is understood:

1. **Talking**: the firmware upload and the status (ready, busy, lamp, adapter, buttons),
   replayed from the captures; `probe` and `status` of a CLI command of the backend. Nothing
   moves.
2. **One scan of the glass** at one resolution (300 dpi, 8 bits), the sequence of the captures,
   with the carriage's travel and return checked against them; every other resolution refused.
3. **The glass complete**: our calibration (the shading Epson does on the host,
   `function_s_0`/`_s_1`), every native resolution tried and added to the guard one at a time,
   16 bits, the `Tone`, the stripes measured; the app scans with it (e2e on a made-up 4490).
4. **Buttons and lamp** in the `Capabilities` and the app.
5. **Films**: the transparency unit's lamp and carriage, its calibration, strips and slides
   (`film`, as on the 3770), medium format.
6. **Digital ICE**: the infrared channel in the contract and the dust removal in
   `scan-imaging`.
7. **Release**: packages with the udev rule, the install guide, CHANGELOG.

### Where it stands (9 November 2026, the scanner not arrived yet)

*History, kept as it was written before the scanner arrived: where the 4490 stands now is in
"What is left, in order" below.*

**Written without the scanner** (branch `epson-4490-milestone1`, not merged into `main` nor
pushed): the crate **`crates/epson-gt-x750`**, in the app's `Registry` after the HP 3770's.
- It finds the scanner by `04b8:0119` (`usbscan.sys` first on Windows, then `nusb`, bulk only);
  without it, it lists nothing and sends nothing.
- Opening it reads the state (`1B 03`) and sends the firmware if bit `0x10` is clear (when that
  is allowed: below), exactly as read from `esxwia54.dll`; `status()` reads `1B 03` again;
  `scan()` answers "not implemented yet"; `poll_buttons()` answers none.
- **A guard on every write** (`guard.rs`): only `1B 03`, `1E 86` and the firmware upload go out;
  every other command, and the moves of `04-carriage.md` by name (`1E 01`, `05`, `25`, `47`,
  `48`, `84`, `F4`), are refused before USB. A command is added there only after a capture
  shows it.
- The firmware: `build.rs` embeds the repository's copy, `firmware/esfw54.bin` (or the one
  `VETROSCAN_ESFW54` names), with one of the two known SHA-256 (any other file stops the
  build); a build without it looks for it at run time
  (`epson_firmware` in `impostazioni.json`, else `esfw54.bin` in the app's settings folder, the
  same place on every system; then System32 on Windows, the `iscan` places on Linux; on macOS
  no driver place is known), with the same check.
- Tried on: Windows (tests, clippy, the command); Linux in WSL (tests, clippy, the command);
  macOS in the CI only (run 42716373743 on `epson-4490-milestone1`, 10 November 2026: the
  tests, the 4490's e2e included, and the macOS packages built; no Mac here). With the
  scanner, so far on no system.
- Provisional `Capabilities`: glass 8.5 × 11.7 in, 8/16 bits, colour and grey, the four buttons,
  no lamp, no adapter.
- 24 unit tests with a mock transport (the upload's exact bytes, any answer but `06` refused, no
  upload when the firmware runs, every move refused); the real System32 copy checked (sum `D9`,
  last byte `F8`).
- The capture tools (`captures/tools`, outside the repository) take `-Device` (a name or
  `VID:PID`) on any USBPcap root hub, and `Start-CapNew`/`Stop-CapNew` capture the devices that
  connect from then on (the power-on upload).

- **The upload waited for the capture**: `UPLOAD_CONFIRMED` was `false`, so opening the 4490
  sent nothing but `1B 03`, and the firmware went out only with `VETROSCAN_EPSON_UPLOAD=1` or
  `epson4490 upload`. **Confirmed on 11 November 2026** (below, "First captures"):
  `UPLOAD_CONFIRMED` is `true` and the variable is gone, so opening a 4490 whose firmware is
  not running sends it.
- A command **`epson4490`** (a binary of the crate, not in the packages): `list`, `firmware`
  (which copy would be sent, no scanner needed), `probe` (list, firmware, state; nothing sent
  but `1B 03`), `upload [--firmware FILE]`.
- A made-up 4490 in the e2e (`FakeScanner::with(epson_gt_x750::capabilities())`): the app shows
  its four buttons and 16 bits, no lamp, no film.
- The udev rule `packaging/linux/70-epson-4490.rules` in the deb, rpm and AUR packages, and the
  post-install trigger for a scanner already plugged in.

**Still open in milestone 1**:
- ~~the power-on capture confirms the upload's bytes~~ · **done on 11 November 2026** (below);
- our own upload tried on the scanner, with both copies (Windows driver and AVASYS,
  `epson4490 upload --firmware`): on Windows Epson's driver sends the firmware first, so this
  needs the scanner without it (Linux, or WinUSB);
- what `1B 03`'s second byte, the lamp and the adapter look like (the status shows only
  ready/busy); the buttons · **done on 12 November 2026** (below); the lamp · **done on 12
  November 2026** (step 6 below). Our own upload is still open: step 12 below.

### First captures (11 November 2026)

Notes and captures in `captures/epson4490` (`05-catture.md`, outside the repository). Both
transport locks released before switching it on. Windows 11 bound Epson's `es54.inf` 3.2.4.1
on `usbscan` by itself.
- **Descriptors**: USB 2.0, class `ff/ff/ff`, `bcdDevice` 1.00, self-powered, one interface
  with two bulk endpoints, `0x81` IN and `0x02` OUT, 512 bytes; no serial number.
- **The firmware upload**: Epson's driver sends it **at plug-in** (17 ms after the scanner
  appears, no program opening it): `1B 03` → `28 00` (bit `0x10` clear), `1B 06` → `06`,
  `01 00 01 00`, the file after its 256-byte header in writes of 65,520 + 16 bytes, `D9` (the
  8-bit sum of those bytes, in place of the file's last byte `F8`) → `06`, `1B 16` → `06`,
  `80` → `06`. Byte for byte what `protocol::upload_firmware` sends (12 bytes of the first write
  were cut by the capture's length, the sum covers them). The scanner **stays on the bus**,
  and answers its next command only after about 8.8 s (it sets itself up; our timeout is 30 s).
- **At rest, Windows' image service polls `1E 85` every second**: the **buttons**, one byte,
  `00` none, `01` Start, `02` Scan to PDF, `03` Scan to E-mail, `04` Copy (pressed one by one by
  Matteo; each read for about 2 s). Epson's driver key declares the scanner needs polling
  (`Capabilities` 0x13), so `stisvc` asks through the same `usbscan` our backend uses: our
  exchanges could fall between its. **During a scan it does not poll**: the device's STI lock.
  Tried on the scanner (12 November 2026): a program holding the lock (`StiCreateInstanceW`,
  `IStillImageW::CreateDevice`, `IStiDevice::LockDevice`) stops the polling at once, and it starts
  again when released. So **the backend holds the STI lock while it has the 4490 open through
  `usbscan`** (`sti.rs`; nothing on Linux and macOS, where no service polls it), and reads the
  buttons itself (`poll_buttons`, `1E 85`, let through by the guard; `epson4490 buttons`). Checked
  with a capture: only our `1E 85` every 200 ms while it held the lock, Windows' polls before and
  after; the four buttons read right. While VetroScan has the scanner open, Epson Scan cannot
  use it and Windows does not see its buttons; a lock another program holds (Epson Scan
  scanning) makes opening fail after 5 s as busy.
- **The first scan of ours** (12 November 2026): `epson4490 replay SCRIPT OUT --move` sends
  the bytes of a script made from a capture (`captures/epson4490/tools/script.py`), compares every
  answer with the capture's and stops at the first that differs (the status polls wait for the
  captured answer; four read-only answers may differ). The first try stopped by itself before the
  lamp or the carriage: `1E A1` is the lamp lit (`01` glass, `02` lid), still the lid's after the
  films; once a WIA scan had switched it back (`1E 77`, `1E A2 01`), the replay of the 300 dpi
  scan ran whole in 14 s and gave the same photo, colours aligned, but darker and warmer than
  Epson's driver: the shading table and `1E 42` sent were the capture's, not computed from that
  day's calibration. **The calibration computed by us** (same night): the shading table is, per
  column and channel, a white byte `floor((white − K) / 2^lw)` and a dark byte
  `floor((dark − dark_min) / 2^ld)`, K and the minimum dark in `1E 42`, the two scales in byte 15
  of `1E 41`, and the black pixels' means in the `1E 42` before the white calibration (notes in
  `captures/epson4490/06-formato-dati.md`; dark byte, minimum dark and scales exact on six
  captures, the white byte about 1 unit off because the driver's way of leaving out dusty lines
  is not known). `replay` computes them from its own calibration reads (`C`, `X41`, `X42`,
  `X42B`, `XT` steps): the image came out as Epson's driver's of the same photo, at the same
  moment (means equal, WIA = 0.994 × ours + 0.4 per channel, 2 levels apart on average, the
  noise between two scans). Next: switching the lamp as the driver does, then computing the
  parameters (`1E 57`, `1E 41`, the motor's) for any area and resolution (both **done since**:
  the set-up and the native scan below, the lamp in step 6).
- **The set-up computed from the request** (12 November 2026, night): every set-up block of a
  glass scan follows from the request by rules read in Epson's interpreter (`esxuin54.dll`,
  addresses and derivation in `captures/epson4490/06-formato-dati.md`, "Il set-up calcolato
  dalla richiesta"): the columns at the sensor's base resolution from x, the width and the x
  origin's correction the scanner stores (bytes 2-3 of its `1E 87` answer, -93 on Matteo's), the
  line time, the motor's 2400 steps per inch, the read's speed and ramp, the start move and the
  lead, the lines per block. Checked on 19 glass captures (WIA at 100-1200 dpi, Epson Scan at 600
  and 2400 dpi, 16 bits): every block the same. A script generated for an area never captured
  (300 dpi, 5 × 5 cm at 40/60 mm), replayed on the scanner, gave WIA's image of the same area.
- **The native glass scan** (`scan.rs`, `setup.rs`; 12 November 2026): the backend computes the
  whole sequence from a request (resolution, area, 8/16 bits, colour) instead of replaying a
  script: WIA's opening, gamma and tone tables, the request, the lamp, `1E 87` (the x correction
  read, not a constant), the line time, the black pixels, the white and the dark with the
  shading computed from them, at 1200 dpi `1E 8B`/`1E 83`, the start move, the read, the
  trailers, the colours realigned (green dpi/50 lines below blue, red twice: exactly WIA's image
  from the same data on captures 03, 04, 30) and cropped. The two fixed ramps of the moves are
  **computed, not copied**: they follow the read ramps' law (400 steps/s up to 10⁶/160 and
  10⁶/250 steps/s, k = 2n − 3), the first value 2500 and the last one less than the full speed's,
  equal to the interpreter's tables and to what the scanner received. WIA's gamma table is
  `round(255 (i/255)^(1/1.8))`; its 8 KB tone table (`1E 84` type 02) has no formula found and is
  carried as captured on the cable (`tables/wia_tone.bin`). Tests: the blocks of the 18
  captures rebuilt (`testdata/`, the captures stripped of image data and long tables), the whole
  scan run against eleven WIA captures write by write, the realigning on made-up lines.
  - **The guard** lets a scan out only inside what the captures cover: glass, colour, 100, 200,
    300, 400, 600 and 1200 dpi at 8 bits, 600 at 16 (opened to every resolution at 16 bits the
    same night, step 8 below); the area inside the glass; the line times each resolution was
    captured with; start moves of the kinds seen (the 256-value ramp and "no move" never
    happen on the glass at these resolutions, refused anyway); no read further than capture 07's
    (29,587 steps); an x correction within ±300. Then only the plan's own blocks go out. Refused:
    800 (WIA asks 1200), 900, 2400, 4800 dpi, black and white, films, Epson Scan's draft mode.
  - At 1200 dpi the scanner answers `1E 8B` with `06` once Epson's driver has loaded a table it
    computes (`1E 84` type 07, seen once, capture 05: the scanner answered `15` and the driver
    read 245,824 bytes with `1E 83` first); how it is computed is not known, so with `15` the scan
    stops before the tone tables and the start move. **Computed since** (step 9 below,
    `table07.rs`).
  - `epson4490 scan DPI X Y W H OUT.png [--bits 16] [--raw FILE] --move` (millimetres); without
    `--move` it prints the plan and what the guard says. The application's `scan()` still
    answers "not implemented": its `Capabilities` (150-4800 dpi, grey) are not yet those of the
    guard. **Done since** (step 4 below: the app scans with it).
  - **Tried on the scanner** (12 November 2026, Matteo present): 300 dpi, 5 × 5 cm at 40/60 mm,
    the area of WIA's capture 30. Geometry identical to WIA's (offset 0/0). The first scan, with
    the scanner idle for hours, came out 6-8 levels lighter (a cold lamp: WIA's own capture 30, made
    right after the lamp came on, was 4-5 lighter than with a warm one); made again at once, the
    same means as WIA's (67.1 / 55.3 / 53.1 against 67.7 / 55.8 / 53.8; WIA = 0.997-0.998 × ours +
    0.6-0.9), 2.75 levels apart on average.
  - **Next, planned for 13 November 2026: the cold lamp, measured.** The scan waits on bit
    `0x01` of `1B 03` (the lamp warming) but did not wait that first time; every wait longer
    than one poll now goes to the log with the states seen. Ten identical scans (300 dpi,
    5 × 5 cm at 40/60 mm) at 0, 20, 40, 60, 90, 120, 180, 300, 480 and 720 s from the first,
    starting with the lamp switched off by itself (or after hours idle), about 12.5 minutes with
    Matteo present: `captures/tools/lamp_series_epson.sh`, outside the repository (steps and
    what to look at in `captures/epson4490/05-catture.md`, "Da fare"). For each: the white
    calibration's K (the lamp's light then), the image's means against the last scan's and WIA's,
    the `1B 03` states after `1E A2 01`. It says how long to wait, and whether the scanner tells
    or the backend must decide (a time, or K no longer rising).
  - **Then, the application's scan** (`Gt4490::scan` on `scan::scan_glass`), after the lamp:
    its `Capabilities` made the guard's (100, 200, 300, 400, 600, 1200 dpi; grey made by the
    application from colour, the scanner always sends RGB); 16 bits only at 600 dpi, which the
    contract cannot say (`depths` is not tied to the resolution): declare 8 bits only for now, or
    a new field; the contract wants linear samples while the scan loads WIA's curves (gamma 1/1.8
    and the tone table): linear tables (never sent, harmless to the mechanics, the image to be
    measured) or WIA's curve declared in `Tone`, measured first; the progress; the e2e with a
    made-up 4490.
- **More captures and the data format**: the glass through WIA (150-1200 dpi, colour, grey,
  black and white, a cancelled scan), Epson Scan (2400 and 4800 dpi, 48 bits), a negative with
  and without Digital ICE, a slide, the lamp (it switches itself off after about 15 minutes);
  the image data decoded (interleaved RGB, 8 or 16 bits, the colours some lines apart, the
  scanner applying shading and gamma from tables the computer loads). Notes in
  `captures/epson4490` (`05-catture.md`, `06-formato-dati.md`).

**Left for after the capture, on purpose**: "firmware missing" shown by the app as a state of
its own, and the firmware's path in the Preferences window (today an error at opening that
lists every place tried, and `epson_firmware` in `impostazioni.json`). Showing them only for a
scanner that needs a firmware is a new field of `Capabilities` (the capabilities rule), better
designed once it is known how the scanner behaves around the upload; on Windows the firmware
is found in System32 anyway.

**The repository became private on 10 November 2026**: the firmware is in it and in every
executable, point 4 of "What we do" above.

### When it arrives, in order

0. **Before switching it on**: release both transport locks (the carriage's at the back and the
   transparency unit's), as Epson's guide asks. Nothing moves the carriage before that.
1. **Read only, no Epson driver yet**: USB ids and descriptors (USB Device Tree Viewer or the
   descriptors USBPcap injects), on Linux `lsusb -v -d 04b8:0119`; whether the id or the
   descriptors change after the firmware (seen later, with the driver). No control or bulk
   transfer of our own.
2. **Epson's driver on Windows 11** (installed, see above): with the scanner plugged in, check
   that it binds `usbscan.sys` (`pnputil /enum-devices /connected /drivers`) and shows in WIA
   (`wiaprops`); whether Epson's driver sends the firmware at plug-in or only when a program
   opens the scanner (the backend must know whether it is already loaded).
3. **Captures**, with the tools of `captures/tools` (made general on 9 November: `Start-Cap name
   -Device '04b8:0119'`, `winrt_scan.ps1 -Device '04b8:0119'`):
   - the **power-on with the firmware upload**: `Start-CapNew 'name'` (USBPcap
     `--capture-from-new-devices` on every root hub), then the scanner is switched on, then
     `Stop-CapNew 'name'`; the bytes sent are
     compared with `esfw54.bin`;
   - Epson Scan's start and idle, a preview and scans of the glass at 300, 600, 1200, 2400 and
     4800 dpi, 8 and 16 bits, small areas at the high ones (4800 dpi on the whole glass at 16
     bits would be about 14 GB), by hand in Professional mode or through WIA;
   - the transparency unit: a strip of negatives, a slide, with and without Digital ICE;
   - the four buttons with Event Manager, a cancelled scan, the lamp switching off after idle.
4. **Our first commands** only once they are understood in the captures, and first the ones that
   do not move anything (status, the firmware upload); the carriage and the transparency
   unit's lamp only by replaying sequences seen, with guards like the 3770's
   (`NEGATIVE_RESOLUTIONS`: what was never tried is refused before anything moves).

### What is left, in order (12 November 2026, night)

Everything the 4490 does (Epson's user guide, the captures 00-37, the interpreter) set against
the backend, the contract and the app. The choices below were left by Matteo to this analysis
("do all the reasoning and update the plan", 12 November 2026).

**Where the milestones stand.** 1 (talking) done, but our own firmware upload has never run on
the scanner (on Windows Epson's driver sends it at plug-in). 2 (one scan) done and passed: the
native scan computes any area at 100, 200, 300, 400, 600, 1200 dpi (`scan.rs`), tried at 300.
4 half done: the buttons are in the app, the lamp is not. 3, 5, 6, 7 open.
**Since** (12-13 November 2026): 3's first half done (steps 1-9 below, the app scans the glass);
4 done (the lamp, step 6); 3's second half (steps 10-11), 5, 6, 7 open, and the upload of 1 still
never run by us (step 12).

**A. The glass in the application** (milestone 3, first half: what makes the 4490 usable)

1. **The cold lamp, measured** (12 November 2026, evening; `captures/epson4490/05-catture.md`,
   "La lampada fredda, misurata"): ten identical scans right after switching the scanner on.
   `1B 03` shows the lamp warming (`19 81`) for about 20 s and the scan waited for it, but the
   image still came out 6 levels lighter in the shadows than once steady (green, against the
   last of the series); within 1 level after about 1.5 minutes, within 0.5 after 3. The white
   calibration's K falls about 14 % in the first minute: a lamp just lit gives more light. Not
   measured: the lamp switched off by itself after 15 minutes and lit again with `1E A2 01`
   (one point that morning: +7, the same). So the scanner's own flag is not enough; the wait
   is the backend's: how long and when (switched on, or idle long enough for the lamp to go
   off), Matteo's choice. **Done**: two minutes (Matteo, 12 November) from when the lamp was
   lit (the scanner's warming flag first seen, or `1E A2 01`), when the scanner flags it
   warming or no scan of ours ended in the last 14 minutes (`scan::WARM_UP`,
   `LAMP_OFF_AFTER`, `lamp_is_cold`); `1B 03` polled every second meanwhile. The command keeps
   the time of its last scan in the temporary folder. Still to come with step 4:
   `Progress::WarmingUp` and `LampInfo::warm_up` in the app · **done with step 4**
   (`LampInfo::warm_up` is `scan::WARM_UP`; `Progress::WarmingUp` reaches the app every second
   since 13 November, so that Cancel works during the two minutes).
   Tried the same evening, 17 minutes after the series: `1B 03` showed the lamp warming at the
   opening (`1B 40` lights it again), the scan waited its two minutes and came out as a WIA scan
   made right after (offset 0/0, means within 0.2, WIA = 0.995-0.998 × ours + 0.1-0.3). The
   x origin's correction from `1E 87` was −150 that evening and −93 in every capture and scan
   before: it **changes from one power-on to the next**, and with −150 the geometry matched
   WIA's, so reading it at every scan is what makes it right.
   **The lamp switched off by the scanner, measured** (13 November 2026, evening, on Linux;
   `scansioni/epson4490_lampada_20261113`): the lamp left lit after a scan ending at 19:35:54
   and nothing sent; Matteo saw it off by 19:52:43 at the latest (the moment not seen). At
   19:54:42 eight identical scans (300 dpi, 16 bits, linear, 50 x 50 mm of a photo): at the
   opening `1B 03` flagged the lamp warming (`19 81`, as on 12 November), the first scan
   waited its two minutes, the others none. Against the mean of the last two (green, by
   band of brightness), the scan read 2:12 after the lamp was lit had its lightest tenth
   1.6 % darker (1.25 levels of 255 after a 2.2 gamma) and its darkest tenth 1.7 % lighter
   (0.2 levels); 2:23 after, -0.9 % in the light; 2:38, -0.4 %; from 3:08 within 0.1 %.
   The last two agree within 0.02 %. As the cold lamp (within 1 level after 1.5 minutes,
   0.5 after 3): the two minutes leave about a level in the light parts. Also seen: the
   dark's minimum rose over the series (925 to 1156 in red), the white's K stayed within
   1 %. **Done**: two and a half minutes (Matteo, 13 November; the light parts about
   0.4 % short at 2:38), `scan::WARM_UP` 150 s.
2. **The glass's height**: the app's preview of the whole glass at 200 dpi (2,340 lines of
   11.7 in) would end at step 29,603, past the furthest captured (29,587, capture 07): the guard
   refuses it. **Done** (12 November): the `Capabilities` say 11.69 in (296.9 mm,
   `GLASS_INCHES`; even 297 mm, 2,339 lines at 200 dpi, ended 4 steps further), so the whole
   glass at every resolution covered stays within the captures (a test); the guard keeps the
   user guide's 11.7 in for the area and the furthest captured step for the carriage, which
   lets WIA's own A4 (3,508 lines at 300) through. Left for the next captures: a WIA scan of the
   whole glass (does the last millimetre read?) and one at 600 dpi of the whole width, which
   needs a line time (4224 µs) captured at 100 and 300 dpi but not at 600: until then the guard
   refuses wider than 4,992 pixels at 600 dpi (211 mm), with a message. **Done since** (step 8:
   Epson Scan's whole glass, captures 38-43, 4224 µs at 600 dpi; the guard's test
   `the_whole_glass_is_accepted`).
3. **Linear samples**: the contract wants linear colour (the app applies the `Tone`, and films
   need the shadows), while the scan loads WIA's curves (gamma 1/1.8 in `1E B3`, the tone table
   `1E 84` type 02). Decided: linear tables (the identity in both), measured first.
   **Measured** (12 November, `captures/epson4490/05-catture.md`, "Curve WIA o campioni
   lineari"; `epson4490 scan --tone linear`): the identity tables give linear samples; WIA's
   image is the tone table applied to them (1.3 levels apart on average), a gamma of 1/1.8,
   while `1E B3` does not act on 8-bit colour; so `Tone { gamma: 1.8 }` on the linear samples
   gives WIA's image back (1.7 levels apart). The price at 8 bits: the darkest shadows get 5
   levels where WIA's curve had 24 (the step about twice the noise there), as on the HP 3770;
   16-bit samples avoid it. **Decided** (Matteo, 12 November): linear, as the 3770 (the contract
   as it is, `Tone { white: 255, gamma: 1.8 }` until the tone is measured), and the shadows
   from the 16 bits of step 8, which the app already turns into its tone at 12 bits
   (`tables_fine16`). Tried the same evening at 600 dpi, 16 bits, linear tables: true 16-bit
   samples, 771 different values in the band where 8 bits had 4.
4. **`Gt4490::scan`** on `scan::scan_glass`. **Done** (12 November): the `Capabilities` are
   the guard's (100, 200, 300, 400, 600, 1200 dpi; 8 and 16 bits; colour, grey and line art,
   the last two made by the app; `Tone { white: 255, gamma: 1.8 }`); identity tables (linear
   samples), 8-bit with the 16-bit `samples16` when asked (the top byte, as the 3770); the
   progress (`Starting`, `WarmingUp` (once at first, every second since 13 November),
   `Calibrating`, `Scanning` in bytes, `Finishing`);
   the lamp's last use kept by the open scanner; `details` with the x correction, the columns
   and K. A test runs the app's scan against capture 29, write by write. **Tried in the app**
   by Matteo (12 November, evening): the preview of the whole glass, a scan at 300 dpi, the
   Photo preset (600 dpi, 16 bits) and Cancel during the read all worked. A scan fails while
   Epson Scan is open (it holds the STI lock); a click on the state opens the scanner again.
5. **Cancelling**: the contract's `progress` returning `false`. **Done**: Epson's driver does
   not just stop reading (as first thought): capture 11 shows `1E 18` (answered `06` after about
   a second), then `1B 03` while the carriage goes home (`58 81` for about 4 s), then `1E A1`.
   The scan does the same when stopped during the read, and stops without sending anything
   more during the warm-up or before the calibration (nothing has moved then); the guard lets
   `1E 18` through during a scan. **Tried on the scanner** (12 November, evening, after two
   wrong first tries that left it stuck until switched off): after `1E 18` the scanner first
   sends what its memory holds (262,176 and 301,101 bytes at 300 dpi, stopped after three
   blocks), then `06`; the scan reads it all, a packet at a time, and leaves it out; the
   carriage then went home in 4 s and the next scan ran normally. Found on the way: right
   after the scanner is switched on Windows does not give the STI lock for a few seconds, and
   a scan opened without it got the image service's `1E 85` between its own commands; opening
   now waits for the lock up to 20 s and refuses without it.
   **A program closed half-way through a scan** (12 November 2026, night; captures 44-51,
   `captures/epson4490/05-catture.md`, "Lo scanner lasciato a metà da un programma chiuso"):
   killed while reading, the scanner keeps its carriage half-way and its lamp on, and gives
   out a packet of its image for each command it gets, nothing by itself; `1B 03` gets image
   bytes, not a state (yesterday's `00 00` and `81 00`). Given the STI lock, **Epson's driver**
   asks `1B 03` itself, takes the image for "no firmware" and sends `1B 06` (the scanner did not
   take it; the lock waited 2 minutes). Stopped with `1E 18`: an empty packet or the image
   still held (4.3 MB once), then `06`, the carriage home in 6 s, `18 81`; the next scan
   normal. `1E 18` to a scanner at rest answers `06`. The killed program's lock was free 27 s
   after. **Done**: opening (`Gt4490::start`) asks `1B 03` before the lock, with room for a
   packet: anything but a state of a scanner at rest (`protocol::is_state`) is stopped with
   `1E 18` (`scan::recover`, the guard's `permit_recovery`), written while a second opening
   reads (`Transport::write_reading`: in the app's first try, capture 52, `1B 03` brought a
   block's trailer and `1E 18` alone then waited 30 s, given up; read meanwhile, capture 53, it
   went through at once), then the lock is taken (up to 45 s
   for a program gone), and told in the status. The firmware goes out only on the state seen
   before it (`28 00`): any other unknown state is an error, never an upload. The command line
   has `epson4490 recover`, `probe --no-lock`, `drain --no-lock`.
6. **The lamp in the app**: `lamp_off` with `1E A2 00` (already sent in every calibration) and
   `LampInfo { warm_up, switches_off_after: about 15 minutes }` (capture 21), so the app offers
   its lamp choices. **Done** (12 November, evening): outside a scan the guard lets through
   `1E A2 00` only; tried on the scanner (`epson4490 lamp-off`): answered `06`, the lamp went
   off (Matteo saw it), `1B 03` stayed `18 81`. Learnt: switched off so, `1E A1` answers
   `00` (no lamp), while after the scanner's own 15 minutes it still answers `01`; the scan
   now takes `00` at the opening as a lamp off, lights it with its usual `1E A2 01`, polls
   `1E A1` until `01` (as Epson's driver did for the lid's lamp, capture 25; here at once) and
   gives it the two minutes. Tried: the scan came out normal. `DeviceStatus::lamp` stays `None`.
7. The e2e of the made-up 4490 with these `Capabilities` · **done**
   (`the_epson_4490_shows_what_its_backend_declares` in `e2e.rs`: lamp, buttons, 16 bits, no
   film).

**B. The glass complete** (milestone 3, second half)

8. **16 bits at every resolution**: only 600 dpi was captured at 16 bits (Epson Scan; WIA gives
   8). Decided: capture Epson Scan at 48 bits at the other resolutions, then open them in the
   guard; the contract stays as it is (`depths` not tied to the resolution). **Done** (12
   November, evening; `captures/epson4490/05-catture.md`, "Epson Scan a 48 bit, tutto il
   vetro"): captures 38-43, Epson Scan at 48 bits on the whole glass at 100 (it offers 96 and
   asks the scanner 100), 200, 300, 400, 1200 and 600 dpi; every block as the rules say (the
   tests, 24 captures now). The guard lets 16 bits through at all six, the line time 4224 µs at
   600 dpi (the whole width, capture 43) and reads up to step 29,601 (capture 42). The last
   two u16 of `1E 21` are zero in all of them, so capture 15's 2 and 1002 are not tied to 16
   bits: the scan sends zero, as WIA. Epson Scan's whole height is 11.69 to 11.696 in, as the
   `Capabilities` say. Learnt on the way: on the long reads at 16 bits the carriage stops,
   goes back a little and starts again (Matteo heard it at 1200 and 600 dpi): **the scanner
   does it by itself**, Epson's driver sends nothing but `1B 03` after the read starts (capture
   42, 250 s of reading); it only needs reading on. **Run at every resolution** (13 November
   2026, evening, on Linux; `scansioni/epson4490_16bit_20261113`): the native scan at 16 bits,
   `--tone linear`, the same 50 x 50 mm of a photo at 100, 200, 300, 400 and 1200 dpi (600 run
   before), one after the other: all whole, `18 81` after each, at 1200 dpi `1E 8B` answered
   `15` and the table was loaded (31 s for 2410 lines, no stop heard of). Brought to 100 dpi,
   the channels' means agree within 0.6 % (0.994-1.003 of the 100 dpi scan), the image within
   a pixel; none at 65535, 28 samples of 16.7 million at 0 at 1200 dpi. `1E 87` gave -117 at
   that power-on.
   **Wider at 1200 dpi** (the same evening, measuring step 11c): 150 x 150 mm at 1200 dpi and
   16 bits hung in the app ("bulk IN: transfer was cancelled", nusb's word for a 30 s timeout)
   and again from the command line, "(after 1E 84)" right after the shading: the scanner
   answered nothing more, not even `1B 03`, until switched off and on. Its shading table is
   85,056 bytes there (28,416 at 50 mm), written whole; in the 24 captures only capture 42's
   is over 64 KB (122,496 bytes, Epson Scan at 1200 dpi on the whole glass), and Epson Scan
   wrote it in two pieces (stripped as 65,508 and 56,976, 12 bytes short of the header's
   length: 65,520, the firmware's piece, taken). The scanner then takes what comes next as
   more of the table (a guess that fits). **Done**: `1E 84` tables in pieces of
   `protocol::TABLE_CHUNK` (`0xFFF0`), the guard lets a table through only so cut (a test);
   tried: the same scan whole, 7135 lines in 83 s, `18 81` after it, the image sound (four
   photos, none at 0 or 65535), 1.13 GB at most in the command line's memory.
   **After a power cycle with the carriage away** (twice that evening, both after a hung
   scan): `1B 03` answered `08 00`, not `28 00`, the front light green, the carriage where it
   stopped; bit `0x20` is taken to be "the carriage at home" (a guess, twice consistent). The
   app refuses to send the firmware then (a state never seen); `epson4490 upload
   --carriage-away` sends it on `08 00` only (a test): both times the firmware moved the
   carriage home by itself (`5A 81` for 1.5 s about 13 s in, `18 81` at 20-22 s), as after
   every upload. **Then decided** (Matteo): `08 00` is a state without the firmware for
   the app too (`protocol::is_state`, `ensure_firmware`); before, the app's opening would have
   taken it for a scan left half-way and sent `1E 18` to a scanner without its firmware (never
   happened: the app was never opened on `08 00`). `--carriage-away` is gone; `epson4490
   upload` prints the state until the scanner is at rest. Tests: the opening on `08 00` sends
   the firmware and stops nothing. Not yet seen in the app on the scanner.
9. **1200 dpi after every power-on**: the scanner answers `1E 8B` with `15` until a 600-byte
   table (`1E 84` type 07) is loaded, which Epson's driver computes from a raw line it reads
   with `1E 83`. **Done** (12 November, evening; `table07.rs`, `captures/epson4490/05-catture.md`,
   "La tabella dei 1200 dpi"): read in the interpreter (`0x40f4f0`): 244,800 bytes of 16-bit
   big-endian samples, six interleaved (two rows of the sensor); per colour, the elements rising
   1260 or more above their neighbours, then the 41 highest of the rest, after a header (the
   length, then 4000, 19900, 35800 less `1E 87`'s correction); three lists of 100 u16. Rebuilt
   byte for byte for captures 05 and 42 (different tables); a test runs capture 05's whole scan
   with it. On the scanner after a power-on: `15`, the table loaded, the scan right; again:
   `06`, the same image.
10. **The stripes and the colour** on the 4490: "Misura le righe" and the colour matrix, as for
    the 3770 (phase 13's tools). **The colour done** (14 November 2026): no target needed,
    Epson Scan's own correction measured. It sends the scanner none (`1E 57` byte 33 is 0 in
    every capture) and applies a matrix on the computer to what the scanner sends (values of
    its tone tables, gamma 1.76-1.87 by channel, its levels per scan). Fitted on its captures,
    the data sent against its TIFF (`captures/epson4490/tools/colour_fit.py`): 15 (600 dpi)
    every pixel exactly, in thirty-seconds, rows of 32: (35, -2, -1), (-3, 38, -3), (-2, -14,
    48); 13 and 57 (2400 dpi, two power-ons) within 0.003; 59 and 14 (4800) looser, Epson
    Scan doing more of its own there. `epson_gt_x750::GLASS_COLOUR` (gamma 1.8) declared for
    `Source::Glass`; the app applies a glass correction in `cleaned`, before anything else, on
    the 16-bit samples or the 8-bit ones brought to 16 (films keep theirs in `develop`), the
    tick in the Scan tab where the scanner declares one; on by default (Matteo, after trying
    it), the "off" older files keep in Documento and Foto cleared once (`Prefs::migrated`,
    `glass_colour_default`, a test). An e2e with a made-up scanner declaring one and one not.
    Checked on our own 2400 scan of capture 57's place against Epson Scan's TIFF, 16 x 16
    blocks, any tone allowed: blue left over 0.231 without, 0.147 with (red 0.097/0.089,
    green 0.121/0.117). **The stripes: none on the glass** (14 November 2026, a white sheet,
    our 8-bit linear scans at 300, 600 twice, 1200, 2400 and 4800 dpi,
    `../scansioni/epson_righe_20261114`): `hp3770 stripes` (the app's threshold, 0.2 % line to
    line) "none worth removing" on all; line to line 0.13-0.15 % against 0.04-0.09 % of noise,
    the rest the paper's grain (the two 600 dpi scans alike), and no repeat of 2-16 lines
    explaining more than 7 % of it (5-7 % at 2400, 1-2 % elsewhere). Nothing built:
    `Capabilities::stripes` stays empty for the glass; films to be measured with their lamp.
    **Step 10 done.** Seen meanwhile: the scanner deaf by itself, idle, ten minutes after a
    good scan (Windows' image service holding it, our writes cancelled, 995; its light steady
    green): switched off and on, all well; not the `98 81` error, cause not known.
11. **2400 dpi**: the rules already rebuild Epson Scan's capture 13 (2400 x 3200, reduced on the
    computer). A second capture at 2400, then the guard and the reduction in the driver.
    **2400 dpi done** (13-14 November 2026): Epson Scan's captures 57 (2400 dpi, 48 bit, 20 x
    30 mm at x 178.7, y 262.2 mm) and 58 (24 bit, the same place, shorter), with 59 and 60 at
    4800 for later; `setup.py --check` gave 3 differences on each, all the columns of `1E 41`:
    it took the x correction as -93, the scanner gave -102 that power-on. Read from each
    capture's `1E 87`, the 21 glass captures give 0 differences. The test files of 57, 58 (and
    42 corrected: its shading is two `=shading` writes, 65,520 and 56,976) are made by
    `tools/strip.py` (`captures/epson4490/tools`, the first generator was not kept; it rebuilds
    the files in `testdata` as they were). In the driver: `setup::READ_ALONG` and
    `Request::glass` (2400 asked as 2400 x 3200, y rounded, the height rounded up),
    `scan::reduce_along` (each line the weighted mean of the lines it covers: 3/4 + 1/4, 1/2 +
    1/2, 1/4 + 3/4; how Epson Scan reduces is not known), the guard (2400 at 8 and 16 bits,
    only x 3200, line time 10,752 µs, the glass counted along at `ydpi`), 2400 in the
    capabilities. A line longer than a data block (65,527 bytes) was never captured: at 2400
    dpi and 16 bits at most 10,912 pixels (115 mm) across, refused beyond with that width
    named; at 8 bits the whole glass. Epson Scan's `1E 57` at 8 bits is its own (gamma `04`, 46
    lines a block); ours is WIA's form (gamma `03`, our lines a block), never seen at 2400 but
    taken by the scanner. Tried (14 November 2026, `../scansioni/epson_2400_20261114`): capture
    57's area at 16 bits, the plan the same as Epson Scan's (start move 26,008 steps, read to
    step 29,129), 44 MB in 43 s; against Epson Scan's TIFF the same place to the pixel (0
    lines, 0 columns), the same picture through a monotonic map (Epson's is not linear),
    Epson's 10 % sharper line to line both ways, no pattern every 3 lines (0.005 % of the
    level; Epson's 0.02 %). Then in the app (Matteo, 14 November 2026): 2400 dpi at 8 bits on
    a small area drawn on the preview, through. Not tried on the scanner: a wide area at 2400.
    **4800 dpi**: another path (other columns, T `0x9480`): its own captures and rules.
    **4800 dpi done** (14 November 2026; notes in `captures/epson4490/06-formato-dati.md`, last
    section, and `../scansioni/epson_4800_20261114`): Epson Scan's captures 14, 59, 60 (the
    night before), 61 (the glass's left edge), 62 (the middle), 64; `setup.py --check` gives 0
    differences on all 26 glass captures. The same sequence as at 2400; 4800 x 4800 (no other
    resolution along); the reads at T 0x41C0 (the table's row), `1E 46` and the black pixels'
    `1E 21` at 0x9480; origin 552, black pixels 70-166, no longer line time for a wide read;
    `1E 21` of its own (`0x41f0a4`, `0x41f03f`); the start column moved by the interpreter
    (`0x40fdd0`: 8,400 to the left, less near the left edge, with two words of `1E 21`;
    `setup::columns_4800`); 200 extra lines (2 x 96 + 8); `1E 8B` and the 07 table as at
    1200. The image: the colours 96 and 192 lines apart as at any resolution, and **the even
    sensor columns 8 lines later** (two staggered rows: against Epson Scan's TIFFs of 59 and
    61, whose reads start on an odd and an even column; `Plan::stagger`, `Plan::late`). The
    block limit: at most 10,912 pixels (58 mm) at 16 bits, 21,840 (116 mm) at 8 bits. Tried:
    59's and 61's areas, then a control (65, ours, right after Epson Scan's 64 on 61's area,
    nothing touched): the raw data the same to 0 lines and 0-2 columns, the image the same as
    Epson's TIFF to the pixel, no teeth between even and odd columns. The scans of the hour
    before were 0.2-0.4 mm off, Epson Scan's own too (61 against 64, same area: 35 lines, 44
    columns): at 4800 dpi the original, or the mechanics, drifts over an hour. Both need
    the size limit of "Risks" (4800 dpi at 16 bits is gigabytes). **The limit, done** (13
    November 2026, step 11c): measured in the app on Linux, a 1200 dpi 16-bit scan of 150 x 150
    mm estimated at 452 MB (`App::memory`) took the app from 70 MB to 1.04 GB at most while its
    image was put together, 595 MB once shown (the command line alone: 1.13 GB). A scan whose
    estimate times 2.5 (`PEAK`) is over half the computer's memory is refused, said in the
    Scan tab and on Scansiona, nothing sent (an e2e). The memory is the computer's total, read
    once on a thread at start (`ram.rs`: `/proc/meminfo`, `sysctl -n hw.memsize`, PowerShell's
    `Win32_ComputerSystem`; no `unsafe`); until known, no limit. Not measured: adjusting and
    saving such a scan; the Windows and macOS readings run only there.

**C. Linux and macOS** (before the films: it is the promise of the plug and play)

12. **Our upload and `nusb` on the scanner**: the scanner on Linux, either a Linux machine or WSL
    with `usbipd` (the scanner taken from Windows' driver meanwhile: Matteo chooses), both
    firmware copies (Windows driver's and AVASYS'), then a glass scan. macOS: no Mac here, the CI
    only. **Next, on a real Linux machine** (Matteo, 13 November 2026, evening: the work moves to
    another computer, so this is the hand-over; nothing tried on Linux yet, everything committed
    and pushed up to here).
    What is tested: `nusb` (usbfs) as the transport, with no STI lock, no Epson driver and no
    button poll in between; **our firmware upload** (`protocol::upload_firmware`), never sent
    by this program to the scanner (on Windows Epson's driver sends it at switch-on; its bytes
    are capture 01's), with the two copies `known_firmware.rs` accepts: the Windows driver's,
    embedded by default from `crates/epson-gt-x750/firmware/esfw54.bin` (private repository),
    and AVASYS' (the iscan plugin, or Fedora's `iscan-firmware`, looked for in
    `/usr/lib/firmware/epson/` and `/usr/share/iscan/`); then a glass scan, and the opening's
    recovery (`Transport::write_reading`) through `nusb`, never run. Matteo present, one step
    at a time, each said before it is run:
    1. Clone, then `cargo build --release -p epson-gt-x750 --bin epson4490` (the app's build
       needs the packages of `docs/install-linux.md`).
    2. The udev rule: `sudo cp packaging/linux/70-epson-4490.rules /etc/udev/rules.d/`,
       `sudo udevadm control --reload`, then plug the scanner in or switch it on.
    3. The scanner just switched on (no firmware running): `epson4490 list`, `epson4490
       firmware` (which copy would go), `epson4490 probe`: expect `28 00`.
    4. `epson4490 upload` (the embedded Windows copy): expect the `06`s, about 9 s, then
       `probe`: `18 81`.
    5. Switched off and on, the same with AVASYS' copy (`--firmware PATH`) if Matteo has it.
    6. `epson4490 scan 300 40 60 50 50 out.png --tone linear --move` (two minutes of warm-up
       when the lamp is cold), set against `scansioni/blocco_20261112/dopo_recover*.png` (the
       same area, with the same photo; on the Windows machine, not in git: copy it across).
    7. The app (`cargo build --release`, `target/release/vetroscan`): a preview, a scan; then
       closed half-way through a scan and opened again (the recovery through `nusb`).
    Meanwhile: `1B 03` answering with bit `80` (`98 81`, the front light flashing red) is the
    scanner's own error since `33d30de`; on Linux there is no Epson Scan to clear it, so if it
    comes, switch the scanner off and on, and note whether that clears it. Captures there with
    `usbmon` and Wireshark/tshark (`sudo modprobe usbmon`), if wanted. The checks on that
    machine too: `cargo fmt --all --check`, `clippy -D warnings`, `cargo test --workspace
    --locked`. The project's memory and the session notes stay on the Windows machine: this
    plan, `CLAUDE.md` and `../captures` (not in git; copy `captures/epson4490` across if the
    capture notes are wanted there) are what a session on Linux has.
    **Tried on Linux** (Matteo's laptop, Omarchy/Arch, Intel graphics, 13 November 2026,
    evening, build of `df978bf`): steps 1-4, 6 and 7 work.
    - The udev rule was already there (`/etc/udev/rules.d/70-vetroscan.rules`, the same line
      for `04b8:0119`): the device node gets the seat's ACL, no root needed.
    - `probe` on the scanner just switched on: `28 00`, as expected.
    - `upload` (the embedded Windows copy) through `nusb`: every step answered `06`, 8.8 s;
      right after it `19 81` (the lamp warming, by itself), `18 81` 15 s later and still so
      after 90 s.
    - `scan 300 40 60 50 50 --tone linear --move`: 135.5 s, of which 120 s the warm-up (the
      program found the lamp off, a few minutes after the `19 81`); `1E 87` gave -104 (-93
      computed before), shading K 44091-44924, 603 lines read in about 3 s, every wait ended
      on `18 81`. 591 x 591 px, no stripes nor shift; levels R 1-194, G 1-196, B 3-185, none
      at 0 or 255 (`scansioni/linux_20261113/vetro_300.png` on the Linux machine). Not yet set
      against `blocco_20261112/dopo_recover*.png` (on the Windows machine).
    - The app: a preview and a scan work; closed half-way through a scan and opened again, the
      scanner answers and the preview works (Matteo; nothing in `tasti.log` beyond
      "scanner opened again").
    - Step 5 (AVASYS' copy) not done: not on that machine (no `/usr/lib/firmware/epson/`,
      no `/usr/share/iscan/`).
    - The checks on Linux: `fmt`, `clippy -D warnings` (the app included) and every test pass,
      but the app's e2e crashed twice (SIGSEGV inside `libvulkan`,
      `vkEnumerateInstanceExtensionProperties`, while `iced_test` made wgpu renderers in
      parallel): NVIDIA's Vulkan driver (`nvidia-utils`) was installed on a machine with no
      NVIDIA card. With `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/intel_icd.json` the e2e
      passes (127, 4 ignored). Not our code; a Linux machine with a stray Vulkan driver.
      With `nvidia-utils` removed, `cargo test --workspace --locked` passes as it is.
    Step 12 is done (Matteo, 13 November 2026). The scan set against Windows' (step 6) is
    dropped: the scan is clean and nothing it could show would change what comes next.
    AVASYS' copy (step 5) is put off, not dropped: the private releases carry the Windows
    copy, which works; it must be tried on the scanner before any public package carries it
    (point 3 of "The firmware and its licence").
    **Back on Windows, first** (what the Linux evening of 13 November changed and only Linux
    ran; commits `7c3cf9c` to `a8cb1f3`):
    1. The checks of `CLAUDE.md` on Windows and in WSL (the commits touch `#[cfg]` code:
       `vetroscan/src/ram.rs`); clippy for `x86_64-pc-windows-msvc` passed on Linux, nothing
       ran on Windows. **Done** (13 November 2026, on `9631cb8`): build, fmt, clippy and tests
       pass on Windows (404 passed, 10 ignored) and in WSL (403 passed, 10 ignored).
    2. `epson4490 scan 1200 30 40 150 150 out.png --bits 16 --tone linear --move` through
       `usbscan.sys`: the shading in pieces (`TABLE_CHUNK`), so far only through `nusb`; then
       the same scan in the app. **Done**, in the app too (Matteo, 13 November 2026, with the
       pause below). From the command line (13 November 2026, notes and
       logs in `../scansioni/epson_1200_larga_20261113`): the pieces back to back went
       unanswered after the shading (no `06`, `usbscan` gave up after 30 s, error 995) three
       times out of three, the scanner then deaf until switched off and on; with USBPcap
       capturing, seven out of seven went through (capture 55, 150 x 150 mm: our URBs the same
       as capture 42's, 65,520 and 19,536 bytes, the second 1.7 ms after the first; capture 56,
       six of 150 x 20 mm). Epson Scan waits 1.2 ms between them (42): a pause of 5 ms before
       each piece but the first (`protocol::TABLE_PAUSE`, a test). With it and no capture: six
       of 150 x 20 mm and one of 150 x 150 mm (103 s, the image sound). On Linux the pieces
       went through with no pause (once): `nusb` may leave its own gap. Seen on the way: a
       scan within `LAMP_OFF_AFTER` of the last, after the scanner was switched off and on,
       does not warm the lamp up if `1B 03`'s warming flag (about 20 s) went by before it (the
       first of the six, 13 November); the scanner's switching on is not seen otherwise.
       **Done** (14 November 2026): `1E 87`'s answer is the power-on's fingerprint. In the
       captures it is the same at every scan of a power-on (03-37: two days, 35 scans with the
       lamp off, warming and warm, `1B 40`, the lid's lamp; 38-43; 55-56; 57-60; 61-65) and
       every number of it differs from one power-on to the next (five: x correction -93, -150,
       -98, -102, -160, and the three numbers after it too; `1E 8B` answering `15`, the 1200 dpi
       table lost, confirms the power-ons at 05, 42, 55, 61). The scan now asks `1E 87` before
       the warm-up (Epson's drivers ask it once the lamp answers, and have no warm-up of their
       own; only `1B 03` polls come after it now when the lamp is cold); a different answer
       from the last scan's (`scan::LampUse`, kept by the open scanner and in the command's
       temporary file) means a cold lamp, said in the log. Tests: `lamp_is_cold`, and capture
       29 scanned a minute after a scan of the same power-on (no warm-up) and of another one
       (warm-up). In the app the open scanner is opened again after a power cycle and its
       last use forgotten, so the app already warmed the lamp up: the command line was the
       case seen. **Tried** the same afternoon (`scansioni/epson_riaccensione_20261114`,
       `epson4490 scan 100 40 60 50 50`): the first scan warmed up (x correction -128 in
       that power-on), the second right after did not (12.6 s, `1E 87` the same); switched off
       for 5 s and on, the scanner at rest at once (`18 81`, no warming flag: the case of the
       fault), the third said "switched off and on since" (-101 now) and waited 150 s.
    3. The app's memory limit there: the Scan tab with a scan over half the PC's memory (the
       memory read with PowerShell, `ram.rs`; until read, no limit). In part (13 November
       2026): `ram.rs`' command, run alone, prints 34,274,361,344 (32 GB) in about 0.5 s,
       which `number()` reads. The refusal itself cannot be seen on this PC: with 32 GB it
       starts above an estimate of about 6.9 GB, out of reach of the Epson at 1200 dpi (A4,
       16 bits: 0.9 GB) and of `VETROSCAN_FAKE_SCANNER` (2400 dpi, 16 bits, the whole glass:
       3.4 GB); it needs a PC under about 17 GB with the made-up scanner, or 4800 dpi (11b).
       **Seen** once 2400 dpi came in (14 November 2026, Matteo): the Scan tab with the
       Epson's whole A4 at 2400 dpi, 8 bits (19,843 x 28,056 px, "in memoria 10021 MB") says
       "servirebbero circa 25,1 GB, il computer ne ha 34,3", in red, and nothing is sent: the
       memory read with PowerShell on Windows. Left: `App::bytes` doubles the estimate above
       1200 dpi for the HP 3770's reasons (its chip's half-width data), a scanner's number in
       the app; for the Epson the estimate is cautious (1.7 GB of image there). **Done** (14
       November 2026): `Capabilities::scan_memory` (a multiple per resolution, none by
       default); the HP 3770 declares 2 at 2400 dpi, the Epson 4490 11/6 at 2400 (the data
       read at 3200 and the image realigned from it, 4/3 each, beside the reduced one; computed
       from the code, not measured) and nothing at 4800; the app looks it up at the resolution
       sent (doubled scans too); an e2e with the made-up flatbed with and without it.
    4. Capture 42's `.pcap`: the first piece of its shading, 65,508 or 65,520 bytes (the
       stripped text says 65,508, the header's length says 65,520). **Done** (13 November
       2026, read from the `.pcap`): USBPcap's `dataLength` is 65,520 (`0xFFF0`, the 65,508
       are the snaplen's cut: 65,535 less the 27-byte header), then 56,976, 1.2 ms after the
       first was done, no `06` between, one `06` after both; `TABLE_CHUNK` does the same.
    Then the steps that need Epson Scan's captures: 11a/11b (2400, 4800 dpi), 13-15 (films),
    17 (Digital ICE).

**D. Films** (milestone 5)

13. **The lid's lamp**: switching it as Epson's driver does (`1E 77`, `1E A2 01`/`02`, `1E A1`
    polled until it answers, capture 25), both ways; today a scan of the glass after films stops
    with an error. **Back to the glass done** (14 November 2026): at the opening's third
    `1E A1`, `02` (the lid's) gets `1E 77`, the state and `1E A1` asked again, then the scan's
    `1E A2 01` lights the glass's and `wait_lamp_lit` asks `1E A1` until `01`, the lamp then
    warmed as one that was off (a test: capture 25 written write by write as far as `1E A2 01`;
    the guard lets `1E 77` out at the opening, no block). Tried on the scanner after an Epson
    Scan session: `02`, `1E 77`, `01` at once, the scan whole. The other way (to the lid's, for
    films) with step 14. **To the lid's done** (14 November 2026, evening; captures 72, 73, 16):
    Epson Scan lights it with `1E A2 02` after the film's `1E 57`; the scanner answers `06` only
    once it has switched lamps (2.52 s every time), then flags warming (`19 81`) and `1E A1`
    answers `02`; Epson Scan asks `1B 03` and `1E A1` every second until `18 81` (18.8, 20.8,
    21.7 s) and calibrates at once. `scan::light_lid_lamp` does the same (a test: capture 72
    write by write; the guard lets `1E A2 02` out only for a film request), the command
    `epson4490 lid-lamp [--positive]`. **Tried on the scanner** (captures 83, 84): after Epson
    Scan the lid's lamp was still the one chosen, and our `1B 40` lit it again by itself (20.6 s
    of `19 81`, as Epson Scan's own in 79); after a glass scan of ours (back to the glass's lamp
    with `1E 77`, as in the morning), `1E A2 02` answered in 2.4 s and the lamp was warm 20.2 s
    later; nothing moved. Not measured: whether the lid's lamp gives steady light by then (the
    glass's needed 150 s more than its flag); to be measured on our first film scans, as for
    the glass. Not tried: `1E A2 00` (the app's lamp timer) with the lid's lamp chosen, never
    captured.
14. **The film path's rules** from captures 16-19 (negatives, a slide, previews; Epson Scan at
    2400 dpi 48 bits): motor base 600 steps per inch, ramps of 64 and 128 values, origins 647 and
    36, 4800 dpi across halved on the computer; then the guard for films, as `NEGATIVE_RESOLUTIONS`
    on the 3770.
    **Captured** (14 November 2026, Matteo at Epson Scan, Professional mode, no corrections;
    `captures/epson4490/05-catture.md`, "Le pellicole, catture 72-81"; `tools/film.py` there
    prints a film capture in short): 72 the glass's preview then a negative's (the lamp switched
    with the glass's warm), 73 a negative's preview right after a power cycle, 74-76 frame 1 of a
    colour negative strip at 600, 1200, 4800 dpi (16 bits), 77-78 frame 6 at 2400 dpi (16, 8
    bits), 79 the slides' preview, 80-81 the slides in positions 1 and 4 at 2400 dpi (16 bits).
    82 (the 6 x 12 cm holder) waits for a medium-format film (Matteo has none yet).
    **Read from them**:
    - **The request** (`1E 57`) is Epson Scan's form: source `01` (byte 26), gamma `04`, the lines
      per block (byte 28) 262,144 over the bytes of a line, at most 32, the film in byte 37 (`01`
      negative, `00` positive); rebuilt byte for byte for all eleven film requests captured
      (`film::request_block`, a test). **The window** is the same for strips and slides: the
      preview asks 810 x 2799 at 300 dpi from (0, 0), 2.7 x 9.33 in (`film::ADAPTER_INCHES`).
      **4800 dpi is read along at 5400** (capture 76: 7,872 lines read, Epson's image 6,997),
      **2400 at 2400** (not 3200 as on the glass; captures 17, 19, 77, 78, 80, 81).
    - **The sequence is the same at every resolution and film**: `1E 25 00` and a move of 970 +
      1,024 steps parks the carriage at 1994 (`1E 86` then answers 1994; `04-carriage.md`'s park);
      `1E 25 02`, 8 steps; a read of 150 lines (columns 2800-3184, while moving 1,920 steps, ramp
      256) with the lid's lamp; `1E 25 02` and a move back of 228-252 steps (`1E 01` flag `0x30`,
      the other direction), then `1E 8A` with 1,024 + those steps (`f0 04`, `fc 04`, `e4 04`):
      the transparency unit's origin found from that read, it seems (the unit's position after
      it is always 1416 less the move back). Then `1E 25 03` and a loop of short reads (4 lines,
      the black pixels 35-83, then columns 2276-8756) with `1E 31` changed between them (bytes
      0-5 from `80 00` to `8a 00 8c 00 90 00`, a binary search; then bytes 8-10 from `22 1b 1b`
      down: the offsets, then the gains or the exposure, set by the driver per film, which the
      glass never does), `1E 92` reading it back; a dark read with **`1E A2 03`** (the lid's
      lamp off, it seems) then `1E A2 02` again; `1E 8B`/`1E 83` and the 07 table as at 1200 on
      the glass; the image read with a shading table (`1E 84` type 05) and `1E 47`.
    - **The guard for films** (`guard::FILM_COVERED`, `check_film_request`, `permit_film`;
      committed): only those requests, inside the window; a film's opening may send its `1E 57`
      and the lid's lamp (`1E A2 02`, not `01`, not `00`), and nothing that moves (`1E 25`, `01`,
      `05`, `47`, `84`, `8A`, `11`, `41` refused for a film: "a film moves nothing yet"); a
      glass scan cannot light the lid's lamp. Tests.
    **The geometry done** (14 November 2026, night; `film::geometry`, a test on all eight film
    scans; the model `captures/epson4490/tools/filmsetup.py --check`, 0 differences): what the
    set-up takes from the request. `1E 25` selects what moves (`00` the carriage, `02` the lid's
    unit, `03` both) and moves are relative. The park: 1994 less the carriage's position (512
    ramp). The unit's origin: 8 steps, the 150-line read over 1920 + 512 steps, back N + 1024,
    `1E 8A` = 1024 + N, the unit then at 1416 - N. The base: 2400 up to 1200 dpi, 4800 at 2400
    and 4800 (a 2400 dpi film is read at 4800 across, two columns a pixel); the pixels read a
    multiple of 16; `1E 11` 8 + 1 at base 2400. **The start column**: the glass's left-edge form
    (`0x40fdd0`, divisor 6 at base 4800, 3 at 2400) from (13,320 + `1E 87`'s correction) x base /
    4800 + x, its words in `1E 21` bytes 13-16 (w1 from the columns + 160); two power-ons (-93,
    -122) at base 4800, one (-122) at base 2400. The lines: h + 2 x ydpi / 50, + 4 at 2400 along,
    + 12 at 5400. `1E 41` byte 8 the ratio code (| 1 at 8 bits), its ratio ydpi / 2400, the lines
    per block 65,527 // a line's bytes. The read's ramp (den x T / num) >> 4, its steps lines x
    2400 / ydpi + the lead (128 at 600 dpi, 32 otherwise), the start move y x 2400 / ydpi + 583 -
    lead - 1024 from the carriage at 2058. The black pixels' columns 35-83 (base 2400) or 70-166,
    the window's for the exposure reads 2276-8756 or 2320-15280. Epson's interpreter places the
    film with values it reads from the scanner (`0x421470`, the holder's marks it seems, with
    defaults): in every capture they gave the same window, so the constant; not known when they
    would differ.
    **The offsets read** (same night; `filmsetup.py --check-offsets`, 0 differences on the
    eleven film captures, previews included, every read and every value): `1E 31` bytes 0, 2, 4
    (one a channel) start at `80`; after each read of the black pixels (4 lines, 16 bits) every
    channel whose mean is above 1536 goes down by the step, below 1024 up, the step `40`, `20`,
    `10`, `08`, `04`, `02`, `01`; every channel is judged again at every read (capture 72: one in
    the window at `88`, out of it at the next read, moved by `02`); the search ends after a read
    that moved none, then one more read with the result. **The exposures, seen**: in `1E 21`
    each channel integrates T less its own value (bytes 3-5, 6-8, 9-11), and the 8-bit levels of
    the window's reads follow it (74: 61 at 3840, 207 at 13,182); for negatives the gains then
    (`1E 31` bytes 8-10, from `22 1b 1b`, at most 5 down a read, about 2.5 % a unit) bring each
    channel's mean to about 204-211. Their exact rules: not read yet. **N, seen**: the 150-line
    read (8 bits, 12 steps a line) sees the lid's light come in by steps between lines 22 and
    31 and go out about line 95-105; N is 12 x a line count (228, 240, 252) and goes down when
    the light comes later (80 against 74), but no threshold crossing tried gives it on all 14
    captures (at 250, within a line on 9): the rule is to read in the interpreter. It moves the
    lid's unit by 12 steps (1/200 in) either way.
    **The exposure, in the interpreter** (same night; `esxuin54.dll`, `llvm-objdump`): the film
    calibration is `0x426af0`: it sets `1E 21` byte 12 to `3f`, the first exposure to 3840 µs
    each channel (`0x42d28c`), a margin M (`0x42d278`, from `0x42d480`, computed from the read's
    columns in `0x422dd0`) and per channel a ratio from a table at `0x40c820`; then the offsets'
    search (`0x4245e0`, the rule above) and one of three: `0x424c00` (negatives, it seems: the
    gains), `0x4254d0` (byte `0x35` = 1: slides?) or `0x426210`. `0x422ff0` turns exposures into
    `1E 21`: X' = (X - 110) x k / 10 + 110 per channel, T = M + the longest X', rounded up to a
    multiple of 384 µs (every T in the captures is one), each channel's value T - X'. From the
    captures M is 5650-5760 at base 2400 and 8900-9016 at base 4800. **The exposure loop read**
    (`0x426210`, the films', and `0x424c00`; `filmsetup.py --check-exposure`, 0 differences on
    the 14 steps captured): after each 4-line read of the window (8 bits) L = (the sum of the
    4 lines' maxima) // 4 per channel; below 222 or above 236 the channel's X becomes
    X x 230 // L, within its limits (at least 3840); 222-236 is done; until every channel is
    done or held. Slides: all three channels (80, 81: three reads). Negatives: the second and
    third held at 3840 (their limit, from the table at `0x40c820`) while the first is found
    (two or three reads); then final exposures that do not come from these reads (the same
    13,550 / 9070 / 5953 for frame 1 at 600, 1200 and 4800 dpi, whatever the loop's last read
    gave: from the preview, it seems), the second and third always 0.6694 and 0.4393 of the
    first; then the gains. **All of it read** (14 November 2026, night; `film_cal.rs`, tests from
    the captures; the model's `--check-gains`, 30 steps, and a check of the 52 window reads'
    `1E 21`, 0 differences): the negatives' final exposures (`0x423180`) are the first channel's
    less 110 µs rounded up to 384, then that, 10/15 and 10/23 of it, each + 110 (13,182 gives
    13,550 / 9070 / 5953; not the preview, as first thought); the rounded value is the image's
    line time (13,440). M (`0x422dd0`) = ((C - X) / r) / 2 + X / 2, C from the interpreter's
    table at `0x40ca20` (41,980 at base 4800, 21,100 at 2400), X the window read's start r8 +
    its 12,960 or 6480 columns + 160, r 6 or 3: 8981 and 5731. The window reads' `1E 21`: T = M
    + the longest exposure rounded up to 384, each channel's value T - X, byte 12 `3f`, the
    window's words (3840, 550 or 2209, 548: the film origin without the x correction), then
    (M + 254) / 2 in the loop or the least value / 2 after the negatives' final step, and the
    greatest value / 2. The gains (`1E 31` bytes 8-10, from `22 1b 1b`): indices into the
    interpreter's 64-value table at `0x401180` (966 to 6424, 2.6 % apart); a channel at 222-230
    keeps its gain, any other goes to the highest index whose value is at most value x 228 /
    level; the search ends after a read that moved none. The table at `0x40ca20` also holds the
    film origin 13,320 and the window's 12,960 columns (6660 and 6480 at base 2400).
    **The rest of the calibration read** (same night; `film_cal.rs`, `shading.rs`, tests from the
    captures; checked on all eight film scans): **N** (`0x420fe0`): each line of the 150-line read
    its second channel's mean; the threshold half the highest (in tens); after more than five
    lines above it, the first line below is the edge; `1E 8A` = (150 - edge + 35) x 12 + 256, the
    move back that less 1024 (all 14 captures). **The white read's `1E 21`** (`0x425c20`): when
    the longest exposure exceeds the next by at least the margin of the image's columns (4338 in
    74, 5509 in 77), T is it rounded up to 384, that channel 0 (byte 12 `3e`, `3b`, `2f` by
    channel), the others T - X; otherwise the last window read's block with the image's words
    (77, 76, 17). **Negatives' dark and image**: every channel the whole line, T the rounded first
    exposure (13,440), byte 12 `2a`; the image's gains from the calibration's: the second to the
    table's value x 2000 / 1500, the third x 3000 / 2300 (`21 09 16` to `21 14 1f`); the dark read
    with the image's settings. **Slides' dark and image**: the white's form with the exposures
    (X - 110) x f / 10 + 110, f Epson Scan's own choice (15 in 80, 10 in 81: its auto exposure
    from the preview, it seems; ours may keep 10); gains `18 1b 18` throughout. **The shading**: the
    glass's computation, but at base 4800 the minimum dark of the even and the odd columns apart
    (`1E 42` carries both: 687 and 1650 in 77): then the dark bytes as Epson's but a few; the white
    bytes 2-4 units off (the glass's about 1; Epson's rule for dusty lines unknown).
    **The film plan** (same night; `film::FilmPlan`, a test: every read's `1E 41` of 74, 76, 77
    and 80 as captured): the origin read (2800-3184, 150 lines, 1/12, `1E 11` 03, T 6144), the
    black pixels' reads (35-83 or 70-166, the image's ratio), the window's (2276-8756 or
    2320-15280, 8 bits), the white (the image's columns, 16 lines, 1/2, byte 11 `40`), the dark (64
    lines, the image's ratio), the image; the black reads' words and margin (60, 0 and 3596 at
    base 2400; 81, 0 and 3634 at 4800, from the captures' T and words). **The film path's
    512-value ramp** (`0x401460`, the park and start moves) computed with the glass's formula,
    period 222 µs and k 1023.065 (`setup::film_ramp`: every value as the interpreter's).
    `film_cal::GAIN_TABLE` is the interpreter's 64 values copied (no formula found), where the
    ramps are computed: kept so (Matteo, 14 November 2026).
    **Decided, not read**: the gamma and tone tables linear (the identity), as on the glass, where
    the app turns the samples into its tone (Epson Scan sends a negative's inverted curve).
    **The film's calibration in the driver** (same night; `scan::calibrate_film`): the lamp, the
    park (from home, or from where the scanner puts the carriages when it lights the lid's lamp
    again, capture 72), the lid unit's origin, `1E 87`, the offsets, the exposures and a
    negative's gains, the black pixels, the white and the dark read; tested write by write
    against captures 74, 77 and 80 (`testdata/*_film.txt`, with reads made up from what the rules
    take from the real ones: the same offsets, exposures and gains as Epson Scan's). Learnt on the
    way: a slide's `1E 43` is `8000` throughout; after its loop a slide sends its window block
    once more, and its black read takes the black pixels' margin. **The guard for the film path**
    (`permit_film_path`, `permit_film_plan`): `1E 25` 00/02/03; only the moves captured (the
    park from home, the unit back from 1078, the 8 steps, the origin read's 1920, the moves back
    of 228, 240 or 252 steps and their `1E 8A`, the white's 64), the plan's start and read moves
    once permitted; `1E 05` only after a move; `1E 21` with a line time a multiple of 384 within
    6144-25,344 µs; one-value ramps no faster than 156; the plan's `1E 41`; the ramps computed
    (the film's 512 and the origin read's); linear tone tables; the plan's shading.
    **The image read in the driver** (14 November 2026, night; `scan::scan_film`, tests: captures
    74, 77 and 80 written write by write to their last line, the carriages home): after the dark
    read `1E 8B` (`06` in every film capture: the 07 table read back with `1E 83`; `15` is handled
    as on the glass, the raw line read and the table computed and loaded, **never captured before a
    film**); `1E 86` must answer where the white read leaves the carriages (the carriage at 2058,
    the lid's unit at 1416 less the move back + 64), or nothing more moves; the image's `1E 21` (the
    dark read's block, sent again); the tone tables, linear; the start move (`Geometry::start_steps`
    with the film's 512-value ramp, `1E 05`); `1E 31` (the offsets and the image's gains), `1E 11`,
    `1E 21`, `1E 41` with the shading's scales in byte 15, `1E 42` and the shading table
    (`Shading::compute_by_parity` at base 4800, `compute` at 2400), `1E 43` (`0040` throughout for a
    negative, `8000` for a slide); the read's one-value ramp ((den x T / num) >> 4 of the image's T)
    and move (lines along + the lead, flag `08`); `1E 47`, the read; `1B 03` until at rest (the
    carriages go home by themselves: about 5 s in 74). The guard now lets the image's start move out
    once, then its read move once, in that order, and the 07 table for a film plan. A frame whose
    start move would be shorter than the ramps (the top 6 mm of the window at 600 dpi, 5 mm
    otherwise) is refused: never captured (the least start move seen is 267 steps beyond them,
    capture 74). Stopping during the image read is not sent as `1E 18` (no film capture was
    stopped): the read goes on to its end, then ends as cancelled; a read that fails is still
    stopped with `1E 18`, as on the glass, so that the scanner is not left sending.
    **The image put together** (same night; `scan::assemble_film`, measured with
    `captures/epson4490/tools/film_image.py`: each capture's raw image read, from the `.pcap`,
    against Epson Scan's TIFF by rank correlation, its curve being monotonic): the colours at 0,
    ydpi / 50 and 2 x ydpi / 50 lines as on the glass, the TIFF's first line the blue's first and
    its first pixel the read's first; read at 4800 across (2400 and 4800 dpi) the **odd** sensor
    columns (`c0 + i` odd) ydpi / 600 lines later, 4 at 2400 along and 9 at 5400 (on the glass the
    even ones), with the start column odd (17, 76) or even (77, 80, 81); a 2400 dpi film halved
    across with weights 1/4, 1/2, 1/4 around the read's pixel 2j + 1 (a pair's mean does 0.97 on a
    negative, the filter 0.998-1.000); 4800 x 5400 reduced to 4800 lines an inch by the glass's
    weighted mean (0.9975 against 0.9976 for the best linear interpolation tried: Epson Scan's own
    rule not found). The rule on the whole image (every fourth line and column), R G B: 74 1.0000
    0.9996 0.9970, 75 1.0000 0.9996 0.9969, 76 (1200 columns) 0.9966 0.9980 0.9970, 77 0.9999 0.9998
    0.9991, 78 0.9982 0.9990 0.9967, 80 0.9973 0.9998 0.9979, 81 0.9986 0.9999 0.9981, 17 1.0000
    0.9994 0.9967; the same size as Epson Scan's TIFFs but 78 (8 bits: Epson Scan asked 2308 pixels
    and saved 2307). The Rust and the tool's model give the same image bit for bit (17, 74, 77).
    **The command** (same night): `epson4490 film-scan DPI X Y W H OUT.png [--positive] [--bits 8]
    [--pixels] [--raw FILE] --move`, the area in millimetres in the film window or in pixels;
    without `--move` it prints the plan and what the guard says, and sends nothing (capture 74's
    request: columns 2379-4683, 898 lines of 576 pixels, start move 267 steps, read move 3720 with a
    lead of 128, as captured). **The first film scan of ours** (14 November 2026, night; Matteo at
    the scanner, captures 87-89, `05-catture.md`): capture 74's frame at 600 dpi. 87 stopped by
    itself after the lid unit's origin read: the light's edge at line 99 asked a move back of 264
    steps, never captured, and nothing more moved (the carriages left at 1994 and 2440; `probe` now
    asks `1E 86`, and the origin read's levels go to the log). The captures' copy of that read is
    not to be trusted past line 99 (USBPcap, packets cut at 65,508): the program's levels are. 88
    refused to start from (1994, 2440); the scanner then took the carriages home by itself. **89
    went through**: the edge at line 100 (252 steps), exposures 13,166 / 8814 / 5786 µs (Epson
    Scan's in 74: 13,550 / 9070 / 5953), 29.5 s, the carriages home; the image against Epson Scan's
    TIFF of 74 gives rank correlations of -0.99 a channel (ours linear, a negative), 7 lines (0.3
    mm) along from it (the strip put back by hand), the colours aligned. The edge moves by a line
    from one scan to the next (74: 101, 86: 100, 87: 99, 89: 100): the moves back of 216 and 264
    steps (the same move, 12 steps shorter or longer, the unit stopping inside the travel it has
    just made) are let out too, Matteo's decision (`guard::FILM_BACK_MOVES`); further ones are still
    refused. The white bytes' rule (2-4 units off) can wait for that scan's result.
    **Every film combination captured tried** (15 November 2026, Matteo at the scanner, captures
    90-96, `05-catture.md`): negatives at 1200, 2400 (16 and 8 bits) and 4800 dpi and a slide at
    2400, each Epson Scan's request, each through and the carriages home (60, 75, 78, 132 and 59 s);
    against Epson Scan's TIFF of the same request rank correlations of 0.94-0.99 a channel, the
    three channels at the same offset (no colour fringes, no comb from the staggered columns at
    4800), 0.1-0.35 mm from Epson Scan's (the film put back by hand). The slide came out mirrored
    left to right against capture 80 (put in the holder the other way: Epson Scan's own read of 80
    is not mirrored against its TIFF); mirrored back, 0.97-0.98, and scanned again turned in the
    holder (97) it came out as Epson Scan's, 0.98, unmirrored. On the way: the slide's red asked
    more than 16,363 µs at its third exposure round, a line time over the longest captured (25,344
    µs; Epson Scan's red 16,213 and 16,272 in 80 and 81), and the guard stopped it (94); a channel
    now stops at that limit (`film_cal::exposure_cap`) and the loop ends when nothing more moves
    (96: red 16,363). A scan stopped half-way leaves the carriages where they were; the next opening
    refuses to start from there, and the scanner then takes them home by itself (88, 95).
    **Slides at every resolution** (15 November 2026; Matteo at Epson Scan, captures 98-101,
    `05-catture.md`): Epson Scan's preview of slides and slide 1 at 600, 1200 and 4800 dpi, 16 bits,
    with their TIFFs. The same sequence as capture 80 at each; the red's exposure up to 18,168 µs,
    and at 4800 dpi window reads with a line time of 27,264 µs (the longest captured before:
    25,344): `guard::FILM_LINE_TIMES` and `film_cal::exposure_cap` follow it. The lines read are
    rounded up to a multiple of the motor ratio's denominator (9 at 5400: 101 read 5328, 76 8100),
    the staggered lines being ydpi / 600 (`film::geometry`; the model's `--check` 0 differences on
    all eleven film scans). Tests follow 99, 100 and 101 write by write to their last line (Epson
    Scan's image factor 10 in all three); the guard now lets out slides at 600, 1200, 2400 and 4800
    dpi, 16 bits. `film_image.py --assemble` against their TIFFs: 0.996-1.000.
    **Left of step 14**, what Epson Scan computes from its calibration reads (each in the
    captures with its reads, to rebuild the same way): N from the 150-line read; the line time T and
    the three exposures of `1E 21` (from reads of the window at T 9600 / 13,056 and exposures
    5760 / 9216: two or three rounds; slides differently from negatives); the gains (`1E 31`
    bytes 8-10, negatives only, steps of 5); the white read's and the dark read's `1E 21`; the
    `1E 42` blocks and the shading (as the glass's?); the gamma tables (`1E B3`: an inverted
    curve for negatives, the preview's levels for slides). Then the sequence in `scan.rs`. Old
    list, kept: the park, the
    unit's origin from the 150-line read, the `1E 31` search (what it aims at, from the reads),
    the dark read, the columns and moves at each resolution, the 4800 reduction along; each
    rebuilt from captures 74-81 by a test before anything moves. The first film scan of ours is
    then an exact replay of capture 74 (600 dpi, the shortest: `epson4490 replay ... --move`,
    the script made from the capture as for the glass's 24-26), Matteo told first; only then
    our own plan.
15. **The adapter in the contract**: `Adapter { area 68 x 236 mm, Slide and Negative, its
    resolutions }`; strips of six and four slides with `film::frames` as on the 3770; the 6 x 12
    cm frame is the "medium format" field already planned (below, "Behind trait Scanner").
    **Since the captures** (14 November 2026): the window is 2.7 x 9.33 in (68.6 x 237 mm,
    `film::ADAPTER_INCHES`), the same for strips and slides, rather than 68 x 236; the
    resolutions to declare are the captured ones (`guard::FILM_COVERED`: negatives 600, 1200,
    2400, 4800; slides 2400 only, so far); Epson Scan's frames are areas the user draws on the
    preview, no frame finding of its own was seen (`film::frames` stays the app's).
    **Done** (15 November 2026): the 4490 declares its adapter (`capabilities()`: `Adapter {
    film::AREA_INCHES, Slide and Negative, 600 1200 2400 4800 }`, slides at every resolution after
    captures 98-101) and its status says it is there (`adapter: Some(true)`: the unit is part of the
    lid); `Scanner::scan` turns a film request into Epson Scan's (`film::for_area`: read along at
    5400 for 4800 dpi, enough lines for the height asked, the image cut to it; always 16 bits, the
    8-bit samples from them) and runs `scan::scan_film` (a test: capture 74 through the contract, to
    its last line). The window is declared less its top 6 mm (`film::TOP_SKIP_STEPS`, 576 steps): a
    read starting higher needs a start move shorter than its ramps, never captured; only the
    holders' edge is there (the first frame of a strip at 8.8 mm, a slide at 31.8 mm). After a film
    the app's lamp timer sends nothing (`1E A2 00` with the lid's lamp chosen was never captured;
    the scanner switches it off by itself), and the next glass scan switches back (`1E 77`) and
    warms the glass's lamp. Memory: a film at 4800 dpi counts 13/8 (read along at 5400). Nothing new
    in the contract or the app; the 6 x 12 cm holder waits for a medium-format film and its capture.
    Not tried yet in the app on the scanner.
16. Phase 13's films (base, levels, colour) on the 4490's scans: nothing new in the app.
    **Material** (14 November 2026): Epson Scan's TIFFs of the same negative frame at 600, 1200
    and 4800 dpi (74-76), of frame 6 at 2400 dpi in 16 and 8 bits (77, 78) and of two slides at
    2400 (80, 81), in `captures/epson4490`, to set our film scans against once the film path
    scans; and the lid's lamp's warm-up to measure then (step 13). Not started.
    **Measured** (15 November 2026, no new scan: our 89-93 and 96 developed as the app does, set
    against those TIFFs; `05-catture.md`, "lo sviluppo delle nostre pellicole contro Epson Scan",
    tools in `captures/epson4490/tools/sviluppo`):
    - Epson Scan's TIFF is its raw read (its 74 and 77 reads, assembled, against their TIFFs: red
      the same, green within 2.8 %, blue 1 %): its development is the `1E 84` type 02 tables it
      loads before the image. For negatives they are **one fixed curve**, the same for the three
      channels, both frames and every resolution (74, 75, 77, 78: 4095 up to input 10, 0 from
      1781): no levels of its own, its colour balance only the exposures per channel (green and
      blue at 0.669 and 0.439 of red's), which ours repeat. So its negatives come out dark and
      green ("corrections off"), and say nothing about which colour is right: nothing to copy, no
      colour matrix to declare for the 4490's negatives.
    - Ours on the 4490's data: one frame found in each scan (Epson Scan's area lies inside a
      frame, no gap: the base from `base_auto`, the unexposed band at the area's top); frame 1 at
      600, 1200 and 4800 dpi gives the same measures (base within 3.5 %, highlights within 0.006
      in density). The film base sits at about a quarter of full scale (15,300 / 12,600 / 11,200),
      the holder at 100-200: the densities above the base are not affected. Highlights beyond
      `high`: 0.4-0.9 % a channel inside the frame, as the rule means (the 3.7 % at full scale
      in the whole image is the opaque holder at its edges, white in the positive). The scene's
      range computed 0.79 and 0.89 decades, taken to the minimum of 1.0 (highlights 0.4-0.6
      above the base; other rolls on the 3770 0.85-1.12): thin negatives or the lid's diffuse
      light, not told apart without the same strip on both scanners.
    - Slides (96 against 80, the same slide): Epson's tables differ by channel and slide (its
      automatic levels; white 2867 / 2806 / 2836 of 4096 in 80, gamma about 1.8-2.0); ours (one
      black and white, 67 and 29,233) clip 0.02-0.06 % in the middle against its 0.01-0.04 %,
      midtones lighter (gamma 2.2): alike to the eye.
    - The shading's white bytes (2-4 units off Epson's): on the film base, the column-to-column
      structure is ours 0.79-0.82 % against Epson's 0.84-0.88 % at 600 dpi, 0.94-0.97 % against
      0.96-0.99 % at 2400, even and odd columns alike within 0.05 %: no difference in the
      images, the exact rule left.
    So nothing changes in the films' development for the 4490. Left: the lid's lamp's warm-up,
    with Matteo at the scanner (`captures/tools/lid_lamp_series_epson.sh`).
    **The lid's lamp, a first series** (15 November 2026, capture 104, frame 1 at 600 dpi as 89,
    `scansioni/epson_coperchio_20261115_1122`): the scanner's warming flag went after 19.2 s; the
    scan calibrated then and the one a minute later, each with its own calibration, differ by
    1.1-1.4 % in every channel and band (the first lighter), the red's exposure found 3 % shorter
    (12,398 to 12,014 µs). The third, 90 s in, stopped before any move: the brighter lamp moved
    the origin read's edge by a line (the 255s to line 102, the next 135 above the threshold of
    125), a move back of 204 steps, never let out; the carriages left at 1994 / 2440. **Done**:
    204 let out (`guard::FILM_BACK_MOVES`, Matteo's decision: a line short of 216, the unit
    stopping at 1212, inside its travel); the same can happen in the app to a film scanned a
    minute after another. The series to be run again from a cold lamp.
    **The lid's lamp from cold, measured** (15 November 2026, capture 105, the scanner switched
    off and on and left 20 minutes; eleven scans of frame 1 at 600 dpi from 0 to 14 minutes after
    the lamp was lit, `scansioni/epson_coperchio_20261115_1152`, `tools/sviluppo/riscaldamento.py`):
    the warming flag went after 19.3 s; every scan went through (moves back 228, 216, once 204
    at 7 minutes: the edge moves either way). The calibration follows the lamp: the red's
    exposure 12,014 µs for the first 2.5 minutes, 12,398 to 5, 12,782 at 7, 13,166 from 10 on
    (yesterday's warm 89 the same), the gains moving by a step now and then. Against the mean of
    the last two (13-14 minutes; those agree within 0.03 %), each scan differs by **a scale per
    channel**, the same in the shadows, midtones and light parts: red +2.3 to +3.9 % for the
    first 7 minutes, green and blue -2 to +2.4 %, in the calibration's steps (at 10 minutes, the
    lamp steady, a gain one step off still gave blue +2.4 %). Developed as the app does (each
    scan its own base and levels), the scale goes: the pictures differ from the steady ones by
    at most 1.0 level of 255 on average (the first scan's blue), the others 0.5 or less, below
    the two steady scans' own difference (1.5-3.7 levels per pixel, noise). **So no wait after
    the scanner's flag** for films (as Epson Scan); nothing changes. Assumed, not measured: a
    roll's base locked from another scan, or a slide (one white for the three channels), keeps
    such a scale, 0.01-0.017 in density, about the 0.6-4 % the 3770's scans differ by anyway,
    and a warm lamp's gain steps give it too.

**Where the films stand, in order** (14 November 2026, night):
1. Done: captures 72-81 (82, medium format, when Matteo has a film); the film guard
   (`47322f3`); the lid's lamp both ways (`38b779b`, tried on the scanner).
2. The film path's rules (step 14, "Left of step 14"), each rebuilt from 74-81 by a test,
   nothing sent.
3. The first film moves of ours: an exact replay of capture 74 (`epson4490 replay ... --move`),
   Matteo told before; then our own plan through the guard, opened one combination at a time.
   **Replay done** (14 November 2026, evening; captures 85, 86; `05-catture.md`, "i nostri primi
   movimenti per le pellicole"). Learnt the hard way (85): **the film moves are relative to
   where the carriage and the lid's unit are**, and the scanner moves them itself when it lights
   the lid's lamp again (after `1B 40` with that lamp chosen and gone off: the carriage to 1994,
   the unit to 1078, as after `1E A2 02` in capture 72, where Epson Scan then skips its park
   move). The first replay took `1E 86`'s answers as "any" and ran 1994 / 1078 steps further
   than Epson Scan (the carriage to 4052 rather than 2058; normal sounds, the carriage home after,
   the data nearly saturated). The replay script now has the positions exact and stops before
   the first move if they differ (`tools/script.py`). Then (86) the exact replay: every
   answer as in 74, 29 s; the calibration reads within 0.2-0.4 % of Epson Scan's (correlation
   0.94-0.998), the image 2.5 % lighter (the strip put back by hand). So: **the film rules take
   the positions as input** (the park move = 1994 less where the carriage is; the unit's moves
   likewise), and the driver's film scan asks `1E 86` before every move, as Epson Scan does.
4. The adapter in the contract and the app (step 15), then phase 13's films on its scans
   (step 16) and the lid's lamp's warm-up measured; then Digital ICE (step 17).

**E. Digital ICE** (milestone 6)

17. The second pass (`1E 57` mode `20`, source `03`, capture 18), the infrared channel in the
    contract and the dust removal in `scan-imaging` (already planned below).

**F. Release** (milestone 7)

18. "Firmware missing" as a state of its own and the firmware's path in the Preferences (left
    on purpose, above); packages, install guide, CHANGELOG. **Since**: the firmware's path is no
    longer to be a preference (`fe81d4e`, 10 November 2026: the copy in the settings folder is
    taken first, the repository's copy is embedded in every build); the udev rule is in the
    packages and in the install guides (`docs/install.md`, `install-linux.md`). Left: "firmware
    missing" as a state of its own, the 4490 in the user's guide (`docs/usage.md` does not name
    it), the release.

**Not planned**: the optional document feeder (Matteo does not have it); Epson Scan's draft
preview mode (the app's preview at 200 dpi takes about 25 s for the whole glass, as WIA's
capture 06); the 9600 dpi along of the user guide's "Micro Step Drive" (never seen in a capture,
interpolated as far as known).

**The contract and the app, in short**: for the glass nothing new (steps 1-11 fit today's
`Capabilities`, `Progress`, `LampInfo`, cancelling); three new fields, all already planned below:
the infrared channel, the medium format, "firmware missing". The app follows the `Capabilities`
by itself (buttons, lamp choices, 16 bits, films); its only new screens are those three.

**How long, roughly** (estimate of 13 November 2026, night, asked by Matteo; an order of
magnitude from the pace of 12 November, when the native scan, the lamp, 16 bits, 1200 dpi,
cancelling and the recovery took about a day, and from how much of each step is still unknown;
not a measure). The hours count the work and Matteo's time at the scanner where a step needs it:

| Step | What | Hours | What can make it longer |
|---|---|---|---|
| 12 | Linux: our upload (both copies), `nusb`, a scan, the app, the recovery | 3-6 | USB permissions; how the scanner behaves after our upload on Linux |
| 10 | Stripes and colour measured on the 4490 · **done 14 November** | 3-6 | a colour target is needed |
| 11a | 2400 dpi: a capture, the guard, the reduction in the driver · **done 14 November** | 3-5 | the rules are there already (capture 13) |
| 11b | 4800 dpi: another path, its own captures and rules · **done 14 November** | 6-12 | other columns and line times, all to understand |
| 11c | A size limit in the app (files of gigabytes) · **done 13 November** | 3-6 | the app keeps the whole scan in memory today |
| 13 | The lid's lamp, switched both ways | 2-4 | |
| 14 | The film path: rules from captures 16-19, the guard | 6-12 | two carriages, another motor: with care |
| 15 | The adapter in the contract: strips, slides, medium format, e2e | 4-8 (was 6-10) | the contract's `Adapter` and the app's film mode exist already (the 3770's): new are the 4490's declaration and the medium format |
| 16 | Phase 13's films on the 4490's scans | 3-6 | measures on real negatives |
| 17 | Digital ICE: the infrared pass, the channel in the contract, the dust removal | 15-30 | the least certain: the protocol, then an algorithm that really works |
| 18 | Release: "firmware missing", the 4490 in the user's guide, the release | 3-6 (was 4-8) | the firmware's path is no preference since `fe81d4e`, and the packages have the udev rule already |
| 8 | The native scan at 16 bits run on the scanner at 100, 200, 300, 400 and 1200 dpi (only 600 so far) | 1-2 | missing from the first estimate |
| 1 | The lamp switched off by the scanner itself and lit again, measured as the cold lamp was (one point only) | 1 | missing from the first estimate |
| — | Small open things: the STI lock busy two minutes after a recovery (capture 50, not explained), `rustfmt` not installed in WSL (checked 13 November) | 1-3 | |

About 55-110 hours in all as first estimated; checked step by step on 13 November against the
code, the commits and the capture notes, steps 15 and 18 come out smaller (above) and steps 8
and 1 had been left out, so about 54-107: the glass complete (12, 10, 11, 8, 1) about 20-38, the
films (13-16) about 15-30, Digital ICE alone 15-30, the largest and least certain step (put off,
the total is about 39-77). About half of it, the captures and the measures, needs Matteo at the
scanner.

Checked as still to do (13 November): 12 (no commit since the Linux note of 12 November), 10
(the 4490 declares no stripes and no colour), 11 (the guard covers 100-1200 dpi only), 11c (the
app shows a scan's memory but limits nothing), 13 (`1E 77` only in the guard's list of refused
movers), 14-15 (`transparency: None`), 17 (no infrared in the contract).

**Not counted here**, open elsewhere in this plan: phase 10's blue on a negative's base, light
along the strip and lower resolutions for films; phase 13's steps 4 (stalled), 7 and 8; the
three coloured lines and the calibration that failed after a power cycle (the stripes of the
glass, above); phases 8 and 9 (postponed).

Added the same night, after the code review ("Open after the code review" below; these hours
are a new estimate, not part of the first): the 4490's state asked before its STI lock, a
capture with Epson Scan scanning while VetroScan starts and the change tried, 2-4; the error
paths never tried on the two scanners, 1-2; the PDF's colour thresholds measured on real
documents, 1-2. With them, about 58-115 hours.

### Behind `trait Scanner`

- A **new backend crate** with its own `Backend` for the `Registry` (`name()` "epson-gt-x750";
  its USB layer like `hp3770-usb`'s, shared if it fits), `list()` by VID/PID, `models()` with the 4490's
  `Capabilities`, so the app shows it before it answers. App, imaging and export do not change.
- The 4490's `Capabilities`, to be confirmed on the device: glass 8.5 × 11.7 in; the
  resolutions the native modes give (no interpolated 6400/9600/12800); depths 8 and 16;
  `transparency` an `Adapter` of 68 × 236 mm with `Slide` and `Negative` and its own
  `resolutions`; the four buttons (with suggested actions); `lamp` only if the host switches it;
  its `Tone` measured; `stripes` measured with "Misura le righe".
- **Capabilities rule**: the app shows only what these declare, as for the 3770 and the made-up
  scanners: no lamp timer if `lamp` is `None`, only these buttons, film only with the adapter.
  New functions come with the field in `Capabilities`, the branch of the app and two e2e
  (a made-up scanner with it, one without):
  - **infrared** (Digital ICE): a fourth channel in `ScanResult` and its use in `scan-imaging`
    (dust found where the infrared is dark); until then the 4490's films scan without it;
  - **medium format**: an `Adapter` whose film is not only 35 mm (`film::frames` knows 35 mm
    strips and slides only);
  - **firmware missing**: a `DeviceStatus` state (or an error at open) the app shows as "needs
    its firmware" with where to get it.
- A **made-up 4490** (`scanner_api::fake`) with these capabilities for the e2e, before the
  hardware.

### Risks

- **Licences**: the firmware's (above); the copy from Epson's Windows driver only in the
  private repository and in the builds kept by Matteo, never in public packages.
- **The Windows driver** is from 2009: it may not install or not scan on Windows 11 (Epson
  says it does); without it the captures cannot be made, and the SANE way on Linux is left.
- **Two carriages**: the transparency unit has its own moving lamp, which must follow the
  scanner's carriage; a wrong command could strain either motor. Nothing moves before it is
  understood.
- **Sizes**: 4800 dpi at 16 bits gives files of gigabytes; the app keeps whole scans in memory
  today, so the largest areas need a limit or a different path.
- **A used scanner**: lamp age, the locks, the state of the glass and of the transparency
  unit's light; the first scans are a check of the scanner too.
- **It may not get to the end**: the protocol is learnt only from the cable (and, if that is not
  enough, from Epson's library), with no open map like `hp3900` for the 3770. VueScan's own
  driver shows it can be done; how long it takes is not known before the first captures. The
  milestones are ordered so that each one left behind is still useful.
- **Done when**: plugged into Windows (with or without Epson's driver), Linux or macOS, the
  4490 is found by VetroScan, takes its firmware by itself and scans the glass at every native
  resolution, 8 and 16 bits, and a strip of negatives; smoke tests on the real scanner like
  the 3770's; Digital ICE is a later step, not part of this.

Sources: sane-devel, August 2005 (Meeuwissen, `alioth-lists.debian.net/pipermail/sane-devel/
2005-August/014432.html`, `014444`, `014475`); Fedora's `iscan-firmware` spec and review (bug
483868); fedora-license-data issues 608 (AVASYS) and 610 (LICENSE.EPSON); the AVASYS Public
License (mirrored in ChromiumOS' `portage/licenses/AVASYS`); `iscan` sources
(`github.com/ip1981/iscan`: `backend/epkowa_ip_api.h`, `backend/get-infofile.c`,
`non-free/COPYING.EPSON.en.txt`); Epson's support page for B11B176011; the Epson Perfection 4490
user's guide (specifications, transport locks); PhotoReview's review of the 4490.

## Phase 13 · films done professionally, for every scanner · decided 10 November 2026

Matteo's question, the same day: are we tuning a generic app to one scanner? The work on films
(the blue's noise, an exposure per colour, slides later) must make sense with several
scanners, for professional use. Below what was measured, the rule decided and the order.

### What was measured (10 November 2026, the HP 3770, 1200 dpi, notes in `captures/tma/note.txt`)

Five strips, each scanned twice in a row without touching it (the two scans 3-5 lines apart):
the film's grain is the same in both, so their difference is the scanner's own noise alone.

- **The film's base sets where each channel starts, not the picture**: on two colour films
  the base lets through about 75 % of red, 29 % of green and 14.5 % of blue (blue/red 0.20 on
  both); the second film's mask is denser, mostly in green (green/red 0.39 against 0.34). A
  chromogenic black-and-white film (orange base, grey picture) has blue/red 0.30. The same
  strip scanned on two days gives the same base and densities within 0.01-0.02.
- **The picture sets how far below the base a channel goes**: blue is lowest in the light,
  bluish parts of the scene (dense in the negative's yellow dye).
- **The noise in the light parts is the scanner's, not grain**: on a colour film's bright sky
  the positive has 4-6 / 8-9.5 / 11-12 levels of 255 (R/G/B), on a light blue T-shirt
  3.9 / 5.8 / 12.1; in both, practically all of it is in the difference of the two scans
  (signal/noise of the samples: red ~33-43, green ~17-26, blue ~13-18). So more light on the
  sensor, or averaging scans, would really help; a gain would not (it scales the noise too:
  the same strip with every number doubled had the same signal/noise).
- **Taking the noise out of the colour only** (Cb/Cr, a 2.5-pixel Gaussian, a try in Python)
  brings the T-shirt to 3.9 / 4.1 / 4.3 and the sky to about 5 / 5 / 5, edges and detail
  untouched (`scansioni/tma/confronto_maglietta_100.png`, `confronto_croma_100.png`).
- HP's own configuration exposes the colours of a negative differently (`[ScanNegColorMode1]`:
  the line 71399, red 50999, green 30599, blue 71399, the whole line): blue longest. Why green
  gets the shortest is not understood, nor how the chip uses `EXPT`/`MEXPT` (phase 10).

### The rule: four levels, no scanner's numbers above the driver

| Level | What it holds | Example |
|---|---|---|
| **Driver** (one per scanner) | the physics: lamps, calibration, line timing, safety limits, and how a generic request becomes the chip's registers | the 3770's white 81/83/70 on the slot; the blue's exposure on `EXPT`/`MEXPT` |
| **Contract** (`scanner-api`) | what the scanner can do, said generically | sources, resolutions per source, stripes per source, an exposure per channel up to ×N |
| **Imaging** (`scan-imaging`) | algorithms that set themselves from what they measure on the scan | film inversion from the base; the noise reduction from the noise it estimates |
| **App** | the workflow and the choices, taken from capabilities and measures | "this roll's base is so, so expose so" |

Nothing measured on the HP 3770 goes into the app or the imaging as a number. The
capabilities rule (above) holds for every new function: field in `Capabilities`, the app's
branch, e2e with and without it.

Checked on 10 November: the app's film code has nothing of the HP 3770 (its only mentions of
HP are two texts: the lamp's timeout and the sharpness "as HP"); the film imaging works from
what it measures. One gap: the stripes are declared per resolution, not per source.

### In order

1. **Colour-noise reduction for films, generic** (`scan-imaging`) · **done on 10 November
   2026** (`scan_imaging::chroma`: the colour noise against the brightness's measured on the
   picture, the ratios red/brightness and blue/brightness smoothed with a Gaussian of
   1.5-1.8 px on the five strips, at most 3; tick **Riduci il rumore di colore** in Pellicola,
   on by default; blue 10-11 -> 5.4 levels on the T-shirt and both skies). The noise of each channel
   estimated on the scan itself (a robust measure on smooth parts, approximating what the
   difference of two scans gave), and the colour (not the brightness) smoothed as much as
   needed to bring the noisier channels near the quietest; a scanner with less noise gets less
   of it by itself. In the films' development by default, light, with a tick; the strength
   used kept with the page, so a page saved again comes out the same. Checked on the five
   strips.
2. **Stripes per source** in the contract · **done on 10 November 2026**
   (`Stripes { source, dpi, repeat }`, `Capabilities::stripes_at(source, dpi)`; the 3770's
   adapter declares its own, `Transparency::stripes`: every 5 lines at 600 and 1200 dpi, both
   measured on negatives; "Misura le righe" only replaces the glass's; an e2e with an adapter
   that declares none). Was: (`Stripes { dpi, repeat }` gets the source, or
   `Capabilities::stripes_at(source, dpi)`): the 3770 declares the same for glass and films
   (same sensor, measured); the 4490 has its own lamp in the lid and may have other stripes or
   none. "Misura le righe" stays for the glass, with a film measure later if needed.
3. **The base locked per roll** · **done on 10 November 2026**
   (`film::measure_strip_with(.., base)`: a base given is used as it is, the highlights and
   black still measured inside the strip's frames; `Strip::gaps`, the base between the frames
   when none is given. Pellicola, Scansione tab, block **Rullino**: "Blocca la base
   dell'ultima striscia" (only after a strip whose base was measured in its gaps), the base
   shown R/G/B, "Nuovo rullino" to unlock; `StoredMode::roll_base` in the preferences of the
   Film mode; `hp3770 develop --base R G B`; an e2e with the adapter). Measured on the strips
   of 10 November (1200 dpi): the same strip scanned twice in a row gives gap bases 0.6-4 %
   apart (the scanner's level from one scan to the next), so a base locked on one scan moves
   another scan's picture by 1-2.5 levels of 255 on average; a single frame without gaps,
   with its base guessed from its own film (`base_auto`), is off by -0.04 to +0.08 in density
   and shifts its picture by up to 12-16 levels in one channel (a cast). Stored with the
   pages: left to step 7 (the raw scan kept with its base in the metadata). Was: measured
   once (on a strip's gaps or a blank leader) and used for every frame of the same roll, so
   frames of one roll come out alike; a new roll measures again. Stored with the pages and in
   the preferences of the Film mode, shown and resettable.
4. **An exposure per channel in the contract**: `Capabilities` says the scanner can expose the
   channels differently, up to which factor and at which resolutions; `ScanRequest` asks for
   the factors; the app computes them from the roll's base (the dimmest channel brought up, the
   brightest never above the calibration's white); each driver turns them into its registers.
   For the 3770 first from the captures (`EXPT`/`MEXPT`), only within line periods already run
   on the motor (the 3x line at 1200 dpi was run on 8 November); anything else guarded before
   anything moves. A scanner without it keeps step 1 alone.
5. **Slides, and more than one exposure**: no orange mask, but a far larger range; the
   professional tool is a normal and a long pass merged (multi-exposure), which is step 4's
   exposure used twice. **Measured on 10 November 2026** (two slides, 1200 dpi, three scans
   without touching them; `captures/tma/misura_diapo.py`, notes in `captures/tma/note.txt`):
   a quarter of a slide's pixels lie beyond density 2 in red and green, 3-4.5 % beyond 2.5;
   with the glass's line (HP's `[ScanTAColorMode]` is the same) the signal/noise is 37-42 in
   the light parts, 6 at density 1.5-2, 3 at 2-2.5 and 1.3 beyond (the scanner's black, about
   120-220 samples with a noise of about 140 per pixel); the negatives' longer line (already
   run at 600 and 1200 dpi) halves the noise in the shadows (6-7.5 at density 2-2.5) for 20 s
   more, with nothing saturated. `film::frames` finds real slides in their mounts
   (35.1 x 23.9 mm). In order:
   1. **the adapter's area for slides** · **done on 10 November 2026** (the profile's
      `Transparency::area` 76.2-183.0 mm down, HP's end; `tma-scan` with the slides at 1200
      dpi then found both whole, 35.1 x 23.4 and 35.1 x 23.9 mm, in 89 s; a strip of
      negatives too, both frames whole and measured as before, in 122 s: its holder sat about
      8 mm lower than in the morning, its window ending at about 179 mm, so the old area would
      have cut its second frame as well). Was (driver, first: without it the second slide is
      cut by 5 mm):
      the profile's area, 76.2-171.6 mm from the glass's top, was the window of the
      negatives' holder seen on 8 November, not the end of the light. With the slides' holder
      the light reaches about 188-190 mm (`tma-scan --height 130`); HP's own area is 42.3 x
      93.1 mm starting `TA_StripToScan` = 1060/2400 in (11.2 mm) below the calibration strip,
      about 89.9-183.0 mm (read in `hpgt3770.dll`'s `GetXPACapabilities`; which of the three
      `TA_StripToScan` values is the 3770's is inferred, 1800 or 1920 would cut the negatives'
      window). The area becomes 76.2-183.0 mm (or HP's, without the slot), checked with both
      holders; the frames' detection and the e2e follow;
   2. **slides in the contract and the app** · **done on 10 November 2026** (checked on the
      scanner: `tma-scan --film slide` at 1200 dpi, now the longer line as the app, both
      slides whole, 34.5 x 23.9 and 34.5 x 23.4 mm, in 121 s, gains 0, highest sample 39270,
      nothing saturated, the positives right; measured as the app does, inside the slides):
      `Source::Slide` among the 3770 adapter's sources, the driver asking the negatives'
      longer line for it too (`session_request`, so the same guard on 600/1200 dpi; `tma-scan
      --film slide` as well); no stripes declared for slides at first (none found on the two
      slides' busy crops); **declared on 10 November 2026** after Matteo saw them in the app:
      on the skies of both slides (`diapo_app_1200dpi`) every 5 lines, 0.73 and 0.89 % from row
      to row, 0.26-0.32 % after removal, so `Transparency::stripes` is declared for both films
      (600 dpi measured on negatives only, the same modes). Imaging:
      `film::Measures` (a negative's or a slide's) and `film::measure_slides`, the levels
      taken inside the slides found (`inside`): on real slides the mounts are 116-224 samples,
      above the opaque level, so they would be the black. App: **Nel supporto** Negativi /
      Diapositive in Pellicola only where the adapter declares both (an adapter with slides
      only gets them without asking), kept in `StoredMode::slides`; Rullino for negatives
      only; the page scanned whole remembers its film for "Separa". The made-up scanner has
      `with_slides` (two slides in the holder, `Controls::load` for the strip); e2e with and
      without the choice;
   3. **more than one exposure** (step 4): a normal and a longer pass merged, for the shadows
      beyond density 2.5; only on line periods already run, measured before. **A longer single
      pass measured on 10 November 2026, and left out**: the three times longer line the
      negatives had on 8 November (`scan_modes::NEGATIVE_TRIPLE_1200`, 1200 dpi only), on the
      two slides with `tma-scan --triple-line`, two scans with each line (notes in
      `captures/tma/note.txt`, comparisons in `scansioni/tma/confronto_riga3x_*.png`):
      - the shadows' signal/noise 1.2-1.35 times higher at density 2-2.5 and about 1.5
        beyond, the scanner's black 1.4 times less noisy (`tools/measure/noise.py`), 151 s
        instead of 120-129;
      - **but a ceiling**: with the triple line the converter saturates on full light and the
        shading brings it back to about 34-37 thousand, where the twice longer line puts the
        same light (the holder's thin gap between the slides) at 53-60 thousand. The two
        slides stayed just below it; the ceiling is at a density of about 0.23, where the
        light parts of many slides lie (white skies, clouds). Used for slides on the same
        evening (c620baa) and taken back (e662caf) when found. A negative's base (density
        about 0.12) is above it as well, as the saturation of 8 November showed.
      So a longer line is worth it only merged with a normal pass that keeps the light parts:
      this step, the normal line for everything brighter than the longer pass's ceiling, the
      longer one below it, scaled by the ratio measured between the two. **Tried as a
      prototype the same night, and set aside by Matteo** (notes in `captures/tma/note.txt`,
      `scansioni/tma/confronto_doppia_ombre.png`): merged pairs gave the shadows 1.25-1.4 times
      the signal/noise, visible only lifted at 100 %; but the passes move between them by
      0.3-1.3 rows untouched, each slide differently, and half a pixel left made one zone
      worse; the light between passes differs by 5-10 %; and the hand-over on the light parts
      could not be seen (the two slides lie under the ceiling). Too much risk of double edges
      or a change of tone in the picture for a gain seen only lifted. Back to it only with a
      sub-pixel alignment per frame proven on slides with bright parts.
6. **The colour of films, per source** (new, from HP's driver, 10 November 2026): HP's WIA
   driver (`hpxp3770.dll`, its resource `HPSJ3770`, Huffman-compressed, decoded in
   `captures/tma/driver_hp`) corrects every source with a 3x3 colour matrix: the glass's
   (`FB`), the slides' (`SLIDE`, with offsets and gammas per channel in `SLIDECOEFFS`) and the
   negatives' (`POST`: 2.632 -1.636 0.005 / -0.175 1.229 -0.054 / -0.211 0.069 1.142, with
   `NEG6X3` and `NEGCOEFFS`: slopes and gammas per channel). For negatives it is the
   correction of the dyes' crossed absorptions that a professional inversion does and ours
   does not (our colours come only from the base and one level per channel). By the four
   levels such numbers belong to the driver, as a colour profile per source in the contract
   (`Capabilities`), the imaging applying whatever matrix it is given, none by default; the
   way to measure them for any scanner is step 7's IT8 target. HP's matrices work on HP's own
   pipeline (8 bits, after its inversion), so they are a hint, not numbers to copy: first a
   try with `hp3770 develop` on the strips of 10 November, judged by Matteo, before anything
   in the app. **Tried on 10 November 2026** (`develop --matrix`, nine numbers row by row,
   `--matrix-gamma` 1 or 2.2, after the inversion and before the colour noise; comparisons in
   `scansioni/tma/matrici`, notes in `captures/tma/note.txt`): `POST`'s rows add up to 1, so
   grey stays grey on our positive (the chromogenic black and white strip stays grey); the
   mean saturation inside the frames goes from 0.22-0.38 to 0.29-0.52, the pixels with a
   channel clipped from 2 to 8 % on the flowers; greens and the blue T-shirt look truer, reds
   pushed hard (an indoor frame turns orange); half of it (`I + (POST - I) / 2`) in between.
   `SLIDE`'s rows add up to 0.87-0.94 (HP follows it with `SLIDECOEFFS`), tried with each row
   scaled to 1: a slide's lavender sky turns blue, 2.8 % clipped (0.5 % at gamma 2.2). Linear
   or gamma 2.2 nearly the same. **Slides done on 10 November 2026** (Matteo chose them,
   negatives left without: `POST` whole is too strong on our inversion and half of it would be
   a number of ours, not a measure): the contract's `Capabilities::colour`
   (`ColourCorrection { source, matrix }`, `colour_at(source)`), `scan_imaging::colour::
   ColourMatrix` applying whatever matrix it is given; the 3770's driver keeps HP's `SLIDE`
   as HP wrote it (`Transparency::slide_matrix`, thousandths) and declares it for
   `Source::Slide` with each row scaled to 1, at gamma 2.2 (0.5 % clipped against 2.8 %
   linear); the app's tick **Correggi i colori dello scanner** in Pellicola, only for a film
   the scanner declares a correction for, on by default, kept in `StoredMode::scanner_colour`,
   applied after the slide is stretched and before the colour noise. The made-up scanner's
   slides have a made-up correction; an e2e checks the tick and the colours. Negatives: none
   until measured (step 7's target; an IT8 is printed on slide film, so it fits slides first).
7. **Later**: the raw linear scan always kept with its scanner, calibration and base in the
   metadata (partly there: `-16bit.png`, `develop`, RAW), so a frame can be developed again;
   an ICC input profile per scanner from an IT8 target, outside the scanners' code; dust and
   hairs (many on the blue-sky strip of 10 November): with an infrared channel where the
   scanner has one (the 4490's Digital ICE, a capability in the contract), else found in
   software on the picture (white specks on a positive), generic in `scan-imaging`.
   From HP's driver too (10 November 2026), to look into when useful: `TMAPwmDuty=36 40 48`
   (perhaps the adapter lamp's brightness: more light is the only real gain for the
   shadows, measured; to be measured on the slot, within HP's values, before any use);
   `TA_FIND_EDGE` (HP aligns the area on the holder's edge; our frames are found on the film
   anyway); HP's white for negatives on the slot, 142/155/144 against the slides' 81/83/70
   (only a scale: measured that gain does not improve the signal/noise).
8. **The measuring box of a film's levels** (proposed 10 November 2026, not done: Matteo said
   "not now"): `film::inside` takes 0.9 of each frame and its densest pixels can lie on the
   dark band by the holder's edge; on `pellicola2` two scans in a row gave highlights 0.91 and
   1.12, with 0.85 0.03 apart, the other four strips unchanged (`captures/tma/misura_luci.py`).
9. **A preview for films, and several areas on every preview** (asked by Matteo on 15 November
   2026, done the same day in the app only, no new move of either scanner): at 2400 and
   4800 dpi a strip scanned whole is gigabytes (the 4490's holder at 4800 dpi about
   13,000 x 45,000 pixels, 3.5 GB of 16-bit samples) and the app refuses it, so the only way to
   use those resolutions was a frame at a time, drawn by hand on a scan at 600 dpi. Now, as
   Epson Scan does: the preview of the whole holder at the lowest film resolution the scanner
   declares (`App::preview_dpi`: 600 dpi on both scanners, a resolution and source each has
   scanned from the app), the film measured on it (`Negative`/`Slide`, kept for every frame,
   as before 9 November), the frames found and **one scan per frame chosen**, the frame and
   1 mm around it, cut and straightened. Without a preview Scansiona still scans the whole
   holder and finds the frames in it.
   The same model for the three ways of working (`areas.rs`): the single selection and the
   originals found became one list of areas on the preview (`App::regions`, `Region { at:
   Found, auto }`), each its own image or page. An area is chosen with a click and edited
   with the handles, the keys and the millimetre fields, turned with **Inclinazione**; one is
   added by dragging on an empty part (`Message::Select` adds when none is chosen) and left
   out with Delete, **Togli area** or **Escludi**. Areas drawn by hand and upright are
   scanned exactly as drawn (a whole page, which "Separa" can cut later); the originals found
   are cut 0.4 mm inside and straightened as before. The automatic crop off hides the
   originals found and keeps those drawn. Passes (`App::plan`): on films always one per area;
   on the glass the originals found in one pass when it fits in `ONE_PASS` and none is drawn
   by hand, else one each. "Rifai" on a page scanned whole scans its own area
   (`Whole::area`), no longer the one chosen now. The made-up scanner records every request
   (`Controls::requests`); five e2e check films (negatives, slides, a frame left out,
   corrected, drawn), Foto and Documento.
   **Not tried on the scanners yet**: a frame scanned alone at 2400/4800 dpi after the 600 dpi
   preview on the 4490, that the frame lands where the preview showed it (the 1 mm margin
   absorbs a small offset) and that the colours match a whole-strip scan developed with its
   own measures (the measures of a 600 dpi preview applied at 4800 dpi; on the 3770, 600 to
   1200 dpi worked before 9 November).
   **Tried by Matteo the same day on the 4490** (two strips of six frames in its holder): the
   frames scanned one by one came out right, but the preview stayed negative and showed six
   frames of twelve. A scan of the whole window at 600 dpi (`epson4490 film-scan 600 0 6.1 68.5
   230.8`, `scansioni/epson_anteprima_pellicole_600.png` and `.bin`) showed why it was not
   developed: `film::base_between` took the base between consecutive frames in reading order,
   which with two strips side by side go from one strip to the other: its median was the
   holder between them (0.001), and the measure failed. Now only frames next to each other on
   one strip (one along the other's length, less than a frame away), and a gap mostly not film
   is left out; on that scan the base comes out 0.221 / 0.175 / 0.158, against 0.233 / 0.192 /
   0.171 on single frames of the same scanner (base_auto, 15 November), and the picture is
   right. The six frames were **not reproduced**: on that scan `film::frames` finds all twelve
   at 600, 300 and 200 dpi (the app's nearest-pixel reduction), from 45 % to 130 % of the light,
   and as slides it finds two pieces, not six; to see again with Matteo's next preview.
   **Then the colours** (Matteo, the same afternoon: "the photos now have a wrong white
   balance"): with the preview measuring, the twelve frames were developed with one measure of
   the whole holder, where before (the measure failing) each frame scanned alone measured
   itself. On the 600 dpi scan the two strips are of two rolls: bases 0.245 / 0.191 / 0.166 and
   0.214 / 0.168 / 0.153, highlights 0.64 / 1.06 / 1.07 and 0.51 / 0.65 / 0.53; the whole
   holder's (0.58 / 1.00 / 1.02) is nearly the outdoor strip's, and turned the indoor one orange
   (`scansioni/confronto_bilanciamento_4490.png`: whole holder, each frame alone, each strip).
   Chosen with Matteo: **each strip on its own** (a roll's base is one; frames of one roll
   alike, single frames corrected in Regola), as on the HP 3770's one-strip holder:
   `film::strips_of` (frames along one line), `measure_strips_with`, `positive_by_strip` (each
   pixel with the nearest strip's measures); the app keeps the preview's strips on the bed
   (`App::strips`) and develops a frame scanned alone with its strip's, a holder scanned whole
   strip by strip. And a likely cause of the six frames: the made-up two-strip holder
   (`FakeScanner::with_two_strips`) showed light across the holder (its lit slot) joining the
   strips into one wide piece, its frames sought across both; `film::split_strips` now splits a
   piece wider than a strip and a half where, across it, the holder is dark. Not proven to be
   Matteo's case (his 600 dpi scan finds twelve either way).
   **Then per frame** (Matteo, tried the same evening: "the white balance per strip is not as
   per frame, before it came out much better"): one white per strip is set by its brightest
   frame (the left strip's sky) and leaves the others dark and flat. Chosen with Matteo
   (`scansioni/confronto_bilanciamento_4490_b.png`: B all per frame, D base per strip and
   levels per frame, C all per strip): **D**, the base the strip's (a single frame measured on
   its own took its shadows for the base: frame 5, 0.214 / 0.157 / 0.133 against its strip's
   0.245 / 0.191 / 0.166), the white and black each frame's: `film::measure_frames`, the app's
   `App::frames` from the preview, `positive_by_place` (each pixel with the nearest frame's
   measures); an area drawn by hand measured on itself on the nearest strip's base (or the
   roll's). **And a crash** at the third frame at 4800 dpi (WER: 0xc0000409, fast fail 7, an
   abort: a failed allocation or a panic, the message lost without a console): the first
   `positive_by_strip` held five whole-frame buffers, 1351 MB for a 4800 dpi frame against
   408 MB (measured, a 4913 x 7181 frame); rewritten with a table per place, 408 MB. Not proven
   to be the crash; a panic hook and every scan asked now go to `tasti.log`.
   **And the screen against the file** (Matteo, the same evening: the file saved does not look
   as in VetroScan). Measured: PNG and TIFF are the app's finished image to the pixel (JPEG
   0.3-0.5 levels on average), no colour profile in any; the screen was drawn from `Raw::proxy`,
   8-bit linear, through the 8-bit tables, the file from the 16-bit samples through the fine
   ones: on the 4490's twelve frames (600 dpi) the shadows 1.2-4.6 levels darker on the screen
   on average, up to 20. `Raw::proxy16` (the small copy at 16 bits, `shrink16`, `at_dpi16` for
   the preview) and `render_raw` draw the screen as the file; an e2e holds them within one
   level (25 levels apart without it). Left: the sharpening works in pixels of what it is given,
   so at 4800 dpi it shows stronger on the small copy than in the file; not measured.
10. **"Senza elaborazioni"** (asked by Matteo on 15 November 2026, done the same day, app
    only): for every scanner and way of working, the equivalent of a scanner's own software
    with every correction off, as Epson Scan's negatives were found to be (one fixed inverting
    curve). `Settings::untouched`, saved per way of working (`StoredMode::untouched`, absent in
    older files: off): `App::cleaned` returns the scan as it came (no stripes, no glass colour
    matrix), `App::doubles` is off, films go through `App::plain` instead of `develop`
    (negatives only inverted at the films' gamma, one less the negative's light shown, the
    orange base left in; with `keep_negative` not inverted; slides as read), and pages scanned
    so render with no adjustment and no sharpening (`untouched_look`, in `Page::effective` and
    in the preview's `render`). The calibration and the scanner's tone (its white and gamma)
    stay: they are how the scanner reads, not a correction. The frames are still found on the
    preview; nothing is measured. Also picked photos (15 November): Ctrl/Shift+click and
    Ctrl+A in the strip, `Page::picked`, "Salva" and "Applica alle foto scelte" on those only.

## Phase 14 · colour profiles: files that say what their numbers are · decided 15 November 2026

Asked by Matteo on 15 November 2026: a film saved by VetroScan looked different in Windows
Photos than in VetroScan, while LightCraft showed it as VetroScan does.

**Measured** (the same day):
- the files are the app's image: PNG and TIFF equal to `Page::finished` to the pixel, JPEG
  0.3-0.5 levels apart on average (compression); the screen was a separate matter, drawn from
  an 8-bit linear copy (fixed the same day, phase 13 step 9: `Raw::proxy16`);
- the same photo as an 8-bit PNG (`scansioni/prova_stessa_foto_8bit.png`, the TIFF's values
  to the bit) looks in Windows Photos as the 16-bit TIFF does, and in LightCraft as both look
  in VetroScan: the difference is between programs, not between files or depths;
- Matteo's monitors are wide gamut: an EIZO ColorEdge CG277 with a profile of his calibration
  (17 October 2023) and an Alienware AW3423DWF. Windows Photos manages colour (an untagged file
  read as sRGB, converted to the monitor's profile); VetroScan sends its numbers as they are,
  so on such a monitor they show more saturated than they are (general knowledge, not
  measured here; checked only by eye);
- **Epson Scan tags every TIFF** (all its captures in `captures/epson4490`, 8 and 16 bits, glass
  and films) with "EPSON Standard RGB - Gamma 1.8": a monitor-class ICC v2.2 profile of 628
  bytes, the sRGB primaries (rXYZ 0.4361 / 0.2225 / 0.0139, gXYZ 0.3851 / 0.7169 / 0.0971, bXYZ
  0.1431 / 0.0606 / 0.7141, D50-adapted), white D65 (0.9505 / 1 / 1.0891), one gamma of
  1.80078125 for the three channels. **VetroScan tags nothing**, though its numbers mean
  different things: the 4490's glass is at gamma 1.8 (its `Capabilities::tone`, Epson Scan's
  values copied), films at 2.2 (`film::TONE`), the HP 3770 at 2.2. An untagged file is read as
  sRGB (about 2.2): the 4490's glass scans show their midtones darker than meant elsewhere (a
  grey meant at 29 % of the light shown at 22 %; worked out from the gammas, not measured on a
  scan), where Epson Scan's own TIFF of the same scan is read right.

**Steps, in order:**
1. **The files say what they are.** Every file written with an ICC profile describing its
   numbers: the sRGB primaries and white, and the gamma of the tone the page was drawn with
   (`Raw::tone.gamma`: 1.8 for the 4490's glass, 2.2 for films and the HP 3770), generated by
   VetroScan from those numbers (Epson's file is Epson's: not copied); a grey profile (`kTRC`)
   for grey images; none for black and white. In TIFF (tag 34675, also the 16-bit and
   multi-page writers), PNG (`iCCP`), JPEG (APP2), WebP, AVIF where the encoder takes it, and
   the PDF's images if the PDF writer allows it (`/ICCBased`); the numbers do not change. The
   gamma comes from the page (a driver's `tone`), so the app and the export know no scanner:
   generic, from what each driver declares. Tests: each format read back with its profile, the
   profile parsed (primaries, white, gamma), a 4490-like made-up scanner at 1.8 and the
   flatbed at 2.2. Then by eye with Matteo: a 4490 glass scan from VetroScan and Epson Scan's
   TIFF of the same original side by side in Windows Photos and LightCraft.
   **Done** (15 November 2026): `vetroscan_export::icc` makes the profiles (monitor class,
   v2.1, sRGB primaries D50-adapted, white D65, one `curv` gamma in u8Fixed8: 1.8 is written
   1.80078125 as Epson's, 2.2 is 2.19921875; grey: `kTRC`); every writer takes a
   `Tagged` image (the image and its page's `Raw::tone.gamma`, films `film::TONE`): PNG
   `iCCP` (the 16-bit PNG's `sRGB` chunk gone), TIFF 34675 as UNDEFINED (8 and 16 bits, each
   page of a multi-page one), JPEG APP2, WebP (an RGB profile for grey too: WebP stores
   colour), the PDF's pictures `[/ICCBased n 0 R]` (one object per gamma and space, shared by
   the pages; a picture turned grey for the PDF gets the grey one). Not AVIF nor BMP (no place
   in their encoders), not black and white. The RAW TIFF's linear profile is the same bytes as
   before (a test), the DNG unchanged. **Measured**: littleCMS (Pillow's `ImageCms`) converts
   to sRGB through our 1.8 profile exactly as through Epson Scan's (`15_epsonscan_600dpi_48bit
   .tif`): grey 64/128/192 to 81/146/203, (200, 80, 40) to (210, 99, 53), both. Tests: every
   format read back with its profile at 1.8 and 2.2, the pixels unchanged, the PDF's shared
   profiles; in the app (e2e) the flatbed at 2.2 and the same at 1.8 (`FakeScanner::
   with_gamma`) in TIFF, JPEG, PDF, grey and black and white, and a 1.8 scanner with an
   adapter: its glass at 16 bits 1.8, its frames 2.2. Decided with Matteo the same day: the
   4490's glass stays at 1.8, as Epson Scan, no choice in Preferences. **To try by eye**.
2. **"Senza elaborazioni" and RAW**: the same for the files without processing; RAW (DNG,
   linear TIFF) keeps its own linear description (to check that DNG's colour tags say so).
3. **The screen managed** (larger, later, to decide with Matteo): VetroScan converts what it
   shows from the page's profile to the monitor's, as Windows Photos does, so that on a wide
   gamut monitor the app shows the colours as they are. Needs the monitor's profile (Windows:
   its association in the registry, read without `unsafe`; macOS ColorSync; Linux colord), or
   one chosen by hand in Preferences as a first step, and a colour engine in pure Rust (for
   example `qcms`, to be checked: licence, 16-bit, speed on a 2400-pixel copy). Until then:
   the CG277 in its sRGB mode shows VetroScan as Windows Photos does (to try).

   **Measured for the design** (15 November 2026, Matteo's machine; a probe in the
   scratchpad, nothing in the repo): the conversion of Epson Scan's 1.8 profile to each
   monitor profile installed, four colours (grey 64/128/192, red 200/80/40) and the time on
   2400 x 1600 pixels at 8 bits, against littleCMS (Pillow's `ImageCms`) as the reference:

   | Monitor profile | littleCMS | qcms 0.3.0 | moxcms 0.8.1 and 0.9.2 |
   |---|---|---|---|
   | sRGB | 146 grey, (210, 99, 53) | the same, 10 ms | the same, 10 ms |
   | CG277 calibrated, `XYZLUT+MTX` v2.2 (1 MB, LUT) | 145 grey, (188, 104, 64) | the same, 98 ms | **wrong**: grey 128 to (104, 106, 97) |
   | CG277 `Custom` v4.2 (matrix) | 145, (184, 106, 62) | the same, 10 ms | the same, 11 ms |
   | AW3423DWF v2.0 (matrix) | 146, (191, 112, 63) | the same, 10 ms | the same, 11 ms |

   So the engine is **qcms** (Mozilla's, Firefox's, pure Rust, MIT; 8-bit only, enough for
   the screen); moxcms (already in the build through `image`) gets the LUT profiles wrong.
   The grey stays grey on both monitors (calibrated to 2.2, D65): what differs is the
   saturation, (210, 99, 53) sent as it is where (188, 104, 64) is meant on the CG277.
   Windows' associations (`HKCU\...\ICM\ProfileAssociations\Display\{4d36e96e-...}\0000`
   to `0009`, several profiles each, `MDM0`) do not say which key is which monitor: reading
   them needs more than `reg query`.

   **Design** (to decide with Matteo):
   - **One place**: every picture the app shows goes through `Picture::from_rgb_max`
     (`viewer.rs`: the page, its thumbnail, the preview, the result), after it is shrunk to
     the texture: there each picture is converted from its page's profile (`icc::rgb(tone
     .gamma)`, the bytes the file carries) to the screen's. The numbers of the page, the
     histogram, the colour picker and the files stay as they are: only the texture changes.
   - **Automatic, by default** (Matteo, 15 November 2026: "mi aspettavo che si autoadattasse
     in base al monitor e al profilo ICC che il monitor usa in quel momento"): the profile of
     the monitor the window is on, read again when the window moves to another monitor and
     when the app comes back to the front (a profile changed meanwhile).
     - Windows, **measured** the same day: PowerShell with `Add-Type` (`CreateDC("DISPLAY",
       device)`, `GetICMProfile`) and `Screen.AllScreens` gives each monitor's device name,
       rectangle and profile file: `DISPLAY2` (3440 x 1440 at 0,0, primary, the AW3423DWF)
       and `DISPLAY1` (1440 x 2560 at 3440,13, the CG277), both "CG277 #1 2023-10-17 22-49
       50cdm2 D6500 2.2 M-S XYZLUT+MTX.icm", as DisplayCAL's Profile Loader says. No
       `unsafe` of ours (the workspace forbids it), as `ram.rs` reads the memory; one call
       of about a second, on a thread, at the start and when the window changes monitor.
     - macOS: to look at whether the system already matches the app's window (a Metal layer
       with a colour space set is converted by macOS: then VetroScan converts to sRGB only),
       or ColorSync's profile of the display.
     - Linux: colord (`colormgr`) where it is installed; most desktops manage no colour.
     - **Never worse than today**: whatever cannot be read (no profile, a file that does not
       load, a system without the tool) gives *sRGB*, which only brings each page's gamma to
       sRGB's (the 4490's glass midtones right, colours as today). An error is said in
       Preferences, never a failure of the app.
   - **In Preferences**, "Profilo del monitor": *Automatico* (the default, saying which
     profile it found for which monitor), *sRGB*, *nessuno* (the numbers as they are, as
     today), or a profile file chosen by hand. Saved preference with its migration.
   - **The engine**: one `qcms::Transform` per (gamma, monitor profile), kept until either
     changes; applied in strips on every core (a LUT profile took 98 ms on one: the slider's
     redraw must not wait that), grey pictures converted as RGB.
   - **Tests**: the conversion against littleCMS's numbers above (the profiles generated, a
     wide-gamut matrix profile made in the test: no monitor file in the repo), *nessuno*
     leaving the bytes as they are, the preference and its migration, an e2e with a page at
     1.8 shown with *sRGB* (grey 128 shown 146) while its file keeps 128.
   - **Done, the first part** (15 November 2026): `vetroscan/src/screen.rs` (qcms 0.3 with
     `iccv4-enabled`), every picture converted in `Picture::from_rgb_max` from
     `icc::rgb(raw.tone.gamma)` to the screen's, the conversions kept per gamma and run in
     strips on every core; "Colori dello schermo" in Preferences (*Automatico* by default,
     *sRGB*, *Nessuna conversione*, *Un profilo scelto…*, "In uso: …" and what went wrong),
     saved as `monitor` (files before it: automatic, a test); on Windows the monitor of the
     window and its profile through PowerShell (`monitor.ps1`, `MonitorFromWindow`,
     `GetICMProfile`), asked at the start, when the window moves and when it comes to the
     front, one at a time; elsewhere sRGB, said so. Tests: qcms against littleCMS's numbers
     (a 1.8 page to sRGB, a 2.2 page to sRGB and to an Adobe-RGB-like matrix profile made with
     `icc::matrix`, within a level), strips equal to one piece, *nessuno* leaving the bytes,
     what does not load giving sRGB with a note; an e2e with a 1.8 scanner: sRGB before any
     monitor is known, the monitor found (a made-up wide profile) and every picture redrawn,
     *Nessuna conversione* giving the numbers, remembered, back to automatic; the page's
     numbers unchanged throughout. **Tried** by Matteo the same evening on his two monitors
     (the release build): "Funziona". macOS and Linux to look at (macOS may convert the window itself; colord).
   - **Found on Matteo's machine**: the AW3423DWF has the CG277's profile assigned (Windows
     and DisplayCAL both say so), so every colour-managed program converts for the wrong
     monitor there; its own `AW3423DWF.icm` is installed. To set by Matteo (Windows' colour
     management or DisplayCAL), or measure the AW3423DWF with the colorimeter. Still to know:
     whether Windows 11's automatic colour management for apps is on for either monitor (then
     the system converts the sRGB itself, and VetroScan must give sRGB there, not convert
     twice).
4. **Later, with step 7 of phase 13**: a scanner's own measured profile (IT8 target) in place
   of "sRGB primaries" for each source.

**Done when:** a scan from each scanner and source opens alike in Windows Photos, LightCraft
and a colour-managed editor; the 4490's glass scans match Epson Scan's TIFFs in a managed
viewer.

## Open after the code review of 13 November 2026

The review of every crate (13 November 2026, commits `3fa8116`, `22e24fd`, `aed84fc`,
`af8caa5`, `6b9ae1f`, `2646915`) fixed what it found; these are what it left to try or to
decide. None changes the commands a scanner gets in the normal cases.

**On the scanners** (each only an error path, never tried):
- HP 3770: `head_park_home` clears the run bit of `E800` when the chip still runs 15 s after a
  scan was cancelled (at 200 or 300 dpi, where nobody reads it): never tried on the scanner. The
  new endings of a scan after an error (stopped, parked, the adapter's lamp off) have no
  automated test: a try with the scanner, an error forced half-way.
- Epson 4490: `1E 18` after a calibration read that fails (the white strip, with the carriage
  moving) was never sent so; nor the refusal when `1E 86` does not give the carriage at home
  before the calibration (every capture gave 0 there).
- **Epson 4490, a risk left open**: opening the scanner asks `1B 03` and reads before taking
  the STI lock, as it must to stop a scan a program left half-way (captures 44-53). If Epson
  Scan is scanning at that moment (VetroScan starting, or trying again after its open answered
  busy), that read can take a packet of Epson Scan's image, and the opening then stops its scan
  with `1E 18`. Only made rarer (VetroScan now tries again every 3 s after ten tries). To decide
  the order (the lock first, and the stop only when no live program holds it): a capture of
  Epson Scan scanning while VetroScan starts, then the change, tried.
  **Captured** (14 November 2026, Matteo at Epson Scan, 1200 dpi, 48 bits, the whole glass, a
  white sheet; `captures/epson4490/05-catture.md`, 70-71): Epson Scan reads its image with no
  command between the blocks. With the lock first (`epson4490 probe`, capture 70) Windows did
  not give the scanner for the whole 20 s (`CreateDevice` or `LockDevice` failing, so
  `StiLock::take` finding nothing to take) and nothing went out: no write from 19.6 s, when
  Epson Scan began reading, to its end at 329 s; Epson Scan's scan finished, the image sound.
  Without the lock (`probe --no-lock`, 71, the opening's first step today without its `1E 18`)
  our `1B 03` went out between two of Epson Scan's reads: the scanner answered with a 512-byte
  packet of the image (our read took `96 9D`, the rest lost), sent one more piece of 1,340
  bytes to Epson Scan and then nothing; Epson Scan waited about 143 s and said it could not
  talk to the scanner (switch it off and on). `epson4490 recover` then (`1E 18` unlocked):
  3,699 bytes left out, the carriage home in 3 s, `18 81`, the lock taken again at once; no
  power cycle. So the risk is real even without the `1E 18`. **Done** (14 November 2026): with
  a lock other programs take (`Transport::locks`, the STI lock) the opening takes the lock
  before writing anything, and a lock held is "in use by another program" with nothing
  written. A scan of ours marks itself in the temporary folder while it lasts (`mark.rs`: a
  file held open without sharing, which Windows closes when the process ends however it
  ends), in the app and in `epson4490 scan`; a mark left by a process gone means a scan of
  ours left half-way, and only then the state is asked before the lock and the scan stopped,
  as since captures 52-54; the mark is cleared once settled. Left as with Epson's own driver:
  a scan of another program left half-way (Epson Scan killed while reading), where taking the
  lock has Epson's driver send `1B 06` (capture 44). Tests: the lock first with nothing
  written, busy with nothing written, a left mark settled before the lock, a scan marked while
  it lasts. **Tried** the same afternoon (the release build of `55c447e`): VetroScan closed
  half-way through a 1200 dpi scan left its mark (and the scanner half-way; one `probe
  --no-lock` sent by mistake then, as capture 46); opened again it said "collegamento" a while,
  the carriage went home, the mark was gone and the scanner ready. Not tried in the app:
  opening it while Epson Scan scans (capture 70 ran the same lock call, nothing written).
- HP 3770 on Windows: the reason a scanner is not found (in use by another program, access
  denied, through `Backend::why_none`) shown in the app with the 3770 held by another program.

**Tried on the Epson 4490 the same afternoon** (Matteo at the scanner; the release build of
`98a9f39` and after): Cancel during the two-minute warm-up stops at once; a whole preview after
it came out normal, the first full run of the review's code. The preview of three photos found
**one** original, the whole glass, with the code of before the review too: the 4490's lid
darkens towards its left and top edges over 10 mm (207 inside, 145 at the edge, measured on
that preview) and the lid's colour was taken 1-2.5 mm in; taken 1-10 mm in, the preview finds
the three photos, in reading order (tried in the app); the two previews of the 3770 kept from
8 November give the same as before. To check on the 3770 with separate photos.

**Tried with the lid open the same evening** (the Epson 4490, Matteo at the scanner):
- **In the app, an error of the scanner's own.** A preview with the lid open: the lamp did not
  light, nothing moved, after a while a click and the front light flashing red; the app said
  "ready" with `1B 03` answering `98 81`, a state never seen in a capture (`18 81` at rest). The
  bit `80` was ignored: the scan's wait took `98 81` as ready (`0x43` clear) and so did the
  status. Opening Epson Scan cleared it (no power cycle), and Epson Scan then scanned with the
  lid open. **Done**: `State::error` (`0x80`), the scan's every `1B 03` wait stops at it with
  nothing more sent (a test, the scanner of capture 29 erring at four points), the status is not
  ready and shows it, `epson4490 probe` prints it. **Not understood**: what set it. Right after,
  with the lamp warm and Epson Scan closed, `epson4490 scan 200 0 0 215.9 296.9` (the app's
  preview) with the lid open ran whole and normal (`1E A1` answered `00`, Epson Scan had
  switched the lamp off: lit by the scan, two minutes, 145 s in all); the earlier try was less
  than 15 minutes after the three photos' preview, so not the lamp switched off by the
  scanner. With no log of the app's try, the step where it started is unknown; the next time
  the scan's error says `98 81` where it stops. To try again in the app, with the lid open,
  after the lamp has been switched off by the app's lamp choice and by the scanner's 15
  minutes.
  **Seen again on Linux** (13 November 2026, evening, lid closed, a photo on the glass): the
  app had been used (a preview, a scan, closed half-way through a scan, opened again, a
  preview, closed: closing switches the lamp off with `1E A2 00`); then `probe` gave `18 81`
  and `epson4490 scan 100 40 60 50 50 --bits 16` stopped at `98 81` before the lamp was lit
  (no "lamp on" in its log: within the opening's `1E 53`, `1B 40`, `1B 13`, `1E A1`, the
  gamma, `1E 57`). Switched off and on, the firmware sent again, the same scan ran whole.
  Both times the app had used the scanner just before; not shown to be the cause. Now the
  scan's error ends with the last command sent ("(after 1E 57)"; a test).
  **Seen again, and caught** (14 November 2026, morning, Windows, captures 66-69 in
  `captures/epson4490/05-catture.md`): the app opened, its lamp switched off by its lamp
  choice, then a preview found `98 81` before sending anything (the last command the buttons'
  poll); lid closed, nothing touched (Matteo). Epson Scan opened then (66) sends `1B 40` at
  once: `06`, the carriages home (Matteo saw), `19 81` with `1E A1` answering `02` (the lid's
  lamp selected) and warming, the lid's light on for a moment, no lamp lit after; **with
  Epson Scan alone, idle, `98 81` again** a couple of minutes later (67: only `1B 03` and `1E
  85` sent). Reset once more (68, lid open, then closed, 69): ready again in under two
  minutes. In capture 24 (12 November) the same warm-up after `1B 40` lasted 53 s. So the
  error is the scanner's own, during the warm-up of the lamp it selected, with or without our
  driver; the cause is not known (the lid's cable was firmly in; an intermittent lamp supply
  is a guess). **Done**: at a scan's opening `98 81` gets `1B 40` (`clear_error`, as Epson
  Scan), 7 s, then the state asked every second as a warm-up (stoppable) until at rest; an
  error again ends the scan, to be switched off and on (tests). The opening's wait is one
  `1B 03` read whether or not there is an error (`wait_state`), so the replays of the captures
  are unchanged. Not yet seen on the scanner: the error does not come on demand.
- **The search with the lid open, on the real scan** (`scansioni/epson_coperchio_aperto_
  20261113.png`, the empty glass): the room is dark (blocks of 6-32 levels of 255, a smooth
  lighter band down the middle) and the search found **one** original, the whole glass: its
  edges passed for the lid against the image's own brightest block. **Done**: the lid must also
  be above a third of the scale (the Epson's closed lid is 145 at its darkest edge, the 3770's
  255); now none, the three photos' preview still three, the 3770's two previews still 5 and
  1, and a test made on the measured scan.

**On real scans** (in software, made on made-up images so far):
- the originals on the glass in reading order (a row is the objects that overlap its topmost
  one by half), and the search with the lid open, which now finds nothing (both tried on the
  Epson, above);
- the PDF's grey pages below 100 %: a page stays in colour when the mean colour of any square
  of 16 pixels is more than 10 levels from the paper's tint. Both numbers are estimates:
  measure them on real documents (a signature in blue ink, a red stamp, a page with only the
  coloured fringes of black text), as the `scanner-measuring` skill does. **Measured the same
  day** on the Epson 4490 (A4 at 300 dpi, 90 %): the page with a blue signature stayed in
  colour, the page of black text too, wrongly: its paper is 10 levels bluer than green, black
  none, and the rule took a dark neutral corner for colour (14 levels). Now the paper's tint
  is scaled to each square's brightness, and the threshold is 15: on the same two pages the
  text page comes out at most 9 levels off (grey), the signature up to 42 (75 squares beyond
  15). Two pages only: to measure again on more documents and on the HP 3770.

## Summary

| Phase | Version | Risk | Without new hardware? |
|---|---|---|---|
| 0 · PDF per page | 0.1.2 | low | yes |
| 1–4 · refactoring (done) | 0.2.0 | medium, covered by tests | yes |
| 5 · Italian and English (done) | 0.3.0 | low, covered by tests | yes |
| 6 · project in English (done) | with 0.3.0 | low | yes |
| 7 · new name + final check (done) | 0.3.0 | medium: updates and preferences | yes |
| 8 · other HP (postponed) | to be decided | high | no: testers needed |
| 9 · other brands (postponed) | to be decided | medium | eSCL yes, with a network scanner |
| 10 · slides and negatives on the 3770 (done: negatives; slides through phase 13 step 5.2; open: blue on the base, light along the strip, lower resolutions) | 0.5.0 (negatives), 0.6.0 (slides) | medium | yes: the 3770's own adapter |
| 11 · precise moves from the keyboard (done) | 0.4.0 | low | yes |
| 12 · Epson Perfection 4490 Photo, native, plug and play (milestone 1 written without the scanner; arrived 11 November 2026, the firmware upload confirmed by a capture; buttons; native glass scans at 100-1200 dpi, 8 and 16 bits, in the app with the lamp, cancelling and a scan left half-way stopped at the opening, 12-13 November; open: steps 10-18 of "What is left, in order", and "Open after the code review") | 0.5.0 (milestone 1, experimental); the glass in the app after v0.7.0, not released yet; the rest to be decided | high: protocol unknown, two carriages | no: the scanner (firmware inside the program) |
| 13 · films done professionally, for every scanner (done: 1-3, 5.1-5.2, 6 for slides; 5.3 set aside; 4 stalled without HP's captures; open: 7, 8) | 0.6.0 (1-3, 5.1-5.2, 6); 7, 8 to be decided | low (1-3, 5.1-5.2, 6, 7 in software, 8), medium (4, 5.3: the motor) | 1-3, 5.1-5.2, 6, 7 in software, 8 yes; 4 needs the 3770's captures |
| 14 · colour profiles in the files, then the screen managed (decided 15 November 2026; done: 1 (the 4490's glass by eye to try), 3 on Windows; open: 2, 3 on macOS and Linux, 4) | to be decided | low (1, 2), medium (3) | yes |
