//! `~/.acpira/shared-links.json`: the links Acpira made and what it moved aside for them. Shared by every window and
//! IDE sidecar, so every change is a read-modify-write under the file lock, written tmp + rename

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::store::file_lock::{with_file_lock, write_atomic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
  /// A symlink (or junction / hard link on Windows) at `path` pointing to `target`
  Link,
  /// An `@<target>` import line at the top of the file at `path`
  Import,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
  pub path: String,
  pub target: String,
  pub kind: EntryKind,
  /// Where whatever sat at `path` before was moved
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub backup: Option<String>,
  /// The repository whose `info/exclude` got a line for `path`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub repo: Option<String>,
  /// The project root for a project-level entry (a link inside the project, or its CLAUDE.md import); None = user level
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub project: Option<String>,
  /// The agent the link / import is for, so turning the agent off can take its entries back
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<String>,
}

impl Entry {
  /// Entries written before `project` existed carry only `repo`
  pub fn project_root(&self) -> Option<&str> {
    self.project.as_deref().or(self.repo.as_deref())
  }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
  /// User level: missing links (other than skipped ones) are created on their own
  #[serde(default)]
  pub auto: bool,
  /// User-level link points left out on purpose in the link panel
  #[serde(default)]
  pub skipped: Vec<String>,
  /// Project level is off: Claude's project links are not made on their own
  #[serde(default)]
  pub project_manual: bool,
  /// Projects whose links stay out of `info/exclude` (committed for the team)
  #[serde(default)]
  pub shared_projects: Vec<String>,
  /// Project roots that got links, so they are kept up to date too
  #[serde(default)]
  pub projects: Vec<String>,
  #[serde(default)]
  pub entries: Vec<Entry>,
}

impl Ledger {
  pub fn find(&self, path: &Path) -> Option<&Entry> {
    let p = path.to_string_lossy();
    self.entries.iter().find(|e| e.path == p)
  }

  /// Record (or replace) the entry for its path
  pub fn put(&mut self, e: Entry) {
    self.entries.retain(|x| x.path != e.path);
    self.entries.push(e);
  }

  pub fn drop_path(&mut self, path: &Path) {
    let p = path.to_string_lossy();
    self.entries.retain(|e| e.path != p);
  }

  pub fn is_skipped(&self, path: &Path) -> bool {
    let p = path.to_string_lossy();
    self.skipped.iter().any(|x| *x == p)
  }

  pub fn is_shared_project(&self, root: &Path) -> bool {
    let r = root.to_string_lossy();
    self.shared_projects.iter().any(|x| *x == r)
  }

  pub fn add_project(&mut self, root: &Path) {
    let r = root.to_string_lossy().into_owned();
    if !self.projects.contains(&r) {
      self.projects.push(r);
    }
  }
}

pub struct LedgerFile {
  path: PathBuf,
}

impl LedgerFile {
  pub fn new(data_root: &Path) -> Self {
    LedgerFile { path: data_root.join("shared-links.json") }
  }

  /// A missing or unreadable file is an empty ledger
  pub async fn read(&self) -> Ledger {
    match tokio::fs::read(&self.path).await {
      Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
      Err(_) => Ledger::default(),
    }
  }

  /// Read, change, write back under the lock; the closure's value is returned
  pub async fn update<T: Send + 'static>(&self, f: impl FnOnce(&mut Ledger) -> T + Send) -> Result<T> {
    let path = self.path.clone();
    // The lock file sits next to the ledger, so its directory has to exist first
    if let Some(dir) = path.parent() {
      tokio::fs::create_dir_all(dir).await?;
    }
    with_file_lock(&path.clone(), || async move {
      let mut ledger = match tokio::fs::read(&path).await {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
        Err(_) => Ledger::default(),
      };
      let out = f(&mut ledger);
      write_atomic(&path, &serde_json::to_vec_pretty(&ledger)?, None).await?;
      Ok(out)
    })
    .await
  }
}
