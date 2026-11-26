//! What the user sees while the agent works: a white frame around the window being driven, the agent's own pointer,
//! and the clones that show a batch's steps (`crate::motion`).
//!
//! One transparent, click-through window covers the target's screen and is ordered directly above the target window,
//! not above everything: whatever the user has in front of the target also covers the frame and the pointers, so the
//! overlay never sits on top of their work. Drawing is Core Animation layers: the frame, click rings, then pointers.
//! Moves are fire-and-forget: an action never waits for a pointer. The overlay fades when the agent has been idle for
//! a while.

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

use crate::motion::{self, Arc as Move};
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Point {
  pub x: f64,
  pub y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Size {
  pub w: f64,
  pub h: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Rect {
  pub origin: Point,
  pub size: Size,
}

// SAFETY: layout and encoding match CGPoint on 64-bit macOS.
unsafe impl Encode for Point {
  const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
// SAFETY: layout and encoding match CGSize on 64-bit macOS.
unsafe impl Encode for Size {
  const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
// SAFETY: layout and encoding match CGRect on 64-bit macOS.
unsafe impl Encode for Rect {
  const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
}

/// Opaque Core Graphics objects, typed so message sends carry the encoding Core Animation's setters declare.
#[repr(C)]
pub struct CGColor {
  _opaque: [u8; 0],
}
#[repr(C)]
pub struct CGPath {
  _opaque: [u8; 0],
}
// SAFETY: the encoding of CGColorRef.
unsafe impl RefEncode for CGColor {
  const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[]));
}
// SAFETY: the encoding of CGPathRef.
unsafe impl RefEncode for CGPath {
  const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[]));
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
  static kCAMediaTimingFunctionEaseOut: *const AnyObject;
  static kCAMediaTimingFunctionEaseIn: *const AnyObject;
  static kCAFillModeForwards: *const AnyObject;
  fn CACurrentMediaTime() -> f64;
  fn CGColorCreateGenericRGB(r: f64, g: f64, b: f64, a: f64) -> *mut CGColor;
  fn CGColorRelease(color: *mut CGColor);
  fn CGPathCreateWithRoundedRect(rect: Rect, rx: f64, ry: f64, transform: *const c_void) -> *mut CGPath;
  fn CGPathCreateWithEllipseInRect(rect: Rect, transform: *const c_void) -> *mut CGPath;
  fn CGPathCreateMutable() -> *mut CGPath;
  fn CGPathMoveToPoint(path: *mut CGPath, transform: *const c_void, x: f64, y: f64);
  fn CGPathAddLineToPoint(path: *mut CGPath, transform: *const c_void, x: f64, y: f64);
  fn CGPathCloseSubpath(path: *mut CGPath);
  fn CGPathRelease(path: *mut CGPath);
  fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
  fn dispatch_after_f(when: u64, queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
  fn dispatch_time(when: u64, delta: i64) -> u64;
  static _dispatch_main_q: c_void;
}

/// The frame is white with a soft white glow, over a thin dark edge that keeps it visible on a light desktop.
const FRAME: (f64, f64, f64) = (1.0, 1.0, 1.0);
/// The agent's pointer is the system's arrow in black and white, a size up so it is not taken for the user's own.
const POINTER_FILL: (f64, f64, f64) = (0.06, 0.06, 0.07);
const POINTER_EDGE: (f64, f64, f64) = (1.0, 1.0, 1.0);
/// The arrow, tip first, in a y-up space with the tip at the origin.
const ARROW: [(f64, f64); 7] = [(0.0, 0.0), (0.0, -21.0), (4.9, -16.3), (8.9, -25.2), (12.4, -23.7), (8.5, -15.0), (15.0, -15.0)];
/// Keyframes along a move's curve; Core Animation draws straight between them.
const MOVE_FRAMES: usize = 40;
/// The frame sits this far outside the window edge.
const FRAME_OUTSET: f64 = 2.0;
/// Seconds without a command before the overlay fades.
const IDLE: f64 = 6.0;
const FADE: f64 = 0.3;

pub enum Command {
  /// Frame the window `window_id` whose bounds are `rect` (screen points, top-left origin of the primary display),
  /// with the window's corner radius.
  Frame { window_id: u32, rect: Rect, radius: f64 },
  /// Move the pointer to a screen point; `click` adds a ripple there.
  Pointer { at: Point, click: bool },
  /// Show one step of a batch with a clone that flies in to a screen point and leaves; `click` adds a ripple.
  Clone { at: Point, click: bool },
  Hide,
}

struct Overlay {
  window: *mut AnyObject,
  frame: *mut AnyObject,
  /// The dark line under the frame's white one.
  edge: *mut AnyObject,
  pointer: *mut AnyObject,
  root: *mut AnyObject,
  /// The arrow's outline, shared by the pointer and every clone; kept for the life of the process.
  arrow: *mut CGPath,
  /// The framed window's bounds (screen points, top-left origin), where clones set off around.
  target: Option<crate::tree::Rect>,
  /// Clones sent so far, which picks the side the next one comes from.
  clones: usize,
  /// The overlay window's frame, in AppKit's space (bottom-left origin of the primary display).
  bounds: Rect,
  /// Height of the primary display, which both coordinate spaces are anchored to.
  primary_height: f64,
  /// The pointer has been placed once: before that it appears where it is sent instead of gliding from a corner.
  placed: bool,
  shown: bool,
}

thread_local! {
  static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

/// Counts commands, so a delayed fade or removal knows whether it is still the latest thing asked for.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Sends a command to the main thread. Callable from any thread.
pub fn send(command: Command) {
  extern "C" fn run(context: *mut c_void) {
    // SAFETY: `context` is the box leaked below, delivered exactly once.
    let command = unsafe { Box::from_raw(context.cast::<Command>()) };
    OVERLAY.with(|slot| {
      if let Some(overlay) = slot.borrow_mut().as_mut() {
        overlay.apply(*command);
      }
    });
  }
  let context = Box::into_raw(Box::new(command)).cast();
  // SAFETY: the main queue is a process-lifetime object; `run` takes ownership of `context`.
  unsafe { dispatch_async_f(&raw const _dispatch_main_q, context, run) };
}

/// Runs `work` on the main thread after `seconds`, handing it the generation current now.
fn later(seconds: f64, work: extern "C" fn(*mut c_void)) {
  let generation = GENERATION.load(Ordering::Acquire) as usize;
  // SAFETY: the main queue is a process-lifetime object; the context is a plain number, never dereferenced.
  unsafe { dispatch_after_f(dispatch_time(0, (seconds * 1e9) as i64), &raw const _dispatch_main_q, generation as *mut c_void, work) };
}

extern "C" fn fade_when_idle(context: *mut c_void) {
  if context as usize as u64 == GENERATION.load(Ordering::Acquire) {
    OVERLAY.with(|slot| {
      if let Some(overlay) = slot.borrow_mut().as_mut() {
        overlay.hide();
      }
    });
  }
}

extern "C" fn remove_when_faded(context: *mut c_void) {
  if context as usize as u64 != GENERATION.load(Ordering::Acquire) {
    return;
  }
  OVERLAY.with(|slot| {
    if let Some(overlay) = slot.borrow_mut().as_mut()
      && overlay.shown
    {
      // SAFETY: main thread; the window lives for the life of the process.
      unsafe {
        let _: () = msg_send![overlay.window, orderOut: std::ptr::null_mut::<AnyObject>()];
      }
      overlay.shown = false;
    }
  });
}

/// # Safety
/// Plain Core Graphics create call; the caller releases the colour.
unsafe fn color(alpha: f64) -> *mut CGColor {
  // SAFETY: no pointers go in.
  unsafe { CGColorCreateGenericRGB(FRAME.0, FRAME.1, FRAME.2, alpha) }
}

/// # Safety
/// Must run on a thread with an autorelease pool; the string is autoreleased.
unsafe fn ns_string(text: &std::ffi::CStr) -> *mut AnyObject {
  // SAFETY: a valid C string for the call.
  unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

/// Builds the overlay and runs the app loop. Must be the main thread; never returns.
pub fn run_main() -> ! {
  // SAFETY: plain AppKit / Core Animation calls on the main thread, with argument and return types matching the
  // Objective-C signatures.
  unsafe {
    let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
    // Accessory: no Dock icon, no menu bar, never becomes the active app.
    let _: bool = msg_send![app, setActivationPolicy: 1isize];

    let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
    let count: usize = msg_send![screens, count];
    let bounds: Rect = if count == 0 {
      Rect { origin: Point::default(), size: Size { w: 1440.0, h: 900.0 } }
    } else {
      let primary: *mut AnyObject = msg_send![screens, objectAtIndex: 0usize];
      msg_send![primary, frame]
    };
    let window: *mut AnyObject = msg_send![class!(NSWindow), alloc];
    // Style 0 = borderless, backing 2 = buffered.
    let window: *mut AnyObject = msg_send![window, initWithContentRect: bounds, styleMask: 0usize, backing: 2usize, defer: false];
    let clear: *mut AnyObject = msg_send![class!(NSColor), clearColor];
    let _: () = msg_send![window, setBackgroundColor: clear];
    let _: () = msg_send![window, setOpaque: false];
    let _: () = msg_send![window, setHasShadow: false];
    let _: () = msg_send![window, setIgnoresMouseEvents: true];
    let _: () = msg_send![window, setReleasedWhenClosed: false];
    // Stationary + ignores-cycle + full-screen auxiliary: not in Mission Control or the window cycle.
    let _: () = msg_send![window, setCollectionBehavior: (1usize << 4) | (1usize << 6) | (1usize << 8)];

    let view: *mut AnyObject = msg_send![window, contentView];
    let _: () = msg_send![view, setWantsLayer: true];
    let root: *mut AnyObject = msg_send![view, layer];

    // A slightly wider dark line under the white one shows as a thin rim on both sides of it.
    let edge: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
    let rim = CGColorCreateGenericRGB(0.0, 0.0, 0.0, 0.28);
    let _: () = msg_send![edge, setFillColor: std::ptr::null_mut::<CGColor>()];
    let _: () = msg_send![edge, setStrokeColor: rim];
    let _: () = msg_send![edge, setLineWidth: 3.5f64];
    let _: () = msg_send![edge, setOpacity: 0.0f32];
    let _: () = msg_send![root, addSublayer: edge];
    let frame: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
    let stroke = color(0.95);
    let glow = color(1.0);
    let _: () = msg_send![frame, setFillColor: std::ptr::null_mut::<CGColor>()];
    let _: () = msg_send![frame, setStrokeColor: stroke];
    let _: () = msg_send![frame, setLineWidth: 2.5f64];
    let _: () = msg_send![frame, setShadowColor: glow];
    let _: () = msg_send![frame, setShadowOpacity: 0.5f32];
    let _: () = msg_send![frame, setShadowRadius: 7.0f64];
    let _: () = msg_send![frame, setShadowOffset: Size::default()];
    let _: () = msg_send![frame, setOpacity: 0.0f32];
    let _: () = msg_send![root, addSublayer: frame];

    // An arrow with its tip at the layer's origin, drawn in a y-up space.
    let arrow = CGPathCreateMutable();
    for (index, (x, y)) in ARROW.into_iter().enumerate() {
      if index == 0 {
        CGPathMoveToPoint(arrow, std::ptr::null(), x, y);
      } else {
        CGPathAddLineToPoint(arrow, std::ptr::null(), x, y);
      }
    }
    CGPathCloseSubpath(arrow);
    let pointer = arrow_layer(arrow);
    let _: () = msg_send![root, addSublayer: pointer];
    for made in [rim, stroke, glow] {
      CGColorRelease(made);
    }

    OVERLAY.with(|slot| {
      *slot.borrow_mut() = Some(Overlay { window, frame, edge, pointer, root, arrow, target: None, clones: 0, bounds, primary_height: bounds.size.h, placed: false, shown: false });
    });
    let _: () = msg_send![app, run];
  }
  std::process::exit(0)
}

/// The screen frame (AppKit space) holding `center`, or `None` when no screen does.
///
/// # Safety
/// Main thread only.
unsafe fn screen_holding(center: Point) -> Option<Rect> {
  // SAFETY: AppKit calls with their declared signatures, on the main thread.
  unsafe {
    let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
    let count: usize = msg_send![screens, count];
    for index in 0..count {
      let screen: *mut AnyObject = msg_send![screens, objectAtIndex: index];
      let frame: Rect = msg_send![screen, frame];
      let inside_x = center.x >= frame.origin.x && center.x < frame.origin.x + frame.size.w;
      let inside_y = center.y >= frame.origin.y && center.y < frame.origin.y + frame.size.h;
      if inside_x && inside_y {
        return Some(frame);
      }
    }
  }
  None
}

impl Overlay {
  /// A screen point (top-left origin of the primary display) in the overlay's layer space.
  fn local(&self, at: Point) -> Point {
    Point { x: at.x - self.bounds.origin.x, y: (self.primary_height - at.y) - self.bounds.origin.y }
  }

  /// A point in the overlay's layer space as a screen point (top-left origin of the primary display).
  fn screen(&self, local: Point) -> Point {
    Point { x: local.x + self.bounds.origin.x, y: self.primary_height - (local.y + self.bounds.origin.y) }
  }

  fn apply(&mut self, command: Command) {
    GENERATION.fetch_add(1, Ordering::AcqRel);
    // SAFETY: main thread; the layers and window live for the life of the process.
    unsafe {
      match command {
        Command::Frame { window_id, rect, radius } => {
          // The overlay covers the screen the target is on.
          let center = Point { x: rect.origin.x + rect.size.w / 2.0, y: self.primary_height - rect.origin.y - rect.size.h / 2.0 };
          if let Some(screen) = screen_holding(center)
            && screen != self.bounds
          {
            let _: () = msg_send![self.window, setFrame: screen, display: false];
            self.bounds = screen;
            self.placed = false;
          }
          let corner = self.local(Point { x: rect.origin.x, y: rect.origin.y + rect.size.h });
          let outer = Rect {
            origin: Point { x: corner.x - FRAME_OUTSET, y: corner.y - FRAME_OUTSET },
            size: Size { w: rect.size.w + 2.0 * FRAME_OUTSET, h: rect.size.h + 2.0 * FRAME_OUTSET },
          };
          let radius = (radius + FRAME_OUTSET).min(outer.size.w / 2.0).min(outer.size.h / 2.0).max(0.0);
          let path = CGPathCreateWithRoundedRect(outer, radius, radius, std::ptr::null());
          let _: () = msg_send![class!(CATransaction), begin];
          let _: () = msg_send![class!(CATransaction), setAnimationDuration: 0.18f64];
          for layer in [self.edge, self.frame] {
            let _: () = msg_send![layer, setPath: path];
            let _: () = msg_send![layer, setOpacity: 1.0f32];
          }
          let _: () = msg_send![class!(CATransaction), commit];
          CGPathRelease(path);
          self.target = Some(crate::tree::Rect { x: rect.origin.x, y: rect.origin.y, w: rect.size.w, h: rect.size.h });
          // 1 = NSWindowAbove: directly over the target, under whatever covers it.
          let _: () = msg_send![self.window, orderWindow: 1isize, relativeTo: window_id as isize];
          self.shown = true;
          later(IDLE, fade_when_idle);
        }
        Command::Pointer { at, click } => {
          let to = self.local(at);
          // From where the pointer is on screen right now, which is mid-flight when the last move has not ended.
          let shown: *mut AnyObject = msg_send![self.pointer, presentationLayer];
          let from: Point = if shown.is_null() { msg_send![self.pointer, position] } else { msg_send![shown, position] };
          let travel = self.placed.then(|| {
            let from = self.screen(from);
            Move::new((from.x, from.y), (at.x, at.y), 1.0)
          });
          let _: () = msg_send![class!(CATransaction), begin];
          let _: () = msg_send![class!(CATransaction), setDisableActions: true];
          let _: () = msg_send![self.pointer, setPosition: to];
          let _: () = msg_send![self.pointer, setOpacity: 1.0f32];
          let _: () = msg_send![class!(CATransaction), commit];
          if let Some(travel) = &travel {
            let _: () = msg_send![self.pointer, addAnimation: self.along(travel), forKey: ns_string(c"move")];
          }
          self.placed = true;
          if click {
            self.ripple(to, travel.map_or(0.0, |travel| travel.seconds));
          }
          later(IDLE, fade_when_idle);
        }
        Command::Clone { at, click } => {
          self.clone_to(at, click);
          later(IDLE, fade_when_idle);
        }
        Command::Hide => self.hide(),
      }
    }
  }

  fn hide(&mut self) {
    // SAFETY: main thread; the layers live for the life of the process.
    unsafe {
      let _: () = msg_send![class!(CATransaction), begin];
      let _: () = msg_send![class!(CATransaction), setAnimationDuration: FADE];
      for layer in [self.edge, self.frame, self.pointer] {
        let _: () = msg_send![layer, setOpacity: 0.0f32];
      }
      let _: () = msg_send![class!(CATransaction), commit];
    }
    // The next appearance starts where it is sent, not from where the pointer faded.
    self.placed = false;
    later(FADE + 0.05, remove_when_faded);
  }

  /// A move as a position animation: keyframes along its curve, evenly spaced in time, so the curve's speed is the
  /// move's own and matches every other system's.
  ///
  /// # Safety
  /// Main thread only; the animation is autoreleased.
  unsafe fn along(&self, travel: &Move) -> *mut AnyObject {
    // SAFETY: Core Animation and Foundation calls with their declared signatures, on the main thread.
    unsafe {
      let values: Vec<*mut AnyObject> = (0..=MOVE_FRAMES)
        .map(|index| {
          let (x, y) = travel.at(index as f64 / MOVE_FRAMES as f64);
          value(self.local(Point { x, y }))
        })
        .collect();
      let values: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: values.as_ptr(), count: values.len()];
      let animation: *mut AnyObject = msg_send![class!(CAKeyframeAnimation), animationWithKeyPath: ns_string(c"position")];
      let _: () = msg_send![animation, setValues: values];
      let _: () = msg_send![animation, setDuration: travel.seconds];
      animation
    }
  }

  /// A clone of the pointer that flies in from outside the window to `at`, stays a moment and backs off as it fades;
  /// it removes itself. Without a framed window yet, the agent's own pointer goes instead.
  ///
  /// # Safety
  /// Main thread only.
  unsafe fn clone_to(&mut self, at: Point, click: bool) {
    let Some(frame) = self.target else {
      self.apply(Command::Pointer { at, click });
      return;
    };
    let start = motion::entry(frame, (at.x, at.y), self.clones, 1.0);
    self.clones += 1;
    let travel = Move::new(start, (at.x, at.y), 1.0);
    let back = motion::retreat(start, (at.x, at.y), 1.0);
    let (to, back) = (self.local(at), self.local(Point { x: back.0, y: back.1 }));
    let leave = travel.seconds + motion::CLONE_HOLD;
    let whole = leave + motion::CLONE_LEAVE;
    // SAFETY: Core Animation calls with their declared signatures, on the main thread.
    unsafe {
      let clone = arrow_layer(self.arrow);
      // Where it ends, invisible: what is left once the animations are over.
      let _: () = msg_send![clone, setPosition: back];
      // Above the agent's own pointer, and above every click ring, which goes under the pointer.
      let _: () = msg_send![self.root, addSublayer: clone];
      let arrive = self.along(&travel);
      let appear = basic(c"opacity", 0.0, 1.0);
      let _: () = msg_send![appear, setDuration: motion::CLONE_APPEAR];
      let away: *mut AnyObject = msg_send![class!(CABasicAnimation), animationWithKeyPath: ns_string(c"position")];
      let _: () = msg_send![away, setFromValue: value(to)];
      let _: () = msg_send![away, setToValue: value(back)];
      let fade = basic(c"opacity", 1.0, 0.0);
      let ease_in: *mut AnyObject = msg_send![class!(CAMediaTimingFunction), functionWithName: kCAMediaTimingFunctionEaseIn];
      let ease_out: *mut AnyObject = msg_send![class!(CAMediaTimingFunction), functionWithName: kCAMediaTimingFunctionEaseOut];
      let _: () = msg_send![fade, setTimingFunction: ease_in];
      let _: () = msg_send![away, setTimingFunction: ease_out];
      for leaving in [away, fade] {
        let _: () = msg_send![leaving, setBeginTime: leave];
        let _: () = msg_send![leaving, setDuration: motion::CLONE_LEAVE];
      }
      // Each part holds its last value until the next one starts, and the whole holds the end until removal.
      for part in [arrive, appear, away, fade] {
        let _: () = msg_send![part, setFillMode: kCAFillModeForwards];
      }
      let parts: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: [arrive, appear, away, fade].as_ptr(), count: 4usize];
      let group: *mut AnyObject = msg_send![class!(CAAnimationGroup), animation];
      let _: () = msg_send![group, setAnimations: parts];
      let _: () = msg_send![group, setDuration: whole];
      let _: () = msg_send![group, setFillMode: kCAFillModeForwards];
      let _: () = msg_send![group, setRemovedOnCompletion: false];
      let _: () = msg_send![clone, addAnimation: group, forKey: ns_string(c"clone")];
      let _: () = msg_send![clone, performSelector: objc2::sel!(removeFromSuperlayer), withObject: std::ptr::null_mut::<AnyObject>(), afterDelay: whole + 0.05];
      if click {
        self.ripple(to, travel.seconds);
      }
    }
  }

  /// A ring that grows from the click point and fades, starting when the pointer arrives.
  ///
  /// # Safety
  /// Main thread only.
  unsafe fn ripple(&self, at: Point, delay: f64) {
    // SAFETY: Core Animation calls with their declared signatures, on the main thread.
    unsafe {
      let ring: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
      let circle = CGPathCreateWithEllipseInRect(Rect { origin: Point { x: -9.0, y: -9.0 }, size: Size { w: 18.0, h: 18.0 } }, std::ptr::null());
      // The pointer's own white, with a shade under it so it reads on a light window too
      let stroke = CGColorCreateGenericRGB(POINTER_EDGE.0, POINTER_EDGE.1, POINTER_EDGE.2, 0.95);
      let shade = CGColorCreateGenericRGB(0.0, 0.0, 0.0, 1.0);
      let _: () = msg_send![ring, setPath: circle];
      let _: () = msg_send![ring, setFillColor: std::ptr::null_mut::<CGColor>()];
      let _: () = msg_send![ring, setStrokeColor: stroke];
      let _: () = msg_send![ring, setLineWidth: 2.0f64];
      let _: () = msg_send![ring, setShadowColor: shade];
      let _: () = msg_send![ring, setShadowOpacity: 0.5f32];
      let _: () = msg_send![ring, setShadowRadius: 2.0f64];
      let _: () = msg_send![ring, setShadowOffset: Size::default()];
      CGColorRelease(shade);
      let _: () = msg_send![ring, setPosition: at];
      let _: () = msg_send![ring, setOpacity: 0.0f32];
      // Under the pointer: the pointer presses, the ring is what spreads out from under its tip
      let _: () = msg_send![self.root, insertSublayer: ring, below: self.pointer];
      CGPathRelease(circle);
      CGColorRelease(stroke);

      let now: f64 = msg_send![ring, convertTime: CACurrentMediaTime(), fromLayer: std::ptr::null_mut::<AnyObject>()];
      let group: *mut AnyObject = msg_send![class!(CAAnimationGroup), animation];
      let scale = basic(c"transform.scale", 0.5, 2.2);
      let fade = basic(c"opacity", 0.9, 0.0);
      let both: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: [scale, fade].as_ptr(), count: 2usize];
      let _: () = msg_send![group, setAnimations: both];
      let _: () = msg_send![group, setDuration: 0.38f64];
      let _: () = msg_send![group, setBeginTime: now + delay];
      let _: () = msg_send![group, setFillMode: kCAFillModeForwards];
      let _: () = msg_send![group, setRemovedOnCompletion: false];
      let _: () = msg_send![ring, addAnimation: group, forKey: ns_string(c"ripple")];
      // The ring removes itself once its animation is over.
      let _: () = msg_send![ring, performSelector: objc2::sel!(removeFromSuperlayer), withObject: std::ptr::null_mut::<AnyObject>(), afterDelay: delay + 0.45];
    }
  }
}

/// A pointer layer: the arrow in black with a white edge and a soft shadow, its tip at the layer's position, hidden
/// until it is shown.
///
/// # Safety
/// Main thread only; `arrow` must stay alive as long as the layer.
unsafe fn arrow_layer(arrow: *mut CGPath) -> *mut AnyObject {
  // SAFETY: Core Animation calls with their declared signatures; every colour made here is released here.
  unsafe {
    let pointer: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
    let fill = CGColorCreateGenericRGB(POINTER_FILL.0, POINTER_FILL.1, POINTER_FILL.2, 1.0);
    let white = CGColorCreateGenericRGB(POINTER_EDGE.0, POINTER_EDGE.1, POINTER_EDGE.2, 1.0);
    let shade = CGColorCreateGenericRGB(0.0, 0.0, 0.0, 1.0);
    let _: () = msg_send![pointer, setPath: arrow];
    let _: () = msg_send![pointer, setFillColor: fill];
    let _: () = msg_send![pointer, setStrokeColor: white];
    let _: () = msg_send![pointer, setLineWidth: 1.8f64];
    // kCALineJoinRound is the NSString @"round".
    let _: () = msg_send![pointer, setLineJoin: ns_string(c"round")];
    let _: () = msg_send![pointer, setShadowColor: shade];
    let _: () = msg_send![pointer, setShadowOpacity: 0.45f32];
    let _: () = msg_send![pointer, setShadowRadius: 4.0f64];
    let _: () = msg_send![pointer, setShadowOffset: Size { w: 0.0, h: -2.0 }];
    let _: () = msg_send![pointer, setOpacity: 0.0f32];
    for made in [fill, white, shade] {
      CGColorRelease(made);
    }
    pointer
  }
}

/// A point boxed for an animation's values.
///
/// # Safety
/// Must run on a thread with an autorelease pool; the value is autoreleased.
unsafe fn value(point: Point) -> *mut AnyObject {
  // SAFETY: NSPoint is CGPoint on 64-bit macOS.
  unsafe { msg_send![class!(NSValue), valueWithPoint: point] }
}

/// # Safety
/// Main thread only; the animation is autoreleased.
unsafe fn basic(key_path: &std::ffi::CStr, from: f64, to: f64) -> *mut AnyObject {
  // SAFETY: Core Animation calls with their declared signatures.
  unsafe {
    let animation: *mut AnyObject = msg_send![class!(CABasicAnimation), animationWithKeyPath: ns_string(key_path)];
    let from: *mut AnyObject = msg_send![class!(NSNumber), numberWithDouble: from];
    let to: *mut AnyObject = msg_send![class!(NSNumber), numberWithDouble: to];
    let _: () = msg_send![animation, setFromValue: from];
    let _: () = msg_send![animation, setToValue: to];
    animation
  }
}
