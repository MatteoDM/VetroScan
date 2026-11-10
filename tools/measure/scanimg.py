# SPDX-License-Identifier: GPL-2.0-or-later
"""What the measuring tools share: reading a scan's linear 16-bit samples,
boxes given in millimetres, densities, and lining two scans up.

Only numpy is needed (no Pillow: it does not read 16-bit RGB PNGs as such).
Run the tools with Python's isolated mode (`py -3 -I` on Windows,
`python3 -I` elsewhere), so that nothing beside the scans is imported.
"""
import struct
import zlib

import numpy as np

FULL = 65535.0


def read_png16(path):
    """A 16-bit RGB (or grey) PNG, not interlaced, as a float array
    (height, width, channels): the NAME-16bit.png the scanners' tools save
    (`hp3770 tma-scan`, VetroScan's 16-bit output)."""
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path}: not a PNG")
    pos, idat, ihdr = 8, [], None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
        pos += 12 + length
    width, height, depth, colour, _, _, interlace = ihdr
    channels = {0: 1, 2: 3}.get(colour)
    if depth != 16 or channels is None or interlace:
        raise ValueError(f"{path}: not a 16-bit grey or RGB PNG, not interlaced")
    bpp = 2 * channels
    stride = width * bpp
    raw = np.frombuffer(zlib.decompress(b"".join(idat)), dtype=np.uint8)
    raw = raw.reshape(height, stride + 1)
    out = np.empty((height, stride), dtype=np.uint8)
    prev = np.zeros(stride, dtype=np.int32)
    for y in range(height):
        f, line = raw[y, 0], raw[y, 1:].astype(np.int32)
        if f == 0:
            cur = line
        elif f == 2:
            cur = (line + prev) & 0xFF
        else:
            cur = np.empty(stride, dtype=np.int32)
            for x in range(stride):
                a = cur[x - bpp] if x >= bpp else 0
                b = prev[x]
                c = prev[x - bpp] if x >= bpp else 0
                if f == 1:
                    p = a
                elif f == 3:
                    p = (a + b) // 2
                else:
                    pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                    p = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                cur[x] = (line[x] + p) & 0xFF
        out[y] = cur
        prev = cur
    samples = out.view(">u2").reshape(height, width, channels)
    return samples.astype(np.float64)


def box(text, dpi):
    """`CX,CY,W,H` in mm (the centre and size, as `hp3770 tma-scan` prints
    a frame) -> (y0, y1, x0, x1) in pixels at `dpi`."""
    cx, cy, w, h = (float(v) for v in text.split(","))
    px = dpi / 25.4
    return (int((cy - h / 2) * px), int((cy + h / 2) * px),
            int((cx - w / 2) * px), int((cx + w / 2) * px))


def shrink(b, share):
    """The middle `share` of a box (0.9: away from a frame's edges)."""
    y0, y1, x0, x1 = b
    dy, dx = (y1 - y0) * (1 - share) / 2, (x1 - x0) * (1 - share) / 2
    return int(y0 + dy), int(y1 - dy), int(x0 + dx), int(x1 - dx)


def density(samples):
    """Optical density of linear samples, -log10(sample / full scale)."""
    return -np.log10(np.maximum(samples, 1) / FULL)


def shift(ref, img, b):
    """Rows and columns `img` is displaced from `ref` inside box `b`, found
    by phase correlation on the log of green: whole pixels only. Whatever is
    left below a pixel still shows as noise where the picture has detail."""
    y0, y1, x0, x1 = b
    g = lambda im: np.log(np.maximum(im[y0:y1, x0:x1, min(1, im.shape[2] - 1)], 1))
    a, c = g(ref), g(img)
    r = np.fft.fft2(a - a.mean()) * np.conj(np.fft.fft2(c - c.mean()))
    corr = np.fft.ifft2(r / np.maximum(np.abs(r), 1e-12)).real
    dy, dx = np.unravel_index(np.argmax(corr), corr.shape)
    dy = dy - corr.shape[0] if dy > corr.shape[0] // 2 else dy
    dx = dx - corr.shape[1] if dx > corr.shape[1] // 2 else dx
    return -int(dy), -int(dx)


def cut(img, b, dy=0, dx=0):
    """Box `b` of `img`, displaced by (dy, dx) as `shift` found it."""
    y0, y1, x0, x1 = b
    return img[y0 + dy:y1 + dy, x0 + dx:x1 + dx]


def iqr_sigma(x):
    """A standard deviation that ignores outliers: the middle half's spread
    (interquartile range / 1.349)."""
    return (np.percentile(x, 75) - np.percentile(x, 25)) / 1.349
