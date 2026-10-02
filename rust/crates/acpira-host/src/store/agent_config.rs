//! Machine-local agent launch definitions. IDE settings are a one-time migration source only;
//! every shell on this host subsequently reads the same agents.json.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::{Map, Value};
use tokio::fs;

use crate::acp::agents::registry::CustomAgentSetting;
use crate::platform::command::Os;
use crate::store::file_lock::{with_file_lock, write_atomic};

pub struct AgentConfig {
  path: PathBuf,
  value: parking_lot::RwLock<Value>,
  reload: tokio::sync::Mutex<()>,
}

impl AgentConfig {
  pub fn new(root: &Path) -> Self {
    Self { path: root.join("agents.json"), value: parking_lot::RwLock::new(Value::Object(Map::new())), reload: Default::default() }
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn snapshot(&self) -> Value {
    self.value.read().clone()
  }

  /// First writer wins, including an empty file: another IDE must never re-import its old overrides.
  /// A malformed existing file is preserved and reported, never replaced with migrated settings.
  pub async fn initialize(&self, legacy: Option<Value>) -> Result<()> {
    fs::create_dir_all(self.path.parent().expect("config parent")).await?;
    with_file_lock(&self.path, || async {
      match fs::metadata(&self.path).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
          let value = migrate(legacy.as_ref(), Os::current());
          let mut bytes = serde_json::to_vec_pretty(&value)?;
          bytes.push(b'\n');
          // Definitions may carry credentials in env; use the same permissions as the vault.
          write_atomic(&self.path, &bytes, Some(0o600)).await?;
        }
        Err(e) => return Err(e.into()),
      }
      Ok(())
    })
    .await?;
    self.reload().await?;
    Ok(())
  }

  /// Only a complete, valid replacement changes the registry. Partial editor saves retain the last good definitions.
  pub async fn reload(&self) -> Result<bool> {
    let _guard = self.reload.lock().await;
    let value = read(&self.path).await?;
    let mut current = self.value.write();
    if *current == value {
      return Ok(false);
    }
    *current = value;
    Ok(true)
  }
}

pub async fn read(path: &Path) -> Result<Value> {
  let bytes = fs::read(path).await?;
  let value: Value =
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("invalid JSON at line {}, column {}", e.line(), e.column()))?;
  let Some(entries) = value.as_object() else { bail!("expected an object of agent definitions") };
  for (id, raw) in entries {
    // Do not include serde's value-bearing error: an env value may contain a credential.
    if id.trim().is_empty() || serde_json::from_value::<CustomAgentSetting>(raw.clone()).is_err() {
      bail!("invalid agent definition; command, args and env must match the agent schema");
    }
  }
  Ok(value)
}

fn migrate(legacy: Option<&Value>, os: Os) -> Value {
  let mut entries = Map::new();
  for (id, raw) in legacy.and_then(Value::as_object).into_iter().flatten() {
    let Ok(def) = serde_json::from_value::<CustomAgentSetting>(raw.clone()) else { continue };
    let command = def.command.trim();
    let drive = command.as_bytes().get(1) == Some(&b':') && command.as_bytes()[0].is_ascii_alphabetic();
    let windows_path = drive || command.starts_with("\\\\");
    // Older shells can still send a synced path from another OS. Discard that entire override,
    // so the built-in definition retains its arguments, authentication and discovery rules.
    if command.is_empty() || (os != Os::Windows && windows_path) || (os == Os::Windows && command.starts_with('/')) {
      continue;
    }
    entries.insert(id.clone(), raw.clone());
  }
  Value::Object(entries)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[tokio::test]
  async fn migrates_once_and_preserves_machine_config_across_shells() {
    let dir = tempfile::tempdir().unwrap();
    let a = AgentConfig::new(dir.path());
    let b = AgentConfig::new(dir.path());
    let custom = json!({"mine": {"command": "my-agent", "args": ["acp"], "env": {"CUSTOM": "value"}}});
    a.initialize(Some(custom.clone())).await.unwrap();
    b.initialize(Some(json!({"devin": {"command": "wrong"}}))).await.unwrap();
    assert_eq!(a.snapshot(), custom);
    assert_eq!(b.snapshot(), custom);
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(fs::metadata(a.path()).await.unwrap().permissions().mode() & 0o777, 0o600);
    }
  }

  #[tokio::test]
  async fn empty_config_is_authoritative_and_concurrent_initialization_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let a = AgentConfig::new(dir.path());
    let b = AgentConfig::new(dir.path());
    let (x, y) = tokio::join!(a.initialize(None), b.initialize(None));
    x.unwrap();
    y.unwrap();
    b.initialize(Some(json!({"devin": {"command": "wrong"}}))).await.unwrap();
    assert_eq!(b.snapshot(), json!({}));
  }

  #[test]
  fn migration_drops_foreign_paths_without_losing_other_agents() {
    let old = json!({
      "devin": {"command": "C:\\Users\\Spark\\AppData\\Local\\devin\\cli\\bin\\devin.exe", "args": ["acp"]},
      "network": {"command": "\\\\server\\tools\\agent.exe"},
      "posix": {"command": "/home/user/bin/agent"},
      "portable": {"command": "my-agent"}
    });
    let posix = migrate(Some(&old), Os::Posix);
    assert!(posix.get("devin").is_none());
    assert!(posix.get("network").is_none());
    assert_eq!(posix["portable"], old["portable"]);
    let windows = migrate(Some(&old), Os::Windows);
    assert!(windows.get("posix").is_none());
    assert_eq!(windows["devin"], old["devin"]);
  }

  #[tokio::test]
  async fn invalid_edits_and_startup_never_overwrite_the_file_or_last_good_config() {
    let dir = tempfile::tempdir().unwrap();
    let a = AgentConfig::new(dir.path());
    a.initialize(None).await.unwrap();
    let custom = json!({"mine": {"command": "agent"}});
    fs::write(a.path(), custom.to_string()).await.unwrap();
    assert!(a.reload().await.unwrap());
    fs::write(a.path(), b"{broken").await.unwrap();
    assert!(a.reload().await.is_err());
    assert_eq!(a.snapshot(), custom);
    assert!(a.initialize(Some(json!({}))).await.is_err());
    assert_eq!(fs::read(a.path()).await.unwrap(), b"{broken");
    fs::write(a.path(), br#"{"mine":{"command":"agent","env":{"API_KEY":12345}}}"#).await.unwrap();
    let error = a.reload().await.unwrap_err().to_string();
    assert!(!error.contains("12345"));
    assert_eq!(a.snapshot(), custom);
    fs::write(a.path(), b"{}").await.unwrap();
    assert!(a.reload().await.unwrap());
    assert_eq!(a.snapshot(), json!({}));
  }
}
