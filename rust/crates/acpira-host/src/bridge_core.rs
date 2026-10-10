//! One per view: routes WebviewMsgs to its viewer / the manager / the
//! settings center, resolves paths before asking the platform for an IDE action, and pushes changes back coalesced

use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::Value;

use acpira_shared::appearance::Appearance;
use acpira_shared::protocol::{FileHit, HostMsg, InitState, WebviewHost, WebviewMsg, is_safe_external_url};
use acpira_shared::session_patch::{Delivery, SentViews};
use acpira_shared::settings::is_setting_key;
use acpira_shared::sidecar::{InitialView, PlanTarget, ToastOpen};

use crate::acp::transcript::normalize::file_url_to_path;
use crate::i18n::{t, tp};
use crate::session_manager::{SessionManager, Viewer};
use crate::settings::SettingsCenter;
use crate::sidecar::platform::SidecarPlatform;
use crate::store::data_dir::normalize;

/// Manager events within this window collapse to one post per message type
pub const BATCH_WINDOW: Duration = Duration::from_millis(30);

pub type Post = Arc<dyn Fn(HostMsg) + Send + Sync>;

/// Streaming updates are dense: within one batch window only the latest message per key survives, and an idle session edge
/// goes out at once together with whatever was pending
#[derive(Default)]
pub struct MsgBatch {
  pending: Vec<(String, HostMsg)>,
  armed: bool,
}

/// What a push asks of the owner
pub enum Pushed {
  /// Post these now
  Flush(Vec<HostMsg>),
  /// Start the window timer, then flush
  Arm,
  /// A timer is already running
  Wait,
}

impl MsgBatch {
  pub fn push(&mut self, m: HostMsg) -> Pushed {
    let idle = matches!(&m, HostMsg::Session { running: false, .. });
    let key = m.batch_key();
    match self.pending.iter().position(|(k, _)| *k == key) {
      Some(i) => self.pending[i].1 = m,
      None => self.pending.push((key, m)),
    }
    if idle {
      return Pushed::Flush(self.flush());
    }
    if self.armed {
      return Pushed::Wait;
    }
    self.armed = true;
    Pushed::Arm
  }

  pub fn flush(&mut self) -> Vec<HostMsg> {
    self.armed = false;
    std::mem::take(&mut self.pending).into_iter().map(|(_, m)| m).collect()
  }

  pub fn clear(&mut self) {
    self.pending.clear();
  }
}

pub struct BridgeCore {
  pub viewer: Arc<Viewer>,
  manager: Arc<SessionManager>,
  settings: Arc<SettingsCenter>,
  platform: Arc<SidecarPlatform>,
  appearance: Arc<dyn Fn() -> Appearance + Send + Sync>,
  post: Post,
  host: WebviewHost,
  blob_base: Option<String>,
  ready: std::sync::atomic::AtomicBool,
  /// A session view was pushed before the page was ready: the session it opens has a view to put into init
  view_shown: tokio::sync::Notify,
  /// The page applies `sessionPatch` messages (it said so in `ready`)
  patches: std::sync::atomic::AtomicBool,
  /// The session views this page holds as far as patches go: the one on screen and the last one sent of each recent
  /// session, what the next patch of that session is computed against (`SentViews`)
  sent: parking_lot::Mutex<SentViews>,
  batch: parking_lot::Mutex<MsgBatch>,
  settings_sub: parking_lot::Mutex<Option<u64>>,
  me: Weak<BridgeCore>,
}

impl BridgeCore {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    manager: Arc<SessionManager>,
    settings: Arc<SettingsCenter>,
    platform: Arc<SidecarPlatform>,
    appearance: Arc<dyn Fn() -> Appearance + Send + Sync>,
    post: Post,
    host: WebviewHost,
    blob_base: Option<String>,
    initial: Option<InitialView>,
  ) -> Arc<BridgeCore> {
    let viewer = manager.attach(initial);
    let core = Arc::new_cyclic(|me: &Weak<BridgeCore>| BridgeCore {
      viewer,
      manager,
      settings,
      platform,
      appearance,
      post,
      host,
      blob_base,
      ready: Default::default(),
      view_shown: Default::default(),
      patches: Default::default(),
      sent: Default::default(),
      batch: Default::default(),
      settings_sub: Default::default(),
      me: me.clone(),
    });
    let weak = core.me.clone();
    core.viewer.subscribe(Arc::new(move |m| {
      if let Some(c) = weak.upgrade() {
        c.queue(m);
      }
    }));
    let weak = core.me.clone();
    let id = core.settings.subscribe(Arc::new(move |view, locale| {
      if let Some(c) = weak.upgrade() {
        c.queue(HostMsg::Settings { settings: view.clone(), locale });
      }
    }));
    *core.settings_sub.lock() = Some(id);
    core
  }

  /// Failures are logged, never thrown at the transport: a bad message must not take the view down
  pub async fn handle(self: Arc<Self>, raw: Value) {
    let kind = WebviewMsg::raw_type(&raw).to_owned();
    let m: WebviewMsg = match serde_json::from_value(raw) {
      Ok(m) => m,
      Err(e) => {
        self.platform.log(&format!("webview {kind} failed: {e}"));
        return;
      }
    };
    if let Err(e) = self.route(m).await {
      self.platform.log(&format!("webview {kind} failed: {e}"));
    }
  }

  async fn route(self: &Arc<Self>, m: WebviewMsg) -> anyhow::Result<()> {
    use WebviewMsg as W;
    let (manager, platform) = (&self.manager, &self.platform);
    match m {
      W::ViewFocus => {}
      W::Ready { patches } => {
        self.patches.store(patches, std::sync::atomic::Ordering::Release);
        // A view (re)opening is a cheap moment to re-check executables and pick up sessions another window created
        let m2 = manager.clone();
        tokio::spawn(async move { m2.reprobe().await });
        manager.refresh_index().await;
        // Init waits for the session on screen to have a view, not for its agent: a stored session is shown as soon as
        // its record is read, while starting the agent and resuming can take seconds, and the page stays blank until init
        let ensure = {
          let (m, v) = (manager.clone(), self.viewer.clone());
          tokio::spawn(async move { m.ensure_active_for(&v).await })
        };
        tokio::select! {
          _ = ensure => {}
          _ = self.view_shown.notified() => {}
        }
        // A page's first pushes start only now: init below carries everything they would have, and opening a long session
        // during the awaits above used to send its whole view once before init and once inside it
        self.ready.store(true, std::sync::atomic::Ordering::Release);
        let (active, active_parts) = match manager.session_msg(self.viewer.active_id().as_deref()) {
          Some(HostMsg::Session { session, parts, .. }) => (Some(session), parts),
          _ => (None, None),
        };
        let state = InitState {
          host: self.host,
          appearance: (self.appearance)(),
          agents: manager.agents(),
          accounts: manager.accounts(),
          account_actions: Some(manager.account_actions()),
          agent_installs: Some(manager.agent_installs()),
          hidden: manager.hidden(),
          sessions: manager.sessions(),
          categories: manager.categories(),
          active,
          settings: self.settings.view(),
          locale: self.settings.locale(),
          home: platform.home(),
          cwd: platform.cwd(),
          blob_base: self.blob_base.clone(),
        };
        // The page takes init's view as a whole and starts its cache over, keeping that view as the one on screen: it
        // becomes the base the next push of that session patches against, instead of a second copy of the transcript
        let mut sent = self.sent.lock();
        sent.clear();
        if let Some(parts) = active_parts.as_ref().filter(|_| self.patches.load(std::sync::atomic::Ordering::Acquire)) {
          sent.deliver(Some(parts));
        }
        (self.post)(HostMsg::Init { state: Box::new(state) });
        drop(sent);
        // A batched list may reach the page around init and lose to init's copy, and the manager does not repeat an
        // unchanged list: the batch gets the current one, ordered against every other list push
        manager.sessions_in_order(|sessions| self.queue(HostMsg::Sessions { sessions }));
      }
      // The page could not apply a patch of the session on screen: that session goes whole, the rest it keeps stay patchable
      W::Resync => {
        if let Some(id) = self.viewer.active_id() {
          self.sent.lock().forget(&id);
        }
        if let Some(m) = manager.session_msg(self.viewer.active_id().as_deref()) {
          self.deliver(m);
        }
      }
      W::ChatgptStatus => {
        let status = manager.chatgpt_status().await;
        self.post_now(HostMsg::ChatgptStatus { status });
      }
      W::OpenInEditor { session_id } => platform.open_in_editor(session_id.or_else(|| self.viewer.active_id())),
      W::OpenFile { session_id, path, line } => {
        // Tool references are relative to the session they came from; a view showing another session ignores them
        let Some(active) = self.viewer.active_id().filter(|a| *a == session_id) else { return Ok(()) };
        let Some(cwd) = manager.session_cwd(&active) else { return Ok(()) };
        let resolved = if crate::platform::file_url::is_file_url(&path) {
          file_url_to_path(&path).ok_or_else(|| anyhow::anyhow!("File URL must be absolute"))
        } else {
          Ok(normalize(&Path::new(&cwd).join(&path)).to_string_lossy().into_owned())
        };
        match resolved {
          Ok(p) => {
            if let Err(e) = platform.open_resolved_file(&p, line.filter(|l| *l > 0)).await {
              platform.toast("error", &e.to_string());
            }
          }
          Err(e) => platform.toast("error", &e.to_string()),
        }
      }
      W::OpenBlob { session_id, name } => {
        if let Some(p) = manager.blob_path(&session_id, &name)
          && let Err(e) = platform.open_resolved_file(&p.to_string_lossy(), None).await
        {
          platform.toast("error", &e.to_string());
        }
      }
      W::OpenPlan { session_id, plan_id } => {
        if let Some(plan) = manager.plan_document(&session_id, &plan_id) {
          let exists = match &plan.path {
            Some(p) => tokio::fs::metadata(p).await.is_ok(),
            None => false,
          };
          let target = match (exists, plan.path) {
            (true, Some(path)) => PlanTarget::Path { path },
            _ => PlanTarget::Markdown { markdown: plan.markdown },
          };
          platform.open_plan_document(target).await?;
        }
      }
      W::OpenExternal { url } => {
        if is_safe_external_url(&url) {
          platform.open_external(&url);
        } else {
          let head: String = url.chars().take(80).collect();
          platform.log(&format!("openExternal refused: scheme not on the allowlist ({head})"));
        }
      }
      W::SearchFiles { query, seq } => {
        // Always answer, even on failure: the webview holds a promise per seq
        let files: Vec<FileHit> = match platform.search_files(&query).await {
          Ok(f) => f,
          Err(e) => {
            platform.log(&format!("searchFiles failed: {e}"));
            vec![]
          }
        };
        self.post_now(HostMsg::Files { seq, files });
      }
      W::SearchSessions { query, seq } => {
        // Always answer, like searchFiles: the history list holds a promise per seq
        let hits = manager.search_sessions(&query).await;
        self.post_now(HostMsg::SessionHits { seq, hits });
      }
      // Always answered, so the page's copy never waits on a request nobody will reply to
      W::DiffSource { session_id, tool_call_id, nth, subagent_id, seq } => {
        let source = manager.diff_source(&session_id, &tool_call_id, nth, subagent_id.as_deref());
        self.post_now(HostMsg::DiffSource { seq, source });
      }
      // The file is not opened on its own: the toast names it and offers an Open button. A repeated export of an unchanged
      // transcript names the same file (SessionManager::export_session)
      W::ExportSession { id, format } => match manager.export_session(&id, format).await {
        Ok(path) => {
          let p = path.to_string_lossy().into_owned();
          let open = ToastOpen { label: t("host.exportOpen"), path: p.clone() };
          platform.toast_with("info", &tp("host.exported", &[("path", &p)]), Some(open));
        }
        Err(e) => platform.toast("error", &e.to_string()),
      },
      W::ListNativeSessions { agent } => {
        // Always answer: the popover is waiting on this agent's list
        match manager.list_native_sessions(&agent).await {
          Ok(sessions) => self.post_now(HostMsg::NativeSessions { agent, sessions, error: None }),
          Err(e) => self.post_now(HostMsg::NativeSessions { agent, sessions: vec![], error: Some(e.to_string()) }),
        }
      }
      W::EditTurn { request_id, edit } => {
        let error = manager.edit_turn(edit).await.err().map(|e| e.to_string());
        self.post_now(HostMsg::EditTurnResult { request_id, error });
      }
      W::SetSetting { key, value } => {
        if is_setting_key(&key)
          && let Err(e) = self.settings.set(&key, &value).await
        {
          platform.log(&format!("settings setSetting failed: {e}"));
        }
      }
      W::SetAppearance { axis, value } => {
        if let Err(e) = self.settings.set_appearance(&axis, &value).await {
          platform.log(&format!("settings setAppearance failed: {e}"));
        }
      }
      W::OpenAgentConfig => {
        platform.open_resolved_file(&self.settings.agent_config_path().to_string_lossy(), None).await?;
      }
      W::OpenPath { path } => {
        // A path from the inventory lists: files open in the editor, directories reveal in the OS file manager
        let dir = tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_dir());
        let r = if dir { platform.reveal_in_os(&path).await } else { platform.open_resolved_file(&path, None).await };
        if let Err(e) = r {
          platform.log(&format!("settings openPath failed: {e}"));
        }
      }
      W::Inventory { agent } => {
        let inventory = self.settings.inventory(&agent).await;
        self.post_now(HostMsg::Inventory { agent, inventory });
      }
      W::Shared => {
        let view = self.settings.shared_view().await;
        self.post_now(HostMsg::Shared { view, error: None });
      }
      W::SharedAction { action } => {
        let error = match self.settings.shared_action(action).await {
          Ok(outcome) => {
            // A created file opens in the editor, a folder is revealed in the OS
            if let Some(p) = outcome.open {
              let path = p.to_string_lossy().into_owned();
              let r = if p.is_dir() { platform.reveal_in_os(&path).await } else { platform.open_resolved_file(&path, None).await };
              if let Err(e) = r {
                platform.log(&format!("shared config: opening {path} failed: {e}"));
              }
            }
            None
          }
          Err(e) => Some(format!("{e:#}")),
        };
        let view = self.settings.shared_view().await;
        self.post_now(HostMsg::Shared { view, error });
      }
      W::Providers => {
        let view = self.settings.providers_view().await;
        self.post_now(HostMsg::Providers { view, error: None });
      }
      W::ProviderAction { action } => {
        let error = self.settings.provider_action(action).await.err().map(|e| format!("{e:#}"));
        let view = self.settings.providers_view().await;
        self.post_now(HostMsg::Providers { view, error });
      }
      W::Controls { agent, fresh } => {
        if fresh != Some(true) {
          let controls = manager.known_controls(&agent).await;
          self.post_now(HostMsg::Controls { agent, controls });
        } else {
          // The probe's initialize brings the version back too; a fresh inventory keeps the facts card in step
          let controls = manager.probe_controls(&agent).await;
          self.post_now(HostMsg::Controls { agent: agent.clone(), controls });
          let inventory = self.settings.inventory(&agent).await;
          self.post_now(HostMsg::Inventory { agent, inventory });
        }
      }
      other => manager.handle_for(&self.viewer, other).await,
    }
    Ok(())
  }

  fn queue(&self, m: HostMsg) {
    if !self.ready.load(std::sync::atomic::Ordering::Acquire) {
      // Dropped: init carries the current state. A session view means the one being opened can go into init now
      // (`notify_one` keeps the wake-up when the ready handler is not waiting yet)
      if matches!(m, HostMsg::Session { .. }) {
        self.view_shown.notify_one();
      }
      return;
    }
    let pushed = self.batch.lock().push(m);
    match pushed {
      Pushed::Flush(queued) => queued.into_iter().for_each(|m| self.deliver(m)),
      Pushed::Arm => {
        let weak = self.me.clone();
        tokio::spawn(async move {
          tokio::time::sleep(BATCH_WINDOW).await;
          if let Some(c) = weak.upgrade() {
            c.flush();
          }
        });
      }
      Pushed::Wait => {}
    }
  }

  fn flush(&self) {
    let queued = self.batch.lock().flush();
    for m in queued {
      self.deliver(m);
    }
  }

  /// Post a batched message; a session view goes out as a patch against the last view of that session the page was sent,
  /// the one on screen or one it keeps from before a switch (`SentViews`). The diff and the post happen under one lock, so
  /// two flushes racing on different threads can never post patches out of order
  fn deliver(&self, m: HostMsg) {
    let HostMsg::Session { parts, .. } = &m else { return (self.post)(m) };
    let mut sent = self.sent.lock();
    let next = parts.as_ref().filter(|_| self.patches.load(std::sync::atomic::Ordering::Acquire));
    match sent.deliver(next) {
      // Older than what the page shows: it carries nothing the page does not have and must not become a patch base
      Delivery::Stale => {}
      Delivery::Patch(patch) => (self.post)(HostMsg::SessionPatch { patch }),
      Delivery::Whole => (self.post)(m),
    }
  }

  pub fn post_now(&self, m: HostMsg) {
    (self.post)(m);
  }

  pub fn push_appearance(&self) {
    self.post_now(HostMsg::Appearance { appearance: (self.appearance)() });
  }

  pub fn dispose(&self) {
    self.batch.lock().clear();
    if let Some(id) = self.settings_sub.lock().take() {
      self.settings.unsubscribe(id);
    }
    self.manager.detach(&self.viewer);
  }
}
