//! Secret vault and account metadata (mirror of apps/vscode/src/accounts/AccountStore.ts). secrets.json is a mode-600 JSON
//! string table, not an encrypted store; both files are shared by every host and edited read → change → write under
//! their file lock

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::fs;

use acpira_shared::transcript::{AccountInfo, AccountQuota, AgentId};

use crate::store::file_lock::{with_file_lock, write_atomic};
use crate::store::transcript_store::LogFn;
use crate::util::{now_iso, random_uuid};

pub const SECRET_PREFIX: &str = "acpira.account.";

pub fn account_secret_key(id: &str) -> String {
  format!("{SECRET_PREFIX}{id}")
}

/// The `meta` entry naming how a credential signs in, when it is not the provider's usual login; `AUTH_API_KEY` marks a
/// pasted API key (the secret), which no sign-in renews
pub const META_AUTH: &str = "auth";
pub const AUTH_API_KEY: &str = "apiKey";

/// Persistent vault at secrets.json: every change re-reads the file under its lock, so a secret another host stored
/// meanwhile survives. A corrupt file is never overwritten in place: reads report it and the next store quarantines
/// it beside the file (`secrets.json.corrupt-<stamp>`) before writing a clean table
pub struct FileVault {
  file: PathBuf,
  log: LogFn,
  corrupt: parking_lot::Mutex<Option<String>>,
}

enum VaultRead {
  Ok(Map<String, Value>),
  Corrupt(String),
  Io(std::io::Error),
}

impl FileVault {
  pub fn new(file: PathBuf, log: LogFn) -> Self {
    FileVault { file, log, corrupt: parking_lot::Mutex::new(None) }
  }

  /// Why secrets.json could not be read on the last access, if it is corrupt
  pub fn error(&self) -> Option<String> {
    let why = self.corrupt.lock().clone()?;
    Some(format!("{} cannot be read ({why}); saving a new key will archive the corrupt file and recreate it", self.file.display()))
  }

  // An absent file reads as an empty table; a corrupt file records why and returns Corrupt
  async fn read(&self) -> VaultRead {
    let raw = match fs::read(&self.file).await {
      Ok(raw) => raw,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
        *self.corrupt.lock() = None;
        return VaultRead::Ok(Map::new());
      }
      Err(e) => return VaultRead::Io(e),
    };
    match serde_json::from_slice::<Value>(&raw) {
      Ok(Value::Object(m)) => {
        *self.corrupt.lock() = None;
        VaultRead::Ok(m.into_iter().filter(|(_, v)| v.is_string()).collect())
      }
      other => {
        let why = match other {
          Err(e) => e.to_string(),
          Ok(_) => "not an object".into(),
        };
        let mut corrupt = self.corrupt.lock();
        if corrupt.is_none() {
          (self.log)(&format!("secrets.json unreadable ({why})"));
        }
        *corrupt = Some(why.clone());
        VaultRead::Corrupt(why)
      }
    }
  }

  async fn quarantine_corrupt(&self, why: &str) -> Result<PathBuf> {
    let stamp = crate::util::now_ms();
    let mut name = self.file.as_os_str().to_owned();
    name.push(format!(".corrupt-{stamp}"));
    let mut target = PathBuf::from(name);
    if fs::metadata(&target).await.is_ok() {
      let mut name = self.file.as_os_str().to_owned();
      name.push(format!(".corrupt-{stamp}-{}", crate::util::random_hex(4)));
      target = PathBuf::from(name);
    }
    fs::rename(&self.file, &target).await?;
    *self.corrupt.lock() = None;
    (self.log)(&format!("secrets.json unreadable ({why}); moved corrupt file to {} and starting fresh", target.display()));
    Ok(target)
  }

  pub async fn get(&self, key: &str) -> Result<Option<String>> {
    with_file_lock(&self.file, || async {
      match self.read().await {
        VaultRead::Ok(m) => Ok(m.get(key).and_then(Value::as_str).map(str::to_owned)),
        VaultRead::Corrupt(_) => Ok(None),
        VaultRead::Io(e) => Err(e.into()),
      }
    })
    .await
  }

  pub async fn store(&self, key: &str, value: &str) -> Result<()> {
    self
      .mutate(true, |d| {
        d.insert(key.to_owned(), Value::from(value));
      })
      .await
  }

  pub async fn delete(&self, key: &str) -> Result<()> {
    self
      .mutate(false, |d| {
        d.remove(key);
      })
      .await
  }

  async fn mutate(&self, recover_corrupt: bool, f: impl FnOnce(&mut Map<String, Value>)) -> Result<()> {
    with_file_lock(&self.file, || async {
      let mut data = match self.read().await {
        VaultRead::Ok(data) => data,
        VaultRead::Corrupt(why) if recover_corrupt => {
          self.quarantine_corrupt(&why).await?;
          Map::new()
        }
        VaultRead::Corrupt(_) => return Ok(()),
        VaultRead::Io(e) => return Err(e.into()),
      };
      f(&mut data);
      if let Some(dir) = self.file.parent() {
        fs::create_dir_all(dir).await?;
      }
      write_atomic(&self.file, serde_json::to_string_pretty(&data)?.as_bytes(), Some(0o600)).await?;
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.file, std::fs::Permissions::from_mode(0o600)).await;
      }
      Ok(())
    })
    .await
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAccount {
  pub id: String,
  pub agent: AgentId,
  pub label: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
  #[serde(default)]
  pub added_at: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_used_at: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub quota: Option<AccountQuota>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub meta: Option<BTreeMap<String, String>>,
  // Fields another build wrote survive our rewrite
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

impl StoredAccount {
  pub fn info(&self) -> AccountInfo {
    AccountInfo {
      id: self.id.clone(),
      agent: self.agent.clone(),
      label: self.label.clone(),
      detail: self.detail.clone(),
      added_at: self.added_at.clone(),
      last_used_at: self.last_used_at.clone(),
      quota: self.quota.clone(),
      quota_issue: None,
      key_auth: self.meta.as_ref().is_some_and(|m| m.get(META_AUTH).is_some_and(|a| a == AUTH_API_KEY)).then_some(true),
      plugin: None,
      badge: None,
      removable: None,
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountCredential {
  pub secret: String,
  pub meta: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountDraft {
  pub label: String,
  pub detail: Option<String>,
  pub secret: String,
  pub meta: Option<BTreeMap<String, String>>,
}

/// Account metadata in accounts.json, secrets in the vault keyed by id. `items` is a cache; every change re-reads the
/// file under its lock, and the whole store is serialized by one async mutex (reload and mutate share the queue)
pub struct AccountStore {
  file: PathBuf,
  vault: std::sync::Arc<FileVault>,
  log: LogFn,
  items: parking_lot::Mutex<Vec<StoredAccount>>,
  queue: tokio::sync::Mutex<()>,
}

fn legacy_detail(d: &Option<String>) -> Option<String> {
  d.as_deref().and_then(|s| s.split(" · ").next()).filter(|s| !s.is_empty()).map(str::to_owned)
}

impl AccountStore {
  pub fn new(file: PathBuf, vault: std::sync::Arc<FileVault>, log: LogFn) -> Self {
    AccountStore { file, vault, log, items: Default::default(), queue: tokio::sync::Mutex::new(()) }
  }

  pub fn vault(&self) -> &std::sync::Arc<FileVault> {
    &self.vault
  }

  async fn read(&self) -> Vec<StoredAccount> {
    let Ok(raw) = fs::read(&self.file).await else { return vec![] };
    match serde_json::from_slice::<Value>(&raw) {
      Ok(Value::Array(items)) => items.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect(),
      Ok(_) => {
        (self.log)("accounts.json unreadable, starting with no accounts (not an array)");
        vec![]
      }
      Err(e) => {
        (self.log)(&format!("accounts.json unreadable, starting with no accounts ({e})"));
        vec![]
      }
    }
  }

  pub async fn load(&self) -> Result<()> {
    {
      let _q = self.queue.lock().await;
      let items = self.read().await;
      *self.items.lock() = items;
    }
    // Older drafts stored detail as '{tier} · {name}': keep the leading segment
    let stale = self.items.lock().iter().any(|a| a.detail != legacy_detail(&a.detail));
    if stale {
      self
        .mutate(|items| {
          for a in items.iter_mut() {
            a.detail = legacy_detail(&a.detail);
          }
          Ok(())
        })
        .await?;
    }
    Ok(())
  }

  /// Re-read what other hosts wrote; true when the list differs
  pub async fn reload(&self) -> bool {
    let _q = self.queue.lock().await;
    let fresh = self.read().await;
    let mut items = self.items.lock();
    let changed = *items != fresh;
    *items = fresh;
    changed
  }

  async fn mutate(&self, f: impl FnOnce(&mut Vec<StoredAccount>) -> Result<()>) -> Result<()> {
    let _q = self.queue.lock().await;
    with_file_lock(&self.file, || async {
      let mut items = self.read().await;
      f(&mut items)?;
      if let Some(dir) = self.file.parent() {
        fs::create_dir_all(dir).await?;
      }
      write_atomic(&self.file, serde_json::to_string_pretty(&items)?.as_bytes(), Some(0o600)).await?;
      *self.items.lock() = items;
      Ok(())
    })
    .await
  }

  pub fn list(&self, agent: Option<&str>) -> Vec<AccountInfo> {
    self.items.lock().iter().filter(|a| agent.is_none_or(|g| a.agent == g)).map(StoredAccount::info).collect()
  }

  pub fn get(&self, id: &str) -> Option<AccountInfo> {
    self.items.lock().iter().find(|a| a.id == id).map(StoredAccount::info)
  }

  /// The most recently used one is the default; if none has ever been used, the earliest added
  pub fn default_for(&self, agent: &str) -> Option<AccountInfo> {
    let mut list = self.list(Some(agent));
    list.sort_by(|a, b| {
      b.last_used_at.as_deref().unwrap_or("").cmp(a.last_used_at.as_deref().unwrap_or("")).then_with(|| a.added_at.cmp(&b.added_at))
    });
    list.into_iter().next()
  }

  /// Same label under the same agent counts as the same account: re-login swaps the secret
  pub async fn add(&self, agent: &str, draft: AccountDraft) -> Result<AccountInfo> {
    let vault = self.vault.clone();
    let _q = self.queue.lock().await;
    with_file_lock(&self.file, || async {
      let mut items = self.read().await;
      let idx = match items.iter().position(|x| x.agent == agent && x.label == draft.label) {
        Some(i) => {
          items[i].detail = draft.detail.clone();
          items[i].meta = draft.meta.clone();
          i
        }
        None => {
          items.push(StoredAccount {
            id: random_uuid(),
            agent: agent.to_owned(),
            label: draft.label.clone(),
            detail: draft.detail.clone(),
            added_at: now_iso(),
            last_used_at: None,
            quota: None,
            meta: draft.meta.clone(),
            extra: Map::new(),
          });
          items.len() - 1
        }
      };
      vault.store(&account_secret_key(&items[idx].id), &draft.secret).await?;
      let added = items[idx].info();
      if let Some(dir) = self.file.parent() {
        fs::create_dir_all(dir).await?;
      }
      write_atomic(&self.file, serde_json::to_string_pretty(&items)?.as_bytes(), Some(0o600)).await?;
      *self.items.lock() = items;
      Ok(added)
    })
    .await
  }

  pub async fn remove(&self, id: &str) -> Result<()> {
    if !self.items.lock().iter().any(|x| x.id == id) {
      return Ok(());
    }
    let vault = self.vault.clone();
    let _q = self.queue.lock().await;
    with_file_lock(&self.file, || async {
      let mut items = self.read().await;
      items.retain(|x| x.id != id);
      vault.delete(&account_secret_key(id)).await?;
      write_atomic(&self.file, serde_json::to_string_pretty(&items)?.as_bytes(), Some(0o600)).await?;
      *self.items.lock() = items;
      Ok(())
    })
    .await
  }

  pub async fn credential(&self, id: &str) -> Result<Option<AccountCredential>> {
    let meta = match self.items.lock().iter().find(|x| x.id == id) {
      Some(a) => a.meta.clone(),
      None => return Ok(None),
    };
    let secret = self.vault.get(&account_secret_key(id)).await?;
    Ok(secret.filter(|s| !s.is_empty()).map(|secret| AccountCredential { secret, meta }))
  }

  /// A new identity for an account (the local login it stands for now belongs to someone else); the secret stays
  pub async fn set_identity(&self, id: &str, label: &str, detail: Option<String>) -> Result<()> {
    let (id, label) = (id.to_owned(), label.to_owned());
    self
      .mutate(move |items| {
        if let Some(a) = items.iter_mut().find(|x| x.id == id) {
          a.label = label;
          a.detail = detail;
        }
        Ok(())
      })
      .await
  }

  /// The plan a quota read reported; only the detail changes, so a rename that raced the read is kept
  pub async fn set_detail(&self, id: &str, detail: Option<String>) -> Result<()> {
    let id = id.to_owned();
    self
      .mutate(move |items| {
        if let Some(a) = items.iter_mut().find(|x| x.id == id) {
          a.detail = detail;
        }
        Ok(())
      })
      .await
  }

  /// accounts-dismissed.json next to accounts.json: `{ agent: [label, …] }`, the local logins the user removed and the
  /// automatic import must leave alone (a separate file, so accounts.json keeps the array older builds read)
  /// quota-cache.json next to accounts.json (`accounts::quota_cache`)
  pub fn quota_cache_file(&self) -> PathBuf {
    self.file.with_file_name("quota-cache.json")
  }

  fn dismissed_file(&self) -> PathBuf {
    self.file.with_file_name("accounts-dismissed.json")
  }

  async fn read_dismissed(&self) -> Map<String, Value> {
    let Ok(raw) = fs::read(self.dismissed_file()).await else { return Map::new() };
    serde_json::from_slice::<Value>(&raw).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default()
  }

  pub async fn is_dismissed(&self, agent: &str, label: &str) -> bool {
    self.read_dismissed().await.get(agent).and_then(Value::as_array).is_some_and(|l| l.iter().any(|x| x.as_str() == Some(label)))
  }

  /// Record (`on`) or clear a dismissal
  pub async fn set_dismissed(&self, agent: &str, label: &str, on: bool) -> Result<()> {
    let file = self.dismissed_file();
    with_file_lock(&file, || async {
      let mut data = self.read_dismissed().await;
      let mut labels: Vec<String> =
        data.get(agent).and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
      let had = labels.iter().any(|l| l == label);
      if had == on {
        return Ok(());
      }
      labels.retain(|l| l != label);
      if on {
        labels.push(label.to_owned());
      }
      if labels.is_empty() {
        data.remove(agent);
      } else {
        data.insert(agent.to_owned(), Value::from(labels));
      }
      if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).await?;
      }
      write_atomic(&file, serde_json::to_string_pretty(&data)?.as_bytes(), Some(0o600)).await
    })
    .await
  }

  /// Marks the account used; one another host removed meanwhile is not resurrected
  pub async fn touch(&self, id: &str) -> Result<()> {
    if !self.items.lock().iter().any(|x| x.id == id) {
      return Ok(());
    }
    let id = id.to_owned();
    self
      .mutate(move |items| {
        if let Some(a) = items.iter_mut().find(|x| x.id == id) {
          a.last_used_at = Some(now_iso());
        }
        Ok(())
      })
      .await
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::Arc;

  #[tokio::test]
  async fn add_touch_remove_and_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let log: LogFn = Arc::new(|_| {});
    let vault = Arc::new(FileVault::new(dir.path().join("secrets.json"), log.clone()));
    let store = AccountStore::new(dir.path().join("accounts.json"), vault.clone(), log);
    store.load().await.unwrap();
    let a =
      store.add("devin", AccountDraft { label: "a@b".into(), detail: Some("Pro".into()), secret: "s1".into(), meta: None }).await.unwrap();
    let again = store.add("devin", AccountDraft { label: "a@b".into(), detail: None, secret: "s2".into(), meta: None }).await.unwrap();
    assert_eq!(a.id, again.id);
    assert_eq!(store.credential(&a.id).await.unwrap().unwrap().secret, "s2");
    store.touch(&a.id).await.unwrap();
    assert!(store.default_for("devin").unwrap().last_used_at.is_some());
    store.remove(&a.id).await.unwrap();
    assert!(store.list(None).is_empty());
    assert_eq!(vault.get(&account_secret_key(&a.id)).await.unwrap(), None);
    // A corrupt vault reports its error, leaves delete as a no-op, and quarantines itself on the next store
    std::fs::write(dir.path().join("secrets.json"), "{oops").unwrap();
    assert_eq!(vault.get("k").await.unwrap(), None);
    assert!(vault.error().is_some());
    vault.delete("k").await.unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join("secrets.json")).unwrap(), "{oops");
    vault.store("k", "v").await.unwrap();
    assert_eq!(vault.error(), None);
    assert_eq!(vault.get("k").await.unwrap().as_deref(), Some("v"));
    let backup = std::fs::read_dir(dir.path())
      .unwrap()
      .filter_map(Result::ok)
      .map(|e| e.path())
      .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("secrets.json.corrupt-")))
      .expect("corrupt vault archived");
    assert_eq!(std::fs::read_to_string(backup).unwrap(), "{oops");
  }
}
