// Pure subagent-view helpers: grouping, trees and labels for the group row and the inspector.
// No DOM — unit-tested in test/subagentState.test.ts; `t` arrives as a parameter so fixtures decide the locale.
import type { MsgKey, Params } from '@shared/i18n';
import type { SubagentSummary } from '@shared/subagents';

export type T = (key: MsgKey, params?: Params) => string;

// The inspector's three tabs; the group row always opens on 'session', the header's link on 'tree'
export type SubagentTab = 'session' | 'tree' | 'info';

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

// Rows of the group: top-level children in announce order. A node whose parent was anchored to another
// turn still shows here — a child row that never renders anywhere would be worse than a slightly wrong one.
export function rootRows(turnNodes: SubagentSummary[]): SubagentSummary[] {
  const ids = new Set(turnNodes.map(n => n.id));
  return turnNodes.filter(n => n.parentId === undefined || !ids.has(n.parentId));
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
  const line = node.result?.split('\n').map(s => s.trim()).find(s => s.length > 0);
  if (line) return line.length > 120 ? `${line.slice(0, 119)}…` : line;
  return stateLabel(node, t);
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
