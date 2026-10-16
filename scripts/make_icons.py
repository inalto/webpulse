#!/usr/bin/env python3
"""Generate the WebPulse icons with the Python standard library only.

Writes:
  whm/addon_plugins/webpulse.png   48x48 RGBA PNG, transparent background
  cpanel/webpulse/webpulse.svg     48x48 SVG of the same design
  cpanel/plugin/webpulse.svg       copy of the SVG (referenced by install.json)

Design: a rounded blue square with a white pulse (heartbeat) polyline.
Usage: python3 scripts/make_icons.py            (run from anywhere)
"""
import math
import os
import struct
import zlib

SIZE = 48
SS = 4  # supersampling factor for anti-aliasing

BLUE = (37, 99, 235)       # #2563eb
BLUE_DARK = (29, 78, 216)  # #1d4ed8, bottom of the vertical gradient
WHITE = (255, 255, 255)

MARGIN = 2.0
RADIUS = 10.0
STROKE = 3.4
POLYLINE = [(8, 27), (16, 27), (20, 14), (26, 36), (30, 22), (33, 27), (40, 27)]

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def in_rounded_square(x, y):
    lo, hi = MARGIN, SIZE - MARGIN
    if not (lo <= x <= hi and lo <= y <= hi):
        return False
    cx = min(max(x, lo + RADIUS), hi - RADIUS)
    cy = min(max(y, lo + RADIUS), hi - RADIUS)
    return (x - cx) ** 2 + (y - cy) ** 2 <= RADIUS ** 2


def dist_to_segment(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    length2 = dx * dx + dy * dy
    t = 0.0 if length2 == 0 else max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / length2))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def on_pulse(x, y):
    half = STROKE / 2.0
    return any(
        dist_to_segment(x, y, ax, ay, bx, by) <= half
        for (ax, ay), (bx, by) in zip(POLYLINE, POLYLINE[1:])
    )


def background(y):
    t = (y - MARGIN) / (SIZE - 2 * MARGIN)
    t = min(max(t, 0.0), 1.0)
    return tuple(BLUE[i] + (BLUE_DARK[i] - BLUE[i]) * t for i in range(3))


def render():
    rows = []
    for py in range(SIZE):
        row = bytearray()
        for px in range(SIZE):
            r = g = b = a = 0.0
            for sy in range(SS):
                for sx in range(SS):
                    x = px + (sx + 0.5) / SS
                    y = py + (sy + 0.5) / SS
                    if not in_rounded_square(x, y):
                        continue
                    col = WHITE if on_pulse(x, y) else background(y)
                    r += col[0]
                    g += col[1]
                    b += col[2]
                    a += 1.0
            n = SS * SS
            if a == 0:
                row += bytes((0, 0, 0, 0))
            else:
                # straight (non-premultiplied) alpha
                row += bytes((round(r / a), round(g / a), round(b / a), round(255 * a / n)))
        rows.append(bytes(row))
    return rows


def png_chunk(tag, data):
    body = tag + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def write_png(path, rows):
    raw = b"".join(b"\x00" + row for row in rows)  # filter type 0 per scanline
    data = (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
        + png_chunk(b"IDAT", zlib.compress(raw, 9))
        + png_chunk(b"IEND", b"")
    )
    with open(path, "wb") as fh:
        fh.write(data)


def svg():
    points = " ".join("%g,%g" % p for p in POLYLINE)
    lo = MARGIN
    side = SIZE - 2 * MARGIN
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48">\n'
        '  <title>WebPulse Analytics</title>\n'
        '  <defs>\n'
        '    <linearGradient id="wp-bg" x1="0" y1="0" x2="0" y2="1">\n'
        '      <stop offset="0" stop-color="#%02x%02x%02x"/>\n'
        '      <stop offset="1" stop-color="#%02x%02x%02x"/>\n'
        '    </linearGradient>\n'
        '  </defs>\n'
        '  <rect x="%g" y="%g" width="%g" height="%g" rx="%g" ry="%g" fill="url(#wp-bg)"/>\n'
        '  <polyline points="%s" fill="none" stroke="#ffffff" stroke-width="%g"'
        ' stroke-linecap="round" stroke-linejoin="round"/>\n'
        '</svg>\n'
    ) % (BLUE + BLUE_DARK + (lo, lo, side, side, RADIUS, RADIUS, points, STROKE))


def write_text(path, text):
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(text)


def main():
    png_path = os.path.join(REPO, "whm", "addon_plugins", "webpulse.png")
    svg_paths = [
        os.path.join(REPO, "cpanel", "webpulse", "webpulse.svg"),
        os.path.join(REPO, "cpanel", "plugin", "webpulse.svg"),
    ]
    for p in [png_path] + svg_paths:
        os.makedirs(os.path.dirname(p), exist_ok=True)
    write_png(png_path, render())
    for p in svg_paths:
        write_text(p, svg())
    print("wrote", os.path.relpath(png_path, REPO))
    for p in svg_paths:
        print("wrote", os.path.relpath(p, REPO))


if __name__ == "__main__":
    main()
