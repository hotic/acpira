//! The shell's side of the platform requests (`acpira_desktop::platform`): settings, notifications, windows, the
//! browser, terminals, editors and the OS file manager, and the folder picker

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use acpira_desktop::editors::{Editor, FileTarget};
use acpira_desktop::platform::{Platform, system};
use acpira_desktop::terminal::TerminalRequest;
use acpira_desktop::wire::Initial;
use acpira_shared::sidecar::{BrowserOpened, ToastOpen};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::{App, ChatWindowOpen, app, log};
use crate::chat_window::ChatWindow;

/// window.open of an http(s) page: the system browser
pub(super) fn open_outside(url: &str) {
  if url.starts_with("http://") || url.starts_with("https://") {
    let _ = system::open_external(url);
  }
}

/// The system folder dialog, as a sheet on the window; pickedFolder carries no path when it was dismissed. Smoke runs
/// script the answers (ACPIRA_DESKTOP_PICK, path-list separated), across windows, without showing it
pub fn pick_folder(w: Arc<ChatWindow>) {
  static SCRIPTED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
  let scripted = SCRIPTED.get_or_init(|| {
    let raw = std::env::var("ACPIRA_DESKTOP_PICK").unwrap_or_default();
    Mutex::new(std::env::split_paths(&raw).map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.is_empty()).collect())
  });
  let next = {
    let mut s = scripted.lock();
    (!s.is_empty()).then(|| s.remove(0))
  };
  if let Some(path) = next {
    w.post(json!({ "type": "pickedFolder", "path": path }));
    return;
  }
  let window = w.window.clone();
  // GTK's dialog would carry rfd's English title ("Select Folder"); the File menu's item names it in the app's language
  #[cfg(target_os = "linux")]
  let title = app().t("desktop.empty.open", &[]).trim_end_matches('…').to_owned();
  tauri::async_runtime::spawn(async move {
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window).set_can_create_directories(true);
    #[cfg(target_os = "linux")]
    let dialog = dialog.set_title(title);
    let picked = dialog.pick_folder().await;
    match picked {
      Some(dir) => w.post(json!({ "type": "pickedFolder", "path": dir.path().to_string_lossy() })),
      None => w.post(json!({ "type": "pickedFolder" })),
    }
  });
}

impl Platform for App {
  fn write_setting(&self, key: &str, value: Option<Value>) {
    self.settings.set(key, value);
  }

  fn setting(&self, key: &str) -> Option<Value> {
    App::setting(self, key)
  }

  fn log(&self, line: &str) {
    log(line);
  }

  fn watching(&self, session_id: &str) -> bool {
    self.windows().iter().any(|w| w.watching(session_id))
  }

  fn chime(&self) {
    if let Some(w) = self.focused_window() {
      w.post(json!({ "type": "chime" }));
    }
  }

  fn notification(&self, title: &str, body: &str, session_id: &str) {
    self.system_notification(title, body, Some(session_id));
  }

  fn toast(&self, level: &str, text: &str, open: Option<ToastOpen>) {
    App::toast(self, level, text, open);
  }

  fn open_window(&self, session_id: Option<String>) {
    let initial = Some(session_id.map(Initial::Session).unwrap_or(Initial::MostRecent));
    app().open(initial, ChatWindowOpen::default());
  }

  fn open_browser(&self, url: &str, folder: Option<&str>) -> Result<BrowserOpened, String> {
    app().open_browser(url, folder)
  }

  fn terminal(&self, r: TerminalRequest) {
    acpira_desktop::terminal::run_in_terminal(&r, &log);
  }

  fn open_in_editor(&self, editor: Editor, target: &FileTarget) -> bool {
    // The login shell's PATH finds an editor's command line; an app started from the Dock has almost none of its own
    let mut env: HashMap<String, String> = std::env::vars().collect();
    env.extend(self.login_env.lock().clone().unwrap_or_default());
    acpira_desktop::editors::open_in_editor(editor, target, &env, &self.paths.home, &log)
  }

  fn open_external(&self, url: &str) -> Result<(), String> {
    system::open_external(url)
  }

  fn reveal(&self, path: &str) -> Result<(), String> {
    system::reveal(path)
  }

  fn open_path(&self, path: &str) -> Result<(), String> {
    system::open_path(path)
  }

  fn t(&self, key: &str, params: &[(&str, &str)]) -> String {
    App::t(self, key, params)
  }
}
