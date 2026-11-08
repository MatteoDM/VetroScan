# Changelog

Every change that matters to whoever uses the scanner, version by version.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the version
numbers [Semantic Versioning](https://semver.org/).

Every change visible to whoever uses the app or the `hp3770` command goes under
`[Unreleased]` in the same commit that makes it, in one of *Added*, *Changed*, *Deprecated*,
*Removed*, *Fixed* and *Security*. At release time the section becomes
`## [X.Y.Z] - YYYY-MM-DD` and the links at the bottom are updated: the release workflow
publishes the section with the tag's number as the notes of the GitHub release, and stops if
it is missing.

The sections up to 0.2.0 were first written in Italian, as were their release notes on
GitHub; here they are in English.

## [Unreleased]

### Added
- **The app in English**: in the Preferences, «Language» chooses between *Automatic*
  (English if the system is in English, Italian otherwise), *Italiano* and *English*. The
  change applies at once, without restarting; numbers follow the language (2.5 in English,
  2,5 in Italian).

### Changed
- **The app is called VetroScan** (it was "HP Scanjet 3770", after the one scanner it knew):
  window, menu entry, installer, packages (`vetroscan-vX.Y.Z-…`) and the app's program
  (`vetroscan`). Installing it over an older version replaces it: the Windows installer
  stays in the same folder and replaces the Start menu entry, the Linux packages replace the
  `hp3770` package, and the settings and presets are copied from the old `hp3770` folder at
  the first start. The `hp3770` command and the HP 3770 driver keep their names.
- The status bar names the scanner in use next to its connection.
- The «Film» mode (disabled) only appears with a scanner that has the adapter for slides and
  negatives: the HP 3770 does not.
- The project's documentation is in English (README, guides, changelog); a short Italian
  README is in `README.it.md`. The guides have English names: `docs/install.md`,
  `docs/install-linux.md`, `docs/usage.md`, `docs/usb-protocol.md`.

## [0.2.0] - 2026-11-08

### Added
- The app's **version**, small, at the top right of its name.
- With several scanners connected, the **Preferences** choose which one to use.

### Changed
- The app shows only what the scanner in use can do: resolutions, glass, 16-bit depth,
  buttons and lamp come from the scanner, no longer from fixed values of the HP 3770. With
  the HP 3770 nothing changes.
- The actions of the **scanner buttons** are remembered for each model of scanner; those
  already chosen for the HP 3770 stay.
- **Each page has its own adjustments**: in the document «This page only» is ticked from the
  scan on, so changing colours or tones of a page does not change the others. With **Apply
  to all pages** the look of the page shown goes to all of them, to the preview and to the
  next scans; unticking it makes a page go back to the common adjustments.

### Fixed
- Sheet preview of the PDF: the red edge of the cut and the dimmed part stay on top of the
  page even where it overflows the sheet, instead of ending up under it.

## [0.1.2] - 2026-11-08

### Added
- **PDF, a page different from the others**: orientation, position on the sheet and margin
  per page («Own layout»), with **Apply to all pages** and **Like the others**.
- **The sheet in the viewer**: with the Output panel open, the page on its sheet as it is in
  the PDF, the margin dashed and in red the edge of what ends up on paper; with «Fill the
  sheet» the page is dragged to choose what to keep.

### Fixed
- PDF at actual size with a margin set: a page as large as the sheet is no longer cut at the
  margin (at actual size the margin does not apply).

### Changed
- When the app opens again the **adjustments** (colours, tones, curve, sharpness, black and
  white threshold) start from zero instead of coming back from the last time; resolution,
  colour, format, PDF and buttons stay as left. A look to reuse is saved as a preset.

## [0.1.1] - 2026-11-08

### Added
- **Lamp**: the app switches it off after a time without scans chosen in the Preferences
  (from «as soon as the scan is over» to 13 minutes, 2 by default) and always when closing,
  instead of leaving it on for 13 minutes; the scanner's timer stays as a safety net.

### Fixed
- Automatic crop: thick prints a few millimetres apart (the lid does not come down between
  them and leaves a grey shadow) were taken for a single original as large as the glass; now
  the lid's shadow does not count as an original, and each one comes out on its own without
  the band of shadow around it. A white sheet is still found from the dark line of its edge.
- The order of the originals found no longer puts a tilted postcard after the photos of the
  row below, into which it barely sticks out.

## [0.1.0] - 2026-11-08

First version.

### Added
- **Desktop app** `vetroscan` for Linux and Windows (on macOS it builds, but it has not yet
  been tried with the scanner), with two ways of working:
  - **Document**: one or more pages, saved as PDF, multi-page TIFF or one image per page;
  - **Photo**: one image per scan, also as RAW (DNG and linear 16-bit TIFF, with the chip's
    samples intact and the highlights recoverable).
- Preview of the glass with selection of the area (handles, moving, zoom) and automatic crop
  of the originals lying on the glass.
- Live adjustments that do not touch the scanner's data: exposure, contrast, point curve,
  noise reduction, descreening, adaptive threshold for black and white; Ctrl+Z, double-click
  to go back to the default value, up to three saved presets and factory presets.
- Scans in colour, grey and black and white from 200 to 2400 dpi, also at 16 bits per
  channel, with the same calibration and the same tone curve as the HP driver.
- The three **scanner buttons** (Scan, Copy, Instant Share) work with the app, in the
  background too, and are set per mode; Copy prints a photocopy.
- **Editor of scans already made**, in Document, Photo and in documents of several pages: it
  opens with «Edit» under the thumbnail, with a double-click on it, with «Edit…» on the viewer
  or with the E key. Changes show at once and always start from the scanner's samples, so they
  spoil nothing until you save; «Back to the original» goes back to the scan as it was. Its
  own Undo and Redo (Ctrl+Z, Ctrl+Y), «Apply» (Enter) or «Discard» (Esc), «Before/After»
  compares with how it was when opened.
  - **Colour**: all the adjustments again (white balance and eyedropper, exposure, contrast,
    brightness, levels and gamma, curve, saturation, vibrance, sharpness, presets) and the
    switch to grey or black and white;
  - **Crop**: rotation by 90° and 180°, mirroring and flipping, fine straightening (±45° in
    steps of 0.1°) with a rule-of-thirds grid and automatic cut of the empty corners, crop
    with handles and fixed proportions (1:1, 4:3, 3:2, 16:9, A4, 10 × 15, like the original)
    upright or lying;
  - **Size**: in pixels, percent, centimetres or inches, with dpi and proportions that can be
    locked, Lanczos resampling; it applies to 16-bit and RAW files too.
- Light, dark or system theme; preferences saved.
- A **usage guide** of the app for people who are not technical, for Linux, Windows and
  macOS (now [docs/usage.md](docs/usage.md)), linked from the README and the installation
  guides.
- The **`hp3770`** command: `list`, `status`, `probe`, `buttons`, `init`, `lamp`, `park`,
  `scan`.
- Linux: a udev rule that lets the scanner be used without root, an entry in the
  applications menu with its icon, and in the app the button **Allow access to the scanner**
  when the rule is missing.
- Windows: works with the HP driver already installed (`usbscan.sys`), without changing
  driver, or with WinUSB.
- Before an action that would throw away scans not saved (going from Document to Photo, New
  document, a new photo, Delete, closing the window) the app asks: Cancel, **Save…** (saves
  and then goes on) or the action.
- **Duplicate** a page (also Ctrl+D): the copy uses the same scan but is cropped, turned and
  adjusted on its own, to keep separate several photos acquired together.
- The thumbnails of the pages are **dragged** to change their order.
- **PDF on paper**: A3, A4, A5, Letter, Legal, 10 × 15 or 13 × 18 cm sheet (or like the
  scan), automatic, portrait or landscape orientation (also for a single page), scan at
  actual size, fitted to the sheet or filling it, margin. The PDF is also in Photo mode, to
  print.
- **Size of the PDF**: quality from 30 to 100%, size estimated before saving and a limit in
  MB the app keeps to by lightening one page at a time, the one that saves the most.
- New icon, also in the window and in the Windows executable.
- Packages for every system, at every version: installer and portable zip for Windows, `.dmg`
  for macOS, `.deb`, `.rpm`, Arch package and AppImage for Linux (x86_64 and aarch64). The
  `.deb`, `.rpm` and Arch packages also install the udev rule.

### Fixed
- Two saves in the same second no longer overwrite each other: the second gets « (2)».
- «Redo»: the progress box said «Scanning page N+1» (the next new page) instead of the page
  redone; now it says «New scan of page N» and, when the scan is over, shows that page in the
  «Adjust» panel too.
- In Photo the messages of «Redo», «Keep the new one», «Keep the old one» and «Delete» talk
  about photos and not pages; saving no longer says «1 pages».
- «Cancel» during a scan or a preview is no longer an error: no «Scan failed» warning, the bar
  says «Scan cancelled».
- An original found near the right edge or at the bottom of the glass no longer makes the
  scan or «Redo» fail with «area … outside the glass»: the area asked for stays on the glass.
- The «TIFF» format with several pages or photos saves one file per page, like PNG and JPEG;
  the single TIFF stays with «Multi-page TIFF». JPEG and AVIF keep the mode's quality.
- Ctrl+Z after switching between Document and Photo no longer puts the settings of one mode in
  the other; Ctrl+Z on a page's own adjustments goes back on that page, not on the one shown
  at that moment.
- When changing mode, the pages that follow the common adjustments are redrawn with those of
  the new mode (before they stayed with the old ones until the next change); the same after
  «Restore factory settings».
- Moving or deleting pages while they are redrawn in the background no longer puts the
  thumbnail of a page on another; with a new scan waiting for a choice, moving the pages no
  longer makes «Keep the new one» replace the wrong one.
- During a scan mode and order of the pages can no longer be changed (the scan ended up in
  the wrong place).
- With the editor open, turning another page and then clicking the thumbnail of the one being
  edited shows it again (before nothing happened).
- The note under the pages names the scanner button that really adds a page (or scans a
  photo), as chosen in the Preferences, and disappears if none does.
- A preset saved with the name of a factory preset could no longer be applied: now the name is
  refused.
- Texts with a run of spaces in the middle (note on RAW, on 16 bits and on the buttons in
  Windows); the button of the default folder uses the system's separator.

[Unreleased]: https://github.com/MatteoDM/scanner/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/MatteoDM/scanner/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/MatteoDM/scanner/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/MatteoDM/scanner/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MatteoDM/scanner/releases/tag/v0.1.0
