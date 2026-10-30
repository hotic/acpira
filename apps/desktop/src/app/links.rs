//! Session links and notifications: `acpira://` links from the launch, a second launch or the system, the Dock
//! icon, and toasts that become system notifications with no window open

use std::path::PathBuf;
use std::sync::Arc;

use acpira_desktop::wire::Initial;
use acpira_shared::sidecar::ToastOpen;
use serde_json::json;

use super::{App, app, log, log_arc};

/// Clicking a system notification brings its session up
pub fn session_clicked(id: String) {
  let h = app().handle.clone();
  let _ = h.run_on_main_thread(move || app().show_session(&id));
}

impl App {
  /// Only reading the corresponding session clears its icon mark; activating another window leaves the others alone.
  pub fn acknowledge_attention(&self, w: &crate::chat_window::ChatWindow) {
    let Some(id) = w.session_id() else { return };
    if w.watching(&id) && self.attention.lock().remove(&id) {
      self.update_attention();
    }
  }

  pub(crate) fn update_attention(&self) {
    // Only macOS has a Dock badge; background (bench) runs leave the Dock alone
    #[cfg(target_os = "macos")]
    if !self.background {
      crate::mac::set_dock_badge(self.attention.lock().len());
    }
  }

  /// A notification or a session link: the window already showing it, else the focused one switches, else a new one
  pub fn show_session(self: &Arc<Self>, id: &str) {
    if let Some(on) = self.windows().into_iter().find(|w| w.session_id().as_deref() == Some(id)) {
      on.reveal();
      return;
    }
    if let Some(w) = self.focused_window() {
      w.reveal();
      w.post(json!({ "type": "showSession", "id": id }));
      return;
    }
    self.open(Some(Initial::Session(id.to_owned())), self.place_of_last());
  }

  /// Session links (acpira://session/<id>), from the launch arguments, a second launch or the system. A link names a
  /// session this machine's store has (or a window shows), else the page says the link came from elsewhere
  pub fn take_link(self: &Arc<Self>, url: &str) {
    {
      let mut pending = self.pending_links.lock();
      if let Some(p) = pending.as_mut() {
        p.push(url.to_owned());
        return;
      }
    }
    let Some(id) = acpira_shared::session_links::session_id_of_link(url) else {
      // Any other acpira:// link (a plain notification's click on Windows, `acpira://open`) brings the app forward
      log(&format!("link without a session: {url}"));
      self.second_instance(vec![]);
      return;
    };
    let sessions = self.client().sessions_dir().map(PathBuf::from).unwrap_or_else(|| self.paths.data_home.join("sessions"));
    let known = self.windows().iter().any(|w| w.session_id().as_deref() == Some(id.as_str()))
      || sessions.join(format!("{id}.json")).exists()
      || self.paths.data_home.join("bridges/chatgpt").join(format!("{id}.json")).exists();
    if known {
      self.show_session(&id);
      return;
    }
    let Some(w) = self.focused_window().or_else(|| self.open(Some(Initial::MostRecent), self.place_of_last())) else { return };
    w.reveal();
    w.post(json!({ "type": "toast", "level": "error", "text": self.t("sessionLink.notHere", &[]) }));
  }

  /// Another launch's arguments: its links, else the app comes forward (a new window when none is open)
  pub fn second_instance(self: &Arc<Self>, args: Vec<String>) {
    let links = acpira_desktop::single_instance::links_in(&args);
    if !links.is_empty() {
      for l in links {
        self.take_link(&l);
      }
      return;
    }
    match self.focused_window() {
      Some(w) => w.reveal(),
      None => {
        self.open(Some(Initial::MostRecent), self.place_of_last());
      }
    }
  }

  /// The Dock icon clicked with no window open
  #[cfg(target_os = "macos")]
  pub fn reopen(self: &Arc<Self>) {
    if self.windows().is_empty() {
      self.open(Some(Initial::MostRecent), self.place_of_last());
    }
  }

  /// A toast lands in the page used last; with no window open it becomes a system notification
  pub fn toast(&self, level: &str, text: &str, open: Option<ToastOpen>) {
    log(&format!("toast {level}: {text}"));
    if let Some(w) = self.focused_window() {
      let mut m = json!({ "type": "toast", "level": level, "text": text });
      if let Some(o) = open {
        m["open"] = json!(o);
      }
      w.post(m);
      return;
    }
    self.system_notification("Acpira", text, None);
  }

  pub(crate) fn system_notification(&self, title: &str, body: &str, session: Option<&str>) {
    // A background run (smoke) shows nothing on the screen: no banner, and no permission prompt for the first one
    if self.background {
      log(&format!("notification (background run, not shown): {title}: {body}"));
      return;
    }
    let notifier = self.notifier.get_or_init(|| crate::os::Notifier::new(crate::app::session_clicked, log_arc()));
    if let Some(n) = notifier {
      n.show(title, body, session);
    }
  }
}
