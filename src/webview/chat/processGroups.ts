import type { AgentBlock, ToolCallBlock } from '@shared/transcript';
import { toolFiles, visibleToolContents } from './toolDetails';
import { parseFileLink, type FileLink } from './fileLinks';

// One item of a turn's process list: a block on its own, or a run of consecutive reads / edits under one head.
// `id` names the item for its fold and entrance: a group goes by its first call, so a single call that gains a
// neighbour keeps its identity; unkeyed blocks go by transcript position
export type ProcessItem =
  | { type: 'block'; id: string; block: AgentBlock }
  | { type: 'group'; id: string; kind: GroupKind; blocks: ToolCallBlock[] };
export type GroupKind = 'read' | 'edit';

// Failed and cancelled calls stay visible on their own row; a completed read without a file reference has nothing
// to list. Running calls join, so a group grows as the agent works instead of a live row trailing it
function groupKind(block: AgentBlock): GroupKind | undefined {
  if (block.type !== 'tool_call' || block.status === 'failed' || block.status === 'cancelled' || block.asyncTask) return;
  if (block.kind === 'read') return block.status !== 'completed' || toolFiles(block).length ? 'read' : undefined;
  if (block.kind === 'edit') return 'edit';
}

// The entrance name of an item, apart from the ids rows use inside (a thought's own EntranceOnce, a tool's `:tool`)
export const itemEntrance = (id: string) => `item:${id}`;

export function blockItemId(block: AgentBlock, at: number): string {
  if (block.type === 'tool_call') return block.id;
  if (block.type === 'thought' && block.startedAt !== undefined) return `thought:${block.startedAt}`;
  return 'id' in block && typeof block.id === 'string' ? block.id : `at:${at}`;
}

export function groupProcess(blocks: AgentBlock[]): ProcessItem[] {
  const out: ProcessItem[] = [];
  let run: ToolCallBlock[] = [];
  let runKind: GroupKind | undefined;
  let runAt = 0;
  const flush = () => {
    if (run.length === 1) out.push({ type: 'block', id: blockItemId(run[0]!, runAt), block: run[0]! });
    else if (run.length > 1) out.push({ type: 'group', id: run[0]!.id, kind: runKind!, blocks: run });
    run = [];
  };
  blocks.forEach((block, at) => {
    const kind = groupKind(block);
    if (kind && kind === runKind) { run.push(block as ToolCallBlock); return; }
    flush();
    runKind = kind;
    if (kind) { run.push(block as ToolCallBlock); runAt = at; } else out.push({ type: 'block', id: blockItemId(block, at), block });
  });
  flush();
  return out;
}

// Whether an item has finished: none of its calls pending or running, its thought no longer streaming.
// A command parked in the background runs on its own and does not hold its row open
export function itemSettled(item: ProcessItem): boolean {
  return item.type === 'group' ? item.blocks.every(blockSettled) : blockSettled(item.block);
}

export function blockSettled(b: AgentBlock): boolean {
  return b.type === 'thought' ? !b.streaming
    : b.type !== 'tool_call' || !!b.background || (b.status !== 'pending' && b.status !== 'in_progress');
}

// A quick action stays open at least this long after it finished, so it never flicks open and shut
export const DWELL_MS = 900;

export interface FoldInput {
  items: ProcessItem[];
  now: number;
  // The turn is working (or its fold is still settling after it stopped), and the reader wants details opened
  live: boolean;
  autoExpand: boolean;
  // The reader is at the bottom; away from it, what is open stays open
  following: boolean;
  // When each settled item was first seen settled (mutated); the first pass of a mounted turn dates them long ago
  doneAt: Map<string, number>;
  first: boolean;
  // Items the rule had open on the previous pass
  held: ReadonlySet<string>;
  manual: (id: string) => boolean | undefined;
}

// The automatic fold of each process item: the latest item is open while the turn works, a finished one closes once
// the next has started and it has been open DWELL_MS, and an item still running (a parallel call) stays open. A lone
// edit is a diff and never opens on its own: a block of code is too tall to open and close again as the next action
// starts. A manual choice wins. `wake` is when the next close falls due, for a turn whose stream has gone quiet
export function foldStates({ items, now, live, autoExpand, following, doneAt, first, held, manual }: FoldInput) {
  const tail = items.at(-1)?.id;
  const folds = new Map<string, boolean>();
  const open = new Set<string>();
  let wake = Infinity;
  for (const item of items) {
    if (itemSettled(item)) { if (!doneAt.has(item.id)) doneAt.set(item.id, first ? -Infinity : now); } else doneAt.delete(item.id);
    const until = (doneAt.get(item.id) ?? Infinity) + DWELL_MS;
    const diff = item.type === 'block' && item.block.type === 'tool_call' && item.block.kind === 'edit';
    let auto = live && autoExpand && !diff && (item.id === tail || now < until);
    if (auto && item.id !== tail && until !== Infinity) wake = Math.min(wake, until);
    if (!auto && !following && held.has(item.id)) auto = true;
    if (auto) open.add(item.id);
    folds.set(item.id, manual(item.id) ?? auto);
  }
  return { folds, open, wake };
}

export const filePath = (block: ToolCallBlock) => block.locations?.[0]?.path
  ?? visibleToolContents(block).find(c => c.type === 'diff')?.source?.path ?? block.target ?? '';
export const fileName = (block: ToolCallBlock) => filePath(block).split(/[\\/]/).pop() || block.target || '';

// Open only an unambiguous file; a multi-file patch title is not a destination.
export function editReference(blocks: ToolCallBlock[]): FileLink | undefined {
  const paths = [...new Set(blocks.flatMap(block => [
    ...(block.locations ?? []).map(location => location.path),
    ...visibleToolContents(block).flatMap(item => item.type === 'diff' && item.source?.path ? [item.source.path] : []),
  ]))];
  if (paths.length > 1) return;
  const file = paths.length ? { path: paths[0]! } : parseFileLink(blocks[0]?.target ?? '');
  if (!file) return;
  const span = editSpan(blocks);
  const location = blocks.flatMap(block => block.locations ?? []).find(location => location.path === file.path && location.line != null);
  const line = span ? Number(/^L(\d+)/.exec(span)![1]) : location?.line ?? file.line;
  return { path: file.path, ...(line != null ? { line } : {}) };
}

// The files a group names in its head, first-seen order
export function groupNames(blocks: ToolCallBlock[]): string[] {
  return [...new Set(blocks.map(fileName).filter(Boolean))];
}

// Consecutive edits of one file share an entry, their diffs stacked in call order
export function editEntries(blocks: ToolCallBlock[]): { key: string; blocks: ToolCallBlock[] }[] {
  const entries = new Map<string, ToolCallBlock[]>();
  for (const block of blocks) {
    const key = filePath(block) || block.id;
    entries.set(key, [...(entries.get(key) ?? []), block]);
  }
  return [...entries].map(([key, list]) => ({ key, blocks: list }));
}

// First to last changed line over the edits' diffs (added lines, or the removed ones for a pure deletion), e.g. L535–549
export function editSpan(blocks: ToolCallBlock[]): string | undefined {
  let lo = Infinity;
  let hi = -Infinity;
  for (const block of blocks) {
    for (const item of visibleToolContents(block)) {
      if (item.type !== 'diff') continue;
      const adds = item.lines.filter(l => l.kind === 'add' && l.newLine !== undefined).map(l => l.newLine!);
      for (const n of adds.length ? adds : item.lines.filter(l => l.kind === 'del' && l.oldLine !== undefined).map(l => l.oldLine!)) {
        lo = Math.min(lo, n);
        hi = Math.max(hi, n);
      }
    }
  }
  return lo === Infinity ? undefined : lo === hi ? `L${lo}` : `L${lo}–${hi}`;
}

export function diffStatOf(blocks: ToolCallBlock[]): { add: number; del: number } | undefined {
  if (!blocks.some(b => b.diffStat)) return;
  return blocks.reduce((s, b) => ({ add: s.add + (b.diffStat?.add ?? 0), del: s.del + (b.diffStat?.del ?? 0) }), { add: 0, del: 0 });
}
