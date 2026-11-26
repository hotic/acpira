//! A picture of one window, for what the accessibility tree does not describe (canvases, images, custom views).
//!
//! The picture is scaled to at most one pixel per screen point and to a longest side a model reads without waste.

/// Longest side of a picture, in pixels.
pub const MAX_SIDE: f64 = 1568.0;
/// JPEG quality, 0 to 1.
pub const QUALITY: f64 = 0.62;

pub struct Shot {
  pub jpeg: Vec<u8>,
  pub width: u32,
  pub height: u32,
  /// Picture pixels per window point: a point in the picture divided by this is a point in the window.
  pub scale: f64,
}

/// The picture's size for a window of `points` whose surface is `pixels`, and the scale between them.
pub fn fit(points: (f64, f64), pixels: (usize, usize)) -> (u32, u32, f64) {
  let longest = points.0.max(points.1).max(1.0);
  let scale = (MAX_SIDE / longest).min(1.0);
  // Never larger than the surface itself (a window on a display that is not high-density).
  let scale = scale.min(pixels.0 as f64 / points.0.max(1.0)).min(pixels.1 as f64 / points.1.max(1.0));
  ((points.0 * scale).round().max(1.0) as u32, (points.1 * scale).round().max(1.0) as u32, scale)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pictures_fit() {
    // A high-density window smaller than the limit: one pixel per point.
    assert_eq!(fit((800.0, 600.0), (1600, 1200)), (800, 600, 1.0));
    // A large window is scaled down to the limit.
    let (width, height, scale) = fit((3136.0, 1000.0), (6272, 2000));
    assert_eq!((width, height), (1568, 500));
    assert!((scale - 0.5).abs() < 1e-9);
    // A surface with fewer pixels than points is never scaled up.
    assert_eq!(fit((800.0, 600.0), (400, 300)), (400, 300, 0.5));
  }
}
