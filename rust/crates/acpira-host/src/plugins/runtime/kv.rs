//! A plugin's key-value store: one JSON object written tmp + rename on every change, shared by every engine

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value};


/// The most a plugin may keep in its key-value store
pub(super) const KV_MAX_BYTES: usize = 1024 * 1024;

/// A plugin's key-value store: one JSON object written tmp + rename on every change, at most `KV_MAX_BYTES`
pub(super) struct Kv {
  path: PathBuf,
  /// What the file held at the stamp it had; read again once the stamp moves (another engine wrote it)
  cache: parking_lot::Mutex<Option<(KvStamp, Map<String, Value>)>>,
  seq: AtomicU64,
}

type KvStamp = Option<(std::time::SystemTime, u64)>;

fn kv_stamp(path: &Path) -> KvStamp {
  let m = std::fs::metadata(path).ok()?;
  Some((m.modified().ok()?, m.len()))
}

impl Kv {
  pub(super) fn new(path: PathBuf) -> Kv {
    Kv { path, cache: Default::default(), seq: AtomicU64::new(0) }
  }

  fn with<T>(&self, f: impl FnOnce(&Map<String, Value>) -> T) -> T {
    let stamp = kv_stamp(&self.path);
    let mut guard = self.cache.lock();
    if guard.as_ref().is_none_or(|(s, _)| *s != stamp) {
      *guard = Some((stamp, self.read()));
    }
    let empty = Map::new();
    f(guard.as_ref().map_or(&empty, |(_, m)| m))
  }

  fn read(&self) -> Map<String, Value> {
    std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
  }

  pub(super) fn get(&self, key: &str) -> Value {
    self.with(|m| m.get(key).cloned().unwrap_or(Value::Null))
  }

  pub(super) fn keys(&self, prefix: &str) -> Vec<String> {
    self.with(|m| m.keys().filter(|k| k.starts_with(prefix)).cloned().collect())
  }

  /// Read, change and write the file under its lock (this engine's and the others'), so no writer loses another's key
  pub(super) async fn change(&self, f: impl FnOnce(&mut Map<String, Value>) + Send) -> Result<(), String> {
    let path = self.path.clone();
    crate::store::file_lock::with_file_lock(&path, || async {
      let mut next = self.read();
      f(&mut next);
      let bytes = serde_json::to_vec(&next)?;
      if bytes.len() > KV_MAX_BYTES {
        anyhow::bail!("the store would hold {} KB, more than the {} KB a plugin may keep", bytes.len() / 1024, KV_MAX_BYTES / 1024);
      }
      if let Some(parent) = self.path.parent() {
        std::fs::create_dir_all(parent)?;
      }
      let tmp = self.path.with_extension(format!("tmp{}-{}", std::process::id(), self.seq.fetch_add(1, Ordering::Relaxed)));
      if let Err(e) = std::fs::write(&tmp, &bytes).and_then(|()| std::fs::rename(&tmp, &self.path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
      }
      *self.cache.lock() = Some((kv_stamp(&self.path), next));
      Ok(())
    })
    .await
    .map_err(|e| format!("{e:#}"))
  }
}
