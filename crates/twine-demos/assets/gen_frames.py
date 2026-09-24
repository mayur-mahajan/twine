#!/usr/bin/env python3
"""Generates the images of the `controls` demo (RGBA PNGs, no dependencies).

- `frame0.png` … `frame3.png` (32x32): a grey ring with a blue quarter sector, rotated by 90
  degrees per frame (the animimg).
- `power_off.png`, `power_on.png` (36x36): a grey / blue disc with a white power sign (the
  image button).

Everything is anti-aliased with 4x4 supersampling. Run from the repository root:

    python3 crates/twine-demos/assets/gen_frames.py
    for n in frame0 frame1 frame2 frame3 power_off power_on; do
      N=$(echo $n | tr a-z A-Z)
      cargo run -p twine-cli -- image --in crates/twine-demos/assets/$n.png --format argb8888 \
        --name $N --crate-path twine::image --out crates/twine-demos/src/assets/$n.rs
    done
"""
import math
import os
import struct
import zlib

SIZE = 32
SS = 4
RING = (0x9E, 0x9E, 0x9E)
FILL = (0x21, 0x96, 0xF3)


def png(path, w, h, rgba):
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes(rgba[y * w * 4:(y + 1) * w * 4]) for y in range(h))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 9)))
        f.write(chunk(b"IEND", b""))


def sample(x, y, start):
    c = SIZE / 2
    dx, dy = x - c, y - c
    r = math.hypot(dx, dy)
    if r > 15:
        return None
    ang = (math.degrees(math.atan2(dy, dx)) - start) % 360
    if ang < 90 and r <= 15:
        return FILL
    if r >= 12:
        return RING
    return None


def power(x, y, fill):
    c = 18
    dx, dy = x - c, y - c
    r = math.hypot(dx, dy)
    if r > 17:
        return None
    # The power sign: an open ring (gap at the top) and a vertical bar.
    if abs(dx) <= 1.5 and -11 <= dy <= -1:
        return (255, 255, 255)
    if 7 <= r <= 10 and not (dy < 0 and abs(dx) < 5):
        return (255, 255, 255)
    return fill


def render(size, fn):
    out = []
    for y in range(size):
        for x in range(size):
            acc = [0, 0, 0, 0]
            for sy in range(SS):
                for sx in range(SS):
                    col = fn(x + (sx + 0.5) / SS, y + (sy + 0.5) / SS)
                    if col:
                        acc[0] += col[0]
                        acc[1] += col[1]
                        acc[2] += col[2]
                        acc[3] += 1
            n = acc[3]
            if n:
                out += [acc[0] // n, acc[1] // n, acc[2] // n, 255 * n // (SS * SS)]
            else:
                out += [0, 0, 0, 0]
    return out


here = os.path.dirname(os.path.abspath(__file__))
for i in range(4):
    png(os.path.join(here, f"frame{i}.png"), SIZE, SIZE, render(SIZE, lambda x, y: sample(x, y, i * 90 - 90)))
png(os.path.join(here, "power_off.png"), 36, 36, render(36, lambda x, y: power(x, y, RING)))
png(os.path.join(here, "power_on.png"), 36, 36, render(36, lambda x, y: power(x, y, FILL)))
