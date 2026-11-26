//! Listens to one app's accessibility notifications: an observer on its own run loop thread stamps each one
//! (`crate::settle`).

use super::ax::{self, AXObserverRef, AXUIElementRef, Elem};
use crate::settle::Stamp;
use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;
use std::sync::Arc;
use std::time::Duration;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
  static kCFRunLoopDefaultMode: CFStringRef;
  fn CFRunLoopGetCurrent() -> *mut c_void;
  fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: CFStringRef);
  fn CFRunLoopRun();
  fn CFRunLoopStop(run_loop: *mut c_void);
  fn CFRelease(object: *mut c_void);
}

/// Notifications that mean "the interface changed". Registered on the app element, which reports them for every
/// element of the app.
const NOTIFICATIONS: &[&str] = &[
  "AXValueChanged",
  "AXTitleChanged",
  "AXUIElementDestroyed",
  "AXCreated",
  "AXWindowCreated",
  "AXSheetCreated",
  "AXDrawerCreated",
  "AXMenuOpened",
  "AXMenuClosed",
  "AXFocusedUIElementChanged",
  "AXFocusedWindowChanged",
  "AXMainWindowChanged",
  "AXSelectedChildrenChanged",
  "AXSelectedTextChanged",
  "AXSelectedRowsChanged",
  "AXLayoutChanged",
  "AXRowCountChanged",
  "AXElementBusyChanged",
  // Web content (WebKit and Chromium).
  "AXLoadComplete",
  "AXLiveRegionChanged",
];

/// Observes one app until dropped.
pub struct Watch {
  pub stamp: Arc<Stamp>,
  /// The observer thread's run loop, as an address; 0 when the observer could not start.
  run_loop: usize,
}

extern "C" fn on_notification(_: AXObserverRef, _: AXUIElementRef, _: CFStringRef, refcon: *mut c_void) {
  // SAFETY: `refcon` is the `Arc<Stamp>` leaked in `Watch::start`, which the observer thread drops only after it has
  // released the observer.
  let stamp = unsafe { &*(refcon as *const Stamp) };
  stamp.hit();
}

impl Watch {
  /// Starts observing `pid`.
  pub fn start(pid: i32, app: &Elem) -> Watch {
    let stamp = Arc::new(Stamp::default());
    let refcon = Arc::into_raw(stamp.clone()) as usize;
    let app = app.clone();
    let (ready, wait) = std::sync::mpsc::channel::<usize>();
    let spawned = std::thread::Builder::new().name(format!("ax-watch-{pid}")).spawn(move || {
      // SAFETY: `refcon` came from `Arc::into_raw` above and is turned back exactly once, on every way out.
      let release = move || drop(unsafe { Arc::from_raw(refcon as *const Stamp) });
      let mut observer: AXObserverRef = std::ptr::null();
      // SAFETY: the callback is a plain function; `observer` receives +1 on success.
      if unsafe { ax::AXObserverCreate(pid, on_notification, &mut observer) } != ax::AX_OK {
        release();
        let _ = ready.send(0);
        return;
      }
      for name in NOTIFICATIONS {
        let name = CFString::from_static_string(name);
        // SAFETY: live observer and element; `refcon` outlives the observer. An app that lacks one notification just
        // refuses it.
        unsafe {
          ax::AXObserverAddNotification(observer, app.raw(), name.as_concrete_TypeRef(), refcon as *mut c_void);
        }
      }
      // SAFETY: the source belongs to the live observer; this thread owns its run loop, which runs until the watch
      // is dropped. The observer is released before the stamp its callback reads.
      unsafe {
        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, ax::AXObserverGetRunLoopSource(observer), kCFRunLoopDefaultMode);
        let _ = ready.send(run_loop as usize);
        CFRunLoopRun();
        CFRelease(observer.cast_mut().cast());
      }
      release();
    });
    // Without the thread nothing is ever seen, and every wait ends after its `first` window.
    let run_loop = match spawned {
      Ok(_) => wait.recv_timeout(Duration::from_millis(500)).unwrap_or(0),
      Err(_) => 0,
    };
    Watch { stamp, run_loop }
  }
}

impl Drop for Watch {
  fn drop(&mut self) {
    if self.run_loop != 0 {
      // SAFETY: the address is the observer thread's run loop, which lives until that thread ends, and the thread
      // ends only after this stop.
      unsafe { CFRunLoopStop(self.run_loop as *mut c_void) };
    }
  }
}
