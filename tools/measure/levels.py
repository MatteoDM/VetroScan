# SPDX-License-Identifier: GPL-2.0-or-later
"""Where the samples of a scan lie, per channel, and whether any saturate.

    levels.py --dpi 1200 [--box CX,CY,W,H] SCAN-16bit.png [MORE-16bit.png ...]

For each scan (a NAME-16bit.png, linear samples), inside the box in mm or
the whole scan: the percentiles, the highest sample, how many samples sit
at it and how many above 98 % of full scale. A channel saturates when many
samples pile up on one highest value (the converter's end, or the
shading's ceiling): a few on 65535 can be a speck of dust's hole, thousands
are clipped light. Use it before trusting a longer exposure, a stronger
lamp or a new calibration: a clipped film base gives wrong colours.
"""
import argparse
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import scanimg as si  # noqa: E402

PERCENTILES = (0.1, 1, 50, 99, 99.9)


def levels(samples):
    """Per channel: (percentiles, highest, samples at the highest, share
    above 98 % of full scale)."""
    out = []
    for ch in range(samples.shape[2]):
        v = samples[..., ch].ravel()
        top = float(v.max())
        out.append((np.percentile(v, PERCENTILES), top, int((v == top).sum()),
                    float((v > 0.98 * si.FULL).mean())))
    return out


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--dpi", type=int, required=True)
    p.add_argument("--box", help="CX,CY,W,H in mm; the whole scan without it")
    p.add_argument("scans", nargs="+")
    args = p.parse_args(argv)
    names = "RGB"
    print("percentiles " + " ".join(f"{q}%" for q in PERCENTILES))
    for path in args.scans:
        img = si.read_png16(path)
        part = si.cut(img, si.box(args.box, args.dpi)) if args.box else img
        print(os.path.basename(path))
        for ch, (pct, top, at_top, above) in enumerate(levels(part)):
            name = names[ch] if part.shape[2] == 3 else "grey"
            print(f"  {name}: " + " ".join(f"{v:7.0f}" for v in pct)
                  + f"   highest {top:6.0f} ({at_top} samples)   above 98 %: {above * 100:.3f} %")


def selftest():
    rng = np.random.default_rng(2)
    img = rng.uniform(1000, 30000, (200, 200, 3))
    img[:20, :, 0] = si.FULL  # a clipped band in red
    (_, top, at_top, above), (_, top_g, at_top_g, above_g), _ = levels(img)
    assert top == si.FULL and at_top == 20 * 200 and abs(above - 0.1) < 1e-9
    assert top_g < si.FULL and at_top_g == 1 and above_g == 0.0
    print("selftest ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    else:
        main(sys.argv[1:])
