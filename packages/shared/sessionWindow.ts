import type { AgentBlock, SessionView, Turn, ViewEarlier } from './transcript';

// Windowed session views (mirror of rust/crates/acpira-shared/src/session_window.rs). A page that said `window` in ready
// gets a view that starts at its tail, `earlier` naming what is left out, and asks for the part right ahead of what it
// holds with loadEarlier. Patches need nothing new: the engine counts their indices inside the page's window

// One answer to loadEarlier. `from` is the window the chunk was cut for: the page takes it only while its view of `id`
// still starts there. `blocks` go ahead of the blocks of turns[0] (the cut turn), then `turns` ahead of the turns; the
// first of `turns` is itself cut when `earlier.blocks` is not 0. `earlier` is what is still left out, absent once the view
// is whole
export interface SessionEarlier {
  id: string;
  from: ViewEarlier;
  turns: Turn[];
  blocks?: AgentBlock[];
  earlier?: ViewEarlier;
}

const WHOLE: ViewEarlier = { turns: 0, blocks: 0 };

// The view with the chunk in front, or undefined when the chunk was cut for another window (a whole view landed since
// the page asked): the page drops it and asks again if it still wants more. The rev stays: nothing in the session changed
export function applySessionEarlier(current: SessionView | undefined, c: SessionEarlier): SessionView | undefined {
  if (!current || current.id !== c.id) return undefined;
  const at = current.earlier ?? WHOLE;
  if (at.turns !== c.from.turns || at.blocks !== c.from.blocks) return undefined;
  let held = current.turns;
  if (c.blocks?.length) {
    const first = held[0];
    if (first?.role !== 'agent') return undefined;
    held = [{ ...first, blocks: c.blocks.concat(first.blocks) }, ...held.slice(1)];
  }
  const { earlier: _, ...rest } = current;
  return { ...rest, ...(c.earlier ? { earlier: c.earlier } : {}), turns: c.turns.concat(held) };
}

// A turn's index in the session, as protocol messages count it, from its index in the view the page holds
export function sessionTurnIndex(view: Pick<SessionView, 'earlier'>, index: number): number {
  return index + (view.earlier?.turns ?? 0);
}
