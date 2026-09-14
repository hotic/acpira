import type { AgentBlock, Turn } from '@shared/transcript';

function unfinished(block: AgentBlock): boolean {
  switch (block.type) {
    case 'text': case 'thought': return !!block.streaming;
    case 'tool_call': return block.status === 'pending' || block.status === 'in_progress';
    case 'compaction': return block.status === 'in_progress';
    case 'permission': return true;
    case 'question': return !block.outcome;
    default: return false;
  }
}

// A disk snapshot carries display history, never ownership of a live request or
// background shell. Restoring it cannot revive timers, approvals or streams.
// Use the last recorded timestamp; time spent with the IDE closed is not work.
export function restoreInterruptedTurns(turns: Turn[], updatedAt: string): Turn[] {
  let observedAt = Date.parse(updatedAt);
  if (!Number.isFinite(observedAt)) observedAt = 0;
  for (const turn of turns) {
    if (turn.role !== 'agent') continue;
    observedAt = Math.max(observedAt, turn.startedAt ?? 0, turn.endedAt ?? 0);
    for (const block of turn.blocks) {
      if (block.type === 'tool_call' || block.type === 'thought') {
        observedAt = Math.max(observedAt, block.startedAt ?? 0);
        if (block.type === 'tool_call') observedAt = Math.max(observedAt, block.endedAt ?? 0);
      }
    }
  }
  return turns.map(turn => {
    if (turn.role !== 'agent') return turn;
    const active = turn.blocks.some(unfinished);
    if (!active && (turn.stop || turn.startedAt === undefined)) return turn;
    const endedAt = turn.endedAt ?? (observedAt || undefined);
    const blocks = turn.blocks.filter(b => b.type !== 'permission').map((block): AgentBlock => {
      if (!unfinished(block)) return block;
      switch (block.type) {
        case 'text': return { ...block, streaming: false };
        case 'thought': return { ...block, streaming: false,
          ...(block.startedAt !== undefined && endedAt !== undefined
            ? { durationSec: Math.max(0, Math.round((endedAt - block.startedAt) / 1000)) } : {}) };
        case 'tool_call': return { ...block, status: 'cancelled',
          ...(block.startedAt !== undefined && endedAt !== undefined ? { endedAt: Math.max(block.startedAt, endedAt) } : {}) };
        case 'compaction': return { ...block, status: 'cancelled' };
        case 'question': return { ...block, outcome: 'cancelled' };
        default: return block;
      }
    });
    return { ...turn, blocks, activity: undefined, stop: turn.stop ?? 'cancelled',
      ...(turn.startedAt !== undefined ? { endedAt } : {}) };
  });
}
