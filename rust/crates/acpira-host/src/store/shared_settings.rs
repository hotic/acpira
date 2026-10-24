//! The settings every shell shares (`acpira_shared::settings::is_shared_setting`): one flat JSON object of `acpira.*`
//! keys without the prefix in `<ACPIRA_HOME>/settings.json`, the single source for VS Code, the desktop app and IntelliJ
//! like the transcripts next to it (2026-10-23). Before, each shell kept its own copy (VS Code's settings.json, the
//! desktop app's userData, IntelliJ's application storage), so two shells on one machine could disagree on which agents
//! are on. Written tmp + rename under the file lock; other engines notice a change through `reload`, which the runtime's
//! one-second config tick calls

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde_json::{Map, Value};

use acpira_shared::settings::is_shared_setting;

use super::file_lock::{with_file_lock, write_atomic};

type Stamp = (u64, Option<SystemTime>);

struct State {
  values: Map<String, Value>,
  // The file as last read or written; None while it does not exist
  stamp: Option<Stamp>,
}

pub struct SharedSettings {
  path: PathBuf,
  state: parking_lot::Mutex<State>,
}

fn stamp_of(path: &Path) -> Option<Stamp> {
  let m = std::fs::metadata(path).ok()?;
  Some((m.len(), m.modified().ok()))
}

// A missing or broken file reads as no values; a broken one is replaced by the next write
fn read_values(path: &Path) -> Map<String, Value> {
  std::fs::read(path)
    .ok()
    .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
    .and_then(|v| match v {
      Value::Object(m) => Some(m),
      _ => None,
    })
    .unwrap_or_default()
}

impl SharedSettings {
  pub fn open(root: &Path) -> Self {
    let path = root.join("settings.json");
    let stamp = stamp_of(&path);
    let values = if stamp.is_some() { read_values(&path) } else { Map::new() };
    SharedSettings { path, state: parking_lot::Mutex::new(State { values, stamp }) }
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  /// Whether the file exists: until it does, readers fall back to their shell's own value
  pub fn exists(&self) -> bool {
    self.state.lock().stamp.is_some()
  }

  pub fn get(&self, key: &str) -> Option<Value> {
    self.state.lock().values.get(key).cloned()
  }

  /// The first engine on a machine creates the file from its shell's snapshot (the values that shell had until now);
  /// a file another engine created meanwhile is kept. Returns whether this call created it
  pub async fn seed(&self, snapshot: &Map<String, Value>) -> Result<bool> {
    let path = self.path.clone();
    let seeded: Map<String, Value> =
      snapshot.iter().filter(|(k, v)| is_shared_setting(k) && !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect();
    let created = with_file_lock(&path, || async {
      if stamp_of(&path).is_some() {
        return Ok(false);
      }
      if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
      }
      write_atomic(&path, serde_json::to_string_pretty(&Value::Object(seeded))?.as_bytes(), None).await?;
      Ok(true)
    })
    .await?;
    self.reload();
    Ok(created)
  }

  /// Write one key (Null removes it, back to the default) onto what is on disk, so two engines' edits compose
  pub async fn write(&self, key: &str, value: Value) -> Result<()> {
    let path = self.path.clone();
    let values = with_file_lock(&path, || async {
      let mut values = read_values(&path);
      if value.is_null() {
        values.remove(key);
      } else {
        values.insert(key.to_owned(), value);
      }
      if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
      }
      write_atomic(&path, serde_json::to_string_pretty(&Value::Object(values.clone()))?.as_bytes(), None).await?;
      Ok(values)
    })
    .await?;
    let mut st = self.state.lock();
    st.values = values;
    st.stamp = stamp_of(&path);
    Ok(())
  }

  /// Read the file again when it changed since the last read or write (another engine wrote it, or a hand edit); the keys
  /// whose value differs
  pub fn reload(&self) -> Vec<String> {
    let stamp = stamp_of(&self.path);
    let mut st = self.state.lock();
    if stamp == st.stamp {
      return vec![];
    }
    let values = if stamp.is_some() { read_values(&self.path) } else { Map::new() };
    let mut changed: Vec<String> = values.iter().filter(|(k, v)| st.values.get(*k) != Some(*v)).map(|(k, _)| k.clone()).collect();
    changed.extend(st.values.keys().filter(|k| !values.contains_key(*k)).cloned());
    st.values = values;
    st.stamp = stamp;
    changed
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn map(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
  }

  #[tokio::test]
  async fn the_first_shell_seeds_the_file_with_its_shared_keys_and_a_later_seed_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let a = SharedSettings::open(dir.path());
    assert!(!a.exists());
    let first = map(json!({ "disabledAgents": ["kimi"], "sessionScope": "all", "appearance.font": "geist", "agents": [{ "id": "x" }], "subagents": [] }));
    assert!(a.seed(&first).await.unwrap());
    assert!(a.exists());
    assert_eq!(a.get("disabledAgents"), Some(json!(["kimi"])));
    assert_eq!(a.get("appearance.font"), Some(json!("geist")));
    // The shell's own keys, the legacy agent list and the personas (their own file) stay out
    for k in ["sessionScope", "agents", "subagents"] {
      assert_eq!(a.get(k), None, "{k}");
    }
    let b = SharedSettings::open(dir.path());
    assert!(!b.seed(&map(json!({ "disabledAgents": [] }))).await.unwrap());
    assert_eq!(b.get("disabledAgents"), Some(json!(["kimi"])));
  }

  #[tokio::test]
  async fn writes_compose_across_engines_and_reload_names_what_changed() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (SharedSettings::open(dir.path()), SharedSettings::open(dir.path()));
    a.write("theme", json!("dark")).await.unwrap();
    b.write("language", json!("en")).await.unwrap();
    assert_eq!(a.get("language"), None, "not read again yet");
    // A same-second rewrite of the same length could keep the stamp; the length differs here
    assert_eq!(a.reload(), vec!["language".to_owned()]);
    assert_eq!(a.get("theme"), Some(json!("dark")));
    assert!(a.reload().is_empty());
    b.write("theme", Value::Null).await.unwrap();
    assert_eq!(a.reload(), vec!["theme".to_owned()]);
    assert_eq!(a.get("theme"), None);
  }
}
