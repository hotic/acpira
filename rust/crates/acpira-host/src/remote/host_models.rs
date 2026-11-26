//! Model sources follow the user to their hosts (docs/dev/remote.md). A host whose engine reads a client's settings
//! (`SET_FEATURE`) keeps this engine's whole built-in agent settings apart, per machine (`put_set`), and the sessions
//! this machine drives there read them instead of the host's own files. An older host engine reads only its own
//! `providers.json`, so there this engine's sources and their keys are merged in instead (`merge`). Either way they go
//! to every connected host when it connects and whenever they change here; the host's built-in agent picks the files up
//! by itself.
//!
//! `merge` is the helper's older side. A source it writes names the machine that brought it (`SYNCED_FROM_FIELD`), and
//! only that machine's entries are replaced or taken away again. The host's own source under the same id wins: a host
//! is often a machine the user also sits at, and its settings page edits that file. `ModelMirror` is this engine's side

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use acpira_shared::providers::{
  PROVIDER_SECRET_PREFIX, PROVIDERS_FILE, PROVIDERS_VERSION, Provider, ProvidersFile, SYNCED_FROM_FIELD, WEB_SECRET_PREFIX, client_settings_dir, provider_secret_key,
};
use acpira_agent::config::SECRETS_FILE;
use acpira_shared::remote::{ConnectionState, ConnectionView};

use super::connections::Connections;
use super::host_store::{body, send_body, this_machine};
use super::wire::WireError;
use crate::accounts::account_store::FileVault;
use crate::providers::ProviderStore;
use crate::store::file_lock::{with_file_lock, write_atomic};
use crate::store::transcript_store::LogFn;

/// The helper operation that merges a machine's sources into the host's own (an older host engine)
pub const PUT_OP: &str = "modelsPut";

/// The helper operation that keeps a machine's built-in agent settings apart for the sessions it drives on the host
pub const SET_OP: &str = "clientSettingsPut";

/// What a host engine that reads a client's settings says in `helloOk.features`
pub const SET_FEATURE: &str = "clientSettings";

/// A save writes the entry, then its key: both go in one push
const SETTLE: Duration = Duration::from_millis(300);

/// What one machine carries to a host: its sources as its file holds them, and the key of each that has one
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Carried {
  #[serde(default)]
  pub providers: Vec<Provider>,
  #[serde(default)]
  pub keys: BTreeMap<String, String>,
}

/// What one machine keeps on a host for the sessions it drives there (`SET_OP`): its settings file with only the sources
/// that travel, and the keys those sources and its search backends use, by vault key
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientSet {
  #[serde(default)]
  pub file: ProvidersFile,
  #[serde(default)]
  pub keys: BTreeMap<String, String>,
}

/// A source that means something on another machine: not a server found on this one, nor an address that names this
/// machine itself, nor one another machine brought here
pub fn travels(p: &Provider) -> bool {
  !p.keyless() && !is_loopback(&p.base_url) && !p.extra.contains_key(SYNCED_FROM_FIELD)
}

fn is_loopback(url: &str) -> bool {
  let rest = url.split_once("://").map_or(url, |(_, r)| r);
  let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
  let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
  // `[::1]:8080`, else `host:port`
  let host = match host.strip_prefix('[') {
    Some(v6) => v6.split(']').next().unwrap_or(""),
    None => host.split(':').next().unwrap_or(""),
  };
  let host = host.to_ascii_lowercase();
  host == "localhost" || host.ends_with(".localhost") || host == "::1" || host == "0.0.0.0" || host.starts_with("127.")
}

/// The vault key holding the key of the host's own source a carried one stood in for (written by older helpers)
pub use acpira_shared::providers::kept_secret_key;

/// The sources that go to this machine's SSH hosts, with their keys.
pub async fn carried_of(store: &ProviderStore) -> Result<Carried, String> {
  let providers: Vec<Provider> = store.read().await?.providers.into_iter().filter(travels).collect();
  let mut keys = BTreeMap::new();
  for p in &providers {
    if let Some(k) = store.vault().get(&provider_secret_key(&p.id)).await.ok().flatten().filter(|k| !k.is_empty()) {
      keys.insert(p.id.clone(), k);
    }
  }
  Ok(Carried { providers, keys })
}

/// This machine's whole built-in agent settings for sessions it drives on an SSH host keeping them apart.
pub async fn client_set_of(store: &ProviderStore) -> Result<ClientSet, String> {
  let mut file = store.read().await?;
  file.providers.retain(travels);
  let wanted: Vec<String> = file
    .providers
    .iter()
    .map(|p| provider_secret_key(&p.id))
    .chain(acpira_shared::providers::WEB_SEARCH_BACKENDS.into_iter().skip(1).map(acpira_shared::providers::web_secret_key))
    .collect();
  let mut keys = BTreeMap::new();
  for key in wanted {
    if let Some(k) = store.vault().get(&key).await.ok().flatten().filter(|k| !k.is_empty()) {
      keys.insert(key, k);
    }
  }
  Ok(ClientSet { file, keys })
}

/// The machine that brought a source, and the host's own entry it stands in for
fn brought_by(p: &Provider) -> Option<(&str, Option<&Value>)> {
  let from = p.extra.get(SYNCED_FROM_FIELD)?;
  Some((from.get("machine").and_then(Value::as_str)?, from.get("kept")))
}

async fn set_secret(vault: &FileVault, key: &str, value: Option<&str>) -> Result<()> {
  let now = vault.get(key).await?;
  match value {
    Some(v) if now.as_deref() != Some(v) => vault.store(key, v).await,
    None if now.is_some() => vault.delete(key).await,
    _ => Ok(()),
  }
}

/// Make this host's sources hold exactly what `machine` carries, leaving every other entry alone. True when the file
/// changed. A file that does not parse is left as it is
pub async fn merge(root: &Path, machine: &str, name: &str, carried: Carried) -> Result<bool> {
  let file = root.join(PROVIDERS_FILE);
  let vault = FileVault::new(root.join("secrets.json"), Arc::new(|_: &str| {}));
  with_file_lock(&file, || async {
    let mut shape = match tokio::fs::read_to_string(&file).await {
      Ok(text) => ProvidersFile::parse(&text).map_err(|e| anyhow!("{} cannot be read: {e}", file.display()))?,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProvidersFile::default(),
      Err(e) => return Err(e.into()),
    };
    let before = shape.clone();
    let incoming: HashSet<&str> = carried.providers.iter().map(|p| p.id.as_str()).collect();

    // Entries brought here before: one standing in for the host's own source (an older helper replaced it) gives the
    // host's entry and key back, whoever brought it; one this machine brought and no longer carries goes
    let mut list = Vec::with_capacity(shape.providers.len());
    for p in std::mem::take(&mut shape.providers) {
      let Some((from, kept)) = brought_by(&p) else {
        list.push(p);
        continue;
      };
      let own = kept.and_then(|k| serde_json::from_value::<Provider>(k.clone()).ok());
      let gone = from == machine && !incoming.contains(p.id.as_str());
      if own.is_none() && !gone {
        list.push(p);
        continue;
      }
      let own_key = vault.get(&kept_secret_key(&p.id)).await?;
      set_secret(&vault, &provider_secret_key(&p.id), own_key.as_deref()).await?;
      set_secret(&vault, &kept_secret_key(&p.id), None).await?;
      match own {
        Some(own) => list.push(own),
        // As a deletion on the settings page: search that went through the source goes back to its default
        None if shape.web_search_via.as_deref() == Some(p.id.as_str()) => shape.web_search_via = None,
        None => {}
      }
    }

    for mut p in carried.providers {
      let slot = list.iter().position(|e| e.id == p.id);
      // The host's own source under this id wins: it is never stood in for
      if slot.is_some_and(|i| !list[i].extra.contains_key(SYNCED_FROM_FIELD)) {
        continue;
      }
      // The key first: a turn that already sees the source finds its key with it
      set_secret(&vault, &provider_secret_key(&p.id), carried.keys.get(&p.id).map(String::as_str)).await?;
      p.extra.insert(SYNCED_FROM_FIELD.into(), json!({ "machine": machine, "name": name }));
      match slot {
        Some(i) => list[i] = p,
        None => list.push(p),
      }
    }
    shape.providers = list;
    if shape == before {
      return Ok(false);
    }
    shape.version = shape.version.max(PROVIDERS_VERSION);
    if let Some(dir) = file.parent() {
      tokio::fs::create_dir_all(dir).await?;
    }
    write_atomic(&file, serde_json::to_string_pretty(&shape)?.as_bytes(), None).await?;
    Ok(true)
  })
  .await
}

/// Keep `set` as what `machine` brings for the host's sessions it drives, or drop it (None: the host's setting is off on
/// that machine). Whatever the machine merged into the host's own file before goes first, so the two never stand side
/// by side. True when anything changed
pub async fn put_set(root: &Path, machine: &str, name: &str, set: Option<ClientSet>) -> Result<bool> {
  let took_back = merge(root, machine, name, Carried::default()).await?;
  let wrote = match set {
    Some(set) => write_set(root, machine, set).await?,
    None => drop_set(root, machine).await?,
  };
  Ok(took_back || wrote)
}

/// A machine's settings folder, its parent made: the folder's own path is what it is locked under, so the lock file
/// sits beside the folder, which a drop removes whole
async fn set_dir(root: &Path, machine: &str) -> Result<std::path::PathBuf> {
  let dir = client_settings_dir(root, machine).ok_or_else(|| anyhow!("not a usable machine id: {machine}"))?;
  if let Some(parent) = dir.parent() {
    tokio::fs::create_dir_all(parent).await?;
  }
  Ok(dir)
}

async fn write_set(root: &Path, machine: &str, mut set: ClientSet) -> Result<bool> {
  let dir = set_dir(root, machine).await?;
  // Only the keys the agent reads: a machine's other secrets never land here
  set.keys.retain(|k, v| (k.starts_with(PROVIDER_SECRET_PREFIX) || k.starts_with(WEB_SECRET_PREFIX)) && !v.is_empty());
  set.file.version = set.file.version.max(PROVIDERS_VERSION);
  let (keys, file) = (serde_json::to_vec_pretty(&set.keys)?, serde_json::to_vec_pretty(&set.file)?);
  with_file_lock(&dir.clone(), || async move {
    tokio::fs::create_dir_all(&dir).await?;
    // The keys first: a turn that already sees a source finds its key with it
    let keys_changed = write_changed(&dir.join(SECRETS_FILE), &keys, Some(0o600)).await?;
    let file_changed = write_changed(&dir.join(PROVIDERS_FILE), &file, None).await?;
    Ok(keys_changed || file_changed)
  })
  .await
}

/// Remove what `machine` keeps here for its sessions. True when there was something
pub async fn drop_set(root: &Path, machine: &str) -> Result<bool> {
  let dir = set_dir(root, machine).await?;
  with_file_lock(&dir.clone(), || async move {
    match tokio::fs::remove_dir_all(&dir).await {
      Ok(()) => Ok(true),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
      Err(e) => Err(e.into()),
    }
  })
  .await
}

/// A file left alone when it already holds these bytes: the agent re-reads on every change of its modification time
async fn write_changed(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<bool> {
  if tokio::fs::read(path).await.is_ok_and(|now| now == bytes) {
    return Ok(false);
  }
  write_atomic(path, bytes, mode).await?;
  Ok(true)
}

/// The request's data root, and the machine it names with that machine's name
fn target(a: &Value) -> Result<(std::path::PathBuf, String, String), WireError> {
  let root = a.get("home").and_then(Value::as_str).map(std::path::PathBuf::from).unwrap_or_else(crate::store::data_dir::acpira_home);
  let machine = a.get("machine").and_then(Value::as_str).filter(|m| !m.is_empty()).ok_or_else(|| WireError::new("invalid", "no machine"))?;
  let name = a.get("name").and_then(Value::as_str).unwrap_or(machine).to_owned();
  Ok((root, machine.to_owned(), name))
}

fn unusable(providers: &[Provider]) -> bool {
  providers.iter().any(|p| p.id.is_empty() || p.id.contains('/'))
}

/// The helper's `modelsPut`: a machine's `Carried` as the body. A machine that sends it merges again (an older build,
/// or a host engine that reads only its own file), so what it kept here apart goes
pub(crate) async fn serve(a: &Value, stdin: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  let (root, machine, name) = target(a)?;
  let carried: Carried = serde_json::from_slice(&body(stdin).await?).map_err(|e| WireError::new("invalid", format!("not model sources: {e}")))?;
  if unusable(&carried.providers) {
    return Err(WireError::new("invalid", "a source without a usable id"));
  }
  let io = |e: anyhow::Error| WireError::new("io", format!("{e:#}"));
  let mut changed = merge(&root, &machine, &name, carried).await.map_err(io)?;
  if client_settings_dir(&root, &machine).is_some() {
    changed |= drop_set(&root, &machine).await.map_err(io)?;
  }
  Ok(json!({ "changed": changed }))
}

/// The helper's `clientSettingsPut`: a machine's `ClientSet` as the body, or `null` to drop it
pub(crate) async fn serve_set(a: &Value, stdin: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  let (root, machine, name) = target(a)?;
  if client_settings_dir(&root, &machine).is_none() {
    return Err(WireError::new("invalid", "not a usable machine id"));
  }
  let set: Option<ClientSet> = serde_json::from_slice(&body(stdin).await?).map_err(|e| WireError::new("invalid", format!("not agent settings: {e}")))?;
  if set.as_ref().is_some_and(|s| unusable(&s.file.providers)) {
    return Err(WireError::new("invalid", "a source without a usable id"));
  }
  let changed = put_set(&root, &machine, &name, set).await.map_err(|e| WireError::new("io", format!("{e:#}")))?;
  Ok(json!({ "changed": changed }))
}

/// How a connected host was last sent this machine's settings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sent {
  /// Its setting was on: settings went (off: none, which takes back what an earlier push brought)
  on: bool,
  /// Its engine reads a client's settings (`SET_FEATURE`): they went apart, else merged into its own file
  apart: bool,
}

#[derive(Default)]
struct State {
  /// Connected hosts whose engine has answered, and what each was last sent
  up: HashMap<String, Sent>,
  /// A change here is waiting to go
  settling: bool,
  sub: Option<u64>,
}

/// This engine's settings kept on every connected host
pub struct ModelMirror {
  connections: Arc<Connections>,
  providers: Arc<ProviderStore>,
  log: LogFn,
  state: parking_lot::Mutex<State>,
  /// One push at a time: a later read of the settings never reaches a host before an earlier one
  sending: tokio::sync::Mutex<()>,
  me: Weak<ModelMirror>,
}

impl ModelMirror {
  pub fn new(connections: Arc<Connections>, providers: Arc<ProviderStore>, log: LogFn) -> Arc<ModelMirror> {
    Arc::new_cyclic(|me| ModelMirror { connections, providers, log, state: Default::default(), sending: Default::default(), me: me.clone() })
  }

  /// Follow the settings' changes and the hosts' connections
  pub fn start(self: &Arc<Self>) {
    let weak = self.me.clone();
    self.providers.on_change(Arc::new(move || {
      if let Some(m) = weak.upgrade() {
        m.changed();
      }
    }));
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
    if let Some(sub) = self.state.lock().sub.take() {
      self.connections.unsubscribe(sub);
    }
  }

  fn changed(&self) {
    {
      let mut st = self.state.lock();
      if st.settling {
        return;
      }
      st.settling = true;
    }
    let weak = self.me.clone();
    tokio::spawn(async move {
      tokio::time::sleep(SETTLE).await;
      let Some(m) = weak.upgrade() else { return };
      let hosts: Vec<String> = {
        let mut st = m.state.lock();
        st.settling = false;
        st.up.iter().filter(|(_, s)| s.on).map(|(a, _)| a.clone()).collect()
      };
      for alias in hosts {
        m.push(&alias).await;
      }
    });
  }

  /// A host that came up, whose setting changed, or whose engine answered differently is sent the settings. One whose
  /// engine has not answered yet waits for it, so a connection never merges first only to take it back a moment later;
  /// with no engine there, no session of the host could use them anyway
  fn on_connections(&self, views: &[ConnectionView]) {
    let todo: Vec<String> = {
      let mut st = self.state.lock();
      let connected: HashMap<&str, Option<Sent>> = views
        .iter()
        .filter(|v| v.state == ConnectionState::Connected)
        .map(|v| (v.alias.as_str(), self.connections.engine_offers(&v.alias, SET_FEATURE).map(|apart| Sent { on: !v.models_off, apart })))
        .collect();
      st.up.retain(|a, _| connected.get(a.as_str()).is_some_and(Option::is_some));
      let todo: Vec<(String, Sent)> =
        connected.iter().filter_map(|(a, s)| Some(((*a).to_owned(), (*s)?))).filter(|(a, s)| st.up.get(a) != Some(s)).collect();
      st.up.extend(todo.iter().cloned());
      todo.into_iter().map(|(a, _)| a).collect()
    };
    for alias in todo {
      let weak = self.me.clone();
      tokio::spawn(async move {
        if let Some(m) = weak.upgrade() {
          m.push(&alias).await;
        }
      });
    }
  }

  async fn push(&self, alias: &str) {
    let _one = self.sending.lock().await;
    // What the host is now, not what it was when this push was queued: another may have waited ahead of it
    let Some(sent) = self.state.lock().up.get(alias).copied() else { return };
    // The channel and the way its own engine takes the settings, together: a helper replaced since has an engine yet
    // to answer, and its hello queues the next push
    let Some((client, apart)) = self.connections.engine_client(alias, SET_FEATURE) else { return };
    let sent = Sent { apart, ..sent };
    let (machine, name) = this_machine();
    let a = json!({ "machine": machine, "name": name });
    if sent.apart {
      let set = if sent.on {
        match client_set_of(&self.providers).await {
          Ok(s) => Some(s),
          // A file that cannot be read says nothing about what the host should hold
          Err(e) => return (self.log)(&format!("host models: {alias}: not sent: {e}")),
        }
      } else {
        None
      };
      match send_body(&client, SET_OP, a.clone(), &serde_json::to_vec(&set).unwrap_or_default()).await {
        Ok(reply) => {
          self.connections.set_settings_apart(alias, true);
          if reply.get("changed").and_then(Value::as_bool) == Some(true) {
            let count = set.as_ref().map_or(0, |s| s.file.providers.len());
            (self.log)(&format!("host models: {alias}: {count} source(s) kept on the host for this machine's sessions"));
          }
          return;
        }
        // A helper older than the engine there (a local build deploys the release it was built on): merged instead
        Err(e) if e.code() == "unknownOp" => {}
        Err(e) => return (self.log)(&format!("host models: {alias}: not sent: {e}")),
      }
    }
    let carried = if sent.on {
      match carried_of(&self.providers).await {
        Ok(c) => c,
        Err(e) => return (self.log)(&format!("host models: {alias}: not sent: {e}")),
      }
    } else {
      Carried::default()
    };
    let merged = send_body(&client, PUT_OP, a, &serde_json::to_vec(&carried).unwrap_or_default()).await;
    if merged.is_ok() {
      self.connections.set_settings_apart(alias, false);
    }
    match merged {
      Ok(reply) if reply.get("changed").and_then(Value::as_bool) == Some(true) => {
        (self.log)(&format!("host models: {alias}: {} source(s) on the host", carried.providers.len()))
      }
      Ok(_) => {}
      // A helper from before this existed answers `unknownOp`: its host keeps its own sources
      Err(e) => (self.log)(&format!("host models: {alias}: not sent: {e}")),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn source(id: &str, url: &str, models: &[&str]) -> Provider {
    serde_json::from_value(json!({ "id": id, "baseUrl": url, "models": models.iter().map(|m| json!({ "id": m })).collect::<Vec<_>>() })).unwrap()
  }

  fn carried(sources: &[(Provider, Option<&str>)]) -> Carried {
    Carried {
      providers: sources.iter().map(|(p, _)| p.clone()).collect(),
      keys: sources.iter().filter_map(|(p, k)| k.map(|k| (p.id.clone(), k.to_owned()))).collect(),
    }
  }

  fn read(root: &Path, file: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root.join(file)).unwrap()).unwrap()
  }

  #[test]
  fn only_sources_another_machine_can_reach_travel() {
    for (url, goes) in [
      ("https://api.test/v1", true),
      ("http://100.93.1.2:30000/v1", true),
      ("http://127.0.0.1:4000/v1", false),
      ("http://localhost:11434", false),
      ("http://user@LOCALHOST/v1", false),
      ("http://[::1]:8080/v1", false),
      ("http://0.0.0.0:1234", false),
    ] {
      assert_eq!(travels(&source("s", url, &[])), goes, "{url}");
    }
    let mut local = source("s", "https://api.test/v1", &[]);
    local.preset = "ollama".into();
    assert!(!travels(&local));
    let mut brought = source("s", "https://api.test/v1", &[]);
    brought.extra.insert(SYNCED_FROM_FIELD.into(), json!({ "machine": "m" }));
    assert!(!travels(&brought), "a source brought here is not carried onwards");
  }

  /// The host's own source under the same id wins and its key is never touched; another machine's sources and the
  /// host's other entries are never touched either
  #[tokio::test]
  async fn a_machine_replaces_and_takes_back_only_what_it_brought() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join(PROVIDERS_FILE), json!({ "approval": "full", "webSearchVia": "extra", "providers": [
      { "id": "gw", "format": "openai-chat", "baseUrl": "https://gw.test/v1", "models": [{ "id": "old" }] },
      { "id": "own", "baseUrl": "https://own.test/v1" },
    ] }).to_string()).unwrap();
    std::fs::write(root.join("secrets.json"), json!({ "acpira.provider.gw": "host-key", "acpira.provider.own": "own-key" }).to_string()).unwrap();

    let mut gw = source("gw", "https://gw.test/v1", &["old", "new"]);
    gw.format = "anthropic".into();
    let extra = source("extra", "https://extra.test/v1", &["x"]);
    assert!(merge(root, "mac", "Mac", carried(&[(gw.clone(), Some("mac-key")), (extra.clone(), None)])).await.unwrap());
    let file = read(root, PROVIDERS_FILE);
    let ids: Vec<&str> = file["providers"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["gw", "own", "extra"]);
    let gw_now = &file["providers"][0];
    assert_eq!((gw_now["format"].as_str(), gw_now["models"].as_array().unwrap().len()), (Some("openai-chat"), 1), "the host's own gw stays");
    assert!(gw_now.get("syncedFrom").is_none());
    assert_eq!(file["providers"][2]["syncedFrom"]["name"], "Mac");
    assert_eq!(file["approval"], "full", "the host's own settings stay");
    let secrets = read(root, "secrets.json");
    assert_eq!(secrets, json!({ "acpira.provider.gw": "host-key", "acpira.provider.own": "own-key" }));

    // The same again changes nothing
    assert!(!merge(root, "mac", "Mac", carried(&[(gw.clone(), Some("mac-key")), (extra.clone(), None)])).await.unwrap());

    // Another machine takes over `extra`, then the first drops everything: only its own copies could go, and it has none
    assert!(merge(root, "book", "Book", carried(&[(extra.clone(), Some("book-key"))])).await.unwrap());
    assert!(!merge(root, "mac", "Mac", Carried::default()).await.unwrap());
    let file = read(root, PROVIDERS_FILE);
    assert_eq!(file["providers"][2]["syncedFrom"]["machine"], "book");

    // The second machine drops everything too: the host is as it was
    assert!(merge(root, "book", "Book", Carried::default()).await.unwrap());
    let file = read(root, PROVIDERS_FILE);
    let ids: Vec<&str> = file["providers"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["gw", "own"]);
    assert!(file.get("webSearchVia").is_none(), "search through a source that went away falls back");
    assert_eq!(read(root, "secrets.json"), json!({ "acpira.provider.gw": "host-key", "acpira.provider.own": "own-key" }));
  }

  /// An older helper replaced the host's own source and kept it inside the entry: the next push gives it back
  #[tokio::test]
  async fn a_stood_in_source_comes_back_on_the_next_push() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let own = json!({ "id": "gw", "format": "openai-chat", "baseUrl": "https://gw.test/v1", "models": [{ "id": "m" }] });
    std::fs::write(root.join(PROVIDERS_FILE), json!({ "providers": [
      { "id": "gw", "format": "anthropic", "baseUrl": "https://gw.test/v1", "syncedFrom": { "machine": "mac", "name": "Mac", "kept": own } },
    ] }).to_string()).unwrap();
    std::fs::write(root.join("secrets.json"), json!({ "acpira.provider.gw": "mac-key", "acpira.provider.gw/kept": "host-key" }).to_string()).unwrap();

    let mut gw = source("gw", "https://gw.test/v1", &["m"]);
    gw.format = "anthropic".into();
    assert!(merge(root, "mac", "Mac", carried(&[(gw, Some("mac-key"))])).await.unwrap());
    let file = read(root, PROVIDERS_FILE);
    assert_eq!(file["providers"].as_array().unwrap().len(), 1);
    assert_eq!(file["providers"][0]["format"], "openai-chat");
    assert!(file["providers"][0].get("syncedFrom").is_none());
    assert_eq!(read(root, "secrets.json"), json!({ "acpira.provider.gw": "host-key" }));
  }

  #[tokio::test]
  async fn a_broken_file_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(PROVIDERS_FILE), "{ not json").unwrap();
    assert!(merge(dir.path(), "mac", "Mac", carried(&[(source("gw", "https://gw.test/v1", &[]), Some("k"))])).await.is_err());
    assert_eq!(std::fs::read_to_string(dir.path().join(PROVIDERS_FILE)).unwrap(), "{ not json");
    assert!(!dir.path().join("secrets.json").exists());
  }

  /// A machine's settings kept apart: what it merged before goes back first, the host's own source under the same id
  /// and its key stay as they are, only the keys the agent reads land, and the same again changes nothing
  #[tokio::test]
  async fn a_machines_settings_stay_apart_from_the_hosts_own() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join(PROVIDERS_FILE), json!({ "approval": "full", "providers": [
      { "id": "gw", "format": "openai-chat", "baseUrl": "https://gw.test/v1", "models": [{ "id": "host" }] },
    ] }).to_string()).unwrap();
    std::fs::write(root.join("secrets.json"), json!({ "acpira.provider.gw": "host-key" }).to_string()).unwrap();
    // An earlier push merged `extra` into the host's own file
    let extra = source("extra", "https://extra.test/v1", &["x"]);
    assert!(merge(root, "mac", "Mac", carried(&[(extra.clone(), Some("extra-key"))])).await.unwrap());

    let mut file: ProvidersFile = serde_json::from_value(json!({ "approval": "ask", "fastModel": "gw/mine" })).unwrap();
    file.providers = vec![source("gw", "https://gw.test/v1", &["mine"]), extra];
    let keys = [("acpira.provider.gw", "mac-key"), ("acpira.web.jina", "j"), ("acpira.account.x", "not this")];
    let set = ClientSet { file, keys: keys.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect() };
    assert!(put_set(root, "mac", "Mac", Some(set.clone())).await.unwrap());
    let host = read(root, PROVIDERS_FILE);
    let ids: Vec<&str> = host["providers"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["gw"], "what the machine merged before went back");
    assert_eq!((host["providers"][0]["models"][0]["id"].as_str(), host["approval"].as_str()), (Some("host"), Some("full")));
    assert_eq!(read(root, "secrets.json"), json!({ "acpira.provider.gw": "host-key" }));

    let kept = client_settings_dir(root, "mac").unwrap();
    let file = read(&kept, PROVIDERS_FILE);
    let ids: Vec<&str> = file["providers"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["gw", "extra"]);
    assert_eq!((file["approval"].as_str(), file["fastModel"].as_str()), (Some("ask"), Some("gw/mine")));
    assert_eq!(read(&kept, "secrets.json"), json!({ "acpira.provider.gw": "mac-key", "acpira.web.jina": "j" }));
    assert!(!put_set(root, "mac", "Mac", Some(set)).await.unwrap(), "the same again changes nothing");

    // The host's setting off on that machine: the folder goes, the host's own file stays as it is
    assert!(put_set(root, "mac", "Mac", None).await.unwrap());
    assert!(!kept.exists());
    assert_eq!(read(root, PROVIDERS_FILE)["providers"].as_array().unwrap().len(), 1);
    assert!(!put_set(root, "mac", "Mac", None).await.unwrap());
  }

  fn body_of(bytes: Vec<u8>) -> Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>> {
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(Some(bytes)).unwrap();
    tx.send(None).unwrap();
    Some(rx)
  }

  /// A machine back to merging (an older build of it) leaves nothing apart behind, and a machine id that is not a plain
  /// name never becomes a folder
  #[tokio::test]
  async fn merging_again_drops_what_was_kept_apart() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let a = json!({ "home": root, "machine": "mac", "name": "Mac" });
    let set = serde_json::to_vec(&Some(ClientSet::default())).unwrap();
    assert_eq!(serve_set(&a, body_of(set.clone())).await.unwrap()["changed"], true);
    let kept = client_settings_dir(root, "mac").unwrap();
    assert!(kept.join(PROVIDERS_FILE).exists());

    let merged = serde_json::to_vec(&carried(&[(source("gw", "https://gw.test/v1", &["m"]), None)])).unwrap();
    assert_eq!(serve(&a, body_of(merged)).await.unwrap()["changed"], true);
    assert!(!kept.exists());
    assert_eq!(read(root, PROVIDERS_FILE)["providers"][0]["syncedFrom"]["machine"], "mac");

    let odd = json!({ "home": root, "machine": "../mac" });
    assert_eq!(serve_set(&odd, body_of(set)).await.unwrap_err().code, "invalid");
  }
}
