import type { AgentBlock, AgentTurn, SessionView, Turn } from './transcript';

// The changes since the session view a page was sent last (mirror of rust/crates/acpira-shared/src/session_patch.rs). The
// page keeps turns[0..keep) of the view at rev `base` and takes `turns` after them; with keepBlocks, turns[0] is the head of
// turn `keep` (its blocks empty) and that turn's blocks are the first keepBlocks of the base turn followed by `blocks`.
// `view` is the whole view apart from its turns
export interface SessionPatch {
  id: string;
  base: number;
  view: SessionView;
  keep: number;
  turns: Turn[];
  keepBlocks?: number;
  blocks?: AgentBlock[];
}

// The view a patch leads to, or undefined when the page does not hold the view it was computed against
export function applySessionPatch(current: SessionView | undefined, p: SessionPatch): SessionView | undefined {
  if (!current || current.id !== p.id || current.rev !== p.base || current.turns.length < p.keep) return undefined;
  const tail = p.turns.slice();
  if (p.keepBlocks !== undefined) {
    const base = current.turns[p.keep];
    const head = tail[0];
    if (base?.role !== 'agent' || head?.role !== 'agent' || base.blocks.length < p.keepBlocks) return undefined;
    tail[0] = { ...head, blocks: base.blocks.slice(0, p.keepBlocks).concat(p.blocks ?? []) };
  }
  return { ...p.view, turns: current.turns.slice(0, p.keep).concat(tail) };
}

// One patch with the effect of `a` followed by `b` (b computed against the view a leads to), or undefined when they do not
// chain. Lets a queue that is behind hold one patch instead of a growing run of them
export function mergeSessionPatches(a: SessionPatch, b: SessionPatch): SessionPatch | undefined {
  if (a.id !== b.id || b.base !== a.view.rev) return undefined;
  // b does not reach into what a changed: a's base view carries it
  if (b.keep < a.keep) return { ...b, base: a.base };
  if (b.keep === a.keep) {
    if (b.keepBlocks === undefined) return { ...b, base: a.base };
    const head = a.turns[0];
    if (head?.role !== 'agent') return undefined;
    // a sent turn `keep` whole: b's kept blocks are a's
    if (a.keepBlocks === undefined) {
      if (head.blocks.length < b.keepBlocks) return undefined;
      return { ...b, base: a.base, keepBlocks: undefined, blocks: undefined, turns: [{ ...b.turns[0] as AgentTurn, blocks: head.blocks.slice(0, b.keepBlocks).concat(b.blocks ?? []) }, ...b.turns.slice(1)] };
    }
    // both kept blocks of the base turn: the shorter run is what the base still has to supply
    if (b.keepBlocks <= a.keepBlocks) return { ...b, base: a.base };
    const extra = b.keepBlocks - a.keepBlocks;
    if ((a.blocks ?? []).length < extra) return undefined;
    return { ...b, base: a.base, keepBlocks: a.keepBlocks, blocks: (a.blocks ?? []).slice(0, extra).concat(b.blocks ?? []) };
  }
  // b keeps some of a's turns: those come from a's tail, whose first entry may still lean on the base view
  const j = b.keep - a.keep;
  if (a.turns.length < j) return undefined;
  let turns = b.turns;
  if (b.keepBlocks !== undefined) {
    const from = a.turns[j];
    const head = b.turns[0];
    if (from?.role !== 'agent' || head?.role !== 'agent' || from.blocks.length < b.keepBlocks) return undefined;
    turns = [{ ...head, blocks: from.blocks.slice(0, b.keepBlocks).concat(b.blocks ?? []) }, ...b.turns.slice(1)];
  }
  return { id: b.id, base: a.base, view: b.view, keep: a.keep, turns: a.turns.slice(0, j).concat(turns), keepBlocks: a.keepBlocks, blocks: a.blocks };
}
