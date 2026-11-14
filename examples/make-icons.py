#!/usr/bin/env python3
"""Draws icon.png for each example app: a coloured tile with a white shape.

    python3 examples/make-icons.py

Needs nothing but Python. Shapes are signed distance functions, so edges come
out smooth at any size.
"""
import math, os, struct, zlib

SIZE = 256


def seg(ax, ay, bx, by, r):
    """A line with round ends, `r` thick each side."""
    def d(x, y):
        px, py, vx, vy = x - ax, y - ay, bx - ax, by - ay
        t = max(0.0, min(1.0, (px * vx + py * vy) / (vx * vx + vy * vy or 1)))
        return math.hypot(px - vx * t, py - vy * t) - r
    return d


def box(x0, y0, x1, y1, r=0.0):
    cx, cy, hw, hh = (x0 + x1) / 2, (y0 + y1) / 2, (x1 - x0) / 2 - r, (y1 - y0) / 2 - r
    def d(x, y):
        qx, qy = abs(x - cx) - hw, abs(y - cy) - hh
        return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - r
    return d


def circle(cx, cy, r):
    return lambda x, y: math.hypot(x - cx, y - cy) - r


def ring(shape, w):
    """The outline of a shape, `w` thick each side."""
    return lambda x, y: abs(shape(x, y)) - w


def any_of(*shapes):
    return lambda x, y: min(s(x, y) for s in shapes)


def minus(a, b):
    return lambda x, y: max(a(x, y), -b(x, y))


def upper(shape, y_max):
    return lambda x, y: max(shape(x, y), y - y_max)


# Coordinates are on a 0..100 grid.
ICONS = {
    "counter": ("#2f9e44", any_of(seg(50, 28, 50, 72, 6), seg(28, 50, 72, 50, 6))),
    "todo": ("#1c7ed6", any_of(seg(27, 52, 43, 68, 6), seg(43, 68, 74, 34, 6))),
    "fetch": ("#e8590c", any_of(seg(50, 24, 50, 60, 6), seg(33, 46, 50, 63, 6), seg(67, 46, 50, 63, 6), seg(28, 78, 72, 78, 6))),
    "sandbox": ("#c92a2a", any_of(
        minus(box(27, 46, 73, 80, 7), any_of(circle(50, 60, 5), seg(50, 60, 50, 69, 2.5))),
        upper(ring(box(35, 22, 65, 70, 15), 4.5), 46))),
    "inbox": ("#7048e8", any_of(ring(box(22, 30, 78, 70, 6), 4), seg(26, 35, 50, 54, 4), seg(74, 35, 50, 54, 4))),
    "slack": ("#0c8599", any_of(seg(42, 26, 36, 74, 5), seg(64, 26, 58, 74, 5), seg(27, 40, 75, 40, 5), seg(25, 60, 73, 60, 5))),
    "widgets": ("#1a73e8", any_of(box(24, 24, 46, 46, 5), circle(65, 35, 11), box(24, 58, 76, 66, 4), ring(box(24, 70, 76, 78, 4), 1.2))),
}


def render(color, glyph, size):
    rgb = [int(color[i:i + 2], 16) for i in (1, 3, 5)]
    tile = box(6, 6, 94, 94, 20)
    k = 100.0 / size
    rows = []
    for py in range(size):
        row = bytearray([0])
        for px in range(size):
            x, y = (px + 0.5) * k, (py + 0.5) * k
            a = max(0.0, min(1.0, 0.5 - tile(x, y) / k))
            g = max(0.0, min(1.0, 0.5 - glyph(x, y) / k))
            # A little lighter at the top than the bottom.
            shade = 1.08 - 0.16 * y / 100
            row += bytes(min(255, round(c * shade * (1 - g) + 255 * g)) for c in rgb)
            row.append(round(a * 255))
        rows.append(bytes(row))
    return b"".join(rows)


def png(raw, size):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))
    head = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", head) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


here = os.path.dirname(os.path.abspath(__file__))
for name, (color, glyph) in ICONS.items():
    path = os.path.join(here, name, "icon.png")
    with open(path, "wb") as f:
        f.write(png(render(color, glyph, SIZE), SIZE))
    print(path)
