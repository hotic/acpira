//! Every undocumented macOS call the driver makes, in one file so an OS update has one place to review. Symbols are
//! looked up at run time: when one is missing, the feature that needs it reports that instead of failing to launch.
//!
//! Signatures, record layouts and event fields are private ABI facts taken from MIT-licensed projects that verified
//! them on macOS 26 and 27: yabai (`window_manager_make_key_window`), Cua Driver (`skylight.rs`), open-computer-use
//! (`SkyLightSPI.swift`) and hush (`private.rs`).

use std::ffi::{CStr, c_void};
use std::sync::OnceLock;

const SKYLIGHT: &CStr = c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";
const APPLICATION_SERVICES: &CStr = c"/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices";

/// Size of a window server event record.
const RECORD: usize = 0xF8;

struct Api {
  post_to_pid: unsafe extern "C" fn(i32, *mut c_void),
  set_window_location: unsafe extern "C" fn(*mut c_void, f64, f64),
  post_record: unsafe extern "C" fn(*const u8, *const u8) -> i32,
  process_for_pid: unsafe extern "C" fn(i32, *mut u8) -> i32,
  main_connection: unsafe extern "C" fn() -> u32,
  capture_windows: unsafe extern "C" fn(u32, *const u32, i32, u32) -> *mut c_void,
}

/// A symbol as a function pointer of type `F`.
///
/// # Safety
/// `F` must be the `extern "C" fn` type of the symbol.
unsafe fn function<F: Copy>(library: &CStr, name: &CStr) -> Option<F> {
  // SAFETY: valid C strings; a null handle or symbol is handled.
  let pointer = unsafe {
    let handle = libc::dlopen(library.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL);
    if handle.is_null() {
      return None;
    }
    libc::dlsym(handle, name.as_ptr())
  };
  if pointer.is_null() || size_of::<F>() != size_of::<*mut c_void>() {
    return None;
  }
  // SAFETY: the caller guarantees `F` is the symbol's function type; sizes were checked.
  Some(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&pointer) })
}

fn api() -> Option<&'static Api> {
  static API: OnceLock<Option<Api>> = OnceLock::new();
  API.get_or_init(|| {
    // SAFETY: each field's type is its symbol's signature on macOS 26 and 27 (sources in the module comment).
    unsafe {
      Some(Api {
        post_to_pid: function(SKYLIGHT, c"SLEventPostToPid")?,
        set_window_location: function(SKYLIGHT, c"CGEventSetWindowLocation")?,
        post_record: function(SKYLIGHT, c"SLPSPostEventRecordTo")?,
        process_for_pid: function(APPLICATION_SERVICES, c"GetProcessForPID")?,
        main_connection: function(SKYLIGHT, c"SLSMainConnectionID")?,
        capture_windows: function(SKYLIGHT, c"SLSHWCaptureWindowList")?,
      })
    }
  })
  .as_ref()
}

const UNAVAILABLE: &str = "this macOS version lacks the window server calls background input needs";

/// Posts a Core Graphics event to one process through the window server, which marks it as live input (Chromium
/// drops events posted with the public call). The event never enters the user's input stream.
pub fn post_to_pid(pid: i32, event: *mut c_void) -> Result<(), String> {
  let api = api().ok_or(UNAVAILABLE)?;
  // SAFETY: `event` is a live CGEvent for the call.
  unsafe { (api.post_to_pid)(pid, event) };
  Ok(())
}

/// Stamps where in its window a pointer event lands: a window-local point on macOS 27, a screen point before.
pub fn set_window_location(event: *mut c_void, x: f64, y: f64) -> Result<(), String> {
  let api = api().ok_or(UNAVAILABLE)?;
  // SAFETY: `event` is a live CGEvent for the call.
  unsafe { (api.set_window_location)(event, x, y) };
  Ok(())
}

/// While this lives, the target app believes it is active with `window` as its key window. The window server's front
/// app, the z-order and the Space do not change, and the user's app never resigns key or active.
pub struct SyntheticFocus {
  psn: [u8; 8],
  window: u32,
}

impl SyntheticFocus {
  pub fn begin(pid: i32, window: u32) -> Result<SyntheticFocus, String> {
    let api = api().ok_or(UNAVAILABLE)?;
    let mut psn = [0u8; 8];
    // SAFETY: `psn` has room for a ProcessSerialNumber.
    let status = unsafe { (api.process_for_pid)(pid, psn.as_mut_ptr()) };
    if status != 0 {
      return Err(format!("the window server does not know process {pid} ({status})"));
    }
    let focus = SyntheticFocus { psn, window };
    focus.post(&activation(window, true))?;
    Ok(focus)
  }

  /// Makes the window key inside the app (yabai's record pair). Only works after `begin`.
  pub fn make_key(&self) -> Result<(), String> {
    let mut record = [0u8; RECORD];
    record[0x04] = 0xF8;
    record[0x3A] = 0x10;
    record[0x3C..0x40].copy_from_slice(&self.window.to_le_bytes());
    // Where the pair's pointer event lands: not a number, on purpose (yabai's bytes). With a real point, even one
    // outside the window, Chromium takes the pair for a click and brings itself to the front.
    record[0x20..0x30].fill(0xFF);
    for kind in [0x01, 0x02] {
      record[0x08] = kind;
      self.post(&record)?;
    }
    Ok(())
  }

  fn post(&self, record: &[u8; RECORD]) -> Result<(), String> {
    let api = api().ok_or(UNAVAILABLE)?;
    // SAFETY: both buffers live for the call; the record has the size the window server reads.
    let status = unsafe { (api.post_record)(self.psn.as_ptr(), record.as_ptr()) };
    if status == 0 { Ok(()) } else { Err(format!("the window server refused a focus record ({status})")) }
  }
}

impl Drop for SyntheticFocus {
  fn drop(&mut self) {
    // The app must never be left believing it is active.
    let _ = self.post(&activation(self.window, false));
  }
}

fn activation(window: u32, focused: bool) -> [u8; RECORD] {
  let mut record = [0u8; RECORD];
  record[0x04] = 0xF8;
  record[0x08] = 0x0D;
  record[0x3C..0x40].copy_from_slice(&window.to_le_bytes());
  record[0x8A] = if focused { 0x01 } else { 0x02 };
  record
}

/// A +1 CGImage of the window's backing store, covered parts included, or `None` when the window server has no
/// picture (minimized, closed, or Screen Recording is off).
pub fn capture_window(window: u32) -> Option<*mut c_void> {
  let api = api()?;
  // 0x800: ignore the global clip shape, so parts under other windows are included.
  // SAFETY: one valid window id; the result is a +1 CFArray or null.
  let array = unsafe { (api.capture_windows)((api.main_connection)(), &window, 1, 0x800) };
  if array.is_null() {
    return None;
  }
  // SAFETY: a live CFArray of CGImages; the first is retained before the array is released.
  unsafe {
    let image = if CFArrayGetCount(array) > 0 { CFArrayGetValueAtIndex(array, 0) } else { std::ptr::null_mut() };
    if !image.is_null() {
      CFRetain(image);
    }
    CFRelease(array);
    (!image.is_null()).then_some(image)
  }
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
  fn CFArrayGetCount(array: *mut c_void) -> isize;
  fn CFArrayGetValueAtIndex(array: *mut c_void, index: isize) -> *mut c_void;
  fn CFRetain(object: *mut c_void) -> *mut c_void;
  fn CFRelease(object: *mut c_void);
}

/// The major macOS version, such as 27; 0 when it cannot be read.
pub fn macos_major() -> u32 {
  static MAJOR: OnceLock<u32> = OnceLock::new();
  *MAJOR.get_or_init(|| {
    let mut buffer = [0u8; 32];
    let mut length = buffer.len();
    // SAFETY: valid name, buffer and length pointers.
    let status = unsafe {
      libc::sysctlbyname(c"kern.osproductversion".as_ptr(), buffer.as_mut_ptr().cast(), &mut length, std::ptr::null_mut(), 0)
    };
    if status != 0 {
      return 0;
    }
    let text = String::from_utf8_lossy(buffer.get(..length.saturating_sub(1)).unwrap_or(&[]));
    text.split('.').next().and_then(|major| major.parse().ok()).unwrap_or(0)
  })
}
