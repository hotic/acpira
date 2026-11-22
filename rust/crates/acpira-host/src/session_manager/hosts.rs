//! Host sessions on this engine's side (docs/dev/remote.md): every SSH host that is not turned off has its engine reached
//! as a peer (`remote::peer`), whichever agents run where, since a host folder has one history, the host's own. One scoped view
//! there brings the registered projects' list, which joins the local list with each folder as its `ssh://` form; a host session shown
//! here gets a view of its own there, whose pushes are rebuilt into this engine's views (its own revs, so pages take
//! patches as for any session). Page actions on a host session go to that view, list actions on one to the list view. The
//! agent decides only where a new session starts: here for those the `sshLocalAgents` setting names (their tools run on
//! the host, `acp/session/remote.rs`), on the host for every other. The last list a host sent stays (also on disk) while
//! it is away

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use acpira_shared::context::{ContextContent, ContextRaw, ContextReport};
use acpira_shared::protocol::{EditTurnRequest, HostMsg, SessionHit};
use acpira_shared::remote::{ConnectionView, format_remote_folder, parse_remote_folder};
use acpira_shared::session_patch::{LastTurnParts, ViewParts};
use acpira_shared::sidecar::SessionListScope;
use acpira_shared::model_shapes::ModelShapes;
use acpira_shared::transcript::{ConfigControl, SessionSummary, SlashCommand};

use super::{SessionManager, Viewer, remote_view};
use crate::store::record::SessionRecord;
use crate::remote::peer::{PeerHello, PeerLink};

mod actions;
mod blobs;
mod events;

use self::blobs::without_blobs;

/// How long one attempt waits for the host's helper channel
const CONNECT_WAIT: Duration = Duration::from_secs(60);
/// Between attempts, doubling from the first
const RETRY_FIRST: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(30);
const LIST_VIEW: &str = "list";
/// How long a new session in a host folder waits for the host's engine to be reached
const NEW_WAIT: Duration = Duration::from_secs(60);
/// How long a page opening a host session waits for the host's engine before it is told why no view comes
const OPEN_WAIT: Duration = Duration::from_secs(30);
/// What this engine serves for the host's engine (the requests its views cause come back here).
/// Terminal commands run on the host through `ssh -t`; a fresh SSH login still has its own security session and
/// cannot unlock a background engine's macOS keychain. `openBrowser` forwards to this engine's shell with `folder` /
/// `cwd` rewritten to `ssh://<alias>/...` so the preview proxy tunnels the host's loopback ports.
const PEER_CAPS: &[&str] = &["toast", "runInTerminal", "openBrowser"];
const PEER_CAPS_NO_BROWSER: &[&str] = &["toast", "runInTerminal"];
/// Blob fetches from one host at a time: a session full of pictures must not queue the view stream behind them all
const BLOB_FETCHES: usize = 3;
/// The largest blob fetched without being asked for; a larger one stays on the host and its view shows it as missing
const AUTO_BLOB_MAX: u64 = 64 << 20;
/// Blobs landing this close together put their session's view out once
const BLOB_SETTLE: Duration = Duration::from_millis(200);
/// A cut-off fetch nobody resumed within this long is dropped
const PARTIAL_KEEP: Duration = Duration::from_secs(7 * 24 * 3600);
/// A bounded on-demand metadata query; disconnected hosts never block a history search.
const QUERY_WAIT: Duration = Duration::from_secs(15);

#[derive(Default)]
pub(super) struct Hosts {
  runs: parking_lot::Mutex<HashMap<String, Arc<HostRun>>>,
  sub: parking_lot::Mutex<Option<u64>>,
}

struct HostRun {
  alias: String,
  /// Envelopes to the host's engine while it is connected
  out: parking_lot::Mutex<Option<mpsc::UnboundedSender<Value>>>,
  /// Whether it is connected now
  up: tokio::sync::watch::Sender<bool>,
  /// The host's list as this engine shows it (folders as `ssh://` forms, no categories of the host's)
  items: parking_lot::Mutex<Vec<SessionSummary>>,
  folders: parking_lot::Mutex<Option<Vec<String>>>,
  /// One-shot search / id lookups. Their results never join the persistent project list.
  requests: parking_lot::Mutex<HashMap<String, tokio::sync::oneshot::Sender<Value>>>,
  /// Host sessions shown here, each through a view of its own on the host, by session id
  shown: parking_lot::Mutex<HashMap<String, Shown>>,
  /// Views that asked the host for a new session, by view id: the viewer to put on it once it shows
  asked: parking_lot::Mutex<HashMap<String, Weak<Viewer>>>,
  /// Delete completions return to the requesting page, even after its shown session was detached.
  deletions: parking_lot::Mutex<HashMap<String, (String, Weak<Viewer>)>>,
  /// Keep the route to an undo after the session has left the host's list.
  deleted: parking_lot::Mutex<HashMap<String, tokio::time::Instant>>,
  /// The host engine's pid while connected (tests stop it)
  pid: parking_lot::Mutex<Option<u32>>,
  seq: AtomicU64,
  task: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
  /// Blobs being fetched from the host into this machine's store, as `<session>/<name>`
  blobs: parking_lot::Mutex<HashSet<String>>,
  /// Fetches running at once (`BLOB_FETCHES`)
  slots: Arc<tokio::sync::Semaphore>,
  /// Sessions whose view goes out again shortly, with the blobs that landed meanwhile (`blob_landed`)
  settling: parking_lot::Mutex<HashSet<String>>,
  /// Public account metadata for this host only; never merged into this machine's account list.
  auth: parking_lot::Mutex<Value>,
  /// The host engine's last `controls` answer per agent, for a new session page while the host is away
  controls: parking_lot::Mutex<HashMap<String, Value>>,
}

/// A host engine's answer to `controls`
#[derive(Debug, Default)]
pub struct HostControls {
  pub controls: Vec<ConfigControl>,
  pub shapes: Option<ModelShapes>,
  pub commands: Vec<SlashCommand>,
}

/// One host session's view here
struct Shown {
  view_id: String,
  /// This engine's line of views for it (a page patches only within one)
  epoch: u64,
  /// The rev of the host's view the parts below were built from; a patch applies only on top of it
  host_rev: Option<i64>,
  parts: Option<ViewParts>,
  msg: Option<HostMsg>,
  /// Blobs the view names that are not on this machine (yet): left out of what pages get, since a page loads a blob
  /// from this machine's store and never reloads one that failed
  absent: HashSet<String>,
}

impl Shown {
  fn new(view_id: String) -> Shown {
    Shown { view_id, epoch: crate::acp::session::next_view_epoch(), host_rev: None, parts: None, msg: None, absent: HashSet::new() }
  }

  /// Keep the host's view as `parts` (what its patches apply to) and build what pages get from it, without the
  /// `absent` blobs
  fn publish(&mut self, id: &str, head: &RawValue, rev: i64, turns: Vec<Box<RawValue>>, last: Option<LastTurnParts>) -> Option<HostMsg> {
    let (parts, msg) = remote_view(id, head, rev, self.epoch, turns, last)?;
    let msg = if self.absent.is_empty() { msg } else { without_blobs(id, &parts, &self.absent, rev, self.epoch).unwrap_or(msg) };
    self.parts = Some(parts);
    self.msg = Some(msg.clone());
    Some(msg)
  }
}

/// Cancellation drops pending replies and closes temporary lookup views as well as timeouts do.
struct HostRequest<'a> {
  run: &'a HostRun,
  key: String,
}

impl Drop for HostRequest<'_> {
  fn drop(&mut self) {
    self.run.requests.lock().remove(&self.key);
    if self.key.starts_with("lookup-") {
      self.run.send(json!({ "type": "detachView", "viewId": self.key }));
    }
  }
}

impl HostRun {
  async fn request(&self, key: String, send: impl FnOnce()) -> Option<Value> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    self.requests.lock().insert(key.clone(), tx);
    let _pending = HostRequest { run: self, key };
    send();
    tokio::time::timeout(QUERY_WAIT, rx).await.ok().and_then(Result::ok)
  }

  /// Request on the host's list view, failing immediately when the link is down rather than waiting for `QUERY_WAIT`.
  async fn request_list(&self, key: String, message: Value) -> Option<Value> {
    if !*self.up.borrow() {
      return None;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _pending = HostRequest { run: self, key: key.clone() };
    {
      // Register under `out.lock()` so a concurrent disconnect clears it instead of stranding the waiter.
      let out = self.out.lock();
      let out = out.as_ref()?;
      self.requests.lock().insert(key.clone(), tx);
      if out.send(json!({ "type": "webviewMessage", "viewId": LIST_VIEW, "message": message })).is_err() {
        return None;
      }
    }
    tokio::time::timeout(QUERY_WAIT, rx).await.ok().and_then(Result::ok)
  }

  async fn context(&self, id: &str, detail: bool) -> Option<ContextReport> {
    let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
    let reply = self
      .request_list(
        format!("context-{seq}"),
        json!({ "type": "sessionContext", "sessionId": id, "seq": seq, "detail": detail }),
      )
      .await?;
    reply.get("report").and_then(|r| serde_json::from_value(r.clone()).ok())
  }

  async fn context_item(&self, id: &str, rev: &str, item_id: &str) -> Option<ContextContent> {
    let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
    let reply = self
      .request_list(
        format!("context-item-{seq}"),
        json!({ "type": "sessionContextItem", "sessionId": id, "rev": rev, "id": item_id, "seq": seq }),
      )
      .await?;
    reply.get("content").and_then(|c| serde_json::from_value(c.clone()).ok())
  }

  async fn context_raw(&self, id: &str) -> Result<Option<ContextRaw>, String> {
    let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
    let Some(reply) = self
      .request_list(
        format!("context-raw-{seq}"),
        json!({ "type": "sessionContextRaw", "sessionId": id, "seq": seq }),
      )
      .await
    else {
      return Err(crate::i18n::tp("host.remoteUnreachable", &[("host", &self.alias)]));
    };
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
      return Err(error.to_owned());
    }
    Ok(reply.get("raw").and_then(|r| serde_json::from_value(r.clone()).ok()))
  }

  /// An edit answers only after the host accepted or rejected it. A lost link cannot tell whether it ran.
  async fn edit(&self, edit: EditTurnRequest) -> Result<()> {
    let key = format!("edit-{}", self.next_view_id());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _pending = HostRequest { run: self, key: key.clone() };
    let unreachable = || anyhow!(crate::i18n::tp("host.remoteUnreachable", &[("host", &self.alias)]));
    {
      // Register before sending, while the driver cannot clear the connection and its pending requests.
      let out = self.out.lock();
      let Some(out) = out.as_ref() else { return Err(unreachable()) };
      self.requests.lock().insert(key.clone(), tx);
      let msg = json!({ "type": "editTurn", "requestId": key, "edit": edit });
      if out.send(json!({ "type": "webviewMessage", "viewId": LIST_VIEW, "message": msg })).is_err() {
        self.requests.lock().remove(&key);
        return Err(unreachable());
      }
    }
    let msg = rx.await.map_err(|_| anyhow!("{}: {}", self.alias, crate::i18n::t("subagents.disconnectedNote")))?;
    match msg.get("error") {
      None | Some(Value::Null) => Ok(()),
      Some(Value::String(error)) => Err(anyhow!(error.clone())),
      _ => Err(anyhow!("invalid edit result from {}", self.alias)),
    }
  }

  async fn lookup(&self, id: &str) -> Option<SessionSummary> {
    // Like `search`: a host that is not connected is not waited for, or every pick of a session this engine does not
    // list would hang on an unreachable host. Sessions a host listed before (`owns`) never come here
    if !*self.up.borrow() {
      return None;
    }
    let view_id = format!("lookup-{}", self.next_view_id());
    let reply = self
      .request(view_id.clone(), || {
        self.open_view(
          &view_id,
          json!({
            "sessionList": { "folders": [], "ids": [id] }
          }),
        )
      })
      .await?;
    let found = reply.pointer("/state/sessions")?.as_array()?.iter().find(|s| s["id"] == id)?;
    let mut summary: SessionSummary = serde_json::from_value(found.clone()).ok()?;
    summary.cwd = local_folder(&self.alias, &summary.cwd);
    summary.category = None;
    Some(summary)
  }

  /// A host session's record as the host's engine holds it, asked as a JSON export (`sessionMarkdown`, the same request
  /// a paired device saves from), its folder in this engine's `ssh://` form
  async fn record(&self, id: &str) -> Result<SessionRecord> {
    let unreachable = || anyhow!(crate::i18n::tp("host.remoteUnreachable", &[("host", &self.alias)]));
    if !*self.up.borrow() {
      return Err(unreachable());
    }
    let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
    let reply = self
      .request(format!("record-{seq}"), || {
        self.send_to_view(LIST_VIEW, json!({ "type": "sessionMarkdown", "id": id, "seq": seq, "format": "json" }));
      })
      .await
      .ok_or_else(unreachable)?;
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
      bail!("{error}");
    }
    let text = reply.get("markdown").and_then(Value::as_str).ok_or_else(|| anyhow!("invalid sessionMarkdown reply from {}", self.alias))?;
    let mut record: SessionRecord = serde_json::from_str(text)?;
    record.cwd = local_folder(&self.alias, &record.cwd);
    Ok(record)
  }

  async fn search(&self, query: &str) -> Vec<SessionHit> {
    if !*self.up.borrow() {
      return vec![];
    }
    let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
    let reply = self
      .request(format!("search-{seq}"), || {
        self.send_to_view(LIST_VIEW, json!({ "type": "searchSessions", "query": query, "seq": seq }));
      })
      .await;
    let mut hits: Vec<SessionHit> =
      reply.and_then(|v| v.get("hits").and_then(|h| serde_json::from_value(h.clone()).ok())).unwrap_or_default();
    hits.retain(|h| h.summary.as_ref().is_some_and(|s| !acpira_shared::remote::is_remote_folder(&s.cwd)));
    for hit in &mut hits {
      if let Some(s) = &mut hit.summary {
        s.cwd = local_folder(&self.alias, &s.cwd);
        s.category = None;
      }
    }
    hits
  }

  fn send(&self, v: Value) -> bool {
    self.out.lock().as_ref().is_some_and(|tx| tx.send(v).is_ok())
  }

  fn send_to_view(&self, view_id: &str, message: Value) -> bool {
    self.send(json!({ "type": "webviewMessage", "viewId": view_id, "message": message }))
  }

  /// A view on the host, ready at once (with patches, except the list's)
  fn open_view(&self, view_id: &str, initial: Value) {
    self.send(json!({ "type": "attachView", "viewId": view_id, "host": "sidebar", "initial": initial }));
    self.send_to_view(view_id, json!({ "type": "ready", "patches": view_id != LIST_VIEW }));
  }

  fn next_view_id(&self) -> String {
    format!("v{}", self.seq.fetch_add(1, Ordering::Relaxed) + 1)
  }

  fn owns_unlisted(&self, id: &str) -> bool {
    let undo = {
      let mut deleted = self.deleted.lock();
      deleted.retain(|_, until| *until > tokio::time::Instant::now());
      deleted.contains_key(id)
    };
    self.shown.lock().contains_key(id) || undo
  }
}

/// A host folder as this engine names it; the path as it is when it is not absolute
fn local_folder(alias: &str, path: &str) -> String {
  format_remote_folder(alias, path).unwrap_or_else(|| path.to_owned())
}

fn host_folder_selected(alias: &str, cwd: &str, folders: Option<&[String]>) -> bool {
  parse_remote_folder(cwd).is_some_and(|f| f.alias == alias && folders.is_none_or(|paths| paths.contains(&f.path)))
}

impl SessionManager {
  /// Follow the connection list: a host that is on gets its engine reached, one turned off or removed is let go
  pub(super) fn start_hosts(self: &Arc<Self>) {
    let Some(connections) = self.deps.remote.clone() else { return };
    let weak = Arc::downgrade(self);
    let id = connections.subscribe(Arc::new(move |views: &[ConnectionView]| {
      if let Some(m) = weak.upgrade() {
        m.sync_hosts(views);
      }
    }));
    *self.hosts.sub.lock() = Some(id);
    self.sync_hosts(&connections.views());
    // Cut-off blob fetches of sessions nobody opened again (`fetch_host_blob`)
    let partials = self.deps.store.partial_dir();
    tokio::task::spawn_blocking(move || {
      for e in std::fs::read_dir(&partials).into_iter().flatten().flatten() {
        if e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > PARTIAL_KEEP) {
          let _ = std::fs::remove_file(e.path());
        }
      }
    });
  }

  pub(super) fn stop_hosts(&self) {
    if let (Some(id), Some(c)) = (self.hosts.sub.lock().take(), self.deps.remote.as_ref()) {
      c.unsubscribe(id);
    }
    for (_, run) in self.hosts.runs.lock().drain() {
      if let Some(t) = run.task.lock().take() {
        t.abort();
      }
      *run.out.lock() = None;
      run.requests.lock().clear();
    }
  }

  /// The set of windows changed: SSH host lists follow what the connected windows show (`host_folders`)
  pub(super) fn viewers_moved(&self) {
    if !self.hosts.runs.lock().is_empty() {
      self.sync_host_scopes();
    }
  }

  /// Whether host lists narrow to the registered projects: only while every connected window is a project shell (a
  /// window with a workspace lists all of a host's sessions); with no window's viewer, the engine-wide shell decides
  fn lists_by_project(&self) -> bool {
    let shells: Vec<bool> = self.viewers().iter().filter_map(|v| v.shell.as_ref().map(|s| (s.projects)())).collect();
    if shells.is_empty() {
      return (self.deps.projects)();
    }
    shells.into_iter().all(|p| p)
  }

  fn host_folders(&self, alias: &str) -> Option<Vec<String>> {
    if !self.lists_by_project() {
      return None;
    }
    Some(
      self
        .categories()
        .projects
        .iter()
        .flatten()
        .filter_map(|cwd| parse_remote_folder(cwd))
        .filter(|f| f.alias == alias)
        .map(|f| f.path)
        .collect(),
    )
  }

  /// Narrow both memory and the offline cache before changing the subscription, including while disconnected.
  pub(super) fn sync_host_scopes(&self) {
    let runs: Vec<_> = self.hosts.runs.lock().values().cloned().collect();
    for run in runs {
      let mut scope = run.folders.lock();
      let folders = self.host_folders(&run.alias);
      if *scope == folders {
        continue;
      }
      *scope = folders;
      let mut items = run.items.lock();
      items.retain(|s| host_folder_selected(&run.alias, &s.cwd, scope.as_deref()));
      self.save_host_list(&run.alias, &items);
      run.send(json!({ "type": "setSessionList", "viewId": LIST_VIEW, "scope": { "folders": *scope } }));
    }
    self.emit_sessions();
  }

  fn sync_hosts(self: &Arc<Self>, views: &[ConnectionView]) {
    let wanted: Vec<&str> = views.iter().filter(|v| !v.disabled).map(|v| v.alias.as_str()).collect();
    let mut changed = false;
    {
      let mut runs = self.hosts.runs.lock();
      runs.retain(|alias, run| {
        let keep = wanted.contains(&alias.as_str());
        if !keep {
          if let Some(t) = run.task.lock().take() {
            t.abort();
          }
          *run.out.lock() = None;
          run.requests.lock().clear();
          changed = true;
        }
        keep
      });
      for alias in wanted {
        if runs.contains_key(alias) {
          continue;
        }
        let folders = self.host_folders(alias);
        let items = self.cached_host_list(alias, folders.as_deref());
        let run = Arc::new(HostRun {
          alias: alias.to_owned(),
          out: Default::default(),
          up: tokio::sync::watch::Sender::new(false),
          items: parking_lot::Mutex::new(items),
          folders: parking_lot::Mutex::new(folders),
          requests: Default::default(),
          shown: Default::default(),
          asked: Default::default(),
          deletions: Default::default(),
          deleted: Default::default(),
          pid: Default::default(),
          seq: Default::default(),
          task: Default::default(),
          blobs: Default::default(),
          slots: Arc::new(tokio::sync::Semaphore::new(BLOB_FETCHES)),
          settling: Default::default(),
          auth: parking_lot::Mutex::new(json!({ "agents": [], "accounts": [], "actions": [] })),
          controls: Default::default(),
        });
        *run.task.lock() = Some(tokio::spawn(drive(Arc::downgrade(self), run.clone())));
        runs.insert(alias.to_owned(), run);
        changed = true;
      }
    }
    if changed {
      self.emit_sessions();
    }
  }

  /// A copy of a session this engine drove left a host (`remote::host_store`). With its id, as it is deleted here: the
  /// host's list drops it at once, else it would show again as one of the host's own. Without: the host removed the
  /// copies, so its engine is asked for the list again, read from its directory
  pub fn host_copy_gone(self: &Arc<Self>, alias: &str, id: Option<&str>) {
    let Some(run) = self.hosts.runs.lock().get(alias).cloned() else { return };
    let Some(id) = id else {
      run.send_to_view(LIST_VIEW, json!({ "type": "ready", "patches": false }));
      return;
    };
    let left = {
      let mut items = run.items.lock();
      let before = items.len();
      items.retain(|s| s.id != id);
      (items.len() != before).then(|| items.clone())
    };
    if let Some(items) = left {
      self.save_host_list(alias, &items);
      self.emit_sessions();
    }
  }

  /// Retire a source-side record after its host took ownership. A running turn is never cut off by reconciliation.
  pub async fn retire_host_copy(self: &Arc<Self>, id: &str, host: &SessionRecord) -> Result<()> {
    let op = self.session_op(id).await;
    if !self.yielding.lock().insert(id.to_owned()) {
      return Err(anyhow!("the local session is handing over"));
    }
    // Block new prompt claims while snapshots and blobs are copied, including across filesystem awaits.
    struct Retiring<'a>(&'a parking_lot::Mutex<HashSet<String>>, String);
    impl Drop for Retiring<'_> {
      fn drop(&mut self) { self.0.lock().remove(&self.1); }
    }
    let _retiring = Retiring(&self.yielding, id.to_owned());
    let pin = match self.leases.claim(id) {
      crate::store::session_lease::Claim::Held(pin) => pin,
      _ => return Err(anyhow!("the local session is held by another engine")),
    };
    let hold = super::leases::LeaseHold { leases: &self.leases, id, pin };
    let live = self.live(id);
    if live.as_ref().is_some_and(|s| s.in_flight()) {
      return Err(anyhow!("the local turn has not settled"));
    }
    let local = match &live {
      Some(s) => Some(s.to_record()),
      None => self.deps.store.load(id).await,
    };
    if let Some(local) = &local {
      crate::remote::host_store::preserve_branch(&self.deps.store, local, host).await?;
    }
    let alias = local.as_ref().and_then(|r| parse_remote_folder(&r.cwd)).map(|f| f.alias);
    let viewers = self.viewers_on(id, None);
    self.forget(id);
    let closing = self.state.lock().closing.get(id).cloned();
    if let Some(mut closing) = closing { let _ = closing.wait_for(|done| *done).await; }
    self.deps.store.remove(id).await;
    {
      let mut st = self.state.lock();
      st.index.retain(|s| s.id != id);
      for items in st.elsewhere.values_mut() { items.retain(|s| s.id != id); }
    }
    self.sync_leases();
    self.refresh_index().await;
    self.emit_sessions();
    drop(hold);
    // The host may not have republished its list yet. Seed its authoritative summary before reopening viewers, or a
    // transiently empty peer list would turn this deliberate retirement into an ordinary missing-record fallback.
    if let Some(run) = alias.as_ref().and_then(|a| self.hosts.runs.lock().get(a).cloned()) {
      let mut summary = host.summary();
      summary.cwd = local_folder(&run.alias, &summary.cwd);
      let mut items = run.items.lock();
      items.retain(|s| s.id != id);
      items.push(summary);
      drop(items);
    }
    drop(op);
    for viewer in viewers { self.select_session_for(&viewer, id).await; }
    Ok(())
  }

  /// The host engines this engine is connected to, with their pids
  pub fn host_engines(&self) -> Vec<(String, u32)> {
    self.hosts.runs.lock().values().filter_map(|r| r.pid.lock().map(|p| (r.alias.clone(), p))).collect()
  }

  /// The host whose session this is. A session this machine has itself (live, stored, or run by another engine here) is
  /// never a host's: the host's list also holds the copies of this machine's sessions in its folders (`remote::host_store`)
  fn locally_owned(&self, id: &str) -> bool {
    let st = self.state.lock();
    st.live.contains_key(id)
      || st.index.iter().any(|s| s.id == id)
      || st.elsewhere.values().flatten().any(|s| s.id == id)
      || st.trash.contains_key(id)
  }

  fn cache_file(&self, alias: &str) -> PathBuf {
    self.deps.lease_root.join("run").join("remote").join(format!("{alias}.sessions.json"))
  }

  fn cached_host_list(&self, alias: &str, folders: Option<&[String]>) -> Vec<SessionSummary> {
    let mut items: Vec<SessionSummary> =
      std::fs::read_to_string(self.cache_file(alias)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let before = items.len();
    items.retain(|s| host_folder_selected(alias, &s.cwd, folders));
    if before != items.len() {
      self.save_host_list(alias, &items);
    }
    items
  }

  fn save_host_list(&self, alias: &str, items: &[SessionSummary]) {
    let path = self.cache_file(alias);
    let Ok(text) = serde_json::to_string(items) else { return };
    // Every engine on this machine keeps the same cache: a temp file of its own each
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let saved = path.parent().map(std::fs::create_dir_all).transpose().and_then(|_| std::fs::write(&tmp, text)).and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = saved {
      self.log(&format!("remote run: {} list not cached: {e}", alias));
    }
  }
}

/// One host's connection, for as long as the host stays in remote run: reach its engine, open the list view and the
/// views of the sessions shown here, carry envelopes until the connection ends, then try again with backoff
async fn drive(weak: Weak<SessionManager>, run: Arc<HostRun>) {
  let mut retry = RETRY_FIRST;
  loop {
    let Some(m) = weak.upgrade() else { return };
    let Some(connections) = m.deps.remote.clone() else { return };
    let caps = if m.deps.browser.is_some() { PEER_CAPS } else { PEER_CAPS_NO_BROWSER };
    drop(m);
    let attempt: Result<(PeerLink, crate::remote::client::RemoteClient)> = async {
      // What an earlier link heard may be another engine's by now (an update; a run cancelled mid-link never got to
      // forget it): unknown until this link's hello
      connections.forget_engine_features(&run.alias);
      let client = connections.follow(&run.alias, CONNECT_WAIT).await?;
      let link = PeerLink::connect(&client, None, PeerHello { version: acpira_shared::version::VERSION, capabilities: caps, host_language: "en" }).await?;
      Ok((link, client))
    }
    .await;
    match attempt {
      Err(e) => {
        if let Some(m) = weak.upgrade() {
          m.log(&format!("remote run: {}'s engine: {e:#}", run.alias));
        }
      }
      Ok((mut link, client)) => {
        retry = RETRY_FIRST;
        *run.pid.lock() = link.hello_ok.pointer("/sidecar/pid").and_then(Value::as_u64).map(|p| p as u32);
        // What the host's engine takes decides how this machine's settings go there (`remote/host_models.rs`)
        let features = link.hello_ok.get("features").and_then(Value::as_array).map(|f| f.iter().filter_map(Value::as_str).map(str::to_owned).collect());
        connections.set_engine_features(&run.alias, &client, features.unwrap_or_default());
        drop(client);
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        *run.out.lock() = Some(tx);
        {
          // Serialize attach with project changes: an update must never overtake the view's creation.
          let folders = run.folders.lock();
          run.open_view(LIST_VIEW, json!({ "sessionList": SessionListScope { folders: folders.clone(), ids: vec![] } }));
        }
        run.up.send_replace(true);
        // Back after a drop: the sessions shown here get their views again, whole first
        let reopen: Vec<(String, String)> = {
          let mut shown = run.shown.lock();
          shown.values_mut().for_each(|s| s.host_rev = None);
          shown.iter().map(|(id, s)| (id.clone(), s.view_id.clone())).collect()
        };
        for (id, view_id) in reopen {
          run.open_view(&view_id, json!({ "sessionList": { "folders": [] }, "session": id }));
        }
        loop {
          tokio::select! {
            out = rx.recv() => match out {
              Some(v) => link.send(&v),
              None => break,
            },
            env = link.recv() => match env {
              Some(env) => match weak.upgrade() {
                Some(m) => m.on_host_envelope(&run, env),
                None => return,
              },
              None => break,
            },
          }
        }
        run.up.send_replace(false);
        *run.out.lock() = None;
        run.requests.lock().clear();
        *run.pid.lock() = None;
        connections.forget_engine_features(&run.alias);
        if let Some(m) = weak.upgrade() {
          m.log(&format!("remote run: lost {}'s engine", run.alias));
          let error = crate::i18n::tp("host.remoteUnreachable", &[("host", &run.alias)]);
          let pending: Vec<_> = run.deletions.lock().drain().collect();
          for (request_id, (_, viewer)) in pending {
            if let Some(v) = viewer.upgrade() {
              v.emit(HostMsg::DeleteSessionResult { request_id, undo: false, error: Some(error.clone()) });
            }
          }
        }
      }
    }
    // A helper channel that comes up during the backoff ends it: the page shows the host as connected from then on, and
    // every pick or new session would otherwise queue until the backoff ran out (up to RETRY_MAX). A channel that was up
    // already (its engine failed) waits the backoff out, or this would spin
    let wake = weak.upgrade().and_then(|m| m.deps.remote.clone()).filter(|c| !c.is_up(&run.alias));
    match wake {
      Some(c) => tokio::select! {
        _ = tokio::time::sleep(retry) => {}
        _ = c.until_up(&run.alias) => {}
      },
      None => tokio::time::sleep(retry).await,
    }
    retry = (retry * 2).min(RETRY_MAX);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use super::events::{apply_patch, split_view, turns_ended};
  use acpira_shared::session_patch::{ViewPartsData, encode_turns, raw, session_patch};
  use acpira_shared::transcript::Turn;

  fn user(text: &str) -> Turn {
    serde_json::from_value(json!({ "role": "user", "text": text })).unwrap()
  }

  fn agent(blocks: &[&str]) -> Turn {
    let blocks: Vec<Value> = blocks.iter().map(|b| json!({ "type": "text", "markdown": b })).collect();
    serde_json::from_value(json!({ "role": "agent", "blocks": blocks })).unwrap()
  }

  /// A view as the host's engine encodes it, and the same view whole
  fn host_view(rev: i64, mut turns: Vec<Turn>) -> (ViewPartsData, Value) {
    let whole = json!({ "id": "s", "rev": rev, "cwd": "/w", "turns": turns });
    let mut last = turns.pop();
    let before: Vec<&Turn> = turns.iter().collect();
    let (encoded, last) = encode_turns(&before, last.as_mut());
    (
      ViewPartsData {
        id: "s".into(),
        epoch: 7,
        rev,
        head: raw(&json!({ "id": "s", "rev": rev, "cwd": "/w", "turns": [] })),
        turns: encoded,
        last,
      },
      whole,
    )
  }

  fn turns_of(t: &[Box<RawValue>]) -> Vec<Value> {
    t.iter().map(|r| serde_json::from_str(r.get()).unwrap()).collect()
  }

  /// What this engine holds after taking a whole view
  fn held(whole: &Value) -> ViewParts {
    let (head, turns, last) = split_view(whole).unwrap();
    ViewParts(Arc::new(ViewPartsData {
      id: "s".into(),
      epoch: 1,
      rev: 1,
      head: serde_json::value::to_raw_value(&head).unwrap(),
      turns,
      last,
    }))
  }

  #[test]
  fn the_hosts_patches_rebuild_its_views_here() {
    let (v1, whole1) = host_view(1, vec![user("a"), agent(&["x"])]);
    // A streamed block on the live turn: the patch keeps the blocks before it
    let (v2, whole2) = host_view(2, vec![user("a"), agent(&["x", "y"])]);
    let patch: Value = serde_json::from_str(session_patch(&v1, &v2).unwrap().get()).unwrap();
    assert!(patch.get("keepBlocks").is_some(), "{patch}");
    let here = held(&whole1);
    let (turns, last) = apply_patch(&here, &patch).unwrap();
    assert_eq!(turns_of(&turns), whole2["turns"].as_array().unwrap().clone());
    assert_eq!(last.as_ref().map(|l| l.blocks.len()), Some(2));

    // A new turn after it: the patch keeps the turns before it, and the live turn is the new last one
    let here = ViewParts(Arc::new(ViewPartsData { id: "s".into(), epoch: 1, rev: 2, head: here.0.head.clone(), turns, last }));
    let (v3, whole3) = host_view(3, vec![user("a"), agent(&["x", "y"]), user("b"), agent(&["z"])]);
    let patch: Value = serde_json::from_str(session_patch(&v2, &v3).unwrap().get()).unwrap();
    let (turns, last) = apply_patch(&here, &patch).unwrap();
    assert_eq!(turns_of(&turns), whole3["turns"].as_array().unwrap().clone());
    assert_eq!(last.map(|l| l.blocks.len()), Some(1));

    // A patch for more than this engine holds does not apply
    let short = held(&json!({ "id": "s", "rev": 1, "cwd": "/w", "turns": [] }));
    assert!(apply_patch(&short, &patch).is_none());
  }

  #[test]
  fn a_host_list_tells_which_turns_ended() {
    let summary = |id: &str, done: Option<i64>| -> SessionSummary {
      serde_json::from_value(json!({ "id": id, "agent": "droid", "title": "", "cwd": "/w", "updatedAt": "", "completedAt": done })).unwrap()
    };
    let before = [summary("a", Some(1)), summary("b", None), summary("c", Some(5))];
    let items = [summary("a", Some(2)), summary("b", Some(3)), summary("c", Some(5)), summary("new", Some(9))];
    let ended: Vec<String> = turns_ended(&before, &items).into_iter().map(|s| s.id).collect();
    // Moved on and newly set count; unchanged and first seen do not
    assert_eq!(ended, ["a", "b"]);
  }

  #[test]
  fn host_summary_ranking_prefers_newer_completed_owner_over_stale_copy() {
    use super::actions::summary_rank;
    let moved_owner: SessionSummary = serde_json::from_value(json!({
      "id": "s1", "agent": "antigravity", "title": "done", "cwd": "ssh://box-b/repo",
      "updatedAt": "2026-11-22T03:05:23.351Z", "completedAt": 1795317095155_i64, "state": "unread"
    }))
    .unwrap();
    let stale_copy: SessionSummary = serde_json::from_value(json!({
      "id": "s1", "agent": "antigravity", "title": "stopped", "cwd": "ssh://box-a/old",
      "updatedAt": "2026-11-22T03:04:47.908Z"
    }))
    .unwrap();
    // Even though "box-a" sorts before "box-b", the moved owner's fresher summary wins deterministically
    assert!(summary_rank(&moved_owner, true, "box-b") > summary_rank(&stale_copy, true, "box-a"));

    // Live activity outranks an idle record even with identical timestamps
    let working: SessionSummary = serde_json::from_value(json!({
      "id": "s1", "agent": "antigravity", "title": "busy", "cwd": "ssh://box-c/repo",
      "updatedAt": "2026-11-22T03:04:47.908Z", "state": "working"
    }))
    .unwrap();
    assert!(summary_rank(&working, true, "box-c") > summary_rank(&moved_owner, true, "box-b"));

    // Completely tied summaries break ties deterministically by alias
    assert!(summary_rank(&stale_copy, true, "alpha") > summary_rank(&stale_copy, true, "beta"));
  }
}
