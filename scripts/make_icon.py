#!/usr/bin/env python3
"""Draws the Kite app icon: a white kite on a blue gradient circle, in the
manner of Telegram's paper plane.

Writes assets/icon/logo.png (the bare circle, for in-app use),
assets/icon/icon.png (1024px macOS icon with the system's padding and
shadow) and assets/icon/Kite.icns. Needs Pillow; iconutil ships with macOS.

    python3 scripts/make_icon.py
"""

import math
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets" / "icon"
SS = 4  # supersampling factor

TOP = (64, 184, 245)  # gradient, top
BOTTOM = (26, 136, 214)  # gradient, bottom
WHITE = (255, 255, 255, 255)


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def rotate(points, degrees, origin):
    a = math.radians(degrees)
    ox, oy = origin
    return [
        (
            ox + (x - ox) * math.cos(a) - (y - oy) * math.sin(a),
            oy + (x - ox) * math.sin(a) + (y - oy) * math.cos(a),
        )
        for x, y in points
    ]


def gradient_disc(size):
    """A circle filling `size`, shaded top to bottom."""
    grad = Image.new("RGBA", (size, size))
    draw = ImageDraw.Draw(grad)
    for y in range(size):
        draw.line([(0, y), (size, y)], fill=lerp(TOP, BOTTOM, y / size) + (255,))
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).ellipse([0, 0, size - 1, size - 1], fill=255)
    disc = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    disc.paste(grad, (0, 0), mask)
    return disc


def kite_glyph(size):
    """The white kite with its tail, on a transparent square of `size`."""
    layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    s = size / 1000  # design units: a 1000-unit square
    cx, cy = 590 * s, 380 * s  # kite centre, up and to the right
    tilt = 24  # leaning into the wind, nose up and right

    # Diamond kite: short top, wide shoulders, long body.
    h, w = 255 * s, 185 * s
    top, right = (cx, cy - h), (cx + w, cy - 0.22 * h)
    bottom, left = (cx, cy + 1.08 * h), (cx - w, cy - 0.22 * h)
    body = rotate([top, right, bottom, left], tilt, (cx, cy))
    draw.polygon(body, fill=WHITE)

    # Spars as cuts showing the blue through, like the folds of Telegram's plane.
    cut = (0, 0, 0, 0)
    spar = round(16 * s)
    t, r, b, l = body
    draw.line([t, b], fill=cut, width=spar)
    draw.line([l, r], fill=cut, width=spar)

    # Tail: a gentle S from the bottom tip, sweeping down and to the left.
    start = b
    steps = 120
    points = []
    for i in range(steps + 1):
        u = i / steps
        x = start[0] - 250 * s * u
        y = start[1] + 300 * s * u + 30 * s * math.sin(u * math.pi * 2)
        points.append((x, y))
    width = round(20 * s)
    draw.line(points, fill=WHITE, width=width, joint="curve")
    for x, y in (points[0], points[-1]):
        draw.ellipse([x - width / 2, y - width / 2, x + width / 2, y + width / 2], fill=WHITE)

    # Bows across the tail: two wings meeting on the string.
    for u in (0.4, 0.78):
        i = round(u * steps)
        (x, y), (x2, y2) = points[i], points[i + 1]
        length = math.hypot(x2 - x, y2 - y)
        tx, ty = (x2 - x) / length, (y2 - y) / length  # along the string
        nx, ny = -ty, tx  # across it
        reach, spread = 42 * s, 22 * s
        for side in (1, -1):
            tip = (x + nx * reach * side, y + ny * reach * side)
            draw.polygon(
                [
                    (x, y),
                    (tip[0] + tx * spread, tip[1] + ty * spread),
                    (tip[0] - tx * spread, tip[1] - ty * spread),
                ],
                fill=WHITE,
            )
    # Leave the rim some air: shrink about the centre.
    scale = 0.88
    small = round(size * scale)
    framed = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    framed.paste(layer.resize((small, small), Image.LANCZOS), ((size - small) // 2,) * 2)
    return framed


def logo(size):
    """The circle with the kite, `size` pixels square."""
    big = size * SS
    disc = gradient_disc(big)
    glyph = kite_glyph(big)
    # Cuts (transparent strokes) must show the gradient beneath: punch the
    # glyph's alpha out of a white layer instead of compositing directly.
    white = Image.new("RGBA", (big, big), WHITE)
    alpha = glyph.getchannel("A")
    disc = Image.composite(white, disc, alpha)
    # Keep the disc's own shape.
    disc.putalpha(ImageChops.multiply(disc.getchannel("A"), gradient_disc(big).getchannel("A")))
    return disc.resize((size, size), Image.LANCZOS)


def mac_icon(size=1024):
    """macOS icon grid: the art spans 824 of 1024 px, with a soft shadow."""
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    art = round(size * 824 / 1024)
    inset = (size - art) // 2
    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).ellipse(
        [inset, inset + size * 0.012, inset + art, inset + art + size * 0.012],
        fill=(0, 0, 0, 90),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(size * 0.014))
    canvas.alpha_composite(shadow)
    canvas.alpha_composite(logo(art), (inset, inset))
    return canvas


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    logo(512).save(OUT / "logo.png")
    icon = mac_icon(1024)
    icon.save(OUT / "icon.png")
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "Kite.iconset"
        iconset.mkdir()
        for base in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                px = base * scale
                name = f"icon_{base}x{base}{'@2x' if scale == 2 else ''}.png"
                icon.resize((px, px), Image.LANCZOS).save(iconset / name)
        if shutil.which("iconutil"):
            subprocess.run(
                ["iconutil", "-c", "icns", str(iconset), "-o", str(OUT / "Kite.icns")],
                check=True,
            )
    print(f"Wrote {OUT}")


if __name__ == "__main__":
    main()
