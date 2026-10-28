//! Menu and page actions: the page-drawn menu, the log, safe mode, page commands, the ChatGPT mirror, menu and
//! theme refreshes, and quitting

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use acpira_desktop::locator::{LocateOpts, sidecar_commands};
use acpira_desktop::platform::system;
use acpira_desktop::wire::Initial;
use serde_json::{Value, json};

use super::{App, ChatWindowOpen, app, log};
use crate::chat_window::Watch;

impl App {
  /// The menu the pages draw changed (menu.rs install, Windows)
  pub fn set_page_menu(&self, menus: Value) {
    *self.page_menu.lock() = Some(menus.clone());
    self.broadcast(json!({ "type": "desktopMenu", "menus": menus }));
  }

  /// The page menu's top-level titles (the smoke's menuBar outside macOS)
  #[cfg(feature = "smoke")]
  pub fn page_menu_titles(&self) -> Option<Vec<String>> {
    let menus = self.page_menu.lock().clone()?;
    Some(menus.as_array()?.iter().filter_map(|m| m["label"].as_str().map(str::to_owned)).collect())
  }

  pub fn show_log(&self) {
    let _ = system::open_path(&self.log_file.to_string_lossy());
  }

  /// Help › Safe Mode: switched with the engine binary (so it works while the engine is stuck), told in a toast; on the
  /// way in an engine that had failed is started again
  pub fn toggle_safe_mode(self: &Arc<Self>) {
    let a = self.clone();
    // Safe mode is a file in the data folder, which a first run has not been allowed to create yet
    if a.first_run.load(Ordering::SeqCst) {
      return;
    }
    std::thread::spawn(move || {
      let home = a.paths.data_home.clone();
      let on = !acpira_desktop::safe_mode::is_on(&home);
      let over = std::env::var("ACPIRA_SIDECAR_BIN").ok();
      let binary = sidecar_commands(&LocateOpts { candidates: &a.paths.engines, override_bin: over.as_deref() }).into_iter().next().map(|c| c.command);
      let Some(binary) = binary else { return };
      match acpira_desktop::safe_mode::set(&binary, &home, on) {
        Ok(()) => {
          log(&format!("safe mode {}", if on { "on" } else { "off" }));
          a.toast("info", &a.t(if on { "host.safeModeOn" } else { "host.safeModeOff" }, &[]), None);
          // A no-op while the engine runs
          if on {
            a.client().retry();
          }
        }
        Err(e) => a.toast("error", &a.t("host.safeModeFailed", &[("error", &e)]), None),
      }
    });
  }

  /// The menu bar's page commands go to the chat used last (a new window when none is open)
  pub fn command(self: &Arc<Self>, c: &str) {
    let Some(w) = self.focused_window().or_else(|| self.open(Some(Initial::MostRecent), self.place_of_last())) else { return };
    w.reveal();
    w.post(json!({ "type": "shellCommand", "command": c }));
  }

  /// A new window bound to a fresh ChatGPT mirror; its connection prompt goes to the clipboard once the window shows it
  pub fn connect_chatgpt(self: &Arc<Self>) {
    let watch: Watch = Box::new(|m| {
      if !matches!(m.kind.as_str(), "session" | "init") {
        return false;
      }
      let Ok(v) = serde_json::from_str::<Value>(m.raw.get()) else { return false };
      let s = if m.kind == "session" { &v["session"] } else { &v["state"]["active"] };
      let Some(prompt) = s["external"]["connectionPrompt"].as_str().map(str::to_owned) else { return false };
      let a = app();
      let h = a.handle.clone();
      let _ = h.run_on_main_thread(move || {
        crate::os::clipboard_write(&prompt);
        let a = app();
        if let Some(w) = a.focused_window() {
          w.post(json!({ "type": "toast", "level": "info", "text": a.t("chatgpt.copied", &[]) }));
        }
      });
      true
    });
    let Some(w) = self.open(None, ChatWindowOpen { watch: Some(watch), ..Default::default() }) else { return };
    let Ok(msg) = serde_json::value::RawValue::from_string(r#"{"type":"connectChatgpt"}"#.into()) else { return };
    self.client().send(&w.view_id, msg);
  }

  pub fn refresh_menu(&self) {
    let h = self.handle.clone();
    let _ = h.run_on_main_thread(|| crate::menu::install(&app().handle));
  }

  pub fn theme_changed(&self) {
    let dark = self.dark();
    for w in self.windows() {
      w.set_theme(dark);
    }
    for b in self.browsers.lock().iter() {
      b.set_theme(dark);
    }
  }

  /// Quitting saves where every window was, then only disconnects: the engine keeps running turns and ends on its own
  /// once idle
  pub fn before_quit(&self) {
    if self.quitting.load(Ordering::SeqCst) {
      return;
    }
    self.save_windows();
    self.quitting.store(true, Ordering::SeqCst);
    let client = self.client().clone();
    let _ = tauri::async_runtime::block_on(async move { tokio::time::timeout(Duration::from_secs(2), client.dispose()).await });
    // A staged update goes in as the app quits (it starts the next time the app does); a download stops and resumes then
    if let Some(u) = self.updater() {
      u.stop();
      if u.staged().is_some()
        && let Err(e) = u.apply()
      {
        log(&format!("update: not installed at quit: {e}"));
      }
    }
  }
}
