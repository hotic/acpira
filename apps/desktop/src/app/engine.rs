//! The engine connection: the client and its failure dialog, the engine binaries to try, starting the engine, and
//! the first-run flow's agent detection and data folder

use std::sync::Arc;
use std::sync::atomic::Ordering;

use acpira_desktop::client::{ClientOpts, HelloPayload, SidecarClient, SidecarState};
use acpira_desktop::first_run;
use acpira_desktop::locator::{LocateOpts, SidecarCommand, sidecar_commands};
use acpira_desktop::page::{APP_ORIGIN, FirstRunPage};
use acpira_desktop::platform::{CAPABILITIES, handle_request, hello_settings};
use serde_json::json;

use super::{App, Detection, app, log, log_arc};
use crate::chat_window::ChatWindow;

impl App {
  #[expect(clippy::expect_used, reason = "set in App::init right after APP, before any window or callback that calls this exists")]
  pub fn client(&self) -> &SidecarClient {
    self.client.get().expect("the client is made at init")
  }

  pub(super) fn make_client(self: &Arc<Self>) -> SidecarClient {
    let data_home = self.paths.data_home.clone();
    let commands = || app().engine_commands();
    let hello = || {
      let a = app();
      let paths = acpira_desktop::vscode_settings::ide_settings_paths(
        acpira_desktop::editors::this_platform(),
        &|k| std::env::var(k).ok(),
        &a.paths.home,
      );
      HelloPayload {
        client: json!({ "name": "acpira-desktop", "version": env!("CARGO_PKG_VERSION"), "capabilities": CAPABILITIES }),
        env: json!({ "hostLanguage": a.host_language(), "projects": true, "blobBase": format!("{APP_ORIGIN}/blobs") }),
        settings: hello_settings(
          a.settings.snapshot(),
          a.shared.exists(),
          || acpira_desktop::vscode_settings::read_ide_settings(&paths, &log),
          &log,
        ),
      }
    };
    let on_request = |r| -> acpira_desktop::client::RequestFuture {
      Box::pin(async move {
        // The actions touch windows and AppKit: they run on the main thread, the answer comes back here
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = app().handle.clone();
        let sent = handle.run_on_main_thread(move || {
          let _ = tx.send(handle_request(r, &**app()));
        });
        if sent.is_err() {
          return Err("the app is quitting".to_owned());
        }
        rx.await.unwrap_or_else(|_| Err("the app is quitting".to_owned()))
      })
    };
    let mut opts = ClientOpts::new(commands, hello, on_request, log_arc());
    // The persistent engine (a Unix socket, a named pipe on Windows); ACPIRA_ENGINE=stdio runs it attached instead
    {
      opts.engine = Box::new(move |cmd| {
        if std::env::var("ACPIRA_ENGINE").as_deref() == Ok("stdio") {
          return None;
        }
        Some(acpira_desktop::engine::engine_endpoint(&acpira_desktop::engine::EndpointOpts {
          binary: &cmd.command,
          home: &data_home,
          cwd: None,
          shell: Some("desktop"),
          tmp: None,
          uid: None,
        }))
      });
    }
    opts.on_state = Box::new(|state, detail| {
      if state != SidecarState::Failed {
        return;
      }
      let detail = detail.unwrap_or_default().to_owned();
      log(&format!("engine failed: {detail}"));
      app().engine_failed(detail);
    });
    SidecarClient::new(opts, tauri::async_runtime::handle().inner().clone())
  }

  /// A failure dialog at a time; the client's own restarts come first, this is what follows when they gave up
  fn engine_failed(&self, detail: String) {
    if self.background || self.failure_shown.swap(true, Ordering::SeqCst) {
      return;
    }
    let (title, retry, show, close) =
      (self.t("desktop.engine.failed", &[]), self.t("common.retry", &[]), self.t("desktop.menu.showLog", &[]), self.t("common.close", &[]));
    tauri::async_runtime::spawn(async move {
      let answer = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(&title)
        .set_description(&detail)
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom(retry.clone(), show.clone(), close))
        .show()
        .await;
      let a = app();
      a.failure_shown.store(false, Ordering::SeqCst);
      match answer {
        rfd::MessageDialogResult::Custom(c) if c == retry => a.client().retry(),
        rfd::MessageDialogResult::Custom(c) if c == show => a.show_log(),
        _ => {}
      }
    });
  }

  /// The engine binaries in preference order, each with the login shell's environment under its own
  fn engine_commands(&self) -> Vec<SidecarCommand> {
    let over = std::env::var("ACPIRA_SIDECAR_BIN").ok();
    let env = self.login_env.lock().clone().unwrap_or_default();
    sidecar_commands(&LocateOpts { candidates: &self.paths.engines, override_bin: over.as_deref() })
      .into_iter()
      .map(|mut c: SidecarCommand| {
        let mut merged: std::collections::BTreeMap<String, String> = env.clone().into_iter().collect();
        merged.extend(std::mem::take(&mut c.env));
        c.env = merged;
        c
      })
      .collect()
  }

  /// The login shell's environment (read off the main thread while the first window loads), then the engine. On a first
  /// run the engine waits for the user's yes (create_data_home), which would create the data folder; meanwhile the
  /// engine binary only lists the CLIs it finds, for the intro
  pub fn start_engine(self: &Arc<Self>) {
    std::thread::spawn(|| {
      let env = acpira_desktop::login_env::resolve_login_env(&log, None);
      let a = app();
      *a.login_env.lock() = env;
      // Paired with create_data_home's store then load: whichever runs second sees the other's write and starts the
      // engine (both may; a second start is a no-op)
      a.env_ready.store(true, Ordering::SeqCst);
      if !a.first_run.load(Ordering::SeqCst) {
        a.client().start();
        return;
      }
      log("first run: no data folder, the engine waits for consent");
      a.probe_agents();
    });
  }

  /// The page's data, when the first-run flow is on: the folder as the consent card names it and the language to use
  pub fn first_run_page(&self) -> Option<FirstRunPage> {
    if !self.first_run.load(Ordering::SeqCst) {
      return None;
    }
    let home = first_run::display_home(&self.paths.data_home, &self.paths.home, cfg!(unix));
    Some(FirstRunPage { home, locale: self.locale().tag().to_owned() })
  }

  /// Runs `acpira agents --json` and answers the windows that asked meanwhile
  fn probe_agents(&self) {
    let found = match self.engine_commands().first() {
      Some(engine) => first_run::probe_agents(engine),
      None => Err("no engine binary".into()),
    };
    let next = match found {
      Ok(agents) => Detection::Found(json!(agents)),
      Err(e) => {
        log(&format!("first run: no agent list: {e}"));
        Detection::Failed
      }
    };
    let waiting = match std::mem::replace(&mut *self.detection.lock(), next) {
      Detection::Running(w) => w,
      _ => vec![],
    };
    for w in waiting {
      self.detect_agents(&w);
    }
  }

  /// A first-run page asks which CLIs this machine has (detectAgents): answered now, or once the probe is done
  pub fn detect_agents(&self, w: &Arc<ChatWindow>) {
    let agents = {
      let mut d = self.detection.lock();
      match &mut *d {
        Detection::Running(waiting) => {
          waiting.push(w.clone());
          return;
        }
        Detection::Found(agents) => agents.clone(),
        Detection::Failed => return,
      }
    };
    w.post_now(json!({ "type": "detectedAgents", "agents": agents }));
  }

  /// The consent card's yes (createDataHome): the folder, then the engine, whose init swaps the page's sample shell for
  /// the real one. Every window waits on the same engine, so a yes in one ends the flow in all
  pub fn create_data_home(&self, w: &Arc<ChatWindow>) {
    if !self.first_run.load(Ordering::SeqCst) {
      return;
    }
    if let Err(e) = first_run::create_home(&self.paths.data_home) {
      log(&format!("first run: creating {} failed: {e}", self.paths.data_home.display()));
      w.post_now(json!({ "type": "firstRunError", "error": e.to_string() }));
      return;
    }
    log(&format!("first run: created {}", self.paths.data_home.display()));
    self.first_run.store(false, Ordering::SeqCst);
    if self.env_ready.load(Ordering::SeqCst) {
      // Off the page's message callback: starting may spawn and connect to the engine
      std::thread::spawn(|| app().client().start());
    }
  }
}
