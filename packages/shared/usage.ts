import type { AgentId } from './transcript';

// The usage page (mirror of rust/crates/acpira-shared/src/usage.rs): what the agents did across every session, hour by
// hour. The engine keeps one entry per turn it has seen (`store/usage_index.rs`) and sends the hours folded per session
// and model; the page folds them again into local days and hours, models, agents and projects

// A session the rows point into; `cwd` is its project
export interface UsageSession {
  agent: AgentId;
  cwd: string;
}

// A model the rows point into: the id the turns name and the family the picker shows for it
export interface UsageModel {
  id: string;
  name: string;
}

// One session's turns in one model within one clock hour. Zero fields are left out
export interface UsageRow {
  // The hour's start, Unix milliseconds (whole UTC hours)
  at: number;
  // Indices into UsageView.sessions / UsageView.models (no model: nothing names it)
  session: number;
  model?: number;
  // The whole prompt (cache included) plus the output, as a turn's own card counts them
  tokens?: number;
  input?: number;
  output?: number;
  cachedRead?: number;
  // Agent turns that started in the hour, and the user's own messages among them (automatic turns left out)
  turns?: number;
  prompts?: number;
  // Working time: from the first streamed output to the turn's end, background waits left out; and the longest turn's
  workMs?: number;
  longestMs?: number;
  tools?: number;
  // Lines the agents' edits added and removed (edit rows' diffStat; edits made through a shell are not seen)
  added?: number;
  removed?: number;
}

export interface UsageView {
  sessions: UsageSession[];
  models: UsageModel[];
  // Ordered by `at`
  rows: UsageRow[];
}
