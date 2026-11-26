//! How the agent's pointers move, the same on every system. A move follows a cubic curve bowed slightly to one side,
//! with a hand's speed: a quick start and a long settle into the target. A step of a batch that is not its last is
//! shown by a clone that flies in from outside the window, acts, backs off and fades, while the agent's own pointer
//! waits for the last step.
//!
//! Points are screen coordinates with y growing downwards; lengths are in points times `scale` (pixels on Windows).

use crate::tree::Rect;

/// How long a clone stays on its target after it lands, then how long it takes to back off and fade.
pub const CLONE_HOLD: f64 = 0.22;
pub const CLONE_LEAVE: f64 = 0.26;
/// A clone fades in over this long as it sets off, so it never pops up in plain sight.
pub const CLONE_APPEAR: f64 = 0.12;
/// How far outside the window a clone sets off, in points.
const CLONE_START: f64 = 120.0;
/// How far a clone backs off the way it came before it is gone, in points.
const CLONE_BACK: f64 = 18.0;

/// One move: where from, the curve's two control points, where to, and how long it takes.
#[derive(Clone, Copy, Debug)]
pub struct Arc {
  from: (f64, f64),
  c1: (f64, f64),
  c2: (f64, f64),
  to: (f64, f64),
  pub seconds: f64,
}

impl Arc {
  pub fn new(from: (f64, f64), to: (f64, f64), scale: f64) -> Arc {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx.hypot(dy);
    let points = length / scale.max(0.1);
    // Bowed to the same side every time, bending most early and straightening into the target.
    let bow = (points * 0.16).min(70.0) * scale;
    let (nx, ny) = if length > 0.0 { (dy / length, -dx / length) } else { (0.0, 0.0) };
    let c1 = (from.0 + dx * 0.28 + nx * bow, from.1 + dy * 0.28 + ny * bow);
    let c2 = (from.0 + dx * 0.78 + nx * bow * 0.55, from.1 + dy * 0.78 + ny * bow * 0.55);
    // Longer moves take longer, within limits that keep a short hop visible and a long one brisk.
    let seconds = ((240.0 + points * 0.55) / 1000.0).clamp(0.26, 0.56);
    Arc { from, c1, c2, to, seconds }
  }

  /// Where the move is after `fraction` of its time, 0 to 1.
  pub fn at(&self, fraction: f64) -> (f64, f64) {
    let e = ease_hand(fraction.clamp(0.0, 1.0));
    let u = 1.0 - e;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * e, 3.0 * u * e * e, e * e * e);
    (a * self.from.0 + b * self.c1.0 + c * self.c2.0 + d * self.to.0, a * self.from.1 + b * self.c1.1 + c * self.c2.1 + d * self.to.1)
  }
}

/// A hand's speed over a move: the timing curve `cubic-bezier(0.32, 0, 0.16, 1)`.
fn ease_hand(x: f64) -> f64 {
  const P1: (f64, f64) = (0.32, 0.0);
  const P2: (f64, f64) = (0.16, 1.0);
  let curve = |s: f64, a: f64, b: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * a + 3.0 * (1.0 - s) * s * s * b + s * s * s;
  // The curve's x grows with its parameter, so halving finds the parameter for `x`.
  let (mut low, mut high) = (0.0, 1.0);
  for _ in 0..40 {
    let middle = (low + high) / 2.0;
    if curve(middle, P1.0, P2.0) < x {
      low = middle;
    } else {
      high = middle;
    }
  }
  curve((low + high) / 2.0, P1.1, P2.1)
}

/// Where the `index`th clone of a batch sets off: outside the window `frame`, level with its target, taking the
/// window's sides in turn so a batch comes in from all around.
pub fn entry(frame: Rect, at: (f64, f64), index: usize, scale: f64) -> (f64, f64) {
  let out = CLONE_START * scale;
  match index % 4 {
    0 => (at.0, frame.y - out),
    1 => (frame.x + frame.w + out, at.1),
    2 => (frame.x - out, at.1),
    _ => (at.0, frame.y + frame.h + out),
  }
}

/// Where a clone that came from `entry` ends up once it has backed off its target `at`.
pub fn retreat(entry: (f64, f64), at: (f64, f64), scale: f64) -> (f64, f64) {
  let (dx, dy) = (entry.0 - at.0, entry.1 - at.1);
  let length = dx.hypot(dy);
  if length <= 0.0 {
    return at;
  }
  let back = CLONE_BACK * scale;
  (at.0 + dx / length * back, at.1 + dy / length * back)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_move_starts_and_ends_where_it_is_asked_and_bows_to_one_side() {
    let arc = Arc::new((100.0, 100.0), (400.0, 100.0), 1.0);
    assert_eq!(arc.at(0.0), (100.0, 100.0));
    let end = arc.at(1.0);
    assert!((end.0 - 400.0).abs() < 1e-6 && (end.1 - 100.0).abs() < 1e-6, "{end:?}");
    // Moving right on a y-down screen, the curve passes above the straight line.
    assert!(arc.at(0.4).1 < 99.0);
    // Time grows with distance, within its limits.
    assert!((Arc::new((0.0, 0.0), (10.0, 0.0), 1.0).seconds - 0.26).abs() < 1e-9);
    assert!((Arc::new((0.0, 0.0), (3000.0, 0.0), 1.0).seconds - 0.56).abs() < 1e-9);
    // The same move on a screen of twice the pixels takes the same time.
    assert!((Arc::new((0.0, 0.0), (300.0, 0.0), 1.0).seconds - Arc::new((0.0, 0.0), (600.0, 0.0), 2.0).seconds).abs() < 1e-9);
  }

  #[test]
  fn a_hand_starts_quick_and_settles_slowly() {
    assert!(ease_hand(0.0).abs() < 1e-6 && (ease_hand(1.0) - 1.0).abs() < 1e-6);
    let mut last = 0.0;
    for step in 1..=20 {
      let value = ease_hand(f64::from(step) / 20.0);
      assert!(value >= last, "not monotonic at {step}");
      last = value;
    }
    // Well past half the way at half the time.
    assert!(ease_hand(0.5) > 0.75);
  }

  #[test]
  fn clones_come_from_outside_the_window_on_every_side_and_back_off_the_way_they_came() {
    let frame = Rect { x: 100.0, y: 100.0, w: 400.0, h: 300.0 };
    let at = (300.0, 250.0);
    let starts: Vec<(f64, f64)> = (0..4).map(|index| entry(frame, at, index, 1.0)).collect();
    assert_eq!(starts, [(300.0, -20.0), (620.0, 250.0), (-20.0, 250.0), (300.0, 520.0)]);
    assert_eq!(retreat(starts[0], at, 1.0), (300.0, 232.0));
    assert_eq!(retreat(at, at, 1.0), at);
  }
}
