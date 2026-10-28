//! macOS pieces Tauri does not cover: WebKit's frame-rate switch, rendering while covered (smoke and bench runs), the
//! title bar's double-click setting, mouse-wheel notches eased like Chromium's, the pasteboard, responder-chain edit actions and native notifications. Private
//! WebKit API is looked up before every call and left at the default (with a log line) where it is missing

use std::ffi::{CStr, c_char};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
  UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions, UNNotificationRequest,
  UNNotificationResponse, UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use crate::os::{Log, TitleDoubleClick, session_of_request};

/// WebKit keeps a page's rendering updates near 60 fps by default (on a 160 Hz display that lands at 80). The switch is
/// the private feature PreferPageRenderingUpdatesNear60FPSEnabled; WKPreferences is shared with the live page, so turning
/// it off applies to what loads next: call it before the real page loads. Private API: fine for a directly distributed
/// app, not for the App Store; checked again on every macOS release (docs/dev/desktop.md)
pub fn unlock_frame_rate(window: &tauri::WebviewWindow, log: Log) {
  set_webkit_feature(window, "frame rate", c"PreferPageRenderingUpdatesNear60FPSEnabled", false, log);
}

/// Overflow scrolling on the page's main thread from the next load on: WebKit's AsyncOverflowScrollingEnabled off, so the
/// page has no overflow nodes in the UI-side scrolling tree. The fallback for a window whose WebContent process WebKit
/// keeps killing over that tree (app.rs page_process_ended)
pub fn sync_overflow_scrolling(window: &tauri::WebviewWindow, log: Log) {
  set_webkit_feature(window, "overflow scrolling", c"AsyncOverflowScrollingEnabled", false, log);
}

/// One private WKPreferences feature of the window's webview by key; WKPreferences is shared with the live page, so the
/// change applies to what loads next. Missing API or key: left at WebKit's default, with a log line naming `what`
fn set_webkit_feature(window: &tauri::WebviewWindow, what: &'static str, feature: &'static CStr, enabled: bool, log: Log) {
  // SAFETY: `wv.inner()` is the live WKWebView; each private selector is checked with respondsToSelector: before it is
  // sent, messages to nil return nil / 0, and the C string is null-checked before it is read
  let _ = window.with_webview(move |wv| unsafe {
    let wk = wv.inner() as *mut AnyObject;
    let config: *mut AnyObject = msg_send![wk, configuration];
    let prefs: *mut AnyObject = msg_send![config, preferences];
    let Some(cls) = AnyClass::get(c"WKPreferences") else {
      log(&format!("{what}: no WKPreferences, left at WebKit's default"));
      return;
    };
    let has_features: Bool = msg_send![cls, respondsToSelector: Sel::register(c"_features")];
    let has_set: Bool = msg_send![prefs, respondsToSelector: Sel::register(c"_setEnabled:forFeature:")];
    if !has_features.as_bool() || !has_set.as_bool() {
      log(&format!("{what}: WKPreferences _features / _setEnabled:forFeature: missing, left at WebKit's default"));
      return;
    }
    let features: *mut AnyObject = msg_send![cls, _features];
    let count: usize = msg_send![features, count];
    for i in 0..count {
      let f: *mut AnyObject = msg_send![features, objectAtIndex: i];
      let key: *mut AnyObject = msg_send![f, key];
      let utf8: *const c_char = msg_send![key, UTF8String];
      if !utf8.is_null() && CStr::from_ptr(utf8) == feature {
        let _: () = msg_send![prefs, _setEnabled: Bool::new(enabled), forFeature: f];
        return;
      }
    }
    log(&format!("{what}: {} is not among WebKit's features, left at the default", feature.to_string_lossy()));
  });
}

/// Smoke and bench runs: the window keeps rendering while other windows cover it and App Nap leaves the app's timers
/// alone, so a run in the background never needs the window in front of whoever uses the Mac
pub fn keep_rendering_in_background(window: &tauri::WebviewWindow, log: Log) {
  // SAFETY: `wv.inner()` is the live WKWebView and the private selector is sent only when it responds to it
  let _ = window.with_webview(move |wv| unsafe {
    let wk = wv.inner() as *mut AnyObject;
    let sel = Sel::register(c"_setWindowOcclusionDetectionEnabled:");
    let responds: Bool = msg_send![wk, respondsToSelector: sel];
    if responds.as_bool() {
      let _: () = msg_send![wk, _setWindowOcclusionDetectionEnabled: Bool::NO];
    } else {
      log("_setWindowOcclusionDetectionEnabled: missing, a covered window stops rendering");
    }
  });
  static ACTIVITY: std::sync::Once = std::sync::Once::new();
  // SAFETY: documented NSProcessInfo messages on the class looked up just before; retaining the token keeps it alive
  ACTIVITY.call_once(|| unsafe {
    let Some(info) = AnyClass::get(c"NSProcessInfo") else { return };
    let process: *mut AnyObject = msg_send![info, processInfo];
    let reason = NSString::from_str("background smoke run");
    // NSActivityUserInitiated | NSActivityLatencyCritical; the token is kept for the life of the process
    let token: *mut AnyObject = msg_send![process, beginActivityWithOptions: 0x00FF_FFFFu64 | 0xFF_0000_0000u64, reason: &*reason];
    let _: *mut AnyObject = msg_send![token, retain];
  });
}

/// What a double click on a title bar does, as System Settings › Desktop & Dock says: zoom (the default), minimize or
/// nothing
pub fn title_double_click() -> TitleDoubleClick {
  // SAFETY: documented NSUserDefaults / NSString messages; nil objects and a null C string are checked before use
  unsafe {
    let Some(cls) = AnyClass::get(c"NSUserDefaults") else { return TitleDoubleClick::Zoom };
    let defaults: *mut AnyObject = msg_send![cls, standardUserDefaults];
    let key = NSString::from_str("AppleActionOnDoubleClick");
    let value: *mut AnyObject = msg_send![defaults, stringForKey: &*key];
    if value.is_null() {
      return TitleDoubleClick::Zoom;
    }
    let utf8: *const c_char = msg_send![value, UTF8String];
    if utf8.is_null() {
      return TitleDoubleClick::Zoom;
    }
    match CStr::from_ptr(utf8).to_bytes() {
      b"Minimize" => TitleDoubleClick::Minimize,
      b"None" => TitleDoubleClick::Nothing,
      _ => TitleDoubleClick::Zoom,
    }
  }
}

/// The user's first preferred language as a BCP 47 tag ("zh-Hans-CN"), what the page and the engine resolve the UI
/// language from (resolve_locale)
pub fn preferred_language() -> Option<String> {
  // SAFETY: documented NSLocale / NSArray / NSString messages (nil answers 0); the index is below the count and the C
  // string is null-checked before it is read
  unsafe {
    let cls = AnyClass::get(c"NSLocale")?;
    let langs: *mut AnyObject = msg_send![cls, preferredLanguages];
    let count: usize = msg_send![langs, count];
    if count == 0 {
      return None;
    }
    let first: *mut AnyObject = msg_send![langs, objectAtIndex: 0usize];
    let utf8: *const c_char = msg_send![first, UTF8String];
    (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned())
  }
}

/// Whether the system appearance is dark (NSApp's effective appearance)
pub fn dark_mode() -> bool {
  // SAFETY: documented NSApplication / NSAppearance messages; nil objects and a null C string are checked before use
  unsafe {
    let Some(cls) = AnyClass::get(c"NSApplication") else { return false };
    let app: *mut AnyObject = msg_send![cls, sharedApplication];
    let appearance: *mut AnyObject = msg_send![app, effectiveAppearance];
    if appearance.is_null() {
      return false;
    }
    let name: *mut AnyObject = msg_send![appearance, name];
    if name.is_null() {
      return false;
    }
    let utf8: *const c_char = msg_send![name, UTF8String];
    !utf8.is_null() && CStr::from_ptr(utf8).to_string_lossy().contains("Dark")
  }
}

/// A mouse wheel's notch eased over a short stretch, as Chromium does, instead of WebKit's one-frame jump (WebKit moves a
/// classic, non-precise notch of 3 lines, 120 px, in a single frame whatever its ScrollAnimatorEnabled says; measured
/// 2026-10-26, docs/dev/desktop.md). A local monitor takes every notch, adds its distance (lines × WebKit's 40 px) to a
/// target and hands it to the window as precise pixel scrolls without a phase (copies of the notch's own event, so the
/// window and the point stay its), one per timer tick along an ease-out, so
/// WebKit scrolls them on its scrolling thread like a trackpad's and the page sees ordinary wheel events. Without a
/// phase there is no rubber band at the ends. Trackpads, Magic Mouse and smoothing tools that simulate a trackpad send
/// precise or phased events already and pass untouched, as does a notch with ⌘, ⌃ or ⌥ held (zoom and the like).
/// ACPIRA_WHEEL_SMOOTH=0 turns it off. Main thread
pub fn smooth_wheel(log: Log) {
  if std::env::var("ACPIRA_WHEEL_SMOOTH").is_ok_and(|v| v == "0") {
    log("wheel: smoothing off (ACPIRA_WHEEL_SMOOTH=0)");
    return;
  }
  let handler = block2::RcBlock::new(|event: *mut AnyObject| -> *mut AnyObject {
    // SAFETY: AppKit hands the monitor a live NSEvent on the main thread; returning nil drops it
    if unsafe { wheel::take(event) } { std::ptr::null_mut() } else { event }
  });
  let Some(cls) = AnyClass::get(c"NSEvent") else { return };
  // NSEventMaskScrollWheel
  let mask: u64 = 1 << 22;
  // SAFETY: a class method taking a mask and a block; the monitor it returns is retained for the life of the app
  unsafe {
    let monitor: *mut AnyObject = msg_send![cls, addLocalMonitorForEventsMatchingMask: mask, handler: &*handler];
    if monitor.is_null() {
      log("wheel: no event monitor, notches stay WebKit's");
      return;
    }
    let _: *mut AnyObject = msg_send![monitor, retain];
  }
}

/// A classic wheel notch (`lines`, positive scrolls up) at a point of the window's content (top-left origin), put into
/// the app's own event queue so it takes the path a mouse's notch takes, the monitor included (smoke and bench runs: a
/// notch posted to the process is routed by the window server to whatever window is in front at that point). Main thread
#[cfg(feature = "smoke")]
pub fn post_wheel_notch(window: &tauri::WebviewWindow, x: f64, y: f64, lines: i32) -> bool {
  let Ok(ns) = window.ns_window() else { return false };
  wheel::post_notch(ns as *mut AnyObject, x, y, lines)
}

mod wheel {
  use std::cell::RefCell;
  use std::ffi::c_void;
  use std::time::{Duration, Instant};

  use objc2::encode::{Encoding, RefEncode};
  use objc2::msg_send;
  use objc2::rc::Retained;
  use objc2::runtime::{AnyClass, AnyObject, Bool};

  /// How long one notch takes to land (Chromium's own ease was ~200 ms, 13 frames at 60 Hz)
  const DURATION: f64 = 0.2;
  /// WebKit's Scrollbar::pixelsPerLineStep: what one line of a non-precise wheel delta scrolls
  const LINE_PX: f64 = 40.0;
  /// The tick: above any display's rate, so every frame gets its share
  const TICK: f64 = 1.0 / 240.0;
  // NSEventModifierFlagControl | Option | Command
  const MODIFIERS: u64 = (1 << 18) | (1 << 19) | (1 << 20);
  // kCGScrollWheelEventDeltaAxis1/2, …FixedPtDeltaAxis1/2, …PointDeltaAxis1/2, …IsContinuous
  const DELTA: [u32; 2] = [11, 12];
  const FIXED: [u32; 2] = [93, 94];
  const POINT: [u32; 2] = [96, 97];
  const CONTINUOUS: u32 = 88;

  #[repr(C)]
  pub struct CGEvent {
    _private: [u8; 0],
  }
  // SAFETY: CGEventRef is a pointer to the opaque struct __CGEvent, which is how AppKit's method signatures encode it
  unsafe impl RefEncode for CGEvent {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CGEvent", &[]));
  }

  #[link(name = "CoreGraphics", kind = "framework")]
  unsafe extern "C" {
    fn CGEventCreate(source: *const c_void) -> *mut CGEvent;
    fn CGEventCreateCopy(event: *const CGEvent) -> *mut CGEvent;
    fn CGEventGetTimestamp(event: *const CGEvent) -> u64;
    fn CGEventSetTimestamp(event: *mut CGEvent, timestamp: u64);
    fn CGEventSetIntegerValueField(event: *mut CGEvent, field: u32, value: i64);
    fn CGEventSetDoubleValueField(event: *mut CGEvent, field: u32, value: f64);
  }
  #[link(name = "CoreFoundation", kind = "framework")]
  unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
  }

  /// The pixels still on their way to one window, eased from where the last notch found them to the target
  struct Glide {
    window: Retained<AnyObject>,
    // A copy of the last notch: every scroll handed over is a copy of it turned precise, so it keeps the notch's window,
    // its point in the window and its modifier flags without composing them (an NSEvent made from a fresh CGEvent has
    // no window, and AppKit drops it or hit-tests the window's corner)
    template: *mut CGEvent,
    start: Instant,
    from: (f64, f64),
    to: (f64, f64),
    // Whole pixels handed over so far, per axis (the rounding remainder stays in the target)
    sent: (i64, i64),
    timer: Option<Retained<AnyObject>>,
  }

  impl Drop for Glide {
    fn drop(&mut self) {
      self.stop();
      // SAFETY: the copy this glide owns
      unsafe { CFRelease(self.template as *const c_void) };
    }
  }

  thread_local! {
    static GLIDE: RefCell<Option<Glide>> = const { RefCell::new(None) };
  }

  /// Takes a classic wheel notch into the glide (true: the monitor drops the original)
  ///
  /// # Safety
  /// `event` is a live NSEvent; main thread
  pub unsafe fn take(event: *mut AnyObject) -> bool {
    // SAFETY: plain NSEvent getters on the live event
    let (precise, phase, momentum, flags, window, dx, dy, cg) = unsafe {
      let precise: Bool = msg_send![event, hasPreciseScrollingDeltas];
      let phase: u64 = msg_send![event, phase];
      let momentum: u64 = msg_send![event, momentumPhase];
      let flags: u64 = msg_send![event, modifierFlags];
      let window: *mut AnyObject = msg_send![event, window];
      let dx: f64 = msg_send![event, scrollingDeltaX];
      let dy: f64 = msg_send![event, scrollingDeltaY];
      let cg: *mut CGEvent = msg_send![event, CGEvent];
      (precise.as_bool(), phase, momentum, flags, window, dx, dy, cg)
    };
    if precise || phase != 0 || momentum != 0 || flags & MODIFIERS != 0 || window.is_null() || cg.is_null() || (dx == 0.0 && dy == 0.0) {
      return false;
    }
    // SAFETY: the event's window, alive while the event is
    let Some(window) = (unsafe { Retained::retain(window) }) else { return false };
    // SAFETY: the event's own CGEvent, copied (the copy is owned by the glide from here on)
    let template = unsafe { CGEventCreateCopy(cg) };
    if template.is_null() {
      return false;
    }
    let now = Instant::now();
    GLIDE.with_borrow_mut(|glide| {
      // A notch over another window lands what the last one still owed first
      if glide.as_ref().is_some_and(|g| !std::ptr::eq(Retained::as_ptr(&g.window), Retained::as_ptr(&window)))
        && let Some(mut old) = glide.take()
      {
        old.finish(now);
      }
      match glide {
        Some(g) => {
          // Retarget from where the glide is now: a quick second notch speeds the scroll up instead of queueing behind it
          g.from = g.position(now);
          g.to = (g.to.0 + dx * LINE_PX, g.to.1 + dy * LINE_PX);
          g.start = now;
          // SAFETY: the previous copy, owned by the glide
          unsafe { CFRelease(g.template as *const c_void) };
          g.template = template;
        }
        None => {
          *glide = Some(Glide { window, template, start: now, from: (0.0, 0.0), to: (dx * LINE_PX, dy * LINE_PX), sent: (0, 0), timer: None });
        }
      }
      let Some(g) = glide.as_mut() else { return };
      if g.timer.is_none() {
        g.timer = start_timer();
      }
      // No timer (the class went missing): everything lands at once rather than never
      if g.timer.is_none() {
        g.finish(now);
        *glide = None;
      }
    });
    true
  }

  impl Glide {
    fn position(&self, now: Instant) -> (f64, f64) {
      let t = (now.duration_since(self.start).as_secs_f64() / DURATION).min(1.0);
      // Ease-out cubic: fast at the notch, settling at the target
      let k = 1.0 - (1.0 - t).powi(3);
      (self.from.0 + (self.to.0 - self.from.0) * k, self.from.1 + (self.to.1 - self.from.1) * k)
    }

    /// Hands over the pixels due by `now`; false once the target is reached
    fn step(&mut self, now: Instant) -> bool {
      let (x, y) = self.position(now);
      let (x, y) = (x.round() as i64, y.round() as i64);
      let (dx, dy) = (x - self.sent.0, y - self.sent.1);
      if dx != 0 || dy != 0 {
        self.sent = (x, y);
        self.deliver([dy, dx]);
      }
      now.duration_since(self.start).as_secs_f64() < DURATION
    }

    /// Lands whatever is still owed in one go
    fn finish(&mut self, now: Instant) {
      self.start = now - Duration::from_secs_f64(DURATION);
      self.step(now);
      self.stop();
    }

    /// One precise scroll of `[vertical, horizontal]` pixels, to the window of the notch it came from
    fn deliver(&self, px: [i64; 2]) {
      let Some(cls) = AnyClass::get(c"NSEvent") else { return };
      // SAFETY: CGEvent copies owned here and released below; NSEvent copies what it needs; the window is retained
      unsafe {
        let cg = CGEventCreateCopy(self.template);
        if cg.is_null() {
          return;
        }
        // What a pixel scroll carries, as CoreGraphics fills it for one (line delta ±1, fixed-point delta a tenth),
        // stamped now so WebKit sees one ongoing scroll. The pixel delta goes last: setting the line delta recomputes
        // it (to 8 px a line), and it is what NSEvent's scrollingDelta reads
        CGEventSetIntegerValueField(cg, CONTINUOUS, 1);
        for axis in 0..2 {
          CGEventSetIntegerValueField(cg, DELTA[axis], px[axis].signum());
          CGEventSetDoubleValueField(cg, FIXED[axis], px[axis] as f64 / 10.0);
          CGEventSetIntegerValueField(cg, POINT[axis], px[axis]);
        }
        let clock = CGEventCreate(std::ptr::null());
        if !clock.is_null() {
          CGEventSetTimestamp(cg, CGEventGetTimestamp(clock));
          CFRelease(clock as *const c_void);
        }
        let event: *mut AnyObject = msg_send![cls, eventWithCGEvent: cg];
        if !event.is_null() {
          let _: () = msg_send![&*self.window, sendEvent: event];
        }
        CFRelease(cg as *const c_void);
      }
    }

    fn stop(&mut self) {
      if let Some(timer) = self.timer.take() {
        // SAFETY: the glide's own NSTimer
        let _: () = unsafe { msg_send![&*timer, invalidate] };
      }
    }
  }

  /// A repeating timer in the run loop's common modes, so the glide goes on during a menu or a window drag
  fn start_timer() -> Option<Retained<AnyObject>> {
    let tick = block2::RcBlock::new(|_timer: *mut AnyObject| {
      GLIDE.with_borrow_mut(|glide| {
        let Some(g) = glide.as_mut() else { return };
        if !g.step(Instant::now()) {
          *glide = None;
        }
      });
    });
    let timer_cls = AnyClass::get(c"NSTimer")?;
    let loop_cls = AnyClass::get(c"NSRunLoop")?;
    // NSRunLoopCommonModes
    let modes = objc2_foundation::NSString::from_str("kCFRunLoopCommonModes");
    // SAFETY: documented NSTimer / NSRunLoop calls on the main thread; the timer is returned +0 and retained here
    unsafe {
      let timer: *mut AnyObject = msg_send![timer_cls, timerWithTimeInterval: TICK, repeats: Bool::YES, block: &*tick];
      let timer = Retained::retain(timer)?;
      let run_loop: *mut AnyObject = msg_send![loop_cls, mainRunLoop];
      let _: () = msg_send![run_loop, addTimer: &*timer, forMode: &*modes];
      Some(timer)
    }
  }

  /// A classic notch at a window-relative point (top-left origin), into the app's event queue: the window comes from
  /// field 51 and the point from CoreGraphics' private CGEventSetWindowLocation, looked up at run time (bench runs only)
  #[cfg(feature = "smoke")]
  pub fn post_notch(ns_window: *mut AnyObject, x: f64, y: f64, lines: i32) -> bool {
    #[repr(C)]
    struct CGPoint {
      x: f64,
      y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
      fn CGEventCreateScrollWheelEvent2(source: *const c_void, units: u32, wheel_count: u32, wheel1: i32, wheel2: i32, wheel3: i32) -> *mut CGEvent;
    }
    unsafe extern "C" {
      fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
    }
    // SAFETY: dlsym with RTLD_DEFAULT (-2) and a NUL-terminated name only looks the symbol up
    let sym = unsafe { dlsym(-2isize as *mut c_void, c"CGEventSetWindowLocation".as_ptr()) };
    let (Some(event_cls), Some(app_cls)) = (AnyClass::get(c"NSEvent"), AnyClass::get(c"NSApplication")) else { return false };
    if sym.is_null() {
      return false;
    }
    // SAFETY: the symbol has this C signature (void CGEventSetWindowLocation(CGEventRef, CGPoint))
    let set_window_location: unsafe extern "C" fn(*mut CGEvent, CGPoint) = unsafe { std::mem::transmute(sym) };
    // SAFETY: the window's own NSWindow; a fresh CGEvent released below; postEvent:atStart: keeps its own reference
    unsafe {
      let number: isize = msg_send![ns_window, windowNumber];
      // kCGScrollEventUnitLine
      let cg = CGEventCreateScrollWheelEvent2(std::ptr::null(), 1, 1, lines, 0, 0);
      if cg.is_null() {
        return false;
      }
      CGEventSetIntegerValueField(cg, 51, number as i64);
      set_window_location(cg, CGPoint { x, y });
      let event: *mut AnyObject = msg_send![event_cls, eventWithCGEvent: cg];
      CFRelease(cg as *const c_void);
      if event.is_null() {
        return false;
      }
      let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
      let _: () = msg_send![app, postEvent: event, atStart: Bool::NO];
    }
    true
  }

  #[cfg(test)]
  mod tests {
    // The ease lands exactly on the target and never overshoots on the way
    #[test]
    fn the_ease_ends_on_the_target() {
      let steps = 48;
      let mut last = 0.0;
      for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let k = 1.0 - (1.0 - t).powi(3);
        assert!(k >= last && k <= 1.0);
        last = k;
      }
      assert_eq!(last, 1.0);
    }
  }
}

/// Shows a window in front of the others without making it key or activating the app (Electron's showInactive): an
/// agent's open_browser, and every window of a background run
pub fn order_front(window: &tauri::WebviewWindow) {
  order(window, false);
}

/// Shows a window behind every other one: a smoke or bench run in the background, where nothing may come up in front of
/// whoever uses the Mac (the window keeps rendering, see keep_rendering_in_background)
pub fn order_back(window: &tauri::WebviewWindow) {
  order(window, true);
}

fn order(window: &tauri::WebviewWindow, back: bool) {
  let Ok(ns) = window.ns_window() else { return };
  let ns = ns as usize;
  // SAFETY: the window's own NSWindow, messaged on the main thread with nil as the sender
  let _ = window.run_on_main_thread(move || unsafe {
    let w = ns as *mut AnyObject;
    let none: *mut AnyObject = std::ptr::null_mut();
    if back {
      let _: () = msg_send![w, orderBack: none];
    } else {
      let _: () = msg_send![w, orderFront: none];
    }
  });
}

/// The window server's number for a window (screencapture -l). Main thread
#[cfg(feature = "smoke")]
pub fn window_number(window: &tauri::WebviewWindow) -> Option<isize> {
  let ns = window.ns_window().ok()? as *mut AnyObject;
  // SAFETY: the window's own NSWindow; windowNumber takes no arguments
  Some(unsafe { msg_send![ns, windowNumber] })
}

fn pasteboard() -> Option<*mut AnyObject> {
  // SAFETY: a documented class message on the class looked up just before; a nil answer is checked
  unsafe {
    let cls = AnyClass::get(c"NSPasteboard")?;
    let pb: *mut AnyObject = msg_send![cls, generalPasteboard];
    (!pb.is_null()).then_some(pb)
  }
}

const PASTEBOARD_STRING: &str = "public.utf8-plain-text";

/// Main thread only
pub fn clipboard_write(text: &str) {
  let Some(pb) = pasteboard() else { return };
  // SAFETY: documented NSPasteboard messages on the non-nil general pasteboard, with NSStrings that outlive the calls
  unsafe {
    let _: isize = msg_send![pb, clearContents];
    let s = NSString::from_str(text);
    let ty = NSString::from_str(PASTEBOARD_STRING);
    let _: Bool = msg_send![pb, setString: &*s, forType: &*ty];
  }
}

/// Main thread only
pub fn clipboard_read() -> String {
  let Some(pb) = pasteboard() else { return String::new() };
  // SAFETY: a documented NSPasteboard message on the non-nil general pasteboard; ns_string checks the answer for nil
  unsafe {
    let ty = NSString::from_str(PASTEBOARD_STRING);
    let s: *mut AnyObject = msg_send![pb, stringForType: &*ty];
    ns_string(s).unwrap_or_default()
  }
}

/// A GIF on the pasteboard as a Finder copy carries a file (its file URL), plus its bytes as GIF data for apps that read
/// images rather than files. Main thread only
pub fn clipboard_write_gif(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
  let pb = pasteboard().ok_or("no general pasteboard")?;
  let classes = (AnyClass::get(c"NSData"), AnyClass::get(c"NSURL"), AnyClass::get(c"NSPasteboardItem"), AnyClass::get(c"NSArray"));
  let (Some(data_cls), Some(url_cls), Some(item_cls), Some(array_cls)) = classes else { return Err("AppKit classes missing".into()) };
  // SAFETY: documented NSData / NSURL / NSPasteboardItem / NSArray / NSPasteboard messages on the classes looked up above;
  // every answer is checked for nil before it is used, dataWithBytes:length: copies `data` while it is borrowed, and the
  // item made with `new` is released by its Retained once the pasteboard holds it
  unsafe {
    let bytes: *mut AnyObject = msg_send![data_cls, dataWithBytes: data.as_ptr().cast::<std::ffi::c_void>(), length: data.len()];
    let file = NSString::from_str(&path.to_string_lossy());
    let url: *mut AnyObject = msg_send![url_cls, fileURLWithPath: &*file];
    if bytes.is_null() || url.is_null() {
      return Err("could not read the image file".into());
    }
    let url_string: *mut AnyObject = msg_send![url, absoluteString];
    let item: Option<Retained<AnyObject>> = msg_send![item_cls, new];
    let item = item.ok_or("no pasteboard item")?;
    let file_type = NSString::from_str("public.file-url");
    let gif_type = NSString::from_str("com.compuserve.gif");
    let _: Bool = msg_send![&*item, setString: url_string, forType: &*file_type];
    let _: Bool = msg_send![&*item, setData: bytes, forType: &*gif_type];
    let items: *mut AnyObject = msg_send![array_cls, arrayWithObject: &*item];
    if items.is_null() {
      return Err("no pasteboard item list".into());
    }
    let _: isize = msg_send![pb, clearContents];
    let written: Bool = msg_send![pb, writeObjects: items];
    if written.as_bool() { Ok(()) } else { Err("the pasteboard refused the image".into()) }
  }
}

fn ns_string(s: *mut AnyObject) -> Option<String> {
  if s.is_null() {
    return None;
  }
  // SAFETY: `s` is a non-nil NSString
  let utf8: *const c_char = unsafe { msg_send![s, UTF8String] };
  // SAFETY: a non-null UTF8String answer is a NUL-terminated C string that lives as long as `s`
  (!utf8.is_null()).then(|| unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned())
}

/// The file paths of the drag in progress (or the last one), from the system's drag pasteboard: WKWebView gives the page
/// a dropped file's name and bytes but not where it lives, which a folder dropped on the session column needs. Main
/// thread only
pub fn drag_paths() -> Vec<String> {
  // SAFETY: documented NSPasteboard / NSArray / NSURL messages; every object is checked for nil before it is messaged
  // further and the index stays below the count
  unsafe {
    let Some(cls) = AnyClass::get(c"NSPasteboard") else { return vec![] };
    let name = NSString::from_str("Apple CFPasteboard drag");
    let pb: *mut AnyObject = msg_send![cls, pasteboardWithName: &*name];
    if pb.is_null() {
      return vec![];
    }
    let items: *mut AnyObject = msg_send![pb, pasteboardItems];
    if items.is_null() {
      return vec![];
    }
    let count: usize = msg_send![items, count];
    let ty = NSString::from_str("public.file-url");
    let Some(url_cls) = AnyClass::get(c"NSURL") else { return vec![] };
    (0..count)
      .filter_map(|i| {
        let item: *mut AnyObject = msg_send![items, objectAtIndex: i];
        let s: *mut AnyObject = msg_send![item, stringForType: &*ty];
        if s.is_null() {
          return None;
        }
        // A file reference URL (file:///.file/id=…) resolves to the file's path through NSURL
        let url: *mut AnyObject = msg_send![url_cls, URLWithString: s];
        if url.is_null() {
          return None;
        }
        let file: *mut AnyObject = msg_send![url, filePathURL];
        let path: *mut AnyObject = msg_send![if file.is_null() { url } else { file }, path];
        ns_string(path)
      })
      .collect()
  }
}

/// An edit action through the responder chain (the focused page, its text field), for menu items with no stock
/// equivalent: pasteAsPlainText:, delete:, startSpeaking:, stopSpeaking:. Main thread only
pub fn send_action(selector: &CStr) {
  // SAFETY: a documented NSApplication message with nil target and sender, which sends the action down the responder chain
  unsafe {
    let Some(cls) = AnyClass::get(c"NSApplication") else { return };
    let app: *mut AnyObject = msg_send![cls, sharedApplication];
    let none: *mut AnyObject = std::ptr::null_mut();
    let _: Bool = msg_send![app, sendAction: Sel::register(selector), to: none, from: none];
  }
}

/// Whether the process runs from an app bundle: UNUserNotificationCenter aborts the process without one (a `cargo run`)
pub fn bundled() -> bool {
  // SAFETY: documented NSBundle messages on the class looked up just before; nil answers 0 / nil
  unsafe {
    let Some(cls) = AnyClass::get(c"NSBundle") else { return false };
    let main: *mut AnyObject = msg_send![cls, mainBundle];
    let id: *mut AnyObject = msg_send![main, bundleIdentifier];
    !id.is_null()
  }
}

type OnClick = Box<dyn Fn(String) + Send + Sync>;

pub struct DelegateIvars {
  on_click: OnClick,
}

define_class!(
  // The notification center's delegate: a click brings the session up; a notification that arrives while the app is in
  // front still shows (the user is in another window or another session)
  #[unsafe(super(NSObject))]
  #[name = "AcpiraNotificationDelegate"]
  #[ivars = DelegateIvars]
  struct NotificationDelegate;

  unsafe impl NSObjectProtocol for NotificationDelegate {}

  unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
    #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
    fn did_receive(&self, _center: &UNUserNotificationCenter, response: &UNNotificationResponse, completion: &block2::DynBlock<dyn Fn()>) {
      let id = response.notification().request().identifier().to_string();
      if let Some(session) = session_of_request(&id) {
        (self.ivars().on_click)(session.to_owned());
      }
      completion.call(());
    }

    #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
    fn will_present(
      &self,
      _center: &UNUserNotificationCenter,
      _n: &UNNotification,
      completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
    ) {
      completion.call((UNNotificationPresentationOptions::Banner
        | UNNotificationPresentationOptions::List
        | UNNotificationPresentationOptions::Sound,));
    }
  }
);

pub struct Notifier {
  center: Retained<UNUserNotificationCenter>,
  _delegate: Retained<NotificationDelegate>,
}

// SAFETY: the center and the delegate are only touched on the main thread (the shell dispatches there)
unsafe impl Send for Notifier {}
// SAFETY: as for Send, every use is on the main thread
unsafe impl Sync for Notifier {}

impl Notifier {
  /// None outside an app bundle. Asks for permission once (the system remembers the answer). Main thread only
  pub fn new(on_click: impl Fn(String) + Send + Sync + 'static, log: Log) -> Option<Notifier> {
    if !bundled() {
      log("notifications: not running from an app bundle, system notifications are off");
      return None;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = NotificationDelegate::alloc().set_ivars(DelegateIvars { on_click: Box::new(on_click) });
    // SAFETY: NSObject's init on the freshly allocated delegate whose ivars were just set
    let delegate: Retained<NotificationDelegate> = unsafe { msg_send![super(delegate), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let l = log.clone();
    let done = block2::RcBlock::new(move |granted: Bool, _err: *mut objc2_foundation::NSError| {
      if !granted.as_bool() {
        l("notifications: permission not granted");
      }
    });
    center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &done);
    Some(Notifier { center, _delegate: delegate })
  }

  /// `session`: clicking brings it up; None for a plain message (a toast with no window open). Main thread only
  pub fn show(&self, title: &str, body: &str, session: Option<&str>) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let id = match session {
      Some(s) => format!("session:{s}:{unique}"),
      None => format!("toast:{unique}"),
    };
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &content, None);
    self.center.addNotificationRequest_withCompletionHandler(&request, None);
  }
}
