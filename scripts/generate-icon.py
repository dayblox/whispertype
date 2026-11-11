"""Rasterize the app's simple waveform icon into a multi-size Windows ICO."""
import math
import pathlib
import struct

ROOT = pathlib.Path(__file__).resolve().parent.parent


def rounded(x, y, cx, cy, hw, hh, radius):
    dx, dy = abs(x - cx) - hw + radius, abs(y - cy) - hh + radius
    return math.hypot(max(dx, 0), max(dy, 0)) + min(max(dx, dy), 0) - radius


def bitmap(size):
    pixels = bytearray()
    scale = size / 64
    for y in reversed(range(size)):
        for x in range(size):
            px, py = (x + .5) / scale, (y + .5) / scale
            distance = rounded(px, py, 32, 32, 31, 31, 16)
            alpha = min(1, max(0, .5 - distance * scale))
            color = [23., 26., 35.]
            border = min(1, max(0, 1 - abs(distance + .5) * scale))
            color = [base + border * (edge - base) for base, edge in zip(color, [82, 110, 105])]
            for cx, height in [(16, 14), (24, 26), (32, 38), (40, 26), (48, 14)]:
                amount = min(1, max(0, .5 - rounded(px, py, cx, 32, 2, height / 2, 2) * scale))
                color = [base + amount * (bar - base) for base, bar in zip(color, [158, 206, 106])]
            pixels.extend([int(color[2]), int(color[1]), int(color[0]), round(alpha * 255)])
    mask = bytes(((size + 31) // 32) * 4 * size)
    header = struct.pack('<IIIHHIIIIII', 40, size, size * 2, 1, 32, 0, len(pixels), 0, 0, 0, 0)
    return header + pixels + mask


sizes = [16, 24, 32, 48, 64, 128, 256]
images = [bitmap(size) for size in sizes]
offset = 6 + 16 * len(images)
directory = bytearray(struct.pack('<HHH', 0, 1, len(images)))
for size, data in zip(sizes, images):
    directory.extend(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
    offset += len(data)
(ROOT / 'assets/whispertype.ico').write_bytes(directory + b''.join(images))
