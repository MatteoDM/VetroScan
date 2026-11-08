# Plan: from "an app for the HP 3770" to "an app for scanners"

Status: **phases 0–6 done (v0.1.2 and v0.2.0 published; phases 5 and 6 come out with
v0.3.0); next: phase 7** (plan written on 8 November 2026, after v0.1.1; updated the same day
with the languages, the new name and the postponed phases). Each phase ends with a commit,
green tests and a version; what is done is marked here.

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
  (`hp3770_usb::UsbBackend`, or the made-up scanner with `HP3770_FAKE_SCANNER`). The e2e run
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

## Phase 7 · a new name, not tied to the HP 3770 → v0.4.0

With several scanners the name "HP Scanjet 3770" no longer describes the app. **Decided by
the user (8 November 2026): retroscan.** It is written `retroscan` in binaries, packages,
crates and folders, "RetroScan" where it is shown (title, bar, menu). Before publishing,
check that it is free: crates.io, GitHub, Flathub, AUR, Homebrew, registered trademarks.

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
- **Done when:** an installation of v0.3.x updates to the new one on Windows and on Linux
  (deb and rpm) keeping preferences and presets, and the final checks below are clean.

## Final coherence check (double) · after phase 7

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

## Summary

| Phase | Version | Risk | Without new hardware? |
|---|---|---|---|
| 0 · PDF per page | 0.1.2 | low | yes |
| 1–4 · refactoring (done) | 0.2.0 | medium, covered by tests | yes |
| 5 · Italian and English (done) | 0.3.0 | low, covered by tests | yes |
| 6 · project in English (done) | with 0.3.0 | low | yes |
| 7 · new name + final check | 0.4.0 | medium: updates and preferences | yes |
| 8 · other HP (postponed) | to be decided | high | no: testers needed |
| 9 · other brands (postponed) | to be decided | medium | eSCL yes, with a network scanner |
