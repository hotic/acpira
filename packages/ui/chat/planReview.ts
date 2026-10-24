import type { PermissionBlock, Turn } from '@acpira/shared/transcript';

// A plan approval's Revise choice: the reject-once option that keeps the agent in Plan mode.
// Kimi's reject-and-exit is a different decision and stays in the card's extra approvals.
export function reviseOption(permission: PermissionBlock) {
  return permission.options.find(o => o.kind === 'reject_once' && !/exit/i.test(o.id) && !/退出/.test(o.label));
}

// The plan approval the newest agent turn is blocked on. Its plan card carries the decision, so while it waits
// the turn reads as finished (no working row, no to-do dock) and the composer revises the plan instead of steering
export function pendingPlanReview(turns: Turn[]): PermissionBlock | undefined {
  const last = turns.at(-1);
  if (last?.role !== 'agent') return undefined;
  return last.blocks.find((b): b is PermissionBlock => b.type === 'permission' && !!b.planId);
}
