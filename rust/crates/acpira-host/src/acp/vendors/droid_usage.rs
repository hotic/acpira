//! Droid's ACP adapter omits per-prompt usage. Its session settings keep the root session's cumulative token counts,
//! separate from child runs and the last-call context tally.

use std::io::Read;
use std::path::{Path, PathBuf};

use acpira_shared::num::Num;
use acpira_shared::transcript::TurnUsage;
use serde::Deserialize;

use crate::store::data_dir::home_dir;

const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

/// The launched CLI's home, derived from the child env diff without requiring a spawned `Command`.
pub(crate) fn sessions_dir_from_env(env_remove: &[String], env_set: &std::collections::BTreeMap<String, String>, cwd: &str) -> PathBuf {
  let eq = |a: &str, b: &str| if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b };
  let env = |key: &str| {
    if let Some((_, v)) = env_set.iter().find(|(k, _)| eq(k, key)) {
      return (!v.is_empty()).then(|| std::ffi::OsString::from(v));
    }
    if env_remove.iter().any(|k| eq(k, key)) {
      return None;
    }
    std::env::var_os(key).filter(|v| !v.is_empty())
  };
  let home = env("FACTORY_HOME_OVERRIDE")
    .or_else(|| env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }))
    .map(PathBuf::from)
    .unwrap_or_else(home_dir);
  Path::new(cwd).join(home).join(".factory").join("sessions")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
  token_usage: Counters,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Counters {
  input_tokens: u64,
  output_tokens: u64,
  cache_read_tokens: u64,
  cache_creation_tokens: u64,
  thinking_tokens: u64,
}

impl Counters {
  fn usage_since(&self, before: &Self) -> Option<TurnUsage> {
    let output = self.output_tokens.checked_sub(before.output_tokens)?;
    if output == 0 {
      return None;
    }
    let read = self.cache_read_tokens.checked_sub(before.cache_read_tokens)?;
    let write = self.cache_creation_tokens.checked_sub(before.cache_creation_tokens)?;
    let input = self.input_tokens.checked_sub(before.input_tokens)?.checked_add(read)?.checked_add(write)?;
    let thinking = self.thinking_tokens.checked_sub(before.thinking_tokens)?;
    Some(TurnUsage {
      input: Some(Num(input as f64)),
      output: Some(Num(output as f64)),
      cached_read: Some(Num(read as f64)),
      cached_write: Some(Num(write as f64)),
      // Output already includes thinking; Droid may estimate the breakdown from the visible reasoning text.
      reasoning: (thinking <= output).then_some(Num(thinking as f64)),
      total: Some(Num(input.checked_add(output)? as f64)),
      ..Default::default()
    })
  }
}

/// One prompt owns both reads. Without its starting snapshot, the end is just a session total.
pub(crate) struct Baseline {
  path: PathBuf,
  counts: Counters,
}

impl Baseline {
  pub(crate) async fn capture(sessions: PathBuf, id: String) -> Option<Self> {
    tokio::task::spawn_blocking(move || {
      let path = settings_path(&sessions, &id)?;
      let counts = read(&path)?;
      Some(Self { path, counts })
    })
    .await
    .ok()
    .flatten()
  }

  pub(crate) async fn usage(self) -> Option<TurnUsage> {
    tokio::task::spawn_blocking(move || {
      let mut after = read(&self.path);
      // Droid's settings writes can lag ACP's end_turn. Recheck once if the snapshot has not moved; never wait at
      // length or substitute a session total for a missing delta.
      if after.as_ref().is_none_or(|c| *c == self.counts) {
        std::thread::sleep(std::time::Duration::from_millis(50));
        after = read(&self.path);
      }
      after?.usage_since(&self.counts)
    })
    .await
    .ok()
    .flatten()
  }
}

/// Match the native session id, not the current workspace: a resumed session can keep its original project directory.
fn settings_path(sessions: &Path, id: &str) -> Option<PathBuf> {
  if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
    return None;
  }
  let name = format!("{id}.settings.json");
  let in_dir = |dir: &Path| {
    let p = dir.join(&name);
    std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()).then_some(p)
  };
  in_dir(sessions).or_else(|| {
    std::fs::read_dir(sessions).ok()?.flatten().find_map(|e| {
      let name = e.file_name();
      let name = name.to_str()?;
      if (name == "btw" || name.starts_with('-')) && e.file_type().ok()?.is_dir() {
        in_dir(&e.path())
      } else {
        None
      }
    })
  })
}

fn read(path: &Path) -> Option<Counters> {
  if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() <= MAX_SETTINGS_BYTES) {
    return None;
  }
  let mut bytes = Vec::new();
  std::fs::File::open(path).ok()?.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes).ok()?;
  if bytes.len() as u64 > MAX_SETTINGS_BYTES {
    return None;
  }
  Some(serde_json::from_slice::<Settings>(&bytes).ok()?.token_usage)
}
