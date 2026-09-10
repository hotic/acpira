import { basename } from 'node:path';
import type * as acp from '@agentclientprotocol/sdk';
import type { MsgKey } from '@shared/i18n';
import { t } from '../i18n';
import { TOOL_OUTPUT_MAX } from '../limits';
import type {
  AgentBlock, AgentTurn, CompactionBlock, CompactionStatus, ConfigControl, PlanPriority, PlanStatus, SessionControls, SessionOption, SlashCommand, ToolCallBlock, ToolContent, ToolKind, Turn, TurnError, Usage,
} from '@shared/transcript';
import { diffLines } from './diff';
import { isTodoTool, todoEntries } from '@shared/todoTools';
import { lastPlanSnapshot, samePlanEntries } from './planSnapshots';

export { diffLines };

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
      const last = lastBlock(t);
      if (last?.type === 'text' && last.streaming) last.markdown += textOf(u.content);
      else { sealStreaming(s, t); t.blocks.push({ type: 'text', markdown: textOf(u.content), streaming: true }); }
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
    case 'usage_update':
      s.usage = { used: u.used, size: u.size, cost: u.cost?.amount ?? undefined };
      return true;
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
    case 'session_info_update':
      if (u.title) s.title = u.title;
      return true;
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
    if (b.type === 'tool_call' && (b.status === 'in_progress' || b.status === 'pending')) {
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

// Build the controls from the session/new / resume response
export function initControls(controls: SessionControls, modes?: acp.SessionModeState | null, configOptions?: acp.SessionConfigOption[] | null) {
  controls.modes = (modes?.availableModes ?? []).map(m => ({ id: m.id, name: m.name, description: m.description ?? undefined }));
  controls.modeId = modes?.currentModeId;
  if (configOptions) applyConfigOptions(controls, configOptions);
}

// configOptions: a select with category=mode is treated purely as modes — it fills in when modes is empty, and is a duplicate when modes is already present (Kimi sends both);
// in neither case does it enter the control list. The remaining selects are ordered model → thought_level → model_config → others, preserving the agent's order within each class. boolean type is not shown for now
const CATEGORY_ORDER = ['model', 'thought_level', 'model_config'];

export function applyConfigOptions(controls: SessionControls, options: acp.SessionConfigOption[]) {
  const mode = options.find(o => o.type === 'select' && o.category === 'mode');
  if (mode && mode.type === 'select' && (controls.modes.length === 0 || controls.modeConfigId === mode.id)) {
    controls.modes = flattenSelect(mode.options);
    controls.modeId = mode.currentValue;
    controls.modeConfigId = mode.id;
  }
  const rank = (c: ConfigControl) => { const i = CATEGORY_ORDER.indexOf(c.category ?? ''); return i < 0 ? CATEGORY_ORDER.length : i; };
  controls.options = options
    .filter(o => o.type === 'select' && o.category !== 'mode')
    .map((o): ConfigControl => ({
      id: o.id, name: o.name, category: o.category ?? undefined,
      options: o.type === 'select' ? flattenSelect(o.options) : [],
      value: o.type === 'select' ? o.currentValue : undefined,
    }))
    .sort((a, b) => rank(a) - rank(b));
}

function flattenSelect(opts: acp.SessionConfigSelectOptions): SessionOption[] {
  const out: SessionOption[] = [];
  for (const o of opts) {
    if ('group' in o) for (const x of o.options) out.push({ id: x.value, name: x.name, description: x.description ?? o.name, group: { id: o.group, name: o.name } });
    else out.push({ id: o.value, name: o.name, description: o.description ?? undefined });
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
  if (u.status) b.status = u.status;
  // Devin parks a command past its exec timeout: the call stays in_progress while the process runs, and later waits address it by shell id
  if (meta?.['cognition.ai/background'] === true) {
    b.background = true;
    const shellId = meta['cognition.ai/backgroundShellId'];
    const command = meta['cognition.ai/backgroundCommand'];
    if (s && typeof shellId === 'string') (s.shells ??= {})[shellId] = typeof command === 'string' && command ? command : b.target ?? shellId;
  }
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
  // A target inferred from the title is only a fallback while there is no target yet; don't overwrite what rawInput / locations provided.
  // A todo / ask / shell tool's title is just its own name — redundant next to the verb, so drop it (the question card carries the questions,
  // a shell tool names the background command it acts on, or its shell id until that command is known).
  const named = b.verbKey === 'verb.todo' || b.verbKey === 'verb.ask' || b.verbKey === 'verb.wait' || b.verbKey === 'verb.kill';
  const target = b.verbKey === 'verb.wait' || b.verbKey === 'verb.kill' ? shellTarget(raw, s) : pickTarget(u, b.kind);
  if (target && !(named && target.fromTitle) && (!target.fromTitle || !b.target)) { b.target = target.text; b.targetMono = target.mono; }
  if (u.content?.length) {
    const c = toolContent(u.content);
    // Kimi sends the edit diff before execution, then a plain success receipt.
    // Preserve the diff on success; failures must still expose their error output.
    const keepDiff = b.kind === 'edit' && b.status === 'completed' && b.content?.type === 'diff' && c?.type === 'text';
    if (c && !keepDiff) {
      b.content = c;
      b.diffStat = c.type === 'diff'
        ? { add: c.lines.filter(l => l.kind === 'add').length, del: c.lines.filter(l => l.kind === 'del').length }
        : undefined;
    }
  }
  if (!b.content && u.rawOutput !== undefined && u.rawOutput !== null) {
    const text = typeof u.rawOutput === 'string' ? u.rawOutput : JSON.stringify(u.rawOutput, null, 2);
    if (text.trim()) b.content = { type: 'text', text: text.slice(0, TOOL_OUTPUT_MAX) };
  }
  if (isTodoTool(b) && b.status === 'completed') {
    // Parse the full wire result before the generic output preview's size limit.
    const entries = todoEntries(u.rawOutput) ?? (b.content?.type === 'text' ? todoEntries(b.content.text) : undefined);
    if (entries !== undefined) b.todoEntries = entries;
  }
}

// What the row shows: execute shows the command; with locations, the file name; otherwise the title
export function pathFromRaw(raw: Record<string, unknown> | undefined): string | undefined {
  return [raw?.path, raw?.file_path, raw?.filePath].find((v): v is string => typeof v === 'string' && !!v);
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

function toolContent(items: acp.ToolCallContent[]): ToolContent | undefined {
  const diff = items.find(c => c.type === 'diff');
  if (diff && diff.type === 'diff') return {
    type: 'diff', lines: diffLines(diff.oldText ?? '', diff.newText),
    // Full sides preserve multiline syntax state and exact copy text after context folding.
    source: { path: diff.path, oldText: diff.oldText ?? '', newText: diff.newText },
  };
  const texts = items.filter(c => c.type === 'content').map(c => c.type === 'content' ? textOf(c.content) : '').filter(Boolean);
  if (texts.length) return { type: 'text', text: texts.join('\n').slice(0, TOOL_OUTPUT_MAX) };
  const term = items.find(c => c.type === 'terminal');
  if (term && term.type === 'terminal') return { type: 'text', text: t('host.terminalNotWired', { id: term.terminalId }) };
  return undefined;
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
