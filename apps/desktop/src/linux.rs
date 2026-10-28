//! Linux pieces Tauri does not cover, the counterpart of mac.rs and win.rs (see os.rs): the colour scheme (the XDG
//! desktop portal, then GTK's settings), the UI language from the locale variables, the clipboard, a drag's file paths,
//! showing a window without activating it, the title bar's double click (GNOME's setting) and notifications over D-Bus
//! (org.freedesktop.Notifications). WebKitGTK paces its frames to the display already, so there is no frame-rate switch

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use gtk::gio;
use gtk::glib::{self, ToVariant};
use gtk::prelude::*;
use parking_lot::Mutex;

use crate::os::{Log, TitleDoubleClick};

/// The name the packages install the app's desktop entry and icon under (tauri.conf.json's mainBinaryName): what a
/// notification names as its sender, so the desktop shows the app's name and icon on it
const DESKTOP_ENTRY: &str = "Acpira";

/// Wheel notches: WebKitGTK has a smooth-scrolling setting of its own (not measured here)
pub fn smooth_wheel(_log: Log) {}

/// WebKitGTK renders at the display's rate already
pub fn unlock_frame_rate(_window: &tauri::WebviewWindow, _log: Log) {}

/// Background smoke runs on Linux happen on a display of their own (Xvfb), where nothing covers the windows
pub fn keep_rendering_in_background(_window: &tauri::WebviewWindow, _log: Log) {}

/// What a double click on a title bar does: GNOME's setting where GNOME runs (its schema may be installed elsewhere
/// too, but other desktops keep their own setting), else zoom, every other desktop's default
pub fn title_double_click() -> TitleDoubleClick {
  const SCHEMA: &str = "org.gnome.desktop.wm.preferences";
  const KEY: &str = "action-double-click-titlebar";
  let gnome = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|d| d.eq_ignore_ascii_case("GNOME")));
  // gio::Settings::new aborts the process on a schema that is not installed: look it up first
  let schema = gnome.then(gio::SettingsSchemaSource::default).flatten().and_then(|s| s.lookup(SCHEMA, true));
  if !schema.is_some_and(|s| s.has_key(KEY)) {
    return TitleDoubleClick::Zoom;
  }
  match gio::Settings::new(SCHEMA).string(KEY).as_str() {
    "minimize" => TitleDoubleClick::Minimize,
    "none" | "lower" | "menu" => TitleDoubleClick::Nothing,
    _ => TitleDoubleClick::Zoom,
  }
}

/// The user's UI language as a BCP 47 tag, from the locale variables (acpira_desktop::freedesktop::ui_language)
pub fn preferred_language() -> Option<String> {
  acpira_desktop::freedesktop::ui_language(&|k| std::env::var(k).ok())
}

/// The scheme a main-thread read last found (0 unknown, 1 light, 2 dark): the app protocol's handler asks off the main
/// thread, where GTK must not be touched
static SCHEME: AtomicU8 = AtomicU8::new(0);
/// The portal did not answer once (none installed, or no session bus): it is not asked again
static NO_PORTAL: AtomicBool = AtomicBool::new(false);

/// Whether the desktop prefers dark: the portal's colour scheme (GNOME 42+, KDE Plasma 5.24+), else GTK's theme
pub fn dark_mode() -> bool {
  match portal_color_scheme() {
    Some(1) => return true,
    Some(2) => return false,
    _ => {}
  }
  if gtk::is_initialized_main_thread() {
    let dark = gtk_prefers_dark();
    SCHEME.store(if dark { 2 } else { 1 }, Ordering::Relaxed);
    return dark;
  }
  SCHEME.load(Ordering::Relaxed) == 2
}

/// org.freedesktop.appearance color-scheme: 1 prefers dark, 2 prefers light, 0 states no preference
fn portal_color_scheme() -> Option<u32> {
  if NO_PORTAL.load(Ordering::Relaxed) {
    return None;
  }
  let reply = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).and_then(|bus| {
    bus.call_sync(
      Some("org.freedesktop.portal.Desktop"),
      "/org/freedesktop/portal/desktop",
      "org.freedesktop.portal.Settings",
      "Read",
      Some(&("org.freedesktop.appearance".to_owned(), "color-scheme".to_owned()).to_variant()),
      None,
      gio::DBusCallFlags::NONE,
      1000,
      gio::Cancellable::NONE,
    )
  });
  let Ok(reply) = reply else {
    NO_PORTAL.store(true, Ordering::Relaxed);
    return None;
  };
  // (v): portals before 1.15 wrap the value twice. as_variant on a value that is not a variant logs a GLib critical
  let mut value = reply.try_child_value(0)?;
  while value.type_() == glib::VariantTy::VARIANT {
    value = value.as_variant()?;
  }
  value.get::<u32>()
}

/// Calls `changed` on the main loop when the desktop's colour scheme may have changed: the portal's SettingChanged for
/// org.freedesktop.appearance, and GTK's theme settings, which desktops without the portal's key change instead. tao
/// listens to the portal too, but sends its ThemeChanged for no window, and Tauri drops it (tao 0.37, tauri 2.12).
/// Main thread
pub fn watch_theme(changed: impl Fn() + 'static) {
  let changed = std::rc::Rc::new(changed);
  if let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
    let c = changed.clone();
    // arg0 is the setting's namespace
    bus.signal_subscribe(
      None,
      Some("org.freedesktop.portal.Settings"),
      Some("SettingChanged"),
      Some("/org/freedesktop/portal/desktop"),
      Some("org.freedesktop.appearance"),
      gio::DBusSignalFlags::NONE,
      move |_, _, _, _, _, params| {
        if params.try_child_value(1).and_then(|k| k.get::<String>()).as_deref() == Some("color-scheme") {
          // A portal that signals answers too, though it did not when first asked
          NO_PORTAL.store(false, Ordering::Relaxed);
          c();
        }
      },
    );
  }
  if let Some(settings) = gtk::Settings::default() {
    let c = changed.clone();
    settings.connect_gtk_theme_name_notify(move |_| c());
    settings.connect_gtk_application_prefer_dark_theme_notify(move |_| changed());
  }
}

/// GTK's own preference, or a theme named dark (Adwaita-dark, Breeze-Dark, Yaru-dark …), GTK_THEME's included. Main thread
fn gtk_prefers_dark() -> bool {
  if std::env::var("GTK_THEME").is_ok_and(|t| t.to_lowercase().contains("dark")) {
    return true;
  }
  let Some(settings) = gtk::Settings::default() else { return false };
  settings.property::<bool>("gtk-application-prefer-dark-theme")
    || settings.property::<Option<String>>("gtk-theme-name").is_some_and(|n| n.to_lowercase().contains("dark"))
}

/// Shows a window in front of the others without giving it the focus (an agent's open_browser)
pub fn order_front(window: &tauri::WebviewWindow) {
  show_without_focus(window, false);
}

/// Shows a window behind every other one (a background smoke run)
pub fn order_back(window: &tauri::WebviewWindow) {
  show_without_focus(window, true);
}

fn show_without_focus(window: &tauri::WebviewWindow, back: bool) {
  let w = window.clone();
  let _ = window.run_on_main_thread(move || {
    let Ok(gw) = w.gtk_window() else { return };
    gw.set_focus_on_map(false);
    // show_all, as tao shows a window: its webview was never shown either
    gw.show_all();
    if let Some(g) = gw.window() {
      if back { g.lower() } else { g.raise() }
    }
  });
}

/// Main thread only
pub fn clipboard_write(text: &str) {
  gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD).set_text(text);
}

/// A GIF on the clipboard: its URI as file managers offer a copied file (GNOME's own list too), plus its bytes as
/// image/gif. GTK hands each out on request while the app owns the clipboard. Main thread only
pub fn clipboard_write_gif(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
  let uri = glib::filename_to_uri(path, None).map_err(|e| e.to_string())?;
  let flags = gtk::TargetFlags::empty();
  let targets = [
    gtk::TargetEntry::new("text/uri-list", flags, 0),
    gtk::TargetEntry::new("x-special/gnome-copied-files", flags, 1),
    gtk::TargetEntry::new("image/gif", flags, 2),
  ];
  let data = data.to_vec();
  let set = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD).set_with_data(&targets, move |_, selection, info| match info {
    0 => {
      selection.set_uris(&[uri.as_str()]);
    }
    1 => selection.set(&selection.target(), 8, format!("copy\n{uri}").as_bytes()),
    _ => selection.set(&selection.target(), 8, &data),
  });
  if set { Ok(()) } else { Err("GTK refused the clipboard".into()) }
}

/// Main thread only: waits for the owner's answer in a nested main loop
pub fn clipboard_read() -> String {
  gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD).wait_for_text().map(|s| s.to_string()).unwrap_or_default()
}

/// The file paths of the last drag that brought a text/uri-list over one of the app's webviews (hook_webview)
static DRAG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The file paths of the drag in progress (or the last one), a folder's ending in '/'. WebKitGTK shows the page none of
/// a drag's files: no File objects, no "Files" type, and its text/uri-list reads empty for file: URLs (WebKit 2.50 keeps
/// file access to its Cocoa port, webkit.org/b/271957), so the page takes them from here (init.js's dragPaths): the
/// column adds the folders as projects, the composer attaches everything by path. Main thread
pub fn drag_paths() -> Vec<String> {
  DRAG.lock().clone()
}

/// What every webview of the app gets before its first page: the file paths of a drag (its text/uri-list), recorded as
/// WebKitGTK receives the drag's data, which it asks for before the page sees `dragenter`. A list of links only (a
/// browser's) clears them, so nothing of an earlier drag is left over; the other formats WebKit asks for leave them.
/// WebKit's own handler runs as before
pub fn hook_webview(window: &tauri::WebviewWindow, log: Log) {
  let hooked = window.with_webview(|wv| {
    wv.inner().connect_drag_data_received(|_, _, _, _, data, _, _| {
      let uris = data.uris();
      if uris.is_empty() {
        return;
      }
      *DRAG.lock() = uris
        .iter()
        .filter_map(|u| gio::File::for_uri(u).path())
        .map(|p| {
          let s = p.to_string_lossy().into_owned();
          if p.is_dir() && !s.ends_with('/') { format!("{s}/") } else { s }
        })
        .collect();
    });
  });
  if let Err(e) = hooked {
    log(&format!("webview hooks: {e}"));
  }
}

/// The window's content as a PNG (the smoke's screenshots), read back from the display server. Main thread
#[cfg(feature = "smoke")]
pub fn screenshot(window: &tauri::WebviewWindow, path: &str) -> Result<(), String> {
  let gw = window.gtk_window().map_err(|e| e.to_string())?;
  let g = gw.window().ok_or("the window is not realized")?;
  let pixbuf = g.pixbuf(0, 0, g.width(), g.height()).ok_or("the display server gave no pixels")?;
  pixbuf.savev(path, "png", &[]).map_err(|e| e.to_string())
}

/// Notifications through the desktop's notification server (org.freedesktop.Notifications: GNOME Shell, Plasma, dunst,
/// xfce4-notifyd …). A notification about a session carries the default action, which the server invokes on a click:
/// the session comes up
pub struct Notifier {
  bus: gio::DBusConnection,
  /// The notifications shown that name a session, by the server's id, until clicked or closed
  sessions: Arc<Mutex<HashMap<u32, String>>>,
  log: Log,
}

impl Notifier {
  /// None without a session bus. Any thread: the server's signals are delivered on the default main context, which the
  /// GTK main loop runs
  pub fn new(on_click: impl Fn(String) + Send + Sync + 'static, log: Log) -> Option<Notifier> {
    let bus = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
      Ok(b) => b,
      Err(e) => {
        log(&format!("notifications: no session bus ({e}), system notifications are off"));
        return None;
      }
    };
    let sessions: Arc<Mutex<HashMap<u32, String>>> = Arc::default();
    let clicked = sessions.clone();
    // Ids are unique across the server, so signals about other apps' notifications find nothing here
    bus.signal_subscribe(
      None,
      Some("org.freedesktop.Notifications"),
      Some("ActionInvoked"),
      Some("/org/freedesktop/Notifications"),
      None,
      gio::DBusSignalFlags::NONE,
      move |_, _, _, _, _, params| {
        let Some((id, action)) = params.get::<(u32, String)>() else { return };
        if action == "default"
          && let Some(session) = clicked.lock().remove(&id)
        {
          on_click(session);
        }
      },
    );
    let closed = sessions.clone();
    bus.signal_subscribe(
      None,
      Some("org.freedesktop.Notifications"),
      Some("NotificationClosed"),
      Some("/org/freedesktop/Notifications"),
      None,
      gio::DBusSignalFlags::NONE,
      move |_, _, _, _, _, params| {
        if let Some((id, _reason)) = params.get::<(u32, u32)>() {
          closed.lock().remove(&id);
        }
      },
    );
    Some(Notifier { bus, sessions, log })
  }

  /// `session`: clicking brings it up; None for a plain message (a toast with no window open). Any thread: the call goes
  /// out from the main loop, where gio delivers its answer (gio's async calls refuse a thread that does not own the
  /// main context, and the engine client's threads do not)
  pub fn show(&self, title: &str, body: &str, session: Option<&str>) {
    // The default action is the click on the notification itself; the empty label keeps it from showing as a button
    let actions: Vec<String> = if session.is_some() { vec!["default".into(), String::new()] } else { vec![] };
    let hints: HashMap<String, glib::Variant> = HashMap::from([("desktop-entry".to_owned(), DESKTOP_ENTRY.to_variant())]);
    let params = (DESKTOP_ENTRY.to_owned(), 0u32, DESKTOP_ENTRY.to_owned(), title.to_owned(), body.to_owned(), actions, hints, -1i32);
    let params = params.to_variant();
    let (bus, sessions, log, session) = (self.bus.clone(), self.sessions.clone(), self.log.clone(), session.map(str::to_owned));
    glib::MainContext::default().invoke(move || {
      bus.call(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "Notify",
        Some(&params),
        None,
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |reply| match reply {
          Ok(v) => {
            if let (Some(s), Some((id,))) = (session, v.get::<(u32,)>()) {
              sessions.lock().insert(id, s);
            }
          }
          Err(e) => log(&format!("notification failed: {e}")),
        },
      );
    });
  }
}
