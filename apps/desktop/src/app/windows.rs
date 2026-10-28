//! Chat windows: opening one (placement, title bar per platform, its webview's setup), its page's life (attached,
//! reloaded, its content process ended), closing, and the placements saved for the next launch

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use acpira_desktop::client::ShellView;
use acpira_desktop::window_state::{Bounds, Placement, WindowEntry, placeable};
use acpira_desktop::wire::Initial;
use serde_json::json;
use tauri::{WebviewWindow, WebviewWindowBuilder};

use super::platform::open_outside;
use super::{App, ChatWindowOpen, allowed_navigation, app, first_url, log, log_arc};
use crate::browser::BrowserPanelWindow;
use crate::chat_window::{ChatWindow, page_url};

// Only the macOS content-process recovery times its repeats
#[cfg(target_os = "macos")]
use std::time::Instant;

impl App {
  pub fn windows(&self) -> Vec<Arc<ChatWindow>> {
    self.windows.lock().clone()
  }

  pub fn window_by_label(&self, label: &str) -> Option<Arc<ChatWindow>> {
    self.windows.lock().iter().find(|w| w.label == label).cloned()
  }

  pub fn browser_by_label(&self, label: &str) -> Option<Arc<BrowserPanelWindow>> {
    self.browsers.lock().iter().find(|w| w.label == label).cloned()
  }

  pub fn focused_window(&self) -> Option<Arc<ChatWindow>> {
    let last = self.last_focused.lock().clone();
    let windows = self.windows.lock();
    last.and_then(|l| windows.iter().find(|w| w.label == l).cloned()).or_else(|| windows.first().cloned())
  }

  pub fn on_focus(&self, w: &ChatWindow) {
    *self.last_focused.lock() = Some(w.label.clone());
    self.client().event(json!({ "type": "windowFocus" }));
  }

  /// The page has its init state: the live context slots go after
  pub fn on_page_ready(&self, w: &ChatWindow) {
    if let Some(u) = self.updater() {
      w.post(json!({ "type": "updateState", "state": u.state() }));
    }
    if let Some(menus) = self.page_menu.lock().clone() {
      w.post(json!({ "type": "desktopMenu", "menus": menus }));
      w.post(json!({ "type": "windowState", "maximized": w.window.is_maximized().unwrap_or(false) }));
    }
    if self.share_context() {
      for m in self.context.lock().replay() {
        w.post(m);
      }
    }
  }

  /// Where WebView2 keeps its profile: Tauri's default (%LOCALAPPDATA%\<identifier>) unless a smoke run gave the app a
  /// profile of its own; every webview of the process has to use the same one. WebKitGTK keeps its website data and its
  /// cache there too, under the cache folder: the page keeps nothing in web storage. WKWebView keeps its data in the
  /// system's store whatever this says (Tauri only keys the webviews' shared context by it)
  pub(super) fn webview_data(&self) -> PathBuf {
    self.paths.webview_data.clone().unwrap_or_else(|| {
      if cfg!(windows) {
        self.paths.logs.parent().map(Path::to_path_buf).unwrap_or_default()
      } else if cfg!(target_os = "linux") {
        self.paths.cache.join("webview")
      } else {
        self.paths.user_data.clone()
      }
    })
  }

  /// WebView2's browser arguments (the same for every webview of the process): wry's own (no mini menu, no SmartScreen
  /// check of the app's pages) and, in a background run, rendering that goes on while other windows cover the app's
  pub(super) fn browser_args(&self) -> String {
    let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    if self.background {
      args.push_str(",CalculateNativeWinOcclusion --disable-backgrounding-occluded-windows --disable-renderer-backgrounding");
      args.push_str(" --disable-background-timer-throttling");
    }
    args
  }

  pub(super) fn dark(&self) -> bool {
    crate::os::dark_mode()
  }

  pub(super) fn next_label(&self, prefix: &str) -> String {
    format!("{prefix}-{}", self.window_seq.fetch_add(1, Ordering::SeqCst) + 1)
  }

  /// A new chat window on a session, the most recent one, or none (a fresh view)
  pub fn open(self: &Arc<Self>, initial: Option<Initial>, how: ChatWindowOpen) -> Option<Arc<ChatWindow>> {
    let label = self.next_label("chat");
    let mut b = WebviewWindowBuilder::new(&self.handle, &label, first_url("editor"))
      .title("Acpira")
      .inner_size(1180.0, 820.0)
      .min_inner_size(420.0, 360.0)
      .visible(false)
      .background_color(if self.dark() {
        tauri::window::Color(0x18, 0x18, 0x18, 0xff)
      } else {
        tauri::window::Color(0xf8, 0xf8, 0xf8, 0xff)
      })
      .initialization_script(crate::INIT_SCRIPT.as_str())
      .data_directory(self.webview_data())
      .additional_browser_args(&self.browser_args())
      // The page's own drag and drop: Tauri's handler would take every drag (Finder files, the session list's reordering)
      // away from the DOM; the page reads a dropped file itself and asks for its path (acpira_drag_paths)
      .disable_drag_drop_handler()
      .on_navigation(allowed_navigation)
      .on_new_window(|url, _| {
        open_outside(url.as_str());
        tauri::webview::NewWindowResponse::Deny
      });
    match how.place {
      Some(Placement::At(r)) => b = b.position(r.x, r.y).inner_size(r.width, r.height),
      Some(Placement::Size { width, height }) => b = b.inner_size(width, height),
      None => {}
    }
    #[cfg(target_os = "macos")]
    {
      // No title bar: the page's column title row and sheet header are the title bar (data-drag-region) and leave the
      // traffic lights their room (desktop.css --lights-w). Tauri measures the lights 9 pt lower than Electron's
      // trafficLightPosition, so (17, 32) here is Electron's (17, 23): the lights' centre on the header row's centre line
      b =
        b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true).traffic_light_position(tauri::LogicalPosition::new(17.0, 32.0));
    }
    // No system title bar either: the page draws the caption's buttons and the menu (chat/DesktopFrame.tsx WindowCaption,
    // AppMenus). The system still gives the undecorated window its shadow, rounded corners and resize edges, and the
    // title row's drag and double click are the system's own (init.js)
    #[cfg(windows)]
    {
      b = b.decorations(false).shadow(true);
    }
    // Linux the same way: Tauri's runtime resizes an undecorated GTK window from its edges, the title row moves it
    // (init.js); there is no system shadow around it
    #[cfg(target_os = "linux")]
    {
      b = b.decorations(false);
    }
    let window = match b.build() {
      Ok(w) => w,
      Err(e) => {
        log(&format!("window failed: {e}"));
        return None;
      }
    };
    self.prepare(&window);
    let w = Arc::new(ChatWindow::new(window.clone(), initial, how.watch));
    self.windows.lock().push(w.clone());
    {
      let mut last = self.last_focused.lock();
      if last.is_none() {
        *last = Some(label.clone());
      }
    }
    if how.maximized {
      let _ = window.maximize();
    }
    // Attach before the page loads: its `ready` must find the view
    self.client().attach(w.clone() as Arc<dyn ShellView>);
    if cfg!(target_os = "macos") {
      w.load();
    }
    self.schedule_save();
    Some(w)
  }

  /// What every page's webview gets before its first real page: the frame-rate switch, background rendering for a
  /// smoke run, on Windows the clipboard and dropped paths (win::hook_webview), on Linux a drag's paths
  /// (linux::hook_webview)
  pub(super) fn prepare(&self, window: &WebviewWindow) {
    crate::os::unlock_frame_rate(window, log_arc());
    #[cfg(windows)]
    crate::win::hook_webview(window, log_arc());
    #[cfg(target_os = "linux")]
    crate::linux::hook_webview(window, log_arc());
    if self.background {
      crate::os::keep_rendering_in_background(window, log_arc());
    }
  }

  /// The page listens: the window shows (the Electron app's ready-to-show), without the focus in a background run
  pub fn page_attached(&self, label: &str) {
    let window = self.window_by_label(label).map(|w| w.window.clone()).or_else(|| self.browser_by_label(label).map(|b| b.window.clone()));
    let Some(window) = window else { return };
    if window.is_visible().unwrap_or(true) {
      return;
    }
    if self.background {
      crate::os::order_back(&window);
    } else if self.browser_by_label(label).is_some_and(|b| !b.focus_on_show()) {
      crate::os::order_front(&window);
    } else {
      let _ = window.show();
      let _ = window.set_focus();
    }
  }

  /// View › Reload: the page starts over on the app's own page
  pub fn reload(&self, label: &str) {
    if let Some(w) = self.window_by_label(label) {
      w.load();
    } else if let Some(b) = self.browser_by_label(label) {
      b.transport.detach();
      let _ = b.window.navigate(page_url("browser"));
    }
  }

  /// macOS: a page's WebContent process ended, which leaves the window blank. Besides a crash, WebKit's UI process
  /// kills it when a layer tree commit fails its checks: on macOS 27.0.1 a long session failed
  /// `RemoteScrollingCoordinatorProxy::establishLayerTreeScrollingRelations`, and since the reloaded page came back the
  /// same, every popover or settings page opened after it killed the process again. The first end reloads the page (what
  /// wry did by default); another within PAGE_END_REPEAT turns async overflow scrolling off for that webview first
  /// (mac.rs sync_overflow_scrolling), which takes the overflow nodes and that check out of the page, at the cost of
  /// overflow scrolling on the page's main thread until the window closes
  #[cfg(target_os = "macos")]
  pub fn page_process_ended(&self, label: &str) {
    const PAGE_END_REPEAT: Duration = Duration::from_secs(300);
    let now = Instant::now();
    let again = self.page_ended.lock().insert(label.to_owned(), now).is_some_and(|t| now.duration_since(t) < PAGE_END_REPEAT);
    let window = self.window_by_label(label).map(|w| w.window.clone()).or_else(|| self.browser_by_label(label).map(|b| b.window.clone()));
    let Some(window) = window else { return };
    if again {
      log(&format!("page {label}: WebContent process ended again; reloading with async overflow scrolling off"));
      crate::mac::sync_overflow_scrolling(&window, log_arc());
    } else {
      log(&format!("page {label}: WebContent process ended; reloading"));
    }
    self.reload(label);
  }

  pub fn window_closed(&self, label: &str) {
    self.page_ended.lock().remove(label);
    if let Some(b) = self.browser_by_label(label) {
      b.relay.dispose();
      self.browsers.lock().retain(|x| x.label != label);
      return;
    }
    let Some(w) = self.window_by_label(label) else { return };
    self.client().detach(&w.view_id);
    w.dispose();
    self.windows.lock().retain(|x| x.label != label);
    {
      let mut last = self.last_focused.lock();
      if last.as_deref() == Some(label) {
        *last = None;
      }
    }
    // A window closed by hand lends its place to the next new one; at quit every window stays in the list
    let entry = self.known.lock().remove(label);
    if !self.quitting.load(Ordering::SeqCst)
      && let Some(e) = entry
    {
      self.windows_state.lock().last = Some(e);
    }
    self.save_windows();
  }

  /// The freshest entry when a window is about to close (the close button, ⌘W)
  pub fn window_closing(&self, label: &str) {
    if self.quitting.load(Ordering::SeqCst) {
      return;
    }
    if let Some(w) = self.window_by_label(label)
      && let Some(e) = self.entry_of(&w)
    {
      self.known.lock().insert(label.to_owned(), e);
    }
  }

  fn entry_of(&self, w: &ChatWindow) -> Option<WindowEntry> {
    let win = &w.window;
    let maximized = win.is_maximized().unwrap_or(false);
    let scale = win.scale_factor().ok()?;
    let bounds = if maximized || win.is_fullscreen().unwrap_or(false) || win.is_minimized().unwrap_or(false) {
      // Not the normal bounds: keep the ones saved before
      self.known.lock().get(&w.label).and_then(|e| e.bounds)
    } else {
      let pos = win.outer_position().ok()?.to_logical::<f64>(scale);
      let size = win.inner_size().ok()?.to_logical::<f64>(scale);
      Some(Bounds { x: pos.x, y: pos.y, width: size.width, height: size.height })
    };
    Some(WindowEntry { bounds, maximized, session: w.session_id() })
  }

  pub fn save_windows(&self) {
    if self.quitting.load(Ordering::SeqCst) {
      return;
    }
    let live = self.windows();
    let entries: Vec<WindowEntry> = live
      .iter()
      .filter_map(|w| {
        let e = self.entry_of(w)?;
        self.known.lock().insert(w.label.clone(), e.clone());
        Some(e)
      })
      .collect();
    let mut state = self.windows_state.lock();
    state.windows = entries;
    self.window_store.write(&state);
  }

  /// Moved, resized or on another session: saved once things settle
  pub fn schedule_save(&self) {
    let seq = self.save_seq.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
      tokio::time::sleep(Duration::from_millis(500)).await;
      let a = app();
      if a.save_seq.load(Ordering::SeqCst) == seq {
        let h = a.handle.clone();
        let _ = h.run_on_main_thread(|| app().save_windows());
      }
    });
  }

  pub fn place_of(&self, e: Option<&WindowEntry>) -> ChatWindowOpen {
    let Some(e) = e else { return ChatWindowOpen::default() };
    let areas: Vec<Bounds> = self
      .handle
      .available_monitors()
      .unwrap_or_default()
      .iter()
      .map(|m| {
        let s = m.scale_factor();
        let a = m.work_area();
        Bounds {
          x: f64::from(a.position.x) / s,
          y: f64::from(a.position.y) / s,
          width: f64::from(a.size.width) / s,
          height: f64::from(a.size.height) / s,
        }
      })
      .collect();
    ChatWindowOpen { place: e.bounds.map(|b| placeable(b, &areas)), maximized: e.maximized, watch: None }
  }

  pub(super) fn place_of_last(&self) -> ChatWindowOpen {
    let last = self.windows_state.lock().last.clone();
    self.place_of(last.as_ref())
  }

  /// The windows open at the last quit come back; links that launched the app show their sessions after that
  pub fn restore(self: &Arc<Self>) {
    let state = self.windows_state.lock().clone();
    let restore = if state.windows.is_empty() { vec![state.last.clone().unwrap_or_default()] } else { state.windows.clone() };
    for e in &restore {
      let initial = Some(e.session.clone().map(Initial::Session).unwrap_or(Initial::MostRecent));
      self.open(initial, self.place_of(Some(e)));
    }
    let links = self.pending_links.lock().take().unwrap_or_default();
    for l in links {
      self.take_link(&l);
    }
  }

  /// A window was resized: its page shows the caption's maximize or restore glyph (Windows)
  pub fn window_resized(&self, label: &str) {
    if self.page_menu.lock().is_none() {
      return;
    }
    if let Some(w) = self.window_by_label(label) {
      let maximized = w.window.is_maximized().unwrap_or(false);
      if w.maximized.swap(maximized, std::sync::atomic::Ordering::SeqCst) != maximized {
        w.post(json!({ "type": "windowState", "maximized": maximized }));
      }
    }
  }
}
