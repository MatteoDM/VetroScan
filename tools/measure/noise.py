# SPDX-License-Identifier: GPL-2.0-or-later
"""The noise of a scanner, by density, from pairs of identical scans.

Scan the same thing twice in a row without touching it (A and B), with the
same settings: what differs between them is noise. Each pair gives, in each
band of density inside the box, the noise (sigma, in samples) and the
signal/noise; several pairs (one per setting: a longer line, another lamp)
print side by side, each measured against its own twin, never against
another setting's scan (a scan shifted by a line against another makes
detail look like noise).

    noise.py --dpi 1200 --box CX,CY,W,H [--black CX,CY,W,H] A:B [C:D ...]

A, B... are NAME-16bit.png files (linear samples). The box is in mm (as
`hp3770 tma-scan` prints a frame); only its middle 0.9 is measured. --black
is an opaque part (a slide's mount, a holder) whose noise is the scanner's
own black: flat, so a leftover shift does not matter there.

Read it so:
- the pair's shift is printed: if not 0 0, whole pixels are lined up but
  what is left below a pixel adds to the noise where the picture has
  detail; trust the flat parts (the black, smooth skies) first;
- the gain g of B against A is printed: two scans in a row may differ by a
  few percent (the lamp); the noise is taken from B - g A;
- a band needs at least 500 pixels to be printed.
"""
import argparse
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import scanimg as si  # noqa: E402

EDGES = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5]


def local_mean(x, k=5):
    """The mean of each pixel's k x k neighbourhood (edges: the nearest
    whole neighbourhood), with cumulative sums: numpy only."""
    h, w = x.shape
    c = np.pad(x, ((1, 0), (1, 0))).cumsum(0).cumsum(1)
    s = c[k:, k:] - c[:-k, k:] - c[k:, :-k] + c[:-k, :-k]
    m = s / (k * k)
    r = k // 2
    return np.pad(m, ((r, h - m.shape[0] - r), (r, w - m.shape[1] - r)), mode="edge")


def pair_noise(a, b, edges=EDGES):
    """For each channel and band of density: (band, pixels, level, sigma,
    signal/noise), from the lined-up boxes `a` and `b`. The band of a pixel
    comes from the 5 x 5 mean of both scans, not from `a` alone: chosen by
    a noisy value, a band would keep the pixels whose noise put them there
    and hide part of it (most in the shadows, where the noise is large
    against the level)."""
    out = []
    for ch in range(a.shape[2]):
        bright = a[..., ch] > 2000
        g = float(np.median(b[..., ch][bright] / a[..., ch][bright])) if bright.any() else 1.0
        va, vb = a[..., ch].ravel(), b[..., ch].ravel()
        dv = si.density(local_mean((a[..., ch] + b[..., ch] / g) / 2).ravel())
        rows = []
        for lo, hi in zip(edges[:-1], edges[1:]):
            m = (dv >= lo) & (dv < hi)
            if m.sum() < 500:
                continue
            level = float(np.median(va[m]))
            sigma = si.iqr_sigma(vb[m] - g * va[m]) / np.sqrt(1 + g * g)
            rows.append(((lo, hi), int(m.sum()), level, sigma, level / sigma))
        out.append((g, rows))
    return out


def black_noise(img, b):
    """Level and noise of a flat opaque part: the median, and the spread of
    the differences between neighbouring pixels (which leaves out a slow
    unevenness)."""
    part = si.cut(img, b)
    d = np.diff(part, axis=1).reshape(-1, part.shape[2]) / np.sqrt(2)
    return np.median(part.reshape(-1, part.shape[2]), 0), d.std(0)


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--dpi", type=int, required=True)
    p.add_argument("--box", required=True, help="CX,CY,W,H in mm")
    p.add_argument("--black", help="CX,CY,W,H in mm of an opaque part")
    p.add_argument("pairs", nargs="+", help="A:B, two identical scans")
    args = p.parse_args(argv)
    b = si.shrink(si.box(args.box, args.dpi), 0.9)
    names = "RGB"
    results = []
    for pair in args.pairs:
        na, nb = pair.split(":")
        a, bb = si.read_png16(na), si.read_png16(nb)
        dy, dx = si.shift(a, bb, b)
        print(f"{os.path.basename(na)} : {os.path.basename(nb)}   shift {dy} rows {dx} columns")
        if args.black:
            k = si.box(args.black, args.dpi)
            for n, img in ((na, a), (nb, bb)):
                level, noise = black_noise(img, k)
                print(f"  black {os.path.basename(n)}: level {level.astype(int)}  noise {np.round(noise, 1)}")
        results.append((pair, pair_noise(si.cut(a, b), si.cut(bb, b, dy, dx))))
    print("\nSIGNAL/NOISE BY DENSITY (level of the first scan of each pair; sigma in samples)")
    for ch in range(len(results[0][1])):
        print(f"  {names[ch] if len(results[0][1]) == 3 else 'grey'}")
        bands = sorted({r[0] for _, res in results for r in res[ch][1]})
        for band in bands:
            cells = []
            for _, res in results:
                row = next((r for r in res[ch][1] if r[0] == band), None)
                cells.append(f"sigma {row[3]:6.1f} S/N {row[4]:6.1f}" if row else f"{'-':>24}")
            print(f"    density {band[0]:.1f}-{band[1]:.1f}: " + "   ".join(cells))
    print("\npairs: " + "   ".join(pair for pair, _ in results))


def selftest():
    """Two made-up scans with a known noise: the tool must find it."""
    rng = np.random.default_rng(1)
    level = np.repeat(np.geomspace(60000, 100, 400)[:, None], 300, axis=1)
    clean = np.stack([level] * 3, axis=2)
    noise = 50.0
    a = clean + rng.normal(0, noise, clean.shape)
    b = clean * 1.02 + rng.normal(0, noise, clean.shape)
    for g, rows in pair_noise(a, b):
        assert abs(g - 1.02) < 0.01, g
        for _, _, _, sigma, _ in rows:
            assert abs(sigma - noise) < 0.1 * noise, sigma
    flat = np.full((100, 100, 3), 200.0) + rng.normal(0, noise, (100, 100, 3))
    _, n = black_noise(flat, (0, 100, 0, 100))
    assert np.all(abs(n - noise) < 0.1 * noise), n
    moved = np.roll(a, (3, -2), axis=(0, 1))
    # The picture moved 3 rows down and 2 columns left: found so, and the
    # box cut there gives back the same pixels.
    found = si.shift(a, moved, (50, 350, 50, 250))
    assert found == (3, -2), found
    assert np.array_equal(si.cut(moved, (50, 350, 50, 250), *found), si.cut(a, (50, 350, 50, 250)))
    print("selftest ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    else:
        main(sys.argv[1:])
