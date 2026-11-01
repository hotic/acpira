"""Renders icons/icon.svg at one size without gradient banding: python3 icon-render.py <svg> <px> <out.png> [windows]

The tile's gradient spans ~14 levels per channel, so librsvg's 8-bit render steps every ~25 px. The svg is split into
three layers rendered by rsvg-convert: the drop shadow (#drop), the tile's coverage (#backdrop painted white) and
everything above the tile. The gradient is computed in floating point across #backdrop's box, the layers are composited
in floating point, and the colour is dithered by one level before it is quantized. Needs numpy and Pillow.

`windows` renders the .ico frames. The svg is drawn on the macOS grid (an inset tile plus a shadow, which the Dock lays
out for), but Windows shows an icon's canvas as is, next to icons that fill theirs (VS Code, Chrome), so this variant
crops the canvas to the tile, drops the shadow and tightens the corners. Frames up to 64 px also nudge the glyph by
sub-pixel offsets and a few percent of scale to land its edges on the pixel grid, which a plain downscale smears."""

import io
import subprocess
import sys
import xml.etree.ElementTree as ET

import numpy as np
from PIL import Image

SVG_NS = 'http://www.w3.org/2000/svg'
ET.register_namespace('', SVG_NS)


def layer(root, keep, white_backdrop=False):
  """An svg string holding the <defs> plus the top-level elements whose id passes `keep`"""
  out = ET.Element(root.tag, root.attrib)
  for el in root:
    if el.tag == f'{{{SVG_NS}}}defs' or keep(el.get('id')):
      el = ET.fromstring(ET.tostring(el))
      if white_backdrop and el.get('id') == 'backdrop':
        el.set('fill', '#fff')
      out.append(el)
  return ET.tostring(out)


# The tile's corner radius on Windows, as a share of its side (the macOS grid uses ~22.5%)
WINDOWS_CORNER = 0.18
# Glyph hinting search for small Windows frames: offsets in pixels and scales around the tile centre
HINT_MAX_PX = 64
HINT_OFFSETS = np.linspace(-0.5, 0.5, 9)
HINT_SCALES = (0.96, 0.98, 1.0, 1.02, 1.04)


def windows_variant(root):
  """Crops the viewBox to #backdrop, removes #drop and scales the tile's corners down"""
  backdrop = next(el for el in root if el.get('id') == 'backdrop')
  x, y, side = (float(backdrop.get(k)) for k in ('x', 'y', 'width'))
  root.set('viewBox', f'{x:g} {y:g} {side:g} {side:g}')
  for el in list(root):
    if el.get('id') == 'drop':
      root.remove(el)
    elif el.tag == f'{{{SVG_NS}}}rect' and el.get('rx'):
      # The backdrop and its hairline stroke share the corner; the stroke sits half a unit inside
      inset = float(el.get('x')) - x
      el.set('rx', f'{side * WINDOWS_CORNER - inset:g}')


def glyph(root):
  """The top-level element drawn over the tile that carries the mark (the one <g>)"""
  return next(el for el in root if el.tag == f'{{{SVG_NS}}}g')


def hint(root, px):
  """Picks the glyph offset and scale whose edges fall closest to whole pixels at px, and applies it"""
  g = glyph(root)
  base = g.get('transform', '')
  vx, vy, vw, _ = (float(v) for v in root.get('viewBox').split())
  cx, cy, unit = vx + vw / 2, vy + vw / 2, vw / px
  # An svg of the glyph alone; ElementTree keeps no parent links, so g stays in root as well
  alone = ET.Element(root.tag, root.attrib)
  alone.append(g)
  best = None
  for s in HINT_SCALES:
    for dy in HINT_OFFSETS:
      for dx in HINT_OFFSETS:
        g.set('transform', f'translate({cx + dx * unit:g} {cy + dy * unit:g}) scale({s:g}) translate({-cx:g} {-cy:g}) {base}')
        a = render(ET.tostring(alone), px)[..., 3]
        # Partial coverage is what reads as blur; prefer the untouched glyph on ties
        score = (np.minimum(a, 1.0 - a).sum(), abs(s - 1.0), abs(dx) + abs(dy))
        if best is None or score < best[0]:
          best = (score, g.get('transform'))
  g.set('transform', best[1])


def render(svg, px):
  """Premultiplied float RGBA in 0..1"""
  png = subprocess.run(['rsvg-convert', '-w', str(px), '-h', str(px)], input=svg, capture_output=True, check=True).stdout
  a = np.asarray(Image.open(io.BytesIO(png)).convert('RGBA'), dtype=np.float64) / 255.0
  a[..., :3] *= a[..., 3:]
  return a


def main(svg_path, px, out_path, variant=None):
  px = int(px)
  root = ET.parse(svg_path).getroot()
  if variant == 'windows':
    windows_variant(root)
    if px <= HINT_MAX_PX:
      hint(root, px)
  backdrop = next(el for el in root if el.get('id') == 'backdrop')
  grad_id = backdrop.get('fill')[len('url(#'):-1]
  grad = next(el for el in root.iter(f'{{{SVG_NS}}}linearGradient') if el.get('id') == grad_id)
  stops = [el.get('stop-color') for el in grad.iter(f'{{{SVG_NS}}}stop')]
  top, bottom = (np.array([int(c[i:i + 2], 16) for i in (1, 3, 5)], dtype=np.float64) / 255.0 for c in stops)

  shadow = render(layer(root, lambda i: i == 'drop'), px)
  cover = render(layer(root, lambda i: i == 'backdrop', white_backdrop=True), px)[..., 3:]
  above = render(layer(root, lambda i: i not in ('drop', 'backdrop')), px)

  # A vertical objectBoundingBox gradient (x1=x2, y1=0, y2=1) across the backdrop's box, sampled at pixel centres
  _, vy, _, view = (float(v) for v in root.get('viewBox').split())
  y0, h = (float(backdrop.get('y')) - vy) / view, float(backdrop.get('height')) / view
  t = np.clip(((np.arange(px) + 0.5) / px - y0) / h, 0.0, 1.0)[:, None, None]
  tile = np.concatenate([np.broadcast_to(top + (bottom - top) * t, (px, px, 3)) * cover, cover], axis=2)

  # Source-over, bottom to top, on premultiplied colour
  img = tile + shadow * (1.0 - tile[..., 3:])
  img = above + img * (1.0 - above[..., 3:])

  alpha = img[..., 3:]
  rgb = np.divide(img[..., :3], alpha, out=np.zeros_like(img[..., :3]), where=alpha > 0) * 255.0
  # Triangular dither of one level, fixed seed so a re-render is byte-identical
  rng = np.random.default_rng(px)
  rgb += rng.random(rgb.shape) - rng.random(rgb.shape)
  out = np.concatenate([rgb, alpha * 255.0], axis=2)
  Image.fromarray(np.clip(np.round(out), 0, 255).astype(np.uint8), 'RGBA').save(out_path, optimize=True)


if __name__ == '__main__':
  main(*sys.argv[1:])
