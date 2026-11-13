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
- **VetroScan scans the glass of the Epson Perfection 4490 Photo**, with its own driver: 100,
  200, 300, 400, 600 and 1200 dpi, 8 or 16 bits, colour (grey and black and white made by
  VetroScan), any area of the glass. Every command is computed from the area asked and kept
  within what the captures of Epson's own drivers showed the scanner doing; the image comes out
  as Epson's driver's. After the scanner has been idle for a quarter of an hour, or just switched
  on, its lamp is given two and a half minutes to warm up first. No films yet. VetroScan switches its lamp off as for
  the HP 3770 (the Preferences' lamp choices, up to the 15 minutes after which the scanner does
  it by itself, and on closing).

### Changed
- A scan that would need more than half of the computer's memory is refused, with how much it
  would need and how much the computer has, instead of starting and perhaps closing VetroScan
  half-way. VetroScan holds up to about two and a half times the memory it shows for a scan
  while putting its image together (measured: 1.04 GB for a scan shown as 452 MB).
- VetroScan sends the Epson Perfection 4490 Photo its firmware when it opens the scanner and
  the firmware is not running (on Linux and macOS, where no Epson driver sends it at
  switch-on): a capture of Epson's driver showed the same bytes. `VETROSCAN_EPSON_UPLOAD` is no longer needed, and
  no longer read.
- The Epson Perfection 4490's four buttons (Start, Scan to PDF, Scan to E-mail, Copy) reach
  VetroScan. On Windows with Epson's driver, VetroScan takes the scanner for itself while it has
  it open, so Windows stops asking it for its buttons in between: Epson Scan cannot use the
  scanner meanwhile, and if Epson Scan is scanning, VetroScan says the scanner is busy.
- An Epson Perfection 4490 left half-way through a scan (VetroScan closed or stopped while
  reading) no longer needs switching off and on: opening it again stops that scan, the carriage
  goes home and the scanner is ready. VetroScan sends the scanner its firmware only when it
  answers as a scanner just switched on, never on an answer it does not know.
- The Epson Perfection 4490's glass is 216 x 296.9 mm in VetroScan (the user guide's 297.2 mm,
  less the last fraction of a millimetre no scan with Epson's driver was seen reaching).

### Fixed
- Epson Perfection 4490: a scan at 1200 dpi wider than about 115 mm no longer hangs before
  reading, leaving the scanner deaf until switched off and on with its carriage away from
  home: its shading table, over 64 KB there, now goes to the scanner in pieces with a short
  pause between them, as Epson's own driver sends it. Tried on the scanner on Linux and on
  Windows: 150 x 150 mm at 1200 dpi and 16 bits.
- Epson Perfection 4490 switched off and on with its carriage left away from home (after a scan
  that hung): VetroScan now sends it its firmware, which brings the carriage home by itself,
  instead of refusing an answer it did not know.
- HP Scanjet 3770: a scan that fails half-way (calibration, film adapter's lamp, image data)
  is now ended as a cancelled one is: the chip stopped, the carriage brought home and, with
  films, the adapter's lamp switched off and the flatbed lamp on again. Before, the adapter's
  lamp stayed lit and the carriage away from home.
- HP Scanjet 3770: if the carriage is still moving 30 s after setting off towards the film
  adapter's slot or the area of the scan, the scan stops there with an error instead of going
  on with the carriage in motion.
- HP Scanjet 3770: a scan cancelled at 200 or 300 dpi, which the scanner may keep running
  for lack of anyone reading it, is stopped after 15 s before the carriage is parked, instead
  of parking it on top of the running scan.
- HP Scanjet 3770: a button press is no longer seen as all buttons held when another program
  (Windows' scan app) resets the scanner while VetroScan reads its state.
- HP Scanjet 3770 on Windows with HP's driver: when the scanner cannot be opened, the message
  says why for each scanner device found (access denied, in use by another program, not an HP
  Scanjet 3770).
- HP Scanjet 3770: a film scan with the flatbed lamp off no longer waits 10 s for that lamp to
  warm up; the film adapter's own lamp is waited for as before.
- Epson Perfection 4490: Cancel works during the lamp's two-minute warm-up, not only after it.
- Epson Perfection 4490: a scan whose image data fails half-way (a timeout, a broken block) is
  stopped as a cancelled one is, so the scanner does not go on sending it into the next scan;
  and a scan whose carriage is not at home before the calibration stops there with an error,
  instead of moving the carriage further.
- The originals found on the glass, and the slides and frames found in the film holder, come
  in the same reading order every time: row by row from the top, left to right in each row.
- The originals on the glass of the Epson Perfection 4490 are found on the preview: its lid
  darkens towards the edges, and the search took that darker grey for the lid and the whole
  glass for one original (three photos came out as one). The lid's colour is now taken from
  1 to 10 mm in from the edges.
- With nothing like the lid along the edges of the glass (the lid left open), the search for
  originals finds none, so that the areas can be drawn by hand, instead of taking the whole
  glass for one original. This holds when the room seen through the open lid is dark and even
  too: the lid is now taken to be light against the whole scale, not only against the
  brightest part of the preview (tried with the Epson Perfection 4490's lid open on an empty
  glass).
- Epson Perfection 4490: when the scanner flags an error of its own (its front light flashing
  red), VetroScan no longer shows it as ready, says so in the scanner's details, and a scan stops
  at the first sign of it without sending the scanner anything more. Opening Epson Scan cleared
  it once; switching the scanner off and on clears it (tried on Linux). The scan's error now
  also names the last command sent before it, to tell where it came.
- 16-bit scans no longer have a few tones a whole 8-bit level off, which showed as spikes in
  their histogram.
- When no scanner is found, the reason the scanner's drivers know now reaches VetroScan: on
  Windows with HP's driver, a scanner device that another program holds or that access is
  denied to is named, instead of a bare "no scanner found". With several scanners that do not
  open, what each answered is kept, the busy one first.
- A film scan asked of a scanner without a film adapter is refused for that reason, not for
  its resolution.
- Saving over a file that is already there no longer destroys it when the save fails half-way
  (a full disk, an encoder error): every file is written beside it first and takes its place
  only once complete.
- A PDF below 100 % quality keeps a page in colour when it has any real colour on it, a
  signature in blue ink or a red stamp included; before, a page whose colour covered under
  0.3 % of it was stored in grey. A page of black text on a paper the scanner sees slightly
  tinted still goes grey (measured on the Epson 4490).
- Fitting a PDF to a size no longer lowers a page's quality by a step that would not make the
  file any smaller.
- A PDF with an empty page is refused with the number of that page, instead of a broken file;
  a multi-page TIFF with no pages is refused instead of written unreadable.
- Long PDFs take much less memory to save: they are written page by page.
- The settings and the presets are saved through a file beside them, so that a save cut short
  (the power going) no longer loses them; a settings file that cannot be read is kept aside, as
  `impostazioni.json.illeggibile`, instead of being written over with the defaults.
- A document saved at 16 bits with a black-and-white page is refused before any file is
  written, not after the pages before it.
- A ZIP holds the pictures in the format chosen (JPEG, WebP…), not always PNG.
- The pages of a document saved one file each are named alike in every format (`doc-01`,
  `doc-02`…); the 16-bit and RAW files were `doc-001`.
- A RAW (DNG or 16-bit TIFF) of a 16-bit scan puts the paper white where the same scan's 8-bit
  RAW does.
- With no scanner there, VetroScan looks for it every 3 seconds after the first few tries,
  instead of every 300 ms.

## [0.7.0] - 2026-11-11

### Added
- `hp3770 tma-scan --triple-line` (experimental, for measuring): films at 1200 dpi with three
  times the glass's line instead of twice, as the HP 3770 first scanned negatives; refused at
  any other resolution, before the scanner moves.

### Changed
- **Lamp · Switch it off** follows the scanner in use: its longest choice is the time after
  which the scanner switches its lamp off by itself, where it does (*the scanner switches it
  off*: 13 minutes on the HP 3770, as before), and the note says the scanner's own warm-up.
  The texts on 8 bits and on sharpness no longer compare them with HP's driver.
- On Windows, **Use the scanner buttons with this app only** is offered for any USB scanner
  whose maker's driver Windows has, found by its USB id, no longer only for the HP 3770.
- On Linux, **Allow access to the scanner** works for every scanner VetroScan knows (the HP
  3770 and the Epson 4490), no longer only for the HP 3770: it installs
  `/etc/udev/rules.d/70-vetroscan.rules`, a line for each, the same lines as the packages'
  rules. A `70-hp3770.rules` installed there by an older version keeps working.

### Removed
- The Epson Perfection 4490 firmware's path in the preferences file (`epson_firmware`, never
  in the interface): the firmware is inside the program, and a copy of `esfw54.bin` put in the
  settings folder is still taken first.

## [0.6.0] - 2026-11-10

### Added
- **Reduce the colour noise**, for films, on by default: the coloured specks in the light and
  blue parts of a negative's picture (the orange base lets little blue light through), measured
  on each picture and smoothed as much as needed, the brightness and its detail untouched. On
  real negatives the blue's noise came down from 10-12 to about 5 levels of 255, as red's and
  green's; a picture with little colour noise is left alone. `hp3770 develop --colour-noise`
  does the same.
- `hp3770 develop --destripe` takes the scanner's stripes away from a saved film scan before
  turning it into its picture.
- **Roll**, for films: the base of a strip (the bare film's orange, measured between its
  frames) can be locked with **Lock the base of the last strip** and is then used for every
  following strip of the same roll, which measures only its highlights and blacks; shown, kept
  in the preferences of Film, unlocked with **New roll**. A strip with a single frame no
  longer has to guess its base from its shadows (casts of up to 12-16 levels of 255 on real
  negatives). `hp3770 develop --base R G B` develops a saved scan with a base given, and
  prints the one between its frames.
- `hp3770 tma-scan --height MM` scans further down the adapter (10-130 mm from the top of its
  area), to measure where its light ends.
- **Slides with the HP 3770's adapter**: in **Film**, **In the holder** chooses **Negatives**
  or **Slides** (only for an adapter that takes both; remembered). Slides come out as they
  are, stretched to the full range with one black and one white for the three colours, each
  slide found in the scan its own image, measured inside the slide, away from the mount. They
  are scanned with the negatives' longer exposure, which on real slides halved the noise in
  their dark parts for about 20 seconds more. **Roll** is for negatives only.
- **Correct the scanner's colours**, for slides, on by default: the HP 3770 declares a colour
  correction for slides (the matrix of HP's own driver), which turns the lavender cast a slide
  gets from the scanner into its blue; untick it for the colours as the sensor reads them.
  Shown only for a film the scanner declares a correction for.
- **Remove the scanner's stripes** now also works on slides: the HP 3770 leaves the same lines
  on them as on negatives (every 5 lines, measured on the skies of two slides at 1200 dpi).
- `hp3770 develop --matrix` (nine numbers, row by row) and `--matrix-gamma` try a colour
  matrix on a saved film scan.

### Changed
- **Remove the scanner's stripes** now also works in **Film**: the HP 3770 leaves the same
  lines on negatives (every 5 lines at 600 and 1200 dpi), most visible in the light parts of
  the picture. They are taken away on the negative as scanned, before it becomes a positive;
  on a real strip they came out three to four times fainter, the colours unchanged. Off by
  default, as on the glass. A scanner declares its stripes for each source (the glass, films):
  the tick shows in Film only where it declares some there.
- The save window says what it saves: with several pages or photos, its title says whether
  they go into one file (PDF, multi-page TIFF) or one file each, numbered at the end of the
  name. It always saved them all, but it showed one name only, as if it saved one scan.
- The HP 3770's adapter is scanned 11 mm further down, to 183 mm from the top of the glass
  (HP's own area): its light reaches there with the slides' holder, and the second slide was
  cut by about 5 mm. A strip of negatives takes about a tenth longer.
- `hp3770 tma-scan --film slide` scans with the negatives' longer line, as the app does, no
  longer with the glass's (without `--film`, the glass's as before).

## [0.5.0] - 2026-11-10

### Added
- **Negatives with the HP 3770's adapter**, in the lid: the **Film** mode, next to Document
  and Photo, scans a strip of 35 mm negatives in the holder and gives them as positives,
  natural colours, each frame found in the scan its own image. No preview: the adapter's area
  is small and full of frames, and Scan finds them itself (a preview would only add a lamp
  warm-up). By default 1200 dpi (about 2 megapixels a frame), 16 bits, JPEG; films at 600 and
  1200 dpi. With the adapter's cable unplugged the Film button says so. Slides are not
  supported yet. Every scan with the adapter waits about 30 s for its light to warm up: a
  strip at 1200 dpi takes about two minutes.
- `hp3770 tma-scan` (experimental) scans the adapter's area, with `--film negative` (or
  `slide`) also its positive; `hp3770 develop` makes the positive again from a saved 16-bit
  scan, without the scanner; `hp3770 lamp on|off --tma` switches the adapter's light.
- **Film frames without a preview**: straight to Scan, the frames are found in the scan
  itself and each becomes its own picture, as after a preview (one lamp warm-up less).
- **Split a scan already made**, without scanning again: a photo scanned whole (automatic
  crop off, or an area drawn by hand) gets **Split the photos** (on the glass) or **Split the
  frames** (films) next to *Edit…*; each original found becomes its own picture, and **Redo**
  on it scans just that one again.
- **Remove the scanner's stripes**, a tick in the scan settings (off by default): the HP 3770
  leaves faint light and dark lines across every scan of the glass, the same with the HP
  driver (about 3 % at 200 and 300 dpi, below 1 % at 600 and 1200); VetroScan finds them in
  the scan itself and takes them away afterwards, without touching the subject. Only on the
  glass, at the resolutions where they were measured.
- **Scan at twice the resolution and reduce**, a tick in the scan settings (off by default):
  for example 300 dpi made from a scan at 600, finer, with less noise and fainter stripes, in
  about twice the time. Only on the glass, where the scanner offers twice the resolution.
- **Measure the stripes**, in the Preferences: with a white sheet on the glass (or the glass
  empty), the scanner scans a strip at each resolution up to 1200 dpi and VetroScan finds
  every how many lines its stripes repeat and how strong they are. The result is kept for that
  scanner and used in place of what its model declares, so the stripes can be taken away on
  any scanner that has them; **The model's** goes back.
- `hp3770 stripes FILE.png` measures the stripes on a scan of something uniform, without the
  scanner.
- **First steps for the Epson Perfection 4490 Photo** (experimental, written before the
  scanner arrived: **it does not scan yet**). VetroScan finds it on USB (`04b8:0119`, next to
  Epson's driver on Windows) and reads its state; it sends nothing that could move the
  carriage. Its firmware (`esfw54.bin`, the copy of Epson's Windows driver) is inside the
  program; a build without it takes it from where Epson's drivers install it. For now it is
  sent only on request (`VETROSCAN_EPSON_UPLOAD=1`). The Linux packages carry a udev rule for it,
  `70-epson-4490.rules`.

### Changed
- The scanner contract: a scan says where it comes from, the glass or a film in the adapter,
  and a scanner its adapter's area and the films it takes.
- **Each mode keeps its own work.** Document, Photo and Film each keep their pages, their
  Ctrl+Z history and what was shown; going from one to the other throws nothing away and
  changes nothing, and coming back finds it as it was left. Before, going from Document to
  Photo kept only the last page (photos cut from one scan were lost going to Document and
  back), and photos took the adjustments of the mode they ended up in. The question "Switch
  to Photo?" is gone. Closing asks about the work not saved in every mode, and **Save…**
  saves each in turn. The glass's preview is shared by Document and Photo, so going to Film
  and back no longer loses it; frames scanned stay reachable with the adapter unplugged.
- In Photo and Film the column of the pages is there from the first photo, with **Edit**,
  **Redo**, **Duplicate** and **Delete**.
- **Split in steps**: *Split the photos* and *Split the frames* no longer cut at once. The
  viewer shows the originals found on the scan, numbered, to correct first: click one to
  choose it and move or resize it with the handles, drag on an empty spot to add one the app
  did not see, leave one out, turn it by 0.1° or 1°, or find them again. **Split into N**
  (Enter) cuts, **Cancel** (Esc) leaves the scan whole. With nothing found, an area can be
  drawn by hand.
- **The colours in one place, the Adjust tab.** **Edit…** now changes only the shape (crop,
  turns, straightening, size); the colour mode of a page or a photo already scanned is
  **Mode** at the top of Adjust, and its adjustments are those of Adjust. The sliders are in
  three groups: **Colour** (eyedropper, temperature, tint, vibrance, saturation), **Tone**
  (exposure, contrast, highlights, shadows, whites, blacks) and **Levels** (auto levels,
  black and white points, gamma, brightness).
- In Photo and Film a change in Adjust with a photo shown is for that photo only (before, it
  changed every photo and the preview): **Apply to all photos** and **Back to the common
  adjustments** are next to it.

### Fixed
- **Colour** in the Scan tab is always for the next scans, as the resolution next to it:
  with a page shown it used to change that page instead, and the status bar and the formats
  offered in Output did not agree with it. A page's own colour is changed in Adjust.

## [0.4.0] - 2026-11-08

### Added
- **Moving precisely from the keyboard**, the same keys wherever something moves with the
  mouse: the arrows move by 1 mm, **Shift** + arrows by 10 mm, **Alt** + arrows by 0.1 mm (or a
  pixel), **Ctrl** (⌘ on macOS) + arrows change the size, **Home** goes back to the middle. They move the
  area to scan on the preview, the crop in the editor, the page on its PDF sheet, the point of
  the curve clicked last (**Tab** the next one, **Delete** away) and a zoomed image. **[** and
  **]** straighten by 0.1° in the editor (**Shift**: 1°); **Page Up** / **Page Down** show the
  page before or after, with **Alt** they move it there.
- **The page anywhere on its PDF sheet**: dragged or with the arrows inside the margin, also at
  actual size and fitted to the sheet; under *Position on the sheet* the distance from the
  left and top edges in millimetres, also typed, **Centre** and **The same for every page**.
  Centred pages come out as before.
- **The area to scan typed in millimetres**: from the left, from the top, width and height.

### Fixed
- The documents said that the HP 3770 has no adapter for slides and negatives (also the
  notes of 0.3.0): it has one, in its lid. VetroScan does not use it yet.

## [0.3.1] - 2026-11-08

### Added
- **Open the folder** of the scans from the window: a button next to «Save …», one next to
  the folder's path in the Preferences, and the **Ctrl+O** shortcut (⌘O on macOS). The folder
  is made if it does not exist yet.

## [0.3.0] - 2026-11-08

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
- The RAW files (DNG and linear 16-bit TIFF) name in their metadata the scanner that made
  the scan and VetroScan as the program.
- The «Film» mode (disabled) only appears with a scanner that has the adapter for slides and
  negatives: the HP 3770 does not.
- The project's documentation is in English (README, guides, changelog); a short Italian
  README is in `README.it.md`. The guides have English names: `docs/install.md`,
  `docs/install-linux.md`, `docs/usage.md`, `docs/usb-protocol.md`.

### Fixed
- Texts cut off in the right panel: the action «Save the document and open the folder» of
  the scanner buttons in the Preferences, and the PDF orientation «Automatic» (it was
  «Automatic (like the scan)»).
- The percentages of blacks and whites clipped under the histogram have a decimal comma in
  Italian, like the other numbers.

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

[Unreleased]: https://github.com/MatteoDM/scanner/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/MatteoDM/scanner/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/MatteoDM/scanner/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/MatteoDM/scanner/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/MatteoDM/scanner/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/MatteoDM/scanner/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/MatteoDM/scanner/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/MatteoDM/scanner/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/MatteoDM/scanner/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/MatteoDM/scanner/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MatteoDM/scanner/releases/tag/v0.1.0
