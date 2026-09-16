import { createHash } from 'node:crypto';
import type { AgentTurn, SessionSummary, SessionView, ToolCallBlock, ToolKind, Turn, TurnStop } from '@shared/transcript';
import { diffLines } from '../acp/diff';

export const CHATGPT_ID = 'chatgpt';
export const STALE_AFTER_MS = 45_000;
export const OUTPUT_LIMIT = 256_000;
const EVENT_LIMIT = 2_000_000;
const TOOL_KINDS = new Set<ToolKind>(['read', 'edit', 'delete', 'move', 'search', 'execute', 'think', 'fetch', 'switch_mode', 'other']);
const STOPS = new Set<TurnStop>(['end_turn', 'cancelled', 'error', 'max_tokens', 'max_turn_requests', 'refusal']);

export type ChatGptEvent = { id: string; turnId: string } & (
  | { type: 'turn_start'; text: string; previousTurnId?: string }
  | { type: 'turn_resume' }
  | { type: 'message'; messageId: string; text: string; phase: 'commentary' | 'final' }
  | { type: 'tool_start'; callId: string; name: string; kind: ToolKind; target?: string; input?: unknown }
  | { type: 'tool_output'; callId: string; text: string }
  | { type: 'tool_end'; callId: string; status: 'completed' | 'failed' | 'cancelled'; detail?: string; diff?: { path: string; oldText: string; newText: string } }
  | { type: 'heartbeat' }
  | { type: 'turn_end'; stop: TurnStop }
);

export interface ChatGptRecord {
  version: 1;
  id: string;
  sourceKey: string;
  title: string;
  cwd: string;
  createdAt: string;
  updatedAt: string;
  lastEventAt: string;
  revision: number;
  turns: Turn[];
  activeTurnId?: string;
  // Late output from an older turn must not renew the current turn's lease.
  activeEventAt?: string;
  pinned?: boolean;
  deletedAt?: number;
  // Event IDs are caller-generated, not MCP transport session IDs. Conflicting retries fail closed.
  receipts: Record<string, string>;
}

export function chatgptSessionId(key: string): string {
  if (!key.trim() || key.length > 512) throw new Error('A nonempty source session key (at most 512 characters) is required');
  return `chatgpt-${createHash('sha256').update(key).digest('hex').slice(0, 32)}`;
}
export function isChatGptId(id: string): boolean { return /^chatgpt-[a-f0-9]{32}$/.test(id); }

function object(v: unknown): Record<string, unknown> {
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw new Error('Expected an event object');
  return v as Record<string, unknown>;
}
function text(v: unknown, field: string): asserts v is string {
  if (typeof v !== 'string') throw new Error(`Expected string: ${field}`);
}
function token(v: unknown, field: string): asserts v is string {
  if (typeof v !== 'string' || !/^[\w.-]{1,160}$/.test(v) || ['__proto__', 'constructor', 'prototype'].includes(v)) throw new Error(`Invalid ${field}`);
}
export function parseChatGptEvent(value: unknown): ChatGptEvent {
  const e = object(value);
  if (Buffer.byteLength(JSON.stringify(e)) > EVENT_LIMIT) throw new Error('Event exceeds 2 MB; split output into chunks');
  token(e.id, 'event id'); token(e.turnId, 'turn id');
  switch (e.type) {
    case 'turn_start':
      text(e.text, 'text');
      if (e.previousTurnId !== undefined) token(e.previousTurnId, 'previous turn id');
      break;
    case 'turn_resume': break;
    case 'message':
      token(e.messageId, 'message id'); text(e.text, 'text');
      if (e.phase !== 'commentary' && e.phase !== 'final') throw new Error('Only visible commentary/final messages may be mirrored');
      break;
    case 'tool_start':
      token(e.callId, 'call id'); text(e.name, 'tool name');
      if (!e.name || !TOOL_KINDS.has(e.kind as ToolKind)) throw new Error('Invalid tool name or kind');
      if (e.target !== undefined) text(e.target, 'target');
      break;
    case 'tool_output': token(e.callId, 'call id'); text(e.text, 'text'); break;
    case 'tool_end':
      token(e.callId, 'call id');
      if (!['completed', 'failed', 'cancelled'].includes(String(e.status))) throw new Error('Invalid tool status');
      if (e.detail !== undefined) text(e.detail, 'detail');
      if (e.diff !== undefined) {
        const d = object(e.diff); text(d.path, 'diff.path'); text(d.oldText, 'diff.oldText'); text(d.newText, 'diff.newText');
      }
      break;
    case 'heartbeat': break;
    case 'turn_end': if (!STOPS.has(e.stop as TurnStop)) throw new Error('Invalid turn stop'); break;
    default: throw new Error('Unsupported ChatGPT bridge event');
  }
  return e as ChatGptEvent;
}

function activeTurn(r: ChatGptRecord, id: string): AgentTurn {
  const turn = r.turns.at(-1);
  if (r.activeTurnId !== id || turn?.role !== 'agent' || turn.stop) throw new Error('Turn is not active; open it with turn_start first');
  return turn;
}
function tool(turn: AgentTurn, id: string): ToolCallBlock {
  const b = turn.blocks.find((b): b is ToolCallBlock => b.type === 'tool_call' && b.id === id);
  if (!b) throw new Error(`Unknown tool call: ${id}`);
  return b;
}

// Pure transactional reducer: the caller owns the file lock and persists only on success.
export function applyChatGptEvent(record: ChatGptRecord, value: unknown, now = Date.now()): ChatGptRecord {
  const e = parseChatGptEvent(value);
  if (record.deletedAt !== undefined) throw new Error('This mirror was deleted; refusing to recreate it');
  const digest = createHash('sha256').update(JSON.stringify(e)).digest('hex');
  if (Object.hasOwn(record.receipts, e.id)) {
    if (record.receipts[e.id] !== digest) throw new Error('Event ID was reused with different content');
    return record;
  }
  if (Object.keys(record.receipts).length >= 100_000) throw new Error('Mirror event limit reached; start a new mirror');
  const r = structuredClone(record);
  const at = new Date(now).toISOString();
  if (e.type === 'turn_start') {
    const existing = r.turns.find(t => t.role === 'user' && t.id === e.turnId);
    if (existing?.role === 'user') {
      if (existing.text !== e.text) throw new Error('Turn ID reused with different text');
      return record; // A transport retry never duplicates a prompt or restarts work.
    }
    if (r.activeTurnId && e.previousTurnId !== r.activeTurnId) throw new Error('Finish the active turn before starting another, or explicitly name --previous-turn');
    if (e.previousTurnId && e.previousTurnId !== r.activeTurnId) throw new Error('Previous turn changed; refresh before continuing');
    // Superseding a turn only records the new user message. The old turn and any
    // unfinished tools retain no completion timestamp or successful stop receipt.
    r.turns.push({ role: 'user', id: e.turnId, text: e.text }, { role: 'agent', blocks: [], startedAt: now });
    r.activeTurnId = e.turnId; r.activeEventAt = at; r.updatedAt = at;
  } else if (e.type === 'turn_resume') {
    const index = r.turns.findIndex(t => t.role === 'user' && t.id === e.turnId);
    const turn = r.turns[index + 1];
    if (index < 0 || index !== r.turns.length - 2 || turn?.role !== 'agent' || turn.stop)
      throw new Error('Only the latest unfinished turn can resume');
    if (r.activeTurnId && r.activeTurnId !== e.turnId) throw new Error('Another turn is active');
    r.activeTurnId = e.turnId; r.activeEventAt = at;
  } else {
    // A command already running can report its result after the next user turn.
    // Route by turn identity, never attach late output to the newest turn.
    const late = e.type === 'tool_output' || e.type === 'tool_end' || e.type === 'turn_end' || e.type === 'heartbeat';
    const index = late ? r.turns.findIndex(t => t.role === 'user' && t.id === e.turnId) : -1;
    const candidate = index >= 0 ? r.turns[index + 1] : undefined;
    const turn = late && candidate?.role === 'agent' ? candidate : activeTurn(r, e.turnId);
    switch (e.type) {
      case 'message': {
        const existing = turn.blocks.find(b => b.type === 'text' && b.id === e.messageId);
        if (existing?.type === 'text') { existing.markdown = e.text; existing.phase = e.phase; }
        else turn.blocks.push({ type: 'text', id: e.messageId, phase: e.phase, markdown: e.text });
        break;
      }
      case 'tool_start':
        if (turn.blocks.some(b => b.type === 'tool_call' && b.id === e.callId)) throw new Error('Tool call ID already exists');
        turn.blocks.push({ type: 'tool_call', id: e.callId, kind: e.kind, verb: e.name, target: e.target, targetMono: true,
          status: 'in_progress', startedAt: now, content: { type: 'text', text: e.input === undefined ? '' : `Input:\n${JSON.stringify(e.input, null, 2)}\n\nOutput:\n` } });
        break;
      case 'tool_output': {
        const b = tool(turn, e.callId);
        if (b.status !== 'in_progress') throw new Error('Tool is already settled');
        const previous = b.content?.type === 'text' ? b.content.text : '';
        if (previous.length < OUTPUT_LIMIT) {
          const joined = previous + e.text;
          b.content = { type: 'text', text: joined.length > OUTPUT_LIMIT ? joined.slice(0, OUTPUT_LIMIT) + '\n[Output truncated at 256,000 characters]' : joined };
        }
        break;
      }
      case 'tool_end': {
        const b = tool(turn, e.callId);
        if (b.status !== 'in_progress') throw new Error('Tool is already settled');
        b.status = e.status; b.endedAt = now; b.meta = e.detail;
        if (e.diff) {
          if (e.status !== 'completed') throw new Error('A failed tool cannot claim an applied diff');
          b.content = { type: 'diff', lines: diffLines(e.diff.oldText, e.diff.newText), source: e.diff };
          b.locations = [{ path: e.diff.path }];
        }
        break;
      }
      case 'turn_end':
        if (turn.stop) {
          if (turn.stop !== e.stop) throw new Error('Conflicting turn completion receipt');
          return record;
        }
        if (e.stop === 'end_turn' && turn.blocks.some(b => b.type === 'tool_call' && b.status === 'in_progress')) throw new Error('Cannot finish while tools have no completion receipt');
        turn.stop = e.stop; turn.endedAt = now; delete turn.activity;
        if (r.activeTurnId === e.turnId) { delete r.activeTurnId; delete r.activeEventAt; }
        break;
      case 'heartbeat':
        // A still-running older command may keep its own bridge alive. It must not
        // fail (which would terminate that child), nor renew the newest turn.
        if (r.activeTurnId !== e.turnId) return record;
        break;
    }
  }
  if (r.activeTurnId === e.turnId) r.activeEventAt = at;
  r.receipts[e.id] = digest; r.revision++; r.lastEventAt = at;
  return r;
}

export function chatgptView(r: ChatGptRecord, now = Date.now()): SessionView {
  const stale = !!r.activeTurnId && now - Date.parse(r.activeEventAt ?? r.lastEventAt) > STALE_AFTER_MS;
  const turns = r.turns.map((turn, index): Turn => {
    if (turn.role !== 'agent') return turn;
    const user = r.turns[index - 1];
    const observed = user?.role === 'user' && user.id === r.activeTurnId && !stale;
    return { ...turn, observation: !turn.stop && !observed ? 'unknown' : undefined,
      blocks: turn.blocks.map(block => block.type === 'tool_call' && block.status === 'in_progress' && !observed
        ? { ...block, observation: 'unknown' as const } : block) };
  });
  return { id: r.id, agent: CHATGPT_ID, title: r.title, cwd: r.cwd, status: 'readonly', turns,
    running: !!r.activeTurnId && !stale, rev: r.revision * 2 + Number(stale), controls: { modes: [], options: [] }, commands: [],
    createdAt: r.createdAt, updatedAt: r.updatedAt,
    external: { source: 'chatgpt', sourceKey: r.sourceKey, state: !r.turns.length ? 'unbound' : stale ? 'stale' : r.activeTurnId ? 'receiving' : 'idle', activeTurnId: r.activeTurnId, lastEventAt: r.lastEventAt } };
}
export function chatgptSummary(r: ChatGptRecord, now = Date.now()): SessionSummary {
  const v = chatgptView(r, now);
  return { id: r.id, title: r.title, agent: CHATGPT_ID, cwd: r.cwd, updatedAt: r.updatedAt, pinned: r.pinned,
    external: true, state: v.running ? 'working' : v.external?.state === 'stale' ? 'waiting' : undefined };
}
