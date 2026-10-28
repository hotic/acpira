//! One session history per host (docs/dev/remote.md): a folder on a host has the sessions in the host's own store,
//! whichever run mode is used. A session this engine drives in a host folder (local run) keeps its record here, written
//! by its owner as any session's, and a copy goes into the host's store with the folder as the host names it, so the
//! host's engine lists it beside its own and remote run, the host's other clients and other machines see one history.
//! The copy names this machine (`copyOf`): every other engine shows it read-only, so only this machine ever writes it.
//!
//! `serve` is the helper's side (the host's store, through the same `TranscriptStore` code); `HostMirror` this engine's:
//! told of every record change by the store, it sends the copies of host folders' records in batches while the host is
//! connected, and on every connection compares what the host has, which also brings over records written before this
//! existed. A copy never replaces one the host updated later, nor one deleted there and still in its trash

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use acpira_shared::remote::{ConnectionState, ConnectionView, parse_remote_folder};

use super::client::RemoteClient;
use super::connections::Connections;
use super::wire::{self, WireError};
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
async fn body(stdin: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Vec<u8>, WireError> {
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
async fn call_with_body(client: &RemoteClient, op: &str, a: Value, bytes: &[u8]) -> Result<Value> {
  let call = client.request(op, a);
  for chunk in bytes.chunks(wire::CHUNK) {
    call.stdin(chunk);
  }
  call.stdin_eof();
  Ok(call.collect().await.map_err(|e| anyhow!("{e}"))?.value)
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
      let mut raw = vec![];
      std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&gz[..]), &mut raw).map_err(|e| WireError::new("invalid", format!("not gzip: {e}")))?;
      let record: SessionRecord = serde_json::from_slice(&raw).map_err(|e| WireError::new("invalid", format!("not a session record: {e}")))?;
      let written = store.put_copy(&record).await.map_err(failed)?;
      Ok(json!({ "written": written, "blobs": store.blob_names(&record.id).await }))
    }
    // One blob of a session, named by its content as every blob is: a name that does not match the bytes is refused
    "storeBlob" => {
      let id = id_arg()?;
      let name = a.get("name").and_then(Value::as_str).unwrap_or("");
      let bytes = body(stdin).await?;
      let ext = name.rsplit_once('.').map(|(_, e)| format!(".{e}")).unwrap_or_default();
      if blob_name(&ext, &bytes) != name {
        return Err(WireError::new("invalid", format!("{name} does not match its bytes")));
      }
      store.save_blob(&id, &ext, &bytes).await.map_err(failed)?;
      Ok(json!({}))
    }
    // A session deleted where it ran: its copy and blobs go, from the trash too
    "storeRemove" => {
      store.remove(&id_arg()?).await;
      Ok(json!({}))
    }
    other => Err(WireError::new("unknownOp", format!("unknown operation \"{other}\""))),
  }
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
    Arc::new_cyclic(|me| HostMirror { connections, store, gone_file, log, batch, state: Default::default(), on_gone: Default::default(), me: me.clone() })
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

  pub fn set_gone_hook(&self, hook: GoneHook) {
    *self.on_gone.write() = Some(hook);
  }

  fn changed(&self, id: &str, change: RecordChange) {
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
        match client.call("storeRemove", json!({ "id": id })).await {
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

  /// One record's copy and the blobs the host lacks. The folder becomes the host's own path; the category stays here
  /// (categories are this machine's list)
  async fn push(&self, client: &RemoteClient, alias: &str, id: &str) -> Result<()> {
    let raw = match tokio::fs::read(self.store.dir().join(format!("{id}.json"))).await {
      Ok(raw) => raw,
      // Deleted meanwhile: its removal is on its way
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
      Err(e) => return Err(e.into()),
    };
    let mut record: Value = serde_json::from_slice(&raw)?;
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
    if reply.get("written").and_then(Value::as_bool) == Some(false) {
      (self.log)(&format!("host store: {alias}: {id} kept as the host has it (newer there, or deleted there)"));
      return Ok(());
    }
    for name in self.store.blob_names(id).await.into_iter().filter(|n| !there.contains(n)) {
      let bytes = self.store.read_blob(id, &name).await?;
      call_with_body(client, "storeBlob", json!({ "id": id, "name": name }), &bytes).await?;
    }
    Ok(())
  }
}
