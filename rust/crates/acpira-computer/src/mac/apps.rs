//! The running apps, and what the driver needs to know about one before acting (hidden, frontmost, Chromium-based).

use crate::apps::App;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::ffi::{CStr, c_char, c_void};

/// # Safety
/// `object` is null or an NSString.
unsafe fn string(object: *mut AnyObject) -> String {
  if object.is_null() {
    return String::new();
  }
  // SAFETY: `object` is an NSString; UTF8String is valid until the pool drains, and is copied at once.
  unsafe {
    let bytes: *const c_char = msg_send![object, UTF8String];
    if bytes.is_null() { String::new() } else { CStr::from_ptr(bytes).to_string_lossy().into_owned() }
  }
}

/// Every app with a user interface (regular or accessory activation policy), front to back as the workspace lists them.
pub fn running() -> Vec<App> {
  let mut apps = Vec::new();
  // SAFETY: AppKit calls with their declared signatures; NSWorkspace is safe to query from any thread.
  unsafe {
    let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
    let list: *mut AnyObject = msg_send![workspace, runningApplications];
    let count: usize = msg_send![list, count];
    for index in 0..count {
      let app: *mut AnyObject = msg_send![list, objectAtIndex: index];
      // 0 regular, 1 accessory, 2 prohibited (no interface at all).
      let policy: isize = msg_send![app, activationPolicy];
      if policy == 2 {
        continue;
      }
      let url: *mut AnyObject = msg_send![app, bundleURL];
      let path: *mut AnyObject = if url.is_null() { std::ptr::null_mut() } else { msg_send![url, path] };
      apps.push(App {
        pid: msg_send![app, processIdentifier],
        name: string(msg_send![app, localizedName]),
        bundle: string(msg_send![app, bundleIdentifier]),
        hidden: msg_send![app, isHidden],
        frontmost: msg_send![app, isActive],
        regular: policy == 0,
        path: string(path),
      });
    }
  }
  apps
}

/// Chromium and Electron apps insert typed text only into the window AppKit considers key, and publish their web
/// content to accessibility only when asked.
pub fn is_chromium(app: &App) -> bool {
  const KNOWN: &[&str] = &["com.google.chrome", "org.chromium.", "com.microsoft.edgemac", "com.brave.browser", "company.thebrowser.browser", "com.vivaldi.", "com.operasoftware."];
  let bundle = app.bundle.to_lowercase();
  if KNOWN.iter().any(|known| bundle.starts_with(known)) {
    return true;
  }
  let frameworks = std::path::Path::new(&app.path).join("Contents/Frameworks");
  frameworks.join("Electron Framework.framework").exists() || frameworks.join("Chromium Embedded Framework.framework").exists()
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
  fn CGSessionCopyCurrentDictionary() -> *const c_void;
  fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
}

/// Whether the window server is drawing the window on the current Space: not minimized, not hidden, not on another
/// Space. Covered by other windows still counts.
pub fn window_on_screen(window: u32) -> bool {
  /// `kCGWindowListOptionIncludingWindow`.
  const INCLUDING_WINDOW: u32 = 1 << 3;
  // SAFETY: the copy call returns a +1 array of dictionaries or null; lookups borrow from it.
  unsafe {
    let list = CGWindowListCopyWindowInfo(INCLUDING_WINDOW, window);
    if list.is_null() {
      return false;
    }
    let list: *mut AnyObject = list.cast_mut().cast();
    let count: usize = msg_send![list, count];
    let mut on_screen = false;
    if count > 0 {
      let entry: *mut AnyObject = msg_send![list, objectAtIndex: 0usize];
      let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c"kCGWindowIsOnscreen".as_ptr()];
      let value: *mut AnyObject = msg_send![entry, objectForKey: key];
      on_screen = if value.is_null() { false } else { msg_send![value, boolValue] };
    }
    let _: () = msg_send![list, release];
    on_screen
  }
}

/// Whether the login session is locked. A locked session hides every window from accessibility: an app then lists
/// itself as its only window.
pub fn screen_locked() -> bool {
  // SAFETY: the copy call returns a +1 dictionary or null; the lookup borrows from it.
  unsafe {
    let session = CGSessionCopyCurrentDictionary();
    if session.is_null() {
      return false;
    }
    let dictionary: *mut AnyObject = session.cast_mut().cast();
    let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c"CGSSessionScreenIsLocked".as_ptr()];
    let value: *mut AnyObject = msg_send![dictionary, objectForKey: key];
    let locked = if value.is_null() { false } else { msg_send![value, boolValue] };
    let _: () = msg_send![dictionary, release];
    locked
  }
}
