//! Touch displays on macOS: a finger moves the window from the page's title-bar regions. AppKit turns a finger that
//! lands and slides at once into scroll events and WKWebView's own touch recognizers take every touch on the page, so
//! the page's mousedown drag (init.js) never starts and no AppKit window drag accepts a touch. A transparent view above
//! the web view covers exactly the regions the page reports (`acpira_touch_regions`) and carries a direct-touch pan that
//! moves the window itself; mouse events that land on it go on to the web view, so the pointer's drag and double click
//! stay the page's. Main thread throughout

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::OnceLock;

use objc2::encode::{Encode, Encoding};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, MessageReceiver, Sel};
use objc2::{msg_send, sel};

use crate::os::Log;

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
  x: f64,
  y: f64,
}

// SAFETY: laid out as CGPoint (two CGFloat, f64 on every Mac this app runs on)
unsafe impl Encode for Point {
  const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
  width: f64,
  height: f64,
}

// SAFETY: laid out as CGSize
unsafe impl Encode for Size {
  const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
  origin: Point,
  size: Size,
}

// SAFETY: laid out as CGRect
unsafe impl Encode for Rect {
  const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
}

/// A region of the page in its CSS pixels: left, top, width, height
pub type Region = [f64; 4];

/// What one overlay knows, boxed behind its ivar
struct State {
  /// The WKWebView under the overlay; not retained, the two share a superview and go away with the window
  web: *mut AnyObject,
  regions: Vec<Region>,
  /// Controls inside the regions: a touch there is the page's
  holes: Vec<Region>,
  /// The page's viewport width in CSS pixels: the overlay's width over it is the page zoom
  page_width: f64,
  /// The window's origin and the finger in screen coordinates when the pan began
  drag: Option<(Point, Point)>,
}

const IVAR: &std::ffi::CStr = c"acpiraTouchState";
// NSTouchTypeMaskDirect
const DIRECT: usize = 1;
// NSWindowStyleMaskFullScreen
const FULL_SCREEN: usize = 1 << 14;
// NSViewWidthSizable | NSViewHeightSizable
const SIZABLE: usize = 2 | 16;

/// The overlay's class, registered on first use; None when AppKit lacks a piece of it
fn class() -> Option<&'static AnyClass> {
  static CLASS: OnceLock<Option<&'static AnyClass>> = OnceLock::new();
  *CLASS.get_or_init(|| {
    let view = AnyClass::get(c"NSView")?;
    AnyClass::get(c"NSPanGestureRecognizer")?;
    let mut b = ClassBuilder::new(c"AcpiraTouchDragView", view)?;
    b.add_ivar::<*mut c_void>(IVAR);
    // SAFETY: each function's signature matches its selector's (AppKit's NSView / NSResponder methods)
    unsafe {
      b.add_method(sel!(isFlipped), flipped as extern "C-unwind" fn(_, _) -> _);
      b.add_method(sel!(hitTest:), hit_test as extern "C-unwind" fn(_, _, _) -> _);
      b.add_method(sel!(mouseDownCanMoveWindow), no as extern "C-unwind" fn(_, _) -> _);
      b.add_method(sel!(acceptsFirstMouse:), accepts_first_mouse as extern "C-unwind" fn(_, _, _) -> _);
      b.add_method(sel!(acpiraPan:), pan as extern "C-unwind" fn(_, _, _));
      b.add_method(sel!(dealloc), dealloc as extern "C-unwind" fn(_, _));
      for s in [
        sel!(mouseDown:),
        sel!(mouseDragged:),
        sel!(mouseUp:),
        sel!(rightMouseDown:),
        sel!(rightMouseDragged:),
        sel!(rightMouseUp:),
        sel!(otherMouseDown:),
        sel!(otherMouseDragged:),
        sel!(otherMouseUp:),
        sel!(scrollWheel:),
        sel!(magnifyWithEvent:),
      ] {
        b.add_method(s, forward as extern "C-unwind" fn(_, _, _));
      }
    }
    Some(b.register())
  })
}

/// The overlay's state, None before it is set or after dealloc
///
/// # Safety
/// `this` is a live instance of the overlay class, on the main thread
unsafe fn state<'a>(this: *mut AnyObject) -> Option<&'a mut State> {
  let ivar = class()?.instance_variable(IVAR)?;
  // SAFETY: the ivar is declared as a pointer on the overlay class and holds null or a leaked Box<State>
  unsafe {
    let p = *ivar.load::<*mut c_void>(&*this);
    (p as *mut State).as_mut()
  }
}

extern "C-unwind" fn flipped(_this: *mut AnyObject, _: Sel) -> Bool {
  // Top-down like the page's CSS pixels
  Bool::YES
}

extern "C-unwind" fn no(_this: *mut AnyObject, _: Sel) -> Bool {
  Bool::NO
}

extern "C-unwind" fn accepts_first_mouse(_this: *mut AnyObject, _: Sel, _event: *mut AnyObject) -> Bool {
  // A press on an inactive window's title bar moves it at once, as a native title bar does
  Bool::YES
}

/// The overlay is hit only inside a region and outside its holes; anywhere else the touch or click falls through to the
/// web view. `p` is in the superview's coordinates
extern "C-unwind" fn hit_test(this: *mut AnyObject, _: Sel, p: Point) -> *mut AnyObject {
  // SAFETY: AppKit calls this on the main thread with `this` a live overlay; superview and bounds are plain NSView calls
  unsafe {
    let Some(st) = state(this) else { return null_mut() };
    if st.regions.is_empty() || st.page_width <= 0.0 {
      return null_mut();
    }
    let sup: *mut AnyObject = msg_send![this, superview];
    let local: Point = msg_send![this, convertPoint: p, fromView: sup];
    let bounds: Rect = msg_send![this, bounds];
    let k = bounds.size.width / st.page_width;
    let (x, y) = (local.x / k, local.y / k);
    let inside = |r: &Region| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3];
    if st.regions.iter().any(inside) && !st.holes.iter().any(inside) { this } else { null_mut() }
  }
}

/// A mouse, trackpad or touch-emulated mouse event on a region: the web view takes it as if the overlay were not there
extern "C-unwind" fn forward(this: *mut AnyObject, cmd: Sel, event: *mut AnyObject) {
  // SAFETY: the web view shares the overlay's window and responds to every NSResponder event selector added above
  unsafe {
    let Some(st) = state(this) else { return };
    if !st.web.is_null() {
      let _: () = st.web.send_message(cmd, (event,));
    }
  }
}

/// The direct-touch pan: the window follows the finger in screen coordinates (the window moves under the finger, so
/// coordinates in the window would chase themselves). Full screen stays put
extern "C-unwind" fn pan(this: *mut AnyObject, _: Sel, g: *mut AnyObject) {
  // SAFETY: AppKit sends the action on the main thread with the overlay's live recognizer; window, frame and
  // setFrameOrigin: are documented NSWindow messages
  unsafe {
    let Some(st) = state(this) else { return };
    let window: *mut AnyObject = msg_send![this, window];
    if window.is_null() {
      return;
    }
    // NSGestureRecognizerState: began 1, changed 2, then ended / cancelled / failed
    let phase: isize = msg_send![g, state];
    let p: Point = msg_send![g, locationInView: null_mut::<AnyObject>()];
    let frame: Rect = msg_send![window, frame];
    let finger = Point { x: frame.origin.x + p.x, y: frame.origin.y + p.y };
    match phase {
      1 => {
        let mask: usize = msg_send![window, styleMask];
        st.drag = (mask & FULL_SCREEN == 0).then_some((frame.origin, finger));
      }
      2 => {
        let Some((origin, start)) = st.drag else { return };
        let mut next = frame;
        next.origin = Point { x: origin.x + finger.x - start.x, y: origin.y + finger.y - start.y };
        // The title bar stays below the menu bar, as in a system drag
        let screen: *mut AnyObject = msg_send![window, screen];
        if !screen.is_null() {
          next = msg_send![window, constrainFrameRect: next, toScreen: screen];
        }
        let _: () = msg_send![window, setFrameOrigin: next.origin];
      }
      _ => st.drag = None,
    }
  }
}

extern "C-unwind" fn dealloc(this: *mut AnyObject, _: Sel) {
  // SAFETY: the ivar holds null or the Box leaked in `install`; NSView's dealloc runs last, as in any subclass
  unsafe {
    if let Some(ivar) = class().and_then(|c| c.instance_variable(IVAR)) {
      let p = ivar.load_mut::<*mut c_void>(&mut *this);
      if !p.is_null() {
        drop(Box::from_raw(*p as *mut State));
        *p = null_mut();
      }
    }
    if let Some(view) = AnyClass::get(c"NSView") {
      let _: () = msg_send![super(&*this, view), dealloc];
    }
  }
}

/// The overlay of a web view, if one is installed
///
/// # Safety
/// `web` is the live WKWebView, on the main thread
unsafe fn overlay_of(web: *mut AnyObject) -> Option<*mut AnyObject> {
  let cls = class()?;
  // SAFETY: superview / subviews are plain NSView calls; the array lives while the superview does
  unsafe {
    let sup: *mut AnyObject = msg_send![web, superview];
    if sup.is_null() {
      return None;
    }
    let views: *mut AnyObject = msg_send![sup, subviews];
    let count: usize = msg_send![views, count];
    (0..count).map(|i| -> *mut AnyObject { msg_send![views, objectAtIndex: i] }).find(|v| {
      let kind: Bool = msg_send![*v, isKindOfClass: cls];
      kind.as_bool()
    })
  }
}

/// Puts the overlay above the window's web view, empty until the page reports its regions
pub fn install(window: &tauri::WebviewWindow, log: Log) {
  // SAFETY: `wv.inner()` is the live WKWebView; the overlay is created, configured and handed to the superview (which
  // retains it) on the main thread, and the recognizer likewise to the overlay
  let _ = window.with_webview(move |wv| unsafe {
    let web = wv.inner() as *mut AnyObject;
    let (Some(cls), Some(pan_cls)) = (class(), AnyClass::get(c"NSPanGestureRecognizer")) else {
      log("touch drag: NSView / NSPanGestureRecognizer missing, a finger cannot move the window");
      return;
    };
    let sup: *mut AnyObject = msg_send![web, superview];
    if sup.is_null() || overlay_of(web).is_some() {
      return;
    }
    let frame: Rect = msg_send![web, frame];
    let view: *mut AnyObject = msg_send![cls, alloc];
    let view: *mut AnyObject = msg_send![view, initWithFrame: frame];
    let st = Box::new(State { web, regions: Vec::new(), holes: Vec::new(), page_width: 0.0, drag: None });
    if let Some(ivar) = cls.instance_variable(IVAR) {
      *ivar.load_mut::<*mut c_void>(&mut *view) = Box::into_raw(st).cast();
    }
    let _: () = msg_send![view, setAutoresizingMask: SIZABLE];
    // NSWindowAbove
    let _: () = msg_send![sup, addSubview: view, positioned: 1isize, relativeTo: web];
    let g: *mut AnyObject = msg_send![pan_cls, alloc];
    let g: *mut AnyObject = msg_send![g, initWithTarget: view, action: sel!(acpiraPan:)];
    // Fingers only: a mouse or trackpad press goes through the forwarded events instead
    let _: () = msg_send![g, setButtonMask: 0usize];
    let _: () = msg_send![g, setAllowedTouchTypes: DIRECT];
    let _: () = msg_send![view, addGestureRecognizer: g];
    let _: () = msg_send![g, release];
    let _: () = msg_send![view, release];
  });
}

/// The page's title-bar regions and the controls inside them, in CSS pixels of a viewport `page_width` wide
pub fn set_regions(window: &tauri::WebviewWindow, page_width: f64, regions: Vec<Region>, holes: Vec<Region>) {
  // SAFETY: `wv.inner()` is the live WKWebView and the overlay a live subview of its superview, on the main thread
  let _ = window.with_webview(move |wv| unsafe {
    let Some(view) = overlay_of(wv.inner() as *mut AnyObject) else { return };
    let Some(st) = state(view) else { return };
    st.page_width = page_width;
    st.regions = regions;
    st.holes = holes;
  });
}
