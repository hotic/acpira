import { Block, SubagentSummary } from '../wire/Blocks';
import { t } from '../store/L10n';
import { commandDuration, tp } from './Folding';

// Where a turn's subagent rows sit and what they say (apps/ios Logic/Subagents.swift, apps/android logic/Subagents.kt,
// chat/subagents.ts): a placed delegation row stays in the turn's process and its node renders there; the others follow
// the turn's process.

/** delegatedIds: the turn's nodes that can sit at their delegation row, those whose id a root tool call carries */
export function delegatedIds(blocks: Block[], turnNodes: SubagentSummary[]): Set<string> {
  const ids = new Set<string>(turnNodes.map((n: SubagentSummary) => n.id));
  const out = new Set<string>();
  for (const b of blocks) {
    if (b.type === 'tool_call' && b.subagentId !== undefined && ids.has(b.subagentId)) {
      out.add(b.subagentId);
    }
  }
  return out;
}

/** What placeNodes answers: the nodes placed at each delegation, and the ones that follow the turn's process */
export class Placement {
  byId: Map<string, SubagentSummary[]> = new Map<string, SubagentSummary[]>();
  rest: SubagentSummary[] = [];
}

/**
 * placeNodes: each placed id takes itself and every descendant anchored to the same turn (the nearest placed ancestor
 * wins); nodes with no placed ancestor go to `rest`. Cycles stop at a visited id
 */
export function placeNodes(turnNodes: SubagentSummary[], placed: Set<string>): Placement {
  const byNode = new Map<string, SubagentSummary>();
  for (const n of turnNodes) {
    if (!byNode.has(n.id)) {
      byNode.set(n.id, n);
    }
  }
  const out = new Placement();
  for (const node of turnNodes) {
    const seen = new Set<string>();
    let cur: SubagentSummary | undefined = node;
    while (cur !== undefined && !placed.has(cur.id) && !seen.has(cur.id)) {
      seen.add(cur.id);
      cur = cur.parentId !== undefined ? byNode.get(cur.parentId) : undefined;
    }
    if (cur !== undefined && placed.has(cur.id)) {
      const list = out.byId.get(cur.id) ?? [];
      list.push(node);
      out.byId.set(cur.id, list);
    } else {
      out.rest.push(node);
    }
  }
  return out;
}

/** rootRows: the group's rows, top-level children in announce order */
export function rootRows(turnNodes: SubagentSummary[]): SubagentSummary[] {
  const ids = new Set<string>(turnNodes.map((n: SubagentSummary) => n.id));
  return turnNodes.filter((n: SubagentSummary) => n.parentId === undefined || !ids.has(n.parentId));
}

/** descendantCount: every descendant through parentId edges; the visited set keeps a cycle from looping */
export function descendantCount(id: string, all: SubagentSummary[]): number {
  const seen = new Set<string>([id]);
  const queue: string[] = [id];
  let count = 0;
  while (queue.length > 0) {
    const cur = queue.shift() as string;
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

export function isWaiting(n: SubagentSummary): boolean {
  return (n.permissions ?? []).length > 0 || n.question !== undefined;
}

export function subagentTitle(n: SubagentSummary): string {
  return n.title ?? n.role ?? t('subagents.untitled');
}

export function stateLabel(n: SubagentSummary): string {
  const known = ['running', 'completed', 'failed', 'cancelled', 'disconnected'];
  return t(`subagents.state.${known.includes(n.state) ? n.state : 'disconnected'}`);
}

/** What the row says the child is doing: a pending card beats the state */
export function statusLabel(n: SubagentSummary): string {
  if ((n.permissions ?? []).length > 0) {
    return t('host.awaitingApproval');
  }
  if (n.question !== undefined) {
    return t('host.awaitingAnswers');
  }
  return stateLabel(n);
}

const resultLineMax = 120;

/** secondLine: the row's detail: a pending decision beats the live activity, a finished child quotes its result */
export function secondLine(n: SubagentSummary): string {
  if ((n.permissions ?? []).length > 0) {
    return t('host.awaitingApproval');
  }
  if (n.question !== undefined) {
    return t('host.awaitingAnswers');
  }
  if (n.state === 'running') {
    return n.activity ?? t('host.working');
  }
  const line = (n.result ?? '').split('\n').map((l: string) => l.trim()).find((l: string) => l.length > 0);
  if (line !== undefined) {
    return line.length > resultLineMax ? line.slice(0, resultLineMax - 1) + '…' : line;
  }
  return stateLabel(n);
}

/** The faint part beside the title: the role when it says more than the title, the count of children below */
export function subagentMeta(n: SubagentSummary, all: SubagentSummary[]): string[] {
  const parts: string[] = [];
  const title = subagentTitle(n);
  if (n.role !== undefined && n.role !== title) {
    parts.push(n.role);
  }
  const below = descendantCount(n.id, all);
  if (below > 0) {
    parts.push(tp('subagents.descendants', 'n', `${below}`));
  }
  return parts;
}

/** elapsedText: from the announcement to the end, or to `now` while it runs, in the tool rows' whole seconds */
export function subagentElapsed(n: SubagentSummary, now: number): string {
  return commandDuration(Math.max(0, (n.endedAt ?? now) - n.announcedAt), true);
}
