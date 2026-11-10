---
name: scanner-measuring
description: How to measure a scanner before changing what it does — noise by density from pairs of identical scans, the converter's ceiling (saturation the calibration hides), the black, the lamp's warm-up, stripes, a film's base — with tools/measure (noise.py, levels.py), the pitfalls already met on the HP 3770, and the order for bringing up a new scanner (the Epson 4490). Use it before choosing or changing an exposure, a line period, a lamp, a calibration or a film/imaging feature, before claiming anything about noise or saturation, and when a new scanner arrives.
---

# Measuring a scanner

VetroScan changes what a scanner does only after measuring it on real scans (CLAUDE.md:
*Measure before building*). This is how, and what went wrong before.

## Ground rules

- **The motor first.** Never send a resolution, mode, line period or motor setting that was
  never run on that scanner (on the 3770 a 200 dpi film preview ground the motor). A new one
  comes from a capture of the maker's driver doing it, is guarded in the driver before
  anything moves, and Matteo is told before he tries it. Measuring tools are no exception:
  `tma-scan --triple-line` refuses everything but films at 1200 dpi.
- **Say what is measured and what is assumed.** "Computed", "expected", "measured on two
  slides" are different claims; Matteo checks.
- **Numbers stay in the driver** (four levels): a measure becomes a profile value or a
  declared capability, never a constant in the app or `scan-imaging`.
- **Write it down where it lasts**: the scans in `../scansioni/<topic>/` (keep the
  `NAME-16bit.png`, the linear samples), the dated diary in `../captures/<topic>/note.txt`
  (Italian, numbers and files named), the conclusion in `docs/architecture-plan.md` at its
  step. Show Matteo a comparison picture before asking him to choose (a crop at 100 % and the
  same lifted ×4 for the shadows).

## The tools (`tools/measure`, numpy only)

Run with Python's isolated mode: `py -3.13 -I` on this PC, `python3 -I` on Linux/macOS. Each
has `--selftest`; run it after changing the tool.

| Tool | What it says |
|---|---|
| `noise.py --dpi D --box CX,CY,W,H [--black CX,CY,W,H] A:B [C:D ...]` | signal/noise by density band for each pair of identical scans, side by side; the black's level and noise; the pair's shift |
| `levels.py --dpi D [--box ...] SCAN ...` | percentiles, highest sample, how many sit on it, share above 98 % |
| `hp3770 stripes FILE.png` | the period and strength of the stripes on a scan of something uniform |
| `hp3770 tma-scan` / `develop` | scans and develops the 3770's adapter; prints each frame's centre and size in mm, which `--box` takes |

The 3770's first scripts (`../captures/tma/misura_*.py`, Italian, outside the repository) are
its history: `misura_diapo.py` chose its bands from one noisy scan, which hides noise in the
shadows; `noise.py` replaces it.

## The method

1. **A pair per setting.** Scan the same thing twice in a row without touching anything (A,
   B), then the other setting twice (C, D). The noise is what differs within a pair. Never
   take one setting's noise from a scan of another: a scan shifted by a line makes detail
   look like noise (on 10 November a 3x scan compared with a 2x one "lost" half its
   signal/noise in the light parts; against its own twin it had not).
2. **Noise by density**, `noise.py` with the frame's box and an opaque part as `--black`
   (a slide's mount, the holder). Read the flat parts first (the black, smooth skies): a
   leftover shift below a pixel inflates the noise where there is detail. If the pair's shift
   is not 0 0, say so with the numbers.
3. **The ceiling, before calling anything unsaturated.** The calibration scales everything
   back to its white, so a longer exposure looks just as bright, and a converter that
   saturates comes back as a flat value well below 65535. Find something at full light in
   every scan (the holder's gaps, the calibration slot, bare glass beside the original) and
   compare its value between settings with `levels.py`: where it drops, that is the new
   ceiling, and its density, log10(full light / ceiling), is the lightest part of an original
   that still comes out. On 10 November the 3x line's ceiling was at density 0.23; "nothing
   saturated" had looked at the slides' maximum only, and the change was taken back.
4. **Repeatability.** Two scans in a row must agree (the 3770: within 0.2-1 % once its lamp is
   steady). Absolute levels differ between days (a whole ×2 once, cause not found): compare
   ratios (base / slot, frame / base), not raw numbers from different days.
5. **Report**: per setting the signal/noise per band, the black, the ceiling, the time per
   scan, what was not measured; then the choice is Matteo's.

## Pitfalls already met

- **Gain is not light.** Raising a gain scales the noise with the signal: the signal/noise
  stays (measured, 10 November). Only more light on the sensor (exposure, lamp) helps, and
  only while below the ceiling.
- **Choosing bands by a noisy value hides noise**: a pixel lands in a band because of its
  noise. `noise.py` takes the band from the 5 × 5 mean of both scans.
- **The lamp moves**: the 3770's adapter lamp gains about 18 % in its first 15 s; calibrate
  only after readings are steady (5 in a row within 3 %).
- **The holder moves** between scans (0.2-0.6 mm once, untouched): expect shifts.
- **Know what is in the picture**: a "sky" was a dusk sky, a colour film was chromogenic black
  and white; a measuring box caught the dark band by the holder's edge (highlights 0.91 vs
  1.12 between two scans). Look at the crop before trusting a number from it.
- **Stripes are fixed**: averaging scans does not remove them (`scan_imaging::stripes` does);
  measure their period on a uniform area, not on a busy crop.
- **A maker's numbers are hints**: HP's colour matrices and per-colour exposures belong to
  HP's own pipeline; tried as they are, red saturated or colours went too far.

## Bringing up a new scanner (the Epson 4490 next)

1. Captures of the maker's driver for every operation to be sent (USBPcap on Windows;
   `crates/hp3770-capture` reads them): start-up, lamp, calibration, a scan at each
   resolution and mode that will be offered, films if any. Nothing is sent that no capture
   or earlier run shows.
2. Declare in `Capabilities` only what is verified; the rest stays undeclared.
3. Lamp: warm-up time and stability (`LampInfo`), whether the scanner switches it off.
4. Calibration: white and black, shading, repeatability of two scans in a row.
5. Stripes per source and resolution (`Stripes`).
6. Noise and ceiling for each mode, with pairs.
7. Films: where the adapter's light ends (scan longer than the area), the base, slides and
   negatives separately, their ceiling.
8. Colour: the maker's matrices as a hint; a measured profile needs an IT8 target.
