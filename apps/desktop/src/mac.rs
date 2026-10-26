//! macOS pieces Tauri does not cover: WebKit's frame-rate switch, rendering while covered (smoke and bench runs), the
//! title bar's double-click setting, the pasteboard, responder-chain edit actions and native notifications. Private
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
  let _ = window.with_webview(move |wv| unsafe {
    let wk = wv.inner() as *mut AnyObject;
    let config: *mut AnyObject = msg_send![wk, configuration];
    let prefs: *mut AnyObject = msg_send![config, preferences];
    let Some(cls) = AnyClass::get(c"WKPreferences") else {
      log("frame rate: no WKPreferences, left at WebKit's default");
      return;
    };
    let has_features: Bool = msg_send![cls, respondsToSelector: Sel::register(c"_features")];
    let has_set: Bool = msg_send![prefs, respondsToSelector: Sel::register(c"_setEnabled:forFeature:")];
    if !has_features.as_bool() || !has_set.as_bool() {
      log("frame rate: WKPreferences _features / _setEnabled:forFeature: missing, left at WebKit's default");
      return;
    }
    let features: *mut AnyObject = msg_send![cls, _features];
    let count: usize = msg_send![features, count];
    for i in 0..count {
      let f: *mut AnyObject = msg_send![features, objectAtIndex: i];
      let key: *mut AnyObject = msg_send![f, key];
      let utf8: *const c_char = msg_send![key, UTF8String];
      if !utf8.is_null() && CStr::from_ptr(utf8).to_bytes() == b"PreferPageRenderingUpdatesNear60FPSEnabled" {
        let _: () = msg_send![prefs, _setEnabled: Bool::NO, forFeature: f];
        return;
      }
    }
    log("frame rate: PreferPageRenderingUpdatesNear60FPSEnabled is not among WebKit's features, left at the default");
  });
}

/// Smoke and bench runs: the window keeps rendering while other windows cover it and App Nap leaves the app's timers
/// alone, so a run in the background never needs the window in front of whoever uses the Mac
pub fn keep_rendering_in_background(window: &tauri::WebviewWindow, log: Log) {
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
  unsafe {
    let Some(cls) = AnyClass::get(c"NSUserDefaults") else { return TitleDoubleClick::Zoom };
    let defaults: *mut AnyObject = msg_send![cls, standardUserDefaults];
    let key = NSString::from_str("AppleActionOnDoubleClick");
    let value: *mut AnyObject = msg_send![defaults, stringForKey: &*key];
    if value.is_null() {
      return TitleDoubleClick::Zoom;
    }
    let utf8: *const c_char = msg_send![value, UTF8String];
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
  Some(unsafe { msg_send![ns, windowNumber] })
}

fn pasteboard() -> Option<*mut AnyObject> {
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
  unsafe {
    let ty = NSString::from_str(PASTEBOARD_STRING);
    let s: *mut AnyObject = msg_send![pb, stringForType: &*ty];
    ns_string(s).unwrap_or_default()
  }
}

fn ns_string(s: *mut AnyObject) -> Option<String> {
  if s.is_null() {
    return None;
  }
  let utf8: *const c_char = unsafe { msg_send![s, UTF8String] };
  (!utf8.is_null()).then(|| unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned())
}

/// The file paths of the drag in progress (or the last one), from the system's drag pasteboard: WKWebView gives the page
/// a dropped file's name and bytes but not where it lives, which a folder dropped on the session column needs. Main
/// thread only
pub fn drag_paths() -> Vec<String> {
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
  unsafe {
    let Some(cls) = AnyClass::get(c"NSApplication") else { return };
    let app: *mut AnyObject = msg_send![cls, sharedApplication];
    let none: *mut AnyObject = std::ptr::null_mut();
    let _: Bool = msg_send![app, sendAction: Sel::register(selector), to: none, from: none];
  }
}

/// Whether the process runs from an app bundle: UNUserNotificationCenter aborts the process without one (a `cargo run`)
pub fn bundled() -> bool {
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

// The center and the delegate are only touched on the main thread (the shell dispatches there)
unsafe impl Send for Notifier {}
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
