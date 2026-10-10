//! The built-in agent's model sources as the settings page edits them: `<root>/providers.json` (contract in
//! `acpira_shared::providers`) plus each source's API key in the vault. Every edit re-reads the file under its lock, so
//! another window's change made meanwhile survives; a file that does not parse is reported and never overwritten. The
//! page's network questions (list a source's models, check it, test a model, find local servers) are answered here by
//! the agent crate's discovery code, so paths, headers and field names exist once

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use tokio::fs;

use acpira_agent::llm::discover;
use acpira_shared::providers::{
  LocalSource, PROVIDERS_FILE, PROVIDERS_VERSION, ProbeOutcome, Provider, ProviderAction, ProviderProbe, ProviderView, ProvidersFile,
  ProvidersView, provider_secret_key,
};

use crate::accounts::account_store::FileVault;
use crate::store::file_lock::{with_file_lock, write_atomic};

pub struct ProviderStore {
  file: PathBuf,
  vault: Arc<FileVault>,
}

impl ProviderStore {
  pub fn new(root: &Path, vault: Arc<FileVault>) -> Self {
    ProviderStore { file: root.join(PROVIDERS_FILE), vault }
  }

  /// An absent file reads as no sources; Err carries why an existing one cannot be used
  async fn read(&self) -> Result<ProvidersFile, String> {
    match fs::read_to_string(&self.file).await {
      Ok(text) => ProvidersFile::parse(&text).map_err(|e| format!("{} cannot be read: {e}", self.file.display())),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ProvidersFile::default()),
      Err(e) => Err(format!("{} cannot be read: {e}", self.file.display())),
    }
  }

  pub async fn view(&self) -> ProvidersView {
    let file = match self.read().await {
      Ok(f) => f,
      Err(error) => return ProvidersView { providers: vec![], error: Some(error), ..tables() },
    };
    let mut providers = Vec::with_capacity(file.providers.len());
    for provider in file.providers {
      let has_key = self.vault.get(&provider_secret_key(&provider.id)).await.ok().flatten().is_some_and(|k| !k.is_empty());
      providers.push(ProviderView { provider, has_key });
    }
    ProvidersView { providers, error: None, ..tables() }
  }

  /// The key a probe uses: one typed into the form, else the stored key of the source with that id
  async fn key_for(&self, provider: &Provider, typed: Option<String>) -> Option<String> {
    if let Some(k) = typed.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()) {
      return Some(k);
    }
    if provider.id.is_empty() {
      return None;
    }
    self.vault.get(&provider_secret_key(&provider.id)).await.ok().flatten().filter(|k| !k.is_empty())
  }

  /// Answer one of the settings page's network questions; nothing is written
  pub async fn probe(&self, probe: ProviderProbe) -> ProbeOutcome {
    let catalog = crate::model_catalog::current();
    let failed = |e: String| ProbeOutcome::Failed { error: e };
    let blocking = |f: Box<dyn FnOnce() -> ProbeOutcome + Send>| async move { tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| failed(e.to_string())) };
    match probe {
      ProviderProbe::Models { provider, key } => {
        let key = self.key_for(&provider, key).await;
        blocking(Box::new(move || match discover::discover(&http(), &normalized(provider), key.as_deref(), &catalog) {
          Ok(models) => ProbeOutcome::Models { models },
          Err(e) => failed(e.to_string()),
        }))
        .await
      }
      ProviderProbe::Check { provider, key } => {
        let key = self.key_for(&provider, key).await;
        blocking(Box::new(move || match discover::check(&http(), &normalized(provider), key.as_deref()) {
          Ok(count) => ProbeOutcome::Check { count },
          Err(e) => failed(e.to_string()),
        }))
        .await
      }
      ProviderProbe::Test { provider, model, key } => {
        let key = self.key_for(&provider, key).await;
        match discover::test(http(), &normalized(provider), &model, key.as_deref()).await {
          Ok(t) => ProbeOutcome::Test { ms: t.ms, text: t.text },
          Err(e) => failed(e.to_string()),
        }
      }
      ProviderProbe::Local => {
        blocking(Box::new(move || ProbeOutcome::Local {
          servers: discover::probe_local(&catalog)
            .into_iter()
            .map(|s| LocalSource { preset: s.preset.into(), name: s.name.into(), base_url: s.base_url, models: s.models })
            .collect(),
        }))
        .await
      }
    }
  }

  pub async fn apply(&self, action: ProviderAction) -> Result<()> {
    match action {
      ProviderAction::Save { provider, key } => {
        let id = self.edit(|file| save(file, provider)).await?;
        // The key follows the entry: a turn that already sees the new source finds its key a moment later at worst
        match key.as_deref().map(str::trim) {
          Some("") => self.vault.delete(&provider_secret_key(&id)).await?,
          Some(k) => self.vault.store(&provider_secret_key(&id), k).await?,
          None => {}
        }
        Ok(())
      }
      ProviderAction::Delete { id } => {
        self
          .edit(|file| {
            file.providers.retain(|p| p.id != id);
            Ok(())
          })
          .await?;
        self.vault.delete(&provider_secret_key(&id)).await
      }
    }
  }

  /// Read → change → write under the file's lock
  async fn edit<T>(&self, f: impl FnOnce(&mut ProvidersFile) -> Result<T>) -> Result<T> {
    with_file_lock(&self.file, || async {
      let mut file = self.read().await.map_err(|e| anyhow!(e))?;
      let out = f(&mut file)?;
      file.version = file.version.max(PROVIDERS_VERSION);
      if let Some(dir) = self.file.parent() {
        fs::create_dir_all(dir).await?;
      }
      write_atomic(&self.file, serde_json::to_string_pretty(&file)?.as_bytes(), None).await?;
      Ok(out)
    })
    .await
  }
}

/// The agent's tables the page builds its forms from
fn tables() -> ProvidersView {
  ProvidersView {
    presets: acpira_agent::llm::presets::presets(),
    families: acpira_agent::llm::family::names().into_iter().map(str::to_owned).collect(),
    ..Default::default()
  }
}

/// The engine's HTTP client for discovery: its proxy, a bounded wait (a test call answers in seconds)
fn http() -> ureq::Agent {
  crate::net_proxy::ureq_config(
    ureq::Agent::config_builder()
      .http_status_as_error(false)
      .timeout_connect(Some(std::time::Duration::from_secs(15)))
      .timeout_global(Some(std::time::Duration::from_secs(90)))
      .user_agent(format!("acpira/{}", env!("CARGO_PKG_VERSION"))),
  )
  .build()
  .into()
}

/// A form draft as the file would hold it (trimmed URL)
fn normalized(mut p: Provider) -> Provider {
  p.base_url = p.base_url.trim().trim_end_matches('/').to_owned();
  p
}

/// Validate and insert / replace one entry; returns its id
fn save(file: &mut ProvidersFile, mut p: Provider) -> Result<String> {
  // The page sends back what it was shown, `hasKey` included; that is not part of the file
  p.extra.remove("hasKey");
  p.id = p.id.trim().to_owned();
  p.name = p.name.trim().to_owned();
  p.base_url = p.base_url.trim().trim_end_matches('/').to_owned();
  if p.id.contains('/') {
    bail!("A source id cannot contain \"/\"");
  }
  if !(p.base_url.starts_with("http://") || p.base_url.starts_with("https://")) {
    bail!("The base URL must start with http:// or https://");
  }
  let mut seen = std::collections::HashSet::new();
  p.models.retain_mut(|m| {
    m.id = m.id.trim().to_owned();
    !m.id.is_empty() && seen.insert(m.id.clone())
  });
  if p.id.is_empty() {
    p.id = unique_id(file, &p);
  }
  let id = p.id.clone();
  match file.providers.iter_mut().find(|e| e.id == id) {
    Some(slot) => *slot = p,
    None => file.providers.push(p),
  }
  Ok(id)
}

/// A readable id from the name (else the URL's host), made unique among the file's sources
fn unique_id(file: &ProvidersFile, p: &Provider) -> String {
  let host = p.base_url.split("://").nth(1).and_then(|r| r.split(['/', ':']).next()).unwrap_or("");
  let from = if p.name.is_empty() { host } else { &p.name };
  let mut base = String::new();
  for c in from.chars().flat_map(char::to_lowercase) {
    if c.is_ascii_alphanumeric() {
      base.push(c);
    } else if !base.ends_with('-') {
      base.push('-');
    }
  }
  let base = base.trim_matches('-');
  let base = if base.is_empty() { "source" } else { base };
  let taken = |id: &str| file.providers.iter().any(|e| e.id == id);
  if !taken(base) {
    return base.to_owned();
  }
  (2..).map(|n| format!("{base}-{n}")).find(|id| !taken(id)).expect("an unused suffix exists")
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::providers::ProviderModel;
  use serde_json::json;

  fn store(dir: &Path) -> ProviderStore {
    ProviderStore::new(dir, Arc::new(FileVault::new(dir.join("secrets.json"), Arc::new(|_| {}))))
  }

  fn draft(name: &str, url: &str, models: &[&str]) -> Provider {
    let mut p: Provider = serde_json::from_value(json!({ "id": "", "name": name, "baseUrl": url })).unwrap();
    p.models = models.iter().map(|m| ProviderModel::new(*m)).collect();
    p
  }

  #[tokio::test]
  async fn save_makes_an_id_stores_the_key_apart_and_delete_removes_both() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let save = |p, key: Option<&str>| s.apply(ProviderAction::Save { provider: p, key: key.map(str::to_owned) });
    save(draft("DeepSeek", "https://api.deepseek.com/v1/", &[" deepseek-chat ", "", "deepseek-chat"]), Some("sk-1")).await.unwrap();
    save(draft("DeepSeek", "https://x.test", &["m"]), None).await.unwrap();
    let v = s.view().await;
    let ids: Vec<_> = v.providers.iter().map(|p| (p.provider.id.as_str(), p.has_key)).collect();
    assert_eq!(ids, [("deepseek", true), ("deepseek-2", false)]);
    let first = &v.providers[0].provider;
    assert_eq!(first.base_url, "https://api.deepseek.com/v1");
    assert_eq!(first.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["deepseek-chat"]);
    let raw = std::fs::read_to_string(dir.path().join(PROVIDERS_FILE)).unwrap();
    assert!(!raw.contains("sk-1") && !raw.contains("hasKey"), "{raw}");

    // Saving what the page was shown (hasKey included) replaces the entry and keeps the key
    let mut shown = serde_json::to_value(&v.providers[0]).unwrap();
    shown["name"] = json!("DS");
    save(serde_json::from_value(shown).unwrap(), None).await.unwrap();
    let v = s.view().await;
    assert_eq!((v.providers[0].provider.name.as_str(), v.providers[0].has_key, v.providers.len()), ("DS", true, 2));

    s.apply(ProviderAction::Delete { id: "deepseek".into() }).await.unwrap();
    assert_eq!(s.view().await.providers.len(), 1);
    let secrets = std::fs::read_to_string(dir.path().join("secrets.json")).unwrap();
    assert!(!secrets.contains("sk-1"));
  }

  #[tokio::test]
  async fn a_bad_url_is_refused_and_a_broken_file_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let e = s.apply(ProviderAction::Save { provider: draft("x", "api.test", &[]), key: None }).await.unwrap_err();
    assert!(e.to_string().contains("http"));
    std::fs::write(dir.path().join(PROVIDERS_FILE), "{ not json").unwrap();
    assert!(s.view().await.error.is_some());
    assert!(s.apply(ProviderAction::Delete { id: "x".into() }).await.is_err());
    assert_eq!(std::fs::read_to_string(dir.path().join(PROVIDERS_FILE)).unwrap(), "{ not json");
  }
}
