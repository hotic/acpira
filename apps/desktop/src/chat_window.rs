//! One chat window: a WebviewWindow running the webview bundle with host 'editor', and its own view in the engine. The
//! relay mirrors the VS Code shell's WebviewBridge (apps/vscode/src/bridge.ts): page
//! messages go to the view, host messages to the page, and a page that already initialized starts over when the engine
//! comes back

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use acpira_desktop::browser::{answering, normalize_preview_url, origin_of};
use acpira_desktop::client::{ShellView, SidecarState};
use acpira_desktop::page::APP_ORIGIN;
use acpira_desktop::wire::{HostMessage, Initial};
use parking_lot::Mutex;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tauri::WebviewWindow;

use crate::browser::BrowserRelay;
use crate::page::Transport;

/// Watches a window's session views (Connect ChatGPT copies the mirror's connection prompt once it shows); true when done
pub type Watch = Box<dyn FnMut(&HostMessage) -> bool + Send>;

pub struct ChatWindow {
  pub view_id: String,
  pub label: String,
  pub window: WebviewWindow,
  pub transport: Transport,
  initial: Option<Initial>,
  initialized: AtomicBool,
  /// The page received init since it last loaded: shell messages before that would find no listener and wait instead
  page_ready: AtomicBool,
  shell_queue: Mutex<Vec<String>>,
  /// The session the page shows, as last pushed
  shown: Mutex<Option<String>>,
  watch: Mutex<Option<Watch>>,
  /// The tool panel's browser tabs by tab id; their pages are frames of this window's page, so a reload ends them all
  tabs: Mutex<HashMap<String, Arc<BrowserRelay>>>,
  /// Maximized as the page's caption last heard (App::window_resized)
  pub maximized: AtomicBool,
}

#[expect(clippy::expect_used, reason = "APP_ORIGIN is a constant absolute URL and every host passed is one of the shell's own names")]
pub fn page_url(host: &str) -> tauri::Url {
  format!("{APP_ORIGIN}/index.html?host={host}").parse().expect("the app page URL parses")
}

impl ChatWindow {
  pub fn new(window: WebviewWindow, initial: Option<Initial>, watch: Option<Watch>) -> ChatWindow {
    ChatWindow {
      view_id: uuid::Uuid::new_v4().to_string(),
      label: window.label().to_owned(),
      window,
      transport: Transport::default(),
      initial,
      initialized: AtomicBool::new(false),
      page_ready: AtomicBool::new(false),
      maximized: AtomicBool::new(false),
      shell_queue: Mutex::new(vec![]),
      shown: Mutex::new(None),
      watch: Mutex::new(watch),
      tabs: Mutex::new(HashMap::new()),
    }
  }

  pub fn session_id(&self) -> Option<String> {
    self.shown.lock().clone()
  }

  /// True once the page has its init state (a chime or a toast can show now)
  pub fn ready(&self) -> bool {
    self.page_ready.load(Ordering::SeqCst)
  }

  /// A navigation of the window's page started: the old document, its shell queue and its browser tabs are gone
  pub fn on_load_started(&self) {
    self.page_ready.store(false, Ordering::SeqCst);
    self.transport.detach();
    self.close_tabs();
  }

  pub fn load(&self) {
    self.on_load_started();
    let _ = self.window.navigate(page_url("editor"));
  }

  /// From the page. viewFocus stays in the shell, like the VS Code bridge, and so do the folder dialog, the updater and
  /// the tool panel's browser tabs
  pub fn on_page_message(self: &Arc<Self>, message: Box<RawValue>) {
    let kind = acpira_desktop::wire::peek(message.get()).0;
    let app = crate::app::app();
    match kind.as_str() {
      "viewFocus" => app.on_focus(self),
      "pickFolder" => crate::app::pick_folder(self.clone()),
      // The updater stays in the shell too (About, the restart toast)
      "updateCheck" | "updateInstall" | "updateAuto" => {
        let v: Value = serde_json::from_str(message.get()).unwrap_or_default();
        match kind.as_str() {
          "updateCheck" => app.check_updates(v["download"].as_bool() == Some(true)),
          "updateAuto" => app.set_update_auto(v["on"].as_bool() != Some(false)),
          _ => app.restart_to_update(),
        }
      }
      // The menu and the caption the page draws where the window has no system title bar (Windows). This runs inside
      // WebView2's message callback on the main thread, where run_on_main_thread would run at once, and a window built
      // or closed there hangs the webview: queued from another thread, it runs once the callback has returned
      "menuCommand" | "windowControl" => {
        let v: Value = serde_json::from_str(message.get()).unwrap_or_default();
        let window = self.window.clone();
        let (kind, arg) = (kind.clone(), v[if kind == "menuCommand" { "id" } else { "op" }].as_str().unwrap_or_default().to_owned());
        std::thread::spawn(move || {
          let w = window.clone();
          let _ = window.run_on_main_thread(move || {
            if kind == "menuCommand" { crate::menu::on_page_command(&w, &arg) } else { crate::menu::window_control(&w, &arg) }
          });
        });
      }
      "panelBrowserOpen" | "panelBrowser" | "panelBrowserClose" | "panelHistory" => {
        if let Ok(v) = serde_json::from_str::<Value>(message.get()) {
          self.panel_message(&kind, v);
        }
      }
      _ => app.client().send(&self.view_id, message),
    }
  }

  fn panel_message(self: &Arc<Self>, kind: &str, m: Value) {
    let tab = m["tab"].as_str().unwrap_or_default().to_owned();
    match kind {
      "panelBrowserOpen" => {
        let folder = m["folder"].as_str().map(str::to_owned);
        let relay = self.tab(&tab);
        let url = m["url"].as_str().and_then(normalize_preview_url);
        // A tab moved to a session on another host shows the page there
        let moved = relay.folder() != folder;
        relay.set_folder(folder);
        let page = url.or_else(|| if moved { relay.target() } else { None });
        if let Some(page) = page
          && (Some(&page) != relay.target().as_ref() || moved)
        {
          relay.navigate(page);
        }
      }
      "panelBrowser" => {
        let relay = self.tabs.lock().get(&tab).cloned();
        if let Some(r) = relay {
          r.on_page_message(m["msg"].clone());
        }
      }
      "panelBrowserClose" => {
        let relay = self.tabs.lock().remove(&tab);
        if let Some(r) = relay {
          r.dispose();
        }
      }
      "panelHistory" => {
        let pages = crate::app::app().history.pages();
        let urls: Vec<String> = pages.iter().map(|p| p.url.clone()).filter(|u| normalize_preview_url(u).is_some()).collect();
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
          let running = answering(urls, std::time::Duration::from_millis(800)).await;
          me.post(json!({ "type": "panelHistory", "pages": pages, "running": running }));
        });
      }
      _ => {}
    }
  }

  fn tab(self: &Arc<Self>, tab: &str) -> Arc<BrowserRelay> {
    let mut tabs = self.tabs.lock();
    if let Some(r) = tabs.get(tab) {
      return r.clone();
    }
    let me = Arc::downgrade(self);
    let id = tab.to_owned();
    let relay = BrowserRelay::new(
      Box::new(move |msg| {
        if let Some(w) = me.upgrade() {
          w.post(json!({ "type": "panelBrowser", "tab": id, "msg": msg }));
        }
      }),
      None,
    );
    tabs.insert(tab.to_owned(), relay.clone());
    relay
  }

  fn close_tabs(&self) {
    let tabs: Vec<_> = self.tabs.lock().drain().map(|(_, r)| r).collect();
    for r in tabs {
      r.dispose();
    }
  }

  /// open_browser in this window's tool panel: the tab already on that site navigates (the page brings it forward), else
  /// the page opens a tab on it. What happened, as the engine reports it
  pub fn show_in_panel(&self, url: &str, folder: Option<&str>) -> Option<&'static str> {
    let origin = origin_of(url);
    let at = self
      .tabs
      .lock()
      .iter()
      .find(|(_, r)| r.target().and_then(|t| origin_of(&t)) == origin && r.folder().as_deref() == folder)
      .map(|(k, r)| (k.clone(), r.clone()));
    let Some((tab, relay)) = at else {
      self.post(json!({ "type": "panelOpenBrowser", "url": url }));
      return None;
    };
    let same = relay.target().as_deref() == Some(url);
    if !same {
      relay.navigate(url.to_owned());
    }
    self.post(json!({ "type": "panelOpenBrowser", "url": url, "tab": tab }));
    Some(if same { "revealed" } else { "navigated" })
  }

  pub fn post_browser_look(&self) {
    for r in self.tabs.lock().values() {
      r.post_look();
    }
  }

  pub fn post_browser_history(&self) {
    for r in self.tabs.lock().values() {
      r.post_history();
    }
  }

  /// Shell-originated messages (showSession, chime, toasts, menu commands) take the same way in, once the page listens
  pub fn post(&self, m: Value) {
    let json = m.to_string();
    if self.ready() {
      self.transport.send(json);
    } else {
      self.shell_queue.lock().push(json);
    }
  }

  /// Focused and on this session: the user is looking at it
  pub fn watching(&self, session_id: &str) -> bool {
    self.window.is_focused().unwrap_or(false) && self.session_id().as_deref() == Some(session_id)
  }

  pub fn set_theme(&self, dark: bool) {
    let _ = self.window.eval(format!("window.__acpiraTheme && window.__acpiraTheme({dark})"));
  }

  /// Brought up with the keyboard; a background run only makes sure it shows, behind everything
  pub fn reveal(&self) {
    if crate::app::app().background {
      crate::os::order_back(&self.window);
      return;
    }
    let _ = self.window.unminimize();
    let _ = self.window.show();
    let _ = self.window.set_focus();
  }

  pub fn dispose(&self) {
    self.close_tabs();
    self.transport.detach();
  }
}

impl ShellView for ChatWindow {
  fn view_id(&self) -> &str {
    &self.view_id
  }

  fn host(&self) -> &str {
    "editor"
  }

  fn initial(&self) -> Option<Initial> {
    self.initial.clone()
  }

  fn blob_base(&self) -> Option<String> {
    Some(format!("{APP_ORIGIN}/blobs"))
  }

  fn on_host_message(&self, m: HostMessage) {
    let before = self.session_id();
    if let Some(id) = &m.session {
      *self.shown.lock() = Some(id.clone());
    }
    if m.kind == "noSession" {
      *self.shown.lock() = None;
    }
    {
      let mut watch = self.watch.lock();
      if let Some(w) = watch.as_mut()
        && w(&m)
      {
        *watch = None;
      }
    }
    let init = m.kind == "init";
    self.transport.send(m.raw.get().to_owned());
    let app = crate::app::app();
    if self.session_id() != before {
      app.schedule_save();
    }
    if !init {
      return;
    }
    self.page_ready.store(true, Ordering::SeqCst);
    for q in self.shell_queue.lock().drain(..) {
      self.transport.send(q);
    }
    app.on_page_ready(self);
  }

  fn on_state(&self, state: SidecarState, _detail: Option<&str>) {
    if state != SidecarState::Ready {
      return;
    }
    if self.initialized.swap(true, Ordering::SeqCst) {
      self.load();
    }
  }
}
