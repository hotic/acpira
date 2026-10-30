//! Host sessions on this engine's side (docs/dev/remote.md): every SSH host that is not turned off has its engine reached
//! as a peer (`remote::peer`), whatever its run mode, since a host folder has one history, the host's own. One empty view
//! there brings its session list, which joins the local list with each folder as its `ssh://` form; a host session shown
//! here gets a view of its own there, whose pushes are rebuilt into this engine's views (its own revs, so pages take
//! patches as for any session). Page actions on a host session go to that view, list actions on one to the list view. The
//! run mode decides only where a new session starts: on the host under remote run, here under local run. The last list a
//! host sent stays (also on disk) while it is away

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use acpira_shared::protocol::{HostMsg, RawJson, WebviewMsg};
use acpira_shared::remote::{ConnectionView, RunMode, format_remote_folder, parse_remote_folder};
use acpira_shared::session_patch::{LastTurnParts, ViewParts};
use acpira_shared::transcript::SessionSummary;

use super::{SessionManager, Viewer, is_forwarded, remote_view, session_id_of};
use crate::remote::peer::{PeerHello, PeerLink};

/// How long one attempt waits for the host's helper channel
const CONNECT_WAIT: Duration = Duration::from_secs(60);
/// Between attempts, doubling from the first
const RETRY_FIRST: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(30);
const LIST_VIEW: &str = "list";
/// How long a new session in a host folder waits for the host's engine to be reached
const NEW_WAIT: Duration = Duration::from_secs(60);
/// What this engine serves for the host's engine (the requests its views cause come back here)
const PEER_CAPS: &[&str] = &["toast"];

#[derive(Default)]
pub(super) struct Hosts {
  runs: parking_lot::Mutex<HashMap<String, Arc<HostRun>>>,
  sub: parking_lot::Mutex<Option<u64>>,
}

struct HostRun {
  alias: String,
  /// The host's run mode now: under remote run new sessions in its folders start there
  mode: parking_lot::Mutex<RunMode>,
  /// Envelopes to the host's engine while it is connected
  out: parking_lot::Mutex<Option<mpsc::UnboundedSender<Value>>>,
  /// Whether it is connected now
  up: tokio::sync::watch::Sender<bool>,
  /// The host's list as this engine shows it (folders as `ssh://` forms, no categories of the host's)
  items: parking_lot::Mutex<Vec<SessionSummary>>,
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
}

impl HostRun {
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
  }

  pub(super) fn stop_hosts(&self) {
    if let (Some(id), Some(c)) = (self.hosts.sub.lock().take(), self.deps.remote.as_ref()) {
      c.unsubscribe(id);
    }
    for (_, run) in self.hosts.runs.lock().drain() {
      if let Some(t) = run.task.lock().take() {
        t.abort();
      }
    }
  }

  fn sync_hosts(self: &Arc<Self>, views: &[ConnectionView]) {
    let wanted: Vec<(&str, RunMode)> = views.iter().filter(|v| !v.disabled).map(|v| (v.alias.as_str(), v.mode)).collect();
    let mut changed = false;
    {
      let mut runs = self.hosts.runs.lock();
      runs.retain(|alias, run| {
        let keep = wanted.iter().any(|(a, _)| a == alias);
        if !keep {
          if let Some(t) = run.task.lock().take() {
            t.abort();
          }
          changed = true;
        }
        keep
      });
      for (alias, mode) in wanted {
        if let Some(run) = runs.get(alias) {
          *run.mode.lock() = mode;
          continue;
        }
        let run = Arc::new(HostRun {
          alias: alias.to_owned(),
          mode: parking_lot::Mutex::new(mode),
          out: Default::default(),
          up: tokio::sync::watch::Sender::new(false),
          items: parking_lot::Mutex::new(self.cached_host_list(alias)),
          shown: Default::default(),
          asked: Default::default(),
          deletions: Default::default(),
          deleted: Default::default(),
          pid: Default::default(),
          seq: Default::default(),
          task: Default::default(),
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

  /// The host engines this engine is connected to, with their pids
  pub fn host_engines(&self) -> Vec<(String, u32)> {
    self.hosts.runs.lock().values().filter_map(|r| r.pid.lock().map(|p| (r.alias.clone(), p))).collect()
  }

  /// The host whose session this is. A session this machine has itself (live, stored, or run by another engine here) is
  /// never a host's: the host's list also holds the copies of this machine's sessions in its folders (`remote::host_store`)
  fn host_of(&self, id: &str) -> Option<Arc<HostRun>> {
    let mine = {
      let st = self.state.lock();
      st.live.contains_key(id) || st.index.iter().any(|s| s.id == id) || st.elsewhere.values().flatten().any(|s| s.id == id)
    };
    if mine {
      return None;
    }
    self.hosts.runs.lock().values().find(|r| r.owns(id)).cloned()
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

  /// A page's action that belongs to a host session (or starts one in a host folder): sent there. None for the rest
  pub(super) async fn host_action(self: &Arc<Self>, v: &Arc<Viewer>, m: &WebviewMsg) -> Option<Result<()>> {
    use WebviewMsg as W;
    if self.hosts.runs.lock().is_empty() {
      return None;
    }
    // Under local run a new session in a host folder starts here (`acp/session/remote.rs`)
    let asked_on = match m {
      W::NewSession { cwd: Some(cwd), .. } => parse_remote_folder(cwd).and_then(|f| {
        let run = self.hosts.runs.lock().get(&f.alias).cloned().filter(|r| *r.mode.lock() == RunMode::Remote);
        run.map(|r| (f, r))
      }),
      _ => None,
    };
    if let (W::NewSession { agent, .. }, Some((f, run))) = (m, asked_on) {
      // A host still being reached (the first connection deploys the helper) is waited for
      let mut up = run.up.subscribe();
      let _ = tokio::time::timeout(NEW_WAIT, up.wait_for(|u| *u)).await;
      let view_id = run.next_view_id();
      run.asked.lock().insert(view_id.clone(), Arc::downgrade(v));
      run.open_view(&view_id, json!({ "empty": true }));
      // The host's categories are its own: a new session there starts unfiled
      let sent = run.send_to_view(&view_id, json!({ "type": "newSession", "agent": agent, "cwd": f.path }));
      return Some(if sent { Ok(()) } else { Err(anyhow!(crate::i18n::tp("host.remoteUnreachable", &[("host", &f.alias)]))) });
    }
    let explicit = session_id_of(m).or_else(|| list_target(m));
    let target = explicit.clone().or_else(|| v.active_id().filter(|_| is_forwarded(m)))?;
    let run = self.host_of(&target)?;
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
  pub(super) fn open_on_host(self: &Arc<Self>, v: &Arc<Viewer>, id: &str) -> bool {
    let Some(run) = self.host_of(id) else { return false };
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
        Some(s) => s.msg.clone(),
        None => {
          let view_id = run.next_view_id();
          shown.insert(id.to_owned(), Shown { view_id: view_id.clone(), epoch: crate::acp::session::next_view_epoch(), host_rev: None, parts: None, msg: None });
          drop(shown);
          run.open_view(&view_id, json!(id));
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

  fn cached_host_list(&self, alias: &str) -> Vec<SessionSummary> {
    std::fs::read_to_string(self.cache_file(alias)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
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
        if request.get("method").and_then(Value::as_str) == Some("toast") {
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

  fn on_host_list(self: &Arc<Self>, run: &Arc<HostRun>, msg: &Value) {
    let list = match msg.get("type").and_then(Value::as_str) {
      Some("init") => msg.pointer("/state/sessions"),
      Some("sessions") => msg.get("sessions"),
      _ => None,
    };
    let Some(list) = list.and_then(|l| serde_json::from_value::<Vec<SessionSummary>>(l.clone()).ok()) else { return };
    // Sessions the host's engine lists from hosts of its own are theirs, not this host's
    let items: Vec<SessionSummary> = list
      .into_iter()
      .filter(|s| !acpira_shared::remote::is_remote_folder(&s.cwd))
      .map(|mut s| {
        s.cwd = local_folder(&run.alias, &s.cwd);
        s.category = None;
        s
      })
      .collect();
    if *run.items.lock() == items {
      return;
    }
    run.deleted.lock().retain(|id, until| *until > tokio::time::Instant::now() && !items.iter().any(|s| &s.id == id));
    self.save_host_list(&run.alias, &items);
    *run.items.lock() = items;
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
    // A view that asked for a new session: the first one it shows is that session, and its viewer goes there
    if let Some(id) = whole.and_then(|w| w.get("id")).and_then(Value::as_str)
      && let Some(viewer) = run.asked.lock().remove(view_id)
    {
      run.shown.lock().insert(id.to_owned(), Shown { view_id: view_id.to_owned(), epoch: crate::acp::session::next_view_epoch(), host_rev: None, parts: None, msg: None });
      if let Some(v) = viewer.upgrade() {
        self.set_active(&v, Some(id.to_owned()));
      }
    }
    let Some(id) = run.shown.lock().iter().find(|(_, s)| s.view_id == view_id).map(|(id, _)| id.clone()) else { return };
    let built = {
      let mut shown = run.shown.lock();
      let Some(s) = shown.get_mut(&id) else { return };
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
          let rev = self.next_mirror_rev();
          let head = serde_json::value::to_raw_value(&head).ok();
          head.and_then(|h| remote_view(&id, &h, rev, s.epoch, turns, last)).map(|(parts, msg)| {
            s.parts = Some(parts);
            s.msg = Some(msg.clone());
            msg
          })
        }
      }
    };
    match built {
      Some(msg) => {
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
      PeerLink::connect(&client, None, PeerHello { version: env!("CARGO_PKG_VERSION"), capabilities: PEER_CAPS, host_language: "en" }).await
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
        run.open_view(LIST_VIEW, json!({ "empty": true }));
        run.up.send_replace(true);
        // Back after a drop: the sessions shown here get their views again, whole first
        let reopen: Vec<(String, String)> = {
          let mut shown = run.shown.lock();
          shown.values_mut().for_each(|s| s.host_rev = None);
          shown.iter().map(|(id, s)| (id.clone(), s.view_id.clone())).collect()
        };
        for (id, view_id) in reopen {
          run.open_view(&view_id, json!(id));
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
    tokio::time::sleep(retry).await;
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
    (ViewPartsData { id: "s".into(), epoch: 7, rev, head: raw(&json!({ "id": "s", "rev": rev, "cwd": "/w", "turns": [] })), turns: encoded, last }, whole)
  }

  fn turns_of(t: &[Box<RawValue>]) -> Vec<Value> {
    t.iter().map(|r| serde_json::from_str(r.get()).unwrap()).collect()
  }

  /// What this engine holds after taking a whole view
  fn held(whole: &Value) -> ViewParts {
    let (head, turns, last) = split_view(whole).unwrap();
    ViewParts(Arc::new(ViewPartsData { id: "s".into(), epoch: 1, rev: 1, head: serde_json::value::to_raw_value(&head).unwrap(), turns, last }))
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
