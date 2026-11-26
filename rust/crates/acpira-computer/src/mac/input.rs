//! Pointer and key events delivered to one window of one app. Nothing goes through the user's input stream: the real
//! pointer does not move, the front app does not change, and keys the user is holding do not leak into typed text.
//!
//! The pointer recipe (a move, a primer press outside the window, then the real press) and its gaps were measured by
//! open-computer-use and hush on macOS 27: each event carries the target pid and window in raw fields and is posted
//! through the window server only, because on macOS 27.2 the public and the private post each deliver, and posting
//! through both clicks twice.

use super::private::{self, SyntheticFocus};
use crate::steps::Button;
use std::ffi::c_void;
use std::thread::sleep;
use std::time::Duration;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
  x: f64,
  y: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
  fn CGEventSourceCreate(state: i32) -> *mut c_void;
  fn CGEventCreateMouseEvent(source: *mut c_void, kind: u32, at: CGPoint, button: u32) -> *mut c_void;
  fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> *mut c_void;
  fn CGEventCreateScrollWheelEvent2(source: *mut c_void, units: u32, wheels: u32, wheel1: i32, wheel2: i32, wheel3: i32) -> *mut c_void;
  fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
  fn CGEventSetFlags(event: *mut c_void, flags: u64);
  fn CGEventSetLocation(event: *mut c_void, at: CGPoint);
  fn CGEventKeyboardSetUnicodeString(event: *mut c_void, length: usize, text: *const u16);
  fn CGEventPostToPid(pid: i32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
  fn CFRelease(object: *mut c_void);
}

/// An owned Core Graphics object, released on drop.
struct Owned(*mut c_void);

impl Drop for Owned {
  fn drop(&mut self) {
    if !self.0.is_null() {
      // SAFETY: balances the create call that produced the pointer.
      unsafe { CFRelease(self.0) };
    }
  }
}

const SOURCE_PRIVATE: i32 = -1;
const SOURCE_HID: i32 = 1;

const MOUSE_MOVED: u32 = 5;
const LEFT_DOWN: u32 = 1;
const LEFT_UP: u32 = 2;
const RIGHT_DOWN: u32 = 3;
const RIGHT_UP: u32 = 4;

/// Raw event fields the recipe stamps.
mod field {
  pub const PHASE: u32 = 0;
  pub const CLICK_STATE: u32 = 1;
  pub const BUTTON: u32 = 3;
  pub const SUBTYPE: u32 = 7;
  pub const TARGET_PID: u32 = 40;
  /// `kCGEventSourceUserData`: marks the driver's own events.
  pub const USER_DATA: u32 = 42;
  pub const WINDOW: u32 = 51;
  pub const CLICK_GROUP: u32 = 58;
  pub const WINDOW_UNDER_POINTER: u32 = 91;
  /// `kCGScrollWheelEventScrollCount`.
  pub const SCROLL_COUNT: u32 = 100;
  pub const WINDOW_FOR_EVENT: u32 = 92;
}

/// ASCII "ACPR": lets input monitors tell the driver's events from the user's.
const MARKER: i64 = 0x4143_5052;

// The gaps cannot be zero: every click failed at zero in open-computer-use's sweep.
const AFTER_MOVE: Duration = Duration::from_millis(3);
const AFTER_PRIMER: Duration = Duration::from_millis(20);
const BETWEEN_CLICKS: Duration = Duration::from_millis(16);
/// Chromium's renderer takes the release asynchronously; the synthetic focus is held this long after it.
const RENDERER_SETTLE: Duration = Duration::from_millis(20);
/// Keys are taken asynchronously too: at a zero gap macOS 26 lost one key in fifty.
const KEY_RELEASE: Duration = Duration::from_millis(10);

/// The window pointer events go to.
#[derive(Clone, Copy)]
pub struct Target {
  pub pid: i32,
  pub window: u32,
  /// The window's top-left corner in screen points.
  pub origin: (f64, f64),
  /// The target is the user's front app, so it needs no synthetic focus.
  pub frontmost: bool,
}

/// Clicks at a window-local point. The caller has checked that the app is not hidden: a hidden app receives the
/// events but AppKit does not hand them to its views.
pub fn click(target: &Target, at: (f64, f64), button: Button, count: u8) -> Result<(), String> {
  // SAFETY: plain create call; a null source is accepted by the event constructors.
  let source = Owned(unsafe { CGEventSourceCreate(SOURCE_HID) });
  let (down, up, number) = match button {
    Button::Left => (LEFT_DOWN, LEFT_UP, 0),
    Button::Right => (RIGHT_DOWN, RIGHT_UP, 1),
  };
  let group = i64::from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos()));
  const PRIMER: (f64, f64) = (-1.0, -1.0);
  let focus = if target.frontmost { None } else { Some(SyntheticFocus::begin(target.pid, target.window)?) };
  let result = (|| {
    pointer(target, &source, MOUSE_MOVED, at, 0, 2, number, group)?;
    sleep(AFTER_MOVE);
    pointer(target, &source, down, PRIMER, 1, 1, number, group)?;
    pointer(target, &source, up, PRIMER, 1, 2, number, group)?;
    sleep(AFTER_PRIMER);
    for n in 1..=i64::from(count.max(1)) {
      pointer(target, &source, down, at, n, 3, number, group)?;
      pointer(target, &source, up, at, n, 3, number, group)?;
      if n < i64::from(count) {
        sleep(BETWEEN_CLICKS);
      }
    }
    Ok(())
  })();
  if focus.is_some() {
    sleep(RENDERER_SETTLE);
  }
  drop(focus);
  result
}

/// What one "line" of a scroll step is, in points: about a line of text.
const LINE: i32 = 20;
/// The most one event carries.
const PER_EVENT: i32 = 120;
const BETWEEN_SCROLLS: Duration = Duration::from_millis(8);

/// Scrolls by lines at a window-local point; positive `down` / `right` show what lies further that way.
///
/// The events carry points and a scroll count, as a trackpad's do, but no gesture phase. A plain wheel event is what
/// the scroll reversers people run for their mice flip (Mos tells a mouse by a zero count, LinearMouse by line
/// units), and they see the driver's events too. A gesture phase would start AppKit's own tracking loop, which then
/// reads the real trackpad. Points also go the same distance in every app: a wheel line is 10 points to AppKit and 40
/// to Chromium.
pub fn scroll(target: &Target, at: (f64, f64), right: i32, down: i32) -> Result<(), String> {
  // Deltas count up for content moving down, the opposite of "scroll down".
  let (mut dy, mut dx) = (-down.saturating_mul(LINE), -right.saturating_mul(LINE));
  while dy != 0 || dx != 0 {
    let (step_y, step_x) = (dy.clamp(-PER_EVENT, PER_EVENT), dx.clamp(-PER_EVENT, PER_EVENT));
    scroll_event(target, at, step_y, step_x)?;
    dy -= step_y;
    dx -= step_x;
    sleep(BETWEEN_SCROLLS);
  }
  Ok(())
}

fn scroll_event(target: &Target, at: (f64, f64), dy: i32, dx: i32) -> Result<(), String> {
  // SAFETY: plain create calls; the event is released by `Owned`.
  let (source, event) = unsafe {
    let source = Owned(CGEventSourceCreate(SOURCE_HID));
    // Unit 0 = points, which also marks the event as continuous.
    let event = Owned(CGEventCreateScrollWheelEvent2(source.0, 0, 2, dy, dx, 0));
    (source, event)
  };
  let _ = source;
  if event.0.is_null() {
    return Err("could not create a scroll event".into());
  }
  // SAFETY: live event.
  unsafe {
    CGEventSetIntegerValueField(event.0, field::SCROLL_COUNT, 1);
    CGEventSetLocation(event.0, screen(target, at));
  }
  stamp(target, event.0, at)?;
  private::post_to_pid(target.pid, event.0)
}

#[allow(clippy::too_many_arguments, reason = "one call per event of the recipe keeps the recipe readable")]
fn pointer(target: &Target, source: &Owned, kind: u32, at: (f64, f64), click_state: i64, phase: i64, button: i64, group: i64) -> Result<(), String> {
  // SAFETY: plain create call; the event is released by `Owned`.
  let event = Owned(unsafe { CGEventCreateMouseEvent(source.0, kind, screen(target, at), button as u32) });
  if event.0.is_null() {
    return Err("could not create a pointer event".into());
  }
  for (field, value) in [(field::PHASE, phase), (field::CLICK_STATE, click_state), (field::BUTTON, button), (field::SUBTYPE, 3), (field::CLICK_GROUP, group)] {
    // SAFETY: live event; the field ids are the ones the window server reads.
    unsafe { CGEventSetIntegerValueField(event.0, field, value) };
  }
  stamp(target, event.0, at)?;
  private::post_to_pid(target.pid, event.0)
}

fn stamp(target: &Target, event: *mut c_void, at: (f64, f64)) -> Result<(), String> {
  let window = i64::from(target.window);
  for (field, value) in [
    (field::TARGET_PID, i64::from(target.pid)),
    (field::WINDOW, window),
    (field::WINDOW_UNDER_POINTER, window),
    (field::WINDOW_FOR_EVENT, window),
    (field::USER_DATA, MARKER),
  ] {
    // SAFETY: live event.
    unsafe { CGEventSetIntegerValueField(event, field, value) };
  }
  // The primer is outside the window by design on every release.
  let local = private::macos_major() >= 27 || at == (-1.0, -1.0);
  let point = if local { CGPoint { x: at.0, y: at.1 } } else { screen(target, at) };
  private::set_window_location(event, point.x, point.y)
}

fn screen(target: &Target, at: (f64, f64)) -> CGPoint {
  if at == (-1.0, -1.0) {
    return CGPoint { x: -1.0, y: -1.0 };
  }
  CGPoint { x: target.origin.0 + at.0, y: target.origin.1 + at.1 }
}

pub const CMD: u64 = 0x10_0000;
pub const SHIFT: u64 = 0x2_0000;
pub const ALT: u64 = 0x8_0000;
pub const CTRL: u64 = 0x4_0000;
pub const FUNCTION: u64 = 0x80_0000;

/// A key with its modifiers, as written in a step: `cmd+shift+s`, `enter`, `f5`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Combo {
  pub code: u16,
  pub flags: u64,
  /// The key as a character, for matching menu shortcuts; `None` for named keys.
  pub character: Option<char>,
}

pub fn parse_combo(text: &str) -> Result<Combo, String> {
  let mut flags = 0;
  let mut key = None;
  for part in text.split('+').map(str::trim) {
    match part.to_ascii_lowercase().as_str() {
      "cmd" | "command" | "meta" | "super" => flags |= CMD,
      "shift" => flags |= SHIFT,
      "alt" | "option" | "opt" => flags |= ALT,
      "ctrl" | "control" => flags |= CTRL,
      "fn" => flags |= FUNCTION,
      "" => return Err(format!("\"{text}\" is not a key")),
      _ if key.is_some() => return Err(format!("\"{text}\" names two keys")),
      _ => key = Some(part),
    }
  }
  let key = key.ok_or_else(|| format!("\"{text}\" names no key"))?;
  let (code, character) = key_code(key).ok_or_else(|| format!("unknown key \"{key}\""))?;
  Ok(Combo { code, flags, character })
}

/// macOS virtual key codes (ANSI layout positions).
fn key_code(key: &str) -> Option<(u16, Option<char>)> {
  let lower = key.to_ascii_lowercase();
  let named = match lower.as_str() {
    "enter" | "return" => 0x24,
    "tab" => 0x30,
    "space" => 0x31,
    "backspace" | "delete" => 0x33,
    "escape" | "esc" => 0x35,
    "forwarddelete" | "del" => 0x75,
    "home" => 0x73,
    "end" => 0x77,
    "pageup" => 0x74,
    "pagedown" => 0x79,
    "left" => 0x7B,
    "right" => 0x7C,
    "down" => 0x7D,
    "up" => 0x7E,
    _ => {
      if let Some(number) = lower.strip_prefix('f').and_then(|n| n.parse::<usize>().ok()) {
        const F: [u16; 12] = [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F];
        return F.get(number.checked_sub(1)?).map(|code| (*code, None));
      }
      let mut chars = lower.chars();
      let (Some(c), None) = (chars.next(), chars.next()) else { return None };
      const KEYS: &str = "asdfhgzxcv\0bqweryt123465=97-80]ou[ip\0lj'k;\\,/nm.";
      let index = KEYS.chars().position(|k| k == c && k != '\0')?;
      return Some((index as u16, Some(c)));
    }
  };
  Some((named, None))
}

/// Presses a key combination `repeat` times.
pub fn press(pid: i32, combo: Combo, repeat: u16) {
  // SAFETY: plain create call. A private source keeps keys the user is holding out of the events.
  let source = Owned(unsafe { CGEventSourceCreate(SOURCE_PRIVATE) });
  for _ in 0..repeat.max(1) {
    key(pid, &source, combo.code, combo.flags);
  }
  sleep(KEY_RELEASE);
}

/// Types text as key events carrying the characters themselves, so the keyboard layout does not matter.
pub fn type_text(pid: i32, text: &str) {
  /// Characters per event: the call takes up to 20 UTF-16 units; fewer leaves room for a surrogate pair at the end.
  const CHUNK: usize = 16;
  // SAFETY: plain create call.
  let source = Owned(unsafe { CGEventSourceCreate(SOURCE_PRIVATE) });
  let mut pending: Vec<u16> = Vec::with_capacity(CHUNK + 2);
  let flush = |pending: &mut Vec<u16>| {
    if pending.is_empty() {
      return;
    }
    for down in [true, false] {
      // SAFETY: plain create call; `pending` holds `len` units for the whole call.
      unsafe {
        let event = Owned(CGEventCreateKeyboardEvent(source.0, 0, down));
        if event.0.is_null() {
          continue;
        }
        CGEventSetFlags(event.0, 0);
        CGEventKeyboardSetUnicodeString(event.0, pending.len(), pending.as_ptr());
        CGEventSetIntegerValueField(event.0, field::USER_DATA, MARKER);
        CGEventPostToPid(pid, event.0);
      }
    }
    pending.clear();
  };
  for c in text.chars() {
    let named = match c {
      '\n' | '\r' => Some(0x24),
      '\t' => Some(0x30),
      _ => None,
    };
    if let Some(code) = named {
      flush(&mut pending);
      key(pid, &source, code, 0);
      continue;
    }
    let mut units = [0u16; 2];
    pending.extend_from_slice(c.encode_utf16(&mut units));
    if pending.len() >= CHUNK {
      flush(&mut pending);
    }
  }
  flush(&mut pending);
  sleep(KEY_RELEASE);
}

fn key(pid: i32, source: &Owned, code: u16, flags: u64) {
  for down in [true, false] {
    // SAFETY: plain create call; the event is released by `Owned`.
    unsafe {
      let event = Owned(CGEventCreateKeyboardEvent(source.0, code, down));
      if event.0.is_null() {
        continue;
      }
      CGEventSetFlags(event.0, flags);
      CGEventSetIntegerValueField(event.0, field::USER_DATA, MARKER);
      CGEventPostToPid(pid, event.0);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn combos_parse() {
    assert_eq!(parse_combo("cmd+s"), Ok(Combo { code: 0x01, flags: CMD, character: Some('s') }));
    assert_eq!(parse_combo("Cmd+Shift+Z"), Ok(Combo { code: 0x06, flags: CMD | SHIFT, character: Some('z') }));
    assert_eq!(parse_combo("enter"), Ok(Combo { code: 0x24, flags: 0, character: None }));
    assert_eq!(parse_combo("f5").map(|c| c.code), Ok(0x60));
    assert_eq!(parse_combo("ctrl+alt+left").map(|c| (c.code, c.flags)), Ok((0x7B, CTRL | ALT)));
    assert_eq!(parse_combo("5").map(|c| c.code), Ok(0x17));
    assert_eq!(parse_combo("m").map(|c| c.code), Ok(0x2E));
    assert!(parse_combo("cmd+").is_err());
    assert!(parse_combo("a+b").is_err());
    assert!(parse_combo("cmd").is_err());
    assert!(parse_combo("nope").is_err());
  }
}
