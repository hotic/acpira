import { basename, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import type * as acp from '@agentclientprotocol/sdk';
import type { MsgKey } from '@shared/i18n';
import { t } from '../i18n';
import { MAX_OUT_IMAGE_BYTES, TOOL_OUTPUT_MAX } from '../limits';
import { base64Bytes, imageMimeOf } from '@shared/attachments';
import type {
  AgentBlock, AgentTurn, AsyncTaskInfo, AsyncTaskState, CompactionBlock, CompactionStatus, ConfigControl, NoticeBlock, PlanPriority, PlanStatus, SessionControls, SessionOption, SlashCommand, ToolCallBlock, ToolContent, ToolKind, ToolStatus, Turn, TurnError, Usage,
} from '@shared/transcript';
import { diffLines } from './diff';
import { isTodoTool, todoEntries } from '@shared/todoTools';
import { lastPlanSnapshot, samePlanEntries } from './planSnapshots';
import type { AgentRuntimeInfo } from '@shared/inventory';
import { failureOf, type SessionFailure } from './sessionFailure';
import type { AsyncTaskEvent } from './subagents/wire';

export { diffLines };

// What the agent told us in initialize: name / version and the MCP transports it can take (the settings page's facts card)
export function runtimeInfoOf(init: acp.InitializeResponse): AgentRuntimeInfo {
  const mcp = init.agentCapabilities?.mcpCapabilities;
  return { name: init.agentInfo?.name, version: init.agentInfo?.version, mcp: mcp ? { http: !!mcp.http, sse: !!mcp.sse } : undefined };
}

// Normalize ACP session/update into transcript blocks. Pure functions + in-place mutation of the Turn array; AcpSession pushes to the webview

export interface NormalizeState {
  turns: Turn[];
  controls: SessionControls;
  usage?: Usage;
  commands: SlashCommand[];
  title?: string;
  // Start time of a thought block; durationSec is computed when it ends
  thoughtStartedAt?: number;
  // Background shell id → command, so a later wait on that shell (Devin's get_output) can name the command it waits for
  shells?: Record<string, string>;
  // Tool id → { path, content }: the file contents an edit tool reported in rawInput (OpenCode's write sends them there
  // and its completion carries only a text receipt), kept so a new-file completion can render as an all-add diff
  pendingWrites?: Record<string, { path: string; content: string }>;
  // Parks an agent-emitted image payload in the session blob store and returns its blob name; absent on pure-normalization
  // paths (tests, export replay) where image content degrades to a `[image]` note instead
  saveImage?: (data: string, mimeType: string) => string | undefined;
  // Same for an image the agent references by local path (codex-acp's view_image emits a resource_link to the file);
  // a readable regular file becomes a blob, anything else falls back to the link's text rendering
  saveImageFile?: (absPath: string) => string | undefined;
  imageSeq?: number;
  // AIR asyncTasks the owning session announced, whether or not a tool row hosts them yet
  tasks?: Map<string, AsyncTaskInfo>;
  // toolCallId → asyncTaskId: a task named a tool row that has not arrived yet; the next tool_call /
  // tool_call_update for that id adopts the task instead of duplicating it
  taskByTool?: Map<string, string>;
  // Validation failures on extension metadata (sessionFailure, asyncTasks) are worth one log line
  log?: (line: string) => void;
}

export function emptyState(): NormalizeState {
  return { turns: [], controls: { modes: [], options: [] }, commands: [] };
}

// The current assistant turn; open one if there is none
function currentAgentTurn(s: NormalizeState): AgentTurn {
  const last = s.turns[s.turns.length - 1];
  if (last?.role === 'agent') return last;
  const t: AgentTurn = { role: 'agent', blocks: [] };
  s.turns.push(t);
  return t;
}

function lastBlock(t: AgentTurn): AgentBlock | undefined {
  return t.blocks[t.blocks.length - 1];
}

// Seal off the streaming body / thought when the block changes
function sealStreaming(s: NormalizeState, t: AgentTurn, except?: AgentBlock['type']) {
  for (const b of t.blocks) {
    if (b.type === except) continue;
    if (b.type === 'thought' && b.streaming) {
      b.streaming = false;
      const from = b.startedAt ?? s.thoughtStartedAt;
      if (from) b.durationSec = Math.max(1, Math.round((Date.now() - from) / 1000));
      s.thoughtStartedAt = undefined;
    }
    if (b.type === 'text' && b.streaming) b.streaming = false;
  }
}

function textOf(c: acp.ContentBlock): string {
  if (c.type === 'text') return c.text;
  if (c.type === 'resource_link') return c.uri;
  if (c.type === 'resource') return 'text' in c.resource ? c.resource.text : c.resource.uri;
  return `[${c.type}]`;
}

// One update comes in, mutate state. Returns whether there is a UI-visible change
export function applyUpdate(s: NormalizeState, u: acp.SessionUpdate): boolean {
  switch (u.sessionUpdate) {
    case 'user_message_chunk': {
      // Appears only during load / resume replay; consecutive chunks merge into the same entry
      const last = s.turns[s.turns.length - 1];
      const text = textOf(u.content);
      if (last?.role === 'user' && (last as { _open?: boolean })._open) last.text += text;
      else s.turns.push(Object.assign({ role: 'user' as const, text }, { _open: true }));
      return true;
    }
    case 'agent_message_chunk': {
      closeUserTurn(s);
      const t = currentAgentTurn(s);
      // An image chunk is its own block: it closes the text run so later prose opens a fresh block
      const img = imageContent(u.content, s);
      if (img?.type === 'image') {
        sealStreaming(s, t);
        s.imageSeq = (s.imageSeq ?? 0) + 1;
        t.blocks.push({ type: 'image', id: `img-${s.imageSeq}`, mimeType: img.mimeType, blob: img.blob, uri: img.uri });
        return true;
      }
      const text = img?.type === 'text' ? img.text : textOf(u.content);
      const last = lastBlock(t);
      if (last?.type === 'text' && last.streaming) last.markdown += text;
      else { sealStreaming(s, t); t.blocks.push({ type: 'text', markdown: text, streaming: true }); }
      return true;
    }
    case 'agent_thought_chunk': {
      const text = textOf(u.content);
      // Empty deltas carry no reasoning body and must not create a timed disclosure.
      if (!text) return false;
      closeUserTurn(s);
      const t = currentAgentTurn(s);
      const last = lastBlock(t);
      if (last?.type === 'thought' && last.streaming) last.text += text;
      else {
        if (!text.trim()) return false;
        sealStreaming(s, t); s.thoughtStartedAt = Date.now(); t.blocks.push({ type: 'thought', text, startedAt: s.thoughtStartedAt, streaming: true });
      }
      return true;
    }
    case 'tool_call': {
      closeUserTurn(s);
      const t = currentAgentTurn(s);
      sealStreaming(s, t);
      const existing = findTool(s, u.toolCallId);
      const block = existing ?? toolBlock(u, s);
      if (existing) mergeTool(existing, u, s);
      else t.blocks.push(block);
      linkAsyncTask(s, block);
      timeTool(block, t.startedAt !== undefined && !t.stop && t.blocks.includes(block));
      return true;
    }
    case 'tool_call_update': {
      const t = currentAgentTurn(s);
      const existing = findTool(s, u.toolCallId);
      const block = existing ?? toolBlock({
        toolCallId: u.toolCallId, title: u.title ?? '', kind: u.kind ?? undefined,
        status: u.status ?? undefined, content: u.content ?? undefined,
        locations: u.locations ?? undefined, rawInput: u.rawInput, _meta: u._meta,
      }, s);
      if (existing) mergeTool(existing, u, s);
      else { sealStreaming(s, t); t.blocks.push(block); }
      linkAsyncTask(s, block);
      timeTool(block, t.startedAt !== undefined && !t.stop && t.blocks.includes(block));
      return true;
    }
    case 'plan': {
      const entries = u.entries.map(e => ({ title: e.content, status: e.status as PlanStatus, priority: e.priority as PlanPriority }));
      // Legacy ACP plan notifications are session snapshots. Grok repeats the
      // completed list after ordinary replies; a repeat must not open a history
      // row, seal streaming prose, or resurrect the completed composer dock.
      const previous = lastPlanSnapshot(s.turns);
      if (previous && samePlanEntries(previous.entries, entries)) return false;
      closeUserTurn(s);
      const t = currentAgentTurn(s);
      const plan = t.blocks.find(b => b.type === 'plan');
      if (plan) { plan.entries = entries; plan.changed = true; }
      else { sealStreaming(s, t); t.blocks.push({ type: 'plan', entries, changed: true }); }
      return true;
    }
    case 'plan_update':
    case 'plan_removed':
      return false;
    case 'usage_update': {
      s.usage = { used: u.used, size: u.size, cost: u.cost?.amount ?? undefined };
      // The context snapshot belongs to the turn it followed: the agent turn that just ended or is still streaming.
      // Devin sends one per model call mid-turn (the last overwrite is the end-of-turn snapshot); Kimi's late idle
      // notification lands on the finished last turn, which is where it belongs
      const last = s.turns[s.turns.length - 1];
      if (last?.role === 'agent') last.usage = { ...last.usage, context: { used: u.used, size: u.size } };
      return true;
    }
    case 'available_commands_update':
      // The list replaces the previous one wholesale: an empty update clears the menu
      s.commands = u.availableCommands.map(c => ({ name: c.name, description: c.description, ...(c.input?.hint ? { input: { hint: c.input.hint } } : {}) }));
      return true;
    case 'current_mode_update':
      s.controls.modeId = u.currentModeId;
      return true;
    case 'config_option_update':
      applyConfigOptions(s.controls, u.configOptions);
      return true;
    case 'session_info_update': {
      if (u.title) s.title = u.title;
      // AIR sessionFailure rides this kind: the payload is only in _meta, and the same update may also carry a title
      const failure = failureOf(u._meta, s.log);
      if (failure) applySessionFailure(s, failure);
      return true;
    }
    case 'compaction_update': {
      closeUserTurn(s);
      const t = currentAgentTurn(s);
      const status = compactionStatus(u.status);
      const existing = findCompaction(s, u.compactionId);
      if (existing) existing.status = status;
      else { sealStreaming(s, t); t.blocks.push({ type: 'compaction', id: u.compactionId, status }); }
      return true;
    }
    case 'compaction_summary_chunk':
      return false;
    default:
      return false;
  }
}

function compactionStatus(v: string): CompactionStatus {
  return v === 'completed' || v === 'failed' || v === 'cancelled' ? v : 'in_progress';
}

function findCompaction(s: NormalizeState, id: string): CompactionBlock | undefined {
  for (let i = s.turns.length - 1; i >= 0; i--) {
    const t = s.turns[i];
    if (t?.role !== 'agent') continue;
    const b = t.blocks.find(b => b.type === 'compaction' && b.id === id);
    if (b) return b as CompactionBlock;
  }
  return undefined;
}

function closeUserTurn(s: NormalizeState) {
  const last = s.turns[s.turns.length - 1] as (Turn & { _open?: boolean }) | undefined;
  if (last?.role === 'user') delete last._open;
}

// Turn ended: seal all streaming blocks, record how it ended; tools still running are marked per stopReason
export function endTurn(s: NormalizeState, stopReason: acp.StopReason) {
  const t = s.turns[s.turns.length - 1];
  if (t?.role !== 'agent') return;
  sealStreaming(s, t);
  // Replay-only turns have no live start time; never invent a duration for them.
  if (t.startedAt !== undefined) t.endedAt ??= Date.now();
  t.activity = undefined;
  t.stop = stopReason;
  for (const b of t.blocks) {
    // A live AIR async task owns its row: the parent turn ending says nothing about it (the task's own
    // state updates settle the row). A disconnected one (observation unknown) is swept like any other
    if (b.type === 'tool_call' && !asyncTaskLive(b) && (b.status === 'in_progress' || b.status === 'pending')) {
      b.status = stopReason === 'cancelled' ? 'cancelled' : 'failed';
      timeTool(b, false);
    }
  }
}

// session/prompt itself failed: wrap up like a cancellation (nothing more is coming) and keep the error on the turn so the UI can show it
export function failTurn(s: NormalizeState, error: TurnError) {
  endTurn(s, 'cancelled');
  const t = s.turns[s.turns.length - 1];
  if (t?.role !== 'agent') return;
  t.stop = 'error';
  t.error = error;
}

// An AIR sessionFailure gets exactly one row per id in the whole session transcript: a higher
// revision rewrites it in place (a retry warning becomes the terminal error under the same id), a
// same or lower revision is a duplicate to ignore, and a different id is a new row even when the
// text happens to match
export function applySessionFailure(s: NormalizeState, f: SessionFailure): boolean {
  const existing = findNotice(s, f.id);
  if (existing) {
    if (existing.revision >= f.revision) return false;
    existing.revision = f.revision;
    existing.category = f.category;
    existing.severity = f.severity;
    existing.title = f.title;
    if (f.details === undefined) delete existing.details; else existing.details = f.details;
    existing.actions = f.actions;
    return true;
  }
  const notice: NoticeBlock = {
    type: 'notice', id: f.id, revision: f.revision, category: f.category, severity: f.severity,
    title: f.title, ...(f.details ? { details: f.details } : {}), actions: f.actions,
  };
  const last = s.turns[s.turns.length - 1];
  if (last?.role === 'agent') last.blocks.push(notice);
  else s.turns.push({ role: 'agent', blocks: [notice], stop: 'end_turn' });
  return true;
}

function findNotice(s: NormalizeState, id: string): NoticeBlock | undefined {
  for (let i = s.turns.length - 1; i >= 0; i--) {
    const t = s.turns[i];
    if (t?.role !== 'agent') continue;
    const b = t.blocks.find(b => b.type === 'notice' && b.id === id);
    if (b) return b as NoticeBlock;
  }
  return undefined;
}

// AIR asyncTasks: the task record lives on the owning session's state (tasks map); a tool row hosts it
// once toolCallId resolves, and a row the adapter asked for without a toolCallId gets synthesized.
// Terminal states are final — a stale 'running' must not resurrect a settled row — with one exception:
// claude-agent-acp closes a task that left the SDK's background level as a best-effort 'stopped' and
// corrects it when the authoritative edge follows (both arrive in the same millisecond on the wire)
const ASYNC_TERMINAL: readonly AsyncTaskState[] = ['completed', 'failed', 'stopped'];
const taskTerminal = (st: AsyncTaskState) => ASYNC_TERMINAL.includes(st);
const taskStateMoves = (from: AsyncTaskState, to: AsyncTaskState) =>
  !taskTerminal(from) || (from === 'stopped' && (to === 'completed' || to === 'failed'));
const taskStatus = (st: AsyncTaskState): ToolStatus => st === 'completed' ? 'completed' : st === 'failed' ? 'failed' : st === 'stopped' ? 'cancelled' : 'in_progress';

// The task is still observable and owns its row across turn ends; 'unknown' observation means the
// host lost the process and the row is swept like any other
export function asyncTaskLive(b: ToolCallBlock): boolean {
  return b.asyncTask !== undefined && b.observation !== 'unknown' && !taskTerminal(b.asyncTask.state);
}

function blockByTask(s: NormalizeState, taskId: string): ToolCallBlock | undefined {
  for (const t of s.turns) {
    if (t.role !== 'agent') continue;
    const b = t.blocks.find(b => b.type === 'tool_call' && b.asyncTask?.id === taskId);
    if (b) return b as ToolCallBlock;
  }
  return undefined;
}

function dropTaskRow(s: NormalizeState, block: ToolCallBlock) {
  for (const t of s.turns) {
    if (t.role !== 'agent') continue;
    const i = t.blocks.indexOf(block);
    if (i >= 0) { t.blocks.splice(i, 1); return; }
  }
}

// The named row adopts the task; a synthesized placeholder hosting it first comes out, so the task
// never shows twice (spawned without toolCallId, then a later update names the row)
function attachAsyncTask(s: NormalizeState, block: ToolCallBlock, info: AsyncTaskInfo) {
  const prev = blockByTask(s, info.id);
  if (prev && prev !== block) {
    if (prev.id === `async:${info.id}`) dropTaskRow(s, prev);
    else delete prev.asyncTask;
  }
  block.asyncTask = info;
  block.background = true;
}

// A task event named a toolCallId before the row existed; the row's arrival adopts the parked task
function linkAsyncTask(s: NormalizeState, block: ToolCallBlock) {
  const taskId = s.taskByTool?.get(block.id);
  if (taskId === undefined) return;
  s.taskByTool!.delete(block.id);
  const info = s.tasks?.get(taskId);
  if (info === undefined) return;
  attachAsyncTask(s, block, info);
  block.status = taskStatus(info.state);
}

// One asyncTasks event on the owning session's transcript. Returns whether the transcript changed;
// a task with no row and no showInTranscript is still recorded in s.tasks so a later toolCallId can link it
export function applyAsyncTask(s: NormalizeState, e: AsyncTaskEvent): boolean {
  const tasks = (s.tasks ??= new Map());
  let info = tasks.get(e.asyncTaskId);
  if (!info) {
    info = { id: e.asyncTaskId, state: 'running', canStop: false };
    tasks.set(e.asyncTaskId, info);
  }
  if (e.name !== undefined) info.name = e.name;
  if (e.taskType !== undefined) info.taskType = e.taskType;
  if (e.description !== undefined) info.description = e.description;
  if (e.summary !== undefined) info.summary = e.summary;
  if (e.lastToolName !== undefined) info.lastToolName = e.lastToolName;
  if (e.outputFilePath !== undefined) info.outputFilePath = e.outputFilePath;
  if (e.usage) info.usage = { ...info.usage, ...e.usage };
  if (e.canStop !== undefined) info.canStop = e.canStop;
  if (e.event === 'state' && e.state) {
    if (taskStateMoves(info.state, e.state)) info.state = e.state;
    if (taskTerminal(info.state)) delete info.stopRequested;
  }
  const named = e.toolCallId ? findTool(s, e.toolCallId) : undefined;
  if (e.toolCallId) {
    if (named) s.taskByTool?.delete(e.toolCallId);
    else (s.taskByTool ??= new Map()).set(e.toolCallId, e.asyncTaskId);
  }
  const block = named ?? blockByTask(s, e.asyncTaskId);
  if (block) {
    if (named) attachAsyncTask(s, block, info);
    // A live wire event is observation itself: a restored 'unknown' marker clears once the task speaks
    delete block.observation;
    block.status = taskStatus(info.state);
    if (taskTerminal(info.state)) block.endedAt ??= Date.now();
    else timeTool(block, true);
    return true;
  }
  if (e.showInTranscript === true) {
    const kind: ToolKind = e.taskType === 'shell' ? 'execute' : 'other';
    const row: ToolCallBlock = { type: 'tool_call', id: `async:${e.asyncTaskId}`, kind, verb: verbOf(kind),
      target: e.name ?? e.description, status: taskStatus(info.state), background: true, startedAt: Date.now(), asyncTask: info };
    const last = s.turns[s.turns.length - 1];
    if (last?.role === 'agent') last.blocks.push(row);
    else s.turns.push({ role: 'agent', blocks: [row], stop: 'end_turn' });
    return true;
  }
  s.log?.(`async task ${e.asyncTaskId}: no transcript row (no toolCallId match, showInTranscript off)`);
  return false;
}

// The process carrying the tasks is gone (exit, reconnect, dispose): live tasks lose their observer —
// the last known state stays honest, the row loses its stop control, and the next sweep settles it
export function disconnectAsyncTasks(s: NormalizeState) {
  for (const t of s.turns) {
    if (t.role !== 'agent') continue;
    for (const b of t.blocks) {
      if (b.type !== 'tool_call' || !b.asyncTask || taskTerminal(b.asyncTask.state)) continue;
      b.observation = 'unknown';
      b.asyncTask.canStop = false;
      delete b.asyncTask.stopRequested;
    }
  }
}

// A session/load replay (OpenCode streams the whole history as live-looking chunks, with no stop reasons and no usage) leaves
// every block looking mid-stream: seal it — open user input closes, streaming flags come off, tools that never reported an
// outcome are cancelled, turns end as end_turn. No timestamps are invented for replayed turns.
export function sealReplay(s: NormalizeState) {
  closeUserTurn(s);
  for (const t of s.turns) {
    if (t.role !== 'agent') { delete (t as { _open?: boolean })._open; continue; }
    t.activity = undefined;
    t.stop ??= 'end_turn';
    for (const b of t.blocks) {
      if (b.type === 'text' || b.type === 'thought') delete b.streaming;
      else if (b.type === 'tool_call' && (b.status === 'pending' || b.status === 'in_progress')) b.status = 'cancelled';
    }
  }
}

// Build the controls from the session/new / resume response
export function initControls(controls: SessionControls, modes?: acp.SessionModeState | null, configOptions?: acp.SessionConfigOption[] | null) {
  controls.modes = (modes?.availableModes ?? []).map(m => ({ id: m.id, name: m.name, description: m.description ?? undefined, kind: kindOf(m._meta) }));
  controls.modeId = modes?.currentModeId;
  if (configOptions) applyConfigOptions(controls, configOptions);
}

// configOptions: a select with category=mode is treated purely as modes — it fills in when modes is empty, and is a duplicate when modes is already present (Kimi sends both);
// in neither case does it enter the control list. The remaining selects are ordered model → thought_level → model_config → others, preserving the agent's order within each class.
// A boolean configOption (advertised via clientCapabilities.session.configOptions.boolean) becomes a control with a synthetic Off/On pair,
// so the string-valued paths (chips, turn settings, hidden lists) keep working; only the wire request carries `type: 'boolean'`
const CATEGORY_ORDER = ['model', 'thought_level', 'model_config'];

const BOOL_OPTIONS: SessionOption[] = [{ id: 'false', name: 'Off' }, { id: 'true', name: 'On' }];

export function applyConfigOptions(controls: SessionControls, options: acp.SessionConfigOption[]) {
  const mode = options.find(o => o.type === 'select' && o.category === 'mode');
  if (mode && mode.type === 'select' && (controls.modes.length === 0 || controls.modeConfigId === mode.id)) {
    controls.modes = flattenSelect(mode.options);
    controls.modeId = mode.currentValue;
    controls.modeConfigId = mode.id;
  }
  const rank = (c: ConfigControl) => { const i = CATEGORY_ORDER.indexOf(c.category ?? ''); return i < 0 ? CATEGORY_ORDER.length : i; };
  controls.options = options
    .filter(o => (o.type === 'select' || o.type === 'boolean') && o.category !== 'mode')
    .map((o): ConfigControl => o.type === 'boolean'
      ? { id: o.id, name: o.name, category: o.category ?? undefined, type: 'boolean', options: BOOL_OPTIONS, value: String(o.currentValue) }
      : { id: o.id, name: o.name, category: o.category ?? undefined, options: flattenSelect(o.options), value: o.currentValue })
    .sort((a, b) => rank(a) - rank(b));
}

// A user pick → the session/set_config_option payload. Boolean controls take `type: 'boolean'` and a real boolean —
// sending the string 'false' would be truthy to agents that parse it loosely
export function configOptionSetValue(control: ConfigControl | undefined, value: string): { type: 'boolean'; value: boolean } | { value: string } {
  return control?.type === 'boolean' ? { type: 'boolean', value: value === 'true' } : { value };
}

// `_meta.kind` on modes and select options (codex / claude: standard / plan / auto_review / full_access); display only
function kindOf(meta: unknown): string | undefined {
  const k = (meta as { kind?: unknown } | null | undefined)?.kind;
  return typeof k === 'string' && k ? k : undefined;
}

function flattenSelect(opts: acp.SessionConfigSelectOptions): SessionOption[] {
  const out: SessionOption[] = [];
  for (const o of opts) {
    if ('group' in o) for (const x of o.options) out.push({ id: x.value, name: x.name, description: x.description ?? o.name, group: { id: o.group, name: o.name }, kind: kindOf(x._meta) });
    else out.push({ id: o.value, name: o.name, description: o.description ?? undefined, kind: kindOf(o._meta) });
  }
  return out;
}

function findTool(s: NormalizeState, id: string): ToolCallBlock | undefined {
  for (let i = s.turns.length - 1; i >= 0; i--) {
    const t = s.turns[i];
    if (t?.role !== 'agent') continue;
    const b = t.blocks.find(b => b.type === 'tool_call' && b.id === id);
    if (b) return b as ToolCallBlock;
  }
  return undefined;
}

const VERB_KEY: Record<ToolKind, MsgKey> = {
  read: 'verb.read', edit: 'verb.edit', delete: 'verb.delete', move: 'verb.move', search: 'verb.search',
  execute: 'verb.execute', think: 'verb.think', fetch: 'verb.fetch', switch_mode: 'verb.switch_mode', other: 'verb.other',
};
const verbOf = (kind: ToolKind): string => t(VERB_KEY[kind]);

// Some agents file their todo-list tool under kind "think"/"other"; recognize it by name and give it its own verb
const TODO_TITLE = /^todo([_\s-]?(write|update|read|list))?$/i;
// The ask-user-question tool by its names on the wire: Grok `ask_user_question` / `Ask 2 questions`, Devin `Asked user 2 questions …`, Kimi `AskUserQuestion` / `Asking user questions`
const ASK_TITLE = /^(ask_?user_?questions?|ask(ed|ing)?\s+(the\s+)?(user\s+)?(\d+\s+)?questions?\b)/i;
// Background-shell tools (Devin): `get_output` arrives titled `Read shell` and `kill_shell` as `Kill shell`, both without a kind and addressing
// the parked exec by `rawInput.shell_id`. get_output blocks for up to its timeout — shown as a generic tool call that looks like a hang,
// so each gets its own verb and the command it acts on as the target (`write_to_process` already comes as kind execute with a usable title)
const SHELL_TOOLS: Record<string, MsgKey> = { get_output: 'verb.wait', kill_shell: 'verb.kill' };
const SHELL_TITLES: [RegExp, MsgKey][] = [
  [/^(get_output|read(ing)?\s+shell(\s+output)?)$/i, 'verb.wait'],
  [/^(kill_shell|kill(ing)?\s+shell)$/i, 'verb.kill'],
];

function shellVerb(meta: Record<string, unknown> | undefined, title: string | null | undefined): MsgKey | undefined {
  const name = meta?.['cognition.ai/inferenceToolName'];
  if (typeof name === 'string' && SHELL_TOOLS[name]) return SHELL_TOOLS[name];
  if (!title) return undefined;
  return SHELL_TITLES.find(([re]) => re.test(title.trim()))?.[1];
}

// Well-known tool names pin down the kind when the agent omitted it or used a grab-bag kind.
// Specific kinds (read/edit/…) always win — only "other" and "think" are treated as unreliable.
const KIND_BY_TITLE: [RegExp, ToolKind][] = [
  [/^(read|open|view|cat)(_[a-z]+)*$/i, 'read'],
  [/^(write|edit|create|patch|apply_?patch|str_?replace|insert)(_[a-z]+)*$/i, 'edit'],
  [/^(list|ls|dir|glob|grep|find|search)(_[a-z]+)*$/i, 'search'],
  [/^(web_?search|google|bing)(_[a-z]+)*$/i, 'search'],
  [/^(bash|shell|terminal|exec|execute|run|command)(_[a-z]+)*$/i, 'execute'],
  [/^(web_?fetch|fetch|browse|curl)(_[a-z]+)*$/i, 'fetch'],
];

function inferKind(title: string | null | undefined): ToolKind | undefined {
  if (!title) return undefined;
  const name = title.trim();
  for (const [re, kind] of KIND_BY_TITLE) if (re.test(name)) return kind;
  return undefined;
}

// Sparse updates retain the first observed start and the first terminal timestamp.
function timeTool(b: ToolCallBlock, live: boolean) {
  if (live && b.status === 'in_progress' && b.endedAt === undefined) b.startedAt ??= Date.now();
  if (b.startedAt !== undefined && b.status !== 'in_progress' && b.status !== 'pending') b.endedAt ??= Date.now();
}

function toolBlock(tc: acp.ToolCall, s?: NormalizeState): ToolCallBlock {
  const b: ToolCallBlock = { type: 'tool_call', id: tc.toolCallId, kind: tc.kind ?? 'other', verb: verbOf(tc.kind ?? 'other'), status: tc.status ?? 'pending' };
  mergeTool(b, tc, s);
  return b;
}

// Fields of tool_call and tool_call_update are all optional; overwrite only the ones provided
function mergeTool(b: ToolCallBlock, u: acp.ToolCall | acp.ToolCallUpdate, s?: NormalizeState) {
  const meta = (u._meta ?? undefined) as Record<string, unknown> | undefined;
  const raw = u.rawInput as Record<string, unknown> | undefined;
  if (u.kind) { b.kind = u.kind; b.verb = verbOf(u.kind); }
  if (u.title && TODO_TITLE.test(u.title.trim())) { b.verbKey = 'verb.todo'; b.verb = t('verb.todo'); }
  const toolName = (meta?.['x.ai/tool'] as { name?: unknown } | undefined)?.name ?? meta?.['cognition.ai/inferenceToolName'];
  if (typeof toolName === 'string' && TODO_TITLE.test(toolName)) { b.verbKey = 'verb.todo'; b.verb = t('verb.todo'); }
  if (u.title && ASK_TITLE.test(u.title.trim())) { b.verbKey = 'verb.ask'; b.verb = t('verb.ask'); }
  const shell = shellVerb(meta, u.title);
  if (shell) { b.verbKey = shell; b.verb = t(shell); }
  if (!b.verbKey && (b.kind === 'other' || b.kind === 'think')) {
    const inferred = inferKind(u.title ?? undefined);
    if (inferred) { b.kind = inferred; b.verb = verbOf(inferred); }
  }
  // A row an async task owns takes its status from the task's state updates, not from tool_call_update
  // statuses the adapter keeps streaming (codex reports the backgrounded shell's outcome while the task runs)
  if (u.status && !b.asyncTask) b.status = u.status;
  // Devin parks a command past its exec timeout: the call stays in_progress while the process runs, and later waits address it by shell id
  if (meta?.['cognition.ai/background'] === true) {
    b.background = true;
    const shellId = meta['cognition.ai/backgroundShellId'];
    const command = meta['cognition.ai/backgroundCommand'];
    if (s && typeof shellId === 'string') (s.shells ??= {})[shellId] = typeof command === 'string' && command ? command : b.target ?? shellId;
  }
  // AIR asyncTasks: codex marks the originating tool row right before async_task_spawned names it
  const airTasks = (meta?.jetbrains as { air?: { asyncTasks?: { backgrounded?: unknown } } } | undefined)?.air?.asyncTasks;
  if (airTasks?.backgrounded === true) b.background = true;
  if (u.locations) b.locations = u.locations.map(l => ({ path: l.path, ...(l.line != null ? { line: l.line } : {}) }));
  // Some ACP tools supply a path in rawInput instead of locations.
  if (b.kind === 'read' && raw) {
    const range = readRangeFromRaw(raw, b.locations);
    if (range) b.readRange = range;
  }
  if (!b.locations?.length && (b.kind === 'read' || b.kind === 'edit' || b.kind === 'delete' || b.kind === 'move')) {
    const path = pathFromRaw(raw);
    if (path) b.locations = [{ path }];
  }
  // OpenCode's write sends the file contents only inside rawInput.content; park them so a completion that proves the
  // file is new (rawOutput.metadata.exists === false) can render the write as an all-add diff
  if (s && b.kind === 'edit' && typeof raw?.content === 'string') {
    const path = pathFromRaw(raw) ?? b.locations?.[0]?.path;
    if (path) (s.pendingWrites ??= {})[b.id] = { path, content: raw.content };
  }
  // A target inferred from the title is only a fallback while there is no target yet; don't overwrite what rawInput / locations provided.
  // A todo / ask / shell tool's title is just its own name — redundant next to the verb, so drop it (the question card carries the questions,
  // a shell tool names the background command it acts on, or its shell id until that command is known).
  const named = b.verbKey === 'verb.todo' || b.verbKey === 'verb.ask' || b.verbKey === 'verb.wait' || b.verbKey === 'verb.kill';
  const target = b.verbKey === 'verb.wait' || b.verbKey === 'verb.kill' ? shellTarget(raw, s) : pickTarget(u, b.kind);
  if (target && !(named && target.fromTitle) && (!target.fromTitle || !b.target)) { b.target = target.text; b.targetMono = target.mono; }
  // OpenCode's write completes with a text receipt only: the parked rawInput content plus rawOutput.metadata.exists ===
  // false prove the file is new, so render the write as an all-add diff with the receipt behind it. Without that
  // evidence (an overwrite, or another agent's write tool) nothing is synthesized — a fabricated diff would lie
  const pendingWrite = s && u.status === 'completed' && b.content?.type !== 'diff' ? s.pendingWrites?.[b.id] : undefined;
  if (pendingWrite) {
    delete s!.pendingWrites![b.id];
    if ((u.rawOutput as { metadata?: { exists?: unknown } } | null | undefined)?.metadata?.exists === false) {
      const diff: Extract<ToolContent, { type: 'diff' }> = {
        type: 'diff', lines: diffLines('', pendingWrite.content),
        source: { path: pendingWrite.path, oldText: '', newText: pendingWrite.content },
      };
      const receipts = (u.content?.length ? toolContents(u.content, s) : []).filter((x): x is Extract<ToolContent, { type: 'text' }> => x.type === 'text' && !!x.text);
      b.content = diff;
      if (receipts.length) b.contents = [diff, ...receipts]; else delete b.contents;
      b.diffStat = { add: diff.lines.filter(l => l.kind === 'add').length, del: 0 };
    }
  }
  if (u.content?.length) {
    const list = toolContents(u.content, s);
    // The primary item: a diff wins over plain text, otherwise the wire order's first
    const c = list.find(x => x.type === 'diff') ?? list[0];
    // Kimi sends the edit diff before execution, then a plain success receipt.
    // Preserve the diff on success; failures must still expose their error output.
    const keepDiff = b.kind === 'edit' && b.status === 'completed' && b.content?.type === 'diff' && list.length === 1 && c?.type === 'text';
    if (c && !keepDiff) {
      b.content = c;
      if (list.length > 1) b.contents = list; else delete b.contents;
      const diffs = list.filter((x): x is Extract<ToolContent, { type: 'diff' }> => x.type === 'diff');
      b.diffStat = diffs.length
        ? { add: diffs.reduce((n, d) => n + d.lines.filter(l => l.kind === 'add').length, 0), del: diffs.reduce((n, d) => n + d.lines.filter(l => l.kind === 'del').length, 0) }
        : undefined;
    }
  }
  // pi-acp and the claude/codex adapters manage terminals themselves and stream output through _meta
  // (terminal_output / terminal_output_delta, then terminal_exit)
  const termOut = (meta?.terminal_output ?? meta?.terminal_output_delta) as { data?: unknown; terminal_id?: unknown } | undefined;
  const terminalOutput = typeof termOut?.data === 'string' ? termOut.data : undefined;
  if (terminalOutput) {
    const prev = b.content?.type === 'text' ? b.content.text : '';
    // A terminal item created the not-wired note before the agent's stream proved it wrong — replace, don't append
    const note = typeof termOut?.terminal_id === 'string' ? t('host.terminalNotWired', { id: termOut.terminal_id }) : undefined;
    const base = note !== undefined && prev === note ? '' : prev;
    b.content = { type: 'text', text: (base + terminalOutput).slice(0, TOOL_OUTPUT_MAX) };
  }
  const exitCode = (meta?.terminal_exit as { exit_code?: unknown } | undefined)?.exit_code;
  if (typeof exitCode === 'number' && exitCode !== 0) {
    const prev = b.content?.type === 'text' ? b.content.text : '';
    if (!prev.endsWith(`exit code ${exitCode}`)) {
      b.content = { type: 'text', text: appendExitCode(prev, exitCode).slice(0, TOOL_OUTPUT_MAX) };
    }
  }
  // A terminal item the agent cannot stream through _meta expects a client-side terminal (which we don't provide):
  // say so instead of leaving the body empty. terminal_info / terminal_output(_delta) / terminal_exit mean the agent wired it up itself
  const agentWiredTerminal = meta?.terminal_info !== undefined || meta?.terminal_output !== undefined || meta?.terminal_output_delta !== undefined || meta?.terminal_exit !== undefined;
  if (!b.content && !agentWiredTerminal) {
    const term = u.content?.find(c => c.type === 'terminal');
    if (term?.type === 'terminal') b.content = { type: 'text', text: t('host.terminalNotWired', { id: term.terminalId }) };
  }
  if (!b.content && u.rawOutput !== undefined && u.rawOutput !== null) {
    // codex-acp's completion receipt { formatted_output, exit_code } renders as plain output, not pretty JSON
    const formatted = formattedOutput(u.rawOutput);
    if (formatted) {
      const text = formatted.exitCode ? appendExitCode(formatted.text, formatted.exitCode) : formatted.text;
      if (text.trim()) b.content = { type: 'text', text: text.slice(0, TOOL_OUTPUT_MAX) };
    } else {
      const text = typeof u.rawOutput === 'string' ? u.rawOutput : JSON.stringify(u.rawOutput, null, 2);
      if (text.trim()) b.content = { type: 'text', text: text.slice(0, TOOL_OUTPUT_MAX) };
    }
  }
  if (isTodoTool(b) && b.status === 'completed') {
    // Parse the full wire result before the generic output preview's size limit.
    const entries = todoEntries(u.rawOutput) ?? (b.content?.type === 'text' ? todoEntries(b.content.text) : undefined);
    if (entries !== undefined) b.todoEntries = entries;
  }
}

// OpenCode's session/request_permission embeds a low-fidelity copy of the call — kind 'other', the parent dir as title,
// file + parent dir as locations, rawInput { filepath, parentDir }. Merging it verbatim downgrades the block the
// original tool_call established (kind → 'other', target → the dir; the card reads "Use tool"). AGENTS.md: "Permission
// requests usually carry only toolCall.title" — with a block already there, take only what the request can improve:
// the status, a specific kind onto an unclassified block, and the path fields only while the block has no file of its
// own (rawInput preferred — pathFromRaw names the file — since the request's locations often list the directory too).
// Without a block (Devin / Kimi plan approvals arrive with no preceding tool_call) the request's toolCall applies whole.
export function permissionToolUpdate(existing: ToolCallBlock | undefined, tc: acp.ToolCallUpdate): acp.SessionUpdate {
  if (!existing) return { sessionUpdate: 'tool_call_update', ...tc };
  const u: acp.ToolCallUpdate = { toolCallId: tc.toolCallId };
  if (tc.status) u.status = tc.status;
  if (tc.kind && tc.kind !== 'other' && (existing.kind === 'other' || existing.kind === 'think')) u.kind = tc.kind;
  const fileKind = existing.kind === 'read' || existing.kind === 'edit' || existing.kind === 'delete' || existing.kind === 'move';
  // A file-kind block without locations only has a title-word for a target — the request may carry the real path
  if (!existing.target || (fileKind && !existing.locations?.length)) {
    if (tc.title !== undefined) u.title = tc.title;
    if (tc.rawInput !== undefined) u.rawInput = tc.rawInput;
  }
  if (tc.locations?.length && !existing.locations?.length && !pathFromRaw((u.rawInput ?? tc.rawInput) as Record<string, unknown> | undefined)) u.locations = tc.locations;
  return { sessionUpdate: 'tool_call_update', ...u };
}

// What the row shows: execute shows the command; with locations, the file name; otherwise the title
export function pathFromRaw(raw: Record<string, unknown> | undefined): string | undefined {
  return [raw?.path, raw?.file_path, raw?.filePath, raw?.filepath].find((v): v is string => typeof v === 'string' && !!v);
}

// Read tools use either inclusive endpoints or a one-based offset plus a line count.
function readRangeFromRaw(raw: Record<string, unknown>, locations: ToolCallBlock['locations']): ToolCallBlock['readRange'] {
  const positive = (value: unknown): number | undefined =>
    typeof value === 'number' && Number.isSafeInteger(value) && value > 0 ? value : undefined;
  const path = pathFromRaw(raw) ?? (locations?.length === 1 ? locations[0]?.path : undefined);
  const start = positive(raw.line_offset ?? raw.start_line ?? raw.startLine ?? raw.offset);
  if (!path || start === undefined) return;
  const count = positive(raw.n_lines ?? raw.limit ?? raw.line_count);
  const end = positive(raw.end_line ?? raw.endLine) ?? (count === undefined ? undefined : start + count - 1);
  return { path, start, ...(end !== undefined && Number.isSafeInteger(end) && end >= start ? { end } : {}) };
}

export function commandFromRaw(raw: Record<string, unknown> | undefined): string | undefined {
  return typeof raw?.command === 'string' ? raw.command : typeof raw?.cmd === 'string' ? raw.cmd : undefined;
}

function pickTarget(u: acp.ToolCall | acp.ToolCallUpdate, kind: ToolKind): { text: string; mono: boolean; fromTitle?: boolean } | undefined {
  const raw = u.rawInput as Record<string, unknown> | undefined;
  if (kind === 'execute') {
    const cmd = commandFromRaw(raw);
    if (cmd) return { text: cmd, mono: true };
  }
  if (kind === 'search') {
    const q = typeof raw?.pattern === 'string' ? raw.pattern : typeof raw?.query === 'string' ? raw.query : undefined;
    if (q) return { text: q, mono: true };
  }
  if (kind === 'fetch' && typeof raw?.url === 'string' && raw.url) return { text: raw.url, mono: true };
  const loc = u.locations?.[0]?.path;
  if (loc) return { text: basename(loc), mono: false };
  if (kind === 'read' || kind === 'edit' || kind === 'delete' || kind === 'move') {
    const path = pathFromRaw(raw);
    if (path) return { text: basename(path), mono: false };
  }
  if (u.title) return { text: stripVerb(u.title), mono: false, fromTitle: true };
  return undefined;
}

// A shell tool names the background command it acts on (rawInput.shell_id → the parked exec), falling back to the bare shell id
function shellTarget(raw: Record<string, unknown> | undefined, s?: NormalizeState): { text: string; mono: boolean; fromTitle?: boolean } | undefined {
  const shellId = [raw?.shell_id, raw?.shellId, raw?.id].find((v): v is string => typeof v === 'string' && !!v);
  if (!shellId) return undefined;
  return { text: s?.shells?.[shellId] ?? shellId, mono: true };
}

// An agent's title is often like "Read file foo.ts"; we supply the verb ourselves, so strip the English verb to avoid duplication
function stripVerb(title: string): string {
  return title.replace(/^(read(ing)?|edit(ing)?|write|writing|search(ing)?|run(ning)?|execute|executing|fetch(ing)?|delete|deleting|move|moving|list(ing)?)\s+(file|files|directory|command)?\s*/i, '').replace(/^`|`$/g, '').trim() || title;
}

// A non-zero exit lands on its own line; an output ending in a newline does not grow a blank line in between
function appendExitCode(prev: string, exitCode: number): string {
  const base = prev.replace(/\n+$/, '');
  return base ? `${base}\nexit code ${exitCode}` : `exit code ${exitCode}`;
}

// A completed command's `rawOutput` shaped like codex-acp's { formatted_output, exit_code }; anything else stays generic
function formattedOutput(rawOutput: unknown): { text: string; exitCode?: number } | undefined {
  if (typeof rawOutput !== 'object' || rawOutput === null) return undefined;
  const o = rawOutput as Record<string, unknown>;
  if (typeof o.formatted_output !== 'string') return undefined;
  return { text: o.formatted_output, exitCode: typeof o.exit_code === 'number' && o.exit_code !== 0 ? o.exit_code : undefined };
}

// MIME types the webview is asked to render inline; anything else degrades to a text note
const SHOWABLE_IMAGE = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp']);

// ACP image content (message chunk or a `content` tool item carrying an image block) → transcript image whose pixels
// go to the blob store through saveImage; without a saver or a payload worth keeping it degrades to a text note so
// the transcript never silently loses a block
function imageContent(c: acp.ContentBlock, s?: NormalizeState): ToolContent | undefined {
  if (c.type !== 'image') return undefined;
  let mimeType = typeof c.mimeType === 'string' ? c.mimeType : 'image/png';
  let uri = typeof c.uri === 'string' && c.uri ? c.uri : undefined;
  let data = typeof c.data === 'string' && c.data ? c.data : undefined;
  // A data URL carries its payload inline, whether it arrived in `data` or `uri`
  const inline = (data ?? uri)?.match(/^data:([\w.+-]+\/[\w.+-]+)?;base64,(.+)$/s);
  if (inline) {
    data = inline[2];
    if (inline[1]) mimeType = inline[1];
    if (uri?.startsWith('data:')) uri = undefined;
  }
  const blob = data && s?.saveImage && SHOWABLE_IMAGE.has(mimeType) && base64Bytes(data) <= MAX_OUT_IMAGE_BYTES
    ? s.saveImage(data, mimeType) : undefined;
  if (blob || (uri && !data)) return { type: 'image', mimeType, ...(blob ? { blob } : {}), ...(uri ? { uri } : {}) };
  return { type: 'text', text: s?.saveImage ? `[image: ${mimeType}, not shown]` : '[image]' };
}

// A local path behind a resource_link's uri (absolute path or file:// URL); anything remote or opaque stays a link
function localPathOf(uri: string): string | undefined {
  if (/^file:/i.test(uri)) {
    try { return fileURLToPath(uri); } catch { return undefined; }
  }
  return isAbsolute(uri) ? uri : undefined;
}

// A `content` item holding a resource_link to a local image file (codex-acp's view_image preview) → image content
// through saveImageFile; the link's uri stays as the caption. A missing saver or unreadable file renders as the link
function fileImageContent(c: acp.ContentBlock, s?: NormalizeState): ToolContent | undefined {
  if (c.type !== 'resource_link' || typeof c.uri !== 'string') return undefined;
  const path = localPathOf(c.uri);
  const mimeType = path ? imageMimeOf(path) : undefined;
  const blob = mimeType && s?.saveImageFile ? s.saveImageFile(path!) : undefined;
  return blob ? { type: 'image', mimeType: mimeType!, blob, uri: c.uri } : undefined;
}

// Every renderable content item, in wire order: a diff per `diff` item, one merged text per run of consecutive
// `content` items, an image per image item (it splits the text run). `terminal` items carry no body here — mergeTool
// fills it from the _meta stream (pi-acp) or the not-wired note (agents expecting a client terminal)
function toolContents(items: acp.ToolCallContent[], s?: NormalizeState): ToolContent[] {
  const out: ToolContent[] = [];
  const pushText = (text: string) => {
    if (!text) return;
    const last = out[out.length - 1];
    if (last?.type === 'text') last.text = (last.text + '\n' + text).slice(0, TOOL_OUTPUT_MAX);
    else out.push({ type: 'text', text: text.slice(0, TOOL_OUTPUT_MAX) });
  };
  for (const item of items) {
    if (item.type === 'diff') {
      out.push({
        type: 'diff', lines: diffLines(item.oldText ?? '', item.newText),
        // Full sides preserve multiline syntax state and exact copy text after context folding.
        source: { path: item.path, oldText: item.oldText ?? '', newText: item.newText },
      });
      continue;
    }
    if (item.type !== 'content') continue;
    const img = imageContent(item.content, s) ?? fileImageContent(item.content, s);
    if (img) {
      if (img.type === 'text') pushText(img.text);
      else out.push(img);
      continue;
    }
    pushText(textOf(item.content));
  }
  return out;
}

// What's happening right now: feeds the Activity line of Turns
export function activityOf(turns: Turn[]): AgentTurn['activity'] {
  const turn = turns[turns.length - 1];
  if (turn?.role !== 'agent') return { kind: 'think', label: t('host.working') };
  for (let i = turn.blocks.length - 1; i >= 0; i--) {
    const b = turn.blocks[i]!;
    // A parked background command runs on its own; the agent has moved on, so it never counts as the current action
    if (b.type === 'tool_call' && !b.background && (b.status === 'in_progress' || b.status === 'pending')) return { kind: b.kind, label: t('host.doing', { verb: b.verb, target: b.target ?? '' }).trim() };
    if (b.type === 'permission') return { kind: 'other', label: t('host.awaitingApproval') };
    if (b.type === 'question' && !b.outcome) return { kind: 'other', label: t('host.awaitingAnswers') };
  }
  const last = turn.blocks[turn.blocks.length - 1];
  // ACP does not identify the transition from reasoning to tool-argument generation.
  if (last?.type === 'thought' && last.streaming) return { kind: 'think', label: t('host.working') };
  if (last?.type === 'text' && last.streaming) return { kind: 'other', label: t('host.replying') };
  return { kind: 'think', label: t('host.working') };
}
