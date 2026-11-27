//! One per view: routes WebviewMsgs to its viewer / the manager / the
//! settings center, resolves paths before asking the platform for an IDE action, and pushes changes back coalesced

use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::Value;

use acpira_shared::appearance::Appearance;
use acpira_shared::protocol::{ExportFormat, HostMsg, WebviewHost, WebviewMsg, is_safe_external_url};
use acpira_shared::session_patch::SentViews;
use acpira_shared::session_window::VIEW_WINDOW_BYTES;
use acpira_shared::settings::is_setting_key;
use acpira_shared::sidecar::{InitialView, PlanTarget, ToastOpen};

use crate::i18n::{t, tp};
use crate::project_files;
use crate::remote::connections::Connections;
use crate::session_manager::{SessionManager, Viewer, ViewerShell};
use crate::settings::SettingsCenter;
use crate::sidecar_platform::SidecarPlatform;
use crate::terminal::TerminalHub;

mod files;
mod push;
mod ready;
mod settings;

pub use self::push::{MsgBatch, Pushed};

/// Manager events within this window collapse to one post per message type
pub const BATCH_WINDOW: Duration = Duration::from_millis(30);

/// How long `init` waits for a host session's first view (one SSH round trip plus the host's record read); past it the
/// page gets init without the session and its view follows as a push
const HOST_VIEW_WAIT: Duration = Duration::from_secs(2);

pub type Post = Arc<dyn Fn(HostMsg) + Send + Sync>;

/// `tokio::spawn` keeping the page's wire as `ORIGIN`: a task-local does not cross a spawn, and without it a request the
/// task makes later (a fetched remote file opening, a toast) would go to whichever window is active by then
fn spawn_on<F>(wire: Option<u64>, fut: F) -> tokio::task::JoinHandle<F::Output>
where
  F: std::future::Future + Send + 'static,
  F::Output: Send + 'static,
{
  match wire {
    Some(w) => tokio::spawn(acpira_host_base::origin::ORIGIN.scope(w, fut)),
    None => tokio::spawn(fut),
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
  /// The shell connection this page belongs to (None: tests, a viewer-less harness): its settings view, and the
  /// `ORIGIN` its spawned tasks keep so the platform requests they make later go back to it
  wire: Option<u64>,
  ready: std::sync::atomic::AtomicBool,
  /// A session view was pushed before the page was ready: the session it opens has a view to put into init
  view_shown: tokio::sync::Notify,
  /// Answers to a pick made before the page was ready that init cannot carry (`sessionUnavailable`): sent after init
  refused: parking_lot::Mutex<Vec<HostMsg>>,
  /// The page applies `sessionPatch` messages (it said so in `ready`)
  patches: std::sync::atomic::AtomicBool,
  /// The page takes session views that start at their tail and asks for the rest (`ready` with `window`; a paired phone)
  window: std::sync::atomic::AtomicBool,
  /// The session views this page holds as far as patches go: the one on screen and the last one sent of each recent
  /// session, what the next patch of that session is computed against (`SentViews`)
  sent: parking_lot::Mutex<SentViews>,
  /// The page applies `sessionsPatch` messages (it said so in `ready`)
  list_patches: std::sync::atomic::AtomicBool,
  /// The session list this page holds and its revision (0 for a whole list, one more per patch), what the next list
  /// patch is computed against; only kept for list patches. Swapped under `sent` too, so a list push racing init waits
  /// for init's list
  listed: parking_lot::Mutex<Option<(u64, Vec<acpira_shared::transcript::SessionSummary>)>>,
  /// Held from taking messages out of the batch until they are posted: two flushes on different threads (the window's
  /// timer and an immediate edge) would otherwise post their batches crosswise, an older list after a newer one
  order: parking_lot::Mutex<()>,
  batch: parking_lot::Mutex<MsgBatch>,
  settings_sub: parking_lot::Mutex<Option<u64>>,
  /// The stored session the page opens on, read from attach on (`ready` waits for this load instead of starting its own)
  preload: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
  /// Set by `dispose`; posts after it are dropped (`emit`)
  disposed: parking_lot::RwLock<bool>,
  /// The engine's terminals and this page's id among their viewers
  terminals: Arc<TerminalHub>,
  terminal_view: u64,
  /// SSH hosts; the page is subscribed from its first `connections` request on
  connections: Arc<Connections>,
  connections_sub: parking_lot::Mutex<Option<u64>>,
  /// The paired-devices gateway; the page is subscribed from its first `remoteAccess` request on
  remote_access: Arc<crate::devices::RemoteAccess>,
  remote_access_sub: parking_lot::Mutex<Option<u64>>,
  /// The user's plugins; the page is subscribed from its first `plugins` request on
  plugins: Arc<crate::plugins::host::PluginHost>,
  plugins_sub: parking_lot::Mutex<Option<u64>>,
  /// The theme layer this page was sent last, and its subscription (from the first `ready` on)
  theme: parking_lot::Mutex<Option<String>>,
  theme_sub: parking_lot::Mutex<Option<u64>>,
  /// The plugins' interface pieces this page was sent last (with the theme's subscription), and its subscription to
  /// what their processes send pages
  ui: parking_lot::Mutex<Option<Vec<acpira_shared::plugins::PluginSlot>>>,
  pages_sub: parking_lot::Mutex<Option<u64>>,
  me: Weak<BridgeCore>,
}

impl BridgeCore {
  #[allow(clippy::too_many_arguments, reason = "one argument per engine part the bridge wires together")]
  pub fn new(
    manager: Arc<SessionManager>,
    settings: Arc<SettingsCenter>,
    platform: Arc<SidecarPlatform>,
    appearance: Arc<dyn Fn() -> Appearance + Send + Sync>,
    post: Post,
    host: WebviewHost,
    blob_base: Option<String>,
    initial: Option<InitialView>,
    wire: Option<u64>,
    shell: Option<ViewerShell>,
    terminals: Arc<TerminalHub>,
    connections: Arc<Connections>,
    plugins: Arc<crate::plugins::host::PluginHost>,
    remote_access: Arc<crate::devices::RemoteAccess>,
  ) -> Arc<BridgeCore> {
    let viewer = manager.attach_in(initial, shell);
    viewer.set_wire(wire);
    // A peer's page is that machine's, any other this machine's: the settings the turns it sends read
    viewer.set_sent_from(match wire.and_then(|w| platform.peer_machine(w)) {
      Some(m) => crate::acp::session::SentFrom::Machine(m),
      None => crate::acp::session::SentFrom::Here,
    });
    // The page loads its bundle between attach and `ready`: a stored session it opens on is read and brought live meanwhile,
    // so `ready` finds its view instead of starting a long record's read only then. A page opening on a new session
    // still starts it at `ready`
    let preload = viewer.active_id().map(|_| {
      let (m, v) = (manager.clone(), viewer.clone());
      spawn_on(wire, async move { m.ensure_active_for(&v).await })
    });
    let core = Arc::new_cyclic(|me: &Weak<BridgeCore>| BridgeCore {
      viewer,
      manager,
      settings,
      platform,
      appearance,
      post,
      host,
      blob_base,
      wire,
      ready: Default::default(),
      view_shown: Default::default(),
      refused: Default::default(),
      patches: Default::default(),
      window: Default::default(),
      list_patches: Default::default(),
      listed: Default::default(),
      order: Default::default(),
      sent: Default::default(),
      batch: Default::default(),
      settings_sub: Default::default(),
      preload: parking_lot::Mutex::new(preload),
      disposed: Default::default(),
      terminal_view: terminals.next_view(),
      terminals,
      connections,
      connections_sub: Default::default(),
      remote_access,
      remote_access_sub: Default::default(),
      plugins,
      plugins_sub: Default::default(),
      theme: Default::default(),
      theme_sub: Default::default(),
      ui: Default::default(),
      pages_sub: Default::default(),
      me: me.clone(),
    });
    let weak = core.me.clone();
    core.viewer.subscribe(Arc::new(move |m| {
      if let Some(c) = weak.upgrade() {
        c.queue(m);
      }
    }));
    let weak = core.me.clone();
    let id = core.settings.subscribe(wire, Arc::new(move |view, locale| {
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
      W::Ready { patches, list_patches, window } => self.on_ready(patches, list_patches, window).await?,
      // The page could not apply a patch of the session on screen: that session goes whole, the rest it keeps stay patchable
      W::Resync => {
        if let Some(id) = self.viewer.active_id() {
          self.sent.lock().forget(&id);
        }
        if let Some(m) = manager.session_msg(self.viewer.active_id().as_deref()) {
          self.deliver(m);
        }
      }
      // The part right ahead of what the page holds, from the view it was sent last (what its copy is made of). A page
      // the engine has no such view for gets the session again, opened at its tail
      W::LoadEarlier { session_id } => {
        let mut sent = self.sent.lock();
        match sent.earlier(&session_id, VIEW_WINDOW_BYTES) {
          Some(chunk) => self.emit(HostMsg::SessionEarlier { chunk }),
          None if !sent.holds(&session_id) => {
            drop(sent);
            if let Some(m) = manager.session_msg(Some(&session_id)).filter(|_| self.viewer.active_id().as_deref() == Some(&session_id)) {
              self.deliver(m);
            }
          }
          None => {}
        }
      }
      W::ResyncSessions => {
        {
          let _sent = self.sent.lock();
          *self.listed.lock() = None;
        }
        manager.sessions_in_order(&self.viewer);
      }
      W::OpenInEditor { session_id } => platform.open_in_editor(session_id.or_else(|| self.viewer.active_id())),
      W::OpenFile { session_id, path, line, reveal } => self.on_open_file(session_id, path, line, reveal).await?,
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
      W::SearchFiles { query, seq } => self.on_search_files(query, seq).await?,
      W::SearchSessions { query, seq } => {
        // Always answer, like searchFiles: the history list holds a promise per seq
        let hits = manager.search_sessions_for(&self.viewer, &query).await;
        self.post_now(HostMsg::SessionHits { seq, hits });
      }
      // Always answered, so the page's copy never waits on a request nobody will reply to
      W::DiffSource { session_id, tool_call_id, nth, subagent_id, seq } => {
        let source = manager.diff_source(&session_id, &tool_call_id, nth, subagent_id.as_deref()).await;
        self.post_now(HostMsg::DiffSource { seq, source });
      }
      // Stage a file a page shows from its path into the session's blob store: a row's image, video or widget kept as a
      // path (every one of a session on an SSH host), or a file reference clicked for preview
      W::ImageBlob { session_id, path, seq, shown } => self.on_image_blob(session_id, path, seq, shown).await?,
      W::StageFile { session_id, path, seq } => self.on_stage_file(session_id, path, seq).await?,
      // The desktop tool panel's reads (project_files.rs). Each is always answered, and runs on its own task: a git
      // status or a large file must not hold up the messages behind it
      W::ProjectDir { session_id, dir, seq } => {
        let (core, root) = (self.clone(), manager.session_cwd(&session_id));
        spawn_on(self.wire, async move {
          let (entries, error) = match core.workspace(root).await {
            Err(e) => (vec![], Some(e.code())),
            Ok(ws) => match ws.list_dir(&dir).await {
              Ok(entries) => (entries, None),
              Err(e) => (vec![], Some(e.code())),
            },
          };
          core.post_now(HostMsg::ProjectDir { seq, entries, error });
        });
      }
      W::ProjectEdit { session_id, edit, seq } => {
        let (core, root) = (self.clone(), manager.session_cwd(&session_id));
        spawn_on(self.wire, async move {
          let done = match core.workspace(root).await {
            Ok(ws) => ws.edit(&edit).await,
            Err(e) => Err(e),
          };
          core.post_now(HostMsg::ProjectEdit { seq, error: done.err().map(|e| e.code()) });
        });
      }
      W::ProjectStatus { session_id, seq } => {
        let (core, root) = (self.clone(), manager.session_cwd(&session_id));
        spawn_on(self.wire, async move {
          let marks = match core.workspace(root.clone()).await {
            Ok(ws) => ws.git_marks().await,
            Err(_) => Default::default(),
          };
          core.post_now(HostMsg::ProjectStatus { seq, root, marks });
        });
      }
      W::ProjectFile { session_id, path, base, seq } => self.on_project_file(session_id, path, base, seq).await?,
      // The tool panel's terminal tabs (terminal.rs). Handled inline, so keys reach the shell in the order typed
      W::TerminalOpen { id, cwd, cols, rows } => {
        let weak = self.me.clone();
        let sink = Arc::new(move |m: HostMsg| {
          if let Some(c) = weak.upgrade() {
            c.post_now(m);
          }
        });
        self.terminals.open(self.terminal_view, sink, &id, cwd.as_deref(), cols.zip(rows));
      }
      W::TerminalInput { id, data, binary } => self.terminals.input(&id, data, binary),
      W::TerminalResize { id, cols, rows } => self.terminals.resize(&id, cols, rows),
      W::TerminalAck { id, chars } => self.terminals.ack(self.terminal_view, &id, chars),
      W::TerminalClose { id } => self.terminals.close(self.terminal_view, &id),
      W::TerminalKill { id } => self.terminals.kill(&id),
      // SSH hosts (remote/connections.rs)
      W::Connections => {
        let mut sub = self.connections_sub.lock();
        if sub.is_none() {
          let weak = self.me.clone();
          *sub = Some(self.connections.subscribe(Arc::new(move |views| {
            if let Some(c) = weak.upgrade() {
              c.post_now(HostMsg::Connections { connections: views.to_vec() });
            }
          })));
        }
        drop(sub);
        self.post_now(HostMsg::Connections { connections: self.connections.views() });
      }
      W::ConnectionOp { op } => {
        let (core, connections) = (self.clone(), self.connections.clone());
        spawn_on(self.wire, async move {
          if let Err(e) = connections.op(op).await {
            core.platform.toast("error", &format!("{e:#}"));
            // The page may have shown the edit already: the list as it is puts it right
            core.post_now(HostMsg::Connections { connections: connections.views() });
          }
        });
      }
      // Paired devices (devices/mod.rs)
      W::RemoteAccess => {
        let mut sub = self.remote_access_sub.lock();
        if sub.is_none() {
          let weak = self.me.clone();
          *sub = Some(self.remote_access.subscribe(Arc::new(move |view| {
            if let Some(c) = weak.upgrade() {
              c.post_now(HostMsg::RemoteAccess { view: view.clone() });
            }
          })));
        }
        drop(sub);
        self.post_now(HostMsg::RemoteAccess { view: self.remote_access.view() });
      }
      W::RemoteAccessOp { op } => {
        let (core, ra) = (self.clone(), self.remote_access.clone());
        spawn_on(self.wire, async move {
          if let Err(e) = ra.op(op).await {
            core.platform.toast("error", &format!("{e:#}"));
          }
        });
      }
      W::SshHosts => {
        let (core, connections) = (self.clone(), self.connections.clone());
        spawn_on(self.wire, async move {
          let hosts = connections.ssh_hosts().await;
          core.post_now(HostMsg::SshHosts { hosts, error: None });
        });
      }
      W::RemoteDir { alias, path, seq } => {
        let (core, connections) = (self.clone(), self.connections.clone());
        spawn_on(self.wire, async move {
          let msg = match connections.browse(&alias, path.as_deref()).await {
            Ok((path, entries)) => HostMsg::RemoteDir { seq, alias, path, entries, error: None },
            Err(e) => HostMsg::RemoteDir { seq, alias, path: path.unwrap_or_default(), entries: vec![], error: Some(format!("{e:#}")) },
          };
          core.post_now(msg);
        });
      }
      W::LocalDir { path, seq } => {
        let core = self.clone();
        spawn_on(self.wire, async move {
          let msg = match project_files::local_folders(path.as_deref()).await {
            Ok((path, parent, entries)) => HostMsg::LocalDir { seq, path, parent, entries, error: None },
            Err(e) => HostMsg::LocalDir { seq, path: path.unwrap_or_default(), parent: None, entries: vec![], error: Some(e) },
          };
          core.post_now(msg);
        });
      }
      W::SessionChanges { session_id, seq } => {
        let (core, root, bases) = (self.clone(), manager.session_cwd(&session_id), manager.edit_bases(&session_id).await);
        spawn_on(self.wire, async move {
          let files = match core.workspace(root).await {
            Ok(ws) => ws.session_changes(bases).await,
            Err(_) => vec![],
          };
          core.post_now(HostMsg::SessionChanges { seq, files });
        });
      }
      // The context split and what its leaves sent (`acp::session::context`), always answered: the card and the report
      // window wait on them. Off the message loop, since the agent answers over its own wire
      W::SessionContext { session_id, seq, detail } => {
        let core = self.clone();
        spawn_on(self.wire, async move {
          let report = core.manager.session_context(&session_id, detail).await;
          core.post_now(HostMsg::SessionContext { seq, report });
        });
      }
      W::SessionContextItem { session_id, rev, id, seq } => {
        let core = self.clone();
        spawn_on(self.wire, async move {
          let content = core.manager.session_context_item(&session_id, &rev, &id).await;
          core.post_now(HostMsg::SessionContextItem { seq, content });
        });
      }
      W::SessionContextRaw { session_id, seq } => {
        let core = self.clone();
        spawn_on(self.wire, async move {
          let (raw, error) = match core.manager.session_context_raw(&session_id).await {
            Ok(raw) => (raw, None),
            Err(e) => (None, Some(e)),
          };
          core.post_now(HostMsg::SessionContextRaw { seq, raw, error });
        });
      }
      // Always answered, like diffSource: the page's clipboard write waits on it
      W::SessionMarkdown { id, seq, format } => {
        let (markdown, error) = match manager.session_markdown(&id, format.unwrap_or(ExportFormat::Markdown)).await {
          Ok(m) => (Some(m), None),
          Err(e) => (None, Some(e.to_string())),
        };
        self.post_now(HostMsg::SessionMarkdown { seq, markdown, error });
      }
      // The file is not opened on its own: the toast names it and offers Open and a reveal in the OS file manager. A
      // repeated export of an unchanged transcript names the same file (SessionManager::export_session)
      W::ExportSession { id, format } => match manager.export_session(&id, format).await {
        Ok(path) => {
          let p = path.to_string_lossy().into_owned();
          // One wording on every OS: the page shows both as icons, the label is their tooltip
          let reveal = t("session.project.revealFolder");
          let open = ToastOpen { label: t("host.exportOpen"), path: p.clone(), reveal: Some(reveal) };
          platform.toast_with("info", &tp("host.exported", &[("path", &p)]), Some(open));
        }
        Err(e) => platform.toast("error", &e.to_string()),
      },
      W::ListNativeSessions { agent, cwd } => {
        // Always answer: the popover is waiting on this agent's list
        match manager.list_native_sessions(&agent, manager.folder_for(Some(&self.viewer), cwd)).await {
          Ok(sessions) => self.post_now(HostMsg::NativeSessions { agent, sessions, error: None }),
          Err(e) => self.post_now(HostMsg::NativeSessions { agent, sessions: vec![], error: Some(e.to_string()) }),
        }
      }
      W::EditTurn { request_id, edit } => {
        let error = manager.edit_turn_from(edit, &self.viewer.sent_from()).await.err().map(|e| e.to_string());
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
      W::SetAgentBinary { agent, path } => {
        if let Err(e) = self.settings.set_agent_binary(&agent, path.as_deref()).await {
          let msg = format!("{e:#}");
          platform.log(&format!("settings setAgentBinary failed: {msg}"));
          platform.toast("error", &msg);
        }
        let inventory = self.settings.inventory(&agent, &self.folder()).await;
        // Emit agents before inventory: an availability flip clears the webview's cached inventory, which the fresh one then fills
        self.manager.emit_agents();
        self.post_now(HostMsg::Inventory { agent, inventory });
      }
      W::OpenPath { path, reveal } => {
        // A path from the inventory lists: files open in the editor, directories (or anything asked to) reveal in the OS
        // file manager
        let dir = reveal || tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_dir());
        let r = if dir { platform.reveal_in_os(&path).await } else { platform.open_resolved_file(&path, None).await };
        if let Err(e) = r {
          platform.log(&format!("settings openPath failed: {e}"));
        }
      }
      W::Inventory { agent } => {
        let inventory = self.settings.inventory(&agent, &self.folder()).await;
        self.post_now(HostMsg::Inventory { agent, inventory });
      }
      W::Shared => {
        let view = self.settings.shared_view(&self.folder()).await;
        self.post_now(HostMsg::Shared { view, error: None });
      }
      W::SharedAction { action } => self.on_shared_action(action).await?,
      W::Storage { detail } => {
        let view = self.manager.storage_view(&detail).await;
        self.post_now(HostMsg::Storage { view, outcome: None, error: None });
      }
      W::StorageAction { action: acpira_shared::storage::StorageAction::Reveal { path }, .. } => {
        match self.manager.storage_reveal(path).await {
          Ok(p) => {
            if let Err(e) = platform.reveal_in_os(&p.to_string_lossy()).await {
              platform.log(&format!("storage: revealing {} failed: {e}", p.display()));
            }
          }
          Err(e) => platform.log(&format!("storage: reveal refused: {e:#}")),
        }
      }
      W::StorageAction { action, detail } => {
        let reply = self.manager.storage_action(action, &detail).await;
        self.post_now(HostMsg::Storage { view: reply.view, outcome: reply.outcome, error: reply.error });
      }
      W::Usage => {
        let view = self.manager.usage_view().await;
        self.post_now(HostMsg::Usage { view });
      }
      W::Plugins => self.on_plugins().await?,
      W::PluginAction { action } => self.on_plugin_action(action).await?,
      // A card's thumbnail: the backdrop of the version it offers, read from the frozen copy
      W::PluginBackdrop { plugin, snapshot } => {
        let (me, plugins) = (self.clone(), self.plugins.clone());
        spawn_on(self.wire, async move {
          let (p, s) = (plugin.clone(), snapshot.clone());
          let html = tokio::task::spawn_blocking(move || plugins.backdrop_of(&p, &s)).await.ok().flatten();
          me.post_now(HostMsg::PluginBackdrop { plugin, snapshot, html });
        });
      }
      // A plugin method may take its process's full time limit: off the message loop, answered by id
      W::PluginCall { id, plugin, method, params } => {
        let me = self.clone();
        spawn_on(self.wire, async move {
          let (result, error) = match me.plugins.runtimes().rpc(&plugin, &method, params, "page").await {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
          };
          me.post_now(HostMsg::PluginCalled { id, result, error });
        });
      }
      W::Providers => {
        let view = self.settings.providers_view().await;
        self.post_now(HostMsg::Providers { view, error: None });
      }
      W::ProviderAction { action } => {
        let error = self.settings.provider_action(action).await.err().map(|e| format!("{e:#}"));
        let view = self.settings.providers_view().await;
        self.post_now(HostMsg::Providers { view, error });
        // A saved, switched or removed source may add or drop a logo the chat surface draws
        self.push_provider_marks().await;
      }
      // Network round trips (a test call takes seconds): off the message loop, answered by id
      W::ProviderProbe { id, probe } => {
        let me = self.clone();
        spawn_on(self.wire, async move {
          let outcome = me.settings.provider_probe(probe).await;
          me.post_now(HostMsg::ProviderProbed { id, outcome });
        });
      }
      W::Controls { agent, fresh, cwd } => self.on_controls(agent, fresh, cwd).await?,
      other => manager.handle_for(&self.viewer, other).await,
    }
    Ok(())
  }

  /// The project the settings page's project files belong to: the view's (`SessionManager::folder_for`); the home
  /// directory, which the scans read as "no project", when there is none
  fn folder(&self) -> String {
    // A folder on an SSH host has no project files here: the scans read only the user's own
    self.manager.folder_for(Some(&self.viewer), None).filter(|f| !crate::workspace::is_remote(f)).unwrap_or_else(|| self.platform.home())
  }

  pub fn dispose(&self) {
    *self.disposed.write() = true;
    self.batch.lock().clear();
    if let Some(id) = self.settings_sub.lock().take() {
      self.settings.unsubscribe(id);
    }
    if let Some(id) = self.connections_sub.lock().take() {
      self.connections.unsubscribe(id);
    }
    if let Some(id) = self.remote_access_sub.lock().take() {
      self.remote_access.unsubscribe(id);
    }
    if let Some(id) = self.plugins_sub.lock().take() {
      self.plugins.unsubscribe(id);
    }
    if let Some(id) = self.theme_sub.lock().take() {
      self.plugins.unsubscribe(id);
    }
    if let Some(id) = self.pages_sub.lock().take() {
      self.plugins.runtimes().unsubscribe_pages(id);
    }
    // Its terminals run on a while without it, for a reload to come back to
    self.terminals.detach_view(self.terminal_view);
    self.manager.detach(&self.viewer);
  }
}

// A path an agent wrote in its reply: `~` / `~/…` is the engine user's home (agents cite `~/.agents/plans/…`), anything
// else is relative to the session cwd. Joining `~/…` onto the cwd would open a missing `<cwd>/~/…`
/// The largest file on an SSH host a click fetches to open here
const REMOTE_OPEN_MAX: u64 = 32 * 1024 * 1024;

#[cfg(test)]
mod tests {
  use std::path::Path;

  #[cfg(unix)]
  use super::files::fallback_open_paths;
  use super::files::remote_copy_path;

  #[cfg(unix)]
  #[test]
  fn fallback_tries_other_folders_then_folders_above() {
    let roots = ["/p/acpira/rust".to_string(), "/p/acpira-web".to_string()];
    let got = fallback_open_paths(&roots, "out/a.mp4");
    assert_eq!(got, ["/p/acpira-web/out/a.mp4", "/p/acpira/out/a.mp4", "/p/out/a.mp4"]);
    // A sibling project's name leading the path is found two levels up a nested session folder
    assert!(fallback_open_paths(&roots[..1], "acpira-web/out/a.mp4").contains(&"/p/acpira-web/out/a.mp4".to_string()));
    for anchored in ["./a.md", "../a.md", "/abs/a.md", "~/a.md", "~"] {
      assert!(fallback_open_paths(&roots, anchored).is_empty(), "{anchored}");
    }
    assert!(fallback_open_paths(&["/".to_string()], "a.md").is_empty());
  }

  #[test]
  fn remote_copy_stays_in_the_cache() {
    let root = Path::new("/d");
    let cache = Path::new("/d/run/remote/files");
    assert_eq!(remote_copy_path(root, "example-host", "/home/u/a.md"), cache.join("example-host/home/u/a.md"));
    // An alias may be `..` or carry characters Windows refuses in a name
    assert_eq!(remote_copy_path(root, "..", "/x"), cache.join("__/x"));
    assert_eq!(remote_copy_path(root, "u@h:22", "/x"), cache.join("u@h_22/x"));
    assert!(remote_copy_path(root, "h", "/a/b:c?.txt").starts_with(cache.join("h")));
  }

  // A page's spawned task still answers to its window when it calls the platform later
  #[tokio::test]
  async fn a_spawned_task_keeps_its_page_s_wire_as_origin() {
    use crate::sidecar_platform::origin;
    assert_eq!(super::spawn_on(Some(7), async { tokio::task::yield_now().await; origin() }).await.ok(), Some(Some(7)));
    assert_eq!(super::spawn_on(None, async { origin() }).await.ok(), Some(None));
  }
}
