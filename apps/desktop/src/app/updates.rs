//! The app's own updater: made from a release build's configuration, its state on every page and the menu, checks,
//! the auto-download switch and the restart that installs a staged bundle

use std::path::Path;
use std::sync::Arc;

use acpira_desktop::update::{self, Phase, UpdateState, Updater};
use serde_json::{Value, json};

use super::{App, app, log, log_arc};

impl App {
  /// The app's own updater (docs/dev/desktop.md "Updates"): a release build's configuration, or the smoke run's local
  /// release server; what keeps this install from replacing itself is known before any check
  pub(super) fn make_updater(&self) -> Arc<Updater> {
    let exe = acpira_desktop::dirs::this_exe();
    // The bundle on macOS, the packaged exe itself on Windows (none for a cargo build)
    let installed = exe.as_deref().and_then(update::swap::install_of);
    let (config, manual) = crate::smoke::update_config(installed.as_deref())
      .unwrap_or_else(|| (update::Config::release(), update::install_blocker(installed.as_deref(), update::stage::signing_team)));
    let env = update::Env {
      config,
      installed,
      cache: self.paths.cache.join("updates"),
      current: env!("CARGO_PKG_VERSION").into(),
      os_version: update::os_version(),
      manual,
      // The engine's route: ACPIRA_PROXY, else the shared `proxy` setting
      proxy: Box::new(|| {
        use acpira_shared::proxy::{AUTO_PORT, ProxyMode, port_listening, resolve};
        let setting = app().setting("proxy");
        resolve(&ProxyMode::effective(setting.as_ref().and_then(Value::as_str)), || port_listening(AUTO_PORT))
      }),
      log: log_arc(),
    };
    let auto = self.settings.get("updateAuto") != Some(Value::Bool(false));
    Updater::new(env, auto, |s| {
      let s = s.clone();
      let h = app().handle.clone();
      let _ = h.run_on_main_thread(move || app().update_changed(s));
    })
  }

  pub fn updater(&self) -> Option<&Arc<Updater>> {
    self.updater.get()
  }

  /// Every page gets the new state; reaching Ready (or leaving it) rebuilds the menu's item, and Ready says so in the
  /// page used last with a restart button. Main thread
  fn update_changed(&self, s: UpdateState) {
    self.broadcast(json!({ "type": "updateState", "state": s }));
    let before = std::mem::replace(&mut *self.update_phase.lock(), s.phase);
    if (before == Phase::Ready) != (s.phase == Phase::Ready) {
      self.refresh_menu();
    }
    if s.phase == Phase::Ready && before != Phase::Ready {
      let version = s.version.clone().unwrap_or_default();
      let text = self.t("desktop.update.ready", &[("version", &version)]);
      log(&format!("toast info: {text}"));
      match self.focused_window() {
        Some(w) => w.post(json!({ "type": "toast", "level": "info", "text": text,
          "action": { "label": self.t("desktop.update.restart", &[]), "command": "updateInstall" } })),
        None => self.system_notification("Acpira", &text, None),
      }
    }
  }

  /// A check from the page or the menu; it downloads when the switch is on or the page asked for it
  pub fn check_updates(&self, download: bool) {
    if let Some(u) = self.updater() {
      u.check(download || u.state().auto);
    }
  }

  pub fn set_update_auto(&self, on: bool) {
    self.settings.set("updateAuto", Some(Value::Bool(on)));
    if let Some(u) = self.updater() {
      u.set_auto(on);
    }
  }

  /// Restart to update: the staged bundle takes the installed one's place, the app quits as usual (window places
  /// saved, the engine left running its turns), and a helper starts the new bundle once this process is gone (on
  /// Windows the new exe itself, which waits for this process before claiming the single instance)
  pub fn restart_to_update(&self) {
    let Some(u) = self.updater() else { return };
    let Some(installed) = u.installed().map(Path::to_path_buf) else { return };
    if let Err(e) = u.apply() {
      self.toast("error", &e, None);
      return;
    }
    self.before_quit();
    if let Err(e) = update::swap::relaunch_after_exit(std::process::id(), &installed, crate::smoke::RELAUNCH_DIRECT) {
      log(&format!("update: the relaunch helper did not start: {e}"));
    }
    self.handle.exit(0);
  }
}
