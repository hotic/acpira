//! The agent's view of the host-written configuration: `providers.json` and the API keys in `secrets.json`, both under
//! the data root given as `--home`. Both are re-read when their modification time changes, so a settings edit applies
//! from the next turn without a handshake; keys never travel over ACP or into logs

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use acpira_shared::providers::{PROVIDERS_FILE, Provider, ProviderModel, ProvidersFile, provider_secret_key, web_secret_key};

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

  /// The approval level set on the settings page; absent or unknown asks
  pub fn approval(&self) -> Approval {
    self.providers.approval.as_deref().and_then(Approval::parse).unwrap_or_default()
  }

  /// Chunk compaction is picked on the settings page (`chunk.rs`); otherwise the summary
  pub fn chunked(&self) -> bool {
    self.providers.compaction.as_deref() == Some(crate::chunk::STRATEGY)
  }

  /// The Raw mode's system prompt from the settings page; empty when none is set
  pub fn raw_system_prompt(&self) -> &str {
    self.providers.raw_system_prompt.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("")
  }

  /// The search setup picked on the settings page. The default backend uses no key; a keyed pick brings every keyed
  /// backend that is set up, the picked one first. DeepSeek goes through `web_search_via`'s source (its address and
  /// key) when one is named, else DeepSeek's own API with its search key
  pub fn web_search(&self) -> crate::tools::web::SearchSetup {
    use crate::tools::web::{DeepSeekSearch, SearchSetup, tools_search_url};
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

  /// The fast model picked on the settings page, while it is still usable
  pub fn fast_model(&self) -> Option<(&Provider, &ProviderModel)> {
    self.providers.fast_model.as_deref().and_then(|p| self.find(p))
  }

  /// The first usable model, the default of a new session
  pub fn default_pick(&self) -> Option<String> {
    self.providers.usable().next().map(|(p, m)| acpira_shared::providers::model_pick(&p.id, &m.id))
  }
}

type Stamp = Option<(SystemTime, u64)>;

pub struct ConfigCache {
  home: PathBuf,
  state: parking_lot::Mutex<(Stamp, Stamp, Arc<Config>)>,
}

fn stamp(path: &Path) -> Stamp {
  let meta = std::fs::metadata(path).ok()?;
  Some((meta.modified().ok()?, meta.len()))
}

impl ConfigCache {
  pub fn new(home: PathBuf) -> Self {
    ConfigCache { home, state: parking_lot::Mutex::new((None, None, Arc::new(Config::default()))) }
  }

  pub fn home(&self) -> &Path {
    &self.home
  }

  /// The current configuration, re-read when either file changed since the last call
  pub fn get(&self) -> Arc<Config> {
    let providers_path = self.home.join(PROVIDERS_FILE);
    let secrets_path = self.home.join(SECRETS_FILE);
    let (p, s) = (stamp(&providers_path), stamp(&secrets_path));
    let mut st = self.state.lock();
    // The initial state reads as "both absent", which is also the right answer when they are
    if st.0 == p && st.1 == s {
      return st.2.clone();
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
    // secrets.json is the host's string table; only this agent's keys are kept in memory
    if let Ok(text) = std::fs::read_to_string(&secrets_path)
      && let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(&text)
    {
      config.secrets = m
        .into_iter()
        .filter(|(k, _)| k.starts_with(acpira_shared::providers::PROVIDER_SECRET_PREFIX) || k.starts_with(acpira_shared::providers::WEB_SECRET_PREFIX))
        .filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_owned())))
        .collect();
    }
    let config = Arc::new(config);
    *st = (p, s, config.clone());
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
}
