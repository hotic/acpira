//! Pi's project trust (pi 0.86.0 `core/trust-manager.js`, `core/project-trust.js`): a project's `.agents/skills` is
//! loaded only when `<agent dir>/trust.json` trusts the project or one of its parents, or the global setting
//! `defaultProjectTrust` is `always`. Over ACP Pi runs in RPC mode, which never asks, so an untrusted project silently
//! goes without its shared skills. The store is Pi's own: keys are canonical paths, values true / false / null, the file
//! is guarded by a `trust.json.lock` directory (proper-lockfile) and written as sorted, pretty JSON

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use super::Places;
use crate::platform::paths::canonical_for_cli;

/// `PI_CODING_AGENT_DIR` when set, else `~/.pi/agent`
pub fn agent_dir(places: &Places) -> PathBuf {
  match std::env::var_os("PI_CODING_AGENT_DIR") {
    Some(d) if !d.is_empty() => PathBuf::from(d),
    _ => places.home.join(".pi").join("agent"),
  }
}

fn read_object(path: &Path) -> Result<Map<String, Value>> {
  match std::fs::read_to_string(path) {
    Ok(text) => match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
      Ok(Value::Object(m)) => Ok(m),
      Ok(_) => bail!("{} is not a JSON object", path.display()),
      Err(e) => bail!("{} is not valid JSON: {e}", path.display()),
    },
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
    Err(e) => Err(e.into()),
  }
}

/// Whether Pi loads project resources in `root`: the nearest trust entry decides, else `defaultProjectTrust`
pub fn trusted(places: &Places, root: &Path) -> bool {
  let dir = agent_dir(places);
  let store = read_object(&dir.join("trust.json")).unwrap_or_default();
  let mut cur = canonical_for_cli(root).unwrap_or_else(|_| root.to_path_buf());
  loop {
    if let Some(Value::Bool(b)) = store.get(&*cur.to_string_lossy()) {
      return *b;
    }
    if !cur.pop() {
      break;
    }
  }
  let settings = read_object(&dir.join("settings.json")).unwrap_or_default();
  settings.get("defaultProjectTrust").and_then(Value::as_str) == Some("always")
}

/// Held while the trust store is edited; the directory is what proper-lockfile creates, so Pi waits for it too
struct Lock(PathBuf);

impl Lock {
  fn take(file: &Path) -> Result<Lock> {
    let dir = PathBuf::from(format!("{}.lock", file.display()));
    for _ in 0..50 {
      match std::fs::create_dir(&dir) {
        Ok(()) => return Ok(Lock(dir)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
          // proper-lockfile's own staleness rule: a lock not refreshed for 10 s belongs to a dead process
          let stale = std::fs::metadata(&dir)
            .and_then(|m| m.modified())
            .is_ok_and(|t| SystemTime::now().duration_since(t).unwrap_or_default() > Duration::from_secs(10));
          if stale {
            let _ = std::fs::remove_dir(&dir);
            continue;
          }
          std::thread::sleep(Duration::from_millis(20));
        }
        Err(e) => return Err(e).with_context(|| format!("locking {}", file.display())),
      }
    }
    bail!("{} is locked by another process", file.display())
  }
}

impl Drop for Lock {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir(&self.0);
  }
}

/// Record `root` as trusted, the same entry Pi's own "Trust" answer writes
pub fn trust(places: &Places, root: &Path) -> Result<()> {
  let dir = agent_dir(places);
  std::fs::create_dir_all(&dir)?;
  let file = dir.join("trust.json");
  let _lock = Lock::take(&file)?;
  let mut store = read_object(&file)?;
  let key = canonical_for_cli(root).unwrap_or_else(|_| root.to_path_buf()).to_string_lossy().into_owned();
  store.insert(key, Value::Bool(true));
  // Pi writes its keys sorted; the workspace's serde_json keeps insertion order, so sort explicitly
  let sorted: std::collections::BTreeMap<String, Value> = store.into_iter().collect();
  let text = format!("{}\n", serde_json::to_string_pretty(&sorted)?);
  let tmp = dir.join(format!(".trust.json.{}.tmp", std::process::id()));
  std::fs::write(&tmp, text)?;
  std::fs::rename(&tmp, &file)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn nearest_entry_decides_and_trust_writes_pis_format() {
    // The env var would point the write at a real Pi store
    if std::env::var_os("PI_CODING_AGENT_DIR").is_some() {
      return;
    }
    let t = tempfile::tempdir().unwrap();
    // Node's realpath uses ordinary drive / UNC paths, without Rust's verbatim prefix.
    #[cfg(windows)]
    let home = t.path().to_path_buf();
    #[cfg(not(windows))]
    let home = std::fs::canonicalize(t.path()).unwrap();
    let root = home.join("work").join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let places = Places { home: home.clone(), config: home.join(".config"), root: Some(root.clone()) };
    let dir = home.join(".pi/agent");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(!trusted(&places, &root));

    std::fs::write(dir.join("settings.json"), r#"{ "defaultProjectTrust": "always" }"#).unwrap();
    assert!(trusted(&places, &root));
    // An explicit "no" for a parent beats the default
    let mut expected = std::collections::BTreeMap::from([(home.join("work").to_string_lossy().into_owned(), false)]);
    std::fs::write(dir.join("trust.json"), serde_json::to_string_pretty(&expected).unwrap()).unwrap();
    assert!(!trusted(&places, &root));

    trust(&places, &root).unwrap();
    assert!(trusted(&places, &root));
    let text = std::fs::read_to_string(dir.join("trust.json")).unwrap();
    expected.insert(root.to_string_lossy().into_owned(), true);
    assert_eq!(text, format!("{}\n", serde_json::to_string_pretty(&expected).unwrap()));
    assert!(!dir.join("trust.json.lock").exists());
  }
}
