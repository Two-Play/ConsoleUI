#!/usr/bin/env python3
"""Renders the images of the ConsoleUI Plymouth theme (Story 6.11).

The boot splash shows what the shell's own boot screen (crates/shell/ui/boot.slint) shows: the
"ConsoleUI" logo with a pulsing glow behind it and three staggered loading dots. consoleui.script
places, scales and animates them like boot.slint (same scale factor, sizes and timings), so the
system boot hands over to the shell without a visible change, like a console that shows its logo
from power-on until the home screen. Only still images are needed: Plymouth animates their opacity.

All images are rendered at twice the 1920x1080 reference size, so they stay sharp on 4K screens;
the script scales them by s / 2.

Needs Pillow and the Noto Sans fonts. Run it after changing the look; the PNGs are checked in, so
installing the theme needs neither:  python3 session/plymouth/generate.py
"""

import math
import os
import sys

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "consoleui")
FONT_DIR = "/usr/share/fonts/google-noto"

# Design tokens (crates/ui/ui/theme.slint) and the boot screen's own colors
TEXT = (0xF4, 0xF6, 0xFB)
GLOW = (0x6A, 0x5C, 0xFF)
GLOW_ALPHA = 0x66 / 255

X = 2  # images are rendered at twice the reference size
SS = 4  # supersampling for smooth edges

# boot.slint at the reference size (s = 1)
LOGO_SIZE = 96
LOGO_SPACING = 4
DOT_SIZE = 12
GLOW_BOX = 900
# Slint's circle gradient spans half the box diagonal; the glow fades out at 60 % of it
GLOW_RADIUS = GLOW_BOX / math.sqrt(2) * 0.6


def font(name, size):
    path = os.path.join(FONT_DIR, name)
    if not os.path.exists(path):
        sys.exit(f"missing font {path} (install google-noto-sans-fonts)")
    return ImageFont.truetype(path, size)


def path(name):
    return os.path.join(OUT, name)


def glow():
    """The radial glow at full strength (the script animates its opacity).

    Slint interpolates #6a5cff66 to transparent black: color and alpha both fall linearly. One pixel
    per reference pixel is plenty for a gradient this soft (the script scales it by s, not s / 2)."""
    size = math.ceil(2 * GLOW_RADIUS)
    img = Image.new("RGBA", (size, size))
    pixels = []
    for y in range(size):
        for x in range(size):
            t = min(1.0, math.hypot(x + 0.5 - size / 2, y + 0.5 - size / 2) / GLOW_RADIUS)
            k = 1 - t
            pixels.append(tuple(round(c * k) for c in GLOW) + (round(255 * GLOW_ALPHA * k),))
    img.putdata(pixels)
    img.save(path("glow.png"), optimize=True)


def logo():
    """The logo text with letter spacing; the image is exactly the text's line box in boot.slint."""
    f = font("NotoSans-ExtraBold.ttf", LOGO_SIZE * X * SS)
    text = "ConsoleUI"
    widths = [f.getlength(c) for c in text]
    spacing = LOGO_SPACING * X * SS
    total = sum(widths) + spacing * (len(text) - 1)
    ascent, descent = f.getmetrics()
    w, h = math.ceil(total / SS), math.ceil((ascent + descent) / SS)
    big = Image.new("RGBA", (w * SS, h * SS), TEXT + (0,))
    draw = ImageDraw.Draw(big)
    x = (w * SS - total) / 2
    for c, cw in zip(text, widths):
        draw.text((x, 0), c, font=f, fill=TEXT + (255,))
        x += cw + spacing
    big.resize((w, h), Image.LANCZOS).save(path("logo.png"), optimize=True)


def circle(diameter, name):
    d = diameter * X
    big = Image.new("RGBA", (d * SS, d * SS), TEXT + (0,))
    ImageDraw.Draw(big).ellipse((0, 0, d * SS - 1, d * SS - 1), fill=TEXT + (255,))
    big.resize((d, d), Image.LANCZOS).save(path(name), optimize=True)


def rounded(w, h, radius, fill, outline, width, name):
    w, h, radius, width = w * X, h * X, radius * X, width * X
    big = Image.new("RGBA", (w * SS, h * SS), (255, 255, 255, 0))
    ImageDraw.Draw(big).rounded_rectangle(
        (0, 0, w * SS - 1, h * SS - 1), radius * SS, fill=fill, outline=outline, width=width * SS
    )
    big.resize((w, h), Image.LANCZOS).save(path(name), optimize=True)


def lock():
    w, h = 40 * X, 48 * X
    u = X * SS  # one reference pixel in the supersampled image
    big = Image.new("RGBA", (w * SS, h * SS), TEXT + (0,))
    d = ImageDraw.Draw(big)
    white = TEXT + (255,)
    d.arc((9 * u, 2 * u, 31 * u, 26 * u), 180, 360, fill=white, width=4 * u)
    d.line((11 * u, 14 * u, 11 * u, 22 * u), fill=white, width=4 * u)
    d.line((29 * u, 14 * u, 29 * u, 22 * u), fill=white, width=4 * u)
    d.rounded_rectangle((3 * u, 20 * u, 37 * u, 46 * u), 6 * u, fill=white)
    d.ellipse((17 * u, 28 * u, 23 * u, 34 * u), fill=TEXT + (0,))
    d.rectangle((19 * u, 32 * u, 21 * u, 39 * u), fill=TEXT + (0,))
    big.resize((w, h), Image.LANCZOS).save(path("lock.png"), optimize=True)


def main():
    os.makedirs(OUT, exist_ok=True)
    for name in os.listdir(OUT):
        if name.endswith(".png"):
            os.remove(path(name))
    glow()
    logo()
    circle(DOT_SIZE, "dot.png")
    # Password prompt: a glass text field like crates/ui/ui/textfield.slint, bullets, a padlock
    rounded(600, 64, 16, (255, 255, 255, 22), (255, 255, 255, 70), 2, "entry.png")
    circle(14, "bullet.png")
    lock()
    # Progress bar of offline updates (track and fill; the script stretches the fill)
    rounded(600, 8, 4, (255, 255, 255, 40), None, 0, "progress-track.png")
    rounded(600, 8, 4, TEXT + (255,), None, 0, "progress-fill.png")


if __name__ == "__main__":
    main()
