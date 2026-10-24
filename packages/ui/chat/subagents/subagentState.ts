// Pure subagent-view helpers: grouping, trees and labels for the group row and the inspector.
// No DOM — unit-tested in test/subagentState.test.ts; `t` arrives as a parameter so fixtures decide the locale.
import type { MsgKey, Params } from '@shared/i18n';
import type { SubagentSummary } from '@shared/subagents';
import type { AgentBlock } from '@shared/transcript';

export type T = (key: MsgKey, params?: Params) => string;

// Nodes anchored to one turn, keyed by the root agent turn's index. The previous map's arrays are
// reused member-wise so memoized turn components keep identical props across session pushes.
export function nodesByTurn(nodes: SubagentSummary[], prev?: Map<number, SubagentSummary[]>): Map<number, SubagentSummary[]> {
  const next = new Map<number, SubagentSummary[]>();
  for (const n of nodes) {
    const list = next.get(n.turnIndex);
    if (list) list.push(n);
    else next.set(n.turnIndex, [n]);
  }
  if (prev) for (const [ti, list] of next) {
    const old = prev.get(ti);
    if (old && old.length === list.length && old.every((n, i) => n === list[i])) next.set(ti, old);
  }
  return next;
}

// Which of a turn's nodes can sit at their delegation row: those whose id a root tool call in the turn carries
export function delegatedIds(blocks: AgentBlock[], turnNodes: SubagentSummary[]): Set<string> {
  const ids = new Set(turnNodes.map(n => n.id));
  const out = new Set<string>();
  for (const b of blocks) if (b.type === 'tool_call' && b.subagentId !== undefined && ids.has(b.subagentId)) out.add(b.subagentId);
  return out;
}

// Splits a turn's nodes by the delegation they hang under: each placed id takes itself and every descendant
// anchored to the same turn (nearest placed ancestor wins); nodes with no placed ancestor go to `rest`,
// which renders after the turn's content as before. Cycles stop at a visited id.
export function placeNodes(turnNodes: SubagentSummary[], placed: ReadonlySet<string>): { byId: Map<string, SubagentSummary[]>; rest: SubagentSummary[] } {
  const byNodeId = new Map(turnNodes.map(n => [n.id, n]));
  const byId = new Map<string, SubagentSummary[]>();
  const rest: SubagentSummary[] = [];
  for (const n of turnNodes) {
    const seen = new Set<string>();
    let cur: SubagentSummary | undefined = n;
    while (cur && !placed.has(cur.id) && !seen.has(cur.id)) {
      seen.add(cur.id);
      cur = cur.parentId !== undefined ? byNodeId.get(cur.parentId) : undefined;
    }
    if (cur && placed.has(cur.id)) {
      const list = byId.get(cur.id);
      if (list) list.push(n);
      else byId.set(cur.id, [n]);
    } else rest.push(n);
  }
  return { byId, rest };
}

// Rows of the group: top-level children in announce order. A node whose parent was anchored to another
// turn still shows here — a child row that never renders anywhere would be worse than a slightly wrong one.
export function rootRows(turnNodes: SubagentSummary[]): SubagentSummary[] {
  const ids = new Set(turnNodes.map(n => n.id));
  return turnNodes.filter(n => n.parentId === undefined || !ids.has(n.parentId));
}

export interface WorkflowProgress { done: number; total: number }

// A Claude workflow's agents are receipt nodes keyed `<asyncTaskId>#<index>` in peer.agentId (`subagent_tree.rs`
// `workflow_progress`), so the run's tool row can count them by task id without another field on the wire
export function workflowProgress(nodes: SubagentSummary[] | undefined): Map<string, WorkflowProgress> {
  const out = new Map<string, WorkflowProgress>();
  for (const n of nodes ?? []) {
    const key = n.visibility === 'receipt' ? /^(.+)#\d+$/.exec(n.peer.agentId ?? '')?.[1] : undefined;
    if (key === undefined) continue;
    const c = out.get(key) ?? { done: 0, total: 0 };
    c.total++;
    if (n.state !== 'running') c.done++;
    out.set(key, c);
  }
  return out;
}

// All descendants through parentId edges; the visited set keeps a malformed cycle from looping
export function descendantCount(id: string, all: SubagentSummary[]): number {
  const seen = new Set([id]);
  const queue = [id];
  let count = 0;
  while (queue.length) {
    const cur = queue.shift()!;
    for (const n of all) {
      if (n.parentId === cur && !seen.has(n.id)) {
        seen.add(n.id);
        queue.push(n.id);
        count++;
      }
    }
  }
  return count;
}

// The inspector shows the receipt-only note (task + result, no process) unless a receipt node gained a transcript of
// its own (a Claude workflow agent whose sidechain log the host reads)
export function receiptOnly(node: SubagentSummary): boolean {
  return node.visibility === 'receipt' && !node.hasProcess;
}

export function isWaiting(node: SubagentSummary): boolean {
  return !!node.permissions?.length || !!node.question;
}

// Header tallies like "2 running · 1 waiting · 3 done"; a node waiting on a card counts in both buckets
export function countsLabel(nodes: SubagentSummary[], t: T): string {
  const buckets: [MsgKey, number][] = [
    ['subagents.running', nodes.filter(n => n.state === 'running').length],
    ['subagents.waiting', nodes.filter(isWaiting).length],
    ['subagents.completed', nodes.filter(n => n.state === 'completed').length],
    ['subagents.failed', nodes.filter(n => n.state === 'failed').length],
    ['subagents.cancelled', nodes.filter(n => n.state === 'cancelled').length],
    ['subagents.disconnected', nodes.filter(n => n.state === 'disconnected').length],
  ];
  return buckets.filter(([, n]) => n > 0).map(([k, n]) => t(k, { n })).join(' · ');
}

// Ancestor chain root→node, the node itself last; stops on a visited id and caps at 16 links
export function breadcrumb(id: string, all: SubagentSummary[]): SubagentSummary[] {
  const byId = new Map(all.map(n => [n.id, n]));
  const chain: SubagentSummary[] = [];
  const seen = new Set<string>();
  let cur = byId.get(id);
  while (cur && !seen.has(cur.id) && chain.length < 16) {
    seen.add(cur.id);
    chain.unshift(cur);
    cur = cur.parentId !== undefined ? byId.get(cur.parentId) : undefined;
  }
  return chain;
}

// Depth-first listing in announce order; children of a missing or cyclic parent still appear, flat
export function flattenTree(all: SubagentSummary[]): { node: SubagentSummary; depth: number }[] {
  const byId = new Map(all.map(n => [n.id, n]));
  const out: { node: SubagentSummary; depth: number }[] = [];
  const seen = new Set<string>();
  const walk = (n: SubagentSummary, depth: number) => {
    if (seen.has(n.id)) return;
    seen.add(n.id);
    out.push({ node: n, depth: Math.min(depth, 8) });
    for (const c of all) if (c.parentId === n.id) walk(c, depth + 1);
  };
  for (const r of all) if (r.parentId === undefined || !byId.has(r.parentId)) walk(r, 0);
  for (const n of all) walk(n, 0);
  return out;
}

export function stateLabel(node: SubagentSummary, t: T): string {
  return t(`subagents.state.${node.state}` as MsgKey);
}

export function subagentTitle(node: SubagentSummary, t: T): string {
  return node.title ?? node.role ?? t('subagents.untitled');
}

// The group row's second line: a pending decision beats the live activity, a finished child quotes its result
export function secondLine(node: SubagentSummary, t: T): string {
  if (node.permissions?.length) return t('host.awaitingApproval');
  if (node.question) return t('host.awaitingAnswers');
  if (node.state === 'running') return node.activity ?? t('host.working');
  const line = node.result && resultText(node.result).split('\n').map(s => s.trim()).find(s => s.length > 0);
  if (line) return line.length > 120 ? `${line.slice(0, 119)}…` : line;
  return stateLabel(node, t);
}

// A string field of a JSON object, up to its closing quote or the end of a cut-off text
const JSON_STRING_FIELD = /"([^"\\]+)"\s*:\s*"((?:[^"\\]|\\.)*)"?/g;

// A schema-bound workflow agent returns a JSON object, and the CLI's result preview cuts it off (about 400 characters),
// so it rarely parses. The row quotes its prose instead: the `summary` field when there is one, else the first string
// values in order. A result not shaped like a JSON object passes through unchanged
export function resultText(result: string): string {
  if (!result.trimStart().startsWith('{')) return result;
  const fields = [...result.matchAll(JSON_STRING_FIELD)].map(m => [m[1]!, unescapeJson(m[2]!)] as const).filter(([, v]) => v.trim());
  if (!fields.length) return result;
  return fields.find(([k]) => k === 'summary')?.[1] ?? fields.slice(0, 3).map(([, v]) => v).join(' · ');
}

function unescapeJson(raw: string): string {
  // A cut can land inside an escape sequence; drop the partial one
  const whole = raw.replace(/\\(?:u[0-9a-fA-F]{0,3})?$/, '');
  try { return JSON.parse(`"${whole}"`) as string; } catch { return whole; }
}

export function elapsedMs(node: SubagentSummary, now: number): number {
  return Math.max(0, (node.endedAt ?? now) - node.announcedAt);
}

// Same compact duration vocabulary as the transcript's tool rows
export function elapsedText(node: SubagentSummary, now: number, t: T): string {
  const seconds = Math.floor(elapsedMs(node, now) / 1000);
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return m ? (s ? t('turns.elapsed.ms', { m, s }) : t('turns.elapsed.m', { m })) : t('turns.elapsed.s', { s });
}
