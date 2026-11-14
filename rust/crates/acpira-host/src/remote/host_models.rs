//! Model sources follow the user to their hosts (docs/dev/remote.md): a session an SSH host runs reads the host's own
//! `providers.json`, which no settings page on this machine edits. So this engine's sources and their keys go to every
//! connected host, when it connects and whenever they change here; the host's built-in agent picks the file up by itself.
//!
//! `merge` is the helper's side. A source it writes names the machine that brought it (`SYNCED_FROM_FIELD`), and only
//! that machine's entries are replaced or taken away again. The host's own source under the same id wins: a host is
//! often a machine the user also sits at, and its settings page edits that file. `ModelMirror` is this engine's side

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use acpira_shared::providers::{PROVIDERS_FILE, PROVIDERS_VERSION, Provider, ProvidersFile, SYNCED_FROM_FIELD, provider_secret_key};
use acpira_shared::remote::{ConnectionState, ConnectionView};

use super::connections::Connections;
use super::host_store::{body, call_with_body, this_machine};
use super::wire::WireError;
use crate::accounts::account_store::FileVault;
use crate::providers::ProviderStore;
use crate::store::file_lock::{with_file_lock, write_atomic};
use crate::store::transcript_store::LogFn;

/// The helper operation that takes a machine's sources
pub const PUT_OP: &str = "modelsPut";

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
pub fn kept_secret_key(id: &str) -> String {
  // A source id holds no `/`, so this never meets another source's key
  format!("{}/kept", provider_secret_key(id))
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

/// The helper's `modelsPut`: a machine's `Carried` as the body
pub(crate) async fn serve(a: &Value, stdin: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  let root = a.get("home").and_then(Value::as_str).map(std::path::PathBuf::from).unwrap_or_else(crate::store::data_dir::acpira_home);
  let machine = a.get("machine").and_then(Value::as_str).filter(|m| !m.is_empty()).ok_or_else(|| WireError::new("invalid", "no machine"))?;
  let name = a.get("name").and_then(Value::as_str).unwrap_or(machine);
  let carried: Carried = serde_json::from_slice(&body(stdin).await?).map_err(|e| WireError::new("invalid", format!("not model sources: {e}")))?;
  if carried.providers.iter().any(|p| p.id.is_empty() || p.id.contains('/')) {
    return Err(WireError::new("invalid", "a source without a usable id"));
  }
  let changed = merge(&root, machine, name, carried).await.map_err(|e| WireError::new("io", format!("{e:#}")))?;
  Ok(json!({ "changed": changed }))
}

#[derive(Default)]
struct State {
  /// Connected hosts, and whether each was last sent sources (false: sent none, its setting being off)
  up: HashMap<String, bool>,
  /// A change here is waiting to go
  settling: bool,
  sub: Option<u64>,
}

/// This engine's sources kept on every connected host
pub struct ModelMirror {
  connections: Arc<Connections>,
  providers: Arc<ProviderStore>,
  log: LogFn,
  state: parking_lot::Mutex<State>,
  /// One push at a time: a later read of the sources never reaches a host before an earlier one
  sending: tokio::sync::Mutex<()>,
  me: Weak<ModelMirror>,
}

impl ModelMirror {
  pub fn new(connections: Arc<Connections>, providers: Arc<ProviderStore>, log: LogFn) -> Arc<ModelMirror> {
    Arc::new_cyclic(|me| ModelMirror { connections, providers, log, state: Default::default(), sending: Default::default(), me: me.clone() })
  }

  /// Follow the sources' changes and the hosts' connections
  pub fn start(self: &Arc<Self>) {
    let weak = self.me.clone();
    self.providers.set_change_hook(Arc::new(move || {
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
      let hosts: Vec<(String, bool)> = {
        let mut st = m.state.lock();
        st.settling = false;
        st.up.iter().filter(|(_, on)| **on).map(|(a, on)| (a.clone(), *on)).collect()
      };
      for (alias, on) in hosts {
        m.push(&alias, on).await;
      }
    });
  }

  /// A host that came up, or whose setting changed, is sent the sources (none with the setting off, which takes back
  /// what an earlier push brought)
  fn on_connections(&self, views: &[ConnectionView]) {
    let todo: Vec<(String, bool)> = {
      let mut st = self.state.lock();
      let connected: HashMap<&str, bool> = views.iter().filter(|v| v.state == ConnectionState::Connected).map(|v| (v.alias.as_str(), !v.models_off)).collect();
      st.up.retain(|a, _| connected.contains_key(a.as_str()));
      let todo: Vec<(String, bool)> = connected.iter().filter(|(a, on)| st.up.get(**a) != Some(*on)).map(|(a, on)| ((*a).to_owned(), *on)).collect();
      st.up.extend(todo.iter().cloned());
      todo
    };
    for (alias, on) in todo {
      let weak = self.me.clone();
      tokio::spawn(async move {
        if let Some(m) = weak.upgrade() {
          m.push(&alias, on).await;
        }
      });
    }
  }

  async fn push(&self, alias: &str, on: bool) {
    let _one = self.sending.lock().await;
    let Some(client) = self.connections.client(alias) else { return };
    let carried = if on {
      match self.providers.carried().await {
        Ok(c) => c,
        // A file that cannot be read says nothing about what the host should hold
        Err(e) => return (self.log)(&format!("host models: {alias}: not sent: {e}")),
      }
    } else {
      Carried::default()
    };
    let (machine, name) = this_machine();
    let sent = async { call_with_body(&client, PUT_OP, json!({ "machine": machine, "name": name }), &serde_json::to_vec(&carried)?).await };
    match sent.await {
      Ok(reply) if reply.get("changed").and_then(Value::as_bool) == Some(true) => (self.log)(&format!("host models: {alias}: {} source(s) on the host", carried.providers.len())),
      Ok(_) => {}
      // A helper from before this existed answers `unknownOp`: its host keeps its own sources
      Err(e) => (self.log)(&format!("host models: {alias}: not sent: {e:#}")),
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
}
