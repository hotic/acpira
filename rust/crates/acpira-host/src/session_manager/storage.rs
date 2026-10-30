//! The Storage tab's session side: the view with this engine's view of each session (running, pinned, held by another
//! engine), and the cleanups that touch sessions. Deleting goes through `delete_session` one by one, so the leases,
//! the 30 s undo window, the index sync and the viewers moving on all stay the history list's

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

use anyhow::Result;
use serde::Deserialize;
use serde::de::IgnoredAny;

use acpira_shared::storage::{StorageAction, StorageBlock, StorageDetail, StorageOutcome, StorageSession, StorageSkip, StorageView};

use super::SessionManager;
use crate::i18n::t;
use crate::store::storage::{Residue, Scan, ScanInput, clean, clean_plugins, scan};

/// What the Storage tab's reply carries back
pub struct StorageReply {
  pub view: StorageView,
  pub outcome: Option<StorageOutcome>,
  pub error: Option<String>,
  /// The shell reveals this in the OS file manager
  pub reveal: Option<PathBuf>,
}

type Stamp = (u64, Option<SystemTime>);

/// What reading a whole record tells, kept per file stamp: the review lists re-open often and a record only changes
/// when its session does
#[derive(Clone, Copy, Default)]
struct Facts {
  subagents: u32,
  /// Bytes slimming would take out; None until asked for
  slim: Option<u64>,
}

static FACTS: LazyLock<parking_lot::Mutex<HashMap<PathBuf, (Stamp, Facts)>>> = LazyLock::new(Default::default);

fn stamp(path: &Path) -> Option<Stamp> {
  std::fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()))
}

/// The record's subagent count (the transcript is tokenized, not built) and, when `slim`, what dropping its diff
/// texts would free: the record as it is minus the record written the way a page gets it
fn facts(path: &Path, slim: bool) -> Facts {
  let Some(stamp) = stamp(path) else { return Facts::default() };
  if let Some((s, f)) = FACTS.lock().get(path).copied()
    && s == stamp
    && (!slim || f.slim.is_some())
  {
    return f;
  }
  let Ok(raw) = std::fs::read(path) else { return Facts::default() };
  #[derive(Deserialize)]
  struct Subagents {
    #[serde(default)]
    subagents: Option<Vec<IgnoredAny>>,
  }
  let subagents = serde_json::from_slice::<Subagents>(&raw).ok().and_then(|s| s.subagents).map_or(0, |s| s.len() as u32);
  let slim = slim.then(|| slim_gain(&raw)).flatten();
  let f = Facts { subagents, slim };
  FACTS.lock().insert(path.to_path_buf(), (stamp, f));
  f
}

/// Bytes a record would lose without its whole-file diff texts; None when it does not parse
pub(crate) fn slim_gain(raw: &[u8]) -> Option<u64> {
  let record: crate::store::record::SessionRecord = serde_json::from_slice(raw).ok()?;
  let slim = acpira_shared::transcript::encode_for_view(|| serde_json::to_vec(&record)).ok()?;
  let full = serde_json::to_vec(&record).ok()?;
  Some(full.len().saturating_sub(slim.len()) as u64)
}

impl SessionManager {
  /// ACPIRA_HOME, where the leases live too
  pub(crate) fn data_root(&self) -> PathBuf {
    self.deps.lease_root.clone()
  }

  async fn scan_storage(self: &Arc<Self>) -> Scan {
    // Other windows' sessions count too: reconcile with the directory first
    self.refresh_index().await;
    let input = ScanInput {
      root: self.data_root(),
      sessions_dir: self.deps.store.dir().to_path_buf(),
      sessions: self.state.lock().index.clone(),
      now: SystemTime::now(),
    };
    let fallback = (input.root.clone(), input.sessions_dir.clone());
    tokio::task::spawn_blocking(move || scan(&input))
      .await
      .unwrap_or_else(|_| scan(&ScanInput { root: fallback.0, sessions_dir: fallback.1, sessions: vec![], now: SystemTime::now() }))
  }

  /// Why this session cannot be picked right now
  fn storage_block(&self, id: &str, pinned: bool) -> Option<StorageBlock> {
    if self.live(id).is_some_and(|s| s.is_running()) {
      Some(StorageBlock::Running)
    } else if self.mirror_of(id).is_some() || self.leases.held_elsewhere(id) {
      Some(StorageBlock::Elsewhere)
    } else if pinned {
      Some(StorageBlock::Pinned)
    } else {
      None
    }
  }

  async fn storage_details(self: &Arc<Self>, scan: Scan, detail: &[StorageDetail]) -> StorageView {
    let Scan { mut view, usage, residue } = scan;
    if detail.contains(&StorageDetail::Sessions) {
      let index: Vec<_> = self.state.lock().index.iter().filter(|s| s.external.is_none()).cloned().collect();
      let slim = detail.contains(&StorageDetail::Slim);
      let dir = self.deps.store.dir().to_path_buf();
      let ids: Vec<String> = index.iter().map(|s| s.id.clone()).collect();
      let facts: Vec<Facts> = tokio::task::spawn_blocking(move || ids.iter().map(|id| facts(&dir.join(format!("{id}.json")), slim)).collect())
        .await
        .unwrap_or_default();
      let rows = index
        .into_iter()
        .zip(facts.into_iter().chain(std::iter::repeat(Facts::default())))
        .map(|(s, f)| StorageSession {
          bytes: usage.get(&s.id).map(|u| u.total()).unwrap_or(0),
          subagents: (f.subagents > 0).then_some(f.subagents),
          blocked: self.storage_block(&s.id, s.pinned.unwrap_or(false)),
          open: self.live(&s.id).is_some(),
          slim: if slim { f.slim } else { None },
          id: s.id,
          title: s.title,
          agent: s.agent,
          cwd: s.cwd,
          updated_at: s.updated_at,
        })
        .collect();
      view.sessions = Some(rows);
    }
    if detail.contains(&StorageDetail::Residue) {
      view.residue = Some(residue.into_iter().map(|r| r.item).collect());
    }
    view
  }

  /// The Storage tab: the data root by owner, plus what the open review needs
  pub async fn storage_view(self: &Arc<Self>, detail: &[StorageDetail]) -> StorageView {
    let scan = self.scan_storage().await;
    self.storage_details(scan, detail).await
  }

  pub async fn storage_action(self: &Arc<Self>, action: StorageAction, detail: &[StorageDetail]) -> StorageReply {
    let kind = action.kind().to_owned();
    let mut outcome = StorageOutcome { action: kind, done: vec![], skipped: vec![], freed: 0 };
    let mut reveal = None;
    let result: Result<()> = async {
      match action {
        StorageAction::DeleteSessions { ids } => self.storage_delete(&ids, &mut outcome).await,
        StorageAction::RestoreSessions { ids } => {
          for id in ids {
            match self.restore_session(&id).await {
              Ok(()) => outcome.done.push(id),
              Err(e) => outcome.skipped.push(StorageSkip { id, reason: format!("{e:#}") }),
            }
          }
        }
        StorageAction::CleanResidue { ids } => self.storage_clean(&ids, &mut outcome).await,
        StorageAction::SlimSessions { ids } => self.storage_slim(&ids, &mut outcome).await,
        StorageAction::Reveal { path } => reveal = Some(self.storage_reveal(path).await?),
      }
      Ok(())
    }
    .await;
    let view = self.storage_view(detail).await;
    StorageReply { view, outcome: Some(outcome), error: result.err().map(|e| format!("{e:#}")), reveal }
  }

  async fn storage_delete(self: &Arc<Self>, ids: &[String], outcome: &mut StorageOutcome) {
    let sizes: HashMap<String, u64> = {
      let scan = self.scan_storage().await;
      scan.usage.into_iter().map(|(id, u)| (id, u.total())).collect()
    };
    for id in ids {
      let pinned = self.state.lock().index.iter().find(|s| &s.id == id).map(|s| s.pinned.unwrap_or(false));
      let Some(pinned) = pinned else {
        outcome.skipped.push(StorageSkip { id: id.clone(), reason: t("host.storage.gone") });
        continue;
      };
      if let Some(block) = self.storage_block(id, pinned) {
        outcome.skipped.push(StorageSkip { id: id.clone(), reason: block_reason(block) });
        continue;
      }
      match self.delete_session(id).await {
        Ok(()) => {
          outcome.freed += sizes.get(id).copied().unwrap_or(0);
          outcome.done.push(id.clone());
        }
        Err(e) => outcome.skipped.push(StorageSkip { id: id.clone(), reason: format!("{e:#}") }),
      }
    }
  }

  async fn storage_clean(self: &Arc<Self>, ids: &[String], outcome: &mut StorageOutcome) {
    // What the review showed may have changed: only what a fresh scan still finds is removed
    let scan = self.scan_storage().await;
    let root = self.data_root();
    let mut files = vec![];
    for id in ids {
      let Some(r) = scan.residue.iter().find(|r| &r.item.id == id) else {
        outcome.skipped.push(StorageSkip { id: id.clone(), reason: t("host.storage.gone") });
        continue;
      };
      if let Some(session) = &r.session {
        // The store remembers unreadable records; removing through it forgets them too
        self.deps.store.remove(session).await;
        outcome.freed += r.item.bytes;
        outcome.done.push(id.clone());
      } else {
        files.push(r.clone());
      }
    }
    use acpira_shared::storage::StorageResidueKind as Kind;
    let (plugin, other): (Vec<Residue>, Vec<Residue>) = files.iter().cloned().partition(|r| matches!(r.item.kind, Kind::PluginFiles | Kind::PluginData));
    let root2 = root.clone();
    let (freed, mut failed) = tokio::task::spawn_blocking(move || clean(&root2, &other.iter().collect::<Vec<_>>())).await.unwrap_or_else(|e| {
      (0, files.iter().filter(|r| !matches!(r.item.kind, Kind::PluginFiles | Kind::PluginData)).map(|r| (r.item.id.clone(), e.to_string())).collect())
    });
    outcome.freed += freed;
    let mut kept = vec![];
    if !plugin.is_empty() {
      // Under the plugin state's lock, checked again: an enable since the scan may need a version again
      let plugins = root.join(crate::plugins::DIR);
      let (root2, ids) = (root.clone(), plugin.iter().map(|r| r.item.id.clone()).collect::<Vec<_>>());
      let done = crate::plugins::store::with_state_lock(&plugins, move || {
        let (freed, failed, kept) = clean_plugins(&root2, &plugin.iter().collect::<Vec<_>>());
        // Versions gone, the shared files only they linked go too
        (freed + crate::plugins::store::prune_blobs(&root2.join(crate::plugins::DIR)), failed, kept)
      })
      .await;
      match done {
        Ok((freed, f, k)) => {
          outcome.freed += freed;
          failed.extend(f);
          kept = k;
        }
        Err(e) => failed.extend(ids.into_iter().map(|id| (id, format!("{e:#}")))),
      }
    }
    for r in &files {
      if kept.contains(&r.item.id) {
        outcome.skipped.push(StorageSkip { id: r.item.id.clone(), reason: t("host.storage.gone") });
        continue;
      }
      match failed.iter().find(|(id, _)| id == &r.item.id) {
        Some((id, e)) => {
          self.log(&format!("storage: removing {id} failed: {e}"));
          outcome.skipped.push(StorageSkip { id: id.clone(), reason: e.clone() });
        }
        None => outcome.done.push(r.item.id.clone()),
      }
    }
  }

  /// Rewrite each record without its whole-file diff texts, the way a page gets it (`encode_for_view`); the diff lines
  /// stay. Pinned sessions may be slimmed (nothing visible goes); a session running, held elsewhere or open here is not
  async fn storage_slim(self: &Arc<Self>, ids: &[String], outcome: &mut StorageOutcome) {
    for id in ids {
      match self.slim_record(id).await {
        Ok(freed) => {
          outcome.freed += freed;
          outcome.done.push(id.clone());
        }
        Err(e) => outcome.skipped.push(StorageSkip { id: id.clone(), reason: format!("{e:#}") }),
      }
    }
  }

  async fn slim_record(self: &Arc<Self>, id: &str) -> Result<u64> {
    match self.storage_block(id, false) {
      Some(b) => return Err(anyhow::anyhow!(block_reason(b))),
      None if self.live(id).is_some() => return Err(anyhow::anyhow!(t("host.storage.open"))),
      None => {}
    }
    let _hold = self.hold_record(id).await?;
    let path = self.deps.store.dir().join(format!("{id}.json"));
    let before = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let record = self.deps.store.load_detailed(id).await.map_err(|e| anyhow::anyhow!(e.detail()))?;
    let slim = acpira_shared::transcript::encode_for_view(|| serde_json::to_vec(&record))?;
    // Nothing to drop: leave the file (and its stamp) as it is
    if slim.len() >= serde_json::to_vec(&record)?.len() {
      return Ok(0);
    }
    // Read back, every source is `omitted` and serializes as such from now on
    let slimmed: crate::store::record::SessionRecord = serde_json::from_slice(&slim)?;
    self.deps.store.flush(Arc::new(slimmed)).await?;
    let after = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(before);
    Ok(before.saturating_sub(after))
  }

  /// The data root, or a place inside it (or a leftover the scan lists): never an arbitrary path from the page
  pub async fn storage_reveal(self: &Arc<Self>, path: Option<String>) -> Result<PathBuf> {
    let root = self.data_root();
    let Some(p) = path.map(PathBuf::from) else { return Ok(root) };
    let base = std::fs::canonicalize(&root).unwrap_or(root.clone());
    let inside = std::fs::canonicalize(&p).is_ok_and(|c| c.starts_with(&base));
    if inside {
      return Ok(p);
    }
    let scan = self.scan_storage().await;
    if scan.residue.iter().any(|r| Path::new(&r.item.path) == p) {
      return Ok(p);
    }
    Err(anyhow::anyhow!(t("host.storage.outside")))
  }
}

fn block_reason(b: StorageBlock) -> String {
  t(match b {
    StorageBlock::Pinned => "host.storage.pinned",
    StorageBlock::Running => "host.storage.running",
    StorageBlock::Elsewhere => "host.sessionBusyElsewhere",
  })
}
