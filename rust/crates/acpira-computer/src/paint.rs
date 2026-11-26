//! The overlay's pixels, for a system that gives the driver a bitmap to fill instead of layers (Windows): the white
//! frame around the window the agent works in, and the agent's pointer with its click ripple. Sizes are in the points
//! macOS draws the same things with (`mac/overlay.rs`), times `scale`.
//!
//! Shapes are drawn from their distance fields, so edges come out smooth without a rasterizer. Pixels are
//! premultiplied `0xAARRGGBB`, the layout of a 32-bit device-independent bitmap.

/// The frame is white with a soft white glow, over a thin dark rim that keeps it visible on a light desktop.
const FRAME_COLOR: (f64, f64, f64) = (1.0, 1.0, 1.0);
const FRAME_LINE: f64 = 2.5;
/// How far the dark rim shows on each side of the white line.
const FRAME_RIM: f64 = 0.5;
const FRAME_GLOW: f64 = 7.0;
/// The frame sits this far outside the window edge.
const FRAME_OUTSET: f64 = 2.0;
/// Room around the window for the line and its glow.
pub const FRAME_MARGIN: f64 = 16.0;

/// The agent's pointer is the system's arrow in black and white, a size up so it is not taken for the user's own.
const POINTER_FILL: (f64, f64, f64) = (0.06, 0.06, 0.07);
const POINTER_EDGE: (f64, f64, f64) = (1.0, 1.0, 1.0);
const POINTER_LINE: f64 = 1.8;
/// The arrow, tip first, in a y-down space with the tip at the origin.
const ARROW: [(f64, f64); 7] = [(0.0, 0.0), (0.0, 21.0), (4.9, 16.3), (8.9, 25.2), (12.4, 23.7), (8.5, 15.0), (15.0, 15.0)];
/// The pointer's bitmap and where the tip is in it: room for the ripple around the tip and the arrow's shadow below.
pub const POINTER_SIZE: (f64, f64) = (60.0, 60.0);
pub const POINTER_TIP: (f64, f64) = (24.0, 24.0);
const RIPPLE_RADIUS: f64 = 9.0;

pub struct Bitmap {
  pub width: usize,
  pub height: usize,
  pub pixels: Vec<u32>,
}

impl Bitmap {
  fn new(width: usize, height: usize) -> Bitmap {
    Bitmap { width, height, pixels: vec![0; width * height] }
  }

  /// Lays a colour over one pixel; `alpha` is 0 to 1.
  fn over(&mut self, x: usize, y: usize, color: (f64, f64, f64), alpha: f64) {
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha <= 0.0 {
      return;
    }
    let Some(pixel) = self.pixels.get_mut(y * self.width + x) else { return };
    let channel = |shift: u32| f64::from((*pixel >> shift) & 0xFF) / 255.0;
    let (a, r, g, b) = (channel(24), channel(16), channel(8), channel(0));
    let keep = 1.0 - alpha;
    let byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    *pixel = byte(alpha + a * keep) << 24 | byte(color.0 * alpha + r * keep) << 16 | byte(color.1 * alpha + g * keep) << 8 | byte(color.2 * alpha + b * keep);
  }
}

/// How much of a pixel a shape covers, from the pixel centre's distance to the shape's edge (negative inside).
fn cover(distance: f64) -> f64 {
  (0.5 - distance).clamp(0.0, 1.0)
}

/// A soft falloff over `radius`, for glows and shadows.
fn soft(distance: f64, radius: f64) -> f64 {
  let t = (distance / radius).clamp(0.0, 1.0);
  (1.0 - t) * (1.0 - t)
}

/// Distance from a point to a rounded rectangle centred on the origin (negative inside).
fn rounded_box(x: f64, y: f64, half: (f64, f64), radius: f64) -> f64 {
  let (qx, qy) = (x.abs() - half.0 + radius, y.abs() - half.1 + radius);
  qx.max(qy).min(0.0) + (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() - radius
}

/// The frame for a window of `window` pixels with corners of `radius` points. The bitmap is larger than the window by
/// `FRAME_MARGIN` points on every side.
pub fn frame(window: (usize, usize), radius: f64, scale: f64) -> Bitmap {
  let margin = (FRAME_MARGIN * scale).ceil();
  let (width, height) = (window.0 + 2 * margin as usize, window.1 + 2 * margin as usize);
  let mut bitmap = Bitmap::new(width, height);
  let outset = FRAME_OUTSET * scale;
  let half = (window.0 as f64 / 2.0 + outset, window.1 as f64 / 2.0 + outset);
  let radius = ((radius + FRAME_OUTSET) * scale).min(half.0).min(half.1);
  let (line, glow) = (FRAME_LINE * scale / 2.0, FRAME_GLOW * scale);
  // Only a band along the edges holds anything: the middle of a large window is never visited.
  let band = (margin + outset + radius + glow).ceil() as usize + 1;
  for y in 0..height {
    let edge_row = y < band || y + band >= height;
    let mut x = 0;
    while x < width {
      if !edge_row && x == band && width > 2 * band {
        x = width - band;
      }
      let distance = rounded_box(x as f64 + 0.5 - width as f64 / 2.0, y as f64 + 0.5 - height as f64 / 2.0, half, radius).abs();
      bitmap.over(x, y, FRAME_COLOR, 0.5 * soft(distance - line, glow));
      bitmap.over(x, y, (0.0, 0.0, 0.0), 0.28 * cover(distance - line - FRAME_RIM * scale));
      bitmap.over(x, y, FRAME_COLOR, 0.95 * cover(distance - line));
      x += 1;
    }
  }
  bitmap
}

/// Distance from a point to a closed polygon (negative inside).
fn polygon(x: f64, y: f64, points: &[(f64, f64)]) -> f64 {
  let mut nearest = f64::MAX;
  let mut inside = false;
  for (index, a) in points.iter().enumerate() {
    let b = points[(index + 1) % points.len()];
    let (ex, ey) = (b.0 - a.0, b.1 - a.1);
    let (px, py) = (x - a.0, y - a.1);
    let t = ((px * ex + py * ey) / (ex * ex + ey * ey)).clamp(0.0, 1.0);
    nearest = nearest.min(((px - ex * t).powi(2) + (py - ey * t).powi(2)).sqrt());
    if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) / ey * ex {
      inside = !inside;
    }
  }
  if inside { -nearest } else { nearest }
}

/// The agent's pointer. `ripple` is how far a click's ring has grown, 0 to 1; `None` without a click.
pub fn pointer(scale: f64, ripple: Option<f64>) -> Bitmap {
  let size = ((POINTER_SIZE.0 * scale).ceil() as usize, (POINTER_SIZE.1 * scale).ceil() as usize);
  let mut bitmap = Bitmap::new(size.0, size.1);
  let line = POINTER_LINE / 2.0;
  for y in 0..size.1 {
    for x in 0..size.0 {
      // In points, from the tip.
      let (px, py) = ((x as f64 + 0.5) / scale - POINTER_TIP.0, (y as f64 + 0.5) / scale - POINTER_TIP.1);
      // The ring first: it spreads out from under the tip, and the pointer covers it.
      if let Some(progress) = ripple {
        // The ring grows from half its size to a bit over twice, and fades as it goes.
        let grown = 0.5 + 1.7 * progress.clamp(0.0, 1.0);
        let ring = ((px * px + py * py).sqrt() - RIPPLE_RADIUS * grown).abs() - grown;
        let fade = 0.9 * (1.0 - progress.clamp(0.0, 1.0));
        // A shade under the white, so the ring reads on a light window too.
        bitmap.over(x, y, (0.0, 0.0, 0.0), 0.5 * fade * soft(ring.max(0.0), 2.5));
        bitmap.over(x, y, POINTER_EDGE, 0.95 * fade * cover(ring * scale));
      }
      // The shadow falls a little below the arrow.
      let shadow = polygon(px, py - 2.0, &ARROW) - line;
      bitmap.over(x, y, (0.0, 0.0, 0.0), 0.45 * soft(shadow.max(0.0), 6.0));
      let distance = polygon(px, py, &ARROW);
      bitmap.over(x, y, POINTER_FILL, cover(distance * scale));
      bitmap.over(x, y, POINTER_EDGE, cover((distance.abs() - line) * scale));
    }
  }
  bitmap
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parts(pixel: u32) -> (u32, u32, u32, u32) {
    (pixel >> 24, (pixel >> 16) & 0xFF, (pixel >> 8) & 0xFF, pixel & 0xFF)
  }

  #[test]
  fn the_frame_is_a_white_line_around_an_empty_middle() {
    let bitmap = frame((400, 300), 10.0, 1.0);
    assert_eq!((bitmap.width, bitmap.height), (432, 332));
    let at = |x: usize, y: usize| parts(bitmap.pixels[y * bitmap.width + x]);
    // The line runs two points outside the window's edge, which is 16 pixels in.
    let (a, r, g, b) = at(216, 14);
    assert!(a > 230 && r > 220 && g > 220 && b > 220, "line pixel {:?}", (a, r, g, b));
    // Just outside it, the dark rim: opaque enough to show, darker than the line.
    let (a, r, _, _) = at(216, 12);
    assert!(a > 60 && r < 200, "rim pixel {:?}", (a, r));
    assert_eq!(at(216, 166), (0, 0, 0, 0));
    assert_eq!(at(60, 166), (0, 0, 0, 0));
    assert_eq!(at(0, 0), (0, 0, 0, 0));
    // Mirrored: the left edge matches the top.
    assert!(at(14, 166).0 > 230);
  }

  #[test]
  fn the_pointer_is_black_with_a_white_edge() {
    let bitmap = pointer(2.0, None);
    assert_eq!((bitmap.width, bitmap.height), (120, 120));
    let at = |x: f64, y: f64| parts(bitmap.pixels[((POINTER_TIP.1 + y) * 2.0) as usize * bitmap.width + ((POINTER_TIP.0 + x) * 2.0) as usize]);
    // Well inside the arrow: opaque and near black.
    let (a, r, g, b) = at(3.0, 10.0);
    assert!(a == 255 && r < 30 && g < 30 && b < 30, "fill pixel {:?}", (a, r, g, b));
    // On its left edge: white.
    let (a, r, _, _) = at(0.0, 10.0);
    assert!(a > 240 && r > 200, "edge pixel {:?}", (a, r));
    // Far from it: nothing.
    assert_eq!(at(-20.0, -20.0), (0, 0, 0, 0));
    // A click's ring appears around the tip and is gone at the end.
    let clicked = pointer(2.0, Some(0.5));
    let ring = RIPPLE_RADIUS * (0.5 + 1.7 * 0.5);
    let pixel = clicked.pixels[(POINTER_TIP.1 * 2.0) as usize * clicked.width + ((POINTER_TIP.0 - ring) * 2.0) as usize];
    assert!(parts(pixel).0 > 80, "ring pixel {:?}", parts(pixel));
    assert_eq!(pointer(2.0, Some(1.0)).pixels[(POINTER_TIP.1 * 2.0) as usize * clicked.width + ((POINTER_TIP.0 - 20.0) * 2.0) as usize], 0);
  }

  /// Writes the bitmaps as raw BGRA files for a look: `CU_PAINT_DUMP=<dir> cargo test -p acpira-computer paint`.
  #[test]
  fn dump() {
    let Ok(dir) = std::env::var("CU_PAINT_DUMP") else { return };
    for (name, bitmap) in [("frame", frame((640, 400), 16.0, 2.0)), ("pointer", pointer(2.0, None)), ("ripple", pointer(2.0, Some(0.4)))] {
      let bytes: Vec<u8> = bitmap.pixels.iter().flat_map(|pixel| pixel.to_le_bytes()).collect();
      std::fs::write(format!("{dir}/{name}-{}x{}.bgra", bitmap.width, bitmap.height), bytes).unwrap();
    }
  }
}
