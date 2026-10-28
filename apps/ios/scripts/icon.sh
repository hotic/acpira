#!/bin/sh
# Renders AppIcon.svg into the asset catalog's single 1024 px icon. The tile's gradient spans only ~14 levels per
# channel, so a plain 8-bit render shows bands; the tile is painted here in floating point with dither instead, and
# rsvg-convert draws the glyph alone on top. The PNG is opaque RGB: App Store Connect rejects an alpha channel.
# Needs rsvg-convert (brew install librsvg) and python3 with numpy + Pillow
set -eu
cd "$(dirname "$0")/.."
out=Acpira/Assets.xcassets/AppIcon.appiconset/AppIcon.png
glyph=$(mktemp -t acpira-icon).png
grep -v 'id="backdrop"' AppIcon.svg | rsvg-convert -w 1024 -h 1024 -o "$glyph"
python3 - "$glyph" "$out" <<'PY'
import sys
import numpy as np
from PIL import Image

glyph_path, out_path = sys.argv[1:]
# The stops of the svg's #tile gradient, top to bottom
top = np.array([0x2d, 0x2e, 0x33], dtype=np.float64)
bottom = np.array([0x1f, 0x20, 0x24], dtype=np.float64)
t = np.linspace(0.0, 1.0, 1024)[:, None, None]
tile = np.broadcast_to(top + (bottom - top) * t, (1024, 1024, 3))

g = np.asarray(Image.open(glyph_path).convert('RGBA'), dtype=np.float64)
a = g[..., 3:] / 255.0
img = g[..., :3] * a + tile * (1.0 - a)

# Triangular dither of one level, fixed seed so a re-render is byte-identical
rng = np.random.default_rng(0)
noise = rng.random(img.shape) - rng.random(img.shape)
img = np.clip(np.round(img + noise), 0, 255).astype(np.uint8)
Image.fromarray(img, 'RGB').save(out_path, optimize=True)
PY
rm -f "$glyph"
