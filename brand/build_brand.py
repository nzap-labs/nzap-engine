"""Build NZAP brand assets from the source logos.

Sources (1536x1024): light.png (black chrome, for light backgrounds) and
dark.png (chrome with bright edge highlights, for dark ones), both
transparent, plus dark-silver.webp (silver chrome on solid black), the
in-app mark of the dark theme.
"""

import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

SRC = Path(sys.argv[1])
OUT = Path(sys.argv[2])
OUT.mkdir(parents=True, exist_ok=True)

light = Image.open(SRC / "light.png").convert("RGBA")
dark = Image.open(SRC / "dark.png").convert("RGBA")


def solid_alpha(image, threshold=24):
    """The faint 1-3 alpha haze over the whole canvas becomes fully clear."""
    r, g, b, a = image.split()
    a = a.point(lambda v: 0 if v < threshold else min(255, int((v - threshold) * 255 / (255 - threshold))))
    return Image.merge("RGBA", (r, g, b, a))


light, dark = solid_alpha(light), solid_alpha(dark)


def from_black(image, floor=6):
    """Un-blend artwork rendered on solid black: alpha is the brightest
    channel and colour is divided by it, so the result composited over black
    is the original, and its glow fades into any dark stage."""
    rgb = np.asarray(image.convert("RGB")).astype(np.float64)
    peak = rgb.max(axis=2)
    alpha = np.clip((peak - floor) / (255 - floor), 0, 1)
    colour = np.where(peak[..., None] > 0, rgb * 255 / np.maximum(peak, 1)[..., None], 0)
    out = np.dstack([np.clip(colour, 0, 255), alpha * 255]).round().astype(np.uint8)
    return Image.fromarray(out, "RGBA")


silver = solid_alpha(from_black(Image.open(SRC / "dark-silver.webp")))


def split_rows(image):
    """Find the gap between the mark and the wordmark (a band of empty rows)."""
    alpha = image.getchannel("A")
    width, height = image.size
    filled = [alpha.crop((0, y, width, y + 1)).getbbox() is not None for y in range(height)]
    # The wordmark is the last filled run; the mark is everything above the gap before it.
    y = height - 1
    while y > 0 and not filled[y]:
        y -= 1
    while y > 0 and filled[y]:
        y -= 1
    word_top = y + 1
    while y > 0 and not filled[y]:
        y -= 1
    return y + 1, word_top


mark_bottom, word_top = split_rows(light)
print("mark ends at", mark_bottom, "wordmark starts at", word_top)


def trim(image, pad=0):
    box = image.getchannel("A").getbbox()
    box = (max(0, box[0] - pad), max(0, box[1] - pad), min(image.width, box[2] + pad), min(image.height, box[3] + pad))
    return image.crop(box)


def square(image, size, fill=0.86):
    """Center on a transparent square canvas, scaled to `fill` of the side."""
    scale = size * fill / max(image.size)
    resized = image.resize((round(image.width * scale), round(image.height * scale)), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    canvas.alpha_composite(resized, ((size - resized.width) // 2, (size - resized.height) // 2))
    return canvas


mark_light = trim(light.crop((0, 0, light.width, mark_bottom)))
mark_dark = trim(dark.crop((0, 0, dark.width, mark_bottom)))
silver_bottom, _ = split_rows(silver)
mark_silver = trim(silver.crop((0, 0, silver.width, silver_bottom)))
full_light = trim(light)
full_dark = trim(dark)

# The wordmark as a pure alpha mask (white, alpha = ink coverage), so pages can
# tint it with CSS `mask-image` + `background: currentColor`.
word = trim(light.crop((0, word_top, light.width, light.height)))
# The letters are opaque in the source; their chrome highlights are colour only.
ink = word.getchannel("A")
wordmark = Image.merge("RGBA", (*[Image.new("L", word.size, 255)] * 3, ink))

mark_light.save(OUT / "nzap-mark-light.png", optimize=True)
mark_dark.save(OUT / "nzap-mark-dark.png", optimize=True)
square(mark_light, 512).save(OUT / "nzap-mark-light-512.png", optimize=True)
square(mark_dark, 512).save(OUT / "nzap-mark-dark-512.png", optimize=True)
full_light.save(OUT / "nzap-logo-light.png", optimize=True)
full_dark.save(OUT / "nzap-logo-dark.png", optimize=True)
wordmark.save(OUT / "nzap-wordmark-mask.png", optimize=True)


def rounded_mask(size, radius):
    mask = Image.new("L", (size * 4, size * 4), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size * 4 - 1, size * 4 - 1), radius * 4, fill=255)
    return mask.resize((size, size), Image.LANCZOS)


def app_icon(size=1024):
    """A dark tile with a soft studio light behind the chrome mark (macOS
    proportions: 824px tile with 100px margins on a 1024 canvas)."""
    tile = 824
    margin = (size - tile) // 2
    base = Image.new("RGBA", (tile, tile), (12, 12, 14, 255))
    # vertical sheen
    sheen = Image.linear_gradient("L").resize((tile, tile)).point(lambda v: int(26 * (1 - v / 255)))
    base = Image.composite(Image.new("RGBA", (tile, tile), (60, 60, 68, 255)), base, sheen)
    # studio light behind the mark
    glow = Image.new("L", (tile, tile), 0)
    ImageDraw.Draw(glow).ellipse((tile * 0.12, tile * 0.10, tile * 0.88, tile * 0.86), fill=150)
    glow = glow.filter(ImageFilter.GaussianBlur(tile * 0.14))
    base = Image.composite(Image.new("RGBA", (tile, tile), (120, 122, 132, 255)), base, glow)
    mark = square(mark_dark, tile, fill=0.74)
    base.alpha_composite(mark)
    # hairline inner border
    border = Image.new("RGBA", (tile, tile), (0, 0, 0, 0))
    ImageDraw.Draw(border).rounded_rectangle((1, 1, tile - 2, tile - 2), int(tile * 0.225), outline=(255, 255, 255, 38), width=3)
    base.alpha_composite(border)
    icon = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    icon.paste(base, (margin, margin), rounded_mask(tile, int(tile * 0.225)))
    return icon


app_icon().save(OUT / "app-icon-1024.png", optimize=True)

# Full-bleed square variant (Windows/Linux like edge-to-edge tiles; favicon).
def favicon(size=512):
    tile = app_icon(1024).crop((100, 100, 924, 924)).resize((size, size), Image.LANCZOS)
    return tile


favicon().save(OUT / "favicon-512.png", optimize=True)

# Social / Open Graph card 1200x630: dark stage, logo centered.
og = Image.new("RGBA", (1200, 630), (9, 9, 11, 255))
glow = Image.new("L", (1200, 630), 0)
ImageDraw.Draw(glow).ellipse((300, 40, 900, 560), fill=110)
glow = glow.filter(ImageFilter.GaussianBlur(120))
og = Image.composite(Image.new("RGBA", (1200, 630), (110, 112, 124, 255)), og, glow)
logo = mark_dark.copy()
scale = 380 / logo.height
logo = logo.resize((round(logo.width * scale), 380), Image.LANCZOS)
og.alpha_composite(logo, ((1200 - logo.width) // 2, 70))
wm = wordmark.copy()
wscale = 520 / wm.width
wm = wm.resize((520, round(wm.height * wscale)), Image.LANCZOS)
tinted = Image.new("RGBA", wm.size, (236, 236, 240, 255))
tinted.putalpha(wm.getchannel("A"))
og.alpha_composite(tinted, ((1200 - wm.width) // 2, 500))
og.convert("RGB").save(OUT / "og-card.png", optimize=True)

print("wrote", sorted(p.name for p in OUT.iterdir()))

# Small sizes for in-app use (2x of the largest rendering). The dark theme
# uses the silver mark; the app icon keeps the dark chrome one.
for name, mark in (("light", mark_light), ("dark", mark_silver)):
    square(mark, 160, fill=0.94).save(OUT / f"nzap-mark-{name}-160.png", optimize=True)
favicon(64).save(OUT / "favicon-64.png", optimize=True)
print("small sizes done")

# Just "NZAP" (left of the widest gap between letters), for the app's sidebar.
alpha = wordmark.getchannel("A")
cols = [alpha.crop((x, 0, x + 1, alpha.height)).getbbox() is not None for x in range(alpha.width)]
gaps, start = [], None
for x, filled in enumerate(cols + [True]):
    if not filled and start is None:
        start = x
    elif filled and start is not None:
        gaps.append((x - start, start))
        start = None
widest = max(gaps)[1]
trim(wordmark.crop((0, 0, widest, wordmark.height))).save(OUT / "nzap-word-mask.png", optimize=True)
print("word split at", widest)
