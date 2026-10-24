// The trace of one agent turn as a span tree on a shared clock: the turn, its tool calls and its subagents (with their own
// tool calls when the child's transcript is at hand). Pure: the webview's trace waterfall draws it, tests read it.
// Timing comes only from what the transcript observed (`startedAt` / `endedAt`); a call without a timer is listed without a bar,
// and nothing here invents model-call spans: the time a turn or subagent spends outside its children is drawn as model time

import type { AgentTurn, ToolCallBlock, Turn } from './transcript';
import type { SubagentSummary } from './subagents';

export type SpanKind = 'turn' | 'subagent' | 'tool';
export type SpanState = 'running' | 'done' | 'failed' | 'cancelled';
// What a bar segment stands for: a tool call, a subagent, time outside any child (the model), a failure
export type SpanTone = 'tool' | 'agent' | 'model' | 'fail';

export interface TraceSpan {
  id: string;
  kind: SpanKind;
  state: SpanState;
  // Milliseconds from the trace's origin; `start` absent = no observed timer, `end` absent = still running
  start?: number;
  end?: number;
  tool?: ToolCallBlock;
  node?: SubagentSummary;
  children: TraceSpan[];
  // Descendants in all
  size: number;
}

export interface TraceStats {
  tokens?: { input?: number; output?: number; total?: number };
  modelCalls?: number;
  tools: number;
  subagents: number;
  errors: number;
}

export interface Trace {
  root: TraceSpan;
  // Epoch ms of t = 0: when the prompt went out
  origin: number;
  // Epoch ms the turn ended; absent while it runs
  endedAt?: number;
  stats: TraceStats;
  // The finished tool call that took longest, when there are at least two to compare
  slowest?: string;
}

export interface TraceInput {
  turn: AgentTurn;
  turnIndex: number;
  // Every subagent of the session; the ones announced in this turn join it
  subagents?: SubagentSummary[];
  // A child's transcript when the host already sent it (the inspector observed it)
  childTurns?: (subagentId: string) => Turn[] | undefined;
}

const TOOL_STATE: Record<ToolCallBlock['status'], SpanState> = {
  pending: 'running', in_progress: 'running', completed: 'done', failed: 'failed', cancelled: 'cancelled',
};
const NODE_STATE: Record<SubagentSummary['state'], SpanState> = {
  running: 'running', completed: 'done', failed: 'failed', cancelled: 'cancelled', disconnected: 'cancelled',
};

const sizeOf = (children: TraceSpan[]) => children.reduce((n, c) => n + 1 + c.size, 0);
const byStart = (a: TraceSpan, b: TraceSpan) => (a.start ?? Infinity) - (b.start ?? Infinity);

// Undefined when the turn has no clock at all (transcripts stored before turn timing)
export function buildTrace(input: TraceInput): Trace | undefined {
  const { turn, turnIndex } = input;
  const origin = turn.startedAt ?? firstToolStart(turn.blocks);
  if (origin === undefined) return undefined;
  const live = turn.endedAt === undefined;
  const at = (ms: number | undefined) => (ms === undefined ? undefined : Math.max(0, ms - origin));
  const all = input.subagents ?? [];
  const nodes = all.filter(n => n.turnIndex === turnIndex);
  const kids = new Map<string, SubagentSummary[]>();
  for (const n of nodes) if (n.parentId) kids.set(n.parentId, [...kids.get(n.parentId) ?? [], n]);
  const placed = new Set<string>();

  const toolSpan = (block: ToolCallBlock, ownerLive: boolean): TraceSpan => {
    const node = block.subagentId ? nodes.find(n => n.id === block.subagentId) : undefined;
    // The delegation row keeps the call's id, so the running call it was becomes the subagent in place
    if (node) return nodeSpan(node, at(block.startedAt), block.id);
    let state = TOOL_STATE[block.status];
    // A call left in progress by a turn that ended was never closed; background commands keep running past the turn
    if (state === 'running' && !ownerLive && !block.background) state = 'cancelled';
    const start = at(block.startedAt);
    const end = state === 'running' ? undefined : at(block.endedAt) ?? (start !== undefined ? at(turn.endedAt) : undefined);
    return { id: block.id, kind: 'tool', state, ...(start !== undefined ? { start } : {}), ...(end !== undefined ? { end } : {}), tool: block, children: [], size: 0 };
  };

  const nodeSpan = (node: SubagentSummary, delegatedAt?: number, id = `subagent:${node.id}`): TraceSpan => {
    placed.add(node.id);
    const state = NODE_STATE[node.state];
    const own = input.childTurns?.(node.id);
    const children: TraceSpan[] = [];
    if (own) for (const t of own) if (t.role === 'agent') for (const b of t.blocks) if (b.type === 'tool_call') children.push(toolSpan(b, state === 'running'));
    for (const child of kids.get(node.id) ?? []) if (!placed.has(child.id)) children.push(nodeSpan(child));
    children.sort(byStart);
    const start = delegatedAt ?? at(node.announcedAt)!;
    const end = state === 'running' ? undefined : at(node.endedAt) ?? at(turn.endedAt);
    return { id, kind: 'subagent', state, start, ...(end !== undefined ? { end } : {}), node, children, size: sizeOf(children) };
  };

  const children: TraceSpan[] = [];
  for (const b of turn.blocks) if (b.type === 'tool_call') children.push(toolSpan(b, live));
  // Children announced without a delegation row in this turn (or whose row the agent never sent)
  for (const n of nodes) if (!n.parentId && !placed.has(n.id)) children.push(nodeSpan(n));
  children.sort(byStart);

  const failed = turn.stop === 'error' || turn.error !== undefined;
  const state: SpanState = live ? 'running' : failed ? 'failed' : turn.stop === 'cancelled' ? 'cancelled' : 'done';
  const root: TraceSpan = {
    id: `turn:${turnIndex}`, kind: 'turn', state, start: 0,
    ...(turn.endedAt !== undefined ? { end: turn.endedAt - origin } : {}),
    children, size: sizeOf(children),
  };

  const stats: TraceStats = { tools: 0, subagents: 0, errors: failed ? 1 : 0 };
  const u = turn.usage;
  if (u && (u.input !== undefined || u.output !== undefined || u.total !== undefined)) {
    stats.tokens = { ...(u.input !== undefined ? { input: u.input } : {}), ...(u.output !== undefined ? { output: u.output } : {}), ...(u.total !== undefined ? { total: u.total } : {}) };
  }
  if (u?.modelCalls !== undefined) stats.modelCalls = u.modelCalls;
  let slowest: { id: string; ms: number } | undefined;
  let finished = 0;
  walk(root, s => {
    if (s.kind === 'tool') stats.tools++;
    if (s.kind === 'subagent') stats.subagents++;
    if (s.kind !== 'turn' && s.state === 'failed') stats.errors++;
    if (s.kind === 'tool' && s.start !== undefined && s.end !== undefined) {
      finished++;
      if (!slowest || s.end - s.start > slowest.ms) slowest = { id: s.id, ms: s.end - s.start };
    }
  });
  return { root, origin, ...(turn.endedAt !== undefined ? { endedAt: turn.endedAt } : {}), stats, ...(slowest && finished > 1 ? { slowest: slowest.id } : {}) };
}

function firstToolStart(blocks: AgentTurn['blocks']): number | undefined {
  let first: number | undefined;
  for (const b of blocks) if (b.type === 'tool_call' && b.startedAt !== undefined && (first === undefined || b.startedAt < first)) first = b.startedAt;
  return first;
}

export function walk(span: TraceSpan, visit: (span: TraceSpan, depth: number) => void, depth = 0) {
  visit(span, depth);
  for (const c of span.children) walk(c, visit, depth + 1);
}

// Where a span's bar ends at `now` (ms from the origin): a running span runs to now
export function spanEnd(span: TraceSpan, now: number): number | undefined {
  if (span.start === undefined) return undefined;
  return span.end ?? Math.max(span.start, now);
}

// `open`: the segment runs up to now (a running child, or model time after the last child of a running span), so the bar
// drawing it follows the clock instead of a fixed end
export interface Segment { from: number; to: number; tone: SpanTone; open?: true }

const RANK: Record<SpanTone, number> = { model: 0, tool: 1, agent: 2, fail: 3 };
const toneOf = (s: TraceSpan): SpanTone => (s.state === 'failed' ? 'fail' : s.kind === 'subagent' ? 'agent' : 'tool');

// A folded span's bar drawn as what its children did: their intervals merged where they overlap (the strongest tone wins)
// and the time between them as model time. Empty when no child has a timer
export function segments(span: TraceSpan, now: number): Segment[] {
  const from = span.start;
  const to = spanEnd(span, now);
  if (from === undefined || to === undefined) return [];
  const runs: Segment[] = [];
  for (const c of [...span.children].sort(byStart)) {
    const a = c.start;
    const b = spanEnd(c, now);
    if (a === undefined || b === undefined) continue;
    const open = c.end === undefined;
    const last = runs[runs.length - 1];
    if (last && a <= last.to) {
      if (b >= last.to) { last.to = b; if (open) last.open = true; else delete last.open; }
      if (RANK[toneOf(c)] > RANK[last.tone]) last.tone = toneOf(c);
    } else runs.push({ from: a, to: b, tone: toneOf(c), ...(open ? { open: true as const } : {}) });
  }
  if (!runs.length) return [];
  const out: Segment[] = [];
  let cursor = from;
  for (const r of runs) {
    if (r.from > cursor) out.push({ from: cursor, to: r.from, tone: 'model' });
    out.push(r);
    cursor = Math.max(cursor, r.to);
  }
  if (to > cursor) out.push({ from: cursor, to, tone: 'model', ...(span.end === undefined ? { open: true as const } : {}) });
  return out;
}

// Tick steps an axis may take, in ms
const STEPS = [500, 1000, 2000, 5000, 10_000, 15_000, 20_000, 30_000, 60_000, 120_000, 300_000, 600_000, 900_000, 1_800_000, 3_600_000, 7_200_000];

// The window a live trace opens with: the clock sweeps across it, then the axis grows with the clock, keeping now at the
// axis' end (the chart zooms out continuously rather than in steps)
export const LIVE_MIN = 10_000;

// The axis length for a trace `duration` ms long: the trace itself, or the opening window while a short one runs.
// The webview ends the axis short of the bar column's edge (trace.css --trace-tail), which leaves the last label its room
export function domain(duration: number, live: boolean): number {
  return Math.max(live ? LIVE_MIN : 250, duration);
}

// The tick step for an axis `max` ms long: the smallest step that keeps at most `ticks` ticks on it
export function tickStep(max: number, ticks = 5): number {
  return STEPS.find(s => Math.ceil(max / s) <= ticks) ?? STEPS[STEPS.length - 1]!;
}
