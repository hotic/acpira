import type { Turn, Usage } from '@acpira/shared/transcript';
import { compactionForDisplay } from './compactionDisplay';

// The compaction policy does not change the model's context window.
export function usageWindow(size: number): number {
  return size;
}

// A budget strictly inside the agent window, shown as a marker — not a replacement for `size`
export function compactBudget(size: number, compactAt?: number): number | undefined {
  if (!compactAt || compactAt <= 0 || compactAt >= size) return undefined;
  return compactAt;
}

export function overCompactBudget(used: number, compactAt?: number): boolean {
  return !!compactAt && compactAt > 0 && used >= compactAt;
}

// Retained UI history includes compacted messages and unbounded tool output.
// It cannot measure the native context, even while the agent is running.
export function liveUsage(usage: Usage, _turns: Turn[], _running?: boolean): Usage {
  return usage;
}

// Ring / bar fill for a snapshot that predates a finished compaction: a deliberately tiny sliver, shown dimmed, until
// the agent's next report replaces it with the real figure
export const STALE_FILL = 0.02;

export function usageFill(usage: Usage): number {
  if (usage.compactedFrom !== undefined) return STALE_FILL;
  return usage.size > 0 ? Math.min(1, usage.used / usage.size) : 0;
}

const COMPACT = /^\/compact(?:\s|$)/;

// Whether a compaction is under way in the running turn: a /compact reply that has not settled (also before its
// first chunk, and Devin / Kimi in the background after acknowledging), or an agent's own mid-turn compaction row.
// A follow-up accepted meanwhile may already sit below it, so the latest agent turn is the one read
export function compactingNow(turns: Turn[], running?: boolean): boolean {
  if (!running) return false;
  let i = turns.length - 1;
  for (; i >= 0; i--) {
    const t = turns[i]!;
    if (t.role !== 'user') break;
    if (i === turns.length - 1 && COMPACT.test(t.text.trim())) return true;
  }
  const turn = turns[i];
  if (turn?.role !== 'agent' || turn.stop) return false;
  const prev = turns[i - 1];
  const shown = prev?.role === 'user' && COMPACT.test(prev.text.trim()) ? compactionForDisplay(turn, true) : turn;
  return shown.blocks.some(b => b.type === 'compaction' && b.status === 'in_progress');
}
