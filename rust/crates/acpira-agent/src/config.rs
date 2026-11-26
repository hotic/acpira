//! The agent's view of the host-written configuration: `providers.json` and the API keys in `secrets.json`, both under
//! the data root given as `--home`, or for a session another machine drives over SSH the settings that machine brought
//! (`ConfigCache::of_client`). Both are re-read when their modification time changes, so a settings edit applies from
//! the next turn without a handshake; keys never travel over ACP or into logs

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use acpira_shared::model_catalog::Catalog;
use acpira_shared::providers::{
  KEY_ENV_FIELD, PROVIDERS_FILE, Provider, ProviderModel, ProvidersFile, client_settings_dir, plugin_providers_path, provider_secret_key, web_secret_key,
};

use crate::permission::Approval;

pub const SECRETS_FILE: &str = "secrets.json";

#[derive(Debug, Default)]
pub struct Config {
  pub providers: ProvidersFile,
  /// Why providers.json could not be used, shown when a prompt needs a model
  pub error: Option<String>,
  secrets: HashMap<String, String>,
}

impl Config {
  pub fn api_key(&self, provider: &str) -> Option<&str> {
    self.secrets.get(&provider_secret_key(provider)).map(String::as_str).filter(|k| !k.is_empty())
  }

  pub fn find(&self, pick: &str) -> Option<(&Provider, &ProviderModel)> {
    self.providers.find(pick)
  }

  /// The approval level a new session starts on (an older record without one too); absent or unknown asks
  pub fn approval(&self) -> Approval {
    self.providers.approval.as_deref().and_then(Approval::parse).unwrap_or_default()
  }

  /// Raw mode is offered in the mode picker (a switch on the settings page)
  pub fn raw_mode(&self) -> bool {
    self.providers.raw_mode == Some(true)
  }

  /// Where sessions take a scratch folder (`scratch.rs`); None leaves them the system's temp folder
  pub fn scratch_place(&self) -> Option<crate::defs::Place> {
    match self.providers.scratch_place_of() {
      "off" => None,
      "project" => Some(crate::defs::Place::Project),
      _ => Some(crate::defs::Place::Data),
    }
  }

  /// Chunk compaction is picked on the settings page (`chunk.rs`); otherwise the summary
  pub fn chunked(&self) -> bool {
    self.providers.compaction.as_deref() == Some(crate::defs::CHUNK_STRATEGY)
  }

  /// The Raw mode's system prompt from the settings page; empty when none is set
  pub fn raw_system_prompt(&self) -> &str {
    self.providers.raw_system_prompt.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("")
  }

  /// The search setup picked on the settings page. The default backend uses no key; a keyed pick brings every keyed
  /// backend that is set up, the picked one first. DeepSeek goes through `web_search_via`'s source (its address and
  /// key) when one is named, else DeepSeek's own API with its search key
  pub fn web_search(&self) -> crate::defs::SearchSetup {
    use crate::defs::{DeepSeekSearch, SearchSetup, tools_search_url};
    let pick = self.providers.web_search.as_deref();
    if !matches!(pick, Some("jina" | "deepseek")) {
      return SearchSetup::default();
    }
    let key = |backend: &str| self.secrets.get(&web_secret_key(backend)).map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
    let deepseek = match self.providers.web_search_via.as_deref() {
      // A named source that is gone or has no key turns DeepSeek off rather than sending its queries elsewhere
      Some(via) => self.providers.providers.iter().find(|p| p.id == via).and_then(|p| {
        self.api_key(&p.id).map(|k| DeepSeekSearch { url: tools_search_url(&p.base_url), key: k.trim().to_owned() })
      }),
      None => key("deepseek").map(DeepSeekSearch::official),
    };
    SearchSetup { jina_key: key("jina"), deepseek, deepseek_first: pick == Some("deepseek") }
  }

  /// The fast model in effect (`ProvidersFile::fast_pick`): the settings page's pick while usable, none when turned
  /// off, else the cheapest model a source with a key offers
  pub fn fast_model(&self, catalog: &Catalog) -> Option<(&Provider, &ProviderModel)> {
    self.providers.fast_pick(catalog, |p| self.api_key(&p.id).is_some())
  }

  /// The first usable model, the default of a new session
  pub fn default_pick(&self) -> Option<String> {
    self.providers.usable().next().map(|(p, m)| acpira_shared::providers::model_pick(&p.id, &m.id))
  }
}

/// A configured model with the limits the configuration leaves open filled from the catalogue (marked estimated)
pub fn limits(config: &Config, catalog: &Catalog, pick: &str) -> Option<ProviderModel> {
  let (_, m) = config.find(pick)?;
  let mut model = m.clone();
  crate::catalog::complete(&mut model, catalog);
  Some(model)
}

/// The plugins' sources (`plugins/providers.json`, written by the engine) after the user's own: an id the user's file
/// has wins. A source's key is the vault's under its id (`own_keys`: the data root's vault, read when a client's
/// settings stand in, since the plugins belong to this engine), else the variable its `keyEnv` names. A broken file is
/// skipped, never in the way of the user's sources
fn merge_plugin_sources(config: &mut Config, path: &Path, own_keys: Option<&HashMap<String, String>>) {
  let Ok(text) = std::fs::read_to_string(path) else { return };
  let Ok(file) = ProvidersFile::parse(&text) else { return };
  for p in file.providers {
    if config.providers.providers.iter().any(|x| x.id == p.id) {
      continue;
    }
    let secret = provider_secret_key(&p.id);
    if !config.secrets.get(&secret).is_some_and(|k| !k.is_empty())
      && let Some(key) = own_keys
        .and_then(|k| k.get(&secret).filter(|k| !k.is_empty()).cloned())
        .or_else(|| p.extra.get(KEY_ENV_FIELD).and_then(serde_json::Value::as_str).and_then(|v| std::env::var(v).ok()).filter(|k| !k.is_empty()))
    {
      config.secrets.insert(secret, key);
    }
    config.providers.providers.push(p);
  }
}

/// This agent's keys in a vault file (`secrets.json` is the host's string table): provider and search keys only
fn read_keys(path: &Path) -> HashMap<String, String> {
  let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
  let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(&text) else { return HashMap::new() };
  m.into_iter()
    .filter(|(k, _)| k.starts_with(acpira_shared::providers::PROVIDER_SECRET_PREFIX) || k.starts_with(acpira_shared::providers::WEB_SECRET_PREFIX))
    .filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_owned())))
    .collect()
}

type Stamp = Option<(SystemTime, u64)>;

pub struct ConfigCache {
  home: PathBuf,
  /// The machine whose settings this reads, and their folder (`client_settings_dir`): while it holds a settings file it
  /// stands in for the data root's own files. The plugins' sources stay the data root's: they belong to this engine
  client: Option<(String, PathBuf)>,
  state: parking_lot::Mutex<([Stamp; 5], Arc<Config>)>,
}

fn stamp(path: &Path) -> Stamp {
  let meta = std::fs::metadata(path).ok()?;
  Some((meta.modified().ok()?, meta.len()))
}

impl ConfigCache {
  pub fn new(home: PathBuf) -> Self {
    ConfigCache { home, client: None, state: parking_lot::Mutex::new((Default::default(), Arc::new(Config::default()))) }
  }

  /// The settings `machine` brought for the sessions it drives here over SSH, else the data root's own while it has
  /// brought none (its setting off there, or nothing arrived yet). None for an id that names no folder
  pub fn of_client(home: PathBuf, machine: &str) -> Option<Self> {
    let dir = client_settings_dir(&home, machine)?;
    Some(ConfigCache { home, client: Some((machine.to_owned(), dir)), state: parking_lot::Mutex::new((Default::default(), Arc::new(Config::default()))) })
  }

  /// The machine whose settings this reads
  pub fn client(&self) -> Option<&str> {
    self.client.as_ref().map(|(m, _)| m.as_str())
  }

  pub fn home(&self) -> &Path {
    &self.home
  }

  /// The current configuration, re-read when any of its files changed since the last call
  pub fn get(&self) -> Arc<Config> {
    let set = self.client.as_ref().map(|(_, dir)| dir.as_path());
    let (set_file, set_keys) = set.map_or((None, None), |d| (stamp(&d.join(PROVIDERS_FILE)), stamp(&d.join(SECRETS_FILE))));
    // A client's folder from the moment its settings file lands until it goes, the data root's own otherwise
    let base = match set {
      Some(dir) if set_file.is_some() => dir,
      _ => self.home.as_path(),
    };
    let providers_path = base.join(PROVIDERS_FILE);
    let secrets_path = base.join(SECRETS_FILE);
    let plugins_path = plugin_providers_path(&self.home);
    let stamps = [set_file, set_keys, stamp(&self.home.join(PROVIDERS_FILE)), stamp(&self.home.join(SECRETS_FILE)), stamp(&plugins_path)];
    let mut st = self.state.lock();
    // The initial state reads as "all absent", which is also the right answer when they are
    if st.0 == stamps {
      return st.1.clone();
    }
    let mut config = Config::default();
    match std::fs::read_to_string(&providers_path) {
      Ok(text) => match ProvidersFile::parse(&text) {
        Ok(f) => config.providers = f,
        Err(e) => config.error = Some(format!("{}: {e}", providers_path.display())),
      },
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
      Err(e) => config.error = Some(format!("{}: {e}", providers_path.display())),
    }
    config.secrets = read_keys(&secrets_path);
    let own_keys = (base != self.home.as_path()).then(|| read_keys(&self.home.join(SECRETS_FILE)));
    merge_plugin_sources(&mut config, &plugins_path, own_keys.as_ref());
    let config = Arc::new(config);
    *st = (stamps, config.clone());
    config
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn reloads_when_a_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ConfigCache::new(dir.path().to_owned());
    assert!(cache.get().default_pick().is_none());
    std::fs::write(
      dir.path().join(PROVIDERS_FILE),
      json!({ "version": 1, "providers": [{ "id": "ds", "baseUrl": "http://x", "models": [{ "id": "m" }] }] }).to_string(),
    )
    .unwrap();
    std::fs::write(dir.path().join(SECRETS_FILE), json!({ "acpira.provider.ds": "sk-1", "acpira.account.a": "other", "acpira.web.jina": "j-1" }).to_string()).unwrap();
    let c = cache.get();
    assert_eq!(c.default_pick().as_deref(), Some("ds/m"));
    assert_eq!(c.api_key("ds"), Some("sk-1"));
    assert!(c.secrets.len() == 2, "only provider and search keys are kept");
    // The key is used only while Jina is the picked backend
    assert_eq!(c.web_search().jina_key, None);
    std::fs::write(
      dir.path().join(PROVIDERS_FILE),
      json!({ "version": 1, "webSearch": "jina", "providers": [{ "id": "ds", "baseUrl": "http://x", "models": [{ "id": "m" }] }] }).to_string(),
    )
    .unwrap();
    assert_eq!(cache.get().web_search().jina_key.as_deref(), Some("j-1"));
    assert_eq!(cache.get().web_search().deepseek, None);
    // DeepSeek through the `ds` source: its address and key, ahead of the stored Jina key
    std::fs::write(
      dir.path().join(PROVIDERS_FILE),
      json!({ "version": 1, "webSearch": "deepseek", "webSearchVia": "ds", "providers": [{ "id": "ds", "baseUrl": "http://x/v1", "models": [{ "id": "m" }] }] }).to_string(),
    )
    .unwrap();
    let setup = cache.get().web_search();
    assert_eq!((setup.deepseek.map(|d| (d.url, d.key)), setup.jina_key.as_deref(), setup.deepseek_first), (Some(("http://x/v1/tools/search".into(), "sk-1".into())), Some("j-1"), true));
    // A named source that is gone turns DeepSeek off instead of falling back to its own API
    std::fs::write(
      dir.path().join(PROVIDERS_FILE),
      json!({ "version": 1, "webSearch": "deepseek", "webSearchVia": "gone", "providers": [] }).to_string(),
    )
    .unwrap();
    assert_eq!(cache.get().web_search().deepseek, None);
    std::fs::write(dir.path().join(PROVIDERS_FILE), "{ broken").unwrap();
    assert!(cache.get().error.is_some());
  }

  #[test]
  fn plugin_sources_follow_the_users_and_take_their_key_from_the_environment() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ConfigCache::new(dir.path().to_owned());
    std::fs::write(dir.path().join(PROVIDERS_FILE), json!({ "version": 1, "providers": [{ "id": "ds", "baseUrl": "http://x", "models": [{ "id": "m" }] }] }).to_string()).unwrap();
    let plugins = plugin_providers_path(dir.path());
    std::fs::create_dir_all(plugins.parent().unwrap()).unwrap();
    // PATH stands in for a key variable: always set, and no test has to change the process environment
    let sources = json!({ "version": 1, "providers": [
      { "id": "ds", "baseUrl": "http://plugin", "models": [{ "id": "other" }], "plugin": "gw" },
      { "id": "gw", "format": "anthropic", "baseUrl": "http://gw", "keyEnv": "PATH", "models": [{ "id": "claude" }], "plugin": "gw" },
    ] });
    std::fs::write(&plugins, sources.to_string()).unwrap();
    let c = cache.get();
    assert_eq!(c.default_pick().as_deref(), Some("ds/m"));
    assert!(c.find("ds/other").is_none(), "the user's `ds` wins");
    assert!(c.find("gw/claude").is_some());
    assert_eq!(c.api_key("gw"), std::env::var("PATH").ok().as_deref());
    // The engine rewrites the file when a plugin leaves; a broken one is skipped
    std::fs::write(&plugins, "{ broken").unwrap();
    let c = cache.get();
    assert!(c.find("gw/claude").is_none() && c.error.is_none());
  }

  /// A client's settings stand in for the data root's own while its folder holds a settings file, the plugins' sources
  /// still joining them; without one the data root's own are read
  #[test]
  fn a_clients_settings_stand_in_while_they_are_there() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::write(home.join(PROVIDERS_FILE), json!({ "approval": "full", "providers": [{ "id": "own", "baseUrl": "http://own", "models": [{ "id": "m" }] }] }).to_string()).unwrap();
    std::fs::write(home.join(SECRETS_FILE), json!({ "acpira.provider.own": "own-key", "acpira.provider.plug": "plug-key" }).to_string()).unwrap();
    let plugins = plugin_providers_path(home);
    std::fs::create_dir_all(plugins.parent().unwrap()).unwrap();
    std::fs::write(&plugins, json!({ "providers": [{ "id": "plug", "baseUrl": "http://plug", "models": [{ "id": "p" }] }] }).to_string()).unwrap();
    assert!(ConfigCache::of_client(home.to_owned(), "../x").is_none());
    let cache = ConfigCache::of_client(home.to_owned(), "mac").unwrap();
    assert_eq!(cache.client(), Some("mac"));
    let c = cache.get();
    assert_eq!((c.default_pick().as_deref(), c.approval()), (Some("own/m"), Approval::Full), "nothing brought yet: the data root's own");

    let set = client_settings_dir(home, "mac").unwrap();
    std::fs::create_dir_all(&set).unwrap();
    std::fs::write(set.join(SECRETS_FILE), json!({ "acpira.provider.gw": "mac-key" }).to_string()).unwrap();
    std::fs::write(set.join(PROVIDERS_FILE), json!({ "providers": [{ "id": "gw", "baseUrl": "http://gw", "models": [{ "id": "a" }] }] }).to_string()).unwrap();
    let c = cache.get();
    assert_eq!(c.default_pick().as_deref(), Some("gw/a"));
    assert!(c.find("own/m").is_none(), "the data root's own sources are not this machine's");
    assert!(c.find("plug/p").is_some(), "the plugins' sources stay");
    assert_eq!((c.api_key("gw"), c.api_key("own"), c.approval()), (Some("mac-key"), None, Approval::Ask));
    assert_eq!(c.api_key("plug"), Some("plug-key"), "a plugin source keeps the key this data root holds for it");

    std::fs::remove_dir_all(&set).unwrap();
    assert_eq!(cache.get().default_pick().as_deref(), Some("own/m"), "gone again: the data root's own");
  }
}
