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

use acpira_shared::protocol::{EditTurnRequest, HostMsg, RawJson, SessionHit, WebviewMsg};
use acpira_shared::remote::{ConnectionView, format_remote_folder, parse_remote_folder};
use acpira_shared::session_patch::{LastTurnParts, ViewParts};
use acpira_shared::sidecar::SessionListScope;
use acpira_shared::model_shapes::ModelShapes;
use acpira_shared::transcript::{ConfigControl, SessionSummary, SlashCommand};

use super::{SessionManager, Viewer, is_forwarded, remote_view, session_id_of};
use crate::remote::client::Event;
use crate::store::record::SessionRecord;
use crate::remote::peer::{PeerHello, PeerLink};

/// How long one attempt waits for the host's helper channel
const CONNECT_WAIT: Duration = Duration::from_secs(60);
/// Between attempts, doubling from the first
const RETRY_FIRST: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(30);
const LIST_VIEW: &str = "list";
/// How long a new session in a host folder waits for the host's engine to be reached
const NEW_WAIT: Duration = Duration::from_secs(60);
/// What this engine serves for the host's engine (the requests its views cause come back here).
/// A local terminal cannot run host logins, and a fresh SSH login cannot unlock the engine's macOS keychain session.
const PEER_CAPS: &[&str] = &["toast"];
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

  async fn lookup(&self, id: &str) -> bool {
    // Like `search`: a host that is not connected is not waited for, or every pick of a session this engine does not
    // list would hang on an unreachable host. Sessions a host listed before (`owns`) never come here
    if !*self.up.borrow() {
      return false;
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
      .await;
    reply.and_then(|v| v.pointer("/state/sessions").and_then(Value::as_array).map(|s| s.iter().any(|s| s["id"] == id))).unwrap_or(false)
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

  fn owns(&self, id: &str) -> bool {
    let undo = {
      let mut deleted = self.deleted.lock();
      deleted.retain(|_, until| *until > tokio::time::Instant::now());
      deleted.contains_key(id)
    };
    self.items.lock().iter().any(|s| s.id == id) || self.shown.lock().contains_key(id) || undo
  }
}

/// List actions the host's engine runs on its own sessions. Categories and moves are this machine's list and folders,
/// so they are not among them
fn list_target(m: &WebviewMsg) -> Option<String> {
  use WebviewMsg as W;
  match m {
    W::RenameSession { id, .. }
    | W::DeleteSession { id, .. }
    | W::RestoreSession { id }
    | W::PinSession { id, .. }
    | W::ArchiveSession { id, .. }
    | W::SetSessionUnread { id, .. } => Some(id.clone()),
    _ => None,
  }
}

/// A host folder as this engine names it; the path as it is when it is not absolute
fn local_folder(alias: &str, path: &str) -> String {
  format_remote_folder(alias, path).unwrap_or_else(|| path.to_owned())
}

fn host_folder_selected(alias: &str, cwd: &str, folders: Option<&[String]>) -> bool {
  parse_remote_folder(cwd).is_some_and(|f| f.alias == alias && folders.is_none_or(|paths| paths.contains(&f.path)))
}

/// A whole view as parts: its head without turns, every turn, and the live agent turn split into its head and blocks
fn split_view(view: &Value) -> Option<(Value, Vec<Box<RawValue>>, Option<LastTurnParts>)> {
  let mut head = view.clone();
  let turns = std::mem::replace(head.get_mut("turns")?, json!([]));
  let turns = turns.as_array()?;
  let mut encoded: Vec<Box<RawValue>> = turns.iter().map(|t| serde_json::value::to_raw_value(t).ok()).collect::<Option<_>>()?;
  let last = turns.last().and_then(split_last);
  if let (Some(l), Some(slot)) = (&last, encoded.last_mut()) {
    *slot = crate::bus::splice(&l.head, "blocks", &l.blocks)?;
  }
  Some((head, encoded, last))
}

/// The live agent turn's head (its `blocks` emptied) and blocks; None for any other turn
fn split_last(turn: &Value) -> Option<LastTurnParts> {
  if turn.get("role").and_then(Value::as_str) != Some("agent") {
    return None;
  }
  let mut head = turn.clone();
  let blocks = std::mem::replace(head.get_mut("blocks")?, json!([]));
  let blocks = blocks.as_array()?.iter().map(|b| serde_json::value::to_raw_value(b).ok()).collect::<Option<Vec<_>>>()?;
  Some(LastTurnParts { head: serde_json::value::to_raw_value(&head).ok()?, blocks })
}

/// The host's `sessionPatch` on top of what this engine holds of that view: the new turns and live turn, or None when
/// it does not fit (another base), which asks the host for the whole view
fn apply_patch(prev: &ViewParts, patch: &Value) -> Option<(Vec<Box<RawValue>>, Option<LastTurnParts>)> {
  let keep = patch.get("keep")?.as_u64()? as usize;
  let held = &prev.0.turns;
  if keep > held.len() {
    return None;
  }
  let raw = |v: &Value| serde_json::value::to_raw_value(v).ok();
  let sent: Vec<Value> = patch.get("turns")?.as_array()?.clone();
  let mut turns: Vec<Box<RawValue>> = held[..keep].to_vec();
  if let Some(kb) = patch.get("keepBlocks").and_then(Value::as_u64) {
    // The same live turn: its new head, the blocks it kept and the ones that changed
    let pl = prev.0.last.as_ref()?;
    let head = raw(sent.first()?)?;
    let mut blocks = pl.blocks.get(..kb as usize)?.to_vec();
    for b in patch.get("blocks")?.as_array()? {
      blocks.push(raw(b)?);
    }
    turns.push(crate::bus::splice(&head, "blocks", &blocks)?);
    return Some((turns, Some(LastTurnParts { head, blocks })));
  }
  for t in &sent {
    turns.push(raw(t)?);
  }
  // The live turn's parts come from the last turn as it now reads; one kept from before has no parts here
  let last = match sent.last() {
    Some(t) => split_last(t),
    None if keep == held.len() => prev.0.last.as_ref().map(|l| LastTurnParts { head: l.head.clone(), blocks: l.blocks.clone() }),
    None => None,
  };
  Some((turns, last))
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

  /// None means the session belongs here; otherwise the host's actual edit result is returned to the page.
  pub(super) async fn edit_on_host(&self, edit: &EditTurnRequest) -> Option<Result<()>> {
    let run = self.host_of(&edit.session_id)?;
    Some(run.edit(edit.clone()).await)
  }

  fn host_of(&self, id: &str) -> Option<Arc<HostRun>> {
    if self.locally_owned(id) {
      return None;
    }
    self.hosts.runs.lock().values().find(|r| r.owns(id)).cloned()
  }

  /// Search only on demand; the owning host supplies summaries outside the project subscription.
  pub(super) async fn search_hosts(&self, query: &str) -> Vec<SessionHit> {
    let runs: Vec<_> = self.hosts.runs.lock().values().cloned().collect();
    let mut tasks = tokio::task::JoinSet::new();
    for run in runs {
      let query = query.to_owned();
      tasks.spawn(async move { run.search(&query).await });
    }
    let mut hits = vec![];
    while let Some(Ok(found)) = tasks.join_next().await {
      hits.extend(found.into_iter().filter(|h| !self.locally_owned(&h.id)));
    }
    hits
  }

  async fn find_host(&self, id: &str) -> Option<Arc<HostRun>> {
    if !crate::store::transcript_store::is_session_id(id) || self.locally_owned(id) {
      return None;
    }
    // A record here that the index has not caught up with yet (another window's) is this machine's too
    if tokio::fs::try_exists(self.deps.store.dir().join(format!("{id}.json"))).await.unwrap_or(false) {
      return None;
    }
    let run = match self.host_of(id) {
      Some(run) => run,
      None => {
        let runs: Vec<_> = self.hosts.runs.lock().values().cloned().collect();
        let mut tasks = tokio::task::JoinSet::new();
        for run in runs {
          let id = id.to_owned();
          tasks.spawn(async move { run.lookup(&id).await.then_some(run) });
        }
        let mut found = None;
        while let Some(reply) = tasks.join_next().await {
          if let Ok(Some(run)) = reply {
            found = Some(run);
            break;
          }
        }
        let run = found?;
        // A local record may have arrived while the host was queried.
        if self.locally_owned(id) {
          return None;
        }
        run
      }
    };
    Some(run)
  }

  /// Whether a session is a host's own (not this machine's, though the host may hold a copy of it)
  pub(super) fn is_host_session(&self, id: &str) -> bool {
    self.host_of(id).is_some()
  }

  /// The host sessions for the list; ids this engine has itself win
  pub(super) fn host_summaries(&self, local: &[SessionSummary]) -> Vec<SessionSummary> {
    let runs: Vec<Arc<HostRun>> = self.hosts.runs.lock().values().cloned().collect();
    let mut out = vec![];
    for r in runs {
      out.extend(r.items.lock().iter().filter(|s| !local.iter().any(|l| l.id == s.id)).cloned());
    }
    out
  }

  /// A message to every connected host's list view (a list action that is not about one session, such as "mark all as read")
  pub(super) fn hosts_send_list(&self, message: Value) {
    let runs: Vec<Arc<HostRun>> = self.hosts.runs.lock().values().cloned().collect();
    for run in runs {
      run.send_to_view(LIST_VIEW, message.clone());
    }
  }

  /// A shown host session's current push
  pub(super) fn host_msg(&self, id: &str) -> Option<HostMsg> {
    let run = self.host_of(id)?;
    run.shown.lock().get(id).and_then(|s| s.msg.clone())
  }

  pub(super) fn host_cwd(&self, id: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Head { cwd: String }
    let run = self.host_of(id)?;
    let shown = run.shown.lock();
    let head = &shown.get(id)?.parts.as_ref()?.0.head;
    serde_json::from_str::<Head>(head.get()).ok().map(|h| h.cwd)
  }

  /// A page's action that belongs to a host session (or starts one in a host folder): sent there. None for the rest
  /// What a new session of `agent` in `cwd` offers when it would start in a host's engine (the same choice as
  /// `host_action`): that engine's answer to `controls`, read from the host's own CLI config, or its last one while the
  /// host is away. None when the session would start here, so the caller answers from this machine
  pub async fn host_controls(&self, agent: &str, cwd: &str, fresh: bool) -> Option<HostControls> {
    let folder = parse_remote_folder(cwd)?;
    if (self.deps.ssh_local_agents)().iter().any(|a| a == agent) {
      return None;
    }
    let run = self.hosts.runs.lock().get(&folder.alias).cloned()?;
    let reply = if *run.up.borrow() {
      // Keyed by agent: a second ask for the same agent replaces the first, which falls back to the last answer
      let message = json!({ "type": "controls", "agent": agent, "fresh": fresh.then_some(true) });
      run
        .request(format!("controls-{agent}"), || {
          run.send_to_view(LIST_VIEW, message);
        })
        .await
    } else {
      None
    };
    let msg = match reply {
      Some(msg) => {
        run.controls.lock().insert(agent.to_owned(), msg.clone());
        msg
      }
      None => run.controls.lock().get(agent).cloned().unwrap_or(Value::Null),
    };
    let field = |k: &str| msg.get(k).cloned().unwrap_or(Value::Null);
    Some(HostControls {
      controls: serde_json::from_value(field("controls")).unwrap_or_default(),
      shapes: serde_json::from_value(field("shapes")).unwrap_or_default(),
      commands: serde_json::from_value(field("commands")).unwrap_or_default(),
    })
  }

  pub(super) async fn host_action(self: &Arc<Self>, v: &Arc<Viewer>, m: &WebviewMsg) -> Option<Result<()>> {
    use WebviewMsg as W;
    if self.hosts.runs.lock().is_empty() {
      return None;
    }
    // A new session in a host folder starts on the host, unless its agent is one the user runs here
    // (`sshLocalAgents`, `acp/session/remote.rs`)
    let asked_on = match m {
      // The folder resolves as `new_session_for` resolves it: a page may name only a category, or nothing (the
      // current session's folder), and either may be a host's
      W::NewSession { cwd, agent, category, chat } if *chat != Some(true) => self.new_session_folder(v, cwd.clone(), category.as_ref()).as_deref().and_then(parse_remote_folder).and_then(|f| {
        let agent = agent.clone().unwrap_or_else(|| self.default_agent());
        if (self.deps.ssh_local_agents)().contains(&agent) {
          return None;
        }
        let run = self.hosts.runs.lock().get(&f.alias).cloned();
        run.map(|r| (f, r))
      }),
      _ => None,
    };
    if let (W::NewSession { agent, .. }, Some((f, run))) = (m, asked_on) {
      self.register_project(Some(v), &local_folder(&f.alias, &f.path)).await;
      // A host still being reached (the first connection deploys the helper) is waited for
      let mut up = run.up.subscribe();
      let _ = tokio::time::timeout(NEW_WAIT, up.wait_for(|u| *u)).await;
      // Keep the host's current viewer so its normal empty-session replacement runs. A shared view stays on the old
      // session for the other pages; this page then needs a separate view on the host.
      let current = v.active_id().filter(|id| self.viewers_on(id, Some(v)).is_empty());
      let reuse = current.and_then(|id| run.shown.lock().get(&id).map(|s| s.view_id.clone()));
      if let Some(view_id) = reuse {
        // Only this page follows the view to its new session; a page that opens the old one meanwhile stays there
        run.asked.lock().insert(view_id.clone(), Arc::downgrade(v));
        let sent = run.send_to_view(&view_id, json!({ "type": "newSession", "agent": agent, "cwd": f.path }));
        return Some(if sent { Ok(()) } else { Err(anyhow!(crate::i18n::tp("host.remoteUnreachable", &[("host", &f.alias)]))) });
      }
      let view_id = run.next_view_id();
      run.asked.lock().insert(view_id.clone(), Arc::downgrade(v));
      run.open_view(&view_id, json!({ "sessionList": { "folders": [] } }));
      // The host's categories are its own: a new session there starts unfiled
      let sent = run.send_to_view(&view_id, json!({ "type": "newSession", "agent": agent, "cwd": f.path }));
      // An empty session this page left here (an agent run here, then one run on the host) goes, as `new_session_for`
      // drops it when the next one starts here
      if sent {
        self.drop_empty_current(v).await;
      }
      return Some(if sent { Ok(()) } else { Err(anyhow!(crate::i18n::tp("host.remoteUnreachable", &[("host", &f.alias)]))) });
    }
    let explicit = session_id_of(m).or_else(|| list_target(m));
    let target = explicit.clone().or_else(|| v.active_id().filter(|_| is_forwarded(m)))?;
    let run = match self.host_of(&target) {
      Some(run) => run,
      None if list_target(m).is_some() && !v.has_list_scope() => self.find_host(&target).await?,
      None => return None,
    };
    let msg = serde_json::to_value(m).ok()?;
    let delete_request = match m {
      W::DeleteSession { request_id, .. } => request_id.as_ref(),
      _ => None,
    };
    if let Some(request_id) = delete_request {
      run.deletions.lock().insert(request_id.clone(), (target.clone(), Arc::downgrade(v)));
    }
    // Deletion must outlive its shown view: that view can be detached while the host closes and saves the session.
    let view_id = if matches!(m, W::DeleteSession { .. }) {
      LIST_VIEW.to_owned()
    } else {
      run.shown.lock().get(&target).map(|s| s.view_id.clone()).unwrap_or_else(|| LIST_VIEW.to_owned())
    };
    Some(if run.send_to_view(&view_id, msg) {
      Ok(())
    } else {
      if let Some(request_id) = delete_request {
        run.deletions.lock().remove(request_id);
      }
      Err(anyhow!(crate::i18n::tp("host.remoteUnreachable", &[("host", &run.alias)])))
    })
  }

  /// A viewer lands on a session: a host's own is shown through the host's engine. False when it is not a host's
  pub(super) async fn open_on_host(self: &Arc<Self>, v: &Arc<Viewer>, id: &str) -> bool {
    if self.locally_owned(id) || v.has_list_scope() {
      return false;
    }
    let Some(run) = self.find_host(id).await else { return false };
    if v.active_id().as_deref() != Some(id) {
      return true;
    }
    self.set_active(v, Some(id.to_owned()));
    self.show_host_session(&run, id, v);
    self.emit_sessions();
    true
  }

  /// A viewer lands on a host session: its view there is opened once, and a view already pushed goes out at once
  fn show_host_session(&self, run: &Arc<HostRun>, id: &str, v: &Viewer) {
    let current = {
      let mut shown = run.shown.lock();
      match shown.get(id) {
        Some(s) => {
          let (view_id, current) = (s.view_id.clone(), s.msg.clone());
          drop(shown);
          // A shared view can carry an explicitly kept unread mark. Landing on its cached transcript must still
          // select it on the owning host, where the mark lives.
          run.send_to_view(&view_id, json!({ "type": "selectSession", "id": id }));
          current
        }
        None => {
          let view_id = run.next_view_id();
          shown.insert(id.to_owned(), Shown::new(view_id.clone()));
          drop(shown);
          run.open_view(&view_id, json!({ "sessionList": { "folders": [] }, "session": id }));
          None
        }
      }
    };
    if let Some(msg) = current {
      v.emit(msg);
    }
  }

  /// Host views no viewer shows any more are closed there
  pub(super) fn prune_host_views(&self) {
    let runs: Vec<Arc<HostRun>> = self.hosts.runs.lock().values().cloned().collect();
    if runs.is_empty() {
      return;
    }
    let active: Vec<String> = self.viewers().iter().filter_map(|v| v.active_id()).collect();
    for run in runs {
      let gone: Vec<String> = {
        let mut shown = run.shown.lock();
        let ids: Vec<String> = shown.keys().filter(|id| !active.contains(id)).cloned().collect();
        ids.into_iter().filter_map(|id| shown.remove(&id).map(|s| s.view_id)).collect()
      };
      for view_id in gone {
        run.send(json!({ "type": "detachView", "viewId": view_id }));
      }
    }
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

  /// One envelope from a host's engine
  fn on_host_envelope(self: &Arc<Self>, run: &Arc<HostRun>, env: Value) {
    match env.get("type").and_then(Value::as_str) {
      Some("hostMessage") => {
        let view_id = env.get("viewId").and_then(Value::as_str).unwrap_or("").to_owned();
        let msg = env.get("message").cloned().unwrap_or(Value::Null);
        let key = match msg.get("type").and_then(Value::as_str) {
          Some("init") if view_id.starts_with("lookup-") => Some(view_id.clone()),
          Some("sessionHits") => msg.get("seq").and_then(Value::as_u64).map(|seq| format!("search-{seq}")),
          Some("controls") if view_id == LIST_VIEW => msg.get("agent").and_then(Value::as_str).map(|a| format!("controls-{a}")),
          Some("editTurnResult") if view_id == LIST_VIEW => msg.get("requestId").and_then(Value::as_str).map(str::to_owned),
          _ => None,
        };
        if let Some(key) = key {
          if let Some(tx) = run.requests.lock().remove(&key) {
            let _ = tx.send(msg);
          }
          return;
        }
        self.on_host_auth(run, &msg);
        if msg.get("type").and_then(Value::as_str) == Some("deleteSessionResult") {
          let Some(request_id) = msg.get("requestId").and_then(Value::as_str) else { return };
          let Some(undo) = msg.get("undo").and_then(Value::as_bool) else { return };
          let error = msg.get("error").and_then(Value::as_str).map(str::to_owned);
          let waiting = run.deletions.lock().remove(request_id);
          if let Some((id, viewer)) = waiting {
            if undo && error.is_none() {
              run.deleted.lock().insert(id, tokio::time::Instant::now() + super::TRASH_TTL);
            }
            if let Some(v) = viewer.upgrade() {
              v.emit(HostMsg::DeleteSessionResult { request_id: request_id.to_owned(), undo, error });
            }
          }
        } else if view_id == LIST_VIEW {
          self.on_host_list(run, &msg);
        } else {
          self.on_host_view(run, &view_id, &msg);
        }
      }
      Some("platformRequest") => {
        let request = env.get("request").cloned().unwrap_or(Value::Null);
        if let Some("toast") = request.get("method").and_then(Value::as_str) {
          let text = request.get("text").and_then(Value::as_str).unwrap_or("");
          let level = request.get("level").and_then(Value::as_str).unwrap_or("info");
          (self.deps.toast)(level, &format!("{}: {text}", run.alias));
        }
        if let Some(id) = env.get("requestId").and_then(Value::as_str) {
          run.send(json!({ "type": "platformResponse", "requestId": id, "error": "not available to a peer" }));
        }
      }
      _ => {}
    }
  }

  /// Account pushes belong to the host, not the local settings. Reissue each shown view's head when they change so
  /// login progress and keychain locks update even while the transcript stands still.
  fn on_host_auth(self: &Arc<Self>, run: &Arc<HostRun>, msg: &Value) {
    let changed = {
      let mut auth = run.auth.lock();
      let before = auth.clone();
      match msg.get("type").and_then(Value::as_str) {
        Some("init") => {
          if let Some(state) = msg.get("state") {
            for (key, source) in [("agents", "agents"), ("accounts", "accounts"), ("actions", "accountActions")] {
              if let Some(v) = state.get(source) {
                auth[key] = v.clone();
              }
            }
          }
        }
        Some("agents") => auth["agents"] = msg.get("agents").cloned().unwrap_or_else(|| json!([])),
        Some("accounts") => auth["accounts"] = msg.get("accounts").cloned().unwrap_or_else(|| json!([])),
        Some("accountActions") => auth["actions"] = msg.get("actions").cloned().unwrap_or_else(|| json!([])),
        _ => return,
      }
      *auth != before
    };
    if !changed {
      return;
    }
    let messages: Vec<(String, HostMsg)> = {
      let mut shown = run.shown.lock();
      shown.iter_mut().filter_map(|(id, s)| {
        let prev = s.parts.as_ref()?;
        let mut head: Value = serde_json::from_str(prev.0.head.get()).ok()?;
        head["remoteAuth"] = remote_auth(&run.auth.lock(), head.get("agent")?.as_str()?);
        let head = serde_json::value::to_raw_value(&head).ok()?;
        let last = prev.0.last.as_ref().map(|l| LastTurnParts { head: l.head.clone(), blocks: l.blocks.clone() });
        let turns = prev.0.turns.clone();
        s.publish(id, &head, self.next_mirror_rev(), turns, last).map(|msg| (id.clone(), msg))
      }).collect()
    };
    for (id, msg) in messages {
      for v in self.viewers_on(&id, None) {
        v.emit(msg.clone());
      }
    }
  }

  fn on_host_list(self: &Arc<Self>, run: &Arc<HostRun>, msg: &Value) {
    let list = match msg.get("type").and_then(Value::as_str) {
      Some("init") => msg.pointer("/state/sessions"),
      Some("sessions") => msg.get("sessions"),
      _ => None,
    };
    let Some(list) = list.and_then(|l| serde_json::from_value::<Vec<SessionSummary>>(l.clone()).ok()) else { return };
    // Sessions the host's engine lists from hosts of its own are theirs, not this host's
    let scope = run.folders.lock();
    let items: Vec<SessionSummary> = list
      .into_iter()
      .filter(|s| !acpira_shared::remote::is_remote_folder(&s.cwd))
      .map(|mut s| {
        s.cwd = local_folder(&run.alias, &s.cwd);
        s.category = None;
        s
      })
      .filter(|s| host_folder_selected(&run.alias, &s.cwd, scope.as_deref()))
      .collect();
    if *run.items.lock() == items {
      return;
    }
    run.deleted.lock().retain(|id, until| *until > tokio::time::Instant::now() && !items.iter().any(|s| &s.id == id));
    self.save_host_list(&run.alias, &items);
    *run.items.lock() = items;
    drop(scope);
    self.emit_sessions();
  }

  /// A subagent transcript the host pushed through a session's view (a page here observes it: `observeSubagent` went to that
  /// view like the session's other actions). The host's rev is the session's own counter, so it passes through
  fn on_host_subagent(&self, run: &Arc<HostRun>, view_id: &str, msg: &Value) {
    let Some(id) = run.shown.lock().iter().find(|(_, s)| s.view_id == view_id).map(|(id, _)| id.clone()) else { return };
    let (Some(sub), Some(rev), Some(turns)) = (msg.get("subagentId").and_then(Value::as_str), msg.get("rev").and_then(Value::as_i64), msg.get("turns")) else {
      return;
    };
    let Ok(turns) = serde_json::value::to_raw_value(turns) else { return };
    let running = msg.get("running").and_then(Value::as_bool).unwrap_or(false);
    let m = HostMsg::Subagent { session_id: id.clone(), subagent_id: sub.to_owned(), rev, running, turns: RawJson(Arc::from(turns)) };
    for v in self.viewers_on(&id, None) {
      v.emit(m.clone());
    }
  }

  /// Bring the blobs a host session's view shows into this machine's store, where every page loads them from (the
  /// webview's blob URLs, a paired phone's blob requests). True while the session's view is held for a fetch, its own
  /// or one already running; the latest view goes out once the last one ends
  /// Bring blobs a host session's view names into this machine's store, in the background: the view went out without
  /// them (`Shown::publish`), and it goes out again as they land (`blob_landed`)
  fn fetch_host_blobs(self: &Arc<Self>, run: &Arc<HostRun>, id: &str, names: Vec<String>) {
    if names.is_empty() {
      return;
    }
    let Some(client) = self.deps.remote.as_ref().and_then(|c| c.client(&run.alias)) else { return };
    for name in names {
      let key = format!("{id}/{name}");
      if !run.blobs.lock().insert(key.clone()) {
        continue;
      }
      let (weak, run, id, client, store) = (Arc::downgrade(self), run.clone(), id.to_owned(), client.clone(), self.deps.store.clone());
      tokio::spawn(async move {
        let fetched = match run.slots.clone().acquire_owned().await {
          Ok(_slot) => fetch_host_blob(&client, &store, &id, &name).await,
          Err(_) => return,
        };
        run.blobs.lock().remove(&key);
        let Some(m) = weak.upgrade() else { return };
        match fetched {
          Ok(true) => m.blob_landed(&run, &id, &name),
          Ok(false) => m.log(&format!("remote run: {id}/{name} on {} is over {} MB, left there", run.alias, AUTO_BLOB_MAX >> 20)),
          Err(e) => m.log(&format!("remote run: {id}/{name} from {}: {e:#}", run.alias)),
        }
      });
    }
  }

  /// A blob of a shown host session is on this machine now: its view goes out again with it, once for the blobs that
  /// land within `BLOB_SETTLE` (each rebuild touches the whole view)
  fn blob_landed(self: &Arc<Self>, run: &Arc<HostRun>, id: &str, name: &str) {
    {
      let mut shown = run.shown.lock();
      let Some(s) = shown.get_mut(id) else { return };
      if !s.absent.remove(name) {
        return;
      }
    }
    if !run.settling.lock().insert(id.to_owned()) {
      return;
    }
    let (weak, run, id) = (Arc::downgrade(self), run.clone(), id.to_owned());
    tokio::spawn(async move {
      tokio::time::sleep(BLOB_SETTLE).await;
      run.settling.lock().remove(&id);
      let Some(m) = weak.upgrade() else { return };
      let msg = {
        let mut shown = run.shown.lock();
        let Some(s) = shown.get_mut(&id) else { return };
        let Some(parts) = s.parts.clone() else { return };
        let last = parts.0.last.as_ref().map(|l| LastTurnParts { head: l.head.clone(), blocks: l.blocks.clone() });
        s.publish(&id, &parts.0.head, m.next_mirror_rev(), parts.0.turns.clone(), last)
      };
      if let Some(msg) = msg {
        for v in m.viewers_on(&id, None) {
          v.emit(msg.clone());
        }
      }
    });
  }

  fn on_host_view(self: &Arc<Self>, run: &Arc<HostRun>, view_id: &str, msg: &Value) {
    if msg.get("type").and_then(Value::as_str) == Some("subagent") {
      self.on_host_subagent(run, view_id, msg);
      return;
    }
    let (whole, patch) = match msg.get("type").and_then(Value::as_str) {
      Some("init") => (msg.pointer("/state/active").filter(|a| !a.is_null()), None),
      Some("session") => (msg.get("session"), None),
      Some("sessionPatch") => (None, msg.get("patch")),
      _ => return,
    };
    // Whether this view showed this session already: a reused view's push of the session it is leaving is no answer
    let again = whole
      .and_then(|w| w.get("id"))
      .and_then(Value::as_str)
      .is_some_and(|id| run.shown.lock().get(id).is_some_and(|s| s.view_id == view_id));
    // A reused host viewer has moved to its new session. Move the local pages with it and retire the old patch base.
    if let Some(id) = whole.and_then(|w| w.get("id")).and_then(Value::as_str) {
      let previous = {
        let mut shown = run.shown.lock();
        let previous = shown.iter().find(|(old, s)| old.as_str() != id && s.view_id == view_id).map(|(old, _)| old.clone());
        if let Some(old) = &previous {
          shown.remove(old);
          shown.insert(id.to_owned(), Shown::new(view_id.to_owned()));
        }
        previous
      };
      if let Some(old) = previous {
        let asker = run.asked.lock().get(view_id).and_then(Weak::upgrade);
        for v in self.viewers_on(&old, None) {
          match &asker {
            // Another page on the old session keeps it, through a view of its own on the host
            Some(a) if !Arc::ptr_eq(a, &v) => self.show_host_session(run, &old, &v),
            _ => self.set_active(&v, Some(id.to_owned())),
          }
        }
      }
    }
    // A view that asked for a new session: the first one it shows is that session, and its viewer goes there
    if let Some(id) = whole.and_then(|w| w.get("id")).and_then(Value::as_str)
      && !again
      && let Some(viewer) = run.asked.lock().remove(view_id)
    {
      run.shown.lock().insert(id.to_owned(), Shown::new(view_id.to_owned()));
      if let Some(v) = viewer.upgrade() {
        self.set_active(&v, Some(id.to_owned()));
      }
    }
    let Some(id) = run.shown.lock().iter().find(|(_, s)| s.view_id == view_id).map(|(id, _)| id.clone()) else { return };
    // The blobs this step shows (a whole view's, or a patch's changed tail): they live in the host's store
    let mut refs = vec![];
    if let Some(step) = whole.or(patch) {
      blob_refs(step, &id, &mut refs);
    }
    // Those not on this machine: the view goes out without them now, and each comes back with its file
    refs.sort_unstable();
    refs.dedup();
    let store = &self.deps.store;
    refs.retain(|name| store.blob_path(&id, name).is_some_and(|p| !p.exists()));
    let built = {
      let mut shown = run.shown.lock();
      let Some(s) = shown.get_mut(&id) else { return };
      s.absent.extend(refs.iter().cloned());
      let step = match (whole, patch) {
        (Some(w), _) => split_view(w),
        (None, Some(p)) if p.get("id").and_then(Value::as_str) == Some(id.as_str()) => {
          let fits = s.host_rev.is_some() && p.get("base").and_then(Value::as_i64) == s.host_rev;
          match (fits, s.parts.as_ref()) {
            (true, Some(prev)) => apply_patch(prev, p).and_then(|(turns, last)| Some((p.get("view")?.clone(), turns, last))),
            _ => None,
          }
        }
        _ => return,
      };
      match step {
        None => None,
        Some((mut head, turns, last)) => {
          s.host_rev = head.get("rev").and_then(Value::as_i64);
          if let Some(cwd) = head.get("cwd").and_then(Value::as_str) {
            head["cwd"] = json!(local_folder(&run.alias, cwd));
          }
          if let Some(agent) = head.get("agent").and_then(Value::as_str) {
            head["remoteAuth"] = remote_auth(&run.auth.lock(), agent);
          }
          let rev = self.next_mirror_rev();
          let head = serde_json::value::to_raw_value(&head).ok();
          head.and_then(|h| s.publish(&id, &h, rev, turns, last))
        }
      }
    };
    match built {
      Some(msg) => {
        self.fetch_host_blobs(run, &id, refs);
        for v in self.viewers_on(&id, None) {
          v.emit(msg.clone());
        }
      }
      // Out of step with the host's view: it sends the whole view again
      None => {
        self.log(&format!("remote run: {id} on {} out of step, asking for the whole view", run.alias));
        run.send_to_view(view_id, json!({ "type": "resync" }));
      }
    }
  }
}

/// Select public metadata for the session's agent. No local account may appear in a host session's menu.
fn remote_auth(auth: &Value, agent: &str) -> Value {
  let info = auth.get("agents").and_then(Value::as_array).and_then(|a| a.iter().find(|a| a.get("id").and_then(Value::as_str) == Some(agent)))
    .cloned().unwrap_or_else(|| json!({ "id": agent, "name": agent }));
  let accounts: Vec<Value> = auth.get("accounts").and_then(Value::as_array).into_iter().flatten()
    .filter(|a| a.get("agent").and_then(Value::as_str) == Some(agent)).cloned().collect();
  let action = auth.get("actions").and_then(Value::as_array).into_iter().flatten()
    .find(|a| a.get("agent").and_then(Value::as_str) == Some(agent));
  let mut out = json!({ "agent": info, "accounts": accounts });
  // The start picker needs this host's CLI availability, not the viewing machine's installation state.
  if let Some(agents) = auth.get("agents").filter(|a| a.is_array()) {
    out["agents"] = agents.clone();
  }
  if let Some(action) = action {
    out["action"] = action.clone();
  }
  out
}

/// Every `blob` a view (or a patch of one) names, whichever block holds it: images, attachments, embedded files
fn blob_refs(v: &Value, session_id: &str, out: &mut Vec<String>) {
  match v {
    Value::Object(m) => {
      // Staged file attachments keep a file URI instead of a `blob` field. Only a file in this session's blob folder
      // is copied; workspace file links stay on the host.
      if m.get("kind").and_then(Value::as_str) == Some("file")
        && let Some(uri) = m.get("uri").and_then(Value::as_str)
        && let Some(path) = crate::platform::file_url::file_url_to_path(uri).map(std::path::PathBuf::from)
        && let Some(parent) = path.parent()
        && parent.file_name().is_some_and(|n| n == session_id)
        && parent.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "sessions")
        && let Some(name) = path.file_name().and_then(|n| n.to_str())
      {
        out.push(name.to_owned());
      }
      for (k, x) in m {
        match (k.as_str(), x) {
          ("blob", Value::String(name)) => out.push(name.clone()),
          _ => blob_refs(x, session_id, out),
        }
      }
    }
    Value::Array(a) => a.iter().for_each(|x| blob_refs(x, session_id, out)),
    _ => {}
  }
}

/// The view pages get of a host session: its parts without the `absent` blobs. Every block and attachment kind takes a
/// missing `blob` for a file that could not be read, so a page shows the rest and loads the file once the next view
/// names it
fn without_blobs(id: &str, parts: &ViewParts, absent: &HashSet<String>, rev: i64, epoch: u64) -> Option<HostMsg> {
  fn strip(v: &mut Value, absent: &HashSet<String>) {
    match v {
      Value::Object(m) => {
        if m.get("blob").and_then(Value::as_str).is_some_and(|b| absent.contains(b)) {
          m.remove("blob");
        }
        m.values_mut().for_each(|x| strip(x, absent));
      }
      Value::Array(a) => a.iter_mut().for_each(|x| strip(x, absent)),
      _ => {}
    }
  }
  // Only the pieces naming one are parsed again; the rest are shared as they are
  let mask = |raw: &RawValue| -> Option<Box<RawValue>> {
    if !absent.iter().any(|n| raw.get().contains(n.as_str())) {
      return Some(raw.to_owned());
    }
    let mut v: Value = serde_json::from_str(raw.get()).ok()?;
    strip(&mut v, absent);
    serde_json::value::to_raw_value(&v).ok()
  };
  let turns = parts.0.turns.iter().map(|t| mask(t)).collect::<Option<Vec<_>>>()?;
  let last = match &parts.0.last {
    Some(l) => Some(LastTurnParts { head: l.head.clone(), blocks: l.blocks.iter().map(|b| mask(b)).collect::<Option<Vec<_>>>()? }),
    None => None,
  };
  remote_view(id, &parts.0.head, rev, epoch, turns, last).map(|(_, msg)| msg)
}

/// One blob of a host session from the host's store into this machine's, written as it comes (`<sessions>/.partial/`)
/// and resumed from there after a cut. Blobs are named by their content, so a file that does not match the name is
/// refused. False when the host left it out as too large (`AUTO_BLOB_MAX`)
async fn fetch_host_blob(
  client: &crate::remote::client::RemoteClient,
  store: &crate::store::transcript_store::TranscriptStore,
  id: &str,
  name: &str,
) -> Result<bool> {
  use tokio::io::AsyncWriteExt;
  let ext = name.rsplit_once('.').map(|(_, e)| format!(".{e}")).unwrap_or_default();
  let dir = store.partial_dir();
  tokio::fs::create_dir_all(&dir).await?;
  let part = dir.join(format!("{id}-{name}"));
  let have = match tokio::fs::metadata(&part).await {
    Ok(m) if m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < PARTIAL_KEEP) => m.len(),
    _ => {
      let _ = tokio::fs::remove_file(&part).await;
      0
    }
  };
  let mut call = client.request("storeBlobGet", json!({ "id": id, "name": name, "offset": have, "max": AUTO_BLOB_MAX }));
  let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(&part).await?;
  let reply = loop {
    match call.next().await {
      Some(Event::Chunk { stream, bytes }) if stream == "data" => file.write_all(&bytes).await?,
      Some(Event::Chunk { .. }) => {}
      Some(Event::Done(r)) => break r.map_err(|e| anyhow!("{e}"))?,
      None => bail!("the connection ended"),
    }
  };
  file.flush().await?;
  drop(file);
  if reply.get("skipped").and_then(Value::as_bool) == Some(true) {
    if have == 0 {
      let _ = tokio::fs::remove_file(&part).await;
    }
    return Ok(false);
  }
  // A helper that does not resume (an older release a local build deployed) sent the whole blob after the kept piece
  if have > 0 && reply.get("offset").and_then(Value::as_u64) != Some(have) {
    let _ = tokio::fs::remove_file(&part).await;
    bail!("the host sent the blob from its start again; fetching it whole next time");
  }
  let (check, ext_of) = (part.clone(), ext.clone());
  let got = tokio::task::spawn_blocking(move || crate::store::transcript_store::blob_name_of_file(&ext_of, &check)).await??;
  if got != name {
    let _ = tokio::fs::remove_file(&part).await;
    bail!("the host's bytes do not match the name");
  }
  store.adopt_blob(id, name, part).await?;
  Ok(true)
}

/// One host's connection, for as long as the host stays in remote run: reach its engine, open the list view and the
/// views of the sessions shown here, carry envelopes until the connection ends, then try again with backoff
async fn drive(weak: Weak<SessionManager>, run: Arc<HostRun>) {
  let mut retry = RETRY_FIRST;
  loop {
    let Some(m) = weak.upgrade() else { return };
    let Some(connections) = m.deps.remote.clone() else { return };
    drop(m);
    let attempt: Result<PeerLink> = async {
      let client = connections.follow(&run.alias, CONNECT_WAIT).await?;
      PeerLink::connect(&client, None, PeerHello { version: acpira_shared::version::VERSION, capabilities: PEER_CAPS, host_language: "en" }).await
    }
    .await;
    match attempt {
      Err(e) => {
        if let Some(m) = weak.upgrade() {
          m.log(&format!("remote run: {}'s engine: {e:#}", run.alias));
        }
      }
      Ok(mut link) => {
        retry = RETRY_FIRST;
        *run.pid.lock() = link.hello_ok.pointer("/sidecar/pid").and_then(Value::as_u64).map(|p| p as u32);
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
}
