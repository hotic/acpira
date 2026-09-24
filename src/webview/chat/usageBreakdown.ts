import type { Turn, Usage } from '@shared/transcript';

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
