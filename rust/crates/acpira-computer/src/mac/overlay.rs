//! What the user sees while the agent works: a blue frame around the window being driven and the agent's own pointer.
//!
//! One transparent, click-through window covers the target's screen and is ordered directly above the target window,
//! not above everything: whatever the user has in front of the target also covers the frame and the pointer, so the
//! overlay never sits on top of their work. Drawing is two Core Animation layers. Moves are fire-and-forget: an action
//! never waits for the pointer. The overlay fades when the agent has been idle for a while.

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
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

/// Blue of the frame.
const BLUE: (f64, f64, f64) = (0.04, 0.52, 1.0);
/// The agent's pointer is the system's arrow in black and white, a size up so it is not taken for the user's own.
const POINTER_FILL: (f64, f64, f64) = (0.06, 0.06, 0.07);
const POINTER_EDGE: (f64, f64, f64) = (1.0, 1.0, 1.0);
/// The arrow, tip first, in a y-up space with the tip at the origin.
const ARROW: [(f64, f64); 7] = [(0.0, 0.0), (0.0, -21.0), (4.9, -16.3), (8.9, -25.2), (12.4, -23.7), (8.5, -15.0), (15.0, -15.0)];
/// Seconds a pointer move takes. It never delays the action it accompanies.
const GLIDE: f64 = 0.16;
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
  Hide,
}

struct Overlay {
  window: *mut AnyObject,
  frame: *mut AnyObject,
  pointer: *mut AnyObject,
  root: *mut AnyObject,
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
  unsafe { CGColorCreateGenericRGB(BLUE.0, BLUE.1, BLUE.2, alpha) }
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

    let frame: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
    let stroke = color(0.95);
    let glow = color(1.0);
    let _: () = msg_send![frame, setFillColor: std::ptr::null_mut::<CGColor>()];
    let _: () = msg_send![frame, setStrokeColor: stroke];
    let _: () = msg_send![frame, setLineWidth: 2.5f64];
    let _: () = msg_send![frame, setShadowColor: glow];
    let _: () = msg_send![frame, setShadowOpacity: 0.55f32];
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
    let _: () = msg_send![root, addSublayer: pointer];
    CGPathRelease(arrow);
    for made in [stroke, glow, fill, white, shade] {
      CGColorRelease(made);
    }

    OVERLAY.with(|slot| {
      *slot.borrow_mut() = Some(Overlay { window, frame, pointer, root, bounds, primary_height: bounds.size.h, placed: false, shown: false });
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
          let _: () = msg_send![self.frame, setPath: path];
          let _: () = msg_send![self.frame, setOpacity: 1.0f32];
          let _: () = msg_send![class!(CATransaction), commit];
          CGPathRelease(path);
          // 1 = NSWindowAbove: directly over the target, under whatever covers it.
          let _: () = msg_send![self.window, orderWindow: 1isize, relativeTo: window_id as isize];
          self.shown = true;
          later(IDLE, fade_when_idle);
        }
        Command::Pointer { at, click } => {
          let to = self.local(at);
          let ease: *mut AnyObject = msg_send![class!(CAMediaTimingFunction), functionWithName: kCAMediaTimingFunctionEaseOut];
          let _: () = msg_send![class!(CATransaction), begin];
          if self.placed {
            let _: () = msg_send![class!(CATransaction), setAnimationDuration: GLIDE];
            let _: () = msg_send![class!(CATransaction), setAnimationTimingFunction: ease];
          } else {
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
          }
          let _: () = msg_send![self.pointer, setPosition: to];
          let _: () = msg_send![class!(CATransaction), commit];
          let _: () = msg_send![self.pointer, setOpacity: 1.0f32];
          let glide = if self.placed { GLIDE } else { 0.0 };
          self.placed = true;
          if click {
            self.ripple(to, glide);
          }
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
      let _: () = msg_send![self.frame, setOpacity: 0.0f32];
      let _: () = msg_send![self.pointer, setOpacity: 0.0f32];
      let _: () = msg_send![class!(CATransaction), commit];
    }
    // The next appearance starts where it is sent, not from where the pointer faded.
    self.placed = false;
    later(FADE + 0.05, remove_when_faded);
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
      let _: () = msg_send![self.root, addSublayer: ring];
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
