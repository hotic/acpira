//! The settings every shell shares (<ACPIRA_HOME>/settings.json, acpira-host store/shared_settings.rs), read by the app
//! for what it needs before or beside the engine: its own UI language, the menu, the browser panel's look, the page's
//! font, how files open. The engine alone writes it (the settings page's setSetting); until the first engine has created
//! it, the app falls back to its own legacy value, the one that engine will seed the file from

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use acpira_shared::settings::is_shared_setting;
use serde_json::{Map, Value};

pub struct SharedSettingsFile {
  pub path: PathBuf,
  values: Arc<Mutex<Option<Map<String, Value>>>>,
  stop: Arc<AtomicBool>,
}

/// What the poll compares: a file replaced by rename gets a new inode even at the same size and second
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
  modified: Option<SystemTime>,
  len: u64,
  ino: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
  let m = std::fs::metadata(path).ok()?;
  #[cfg(unix)]
  let ino = std::os::unix::fs::MetadataExt::ino(&m);
  #[cfg(not(unix))]
  let ino = 0;
  Some(Stamp { modified: m.modified().ok(), len: m.len(), ino })
}

impl SharedSettingsFile {
  pub fn new(home: &Path) -> SharedSettingsFile {
    let path = home.join("settings.json");
    let values = read(&path);
    SharedSettingsFile { path, values: Arc::new(Mutex::new(values)), stop: Arc::new(AtomicBool::new(false)) }
  }

  pub fn exists(&self) -> bool {
    self.values.lock().unwrap().is_some()
  }

  /// A shared key from the file once it exists; otherwise (and for the shell's own keys) the shell's value
  pub fn get(&self, key: &str, own: impl FnOnce(&str) -> Option<Value>) -> Option<Value> {
    let v = self.values.lock().unwrap();
    match v.as_ref() {
      Some(m) if is_shared_setting(key) => m.get(key).cloned(),
      _ => {
        drop(v);
        own(key)
      }
    }
  }

  /// Polled (one stat a second, `interval`): the engines replace the file by rename, and ~/.acpira can sit on a volume
  /// without reliable native watch events. `on_change` gets the keys whose value moved, on the poll thread
  pub fn watch(&self, interval: Duration, on_change: impl Fn(Vec<String>) + Send + 'static) {
    let (path, values, stop) = (self.path.clone(), self.values.clone(), self.stop.clone());
    std::thread::Builder::new()
      .name("shared-settings".into())
      .spawn(move || {
        let mut last = stamp(&path);
        while !stop.load(Ordering::Relaxed) {
          std::thread::sleep(interval);
          let now = stamp(&path);
          if now == last {
            continue;
          }
          last = now;
          let after = read(&path);
          let keys = {
            let mut v = values.lock().unwrap();
            let before = v.clone().unwrap_or_default();
            let a = after.clone().unwrap_or_default();
            *v = after;
            let mut keys: Vec<String> = before.keys().chain(a.keys().filter(|k| !before.contains_key(*k))).cloned().collect();
            keys.retain(|k| before.get(k) != a.get(k));
            keys
          };
          if !keys.is_empty() {
            on_change(keys);
          }
        }
      })
      .expect("shared settings thread");
  }

  pub fn unwatch(&self) {
    self.stop.store(true, Ordering::Relaxed);
  }
}

impl Drop for SharedSettingsFile {
  fn drop(&mut self) {
    self.unwatch();
  }
}

/// None: no file yet; a file that does not parse reads as empty (the engine replaces it on its next write)
fn read(path: &Path) -> Option<Map<String, Value>> {
  let bytes = std::fs::read(path).ok()?;
  Some(match serde_json::from_slice::<Value>(&bytes) {
    Ok(Value::Object(m)) => m,
    _ => Map::new(),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn reads_shared_keys_from_the_file_once_it_exists_and_the_shells_own_otherwise() {
    let d = tempfile::tempdir().unwrap();
    let f = SharedSettingsFile::new(d.path());
    assert!(!f.exists());
    assert_eq!(f.get("language", |_| Some(json!("ja"))), Some(json!("ja")));
    std::fs::write(d.path().join("settings.json"), r#"{ "language": "zh-CN", "sessionListPosition": "right" }"#).unwrap();
    let f = SharedSettingsFile::new(d.path());
    assert!(f.exists());
    assert_eq!(f.get("language", |_| Some(json!("ja"))), Some(json!("zh-CN")));
    // A shell key never comes from the shared file
    assert_eq!(f.get("sessionListPosition", |_| Some(json!("left"))), Some(json!("left")));
    // A shared key missing from the file is unset, not the shell's legacy value
    assert_eq!(f.get("theme", |_| Some(json!("dark"))), None);
  }

  #[test]
  fn the_poll_reports_the_keys_whose_value_moved() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("settings.json");
    std::fs::write(&path, r#"{ "language": "en", "theme": "dark" }"#).unwrap();
    let f = SharedSettingsFile::new(d.path());
    let (tx, rx) = std::sync::mpsc::channel();
    f.watch(Duration::from_millis(20), move |keys| {
      let _ = tx.send(keys);
    });
    std::thread::sleep(Duration::from_millis(60));
    let tmp = d.path().join("settings.json.tmp");
    std::fs::write(&tmp, r#"{ "language": "fr", "theme": "dark", "uiFontSize": 14 }"#).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    let keys = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(keys, ["language", "uiFontSize"]);
    assert_eq!(f.get("language", |_| None), Some(json!("fr")));
  }
}
