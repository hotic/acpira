//! The desktop app's own settings store: flat acpira.* keys without the prefix, like VS Code's settings.json section and
//! IntelliJ's application storage. It holds only the settings that follow the window (SHELL_SETTING_KEYS); the shared
//! ones live in <ACPIRA_HOME>/settings.json. The engine reads the snapshot (hello) and writes through writeSetting. It
//! lives in the app's own data directory, not in ~/.acpira, because the Storage tab files everything there by the record
//! that references it and would call an unreferenced file a leftover

use std::path::{Path, PathBuf};
use parking_lot::Mutex;
use serde_json::{Map, Value};

/// What a first launch starts with: the session column on the left over every project (the app has no workspace)
pub fn desktop_defaults() -> Map<String, Value> {
  let mut m = Map::new();
  m.insert("sessionListPosition".into(), Value::from("left"));
  m.insert("sessionScope".into(), Value::from("all"));
  m
}

type Listener = Box<dyn Fn(&[String], &Map<String, Value>) + Send + Sync>;

pub struct SettingsStore {
  file: PathBuf,
  values: Mutex<Map<String, Value>>,
  listeners: Mutex<Vec<Listener>>,
  /// True when this run created the file (a first launch)
  pub fresh: bool,
}

impl SettingsStore {
  pub fn new(file: impl Into<PathBuf>) -> SettingsStore {
    let file = file.into();
    let read = read(&file);
    let fresh = read.is_none();
    let store = SettingsStore { values: Mutex::new(read.unwrap_or_else(desktop_defaults)), file, listeners: Mutex::new(vec![]), fresh };
    if fresh {
      store.save(&store.values.lock());
    }
    store
  }

  pub fn file(&self) -> &Path {
    &self.file
  }

  pub fn snapshot(&self) -> Map<String, Value> {
    self.values.lock().clone()
  }

  pub fn get(&self, key: &str) -> Option<Value> {
    self.values.lock().get(key).cloned()
  }

  /// None removes the key (back to the engine's default)
  pub fn set(&self, key: &str, value: Option<Value>) {
    let snap = {
      let mut v = self.values.lock();
      match value {
        // JSON null is how a removal arrives over the wire (writeSetting with value undefined)
        None | Some(Value::Null) => {
          v.remove(key);
        }
        Some(x) => {
          v.insert(key.to_owned(), x);
        }
      }
      self.save(&v);
      v.clone()
    };
    self.notify(&[key.to_owned()], &snap);
  }

  /// Several keys at once (an import), one change event
  pub fn merge(&self, values: Map<String, Value>) {
    if values.is_empty() {
      return;
    }
    let keys: Vec<String> = values.keys().cloned().collect();
    let snap = {
      let mut v = self.values.lock();
      v.extend(values);
      self.save(&v);
      v.clone()
    };
    self.notify(&keys, &snap);
  }

  pub fn on_change(&self, listener: impl Fn(&[String], &Map<String, Value>) + Send + Sync + 'static) {
    self.listeners.lock().push(Box::new(listener));
  }

  fn notify(&self, keys: &[String], snap: &Map<String, Value>) {
    for l in self.listeners.lock().iter() {
      l(keys, snap);
    }
  }

  fn save(&self, values: &Map<String, Value>) {
    // A read-only profile keeps the values for this run
    let _ = crate::json_file::write_json_strict(&self.file, &Value::Object(values.clone()));
  }
}

fn read(file: &Path) -> Option<Map<String, Value>> {
  let text = std::fs::read(file).ok()?;
  match serde_json::from_slice::<Value>(&text) {
    Ok(Value::Object(m)) => Some(m),
    Ok(_) => Some(Map::new()),
    Err(_) => {
      // A broken file is kept aside rather than overwritten, and the app starts on what it can read: nothing
      let mut broken = file.as_os_str().to_owned();
      broken.push(".broken");
      let _ = std::fs::rename(file, broken);
      Some(Map::new())
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;
  use std::sync::Arc;

  // Ported from the Electron app's settings.test.ts
  fn file(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("nested").join("settings.json")
  }

  #[test]
  fn starts_a_first_launch_on_the_desktop_defaults_and_persists_them() {
    let d = tempfile::tempdir().unwrap();
    let f = file(&d);
    let s = SettingsStore::new(&f);
    assert!(s.fresh);
    assert_eq!(s.snapshot(), desktop_defaults());
    assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&f).unwrap()).unwrap(), Value::Object(desktop_defaults()));
    assert!(!SettingsStore::new(&f).fresh);
  }

  #[test]
  fn writes_removes_and_merges_keys_announcing_what_changed() {
    let d = tempfile::tempdir().unwrap();
    let f = file(&d);
    let s = SettingsStore::new(&f);
    let seen = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let w = seen.clone();
    s.on_change(move |keys, _| w.lock().push(keys.to_vec()));
    s.set("defaultAgent", Some(json!("claude")));
    s.set("sessionScope", None);
    let mut m = Map::new();
    m.insert("appearance.font".into(), json!("inter"));
    m.insert("hiddenOptions".into(), json!({}));
    s.merge(m);
    s.merge(Map::new());
    assert_eq!(
      *seen.lock(),
      vec![
        vec!["defaultAgent".to_owned()],
        vec!["sessionScope".to_owned()],
        vec!["appearance.font".to_owned(), "hiddenOptions".to_owned()]
      ]
    );
    assert_eq!(
      Value::Object(SettingsStore::new(&f).snapshot()),
      json!({ "sessionListPosition": "left", "defaultAgent": "claude", "appearance.font": "inter", "hiddenOptions": {} })
    );
  }

  #[test]
  fn keeps_a_broken_file_aside_instead_of_overwriting_it_with_defaults() {
    let d = tempfile::tempdir().unwrap();
    let f = file(&d);
    SettingsStore::new(&f);
    std::fs::write(&f, "{ not json").unwrap();
    let s = SettingsStore::new(&f);
    assert!(!s.fresh);
    assert!(s.snapshot().is_empty());
    assert_eq!(std::fs::read_to_string(f.with_file_name("settings.json.broken")).unwrap(), "{ not json");
  }
}
