"""Renders icons/icon.svg at one size without gradient banding: python3 icon-render.py <svg> <px> <out.png>

The tile's gradient spans ~14 levels per channel, so librsvg's 8-bit render steps every ~25 px. The svg is split into
three layers rendered by rsvg-convert: the drop shadow (#drop), the tile's coverage (#backdrop painted white) and
everything above the tile. The gradient is computed in floating point across #backdrop's box, the layers are composited
in floating point, and the colour is dithered by one level before it is quantized. Needs numpy and Pillow."""

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


def render(svg, px):
  """Premultiplied float RGBA in 0..1"""
  png = subprocess.run(['rsvg-convert', '-w', str(px), '-h', str(px)], input=svg, capture_output=True, check=True).stdout
  a = np.asarray(Image.open(io.BytesIO(png)).convert('RGBA'), dtype=np.float64) / 255.0
  a[..., :3] *= a[..., 3:]
  return a


def main(svg_path, px, out_path):
  px = int(px)
  root = ET.parse(svg_path).getroot()
  backdrop = next(el for el in root if el.get('id') == 'backdrop')
  grad_id = backdrop.get('fill')[len('url(#'):-1]
  grad = next(el for el in root.iter(f'{{{SVG_NS}}}linearGradient') if el.get('id') == grad_id)
  stops = [el.get('stop-color') for el in grad.iter(f'{{{SVG_NS}}}stop')]
  top, bottom = (np.array([int(c[i:i + 2], 16) for i in (1, 3, 5)], dtype=np.float64) / 255.0 for c in stops)

  shadow = render(layer(root, lambda i: i == 'drop'), px)
  cover = render(layer(root, lambda i: i == 'backdrop', white_backdrop=True), px)[..., 3:]
  above = render(layer(root, lambda i: i not in ('drop', 'backdrop')), px)

  # A vertical objectBoundingBox gradient (x1=x2, y1=0, y2=1) across the backdrop's box, sampled at pixel centres
  view = float(root.get('viewBox').split()[3])
  y0, h = float(backdrop.get('y')) / view, float(backdrop.get('height')) / view
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
