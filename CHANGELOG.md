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

### Fixed
- Switching between **Negatives** and **B&W negatives** kept the preview, its measures and
  the base of the strip measured as the other film: a colour negative developed on a black and
  white strip's base came out with a strong blue cast, and the frames found could be the other
  strip's. Choosing another film now forgets them, as switching to slides already did, and
  unlocks a roll's base locked on the other film, saying so.
- **Lock the base of the last strip** could lock an older strip's base when the last one
  previewed showed no gap between its frames, or was previewed with **No processing**: there is
  now no base to lock then. A preview with **No processing** also forgets the measures of the
  strip previewed before, which a scan made after turning it off used to develop with.

## [0.11.3] - 2026-11-21

### Changed
- The README, the install guides and the usage guide describe both scanners, the HP Scanjet
  3770 and the Epson Perfection 4490 Photo, with films and Digital ICE; the Linux menu entry and
  the packages' descriptions name the 4490 and films too.
- The GitHub repository is now called VetroScan (github.com/MatteoDM/VetroScan): the download
  links, the Windows installer's links, the Linux packages' home page and the AUR package point
  there. The old `MatteoDM/scanner` links and clones keep working, as GitHub forwards them.

## [0.11.2] - 2026-11-21

### Fixed
- Epson Perfection 4490, *Leggi l'infrarosso*: a negative at 2400 dpi stopped before its
  infrared pass with "the infrared read would start 37 (or 38) steps after the colour one,
  never captured (39-45)". The scanner places its infrared pass from the holder's holes read
  twice, and the two reads differed by a few hundredths of a millimetre more than ever seen;
  the driver now lets out a start from 35 steps (a step is a hundredth of a millimetre).
- Epson Perfection 4490: after a Digital ICE scan stopped in its infrared pass, the next scan
  stopped at once with "1E A1 answered 04" (the infrared light still in use for a few tens of
  seconds). It now waits, up to a minute, for the scanner to go back to its lid's lamp.

## [0.11.1] - 2026-11-21

### Fixed
- Epson Perfection 4490, films: a film scan stopped right after reading the lid's origin (a
  move back never seen, such as 276 steps) left the carriages out, and the next scan refused to
  start from there ("the carriages at (1994, 2440)"). Now the scan waits, up to a minute, for
  the scanner to bring them home by itself, as it already did after the lid's lamp lit again.
- Epson Perfection 4490, films: the light's edge the scanner finds in its lid at the start of
  every film scan moves by a line or two from one scan to the next (two lines in nine minutes,
  measured), and the scan stopped when it moved a little further than seen before ("a move
  back of 276 steps, none of those let out"). The driver now follows it as Epson Scan does,
  over a wider margin (moves back of 180 to 312 steps; a read gone wrong is still refused).
  With *Leggi l'infrarosso* an area reaching the holder's bottom now loses its last millimetre
  or less (a quarter of a millimetre more), to stay within the reach Epson Scan was seen using.

## [0.11.0] - 2026-11-21

### Added
- **Negativi B/N** in *Pellicola*'s holder menu: black and white negatives, scanned as negatives
  (nothing new asked of the scanner) and made grey, the colour set to *Grigio* by itself and
  remembered; for every scanner with an adapter for negatives (the HP 3770 too). Tried with
  Digital ICE on a chromogenic film (Kodak BW400CN) on the Epson Perfection 4490.
- **Leggi l'infrarosso** in *Pellicola*, for scanners that read an infrared channel from a
  film (Digital ICE): a second reading in infrared light, which goes through the film but not
  through dust and scratches. The channel is kept with each frame and saved beside its 16-bit
  and RAW files (`…-infrarosso.png` / `.tif`, linear). Shown only where the scanner declares
  it, for that film and resolution: the Epson Perfection 4490 for negatives and slides at 600,
  1200 and 2400 dpi (a scan takes both passes, about twice as long), on any frame, the lowest
  too: an area reaching the holder's bottom loses its last half millimetre or so (its infrared
  light reads further on in the lid, and only as far as Epson Scan was seen going, which stops
  an area 0.3 mm above the bottom). The dust, hairs and specks it shows
  are taken out of the scan (`scan_imaging::dust`): found where the infrared is darker than its
  surroundings by four times its own noise, the picture's faint copy in the infrared (a film's
  cyan) taken out first, each colour's shadow found where it lies (on a slide the red's is more
  than a line away); light dust divided by what it took (the grain stays), the rest filled from
  around. Against Epson Scan's own ICE on nine scans: 90-98 % of what it changes on negatives,
  49-63 % on slides; none of its marks where the infrared is only blurred.
- `epson4490 film-scan … --ice`: the Epson Perfection 4490 scans a film with Digital ICE as Epson
  Scan does, colours then infrared, and lays the infrared on the colours itself; only what
  Epson Scan was seen doing with ICE (negatives and slides at 600, 1200 and 2400 dpi). When the
  lid's lamp, lit again, has left the carriages out, it waits for the scanner to bring them home
  (at most a minute) instead of stopping. `epson4490 lid-lamp --watch SECONDS` then reads where the
  carriages are, for that long (`--every SECONDS`: that often).
- **Firmware mancante**: a scanner that needs its firmware sent at every power-on (the Epson
  Perfection 4490, `esfw54.bin`) and finds no copy of it is shown as found, with an orange dot,
  not as *Scanner non trovato*: the **Scansione** tab names the file, where it was looked for,
  and has a button that opens the settings folder, where to copy it. Copied there, the file is
  taken without restarting the app (it was looked for only when the app started). The builds
  of the releases carry the 4490's firmware inside, so this shows only in a build without it.
  For drivers, a new error kind in the contract, `Error::FirmwareMissing`.
- Under **Leggi l'infrarosso**, what it is for: colour films and chromogenic black and white
  ones (Ilford XP2, Kodak BW400CN); on a classic, silver black and white film the dust removal
  may take parts of the picture for dust (general knowledge: not yet tried on such a film).
- The user's guide names the Epson Perfection 4490 among the scanners, and the install guide
  says what it needs on each system and where its firmware is looked for.

### Changed
- Films: the preview stays to go back to after a scan, as on the glass: the **Anteprima**
  button on the viewer shows it again with its frames, to choose others and scan them without a
  new preview; the **Anteprima** button at the top redoes it, only when pressed. Before, in
  *Pellicola* the viewer had no way back to the preview, and the only way to choose other frames
  was to scan the whole holder again.
- A scan is cleaned away from the window: the dust the infrared shows (about 2 s a frame at
  2400 dpi), the scanner's stripes (0.6 s for an A4 page at 600 dpi), its colour correction and
  the halving of a scan at twice the resolution no longer freeze VetroScan while they run. The
  progress card says *Pulizia della scansione* meanwhile; the frames come in their order as
  before, and *Annulla* stops the cleaning too.

### Fixed
- Epson Perfection 4490: a film's image read takes the run-up Epson's driver gives it for the
  carriage's speed (128 steps when it runs fast, as at 1200 dpi with a bright lamp), not one
  fixed by the resolution; a Digital ICE scan right after switching the scanner on is no longer
  stopped (its infrared read starts a step later then, as with Epson Scan). On the holder's
  right strip the infrared read starts at Epson Scan's column, not one beside it.
- Epson Perfection 4490, films at 600 and 1200 dpi: when the scanner's own correction of where
  its sensor starts is an odd number, the frame was read one column off from Epson Scan's.

## [0.10.0] - 2026-11-15

### Added
- **Screen colours** (Preferences): the pictures are shown as colour-managed programs show
  them, from each page's profile to the profile of the monitor the window is on, found by
  itself on Windows (again when the window moves to another monitor). On a wide-gamut
  monitor the colours are no longer too saturated, and the Epson 4490's glass scans show the
  midtones Windows Photos shows. Elsewhere, or without a profile, the pages are brought to
  sRGB; *No conversion* shows the numbers as before, and a profile file can be chosen by
  hand. Only the screen changes, not the files.
- Every file saved says what its colours are: an **ICC profile** made by VetroScan with sRGB's
  colours and the tone curve the scan was drawn with (gamma 1.8 for the Epson 4490's glass,
  as Epson Scan's files say; 2.2 for films and the HP 3770), grey for grey images, none for
  black and white. In PNG, JPEG, TIFF (multi-page and 16 bits too), WebP and the PDF's
  pictures; not AVIF nor BMP. The numbers do not change. Programs that manage colour (Windows
  Photos, Photoshop, browsers) now show the Epson 4490's glass scans with the midtones meant,
  as Epson Scan's TIFFs, where they read them darker before; the 16-bit PNG no longer claims
  to be sRGB.
- Film: a **preview** again, for negatives and slides, at the lowest resolution the adapter
  scans at (600 dpi on the HP 3770 and the Epson 4490). The frames found are outlined on it and
  **each one chosen is scanned on its own**, only it and a millimetre around it: at 2400 and
  4800 dpi a whole strip was too large to scan. The film is measured on the preview and every
  frame is developed with those measures. Scanning without a preview still scans the whole
  holder and finds the frames in it.
- Document, Photo and Film: **several areas on the preview**, each its own page or image with
  its own scan. The originals found by the automatic crop are areas like the others: chosen
  with a click, they are moved and resized with the handles, typed in millimetres, turned with
  **Tilt**; an area is added by dragging on an empty part and removed with **Delete**,
  **Remove area** or **Leave out**.
- Photo and Film: **several photos picked** in the column of thumbnails, with Ctrl+click
  (Cmd+click on macOS), Shift+click or Ctrl+A. **Save** then saves only those, with the same
  settings, and **Apply to the photos picked** gives them the look of the photo shown.
- **No processing** (Scan panel, every scanner and way of working): the file as the scanner
  gives it, like its own software with every correction off, only brought to a gamma to be
  seen. No stripes removed, no colour correction of the scanner, no double resolution, no
  development of films, no colour noise reduction, no sharpening, no adjustments (Regola has
  no effect on such a scan). Negatives come out only inverted, orange base and all, or with
  **Leave negatives as they are** not inverted, for programs that invert them themselves.
  Slides as the scanner reads them. It applies to the next scans; the frames are still found.

### Changed
- The log in the settings folder (`tasti.log`) also says how each scan ended, and why when it
  did not finish.
- A frame drawn on the preview no longer takes the place of the originals found: it is added
  to them. Turning the automatic crop off keeps only the areas drawn by hand. **Esc** lets the
  area chosen go instead of removing it (**Delete** removes it).
- **Redo** on a page scanned whole scans that page's own area again, not the one chosen on the
  preview now.

### Fixed
- The screen showed photos scanned at 16 bits (films above all) with darker shadows than the
  file saved: it was drawn from a small copy at 8 bits, linear, coarse in the dark tones (up to
  20 levels darker on the Epson 4490's frames). The small copy keeps its 16 bits now, and the
  screen is drawn as the file is; the preview too.
- Film: with two strips of negatives side by side (the Epson 4490's holder) the film was not
  measured, so the preview and a strip scanned whole stayed negative: the base was looked for
  between a frame of one strip and the next frame in reading order, on the other strip, across
  the holder. Now only between frames next to each other on the same strip, and a gap covered
  by the holder is left out.
- Film: negatives are developed with **the base of their strip and the white of each frame**, on
  the preview, on the frames scanned one by one and on a holder scanned whole. Two strips side
  by side may be of two rolls: measured together, the white of one set the colours of both (on
  the Epson 4490 the indoor strip came out orange). The base is the roll's, one per strip; the
  white and the black follow each picture, as a lab balances each frame. An area drawn by hand
  is measured on itself, on the base of its strip.
- Film at high resolutions: developing a frame took more memory than it needed (about 1.4 GB
  for a frame at 4800 dpi, now about 0.4 GB, as before), and VetroScan closed once at the third
  frame. The scans asked and any crash's message are now written to `tasti.log`.
- Film: two strips side by side joined by light across the holder (a lit slot, a hole, a leak at
  an end) were taken for one wide strip, and only some frames were found, between the strips.
  Such a piece is now split into its strips.

## [0.9.0] - 2026-11-15

### Added
- Epson Perfection 4490: films. **Film** now appears for the 4490: strips of negatives and
  slides in the transparency unit of its lid, at 600, 1200, 2400 and 4800 dpi, as Epson Scan
  scans them (every combination tried on the scanner against Epson Scan's own scans of the same
  frames). A scan covers the holder's window except its top 6 mm. No dust removal with the
  infrared yet.

### Fixed
- Film: a slide whose mount lets light past its edges (as in the Epson 4490's holder) was found
  as the whole mount, about 51 mm square; now the picture inside it is found, 36 x 24 mm.
- Epson Perfection 4490 on Windows: opening VetroScan while Epson Scan is scanning no longer
  stops Epson Scan's scan (it ended with "cannot communicate with the scanner"): VetroScan now
  takes the scanner from Windows before sending it anything, and says it is in use by another
  program. A scan of VetroScan's own left half-way (the app closed or killed while reading) is
  still stopped when the app opens again.
- Epson Perfection 4490: a scan within 14 minutes of the last one, after the scanner was
  switched off and on, gets the lamp's two and a half minutes of warm-up again (before, it
  went without once the scanner had stopped saying its lamp was warming, about 20 s after
  switching on). The scanner is known to have been switched on by the numbers it measures then
  (`1E 87`), the same at every scan until it is switched off.

### Changed
- The memory a scan needs (said in the Scan tab, and above half the computer's memory the scan
  is refused) counts what each scanner's driver holds at that resolution: for the Epson 4490
  about 1.8 times the image at 2400 dpi (read at 3200 and reduced) and the image alone at 4800
  dpi, where before every scanner above 1200 dpi counted twice, as the HP Scanjet 3770 at 2400
  dpi (unchanged). A doubled scan counts at the resolution sent to the scanner.

## [0.8.0] - 2026-11-14

### Added
- **VetroScan scans the glass of the Epson Perfection 4490 Photo**, with its own driver, on
  Windows, Linux and macOS: 100, 200, 300, 400, 600, 1200, 2400 and 4800 dpi, 8 or 16 bits,
  colour (grey and black and white made by VetroScan), any area of the glass (216 x 296.9 mm).
  Every command is computed from the area asked and kept within what the captures of Epson's
  own drivers showed the scanner doing; the image comes out as Epson's drivers' (checked
  against Epson Scan's at 2400 and 4800 dpi: the same place to the pixel). At 2400 dpi a
  16-bit scan can be at most 115 mm wide, at 4800 dpi 58 mm at 16 bits and 116 mm at 8 (a
  wider line was never seen from the scanner).
  - Its colours as Epson Scan corrects them (**Correct the scanner's colours**, on by
    default; see Changed).
  - Its lamp: given two and a half minutes to warm up after the scanner has been idle for a
    quarter of an hour or just switched on, Cancel heard meanwhile; switched off as for the
    HP 3770 (the Preferences' lamp choices, and on closing). After Epson Scan, or with the
    lid's lamp in use, VetroScan switches to the glass's lamp as Epson's driver does.
  - Its four buttons (Start, Scan to PDF, Scan to E-mail, Copy). On Windows with Epson's
    driver VetroScan takes the scanner for itself while it has it open, so Windows stops
    asking it for its buttons in between: Epson Scan cannot use it meanwhile, and if Epson
    Scan is scanning, VetroScan says the scanner is busy.
  - Its firmware sent when VetroScan opens it and it is not running (Linux and macOS, where no
    Epson driver sends it at switch-on; also after it was switched off with its carriage away
    from home, which brings the carriage home).
  - A scan left half-way (VetroScan closed or stopped while reading) is stopped when the
    scanner is opened again, the carriage brought home; a scan whose image data fails
    half-way is stopped as a cancelled one is.
  - When the scanner flags an error of its own (its front light flashing red, which comes
    now and then while its lamp warms up, with Epson Scan too), the next preview or scan
    clears it as Epson Scan does and goes on; only if it comes back at once does VetroScan ask
    for the scanner to be switched off and on.
  - Its originals found on the preview (its lid darkens towards the edges of the glass).
- **Correct the scanner's colours** on the glass, for a scanner that declares a correction
  for it, as it already was for films.

### Changed
- A scan that would need more than half of the computer's memory is refused, with how much it
  would need and how much the computer has, instead of starting and perhaps closing VetroScan
  half-way. VetroScan holds up to about two and a half times the memory it shows for a scan
  while putting its image together (measured: 1.04 GB for a scan shown as 452 MB).
- **Correct the scanner's colours** is on by default in Document and Photo too: settings saved
  by earlier versions take the new default once.
- `VETROSCAN_EPSON_UPLOAD` is no longer needed, and no longer read: VetroScan sends the Epson
  Perfection 4490 its firmware by itself when it is not running (a capture of Epson's driver
  showed the same bytes).

### Fixed
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
- The originals found on the glass, and the slides and frames found in the film holder, come
  in the same reading order every time: row by row from the top, left to right in each row.
- With nothing like the lid along the edges of the glass (the lid left open), the search for
  originals finds none, so that the areas can be drawn by hand, instead of taking the whole
  glass for one original. This holds when the room seen through the open lid is dark and even
  too: the lid is now taken to be light against the whole scale, not only against the
  brightest part of the preview (tried with the Epson Perfection 4490's lid open on an empty
  glass).
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

[Unreleased]: https://github.com/MatteoDM/VetroScan/compare/v0.11.3...HEAD
[0.11.3]: https://github.com/MatteoDM/VetroScan/compare/v0.11.2...v0.11.3
[0.11.2]: https://github.com/MatteoDM/VetroScan/compare/v0.11.1...v0.11.2
[0.11.1]: https://github.com/MatteoDM/VetroScan/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/MatteoDM/VetroScan/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/MatteoDM/VetroScan/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/MatteoDM/VetroScan/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/MatteoDM/VetroScan/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/MatteoDM/VetroScan/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/MatteoDM/VetroScan/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/MatteoDM/VetroScan/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/MatteoDM/VetroScan/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/MatteoDM/VetroScan/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/MatteoDM/VetroScan/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/MatteoDM/VetroScan/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/MatteoDM/VetroScan/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/MatteoDM/VetroScan/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MatteoDM/VetroScan/releases/tag/v0.1.0
