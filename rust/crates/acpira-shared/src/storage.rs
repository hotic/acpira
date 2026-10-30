//! The settings page's Storage tab (mirror of packages/shared/storage.ts): what the data directory holds, by owner, and the
//! cleanups that are reviewed before they run. Sizes are file lengths in bytes, added up

use serde::{Deserialize, Serialize};

use crate::transcript::AgentId;

/// The usage bar's segments, in bar order
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageSegmentKind {
  /// The session records
  Sessions,
  /// Their blob folders
  Attachments,
  /// The built-in agent's logs a record still drives
  AgentLogs,
  /// The leftovers the Clean leftovers review lists
  Reclaimable,
  /// Everything else under the data root: size and place only
  Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSegment {
  pub kind: StorageSegmentKind,
  pub bytes: u64,
  /// Sessions for the first three, items for reclaimable, entries for other
  pub count: u64,
}

/// One entry of the other segment: a top-level name under the data root and its size
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageEntry {
  pub name: String,
  pub path: String,
  pub bytes: u64,
}

/// Why a session cannot be picked for deletion or slimming
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageBlock {
  Pinned,
  /// A turn is running in this engine
  Running,
  /// Another window's engine drives it (its lease is held elsewhere, or this engine only mirrors it)
  Elsewhere,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSession {
  pub id: String,
  pub title: String,
  pub agent: AgentId,
  pub cwd: String,
  pub updated_at: String,
  /// Record + attachments + the built-in agent's log
  pub bytes: u64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub subagents: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub blocked: Option<StorageBlock>,
  /// Open in this engine (a tab or the sidebar shows it): its transcript in memory would write the texts back, so it
  /// cannot be slimmed until closed; deleting it is fine
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub open: bool,
  /// What slimming would free; only with the slim detail
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub slim: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageResidueKind {
  /// A built-in agent log no record drives
  AgentLog,
  /// The log of an engine that is not running
  EngineLog,
  /// An attachment no record names
  Blob,
  /// A record that does not parse (the list never shows it)
  Unreadable,
  /// A Shared tab backup older than 30 days that nothing can put back
  Backup,
  /// The old globalStorage copy a migration left behind
  Legacy,
  /// What an uninstalled plugin left (its moved-aside folder, frozen versions, compiled output) and versions or output
  /// nothing in effect uses
  PluginFiles,
  /// The data of an uninstalled plugin
  PluginData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageResidue {
  /// Stable across scans: the kind and the path
  pub id: String,
  pub kind: StorageResidueKind,
  pub path: String,
  pub bytes: u64,
  pub modified_at: String,
  /// Ticked when the review opens
  pub preselected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageDetail {
  Sessions,
  Residue,
  Slim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageView {
  pub root: String,
  pub total: u64,
  pub segments: Vec<StorageSegment>,
  /// Largest first
  pub other: Vec<StorageEntry>,
  pub scanned_at: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub sessions: Option<Vec<StorageSession>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub residue: Option<Vec<StorageResidue>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum StorageAction {
  /// Soft deletion, the same as the history list's: restorable for 30 s
  DeleteSessions { ids: Vec<String> },
  RestoreSessions { ids: Vec<String> },
  CleanResidue { ids: Vec<String> },
  /// Drop whole-file diff texts from the records; cannot be undone
  SlimSessions { ids: Vec<String> },
  /// The data root, or a path the view listed, in the OS file manager
  Reveal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
  },
}

impl StorageAction {
  /// The TS `kind` tag, what `StorageOutcome.action` carries
  pub fn kind(&self) -> &'static str {
    match self {
      StorageAction::DeleteSessions { .. } => "deleteSessions",
      StorageAction::RestoreSessions { .. } => "restoreSessions",
      StorageAction::CleanResidue { .. } => "cleanResidue",
      StorageAction::SlimSessions { .. } => "slimSessions",
      StorageAction::Reveal { .. } => "reveal",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSkip {
  pub id: String,
  /// A translated sentence
  pub reason: String,
}

/// What an action did: the ids it carried out, the ones it passed over and why, bytes freed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageOutcome {
  pub action: String,
  pub done: Vec<String>,
  pub skipped: Vec<StorageSkip>,
  pub freed: u64,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn actions_and_views_keep_the_ts_spelling() {
    let a: StorageAction = serde_json::from_str(r#"{"kind":"cleanResidue","ids":["agentLog:/x"]}"#).unwrap();
    assert_eq!(a, StorageAction::CleanResidue { ids: vec!["agentLog:/x".into()] });
    assert_eq!(a.kind(), "cleanResidue");
    let r: StorageAction = serde_json::from_str(r#"{"kind":"reveal"}"#).unwrap();
    assert_eq!(r, StorageAction::Reveal { path: None });
    let s = StorageSegment { kind: StorageSegmentKind::AgentLogs, bytes: 1, count: 1 };
    assert_eq!(serde_json::to_value(&s).unwrap()["kind"], "agentLogs");
    let res = StorageResidue { id: "i".into(), kind: StorageResidueKind::EngineLog, path: "/p".into(), bytes: 2, modified_at: "t".into(), preselected: true };
    let v = serde_json::to_value(&res).unwrap();
    assert_eq!(v["kind"], "engineLog");
    assert_eq!(v["modifiedAt"], "t");
  }
}
