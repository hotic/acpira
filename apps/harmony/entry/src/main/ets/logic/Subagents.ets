import { Block, SubagentSummary } from '../wire/Blocks';
import { SessionControls } from '../wire/Wire';
import { t } from '../store/L10n';
import { commandDuration, tp } from './Folding';
import { ComposerControls, vendorBrand } from './ModelCatalog';

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

/** The persona a session's sidekick goes by (subagents.ts SIDEKICK_ID) */
const sidekickId: string = 'sidekick';

/** isSidekick (SidekickRelay.tsx): a round of the session's sidekick, drawn as a relay rather than a subagent row */
export function isSidekick(n: SubagentSummary): boolean {
  return n.harness?.persona === sidekickId;
}

/** A model's mark: its vendor's brand key (Icons.ets MARKS; undefined: none known) and the name an unknown one shows the initial of */
export class ModelMark {
  brand: string | undefined = undefined;
  name: string = '';
}

/** The sidekick's mark, leading the handoff, every step and the handback: the round's model id names the vendor best */
export function sidekickMark(n: SubagentSummary, name: string): ModelMark {
  const m = new ModelMark();
  m.brand = (n.model !== undefined ? vendorBrand(n.model) : undefined) ?? vendorBrand(name);
  m.name = name;
  return m;
}

/**
 * SidekickRelay.swift SidekickModels, less the lead model only the handoff's flip shows: the sidekick's display names
 * from the session's sidekick select. A round carries only its model id: the select's value for the built-in agent, the
 * id inside its CLI for the host's select, whose values are `<cli>/<model>`. `agent` is the session's CLI: a sidekick
 * running in another one carries that CLI's mark
 */
export class SidekickModels {
  names: Map<string, string> = new Map<string, string>();
  agent: string = '';

  /** The round's model as the select names it, else as the round reports it */
  name(n: SubagentSummary): string | undefined {
    const model = n.model;
    if (model === undefined) {
      return undefined;
    }
    const cli = n.harness?.agent ?? '';
    return this.names.get(model) ?? (cli.length > 0 ? this.names.get(`${cli}/${model}`) : undefined) ?? model;
  }

  /** The CLI the round runs in, when it is not the session's own */
  away(n: SubagentSummary): string | undefined {
    const cli = n.harness?.agent ?? '';
    return cli.length > 0 && this.agent.length > 0 && cli !== this.agent ? cli : undefined;
  }
}

/** The view's controls (the thread reads them loosely) as the relay reads them */
export function sidekickModels(controls: Object | undefined, agent: string): SidekickModels {
  const out = new SidekickModels();
  out.agent = agent;
  const split = ComposerControls.split((controls as SessionControls | undefined)?.options ?? []);
  for (const o of split.sidekick.length > 0 ? split.sidekick[0].options : []) {
    if (!out.names.has(o.id)) {
      out.names.set(o.id, o.name);
    }
  }
  return out;
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

const plainSkip = /^(```|~~~|([-*_])\s*\2\s*\2[\s\-*_]*$)/;

/**
 * firstPlainLine (subagents.ts): a returned reply's first line in plain words for a one-line preview. Emphasis, code and
 * strike marks go, a link or image keeps its text, a heading, quote or list marker leaves the start; fence and rule
 * lines are skipped
 */
export function firstPlainLine(markdown: string): string | undefined {
  const line = markdown.split('\n').map((l: string) => l.trim()).find((l: string) => l.length > 0 && !plainSkip.test(l));
  if (line === undefined) {
    return undefined;
  }
  const plain = line
    .replace(/^(#{1,6}\s+|>\s*|[-*+]\s+(\[[ xX]\]\s+)?)/, '')
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/(\*\*|__|~~)(?=\S)(.+?)(?<=\S)\1/g, '$2')
    .replace(/(^|[^\w*])\*(?=[^\s*])([^*]*?[^\s*])\*(?![\w*])/g, '$1$2')
    .replace(/(^|[^\w_])_(?=[^\s_])([^_]*?[^\s_])_(?![\w_])/g, '$1$2')
    .replace(/`([^`]+)`/g, '$1')
    .trim();
  return plain.length > 0 ? plain : undefined;
}

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
  const line = firstPlainLine(n.result ?? '');
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
