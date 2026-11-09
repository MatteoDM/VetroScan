# Plan: from "an app for the HP 3770" to "an app for scanners"

Status: **phases 0–7 done and published (v0.1.2, v0.2.0, v0.3.0), with the double final
check; phases 8–9 postponed until there is hardware; phases 10–11 (films on the 3770,
precise moves from the keyboard) planned** (plan written on 8 November 2026, after v0.1.1;
updated the same day with the languages, the new name, the postponed phases and phases
10–11). Each phase ends with a commit, green tests and a version; what is done is marked
here.

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

## Phase 10 · slides and negatives with the HP 3770's adapter · negatives done

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
  Still open: the last few percent of light along the strip (a correction measured once on the
  empty holder, if it shows); blue's signal on a negative's base is the lowest (HP gives each
  colour its own exposure for that; not understood yet); slides not tried yet (their mode would be the glass's, the gains at their
  maximum: perhaps the negatives' longer line for them too); the contract and the app (below).
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
     tried by Matteo** the same day, it works;
  2. **blue on a negative's base**, the lowest signal: try HP's exposure per colour again
     (`NEGATIVE_EXPOSURES`, how the chip uses `EXPT`/`MEXPT`) or a longer line for blue only;
  3. the **light along the strip**: a correction measured once on the empty holder, only if
     the few percent show in the pictures;
  4. **slides**, when Matteo has some: `Source::Slide` in the 3770's `Adapter::sources`, its
     mode (the glass's line at the gains' maximum, or the negatives' longer line), `Slide` in
     `film`, an e2e like the negatives' one, a try on the scanner;
  5. lower resolutions for films (300, 200 dpi) only with a motor ramp or HP's longer lines,
     and never without a guard in `session::scan` first;
  6. a **release** with the Film mode (the `release` skill: tag, CI publishes), when Matteo
     wants it.
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
| 10 · slides and negatives on the 3770 (negatives done, slides open) | to be decided | medium | yes: the 3770's own adapter |
| 11 · precise moves from the keyboard (done) | to be decided | low | yes |
