//! quota-cache.json next to accounts.json: the last quota read of each account, shared by every host on the data dir.
//! Every VS Code / Cursor window and IDEA runs its own sidecar over the same accounts, and vendors rate-limit the quota
//! endpoints per token (Claude: a handful of calls), so one host's answer — bars or a 429 pause — serves all of them.
//! Written tmp + rename under the shared file lock; reads go without the lock since a rename is atomic

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::fs;

use acpira_shared::transcript::{AccountQuota, AccountQuotaIssue};

use crate::store::file_lock::{with_file_lock, write_atomic};

/// The last quota read of an account failed: why, when, and until when the next read waits (epoch ms)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaFailure {
  pub issue: AccountQuotaIssue,
  pub at_ms: i64,
  pub until_ms: i64,
  /// Consecutive 429s, for the backoff
  pub rate_limits: u32,
}

/// What one host last learned about an account's quota: the newest bars (kept through failed re-reads) and the
/// failure after them, if any
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaEntry {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub quota: Option<AccountQuota>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub failure: Option<QuotaFailure>,
}

pub struct QuotaCache {
  file: PathBuf,
}

impl QuotaCache {
  pub fn new(file: PathBuf) -> Self {
    QuotaCache { file }
  }

  async fn read_all(&self) -> BTreeMap<String, QuotaEntry> {
    let Ok(raw) = fs::read(&self.file).await else { return BTreeMap::new() };
    serde_json::from_slice(&raw).unwrap_or_default()
  }

  pub async fn get(&self, id: &str) -> Option<QuotaEntry> {
    self.read_all().await.remove(id)
  }

  /// Replace (Some) or drop (None) one account's entry; the rest of the file is re-read inside the lock and kept
  pub async fn set(&self, id: &str, entry: Option<QuotaEntry>) -> Result<()> {
    with_file_lock(&self.file, || async {
      let mut all = self.read_all().await;
      let changed = match entry {
        Some(e) => all.insert(id.to_owned(), e.clone()).as_ref() != Some(&e),
        None => all.remove(id).is_some(),
      };
      if !changed {
        return Ok(());
      }
      if let Some(dir) = self.file.parent() {
        fs::create_dir_all(dir).await?;
      }
      write_atomic(&self.file, serde_json::to_string_pretty(&all)?.as_bytes(), Some(0o600)).await
    })
    .await
  }
}
