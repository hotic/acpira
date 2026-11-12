//! One session history per host (docs/dev/remote.md): a folder on a host has the sessions in the host's own store,
//! whichever run mode is used. A session this engine drives in a host folder (local run) keeps its record here, written
//! by its owner as any session's, and a copy goes into the host's store with the folder as the host names it, so the
//! host's engine lists it beside its own and remote run, the host's other clients and other machines see one history.
//! The copy names this machine (`copyOf`): the host follows it without a process until a local action takes ownership.
//!
//! `serve` is the helper's side (the host's store, through the same `TranscriptStore` code); `HostMirror` this engine's:
//! told of every record change by the store, it sends the copies of host folders' records in batches while the host is
//! connected, and on every connection compares what the host has, which also brings over records written before this
//! existed. A copy never replaces one the host updated later, nor one deleted there and still in its trash

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use acpira_shared::remote::{ConnectionState, ConnectionView, parse_remote_folder};
use acpira_shared::transcript::Turn;

use super::client::RemoteClient;
use super::connections::Connections;
use super::wire::{self, WireError};
use crate::store::diff_texts;
use crate::store::file_lock::{with_file_lock, write_atomic};
use crate::store::record::SessionRecord;
use crate::store::transcript_store::{ChangeHook, CopyStamp, LogFn, RecordChange, TranscriptStore, blob_name, is_session_id};

/// Changes are gathered this long before they go to the host: a streaming turn rewrites its record every few seconds
const BATCH: Duration = Duration::from_secs(5);

/// This machine as copies name it: its id (`helper::machine_id`) and its name, read once
pub fn this_machine() -> &'static (String, String) {
  static ME: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
  ME.get_or_init(|| {
    let name = crate::accounts::local::hostname();
    (super::helper::machine_id(), name.strip_suffix(".local").unwrap_or(&name).to_owned())
  })
}

/// The most a record or blob sent to the host may weigh, gzipped for a record
const MAX_BODY: usize = 512 * 1024 * 1024;

/// A request's body: its `stdin` frames up to their end (a line holds at most `wire::MAX_LINE`, a record can be more)
pub(super) async fn body(stdin: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Vec<u8>, WireError> {
  let Some(mut rx) = stdin else { return Err(WireError::new("invalid", "no body")) };
  let mut buf = vec![];
  while let Some(Some(chunk)) = rx.recv().await {
    buf.extend_from_slice(&chunk);
    if buf.len() > MAX_BODY {
      return Err(WireError::new("invalid", "the body is too large"));
    }
  }
  Ok(buf)
}

/// Send `bytes` as a request's body and wait for its answer
pub(super) async fn call_with_body(client: &RemoteClient, op: &str, a: Value, bytes: &[u8]) -> Result<Value> {
  let call = client.request(op, a);
  for chunk in bytes.chunks(wire::CHUNK) {
    call.stdin(chunk);
  }
  call.stdin_eof();
  Ok(call.collect().await.map_err(|e| anyhow!("{e}"))?.value)
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
  let mut raw = vec![];
  flate2::read::GzDecoder::new(bytes).take(MAX_BODY as u64 + 1).read_to_end(&mut raw)?;
  if raw.len() > MAX_BODY { return Err(anyhow!("the expanded body is too large")); }
  Ok(raw)
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
  let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::new(6));
  gz.write_all(bytes)?;
  Ok(gz.finish()?)
}

/// The helper's store operations on this host's sessions directory
pub(crate) async fn serve(op: &str, a: &Value, stdin: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  let root = a.get("home").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(crate::store::data_dir::acpira_home);
  let store = TranscriptStore::new(root.join("sessions"), Arc::new(|_: &str| {}), None);
  let id_arg = || {
    let id = a.get("id").and_then(Value::as_str).unwrap_or("");
    if is_session_id(id) { Ok(id.to_owned()) } else { Err(WireError::new("invalid", format!("\"{id}\" is not a session id"))) }
  };
  let failed = |e: anyhow::Error| WireError::new("io", format!("{e:#}"));
  match op {
    // What the host has of these sessions: when each was last updated and its blobs
    "storeStat" => {
      let ids: Vec<String> = a.get("ids").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
      let stamps = store.copy_stamps(&ids).await;
      Ok(json!({ "records": stamps }))
    }
    // A record copy, gzipped JSON as the body; answers whether it was written and the blobs the session has here
    "storePut" => {
      let gz = body(stdin).await?;
      let raw = gunzip(&gz).map_err(|e| WireError::new("invalid", e.to_string()))?;
      let record: SessionRecord = serde_json::from_slice(&raw).map_err(|e| WireError::new("invalid", format!("not a session record: {e}")))?;
      // The built-in agent's log the host has is of the copy being replaced: it goes with that copy, and the log of
      // this one follows (`storeAgentLog`). Until it does a promotion hands the transcript over, never the old context
      let log = (record.agent == "acpira").then_some(record.acp_session_id.as_deref()).flatten().filter(|sid| is_session_id(sid)).map(|sid| root.join("agent/sessions").join(format!("{sid}.jsonl")));
      let written = store.put_copy(&record, log.as_deref()).await.map_err(failed)?;
      let stamps = store.copy_stamps(std::slice::from_ref(&record.id)).await;
      let taken_over = stamps.get(&record.id).is_some_and(|s| s.copy_of.as_ref().map(|c| &c.machine) != record.copy_of.as_ref().map(|c| &c.machine));
      Ok(json!({ "written": written, "takenOver": taken_over, "blobs": store.blob_names(&record.id).await }))
    }
    "storeAgentLog" => {
      let id = id_arg()?;
      let session = a.get("sessionId").and_then(Value::as_str).filter(|s| is_session_id(s)).ok_or_else(|| WireError::new("invalid", "invalid sessionId"))?;
      let bytes = gunzip(&body(stdin).await?).map_err(|e| WireError::new("invalid", e.to_string()))?;
      let leases = crate::store::session_lease::SessionLeases::new(&root, Arc::new(|_: &str| {}));
      if !matches!(leases.claim(session), crate::store::session_lease::Claim::Held(_)) {
        return Ok(json!({ "written": false }));
      }
      let record = store.load(session).await.ok_or_else(|| WireError::new("notFound", "session record missing"))?;
      if record.copy_of.is_none() || record.agent != "acpira" || record.acp_session_id.as_deref() != Some(&id) {
        return Ok(json!({ "written": false, "takenOver": record.copy_of.is_none() }));
      }
      let dir = root.join("agent/sessions");
      tokio::fs::create_dir_all(&dir).await.map_err(|e| WireError::io(&e, "agent sessions"))?;
      write_atomic(&dir.join(format!("{id}.jsonl")), &bytes, None).await.map_err(failed)?;
      Ok(json!({ "written": true }))
    }
    // One blob of a session, named by its content as every blob is: a name that does not match the bytes is refused. A
    // diff text blob is named by the text it holds compressed (`store::diff_texts`)
    "storeBlob" => {
      let id = id_arg()?;
      let name = a.get("name").and_then(Value::as_str).unwrap_or("");
      let bytes = body(stdin).await?;
      let ext = name.rsplit_once('.').map(|(_, e)| format!(".{e}")).unwrap_or_default();
      let fits = blob_name(&ext, &bytes) == name
        || (diff_texts::is_text_blob(name) && diff_texts::decode(&bytes).is_ok_and(|t| diff_texts::name_of(&t) == name));
      if !fits {
        return Err(WireError::new("invalid", format!("{name} does not match its bytes")));
      }
      store.put_blob(&id, name, bytes).await.map_err(failed)?;
      Ok(json!({}))
    }
    // A session deleted where it ran: its copy and blobs go, from the trash too
    "storeRemove" => {
      let id = id_arg()?;
      let leases = crate::store::session_lease::SessionLeases::new(&root, Arc::new(|_: &str| {}));
      if !matches!(leases.claim(&id), crate::store::session_lease::Claim::Held(_)) {
        return Ok(json!({ "written": false }));
      }
      if store.load(&id).await.is_some_and(|r| r.copy_of.is_none()) {
        return Ok(json!({ "takenOver": true }));
      }
      store.remove(&id).await;
      Ok(json!({}))
    }
    other => Err(WireError::new("unknownOp", format!("unknown operation \"{other}\""))),
  }
}

/// A complete record sent in bulk frames, never squeezed into the helper's bounded response line.
pub(crate) async fn get_record(a: &Value) -> Result<Vec<u8>, WireError> {
  let root = a.get("home").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(crate::store::data_dir::acpira_home);
  let id = a.get("id").and_then(Value::as_str).filter(|id| is_session_id(id)).ok_or_else(|| WireError::new("invalid", "invalid session id"))?;
  let store = TranscriptStore::new(root.join("sessions"), Arc::new(|_: &str| {}), None);
  let record = store.load_detailed(id).await.map_err(|e| WireError::new("notFound", e.detail()))?;
  gzip(&serde_json::to_vec(&record).map_err(|e| WireError::new("io", e.to_string()))?).map_err(|e| WireError::new("io", e.to_string()))
}

/// One blob of a session in this host's store opened for `storeBlobGet`, with its length: the store confines both names
/// to its sessions directory
pub(crate) async fn open_blob(a: &Value) -> Result<(tokio::fs::File, u64), WireError> {
  let root = a.get("home").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(crate::store::data_dir::acpira_home);
  let store = TranscriptStore::new(root.join("sessions"), Arc::new(|_: &str| {}), None);
  let id = a.get("id").and_then(Value::as_str).unwrap_or("");
  let name = a.get("name").and_then(Value::as_str).unwrap_or("");
  let Some(path) = store.blob_path(id, name) else { return Err(WireError::new("invalid", format!("\"{id}/{name}\" is not a blob"))) };
  if let Err(e) = tokio::fs::metadata(&path).await {
    return Err(WireError::io(&e, name));
  }
  store.open_blob(id, name).await.map_err(|e| WireError::new("io", format!("{e:#}")))
}

/// Whether the host's turns open with every local one. Ignore presentation-only live state.
fn turns_within(local: &[Turn], host: &[Turn]) -> bool {
  local.len() <= host.len() && local.iter().zip(host).all(|(a, b)| match (a, b) {
    (Turn::User(a), Turn::User(b)) => a.text == b.text && a.attachments == b.attachments,
    (Turn::Agent(a), Turn::Agent(b)) => a.blocks == b.blocks,
    _ => false,
  })
}

/// True only when every retained local turn is already present on the host, a subagent's as much as the session's own:
/// a subagent still writing after the last copy leaves the session's turns as the host has them.
fn is_prefix(local: &SessionRecord, host: &SessionRecord) -> bool {
  let there = host.subagents.as_deref().unwrap_or_default();
  turns_within(&local.turns, &host.turns)
    && local.subagents.iter().flatten().all(|mine| {
      mine.turns.is_empty() || there.iter().any(|theirs| theirs.core.id == mine.core.id && turns_within(&mine.turns, &theirs.turns))
    })
}

/// Save the losing branch before retiring an identity. A copy failure leaves the original intact for a later retry.
pub(crate) async fn preserve_branch(store: &TranscriptStore, local: &SessionRecord, host: &SessionRecord) -> Result<Option<String>> {
  if is_prefix(local, host) { return Ok(None); }
  let mut branch = local.clone();
  branch.id = crate::util::random_uuid();
  branch.acp_session_id = None;
  branch.copy_of = None;
  branch.history_pending = !branch.turns.is_empty();
  branch.history_context = branch.history_pending;
  branch.forked_from = Some(crate::store::record::ForkedFrom {
    session_id: local.id.clone(), turn_index: local.turns.len() as i64 - 1, agent: None, model: None,
  });
  branch.created_at = crate::util::now_iso();
  branch.updated_at = branch.created_at.clone();
  for name in store.blob_names(&local.id).await {
    let bytes = store.read_blob(&local.id, &name).await?;
    store.put_blob(&branch.id, &name, bytes).await?;
  }
  let id = branch.id.clone();
  store.flush(Arc::new(branch)).await?;
  Ok(Some(id))
}

/// Told about deletions on a host (`alias`): a session id as it is deleted here, so the host's list drops it at once;
/// None once the host removed the copies, so the host's list is read again
pub type GoneHook = Arc<dyn Fn(&str, Option<&str>) + Send + Sync>;

/// Deletions still to reach their host, by alias (`<ACPIRA_HOME>/run/remote/gone.json`): on disk, so one made while the
/// host is away survives a restart (its copy would list again as the host's own session), and shared by every engine
/// on this machine
type Tombs = BTreeMap<String, BTreeSet<String>>;

async fn read_tombs(file: &Path) -> Tombs {
  let Ok(raw) = tokio::fs::read(file).await else { return Tombs::new() };
  serde_json::from_slice(&raw).unwrap_or_default()
}

async fn edit_tombs(file: &Path, f: impl FnOnce(&mut Tombs) + Send) -> Result<()> {
  with_file_lock(file, || async move {
    let mut tombs = read_tombs(file).await;
    f(&mut tombs);
    tombs.retain(|_, ids| !ids.is_empty());
    if let Some(dir) = file.parent() {
      tokio::fs::create_dir_all(dir).await?;
    }
    write_atomic(file, &serde_json::to_vec_pretty(&tombs)?, None).await
  })
  .await
}

#[derive(Default)]
struct State {
  /// Sessions whose copy each host needs again, by alias
  dirty: HashMap<String, HashSet<String>>,
  /// Deletions and restores in the order they happened (alias, session, deleted), for the one task that files them
  edits: Option<mpsc::UnboundedSender<(String, String, bool)>>,
  /// A batch is waiting to go
  batching: bool,
  /// Hosts whose channel is up and was compared since
  up: HashSet<String>,
  sub: Option<u64>,
}

pub struct HostMirror {
  connections: Arc<Connections>,
  store: Arc<TranscriptStore>,
  root: PathBuf,
  manager: parking_lot::RwLock<Weak<crate::session_manager::SessionManager>>,
  retiring: parking_lot::Mutex<HashSet<String>>,
  reconcile_lock: tokio::sync::Mutex<()>,
  /// `Tombs`
  gone_file: PathBuf,
  log: LogFn,
  batch: Duration,
  state: parking_lot::Mutex<State>,
  on_gone: parking_lot::RwLock<Option<GoneHook>>,
  me: Weak<HostMirror>,
}

impl HostMirror {
  /// `root`: ACPIRA_HOME
  pub fn new(connections: Arc<Connections>, store: Arc<TranscriptStore>, root: &Path, log: LogFn) -> Arc<HostMirror> {
    Self::with_batch(connections, store, root, log, BATCH)
  }

  /// `new` gathering changes for `batch` (tests)
  pub fn with_batch(connections: Arc<Connections>, store: Arc<TranscriptStore>, root: &Path, log: LogFn, batch: Duration) -> Arc<HostMirror> {
    let gone_file = root.join("run").join("remote").join("gone.json");
    Arc::new_cyclic(|me| HostMirror { connections, store, root: root.to_path_buf(), manager: Default::default(), retiring: Default::default(), reconcile_lock: Default::default(), gone_file, log, batch, state: Default::default(), on_gone: Default::default(), me: me.clone() })
  }

  /// Follow the store's changes and the hosts' connections
  pub fn start(self: &Arc<Self>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<(String, String, bool)>();
    self.state.lock().edits = Some(tx);
    let weak = self.me.clone();
    tokio::spawn(async move {
      while let Some((alias, id, gone)) = rx.recv().await {
        let Some(m) = weak.upgrade() else { return };
        let filed = edit_tombs(&m.gone_file, |t| {
          if m.retiring.lock().contains(&id) { return; }
          let ids = t.entry(alias.clone()).or_default();
          if gone {
            ids.insert(id.clone());
          } else {
            ids.remove(&id);
          }
        });
        if let Err(e) = filed.await {
          (m.log)(&format!("host store: {alias}: the deletion of {id} not filed: {e:#}"));
        }
        if gone {
          m.schedule();
        }
      }
    });
    let weak = self.me.clone();
    let hook: ChangeHook = Arc::new(move |id: &str, change: RecordChange| {
      if let Some(m) = weak.upgrade() {
        m.changed(id, change);
      }
    });
    self.store.set_change_hook(hook);
    let weak = self.me.clone();
    let sub = self.connections.subscribe(Arc::new(move |views: &[ConnectionView]| {
      if let Some(m) = weak.upgrade() {
        m.on_connections(views);
      }
    }));
    self.state.lock().sub = Some(sub);
    self.on_connections(&self.connections.views());
  }

  pub fn stop(&self) {
    let mut st = self.state.lock();
    st.edits = None;
    if let Some(sub) = st.sub.take() {
      self.connections.unsubscribe(sub);
    }
  }

  pub fn set_manager(&self, manager: Weak<crate::session_manager::SessionManager>) {
    *self.manager.write() = manager;
  }

  pub fn set_gone_hook(&self, hook: GoneHook) {
    *self.on_gone.write() = Some(hook);
  }

  fn changed(&self, id: &str, change: RecordChange) {
    if self.retiring.lock().contains(id) { return; }
    let cwd = match change {
      RecordChange::Written { cwd } | RecordChange::Gone { cwd } | RecordChange::Restored { cwd } => cwd,
    };
    let Some(alias) = parse_remote_folder(cwd).map(|f| f.alias) else { return };
    let gone = matches!(change, RecordChange::Gone { .. });
    {
      let mut st = self.state.lock();
      if gone {
        if let Some(d) = st.dirty.get_mut(&alias) {
          d.remove(id);
        }
      } else {
        st.dirty.entry(alias.clone()).or_default().insert(id.to_owned());
      }
      // Only a restore takes a filed deletion back; a write after it is copied again once the removal is done
      if (gone || matches!(change, RecordChange::Restored { .. }))
        && let Some(tx) = &st.edits
      {
        let _ = tx.send((alias.clone(), id.to_owned(), gone));
      }
    }
    if gone {
      let hook = self.on_gone.read().clone();
      if let Some(f) = hook {
        f(&alias, Some(id));
      }
    } else {
      self.schedule();
    }
  }

  fn schedule(&self) {
    {
      let mut st = self.state.lock();
      if st.batching {
        return;
      }
      st.batching = true;
    }
    let (weak, m_batch) = (self.me.clone(), self.batch);
    tokio::spawn(async move {
      tokio::time::sleep(m_batch).await;
      let Some(m) = weak.upgrade() else { return };
      m.state.lock().batching = false;
      m.flush().await;
    });
  }

  /// Send every pending change to the hosts that are up; the rest wait for their host's next connection
  async fn flush(&self) {
    let (up, dirty) = {
      let mut st = self.state.lock();
      let up = st.up.clone();
      let keys: Vec<String> = st.dirty.keys().filter(|a| up.contains(*a)).cloned().collect();
      let dirty: Vec<(String, HashSet<String>)> = keys.into_iter().filter_map(|a| st.dirty.remove(&a).map(|ids| (a, ids))).collect();
      (up, dirty)
    };
    for (alias, ids) in read_tombs(&self.gone_file).await.into_iter().filter(|(a, _)| up.contains(a)) {
      let Some(client) = self.connections.client(&alias) else { continue };
      let mut done = vec![];
      for id in ids {
        if self.retiring.lock().contains(&id) { done.push(id); continue; }
        match client.call("storeRemove", json!({ "id": id })).await {
          Ok(reply) if reply.value.get("takenOver").and_then(Value::as_bool) == Some(true) => {
            if let Err(e) = self.reconcile(&client, &alias, &id).await {
              (self.log)(&format!("host store: {alias}: {id} reconciliation deferred: {e:#}"));
            } else { done.push(id); }
          }
          Ok(reply) if reply.value.get("written").and_then(Value::as_bool) == Some(false) => self.schedule(),
          Ok(_) => done.push(id),
          Err(e) => (self.log)(&format!("host store: {alias}: {id} not removed there: {e}")),
        }
      }
      if done.is_empty() {
        continue;
      }
      let unfiled = edit_tombs(&self.gone_file, |t| {
        if let Some(ids) = t.get_mut(&alias) {
          for id in &done {
            ids.remove(id);
          }
        }
      });
      if let Err(e) = unfiled.await {
        (self.log)(&format!("host store: {alias}: deletions done there not unfiled: {e:#}"));
      }
      // The host's engine reads its directory again when its list is asked for again
      let hook = self.on_gone.read().clone();
      if let Some(f) = hook {
        f(&alias, None);
      }
    }
    for (alias, ids) in dirty {
      let Some(client) = self.connections.client(&alias) else {
        self.requeue(&alias, ids);
        continue;
      };
      for id in ids {
        if let Err(e) = self.push(&client, &alias, &id).await {
          (self.log)(&format!("host store: {alias}: {id} not copied: {e:#}"));
          self.requeue(&alias, [id].into());
          self.schedule();
        }
      }
    }
  }

  fn requeue(&self, alias: &str, ids: HashSet<String>) {
    self.state.lock().dirty.entry(alias.to_owned()).or_default().extend(ids);
  }

  /// A host that came up gets compared; one that went down waits for its next connection
  fn on_connections(&self, views: &[ConnectionView]) {
    let came: Vec<String> = {
      let mut st = self.state.lock();
      let connected: HashSet<String> = views.iter().filter(|v| v.state == ConnectionState::Connected).map(|v| v.alias.clone()).collect();
      st.up.retain(|a| connected.contains(a));
      let came: Vec<String> = connected.into_iter().filter(|a| !st.up.contains(a)).collect();
      st.up.extend(came.iter().cloned());
      came
    };
    for alias in came {
      let weak = self.me.clone();
      tokio::spawn(async move {
        if let Some(m) = weak.upgrade() {
          m.compare(&alias).await;
        }
      });
    }
  }

  /// Bring the host's copies of this engine's records in its folders up to date: missing or older ones go again, with
  /// their blobs. Deletions made while it was away go first
  async fn compare(&self, alias: &str) {
    let Some(client) = self.connections.client(alias) else { return };
    let mine: Vec<(String, String)> = match self.store.load_index().await {
      Ok(list) => list.into_iter().filter(|s| parse_remote_folder(&s.cwd).is_some_and(|f| f.alias == alias)).map(|s| (s.id, s.updated_at)).collect(),
      Err(e) => {
        (self.log)(&format!("host store: {alias}: the list could not be read: {e:#}"));
        return;
      }
    };
    // Changes made while it was away (a deletion, a rename that kept `updatedAt`) go with the next batch
    let waiting = self.state.lock().dirty.get(alias).is_some_and(|d| !d.is_empty()) || read_tombs(&self.gone_file).await.contains_key(alias);
    if waiting {
      self.schedule();
    }
    if mine.is_empty() {
      return;
    }
    let ids: Vec<&str> = mine.iter().map(|(id, _)| id.as_str()).collect();
    let stamps: HashMap<String, CopyStamp> = match client.call("storeStat", json!({ "ids": ids })).await {
      Ok(r) => serde_json::from_value(r.value.get("records").cloned().unwrap_or(Value::Null)).unwrap_or_default(),
      Err(e) => {
        (self.log)(&format!("host store: {alias}: not compared: {e}"));
        return;
      }
    };
    let mut sent = 0;
    for (id, updated_at) in &mine {
      if stamps.get(id).is_some_and(|s| s.taken_over || s.copy_of.as_ref().is_some_and(|c| c.machine != this_machine().0)) {
        if let Err(e) = self.reconcile(&client, alias, id).await {
          (self.log)(&format!("host store: {alias}: {id} reconciliation deferred: {e:#}"));
          self.requeue(alias, [id.clone()].into());
          self.schedule();
        }
        continue;
      }
      let behind = match stamps.get(id) {
        None => true,
        Some(there) => there.updated_at < *updated_at || self.store.blob_names(id).await.iter().any(|b| !there.blobs.contains(b)),
      };
      if !behind {
        continue;
      }
      match self.push(&client, alias, id).await {
        Ok(()) => sent += 1,
        Err(e) => (self.log)(&format!("host store: {alias}: {id} not copied: {e:#}")),
      }
    }
    if sent > 0 {
      (self.log)(&format!("host store: {alias}: {sent} session(s) copied to the host"));
    }
  }

  /// The host now owns this identity. Keep any local divergent work before retiring it, never as a deletion there.
  async fn reconcile(&self, client: &RemoteClient, alias: &str, id: &str) -> Result<()> {
    let _serial = self.reconcile_lock.lock().await;
    let reply = client.call("storeGet", json!({ "id": id })).await.map_err(|e| anyhow!("{e}"))?;
    let host: SessionRecord = serde_json::from_slice(&gunzip(&reply.data)?)?;
    if host.id != id || host.copy_of.as_ref().is_some_and(|c| c.machine == this_machine().0) {
      return Err(anyhow!("host ownership changed while reconciling"));
    }
    let manager = self.manager.read().upgrade();
    self.retiring.lock().insert(id.to_owned());
    let result = match manager {
      Some(manager) => manager.retire_host_copy(id, &host).await,
      None => async {
        let leases = crate::store::session_lease::SessionLeases::new(&self.root, self.log.clone());
        if !matches!(leases.claim(id), crate::store::session_lease::Claim::Held(_)) {
          return Err(anyhow!("local session is held by another engine"));
        }
        if let Some(local) = self.store.load(id).await {
          preserve_branch(&self.store, &local, &host).await?;
          self.store.remove(id).await;
          self.store.sync_index(&[], &HashSet::new()).await?;
        }
        Ok(())
      }.await,
    };
    if let Err(e) = result {
      self.retiring.lock().remove(id);
      return Err(e);
    }
    // Keep the id suppressed: already queued change notifications must not resurrect its deletion later.
    if let Some(ids) = self.state.lock().dirty.get_mut(alias) { ids.remove(id); }
    edit_tombs(&self.gone_file, |t| {
      if let Some(ids) = t.get_mut(alias) { ids.remove(id); }
    }).await?;
    if let Some(hook) = self.on_gone.read().clone() { hook(alias, None); }
    Ok(())
  }

  /// One record's copy and the blobs the host lacks. The folder becomes the host's own path; the category stays here
  /// (categories are this machine's list)
  async fn push(&self, client: &RemoteClient, alias: &str, id: &str) -> Result<()> {
    if self.retiring.lock().contains(id) { return Ok(()); }
    // The wire format stays a complete record whether the local body is a file, a packed archive or SQLite rows.
    let Some(record) = self.store.load(id).await else { return Ok(()) };
    let mut record: Value = serde_json::to_value(record)?;
    let Some(folder) = record.get("cwd").and_then(Value::as_str).and_then(parse_remote_folder) else { return Ok(()) };
    if folder.alias != alias {
      return Ok(());
    }
    let obj = record.as_object_mut().ok_or_else(|| anyhow!("not a record"))?;
    obj.insert("cwd".into(), Value::String(folder.path));
    obj.remove("category");
    let (machine, name) = this_machine();
    obj.insert("copyOf".into(), json!({ "machine": machine, "name": name }));
    let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::new(6));
    gz.write_all(&serde_json::to_vec(&record)?)?;
    let reply = call_with_body(client, "storePut", json!({}), &gz.finish()?).await?;
    let there: Vec<String> = serde_json::from_value(reply.get("blobs").cloned().unwrap_or(Value::Null)).unwrap_or_default();
    if reply.get("takenOver").and_then(Value::as_bool) == Some(true) {
      return self.reconcile(client, alias, id).await;
    }
    if reply.get("written").and_then(Value::as_bool) == Some(false) {
      (self.log)(&format!("host store: {alias}: {id} kept as the host has it (newer there, or deleted there)"));
      return Ok(());
    }
    if record.get("agent").and_then(Value::as_str) == Some("acpira")
      && let Some(sid) = record.get("acpSessionId").and_then(Value::as_str).filter(|s| is_session_id(s))
      && let Ok(bytes) = tokio::fs::read(self.root.join("agent/sessions").join(format!("{sid}.jsonl"))).await
    {
      let call = client.request("storeAgentLog", json!({ "sessionId": id, "id": sid }));
      for chunk in gzip(&bytes)?.chunks(wire::CHUNK) {
        call.stdin(chunk);
      }
      call.stdin_eof();
      match call.collect().await {
        Ok(reply) if reply.value.get("takenOver").and_then(Value::as_bool) == Some(true) => return self.reconcile(client, alias, id).await,
        // The host dropped the log of its last copy when it took this record: the copy is whole again only with this
        // one, so a log that did not land goes again with the record
        Ok(reply) if reply.value.get("written").and_then(Value::as_bool) == Some(false) => return Err(anyhow!("native log {sid}: the host is busy with the session")),
        Ok(_) => {}
        // Older helpers have no native-log operation; the transcript handoff still works there.
        Err(e) if e.code() == "unknownOp" => (self.log)(&format!("host store: {alias}: native log {sid} not copied: {e}")),
        Err(e) => return Err(anyhow!("native log {sid}: {e}")),
      }
    }
    for name in self.store.blob_names(id).await.into_iter().filter(|n| !there.contains(n)) {
      let bytes = self.store.read_blob(id, &name).await?;
      call_with_body(client, "storeBlob", json!({ "id": id, "name": name }), &bytes).await?;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn body_of(bytes: Vec<u8>) -> Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>> {
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(Some(bytes)).unwrap();
    tx.send(None).unwrap();
    Some(rx)
  }

  #[tokio::test]
  async fn owned_records_and_native_logs_refuse_later_foreign_writes() {
    let dir = tempfile::tempdir().unwrap();
    let store = TranscriptStore::new(dir.path().join("sessions"), Arc::new(|_| {}), None);
    let mut record: SessionRecord = serde_json::from_value(json!({
      "id": "session", "agent": "acpira", "acpSessionId": "native", "cwd": "/work", "title": "owned",
      "updatedAt": "2026-01-01T00:00:00.000Z", "turns": []
    })).unwrap();
    store.flush(Arc::new(record.clone())).await.unwrap();
    record.copy_of = Some(crate::store::record::CopyOf { machine: "source".into(), name: "Source".into() });
    record.updated_at = "2999-01-01T00:00:00.000Z".into();
    let args = json!({ "home": dir.path(), "sessionId": "session", "id": "native" });
    let result = serve("storePut", &args, body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
    assert_eq!(result["written"], false);
    assert_eq!(result["takenOver"], true);
    assert!(store.load("session").await.unwrap().copy_of.is_none());
    let result = serve("storeAgentLog", &args, body_of(gzip(b"do not replace").unwrap())).await.unwrap();
    assert_eq!(result["written"], false);
    assert!(!dir.path().join("agent/sessions/native.jsonl").exists());
    let bad = json!({ "home": dir.path(), "sessionId": "session", "id": "../outside" });
    assert!(serve("storeAgentLog", &bad, body_of(gzip(b"bad").unwrap())).await.is_err());
    let bytes = get_record(&json!({ "home": dir.path(), "id": "session" })).await.unwrap();
    assert!(serde_json::from_slice::<SessionRecord>(&gunzip(&bytes).unwrap()).unwrap().copy_of.is_none());
  }

  #[tokio::test]
  async fn a_new_copy_drops_the_native_log_of_the_one_it_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let mut record: SessionRecord = serde_json::from_value(json!({
      "id": "session", "agent": "acpira", "acpSessionId": "native", "cwd": "/work", "title": "copied",
      "updatedAt": "2026-01-01T00:00:00.000Z", "turns": [], "copyOf": { "machine": "source", "name": "Source" }
    })).unwrap();
    let args = json!({ "home": dir.path(), "sessionId": "session", "id": "native" });
    let log = dir.path().join("agent/sessions/native.jsonl");
    let put = serve("storePut", &args, body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
    assert_eq!(put["written"], true);
    assert_eq!(serve("storeAgentLog", &args, body_of(gzip(b"first").unwrap())).await.unwrap()["written"], true);
    assert_eq!(std::fs::read(&log).unwrap(), b"first");
    // The next copy lands and its log does not (yet): a promotion now must not resume the context of the copy before
    record.updated_at = "2026-01-02T00:00:00.000Z".into();
    let put = serve("storePut", &args, body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
    assert_eq!(put["written"], true);
    assert!(!log.exists());
    assert_eq!(serve("storeAgentLog", &args, body_of(gzip(b"second").unwrap())).await.unwrap()["written"], true);
    assert_eq!(std::fs::read(&log).unwrap(), b"second");
  }

  #[test]
  fn a_subagent_ahead_of_the_host_is_local_work_to_keep() {
    let host: SessionRecord = serde_json::from_value(json!({
      "id": "session", "agent": "claude", "cwd": "/work", "updatedAt": "1", "turns": [{ "role": "user", "text": "go" }],
      "subagents": [{ "id": "child", "turnIndex": 0, "visibility": "session", "state": "running", "stateSource": "agent", "announcedAt": 1,
        "turns": [{ "role": "user", "text": "task" }] }]
    })).unwrap();
    let mut local = host.clone();
    assert!(is_prefix(&local, &host));
    local.subagents.as_mut().unwrap()[0].turns.push(serde_json::from_value(json!({ "role": "user", "text": "written after the last copy" })).unwrap());
    assert!(!is_prefix(&local, &host), "the session's own turns match, the subagent's do not");
    assert!(is_prefix(&host, &local));
  }
}
