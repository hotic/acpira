import type { PlanBlock, PlanEntry, Turn } from '@shared/transcript';
import { isTodoTool } from '@shared/todoTools';

export function samePlanEntries(a: PlanEntry[], b: PlanEntry[]): boolean {
  return a.length === b.length && a.every((entry, i) => {
    const other = b[i]!;
    return entry.title === other.title && entry.status === other.status && entry.priority === other.priority;
  });
}

export function lastPlanSnapshot(turns: Turn[]): PlanBlock | undefined {
  for (let i = turns.length - 1; i >= 0; i--) {
    const turn = turns[i]!;
    if (turn.role !== 'agent') continue;
    for (let j = turn.blocks.length - 1; j >= 0; j--) {
      const block = turn.blocks[j]!;
      if (block.type === 'plan') return block;
    }
  }
}

// Older hosts appended Grok's unchanged completed snapshot to every follow-up.
// Clean those history copies when opening a record, without changing turn indices
// or the original record. Explicit todo actions and known changes remain intact.
export function restorePlanSnapshots(turns: Turn[]): Turn[] {
  let previous: PlanBlock | undefined;
  return turns.map(turn => {
    if (turn.role !== 'agent') return turn;
    const edited = turn.blocks.some(b => b.type === 'tool_call' && isTodoTool(b));
    const blocks = turn.blocks.filter(block => {
      if (block.type !== 'plan') return true;
      const repeated = !block.changed && !edited && block.entries.length > 0
        && block.entries.every(e => e.status === 'completed')
        && previous && samePlanEntries(previous.entries, block.entries);
      previous = block;
      return !repeated;
    });
    return blocks.length === turn.blocks.length ? turn : { ...turn, blocks };
  });
}
