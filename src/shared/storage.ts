import type { AgentId } from './transcript';

// The settings page's Storage tab (mirror of rust/crates/acpira-shared/src/storage.rs): what the data directory holds,
// by owner, and the cleanups that are reviewed before they run. Sizes are file lengths in bytes, added up

// The usage bar's segments, in bar order. sessions: the records; attachments: their blob folders; agentLogs: the
// built-in agent's logs a record still drives; reclaimable: the leftovers the Clean leftovers review lists; other:
// everything else under the data root (exports, backups, caches, config, installed agents), size and place only
export type StorageSegmentKind = 'sessions' | 'attachments' | 'agentLogs' | 'reclaimable' | 'other';
export const STORAGE_SEGMENTS: readonly StorageSegmentKind[] = ['sessions', 'attachments', 'agentLogs', 'reclaimable', 'other'];

export interface StorageSegment {
  kind: StorageSegmentKind;
  bytes: number;
  // Sessions for the first three, items for reclaimable, entries for other
  count: number;
}

// One entry of the other segment: a top-level name under the data root and its size
export interface StorageEntry {
  name: string;
  path: string;
  bytes: number;
}

// Why a session cannot be picked for deletion or slimming: pinned by the user, a turn running here, or another window's
// engine drives it (its lease is held elsewhere, or this engine only mirrors it)
export type StorageBlock = 'pinned' | 'running' | 'elsewhere';

export interface StorageSession {
  id: string;
  title: string;
  agent: AgentId;
  cwd: string;
  updatedAt: string;
  // Record + attachments + the built-in agent's log
  bytes: number;
  // Subagent runs kept inside the record
  subagents?: number;
  blocked?: StorageBlock;
  // What slimming would free; only with the slim detail, absent when the record was not estimated
  slim?: number;
}

// agentLog: a built-in agent log no record drives; engineLog: the log of an engine that is not running; blob: an
// attachment no record names; unreadable: a record that does not parse (hidden from the list); backup: a Shared tab
// backup older than 30 days nothing can put back; legacy: the old globalStorage copy a migration left behind
export type StorageResidueKind = 'agentLog' | 'engineLog' | 'blob' | 'unreadable' | 'backup' | 'legacy';
export const STORAGE_RESIDUE_KINDS: readonly StorageResidueKind[] = ['agentLog', 'engineLog', 'blob', 'unreadable', 'backup', 'legacy'];

export interface StorageResidue {
  // Stable across scans: the kind and the path
  id: string;
  kind: StorageResidueKind;
  path: string;
  bytes: number;
  modifiedAt: string;
  // Ticked when the review opens: only for leftovers nothing can miss (orphaned agent logs, stopped engines' logs)
  preselected: boolean;
}

// What a storage request adds to the overview: the session list (old sessions and slim reviews), the leftovers, the
// slimming estimates (with sessions)
export type StorageDetail = 'sessions' | 'residue' | 'slim';

export interface StorageView {
  root: string;
  total: number;
  segments: StorageSegment[];
  // Largest first
  other: StorageEntry[];
  scannedAt: string;
  sessions?: StorageSession[];
  residue?: StorageResidue[];
}

export type StorageAction =
  // Soft deletion, the same as the history list's: restorable for 30 s with restoreSessions
  | { kind: 'deleteSessions'; ids: string[] }
  | { kind: 'restoreSessions'; ids: string[] }
  | { kind: 'cleanResidue'; ids: string[] }
  // Drop whole-file diff texts from the records; cannot be undone
  | { kind: 'slimSessions'; ids: string[] }
  // The data root, or a path the view listed, in the OS file manager
  | { kind: 'reveal'; path?: string };

// What an action did: the ids it carried out, the ones it passed over and why (a translated sentence), bytes freed
export interface StorageOutcome {
  action: StorageAction['kind'];
  done: string[];
  skipped: { id: string; reason: string }[];
  freed: number;
}
