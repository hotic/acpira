//! One browser page (the bundle with host 'browser', webview/browser/BrowserPanel.tsx) showing a loopback dev page through
//! the engine's preview proxy, which adds the element picker. It has no engine view; this side holds the proxy lease (the
//! client asks again after a reconnect). A window of its own (BrowserPanelWindow) or a tab of a chat window's tool panel
//! (ChatWindow) carries it. Same messages, same order as the VS Code shell's BrowserPanel

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use acpira_desktop::browser::{BrowserPage, host_of, normalize_preview_url};
use acpira_desktop::client::{PreviewAnswer, PreviewHandle, PreviewLease};
use serde_json::{Value, json};
use tauri::WebviewWindow;

use crate::page::Transport;

type Send = Box<dyn Fn(Value) + std::marker::Send + Sync>;
type OnTitle = Box<dyn Fn(String) + std::marker::Send + Sync>;

pub struct BrowserRelay {
  pub panel_id: String,
  /// The page as asked for: what the address bar shows and what a reconnect asks for again; None while empty
  target: Mutex<Option<String>>,
  /// The session folder on an SSH host the tab belongs to: its pages are that host's dev servers
  folder: Mutex<Option<String>>,
  lease: Mutex<Option<PreviewHandle>>,
  ready: AtomicBool,
  /// The frame (or error) waits for the page to listen
  pending: Mutex<Option<Value>>,
  send: Send,
  on_title: Option<OnTitle>,
  me: Weak<BrowserRelay>,
}

impl BrowserRelay {
  pub fn new(send: Send, on_title: Option<OnTitle>) -> Arc<BrowserRelay> {
    Arc::new_cyclic(|me| BrowserRelay {
      panel_id: uuid::Uuid::new_v4().to_string(),
      target: Mutex::new(None),
      folder: Mutex::new(None),
      lease: Mutex::new(None),
      ready: AtomicBool::new(false),
      pending: Mutex::new(None),
      send,
      on_title,
      me: me.clone(),
    })
  }

  pub fn target(&self) -> Option<String> {
    self.target.lock().unwrap().clone()
  }

  pub fn folder(&self) -> Option<String> {
    self.folder.lock().unwrap().clone()
  }

  pub fn set_folder(&self, folder: Option<String>) {
    *self.folder.lock().unwrap() = folder;
  }

  /// Another page through the same lease: the engine keeps the proxy while the site stays the same
  pub fn navigate(&self, url: String) {
    *self.target.lock().unwrap() = Some(url.clone());
    let me = self.me.clone();
    let asked = url.clone();
    let app = crate::app::app();
    let lease = PreviewLease {
      panel_id: self.panel_id.clone(),
      url,
      picker: app.paths.picker.to_string_lossy().into_owned(),
      folder: self.folder(),
      on_opened: Box::new(move |answer| {
        if let Some(r) = me.upgrade() {
          r.opened(&asked, answer);
        }
      }),
    };
    let handle = app.client().preview(lease);
    *self.lease.lock().unwrap() = Some(handle);
  }

  pub fn post_look(&self) {
    if self.ready.load(Ordering::SeqCst) {
      (self.send)(json!({ "type": "browserLook", "look": crate::app::app().browser_look() }));
    }
  }

  pub fn post_history(&self) {
    if self.ready.load(Ordering::SeqCst) {
      (self.send)(json!({ "type": "browserHistory", "pages": crate::app::app().history.pages() }));
    }
  }

  // A local page reaches the proxy as it is (the engine runs on this machine), so no forwarding step comes between
  fn opened(&self, url: &str, answer: PreviewAnswer) {
    if self.target().as_deref() != Some(url) {
      return;
    }
    self.post(match answer {
      PreviewAnswer::Opened { url: src, token } => json!({ "type": "browserFrame", "target": url, "src": src, "token": token }),
      PreviewAnswer::Error(error) => json!({ "type": "browserError", "target": url, "error": error }),
    });
  }

  // A frame or an error, which takes the place of the page shown so far
  fn post(&self, m: Value) {
    crate::app::app().leave_page(&self.panel_id);
    *self.pending.lock().unwrap() = Some(m.clone());
    if self.ready.load(Ordering::SeqCst) {
      (self.send)(m);
    }
  }

  pub fn on_page_message(&self, m: Value) {
    let app = crate::app::app();
    match m["type"].as_str().unwrap_or_default() {
      "browserReady" => {
        // A page that starts again (reloaded) shows the page anew; the old document is gone
        app.leave_page(&self.panel_id);
        self.ready.store(true, Ordering::SeqCst);
        self.post_look();
        self.post_history();
        let pending = self.pending.lock().unwrap().clone();
        if let Some(p) = pending {
          (self.send)(p);
        }
      }
      "browserNavigate" => {
        if let Some(url) = m["url"].as_str().and_then(normalize_preview_url) {
          self.navigate(url);
        }
      }
      "browserTitle" => {
        let url = m["url"].as_str().unwrap_or_default();
        let title: String = m["title"].as_str().unwrap_or_default().trim().chars().take(80).collect();
        if let Some(on_title) = &self.on_title {
          on_title(if title.is_empty() { host_of(url) } else { title.clone() });
        }
        if normalize_preview_url(url).is_some() {
          app.visited(BrowserPage { url: url.to_owned(), title });
        }
      }
      "browserAdd" => app.add_picked(m["item"].clone()),
      "browserOpenExternal" => {
        if let Some(url) = m["url"].as_str().filter(|u| u.starts_with("http://") || u.starts_with("https://")) {
          let _ = acpira_desktop::platform::system::open_external(url);
        }
      }
      "browserContext" => {
        let id = m["id"].clone();
        let answer = match app.page_context(&self.panel_id, m["op"].as_str().unwrap_or_default(), &m["payload"]) {
          Ok(shown) => match shown {
            Some(s) => json!({ "type": "browserContextDone", "id": id, "shown": s }),
            None => json!({ "type": "browserContextDone", "id": id }),
          },
          Err(e) => json!({ "type": "browserContextDone", "id": id, "error": e }),
        };
        (self.send)(answer);
      }
      _ => {}
    }
  }

  pub fn dispose(&self) {
    crate::app::app().leave_page(&self.panel_id);
    if let Some(h) = self.lease.lock().unwrap().take() {
      h.release();
    }
  }
}

/// The browser page as a window of its own (open_browser while no chat window can take it)
pub struct BrowserPanelWindow {
  pub label: String,
  pub window: WebviewWindow,
  pub transport: Arc<Transport>,
  pub relay: Arc<BrowserRelay>,
  /// Shown with the keyboard; an agent's open_browser leaves the caret in the chat
  focus: bool,
}

impl BrowserPanelWindow {
  pub fn new(window: WebviewWindow, folder: Option<String>, focus: bool) -> BrowserPanelWindow {
    let transport = Arc::new(Transport::default());
    let t = transport.clone();
    let w = window.clone();
    let relay = BrowserRelay::new(
      Box::new(move |m| t.send(m.to_string())),
      Some(Box::new(move |title| {
        let _ = w.set_title(&title);
      })),
    );
    relay.set_folder(folder);
    BrowserPanelWindow { label: window.label().to_owned(), window, transport, relay, focus }
  }

  pub fn focus_on_show(&self) -> bool {
    self.focus
  }

  pub fn target(&self) -> Option<String> {
    self.relay.target()
  }

  pub fn folder(&self) -> Option<String> {
    self.relay.folder()
  }

  pub fn reveal(&self, focus: bool) {
    if crate::app::app().background {
      crate::mac::order_back(&self.window);
      return;
    }
    let _ = self.window.unminimize();
    if focus {
      let _ = self.window.show();
      let _ = self.window.set_focus();
    } else {
      crate::mac::order_front(&self.window);
    }
  }

  pub fn set_theme(&self, dark: bool) {
    let _ = self.window.eval(format!("window.__acpiraTheme && window.__acpiraTheme({dark})"));
  }
}
