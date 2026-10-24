import type { PlanEntry, Turn } from '@acpira/shared/transcript';
import { toolTodoEntries } from '@acpira/shared/todoTools';

// Open entries stay in the dock across turns. A finished list stays only while
// its own turn is still running, so a later prompt does not resurrect it.
export function dockPlan(turns: Turn[], running: boolean): PlanEntry[] | undefined {
  const found = latestPlan(turns);
  if (!found) return undefined;
  // A list written while planning is part of the plan, not work under way: the dock waits until that turn's plan is
  // built (approved / executing), as Cursor's to-dos appear with Build. A drafted, pending or revised plan keeps it hidden
  const turn = turns[found.index];
  if (turn?.role === 'agent' && turn.blocks.some(b => b.type === 'plan_document' && b.status !== 'approved' && b.status !== 'executing')) return undefined;
  if (found.entries.some(entry => entry.status !== 'completed')) return found.entries;
  if (running && found.index === turns.length - 1) return found.entries;
  return undefined;
}

// The most recent list the agent reported: every update replaces the whole list, so only the newest turn's list
// matters. Agents report it as an ACP `plan` update, as a todo tool's result (the built-in agent, OpenCode), or both
// (Grok). A turn's `plan` block is updated in place and so always holds that turn's newest plan, wherever it sits
// among the rows; it wins over the turn's todo rows, which only stand in when the agent sends no plan at all.
export function latestPlan(turns: Turn[]): { entries: PlanEntry[]; index: number } | undefined {
  for (let i = turns.length - 1; i >= 0; i--) {
    const turn = turns[i];
    if (turn?.role !== 'agent') continue;
    let todos: PlanEntry[] | undefined;
    for (let j = turn.blocks.length - 1; j >= 0; j--) {
      const b = turn.blocks[j];
      if (b?.type === 'plan') return { entries: b.entries, index: i };
      if (!todos && b?.type === 'tool_call') todos = toolTodoEntries(b);
    }
    if (todos) return { entries: todos, index: i };
  }
  return undefined;
}
