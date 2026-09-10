// Normalized shape of the transcript: host-side normalize.ts reduces ACP session/update into these blocks; the webview only understands these

import type { MsgKey } from './i18n/keys';

// Built-in devin / grok; custom ids can be added in acpira.agents
export type AgentId = string;

export interface AgentInfo {
  id: AgentId;
  name: string;
  // Goes through the account layer (multiple logins can be stored and switched); agents without it rely on their own CLI's login
  accounts?: boolean;
  // Read-only official CLI account; independent of the selected model's provider.
  localAccount?: LocalAccountInfo;
  // An executable was detected locally; false greys it out in the menu, undefined means not probed yet
  available?: boolean;
  // How to get the CLI when none was found: the vendor's one-line install for this platform and its docs page
  install?: AgentInstall;
}

export interface AgentInstall {
  // A shell line to run in a terminal; absent when the vendor publishes no one-liner for this platform
  command?: string;
  docs?: string;
}

// One allowance window as the vendor reports it (Devin: daily / weekly; a plan may hide either). remaining is 0..1, resetsAt an ISO timestamp
export interface QuotaWindow {
  id: string;
  remaining: number;
  resetsAt?: string;
}

// Usage allowance of an account, fetched from the vendor by the account provider; in memory only, refreshed after turns and on demand
export interface AccountQuota {
  windows: QuotaWindow[];
  fetchedAt: string;
}

export interface LocalAccountInfo {
  label: string;
  detail?: string;
  status: 'loading' | 'ready' | 'login_required' | 'expired' | 'unavailable';
  quota?: AccountQuota;
}

// Account: one login identity of an agent. Only metadata here; secrets live in the host vault and never enter the webview
export interface AccountInfo {
  id: string;
  agent: AgentId;
  // Primary label (email) and secondary label (plan · name)
  label: string;
  detail?: string;
  addedAt: string;
  lastUsedAt?: string;
  quota?: AccountQuota;
}

// Session-level options: modes come from modes.availableModes of session/new; the rest (model / reasoning level / …) are select-type configOptions —
// whatever ACP provides is what we show, we don't invent our own
export interface SessionOption {
  id: string;
  name: string;
  description?: string;
  // ACP group identity survives flattening; it need not represent a provider.
  group?: { id: string; name: string };
  // Verified by an agent adapter, never inferred from a display name.
  source?: { id: string; name: string; kind: 'official' | 'custom' };
}

export interface ConfigControl {
  // id of the configOption; required when calling set_config_option
  id: string;
  name: string;
  // ACP's semantic category: model / thought_level / model_config / custom; only affects ordering and icon, not correctness
  category?: string;
  options: SessionOption[];
  value?: string;
}

export interface SessionControls {
  modes: SessionOption[];
  modeId?: string;
  // If modes come from a category=mode configOption, record its id
  modeConfigId?: string;
  options: ConfigControl[];
}

// Slash commands from available_commands_update. `input.hint` is the agent's unstructured hint for the text
// typed after the command name — a display hint, never a schema to validate against
export interface SlashCommand {
  name: string;
  description: string;
  input?: { hint: string };
}

export interface AuthMethodInfo {
  id: string;
  name: string;
  description?: string;
}

// Session lifecycle: starting (spawn process / initialize / session.new) → ready; login failure → auth_required;
// old sessions that can't be resumed → readonly; process died → error
export type SessionStatus = 'starting' | 'ready' | 'auth_required' | 'readonly' | 'error' | 'closed';

// Aligned with ACP ToolKind
export type ToolKind = 'read' | 'edit' | 'delete' | 'move' | 'search' | 'execute' | 'think' | 'fetch' | 'switch_mode' | 'other';
export type ToolStatus = 'pending' | 'in_progress' | 'completed' | 'failed' | 'cancelled';

export type ToolContent =
  | { type: 'text'; text: string }
  | { type: 'diff'; lines: DiffLine[]; source?: DiffSource }
  | { type: 'list'; items: string[] };

export interface DiffSource {
  path: string;
  oldText: string;
  newText: string;
}

export interface DiffLine {
  kind: 'hunk' | 'add' | 'del' | 'ctx';
  text: string;
  // Source positions are captured before unchanged context is collapsed.
  oldLine?: number;
  newLine?: number;
}

export interface ToolCallBlock {
  type: 'tool_call';
  id: string;
  kind: ToolKind;
  verb: string;
  // Set when the verb comes from the tool's identity (e.g. a todo-list tool filed under kind "think") rather than its ACP kind; renders in the current UI locale
  verbKey?: MsgKey;
  target?: string;
  targetMono?: boolean;
  // Preserve ACP file references independently of the compact heading target.
  locations?: { path: string; line?: number }[];
  // Explicit read parameters; kept separately because later ACP locations omit the range.
  readRange?: { path: string; start: number; end?: number };
  status: ToolStatus;
  // A command the agent parked in the background (Devin's exec past its timeout): it stays in_progress until the process exits,
  // so it must not pass for what the agent is doing right now
  background?: boolean;
  // Observed execution time; initial pending approvals and replay-only tools have no timer.
  startedAt?: number;
  endedAt?: number;
  meta?: string;
  diffStat?: { add: number; del: number };
  // Confirmed todo-tool output, separate from the standard live plan update.
  todoEntries?: PlanEntry[];
  content?: ToolContent;
}

export interface ThoughtBlock {
  type: 'thought';
  text: string;
  // Legacy observation interval until the next block, not actual reasoning duration.
  // Retained for stored transcripts; the UI must not display it as a thought timer.
  startedAt?: number;
  durationSec?: number;
  streaming?: boolean;
}

export type PlanStatus = 'pending' | 'in_progress' | 'completed';
export type PlanPriority = 'high' | 'medium' | 'low';

export interface PlanEntry {
  title: string;
  status: PlanStatus;
  // ACP PlanEntry.priority; absent on entries persisted before it was kept
  priority?: PlanPriority;
}

export interface PlanBlock {
  type: 'plan';
  entries: PlanEntry[];
  // This turn contained a real list change, even if it ended at an earlier state.
  // Absent on legacy records that also stored unchanged session snapshots.
  changed?: true;
}

export interface TextBlock {
  type: 'text';
  markdown: string;
  streaming?: boolean;
}

export type PermissionKind = 'allow_once' | 'allow_always' | 'reject_once' | 'reject_always';

export interface PermissionBlock {
  type: 'permission';
  id: string;
  title: string;
  command?: string;
  description?: string;
  planId?: string;
  options: { id: string; label: string; kind: PermissionKind }[];
}

// A saved implementation plan, separate from the live to-do list.
export interface PlanDocumentBlock {
  type: 'plan_document';
  id: string;
  title: string;
  markdown: string;
  path?: string;
  toolCallId: string;
  approvalToolCallId?: string;
  status: 'draft' | 'ready' | 'approved' | 'rejected' | 'executing';
}

// Context compaction (ACP compaction_update): a single status line
export type CompactionStatus = 'in_progress' | 'completed' | 'failed' | 'cancelled';

export interface CompactionBlock {
  type: 'compaction';
  id: string;
  status: CompactionStatus;
}

// A structured question the agent put to the user: one property of an elicitation form (Devin / Kimi `elicitation/create`, mode form)
// or one entry of Grok's `_x.ai/ask_user_question`. `id` is the key the answer goes back under (form property / Grok question text)
export interface QuestionOption {
  id: string;
  label: string;
  description?: string;
}

export type QuestionKind = 'single' | 'multiple' | 'text';

export interface Question {
  id: string;
  // Short header (Devin / Kimi `header`); the question itself is `text`
  title?: string;
  text: string;
  kind: QuestionKind;
  // Empty for a free-text question
  options: QuestionOption[];
  // A free-text answer is accepted alongside the options (Grok always; Devin when it says allowOther)
  other?: boolean;
  // A text answer must be a number (schema type number / integer)
  numeric?: boolean;
  required?: boolean;
}

// The answer of one question: the chosen option id, the free text, or the chosen ids of a multi-select
export type QuestionAnswer = string | string[];
export type QuestionAnswers = Record<string, QuestionAnswer>;

// How the card was closed: `answered` sent the answers, `skipped` told the agent to go on with what it has, `cancelled` is the turn ending first
export type QuestionOutcome = 'answered' | 'skipped' | 'cancelled';

// The question card. Pending (no outcome) it is pinned above the composer; resolved it stays in the message as the record of what was asked and picked
export interface QuestionBlock {
  type: 'question';
  id: string;
  toolCallId?: string;
  // The form's own message, when it says more than the questions do
  message?: string;
  questions: Question[];
  outcome?: QuestionOutcome;
  answers?: QuestionAnswers;
}

export type AgentBlock = ThoughtBlock | PlanBlock | ToolCallBlock | TextBlock | PermissionBlock | CompactionBlock | PlanDocumentBlock | QuestionBlock;

// What the composer attaches to a prompt before the host has seen it: images and dropped text carry their payload (base64 / text),
// files carry a URI (Explorer drag / @ mention) that the host resolves — image files become `image`, everything else stays a link
export type Draft =
  | { kind: 'image'; mimeType: string; data: string; name?: string }
  | { kind: 'text'; name: string; text: string }
  | { kind: 'file'; uri: string; name: string };

// Attachment as persisted on a user turn. Images and dropped text live in the session's blob directory (the turn keeps only the file name,
// the webview loads it via blobBase; absent when the write failed — the prompt still went out, only the preview is gone); files are paths the agent reads by itself (sent as resource_link)
export type Attachment =
  | { kind: 'image'; blob?: string; mimeType: string; name?: string }
  | { kind: 'text'; blob?: string; name: string }
  | { kind: 'file'; uri: string; name: string };

export interface UserTurn {
  role: 'user';
  id?: string;
  text: string;
  attachments?: Attachment[];
  // The exact ACP selections used when this message was sent; older records omit it.
  settings?: TurnSettings;
  // Retrying a failed edited turn must rebuild its preceding context as well.
  edited?: true;
  // Internal execution instruction; the plan card represents it in the UI.
  planId?: string;
  // Sent automatically by Acpira (/compact over threshold); rendered as a note line instead of a bubble
  auto?: boolean;
}

export interface TurnSettings {
  modeId?: string;
  config: Record<string, string>;
}

// How an agent turn ended. `end_turn` and `cancelled` are the normal outcomes; the rest stopped the turn short and are shown to the user:
// the ACP stopReasons max_tokens / max_turn_requests / refusal, plus `error` when session/prompt itself failed (details in AgentTurn.error)
export type TurnStop = 'end_turn' | 'max_tokens' | 'max_turn_requests' | 'refusal' | 'cancelled' | 'error';

export interface TurnError {
  message: string;
  // JSON-RPC error code when the failure was a protocol error
  code?: number;
  // The vendor's error kind (Devin: data['cognition.ai/errorKind']) and whether it says the same request may succeed if retried
  kind?: string;
  retryable?: boolean;
}

export interface AgentTurn {
  role: 'agent';
  blocks: AgentBlock[];
  // Wall-clock prompt duration, including tools and permission waits. Absent on older transcripts.
  startedAt?: number;
  endedAt?: number;
  // What it's currently doing (inferred from usage and tool states); empty when the turn ends
  activity?: { kind: ToolKind; label: string };
  // How the turn ended; absent while it runs (and on turns persisted before this field existed)
  stop?: TurnStop;
  error?: TurnError;
}

export type Turn = UserTurn | AgentTurn;

export interface SessionSummary {
  id: string;
  title: string;
  agent: AgentId;
  accountId?: string;
  // The project the session belongs to: the workspace folder it was opened in (also the agent's working directory)
  cwd: string;
  // ISO timestamp; the webview formats it itself
  updatedAt: string;
  pinned?: boolean;
  state?: 'working' | 'waiting' | 'unread' | 'error';
}

export interface Usage {
  used: number;
  size: number;
  cost?: number;
}

// A prompt waiting for the running turn to end. Its attachments are staged when it is queued (blobs on disk), so the webview shows
// them the way it shows a sent turn's; the id addresses it for editing / removal while it waits
export interface QueuedPrompt {
  id: string;
  text: string;
  attachments: Attachment[];
  sending?: boolean;
}

// Everything a session looks like to the webview: the host pushes the whole thing on every change (the transcript is small, not worth diffing)
export interface SessionView {
  id: string;
  agent: AgentId;
  // Bound account (only agents on the account layer have one); a session uses a single account from start to finish
  accountId?: string;
  title: string;
  cwd: string;
  status: SessionStatus;
  error?: string;
  authMethods?: AuthMethodInfo[];
  turns: Turn[];
  running: boolean;
  // Monotonic snapshot id for this process. Turns are mutated in place and postMessage clones
  // asynchronously, so an older running:true payload can arrive after settle; the webview drops a
  // lower or equal rev. Not persisted.
  rev?: number;
  controls: SessionControls;
  usage?: Usage;
  commands: SlashCommand[];
  // Prompts sent while a turn was in progress, in send order; the first goes out when the turn ends
  queued?: QueuedPrompt[];
  createdAt: string;
  updatedAt: string;
}
