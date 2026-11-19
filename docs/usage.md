# Using VetroScan

This guide explains how to use **VetroScan** (`vetroscan`) with the HP Scanjet 3770 to acquire
documents and photos: what each button does, which format to choose, how to correct a scan
already made and what to do when something goes wrong. It applies to Linux, Windows and
macOS; where a system behaves differently, it says so.

To install the app and the scanner, see [install.md](install.md) first (or, for a new
computer with Omarchy, [install-linux.md](install-linux.md)).

> On macOS the app installs and opens, but it has not yet been tried with a real scanner.

The app speaks Italian and English: the names of buttons and tabs in this guide are the
English ones. It follows the system's language; to change it, see
[The Preferences](#the-preferences).

- [The first scan in a minute](#the-first-scan-in-a-minute)
- [The window](#the-window)
- [Document and Photo](#document-and-photo)
- [Preview, area and automatic crop](#preview-area-and-automatic-crop)
- [Resolution, colour and depth](#resolution-colour-and-depth)
- [The Adjust tab](#the-adjust-tab)
- [The pages: Redo, Move, Delete](#the-pages-redo-move-delete)
- [Editing a scan already made](#editing-a-scan-already-made)
- [Saving: the formats](#saving-the-formats)
- [The scanner buttons and the photocopy](#the-scanner-buttons-and-the-photocopy)
- [The Preferences](#the-preferences)
- [Keyboard and mouse shortcuts](#keyboard-and-mouse-shortcuts)
- [Common problems](#common-problems)

---

## The first scan in a minute

1. Switch the scanner on and open the app. At the top right a green dot says
   **Ready · lamp off** (or **on**): the scanner is connected.
2. At the top choose **Document** (sheets, also several pages in the same file) or **Photo**.
3. Put the original on the glass **face down**, against the corner shown by the arrow on the
   scanner, and close the lid.
4. Press **Preview**: after a few seconds you see the whole glass. The first time the lamp
   has to warm up and it takes a little longer.
5. If you only want a part, **drag on the preview** a frame around what you need (drag again
   on an empty part for another one: each becomes its own page or image). In Photo the app
   finds by itself the photos lying on the glass, and you can correct them by hand.
6. Press the blue button at the top (**Scan page 1**, **Scan photo**…).
7. At the bottom right press **Save …** (for instance **Save PDF…**), choose the name and the
   folder, and confirm.

The rest of this guide is about doing these same things better.

---

## The window

| Where | What is there |
|---|---|
| **At the top** | the app's name and version, the **Document** and **Photo** modes, the scanner's state, the **Preview** and **Scan…** buttons, the choice of theme (**System**, **Light**, **Dark**) |
| **On the left** | the column of the **pages** (always in Document, in Photo and Film from the first photo) |
| **In the middle** | the **viewer**: the preview of the glass or the scan made |
| **On the right** | four tabs: **Scan**, **Adjust**, **Output**, **Preferences**; below them, the **Save …** and **Open the folder** buttons |
| **At the bottom** | the status bar: the scanner in use and its connection, resolution and colour, size of the image in pixels, the app's last message |

The **scanner's state**, at the top right:

| Dot | Text | Means |
|---|---|---|
| grey | Connecting… | the app is looking for the scanner |
| green | Ready · lamp off / on | you can scan (a scanner without a lamp just says **Ready**) |
| orange | Working | a preview or a scan is under way |
| red | Scanner not found | see [Common problems](#scanner-not-found) |

A click on the state reads it again from the scanner.

On the viewer there are two buttons, **Preview** and **Page** (or **Photo**), to go from the
glass to the scan made. Next to them there are **Edit…** (opens the
[editor](#editing-a-scan-already-made)), **Eyedropper** and **Clear selection**; at the top
right **Before** / **After**; at the bottom right the zoom, with **Fit** (the whole image in
the window) and **1:1** (one pixel of the image per pixel of the screen).

While the scanner works a box appears with the percentage, the phase (start-up, lamp
warm-up, calibration, reading, end of the scan) and the **Cancel** button. During a scan you
cannot change mode or the order of the pages, and you cannot save.

When a page of the document is ready, a photo has been saved by itself or a scan fails, the
system also shows a **notification** with its sound: useful if you are standing at the
scanner.

The app only shows what the scanner in use can do: a scanner without a lamp has no lamp
settings, one without buttons has no button settings, one that does not give 16 bits has no
choice of depth, and the resolutions and the glass are the scanner's own.

---

## Document and Photo

The app has two ways of working, chosen at the top. Each remembers its own settings
(resolution, colour, format, adjustments, scanner buttons) and keeps its own work: the pages
of the document, the photos of the last scan. Going from one to the other throws nothing away
and changes nothing: you find again what you had left, as you had left it (the preview of the
glass too, which the two share). Closing and opening the app again brings back resolution,
colour, format and buttons; the **adjustments** instead start from zero, so that a new scan
does not inherit by mistake the colours of the session before. A look to reuse is saved as a
[preset](#the-adjust-tab).

| | **Document** | **Photo** |
|---|---|---|
| For | letters, contracts, receipts, magazines, books | printed photos, postcards, drawings |
| Each scan | **adds a page** to the document | **replaces** the photos of the scan before |
| Saved as | a single PDF or TIFF, or one image per page | one image per photo, also RAW |
| By default | 300 dpi, colour, 8 bits, PDF, automatic crop off | 600 dpi, colour, 16 bits, JPEG, automatic crop on |

With a scanner that has an adapter for films a third button appears, **Film**. The HP 3770
has one in its lid: the light panel, with the holder for two slides or a strip of negatives.
VetroScan scans both **negatives** and **slides** with it:

1. put the strip or the slides in their holder and the holder on the glass, as the HP manual
   shows, and plug the adapter's cable in at the back of the scanner;
2. choose **Film** (with the cable unplugged a message says so) and, in the **Scan** tab,
   what is **In the holder**: **Negatives** or **Slides** (the choice shows only for an
   adapter that takes both; VetroScan remembers it);
3. **Preview**: the whole holder at the lowest resolution the adapter scans at (600 dpi on
   the HP 3770 and on the Epson 4490). The negatives are shown as positives with their natural
   colours (slides are positives already: they are only stretched to the full range, one black
   and one white for the three colours, so that their colours stay the film's), and the film
   is measured there once for all the scans: the orange base of each strip (the Epson 4490's
   holder takes two, maybe of two rolls), and the white and black of each frame, which is
   balanced on itself as a lab does. With the **automatic crop** on (the
   default) each frame or slide found is outlined and numbered: leave out the ones you do not
   want, correct one with its handles, or drag on an empty part to add one the app missed, as
   in [Choosing the areas](#choosing-the-areas);
4. **Scan 2 photos** (or as many as there are): **one scan for each photo**, at the resolution
   chosen, of that photo and a millimetre around it only. This is what makes 2400 and 4800 dpi
   usable: the whole strip at 4800 dpi would be gigabytes, the samples of a 35 mm frame are about
   185 MB.
   **Redo** on a photo scans that photo only.

**Scan photo** without a preview still works: the scan covers the whole holder and, with the
automatic crop, finds the frames in it, one image each (at the resolutions where the whole
holder fits in memory).

The **Epson Perfection 4490** has its adapter in the lid too, with nothing to plug in: put the
strip or the slides in the 4490's holder on the glass as Epson's manual shows (the document
mat taken off the lid), choose **Film** and what is in the holder, and scan as above. The scan
covers the holder's window but its top 6 mm, where only the holder's edge is. Each film scan
lights the lid's lamp (about 20 seconds when it was off) and calibrates on its own; a frame
took a minute at 1200 dpi and two at 4800 on the scanner, the whole strip takes several. The
4490 also reads an infrared channel (Digital ICE): tick **Read the infrared** in **Scan** for a
negative at 1200 or 2400 dpi or a slide at 2400 (the tick shows only there), on a frame chosen
in the preview, the lowest too: an area reaching the bottom of the holder loses its last half
millimetre or so (the infrared light reads further on in the lid, and only as far as Epson Scan
was seen going). The scan then
takes two whole passes, colours then infrared, about twice as long (two minutes for a frame at
1200 dpi); the infrared, laid on the colours by the driver, is kept with each frame and saved
beside it (`…-infrarosso`). Dust and hairs show dark in it, and VetroScan takes them out of the
scan: light dust is divided by what it took (the grain under it stays), a speck or a hair that
lets little through is filled from around it, in each colour where its shadow lies. What is not
dust (the picture's edges, a hair out of focus) is left alone; a mark wider than a few tenths of
a millimetre stays. To see what was found, open the `…-infrarosso` file beside the scan. After a
film, the next scan of the glass switches back to the glass's lamp and warms it
up first.

Slides are scanned as negatives are on the HP 3770, with the longer exposure of its negatives'
modes: measured on real slides, it halves the noise in their dark parts (where a slide is far
denser than a negative), for about 20 seconds more; nothing came out burnt in the light parts.

With the automatic crop off you get the whole strip as one image; **Split the frames**, on the
viewer, shows the frames found on it, to correct, then cuts it into one image per frame,
without scanning again (see [Several photos from a single scan](#several-photos-from-a-single-scan)). To scan only a
part of the adapter's area, type it in millimetres in the **Scan** tab.

Film keeps its own settings: by default 1200 dpi, colour, 16 bits, JPEG; the resolution is
600 or 1200 dpi only on the HP 3770 (lower ones would make the motor stall), 600, 1200, 2400
or 4800 dpi on the Epson 4490. On the HP 3770 every scan with the adapter first switches its light on and waits about 30 seconds for it to warm up, then
calibrates on the empty slot at the top of the holder: a strip at 1200 dpi takes about two
minutes. 1200 dpi gives a 35 mm frame about 2 megapixels, fine for the screen and 10 × 15
prints; the scanner has no infrared, so no automatic dust removal: blow on the film first.

Two ticks in the **Scan** tab clean a film's picture, and a third, for slides, corrects
their colours:

- **Reduce the colour noise** (on by default): the light and blue parts of a negative come
  through the film's orange base with little blue light, so its positive gets coloured specks
  there (measured on a light blue T-shirt: about three times more noise in blue than in red).
  VetroScan measures the noise on each picture and smooths only its colour, as much as it
  needs, leaving the brightness, where the detail is, as it was; a picture without coloured
  noise is left alone.
- **Remove the scanner's stripes** (off by default): the faint horizontal lines the HP 3770
  leaves on every scan, on negatives and slides alike, taken away from the film as scanned,
  before it becomes its picture.
- **Correct the scanner's colours** (on by default, shown only for a film the scanner declares
  a correction for: on the HP 3770, slides): the scanner sees a slide's colours through its
  own lamp and sensor, and on the HP 3770 a blue sky comes out lavender. The correction is
  the one HP's own driver uses for slides; untick it to get the colours as the sensor reads
  them. A profile measured on a calibration slide (IT8) would be more exact, and may replace
  it later.

Below them, for negatives only, **Roll**: the film's **base**, the orange of the bare film,
which sets how each colour is turned into the positive. By default each strip measures its own, in the gaps
between its frames. The frames of one roll all have the same base, so once a strip with at
least two frames has been scanned you can press **Lock the base of the last strip**: the base
is shown (red, green and blue, as the share of the lamp's light the bare film lets through)
and every following strip uses it, measuring by itself only its highlights and blacks, which
follow the scenes. It matters most for a strip with a single frame, or with no gap in view:
without a locked base its base is guessed from the darkest shadows of the picture, and on
real negatives that gave casts of up to 12-16 levels of 255 in one colour; with the roll's
base, one or two. VetroScan remembers the base locked, even after closing; for another film
press **New roll** and each strip measures its own again.

**Take care:**

- In **Photo** and **Film** a new scan takes the place of the photos you see (the app asks
  first if they are not saved). Save first, or tick **Save each photo to the folder by
  itself** in the **Output** tab.
- Film keeps its own work too: the frames scanned stay there to see and save even with the
  adapter's cable unplugged, or with a scanner without the adapter.

---

## Preview, area and automatic crop

### The preview

**Preview** acquires the whole glass (in Film, the whole holder) at low resolution, in a few
seconds. It is for seeing where the originals are, choosing the areas and trying the
adjustments before the real scan. It is not compulsory: without a preview an A4 sheet is
acquired (in Film, the whole holder).

### Choosing the areas

The same in Document, Photo and Film: on the preview there can be **several areas**, and
**each becomes its own page or image**, with a scan of its own.

- **Drag** on an empty part of the preview with the left button to draw an area; drag again
  elsewhere for another one. Areas are numbered in the order they will come out.
- **Click** an area to choose it: its **eight handles** (corners and sides) appear; take one
  to resize it, or drag it from inside to move it. A click on an empty part lets it go
  (**Esc** too).
- **Remove area** (or **Delete**) removes the area chosen; **Leave out**, next to an area in
  the list of the **Scan** tab, removes that one.
- With the **keyboard**: the arrows move the area chosen by 1 mm (**Shift** 10 mm, **Alt** a
  pixel of the preview, 0.13 mm); **Ctrl** + arrows change its size, from the right and bottom
  edges; **Home** puts it in the middle of the glass. It stops at the edges of the glass.
- **Tilt** (−1°, −0.1°, +0.1°, +1°, **Upright**), in the **Scan** tab, turns the area chosen:
  the image is cut out at that angle and straightened.

The **Scan** tab lists the areas with their size, and for the area chosen says where it starts
and how large it is, in millimetres (**From the left**, **From the top**, **Width**,
**Height**: you can also type them, with a comma or a point). An area drawn by hand and left
upright is acquired exactly as drawn. **With no area** the **A4** format is acquired
(210 × 297 mm from the corner of the glass), and the tab says how many pixels and how much
memory it will take; typing a number makes an area from it.

### Automatic crop

With **Automatic crop** on (**Scan** tab, *Area* section), after the preview the app looks
for the originals lying on the glass (in Film, the frames and the slides in the holder):
photos, postcards, cards, receipts, even several together and a little askew. Each one appears
as an area on the preview and in the list, with its size and how askew it is. When scanning
**each one becomes an image of its own, cropped a little inside its edge and straightened**
(in Document, a page of its own).

- The originals found are areas like the others: correct them with the handles and the tilt,
  leave out those you do not want, add by hand one the app missed.
- **Find again** repeats the search on the preview, in place of the originals found and
  corrected (the areas drawn by hand stay).
- Turning the automatic crop off hides the originals found and keeps only the areas drawn by
  hand; turning it on shows them again.
- A white sheet on the white lid can escape: in that case draw the area by hand.
- On the glass, several originals found are acquired in a single pass, or one at a time if all
  together would take too much memory or if an area drawn by hand is among them; films always
  one at a time. The button at the top says how many there will be (**Scan 3 photos**).

The automatic crop is on by default in Photo and off in Document.

### No processing

**No processing**, in the **Scan** tab, gives the file as the scanner gives it: the equivalent
of the scanner's own software with every correction off, for those who process their scans
in another program. The scanner's calibration stays (it is how the scanner reads at all);
the only conversion is to a gamma, so that the file opens and looks right anywhere. Nothing
else is done: no stripes removed, no colour correction of the scanner, no double resolution,
no colour noise reduction, no sharpening, no adjustments (**Adjust** says it has no effect
on such a scan), and films are not developed:

- **negatives** come out only **inverted**, with their orange base in: dark and with a colour
  cast, as the scanner's software gives them with its corrections off;
- with **Leave negatives as they are**, not even inverted: the negative with its orange base,
  for programs that make the positive themselves;
- **slides** as the scanner reads them, not stretched.

The areas and the frames are found on the preview as usual. It applies to the next scans and
is remembered for each way of working; a scan made so stays so. For the linear samples,
without even the gamma, there is the **RAW** format.

### Warnings about the size

If the scan takes a lot of memory (more than about 400 MB) the **Scan** tab says so in red,
with the GB and the minutes to expect. At **2400 dpi** without a selection it warns that the
resolution is for small originals: select the area.

A scan that would need more than half of the computer's memory is **refused**: the **Scan**
tab says so, with how much it would need and how much the computer has, and **Scan** sends
nothing to the scanner. Select a smaller area or lower the resolution. While putting a scan's
image together VetroScan holds up to about two and a half times the memory shown under the
area, so on a computer with 8 GB a scan shown at about 1.5 GB is the largest allowed.

---

## Resolution, colour and depth

All in the **Scan** tab, *Original* section.

### Resolution

The choice is the scanner's: for the HP 3770 **200, 300, 600, 1200 and 2400 dpi** (dots per
inch). More dpi = more detail, but larger files and slower scans. A resolution chosen on a
scanner and not offered by another stays saved; meanwhile the app uses the nearest one the
scanner in use has.

| Original | Suggested dpi |
|---|---|
| Documents to read, file, send | 200–300 |
| Text to give to a recognition (OCR) program | 300 |
| Photos to look at on screen or print at the same size | 300–600 |
| Photos to enlarge, quality archive | 1200 |
| Stamps, small details (only with an area selected) | 2400 |

### Colour

- **Colour**: for everything that has colours.
- **Grey**: black and white photos, documents with pencil or faded stamps.
- **Black and white**: only pure black and white, very small files; ideal for printed text.
  How dark a point must be to become black is decided by the *threshold* (see
  [Adjust](#the-adjust-tab)).

The scanner always reads in colour: the colour chosen is applied afterwards. That is why,
even on a scan already made, **Mode** at the top of the [Adjust tab](#the-adjust-tab) can go
from grey to colour and back.

**Colour** in the **Scan** tab, like the resolution next to it, is always for the next scans,
even while the viewer shows a page: the pages keep theirs. The colour of a page already
scanned is changed in the **Adjust** tab, with **Mode** at the top while the page is shown.

**Correct the scanner's colours** (in the **Scan** tab, on by default, shown only for a
scanner that declares a correction for the glass: the Epson Perfection 4490, with the one
Epson Scan applies, which makes blues and greens livelier): untick it to get the colours as
the sensor reads them. It applies from the next scan.

### Depth: 8 or 16 bits

Only for scanners that give 16 bits, like the HP 3770.

- **8 bits per channel**: smaller files. Fine for documents.
- **16 bits per channel**: the scanner sends 65,536 shades per colour instead of 256. Shadows
  and gradients stay smooth even after strong curves and corrections. It takes more memory
  (about three times) and it is the default in Photo.

The adjustments work on the 16-bit samples even when you then save in a normal format (JPEG,
PNG): the 16 bits help already like that. To keep them **in the file too**, choose
**PNG 16 bit** or **TIFF 16 bit** in **Output**, which only appear with the 16-bit depth (and
not in Black and white). The RAW formats are always acquired at 16 bits, whatever depth is
chosen, when the scanner gives them.

---

## The Adjust tab

The adjustments **do not touch the scanner's data**: the app keeps the scan aside as it
arrived and applies the adjustments on top, live on the preview or the page, and at full
resolution only when you save. You can change your mind as often as you like without losing
quality.

### What they apply to

At the top of the tab it says what you are adjusting:

- **NEXT SCAN** (with the **Preview** view): you adjust the preview and the settings the next
  scans will come with. In Document they also apply to the pages without «This page only», in
  Photo and Film to the photos following the common adjustments.
- **PAGE 2**, **PHOTO**… (with the **Page** or **Photo** view): you adjust the scan shown.
  - In **Document** each page has its own adjustments from the moment it is scanned: the box
    **This page only** is ticked, and changing a page does not change the others. Below it,
    **Apply to all pages** gives the look of the page shown to every page, to the preview and
    to the next scans. Unticking the box makes the page follow the common adjustments again
    (those of the preview), together with the other pages without the box.
  - In **Photo** and **Film** each photo follows the common adjustments (those of the
    preview) until you change one: from then on the changes apply to that photo only, and the
    other photos, the preview and the next scans stay as they are. **Apply to all photos**
    makes its look the common one, which every photo follows again; **Back to the common
    adjustments** drops its own. With some photos [picked](#picking-several-photos),
    **Apply to the photos picked (n)** gives the look of the photo shown to those only (each
    then has it as its own).
- **Mode**, below (only with a page or a photo shown): Colour, Grey or Black and white for
  that scan, even different from how it was acquired. The next scans keep the colour chosen
  in the **Scan** tab.

### The controls

**Presets.** At the top, **Apply a preset…** offers the ready-made settings of the mode:

| Document | Photo |
|---|---|
| Text for OCR (black and white) | Standard photo |
| Colour document | Photo archive (1200 dpi, TIFF) |
| Receipt | Faded photo (colour recovery) |
| Magazine / newspaper | Black and white photo |
| Archive (600 dpi, lossless) | Photo printed in a magazine |

A preset changes resolution, colour, depth, format and adjustments (not the automatic crop
nor the automatic saving). To save your own, type a name below and press **Save** (or Enter):
**up to three** are kept, seen in both modes. With the same name as an existing one you
replace it; **Delete** next to the name removes it. The names of the factory presets cannot
be used. A preset applied is undone with **Ctrl+Z**.

> Applied while looking at a page, the preset only changes colour and adjustments:
> resolution, depth and format stay as before. To apply it in full, go to the **Preview**
> view first.

**Histogram.** It shows how many points of the image are dark (on the left) or light (on the
right), and the percentage of **blacks clipped** and **whites clipped**: if it is high,
details are lost in the shadows or in the highlights.

**Colour** (only in Colour):
- **Neutral eyedropper**: then click on the image a point that should be grey or white (a
  wall, a sheet, an overcast sky): the app removes the colour cast starting from there.
  **Esc** cancels it.
- **Temperature** (cool ↔ warm), **Tint** (green ↔ magenta), **Vibrance** (livens up dull
  colours and spares those already bright) and **Saturation** (all colours the same way).

**Tone** (not in Black and white): **Exposure** (in stops), **Contrast**, **Highlights**,
**Shadows**, **Whites**, **Blacks**.

**Levels** (not in Black and white):
- **Auto levels**: chooses by itself the black and white points.
- **Black point**, **White point**, **Gamma** (midtones), **Brightness**.

**Curve** (not in Black and white). The graph goes from the shadows (bottom left) to the
highlights (top right), with the histogram behind. **Click** adds a point (up to 16), **drag**
to move it, **double-click** or drag it out to remove it. In Colour you can work on all
channels (**All**) or on **R**, **G**, **B** separately. Three ready-made curves: **Linear**,
**Medium contrast**, **Strong contrast**.
The point clicked last stays chosen (it is drawn full, with its input and output): the
**arrows** move it by one level (**Shift** ten), **Tab** chooses the next one (**Shift+Tab**
the one before), **Delete** removes it (not the two ends), **Esc** lets it go.

**Detail**:
- **Sharpness**: 1.5 is what the HP driver does; 0 switches it off.
- **Noise reduction**.
- **Descreening**: removes the print screen of newspapers and magazines (the moiré effect,
  the wavy lines). Choose the kind of printed paper: *Newspaper (85 lpi)*, *Magazine
  (133 lpi)*, *Glossy magazine (150 lpi)*, *Art print (175 lpi)*.

**Black and white** (only with the Black and white colour):
- **Automatic threshold**: the app chooses it by looking at the image.
- **Threshold**: higher = more black.
- **Adaptive threshold**: for unevenly lit pages (a book that does not lie flat on the glass,
  a sheet yellowed at the edges).

Each group opens and closes with a click on its title, and has its own **Restore**. A group
that is not needed with the colour chosen stays closed and says why.

### Sliders and numbers

- **Double-click** on a slider brings it back to its default.
- The number next to each slider can be typed by hand: confirm it with **Enter** (both comma
  and point are fine).

### Before / After

**Before** (on the viewer, top right, or the **\\** key) shows the image without adjustments;
**After** with them again. In the editor, **Before** shows the scan as it was when you opened
it.

### Undoing: Ctrl+Z

**Ctrl+Z** (on macOS **⌘Z**) undoes the last change to adjustments, curve, colour and preset;
**Ctrl+Y** or **Ctrl+Shift+Z** redoes it. Dragging a slider, even for long, counts as one step
only. The last 100 steps are kept.

- A change made with **This page only**, or to a photo, is undone on that page, even if
  meanwhile you look at another one. A photo's first change undone, it follows the common
  adjustments again.
- Each mode has its own history: going back to a mode, Ctrl+Z goes on from where it was.
- Ctrl+Z does **not** bring back resolution, depth and format chosen by hand, nor pages
  deleted, moved or replaced.

---

## The pages: Redo, Move, Delete

The column on the left shows the pages (or the photos) in order, with their number. Next to
the number, **own** means the page has adjustments of its own, **✎** that it was turned,
cropped or resized.

- **Click** on the thumbnail: shows that page in the viewer.
- **Double-click**: opens it in the [editor](#editing-a-scan-already-made).
- **Drag** the thumbnail onto another: the page takes that place. A coloured bar shows where
  it will go; let go outside the column and it stays where it was.
- **↑** / **↓**: moves it one place (also without a mouse).
- **↻**: turns it 90° clockwise.
- **Edit**: opens the editor.
- **Duplicate** (or **Ctrl+D** on the page shown): adds a copy of the page right after it.
  The copy uses the same scan, but is cropped, turned and adjusted on its own.
- **Delete**: removes it. If the page is not saved, the app asks first; it cannot be undone.
- **Redo**: acquires it again (see below).

#### Picking several photos

In **Photo** and **Film** several photos can be picked, to save or adjust only those:

- **Ctrl+click** (**Cmd+click** on macOS) on a thumbnail picks it, or lets it go if it was
  picked; the first time the photo shown is picked too, so Ctrl+click on a second photo makes
  two.
- **Shift+click** picks every photo from the one shown to the one clicked.
- **Ctrl+A** picks them all; **Esc**, **None** above the column or a plain click on a
  thumbnail lets them go.

The photos picked have a tinted frame and a **✓** by their number, and the top of the column
says how many there are. While some are picked, **Save the n picked…** saves only those, with
the same settings as always (one file each, a single file, ZIP, 16 bits or RAW as chosen);
the others stay to save. In **Adjust**, **Apply to the photos picked (n)** gives them the look
of the photo shown.

Moving, duplicating, deleting and redoing are not possible during a scan.

### Several photos from a single scan

To keep separate several photos acquired together (or several crops of the same photo):
**Duplicate** the scan once for each photo, open each copy with **Edit** and crop it on its
photo, then save in an image format: with several pages the app writes **one file per page**
(`name-01.jpg`, `name-02.jpg`, …). In **Photo** mode, with the automatic crop, the app
already does it by itself for the originals it finds on the glass.

A photo scanned whole (automatic crop off, or an area drawn by hand) can be split afterwards,
without scanning again: **Split the photos** (**Split the frames** for a film), on the viewer.
Nothing is cut at once: the viewer shows the scan with the originals found on it, numbered,
and the panel lists them, to correct first:

- **click** an area to choose it, then drag it, or pull its sides by the handles;
- **drag** on an empty spot to add an area the app did not see;
- **Leave out** (or **Delete** on the area chosen) leaves one out;
- **Tilt** turns the area chosen by 0.1° or 1° at a time, **Upright** puts it straight
  (the handles move its sides as if it were upright; its outline shows the cut);
- **Find again** goes back to the areas found by the app.

**Split into N** (or **Enter**) cuts them out, each a picture of its own in the place of the
scan; **Cancel** (or **Esc**) leaves the scan whole. Changing mode, scanning, or showing or
moving another page cancels too.

### Redo and "Keep the new one"

**Redo** is for when a page came out askew, blurred or with the lid open: put the original
back on the glass and press **Redo** under its thumbnail.

- A page cut out by the **automatic crop** is acquired again exactly in the same spot of the
  glass (with a millimetre of margin).
- The other pages are acquired again with the **area and settings of now** (the selection on
  the preview, or the A4).

When the scan is over, the viewer shows the new version with the words **New scan of page N**
and two buttons:

- **Keep the new one**: the old page is replaced, in the same place in the document.
- **Keep the old one**: the new scan is thrown away.

While the choice is pending you can zoom in on the new scan to check it. Choose before doing
anything else: clicking another thumbnail leaves the new scan without warning.

The page redone starts from zero: the rotations, crops and adjustments of the old page do not
pass to the new one.

### New document

In the **Output** tab, **New document** removes all the pages to start another one. If there
are pages not saved, it asks first.

---

## Editing a scan already made

The **editor** corrects the shape of a page or a photo after the scan: rotation,
straightening, crop, size (its colours are changed in the [Adjust tab](#the-adjust-tab)). Like the adjustments, **it spoils nothing**: it always starts from the original
scan, until you save.

**Opening it:** **Edit** under the thumbnail, double-click on the thumbnail, **Edit…** on the
viewer, or the **E** key while looking at a page.

**Closing it:**

- **Apply** (or **Enter**): keeps the changes.
- **Discard** (or **Esc**): puts the page back as it was when you opened the editor.
- Changing mode, going back to the preview, making a new scan or moving, deleting or redoing a
  page, the editor closes **keeping** the changes.

Above the viewer there is the bar **Editing page N** with **Undo**, **Redo**, **Discard** and
**Apply**. The editor has its own history: **Ctrl+Z** and **Ctrl+Y** undo and redo only the
steps made inside it. **Back to the original**, at the top right of the panel, puts back the
crop, the turns and the size the page was scanned with (this too is undone with Ctrl+Z).

The panel has two parts: **Crop** and **Size**.

### Crop

- **Rotate**: **90° left**, **90° right**, **180°**; **Mirror** (right ↔ left) and **Flip
  vertically** (top ↔ bottom).
- **Straighten**: the **Angle** slider goes from −45° to +45°, in steps of 0.1°; the buttons
  **−1°**, **−0.1°**, **+0.1°**, **+1°** move it a little, as do the keys **[** and **]**
  (0.1°; with **Shift** 1°); double-click on the slider resets it. A grid appears on the viewer: line up an edge or a line of text with it. Until you
  choose a crop, the empty corners left by the rotation are cut off by themselves.
- **Crop**: drag on the image for a new frame, move the edges with the handles, or the whole
  frame by dragging it from inside. **Proportions** keeps it in a fixed shape: *Free*, *Like
  the original*, *Square 1:1*, *4:3*, *3:2*, *16:9*, *A4 / A3 / A5*, *Photo 10 × 15*; for
  those that are not square a button switches from **portrait** to **landscape**. Below you
  read the pixels and millimetres of the crop. **The whole image** removes the crop. With the
  keyboard the arrows move the crop by 1 mm (**Shift** 10 mm, **Alt** 0.1 mm or a pixel),
  **Ctrl** + arrows change its size keeping the proportions chosen, **Home** centres it; a
  series of presses is one step of **Ctrl+Z**.
- **Reset rotation and crop** undoes all of this, leaving the size.

With the keyboard: **R** turns 90° right, **Shift+R** left.

### Size

To shrink a photo to send by email or fit it to a print size.

- **Unit**: *Pixels*, *Percent*, *Centimetres* or *Inches*.
- **Width** and **Height**: with **Keep the proportions** on, changing one changes the other
  too.
- **Resolution** (dpi): in centimetres and inches the size on paper stays and changing the
  dpi changes the pixels; with pixels and percent the dpi written in the file change.
- Quick buttons **25 %**, **50 %**, **75 %**, **200 %**.
- The **Result** box gives the final pixels, centimetres and megapixels.
- **Original size** removes the resizing.

The app resamples with the Lanczos method, which keeps details sharp. Enlarging does not add
detail: for a larger and sharper image, scan again at more dpi.

Rotations, crop, straightening and size apply to **all** formats, RAW and 16 bits included.

---

## Saving: the formats

The format is chosen in the **Output** tab; the button at the bottom right changes its name
with it (**Save PDF…**, **Save JPEG…**). Pressing it opens the system's window with a name
already filled in, for instance `documento-2026-11-08-153012.pdf`, in the
[scans folder](#the-preferences). Name and folder can be changed.

- In **Document** all the pages are saved.
- In **Photo** all the photos of the left column are saved (just one, if there is one).

### Document

| Format | What comes out | When |
|---|---|---|
| **PDF** | one file with all the pages | to send, print, file: the handiest |
| **Multi-page TIFF** | one file with all the pages, lossless | quality archive, OCR programs |
| **PNG** | one image per page, lossless | text and drawings to use as an image |
| **JPEG** | one image per page, compressed | colour pages to send in a hurry |
| **TIFF** | one image per page, lossless | like PNG, for programs that want TIFF |

For the PDF you also choose the **Sheet**: *Like the scan* (the page has the size of the
area acquired), A4, A3, A5, Letter, Legal, Photo 10 × 15 or 13 × 18 cm. On a real sheet you
then choose **On the sheet**:

| On the sheet | What it does | When |
|---|---|---|
| **Actual size** | the scan at its size, in the middle (it can be moved); shrunk only if it does not fit | printing at 100%: an A4 document stays A4, a 10 × 15 photo stays 10 × 15 |
| **Fit to the sheet** | the whole scan, as large as the sheet allows inside the **Margin** | a small crop (a detail, a passport photo) enlarged on an A4 |
| **Fill the sheet** | covers the sheet inside the margin, cutting what is left over at the sides | photo prints without white borders, on 10 × 15 or 13 × 18 |

**Orientation**: *Automatic* turns the sheet to landscape for scans wider than tall;
*Portrait* or *Landscape* keep all sheets that way (a landscape crop on a portrait A4, for
instance, goes at the top, as wide as the sheet).

**A page different from the others.** Orientation, *On the sheet* and margin apply to all
pages; for the one shown in the viewer, **Own layout for page N** gives it choices of its
own (in the pages column **sheet** appears next to the number). From there **Apply to all
pages** makes its choices those of the document (and removes the exceptions), **Like the
others** brings it back to those of the document.

**The sheet in the viewer.** With the **Output** panel open and a PDF on a real sheet, the
viewer shows the page on its sheet as it is in the PDF: the margin dashed and, **in red, the
edge of what ends up on paper**, always on top of the page. With **Fill the sheet** the part
left over stays visible, dimmed, outside the red edge: **drag the page** to choose which part
to keep (the choice stays with each page and the PDF cuts exactly like that).

**Where the page goes on the sheet.** A page smaller than the sheet is in the middle; drag
it to put it anywhere inside the margin, or move it with the **arrows** (1 mm; **Shift**
10 mm; **Alt** 0.1 mm, or a pixel of the scan when that is larger). Under **Position on the
sheet** the **Output** panel says how far it is from the left and top edges, in millimetres,
and you can type them; **Centre** (or **Home**) puts it back in the middle, **The same for
every page** gives its position to all pages, and so does **Apply to all pages** of a page
with its own layout. A page that fills the sheet one way moves only the other way.

To have exactly the shape of the sheet, crop first in the
[editor](#editing-a-scan-already-made) with the proportion *A4 / A3 / A5* or *Photo 10 × 15*:
with **Fit to the sheet** the crop then fills the whole sheet, margin apart.

#### The size of the PDF

Below, **Size of the PDF** says how large the file will be (*Estimated size: about …*),
updated at every change:

- **Quality** (from 30 to 100%): at **100%** the pages stay as scanned (JPEG 95). Lower down
  the JPEG quality drops and, below 90%, the maximum resolution (300 dpi, then 200, then
  150); colour pages that have no colour (a letter in black) go to grey, a third of the data.
  Black and white pages always stay sharp and lossless.
- **At most … MB**: the limit of the file (for instance 5 for an email attachment, or 2.5).
  The app lightens **one page at a time**, always the one that saves the most, until the PDF
  fits: pages already light (text) stay as they are, the large photo gives way first. Each
  page goes down by degrees, quality first and then resolution, never below 100 dpi and
  JPEG 45. Before saving the estimate says how many pages will be lightened and how far;
  afterwards, the message at the bottom gives the real size. If even so it does not fit, the
  PDF is saved anyway, at the minimum, and the app says so: then it is better to scan in Grey
  or Black and white, or at a lower resolution.

Quality and limit are remembered, separately for Document and Photo.

### Photo

| Format | When |
|---|---|
| **JPEG** | for everything: sharing, looking, printing. The most compatible |
| **PNG** | lossless, for photos still to be retouched |
| **TIFF** | lossless, for archives and printing programs |
| **WebP** | lossless like PNG, but smaller; for the web |
| **AVIF** | compressed like JPEG but much smaller; not every program opens it |
| **PDF** | to print: the photo on a sheet (A4, 10 × 15…) at its true size, fitted or full sheet, as for the documents above |
| **RAW · linear 16-bit TIFF** | to develop the photo in Photoshop or another photo editor |
| **RAW · DNG (Lightroom, Camera Raw)** | to develop the photo in Lightroom or Camera Raw |

### 16 bits and RAW

- **PNG 16 bit** and **TIFF 16 bit** (in both modes, with the 16-bit depth, in colour or
  grey): the finished image, **with** all the adjustments, but with 65,536 shades per colour.
  For whoever goes on retouching in a photo editor without losing quality.
- **RAW** (only in Photo, only in colour): the scanner's samples as the chip reads them,
  **without** adjustments, with the highlights intact. Crop, straightening, rotation and size
  apply; the colours, curves and other sliders of the app only serve to look at the preview.
  The photo is then "developed" in Lightroom, Camera Raw or Photoshop, like a camera's RAW
  photo. The scan for a RAW is always made at 16 bits, when the scanner gives them.

If you do not know what to choose: **JPEG** for photos, **PDF** for documents.

### The colours in the file

Every file says what its numbers mean, with an ICC colour profile written by VetroScan: sRGB's
colours and white, and the tone curve the scan was drawn with, which each scanner declares
(gamma 1.8 for the Epson 4490's glass, as Epson Scan's own files say; 2.2 for films and for
the HP 3770). Grey images have a grey profile; black and white none. The profile goes into
PNG, JPEG, TIFF (every page of a multi-page one, and at 16 bits), WebP and the pictures of a
PDF; AVIF and BMP have none. The numbers are the same as without it.

Programs that manage colour (Windows Photos, Photoshop, Lightroom, web browsers) read the
profile and show the file as it was meant on any monitor; VetroScan shows its pages the same
way (see **Screen colours** in [the Preferences](#the-preferences)). Programs that do not
show the numbers as they are: on a wide-gamut monitor the colours look more saturated there,
and an Epson 4490 glass scan has darker midtones there than in Windows Photos. The RAW files keep their own description (the linear TIFF its linear
sRGB profile, the DNG its own colour tags).

### Several pages or photos as separate images

With PNG, JPEG, TIFF, WebP and AVIF each page becomes a file: from the name chosen, for
instance `photo.jpg`, come `photo-01.jpg`, `photo-02.jpg`… in the same folder. With 16 bits
and RAW the files are called `photo-001.tif`, `photo-002.tif`… Typing a name that ends in
**`.zip`**, the pages all go instead into a single ZIP archive, as PNG images.

The save window asks for one name only, but its title says what happens: for example
*Save the 2 photos: one file per photo, with the number at the end of the name*, or *Save the
3 pages in one file* for a PDF. Closing the app with scans not saved, **Save…** in the
question saves them all in the same way, mode by mode.

**Black and white** is not saved as JPEG or AVIF (they would spoil the edges of the letters),
nor at 16 bits: the **Output** tab does not offer them.

### Automatic saving (Photo)

With **Save each photo to the folder by itself** (**Output** tab, only in Photo) each photo is
saved as soon as it is acquired, in the scans folder and with the automatic name, without
windows. The bar at the bottom and a notification say where.

---

## The scanner buttons and the photocopy

The three buttons on the front of the HP 3770 — **Scan**, **Copy** and **Instant Share** —
work with the app **even when it is in the background** or minimised. During a scan they are
ignored. What they do depends on the mode, and is changed in the
[Preferences](#the-preferences):

| Button | In **Document** (default) | In **Photo** (default) |
|---|---|---|
| **Scan** | Add a page | Scan the photo |
| **Copy** | Redo the last page | Photocopy (scan and print) |
| **Instant Share** | Save the document and open the folder | Open the folder |

Other choices: in Document *Open the folder* and *Nothing*; in Photo *Open the folder* and
*Nothing*.

Another scanner shows its own buttons, with the labels printed on it; until you choose, each
does what its maker meant it for (scan, copy, send). The choices are remembered for each
model of scanner.

- **Add a page** / **Scan the photo**: like the app's **Scan** button, with the area and
  settings of now. This way you acquire a whole document while standing at the scanner: turn
  the sheet, press Scan, and the notification says when the page is ready.
- **Redo the last page**: acquires the last page again and **replaces it at once**, without
  asking "Keep the new one".
- **Save the document and open the folder**: saves all the pages in the scans folder, in the
  format chosen and with the automatic name, without windows; then opens the folder.
- **Photocopy**: acquires the photo and sends it to the **default printer**. On Windows
  printing goes through Paint; on Linux and macOS through the printing system (CUPS, the `lp`
  command). If the automatic crop finds several originals on the glass, only the first is
  printed.

The **Scan** tab reminds at the bottom what the buttons do in the current mode, and under the
pages a note says which button adds a page (or scans a photo).

### Windows: the buttons and Windows' app

With the scanner maker's own driver installed (for the HP 3770, HP's), at every button press
Windows also opens its own scanning app, which takes the scanner away from this one. In the
**Preferences**, press **Use the scanner buttons with this app only** and confirm as
administrator: Windows stops reacting to the buttons. **Give the buttons back to Windows** puts
everything back as it was. The button only appears when Windows has such a driver for the
scanner in use (found by its USB id); with WinUSB the problem does not arise.

### Linux and macOS

On Linux no system service takes the buttons: there is nothing to set. On macOS they have not
been tried yet.

---

## The Preferences

The fourth tab on the right. Every change is saved by itself.

- **Language**: *Automatic (system language)* — English if the system is in English, Italian
  otherwise —, *Italiano* or *English*. The change applies at once, without restarting;
  numbers follow the language (2.5 in English, 2,5 in Italian).
- **Screen colours**: how the pictures are shown on the monitor. *Automatic: the monitor's
  profile* (the default) shows them as colour-managed programs do, from each page's profile
  to the profile of the monitor the window is on: on a wide-gamut monitor the colours are no
  longer too saturated, and an Epson 4490 glass scan has the midtones Windows Photos shows.
  The monitor is looked for again when the window moves to another monitor or the app comes
  back to the front. The monitor's profile is found by itself on **Windows** only, for now;
  elsewhere, or with no profile, *sRGB* is used. *sRGB* brings each page to sRGB (for a
  monitor set to sRGB); *No conversion* shows the numbers as they are, as before; *A profile
  of your choice…* takes a `.icc`/`.icm` file. **In use** says which profile is used, for
  which monitor. Only the picture on the screen changes: the files, the histogram and the
  numbers stay the same.
- **At start-up · Mode**: which mode the app opens with: *The last one used*, *Document* or
  *Photo*. Resolution, colour, format and PDF page of each mode are found again as you left
  them anyway; the adjustments start from zero (to find them again, save them in a preset).
- **Folder of the files**: where the scans are saved. **Open** shows it in the file manager
  (also **Open the folder**, next to **Save …**, and **Ctrl+O**). **Choose…** points to
  another one; the
  button next to it goes back to the default, **Scansioni** inside the **Pictures** folder
  (the folder keeps its Italian name in every language, so the scans stay together):

  | System | Default folder |
  |---|---|
  | Linux | `~/Pictures/Scansioni` (or `~/Immagini/Scansioni`, depending on the system's language) |
  | Windows | `C:\Users\<name>\Pictures\Scansioni` |
  | macOS | `~/Pictures/Scansioni` |

  If it does not exist, the app makes it the first time you save.
- **File names**: how file names start, separately for **Documents** and **Photo** (by
  default `documento` and `foto`, in Italian like the folder). Date and time come after;
  below there is an example.
- **Scanner**: with several scanners connected, **Use** chooses which one.
- **Scanner buttons in Document / in Photo**: what each button of the scanner in use does in
  the mode you are in. To set the other mode, go to it first. Only for a scanner with buttons.
- On **Windows**, with the scanner maker's driver installed: **Use the scanner buttons with
  this app only** (see above).
- **Lamp · Switch it off**: after how long without scans the app switches the lamp off: *As
  soon as the scan is over* (actually after 15 seconds, so that photos acquired one after the
  other do not each wait for the lamp), after 1, 2 (default), 5 or 10 minutes, or, for a
  scanner that switches its lamp off by itself, after that time (*the scanner switches it
  off*: 13 minutes on the HP 3770). When it closes the app always switches it off. A lamp
  switched off switches on again by itself at the next scan, with the warm-up the scanner
  declares (about 10 seconds on the HP 3770), as the note below the choice says. A scanner
  with its own timer switches the lamp off anyway, even if the app closes badly. Only for a
  scanner with a lamp.
- **Settings**: says which file they are saved in; **Restore factory settings** puts
  everything back as at the first start (modes, folder, names, buttons, theme, lamp, screen
  colours). The saved presets stay.

The **theme** (System, Light, Dark) is chosen at the top right, in the bar.

Preferences and presets are in the settings folder (see
[Where the settings are](install.md#where-the-settings-are)): they stay after an update.

---

## Keyboard and mouse shortcuts

On macOS, **⌘ Cmd** is used instead of **Ctrl**. The shortcuts do not work while typing in a
text field (a name, a number).

| Keys | What it does |
|---|---|
| **Ctrl+P** | Preview |
| **Ctrl+Enter** | Scan |
| **Ctrl+S** | Save |
| **Ctrl+O** | open the folder of the scans |
| **Ctrl+Z** | undo (in the editor: undo in the editor) |
| **Ctrl+Y** or **Ctrl+Shift+Z** | redo |
| **Ctrl+D** | duplicate the page shown |
| **Ctrl+A** | Photo and Film: pick every photo (**Esc** lets them go) |
| **F** | fit the image to the window |
| **1** | zoom 1:1 |
| **\\** | Before / After |
| **I** | eyedropper |
| **Esc** | switches the eyedropper off, lets the point of the curve go, or clears the selection; in the editor: **Discard** |
| **E** | opens the editor on the page shown |
| **Enter** | in the editor: **Apply** |
| **R** / **Shift+R** | in the editor: turn 90° right / left |
| **Page Down** / **Page Up** | the next / the previous page |
| **Alt+Page Down** / **Alt+Page Up** | moves the page shown after / before the next one |

**Moving precisely**, the same keys wherever something moves with the mouse: the point of the
curve clicked last, the crop in the editor, the page on its sheet (**Output** panel, PDF), the
area on the preview; with none of them, a zoomed image.

| Keys | What it does |
|---|---|
| **Arrows** | move by 1 mm (the curve: by one level; a zoomed image: by a bit) |
| **Shift** + arrows | by 10 mm (the curve: ten levels) |
| **Alt** + arrows | by 0.1 mm, or one pixel of the scan when that is larger |
| **Ctrl** + arrows | the size instead of the position (the right and bottom edges), same steps |
| **Home** | back to the middle |
| **[** / **]** | in the editor, *Crop*: straighten by −0.1° / +0.1° (with **Shift**: 1°) |
| **Tab** / **Shift+Tab** | on the curve: the next / the previous point |
| **Delete** | on the curve: removes the point chosen (not the two ends); on the preview: removes the area chosen |

At an edge it stops. A series of presses within a second is one step of **Ctrl+Z**, like a
slider dragged (where the mouse has **Ctrl+Z** too: curve and editor).

| Mouse | What it does |
|---|---|
| **Wheel** | zoom around the pointer |
| **Right** or **middle button** dragged | moves the image |
| **Left button** dragged | on the preview draws an area (on an empty part) or moves the one under it; in the editor, in *Crop*, the crop; elsewhere moves the image |
| **Double-click on a slider** | brings it back to its default |
| **Double-click on a thumbnail** | opens the editor |
| **Ctrl+click** / **Shift+click on a thumbnail** | Photo and Film: picks it / picks from the photo shown to it |

---

## Common problems

### "Scanner not found"

The dot at the top is red and the bar at the bottom says *Scanner not connected*.

1. Is the scanner **switched on** and plugged into the **mains**? Is the USB cable in
   properly, preferably directly in the computer (a hub is fine only if it has its own power
   supply)? Try another port.
2. **Only one program at a time** can use the scanner. Close another window of this app, the
   `hp3770` command in a terminal, and on Windows the scanning app of Windows or HP.
3. Click on the state at the top to try again. The app also tries again by itself: when the
   scanner comes back, the dot turns green again without restarting anything.
4. If that is not enough:
   - **Linux**: if the **Scan** tab says *The scanner is plugged in, but Linux does not let
     this user use it*, press **Allow access to the scanner** and type the computer's
     password: the app installs the `70-vetroscan.rules` rule and connects again by itself,
     without unplugging the cable. It is needed only once. Otherwise follow
     [Checking that it works (Linux)](install.md#checking-that-it-works-linux).
   - **Windows**: check the driver in *Device Manager*, as explained in
     [install.md](install.md#1-the-driver-check-what-is-already-there).
   - **macOS**: if the system asked to allow access to a USB accessory, it must have been
     granted.

### The scanner was unplugged during a scan

The scan stops and the bar at the bottom says **Error:** followed by an explanation (often
technical, in English). The pages already acquired stay. Plug the scanner back in, wait for
the green dot and repeat the scan (for a page that came out half, use **Redo**).

### The scan is very slow

- Look at the bottom left: **USB 1.1** means a slow port or hub. Plug the scanner into another
  port, directly on the computer.
- 1200 and 2400 dpi on a whole sheet take minutes and a lot of memory: select only the area
  needed or lower the resolution.
- The first scan after some rest waits for the lamp to warm up (about 10 seconds). If it
  happens too often, in **Preferences → Lamp** let the lamp switch off later.

### The preview does not find the photos on the glass

The automatic crop must be on (**Scan** tab). Originals smaller than 1.5 cm and white sheets
on the white lid can escape: draw the area by hand. If the originals touch each other, space
them a few millimetres apart and make the preview again.

### The scanner buttons do nothing

- During a scan they are ignored (the bar at the bottom says so).
- Check in the **Preferences** what they do in the mode you are in: they may be on *Nothing*.
- **Windows**: if Windows' app opens at every button, use **Use the scanner buttons with this
  app only**.
- Every press read ends up in the file `tasti.log`, in the settings folder: useful for a
  report.

### The photocopy does not print

A **default printer** is needed. On Windows set it in *Settings → Printers & scanners*. On
Linux and macOS printing goes through CUPS: the printer must be set up and be the default
(try `lp` followed by the name of an image in a terminal).

### "Save" does not open the window to choose the file (Linux)

The desktop portal is missing: see [Common problems (Linux)](install.md#common-problems-linux).

### "Saving failed"

- *black and white: no 16 bits*: a Black and white page is not saved as PNG or TIFF 16 bit.
  Choose another format, or put the page back to Grey or Colour in the editor.
- *nothing to save*: there is no scan yet.
- Otherwise, usually, the folder is not writable or the disk is full: choose another folder.

### "Settings not saved"

The app cannot write to the settings folder (disk full or permissions). Scans work anyway,
but at the next start you will find the old settings.

### I deleted a page by mistake

Delete cannot be undone (that is why, if the page is not saved, the app asks first): put the
original back on the glass and acquire it again, then drag it to its place.

### The app asks "Make a new scan?", "Close without saving?"…

Before an action that would throw away scans not saved yet, the app asks: **New document**, a
new scan in Photo or Film (it replaces the photos there), **Delete** and closing the window.
Changing mode never asks: nothing is thrown away. You can answer **Cancel** (or Esc: nothing
changes), **Save…** (saves, then does the action) or confirm the action. After a save,
automatic too, the app does not ask again until something changes.

Closing, the question counts the work of every mode (*Still to save: 3 pages in Document, 2
photos in Photo*). **Save…** saves the mode shown, or the first with work not saved, then asks
again for the next one; the window closes when nothing is left to save.

### Starting from scratch

**Preferences → Restore factory settings**. To delete the presets too, close the app and
delete the settings folder ([where it is](install.md#where-the-settings-are)).
