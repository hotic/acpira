//! One per view: routes WebviewMsgs to its viewer / the manager / the
//! settings center, resolves paths before asking the platform for an IDE action, and pushes changes back coalesced

use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::Value;

use acpira_shared::appearance::Appearance;
use acpira_shared::protocol::{ExportFormat, FileHit, HostMsg, InitState, ProjectBase, WebviewHost, WebviewMsg, is_safe_external_url};
use acpira_shared::remote::normalize_remote_path;
use acpira_shared::session_patch::{Delivery, SentViews};
use acpira_shared::settings::is_setting_key;
use acpira_shared::sidecar::{InitialView, PlanTarget, ToastOpen};

use crate::acp::transcript::normalize::file_url_to_path;
use crate::i18n::{t, tp};
use crate::project_files;
use crate::session_manager::{SessionManager, Viewer};
use crate::settings::SettingsCenter;
use crate::sidecar::platform::SidecarPlatform;
use crate::store::data_dir::{home_dir, normalize};
use crate::remote::connections::Connections;
use crate::terminal::TerminalHub;

/// Manager events within this window collapse to one post per message type
pub const BATCH_WINDOW: Duration = Duration::from_millis(30);

pub type Post = Arc<dyn Fn(HostMsg) + Send + Sync>;

/// Streaming updates are dense: within one batch window only the latest message per key survives, and an idle session edge
/// goes out at once together with whatever was pending. Only the edge: a session whose own turn is over may still stream
/// (a summoned child the turn left running), and those idle views coalesce like running ones
#[derive(Default)]
pub struct MsgBatch {
  pending: Vec<(String, HostMsg)>,
  armed: bool,
  /// The running flag of the last session view pushed; None before the first
  running: Option<bool>,
  /// The id of the last session view pushed: switching to another session flushes at once instead of waiting out the window
  session_id: Option<String>,
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
    let immediate = match &m {
      HostMsg::Session { running, parts, .. } => {
        let switched = match parts.as_ref().map(|p| p.0.id.as_str()) {
          Some(id) => self.session_id.replace(id.to_owned()).as_deref() != Some(id),
          None => false,
        };
        let idle = self.running.replace(*running) != Some(false) && !*running;
        switched || idle
      }
      HostMsg::DeleteSessionResult { .. } => true,
      _ => false,
    };
    let key = m.batch_key();
    match self.pending.iter().position(|(k, _)| *k == key) {
      Some(i) => self.pending[i].1 = m,
      None => self.pending.push((key, m)),
    }
    if immediate {
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
    terminals: Arc<TerminalHub>,
    connections: Arc<Connections>,
    plugins: Arc<crate::plugins::host::PluginHost>,
    remote_access: Arc<crate::devices::RemoteAccess>,
  ) -> Arc<BridgeCore> {
    let viewer = manager.attach(initial);
    // The page loads its bundle between attach and `ready`: a stored session it opens on is read and brought live meanwhile,
    // so `ready` finds its view instead of starting a long record's read only then. A page opening on a new session
    // still starts it at `ready`
    let preload = viewer.active_id().map(|_| {
      let (m, v) = (manager.clone(), viewer.clone());
      tokio::spawn(async move { m.ensure_active_for(&v).await })
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
      ready: Default::default(),
      view_shown: Default::default(),
      patches: Default::default(),
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
          let preload = self.preload.lock().take();
          tokio::spawn(async move {
            // The load begun at attach goes first: a session it could not open is replaced below, as without it
            if let Some(load) = preload {
              let _ = load.await;
            }
            m.ensure_active_for(&v).await
          })
        };
        tokio::select! {
          _ = ensure => {}
          _ = self.view_shown.notified() => {}
        }
        // A page's first pushes start only now: init below carries everything they would have, and opening a long session
        // during the awaits above used to send its whole view once before init and once inside it.
        // The sent views stay locked from here until init is out: a session push racing init (the agent starting, a status
        // change) waits in `deliver` and goes as a patch against init's view. Before, it found no base yet and went whole,
        // a second copy of a long transcript just before init
        let mut sent = self.sent.lock();
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
          // A project shell without a project yet has the home directory here, like a shell without a workspace
          cwd: manager.folder_for(Some(&self.viewer), None).unwrap_or_else(|| platform.home()),
          blob_base: self.blob_base.clone(),
          chats: Some(manager.chat_root().to_string_lossy().into_owned()),
        };
        // The page takes init's view as a whole and starts its cache over, keeping that view as the one on screen: it
        // becomes the base the next push of that session patches against, instead of a second copy of the transcript
        sent.clear();
        if let Some(parts) = active_parts.as_ref().filter(|_| self.patches.load(std::sync::atomic::Ordering::Acquire)) {
          sent.deliver(Some(parts));
        }
        self.emit(HostMsg::Init { state: Box::new(state) });
        drop(sent);
        // A batched list may reach the page around init and lose to init's copy, and the manager does not repeat an
        // unchanged list: the batch gets the current one, ordered against every other list push
        manager.sessions_in_order(|sessions| self.queue(HostMsg::Sessions { sessions }));
        // A reloaded page lost its theme layer: it goes again after every init (a task of its own: `sent`'s guard above
        // keeps this future from crossing an await)
        *self.theme.lock() = None;
        *self.ui.lock() = None;
        let me = self.clone();
        tokio::spawn(async move {
          me.push_theme().await;
          me.push_ui().await;
        });
        let mut sub = self.theme_sub.lock();
        if sub.is_none() {
          let weak = self.me.clone();
          *sub = Some(self.plugins.subscribe(Arc::new(move || {
            if let Some(c) = weak.upgrade() {
              tokio::spawn(async move {
                c.push_theme().await;
                c.push_ui().await;
              });
            }
          })));
        }
        drop(sub);
        let mut pages = self.pages_sub.lock();
        if pages.is_none() {
          let weak = self.me.clone();
          *pages = Some(self.plugins.runtimes().subscribe_pages(Arc::new(move |msg| {
            if let Some(c) = weak.upgrade() {
              c.post_now(msg);
            }
          })));
        }
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
        let status = manager.chatgpt_status(manager.folder_for(Some(&self.viewer), None)).await;
        self.post_now(HostMsg::ChatgptStatus { status });
      }
      W::OpenInEditor { session_id } => platform.open_in_editor(session_id.or_else(|| self.viewer.active_id())),
      W::OpenFile { session_id, path, line, reveal } => {
        // Tool references are relative to the session they came from; a view showing another session ignores them
        let Some(active) = self.viewer.active_id().filter(|a| *a == session_id) else { return Ok(()) };
        let Some(cwd) = manager.session_cwd(&active) else { return Ok(()) };
        // A file on an SSH host opens as a read-only copy fetched here, so the editor of choice works with the file the
        // agent names; the copy has no folder worth revealing
        if let Some(f) = acpira_shared::remote::parse_remote_folder(&cwd) {
          let named = if crate::platform::file_url::is_file_url(&path) { file_url_to_path(&path) } else { Some(path.clone()) };
          let stand = crate::acp::vendors::claude_remote::Stand::new(&f.alias, &f.path, &manager.data_root());
          let host_path = named.filter(|p| !reveal && *p != "~" && !p.starts_with("~/")).and_then(|p| normalize_remote_path(&stand.host_path(&p)));
          let Some(host_path) = host_path else {
            platform.toast("info", &tp("host.remoteFile", &[("path", &path), ("host", &f.alias)]));
            return Ok(());
          };
          let core = self.clone();
          let copy = remote_copy_path(&manager.data_root(), &f.alias, &host_path);
          tokio::spawn(async move { core.open_remote_file(cwd, f.alias, host_path, copy, line.filter(|l| *l > 0)).await });
          return Ok(());
        }
        let resolved = if crate::platform::file_url::is_file_url(&path) {
          file_url_to_path(&path).ok_or_else(|| anyhow::anyhow!("File URL must be absolute"))
        } else {
          Ok(normalize(&resolve_open_path(&cwd, &path)).to_string_lossy().into_owned())
        };
        match resolved {
          Ok(p) => {
            let res = if reveal {
              platform.reveal_in_os(&p).await
            } else {
              platform.open_resolved_file(&p, line.filter(|l| *l > 0)).await
            };
            if let Err(e) = res {
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
        let root = manager.folder_for(Some(&self.viewer), None);
        // A folder on an SSH host is searched there; the shell's own search only knows this machine
        if let Some(root) = root.clone().filter(|r| crate::workspace::is_remote(r)) {
          let core = self.clone();
          tokio::spawn(async move {
            let files = match core.workspace(Some(root)).await {
              Ok(ws) => ws.search_files(&query).await,
              Err(_) => vec![],
            };
            core.post_now(HostMsg::Files { seq, files });
          });
          return Ok(());
        }
        let files: Vec<FileHit> = match platform.search_files(root.as_deref(), &query).await {
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
      // Stage an image file into the session's blob store on demand when a file reference is clicked for preview
      W::ImageBlob { session_id, path, seq } => {
        let (core, root) = (self.clone(), manager.session_cwd(&session_id));
        tokio::spawn(async move {
          let blob = async {
            let ws = core.workspace(root).await.ok()?;
            let raw = if crate::platform::file_url::is_file_url(&path) {
              std::path::PathBuf::from(file_url_to_path(&path)?)
            } else {
              resolve_open_path(&ws.path().to_string_lossy(), &path)
            };
            let resolved = if matches!(ws, crate::workspace::Workspace::Local(_)) { normalize(&raw) } else { raw };
            let mime = acpira_shared::attachments::image_mime_of(&resolved.to_string_lossy())?;
            let bytes = ws.read_file(&resolved, crate::limits::MAX_OUT_IMAGE_BYTES).await?;
            let ext = acpira_shared::attachments::ext_of_mime(mime);
            core.manager.save_blob(&session_id, ext, &bytes).await
          }
          .await;
          core.post_now(HostMsg::ImageBlob { seq, blob });
        });
      }
      // The desktop tool panel's reads (project_files.rs). Each is always answered, and runs on its own task: a git
      // status or a large file must not hold up the messages behind it
      W::ProjectDir { session_id, dir, seq } => {
        let (core, root) = (self.clone(), manager.session_cwd(&session_id));
        tokio::spawn(async move {
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
      W::ProjectStatus { session_id, seq } => {
        let (core, root) = (self.clone(), manager.session_cwd(&session_id));
        tokio::spawn(async move {
          let marks = match core.workspace(root.clone()).await {
            Ok(ws) => ws.git_marks().await,
            Err(_) => Default::default(),
          };
          core.post_now(HostMsg::ProjectStatus { seq, root, marks });
        });
      }
      W::ProjectFile { session_id, path, base, seq } => {
        let (core, root) = (self.clone(), manager.session_cwd(&session_id));
        // A `~` / `~/…` path on a local workspace resolves against the user's home, and an absolute path inside `root`
        // shortens to its relative path so git marks and HEAD lookups apply
        let path = match &root {
          Some(r) if !crate::workspace::is_remote(r) => {
            let expanded = if path == "~" {
              home_dir().to_string_lossy().into_owned()
            } else if let Some(rest) = path.strip_prefix("~/").or_else(|| if cfg!(windows) { path.strip_prefix("~\\") } else { None }) {
              home_dir().join(rest).to_string_lossy().into_owned()
            } else {
              path
            };
            let root_path = crate::workspace::folder_path(r);
            match Path::new(&expanded).strip_prefix(&root_path) {
              Ok(rel) if !rel.as_os_str().is_empty() => project_files::relative(&root_path, &expanded),
              _ => expanded,
            }
          }
          _ => path,
        };
        // The session's base is looked up now, while its view is at hand
        let session_base = root.as_ref().and_then(|r| {
          let abs = crate::workspace::folder_path(r).join(&path);
          manager.edit_bases(&session_id).into_iter().find(|(p, _)| Path::new(p) == abs).and_then(|(_, b)| b)
        });
        tokio::spawn(async move {
          let ws = match core.workspace(root).await {
            Ok(ws) => ws,
            Err(e) => {
              core.post_now(HostMsg::ProjectFile { seq, text: None, hunks: vec![], error: Some(e.code()) });
              return;
            }
          };
          let text = match ws.read_text(&path).await {
            Ok(text) => text,
            Err(e) => {
              core.post_now(HostMsg::ProjectFile { seq, text: None, hunks: vec![], error: Some(e.code()) });
              return;
            }
          };
          let base_text = match base {
            Some(ProjectBase::Session) => session_base,
            Some(ProjectBase::Head) => ws.head_text(&path).await.or(session_base),
            None => None,
          };
          let (text, hunks) = match base_text {
            Some(b) => tokio::task::spawn_blocking(move || {
              let h = project_files::hunks(&b, &text);
              (text, h)
            })
            .await
            .unwrap_or_default(),
            None => (text, vec![]),
          };
          core.post_now(HostMsg::ProjectFile { seq, text: Some(text), hunks, error: None });
        });
      }
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
        tokio::spawn(async move {
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
        tokio::spawn(async move {
          if let Err(e) = ra.op(op).await {
            core.platform.toast("error", &format!("{e:#}"));
          }
        });
      }
      W::SshHosts => {
        let (core, connections) = (self.clone(), self.connections.clone());
        tokio::spawn(async move {
          let hosts = connections.ssh_hosts().await;
          core.post_now(HostMsg::SshHosts { hosts, error: None });
        });
      }
      W::RemoteDir { alias, path, seq } => {
        let (core, connections) = (self.clone(), self.connections.clone());
        tokio::spawn(async move {
          let msg = match connections.browse(&alias, path.as_deref()).await {
            Ok((path, entries)) => HostMsg::RemoteDir { seq, alias, path, entries, error: None },
            Err(e) => HostMsg::RemoteDir { seq, alias, path: path.unwrap_or_default(), entries: vec![], error: Some(format!("{e:#}")) },
          };
          core.post_now(msg);
        });
      }
      W::SessionChanges { session_id, seq } => {
        let (core, root, bases) = (self.clone(), manager.session_cwd(&session_id), manager.edit_bases(&session_id));
        tokio::spawn(async move {
          let files = match core.workspace(root).await {
            Ok(ws) => ws.session_changes(bases).await,
            Err(_) => vec![],
          };
          core.post_now(HostMsg::SessionChanges { seq, files });
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
      W::ListNativeSessions { agent, cwd } => {
        // Always answer: the popover is waiting on this agent's list
        match manager.list_native_sessions(&agent, manager.folder_for(Some(&self.viewer), cwd)).await {
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
        let inventory = self.settings.inventory(&agent, &self.folder()).await;
        self.post_now(HostMsg::Inventory { agent, inventory });
      }
      W::Shared => {
        let view = self.settings.shared_view(&self.folder()).await;
        self.post_now(HostMsg::Shared { view, error: None });
      }
      W::SharedAction { action } => {
        let error = match self.settings.shared_action(action, &self.folder()).await {
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
        let view = self.settings.shared_view(&self.folder()).await;
        self.post_now(HostMsg::Shared { view, error });
      }
      W::Storage { detail } => {
        let view = self.manager.storage_view(&detail).await;
        self.post_now(HostMsg::Storage { view, outcome: None, error: None });
      }
      W::StorageAction { action: acpira_shared::storage::StorageAction::Reveal { path }, .. } => match self.manager.storage_reveal(path).await {
        Ok(p) => {
          if let Err(e) = platform.reveal_in_os(&p.to_string_lossy()).await {
            platform.log(&format!("storage: revealing {} failed: {e}", p.display()));
          }
        }
        Err(e) => platform.log(&format!("storage: reveal refused: {e:#}")),
      },
      W::StorageAction { action, detail } => {
        let reply = self.manager.storage_action(action, &detail).await;
        self.post_now(HostMsg::Storage { view: reply.view, outcome: reply.outcome, error: reply.error });
      }
      // The plugin page (plugins/host.rs): every change, another engine's included, sends the view again
      W::Plugins => {
        {
          let mut sub = self.plugins_sub.lock();
          if sub.is_none() {
            let weak = self.me.clone();
            *sub = Some(self.plugins.subscribe(Arc::new(move || {
              if let Some(c) = weak.upgrade() {
                tokio::spawn(async move {
                  let view = c.plugins_view().await;
                  c.post_now(HostMsg::Plugins { view, error: None });
                });
              }
            })));
          }
        }
        let view = self.plugins_view().await;
        self.post_now(HostMsg::Plugins { view, error: None });
      }
      W::PluginAction { action } => {
        let error = match self.plugins.action(action).await {
          Ok(Some(dir)) => {
            if let Err(e) = platform.reveal_in_os(&dir.to_string_lossy()).await {
              platform.log(&format!("plugins: revealing {} failed: {e}", dir.display()));
            }
            None
          }
          Ok(None) => None,
          Err(e) => Some(format!("{e:#}")),
        };
        let view = self.plugins_view().await;
        self.post_now(HostMsg::Plugins { view, error });
      }
      // A card's thumbnail: the backdrop of the version it offers, read from the frozen copy
      W::PluginBackdrop { plugin, snapshot } => {
        let (me, plugins) = (self.clone(), self.plugins.clone());
        tokio::spawn(async move {
          let (p, s) = (plugin.clone(), snapshot.clone());
          let html = tokio::task::spawn_blocking(move || plugins.backdrop_of(&p, &s)).await.ok().flatten();
          me.post_now(HostMsg::PluginBackdrop { plugin, snapshot, html });
        });
      }
      // A plugin method may take its process's full time limit: off the message loop, answered by id
      W::PluginCall { id, plugin, method, params } => {
        let me = self.clone();
        tokio::spawn(async move {
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
      }
      // Network round trips (a test call takes seconds): off the message loop, answered by id
      W::ProviderProbe { id, probe } => {
        let me = self.clone();
        tokio::spawn(async move {
          let outcome = me.settings.provider_probe(probe).await;
          me.post_now(HostMsg::ProviderProbed { id, outcome });
        });
      }
      W::Controls { agent, fresh } => {
        if fresh != Some(true) {
          let controls = manager.known_controls(&agent).await;
          let shapes = manager.model_shapes(&agent);
          self.post_now(HostMsg::Controls { agent, controls, shapes });
        } else {
          // The probe's initialize brings the version back too; a fresh inventory keeps the facts card in step
          let controls = manager.probe_controls(&agent).await;
          let shapes = manager.model_shapes(&agent);
          self.post_now(HostMsg::Controls { agent: agent.clone(), controls, shapes });
          let inventory = self.settings.inventory(&agent, &self.folder()).await;
          self.post_now(HostMsg::Inventory { agent, inventory });
        }
      }
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

  /// A session folder's files (`workspace.rs`); "noProject" without one
  async fn workspace(&self, root: Option<String>) -> Result<crate::workspace::Workspace, project_files::ProjectError> {
    let root = root.ok_or_else(|| project_files::ProjectError::Other("noProject".into()))?;
    crate::workspace::Workspace::open(&root, Some(&self.connections)).await
  }

  /// A file on an SSH host fetched into its read-only copy here (`remote_copy_path`), then opened as a local file is. Every
  /// open fetches again, so the copy shows the host's file as it is now
  async fn open_remote_file(&self, folder: String, alias: String, host_path: String, copy: std::path::PathBuf, line: Option<i64>) {
    let fetched = match self.workspace(Some(folder)).await {
      Ok(ws) => ws.read_file(Path::new(&host_path), REMOTE_OPEN_MAX).await,
      Err(e) => {
        self.platform.log(&format!("openFile {alias}:{host_path}: {e:?}"));
        None
      }
    };
    let Some(bytes) = fetched else {
      self.platform.toast("error", &tp("host.remoteFileFailed", &[("path", &host_path), ("host", &alias)]));
      return;
    };
    let res = match write_read_only(&copy, &bytes).await {
      Ok(()) => self.platform.open_resolved_file(&copy.to_string_lossy(), line).await,
      Err(e) => Err(e.into()),
    };
    if let Err(e) = res {
      self.platform.toast("error", &e.to_string());
    }
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
    let HostMsg::Session { parts, .. } = &m else { return self.emit(m) };
    let mut sent = self.sent.lock();
    let next = parts.as_ref().filter(|_| self.patches.load(std::sync::atomic::Ordering::Acquire));
    match sent.deliver(next) {
      // Older than what the page shows: it carries nothing the page does not have and must not become a patch base
      Delivery::Stale => {}
      Delivery::Patch(patch) => self.emit(HostMsg::SessionPatch { patch }),
      Delivery::Whole => self.emit(m),
    }
  }

  pub fn post_now(&self, m: HostMsg) {
    self.emit(m);
  }

  /// Every post to the page goes through here. The read guard is held across the post, so once `dispose` has taken the
  /// write side nothing is mid-post and nothing posts again: a batch timer or a handler still running after detachView
  /// cannot reach the shell behind the detach
  fn emit(&self, m: HostMsg) {
    let disposed = self.disposed.read();
    if !*disposed {
      (self.post)(m);
    }
  }

  pub fn push_appearance(&self) {
    self.post_now(HostMsg::Appearance { appearance: (self.appearance)() });
  }

  /// The plugin page's view, built off the runtime threads (it reads and hashes the source folders)
  /// Send the theme layer when it differs from what the page has (nothing for a page that never had one and gets none)
  async fn push_theme(&self) {
    let plugins = self.plugins.clone();
    let Ok((css, skin, generation)) = tokio::task::spawn_blocking(move || plugins.theme()).await else { return };
    {
      // Compared without the generation: a commit that left the look alone sends nothing
      let key = format!("{css}\u{0}{}", skin.as_ref().map(|s| format!("{}/{}", s.plugin, s.snapshot)).unwrap_or_default());
      let mut last = self.theme.lock();
      if last.as_deref() == Some(key.as_str()) || (last.is_none() && css.is_empty() && skin.is_none()) {
        return;
      }
      *last = Some(key);
    }
    self.post_now(HostMsg::PluginTheme { css, skin, generation });
  }

  /// Send the interface pieces when they differ from what the page has, like the theme layer
  async fn push_ui(&self) {
    let plugins = self.plugins.clone();
    let Ok(slots) = tokio::task::spawn_blocking(move || plugins.ui_slots()).await else { return };
    {
      let mut last = self.ui.lock();
      if last.as_ref() == Some(&slots) || (last.is_none() && slots.is_empty()) {
        return;
      }
      *last = Some(slots.clone());
    }
    self.post_now(HostMsg::PluginUi { slots });
  }

  async fn plugins_view(&self) -> acpira_shared::plugins::PluginsView {
    let plugins = self.plugins.clone();
    tokio::task::spawn_blocking(move || plugins.view()).await.unwrap_or_default()
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

/// Where a file on an SSH host is copied to open here: `<data root>/run/remote/files/<alias>/<host path>`, a cache apart
/// from the Claude stand-in folders, which must stay empty. `host_path` is normalized (no `..`); each segment is made a
/// safe local name, so neither an alias like `..` nor a character Windows refuses leaves the cache or fails the write
fn remote_copy_path(data_root: &Path, alias: &str, host_path: &str) -> std::path::PathBuf {
  let safe = |seg: &str| -> String {
    let s: String = seg.chars().map(|c| if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\\') { '_' } else { c }).collect();
    if s.chars().all(|c| c == '.') { s.replace('.', "_") } else { s }
  };
  let mut p = data_root.join("run").join("remote").join("files").join(safe(alias));
  for seg in host_path.split('/').filter(|s| !s.is_empty()) {
    p.push(safe(seg));
  }
  p
}

/// The copy written tmp + rename and left read-only, so an edit there is not mistaken for one on the host. The old copy
/// goes first: Windows refuses to rename over a read-only file
async fn write_read_only(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
  let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent folder"))?;
  tokio::fs::create_dir_all(dir).await?;
  let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
  tokio::fs::write(&tmp, bytes).await?;
  let mut perms = tokio::fs::metadata(&tmp).await?.permissions();
  perms.set_readonly(true);
  tokio::fs::set_permissions(&tmp, perms).await?;
  let _ = tokio::fs::remove_file(path).await;
  tokio::fs::rename(&tmp, path).await
}

fn resolve_open_path(cwd: &str, path: &str) -> std::path::PathBuf {
  if path == "~" {
    return home_dir();
  }
  if let Some(rest) = path.strip_prefix("~/").or_else(|| if cfg!(windows) { path.strip_prefix("~\\") } else { None }) {
    return home_dir().join(rest);
  }
  Path::new(cwd).join(path)
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  use super::remote_copy_path;

  #[test]
  fn remote_copy_stays_in_the_cache() {
    let root = Path::new("/d");
    let cache = Path::new("/d/run/remote/files");
    assert_eq!(remote_copy_path(root, "gen1", "/home/u/a.md"), cache.join("gen1/home/u/a.md"));
    // An alias may be `..` or carry characters Windows refuses in a name
    assert_eq!(remote_copy_path(root, "..", "/x"), cache.join("__/x"));
    assert_eq!(remote_copy_path(root, "u@h:22", "/x"), cache.join("u@h_22/x"));
    assert!(remote_copy_path(root, "h", "/a/b:c?.txt").starts_with(cache.join("h")));
  }
}
