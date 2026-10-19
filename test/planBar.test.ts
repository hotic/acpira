import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { PlanBlock, PlanEntry, ToolCallBlock, Turn } from '../src/shared/transcript';
import { dockPlan } from '../src/webview/chat/dockPlan';

const entry = (title: string, status: PlanEntry['status']): PlanEntry => ({ title, status });
const plan = (...entries: PlanEntry[]): PlanBlock => ({ type: 'plan', entries });
const done = plan(entry('A', 'completed'), entry('B', 'completed'));
const open = plan(entry('A', 'completed'), entry('B', 'pending'));
// A todo tool's row as the engine stores it, the parsed list on `todoEntries`
const todoRow = (entries: PlanEntry[], status: ToolCallBlock['status'] = 'completed'): ToolCallBlock =>
  ({ type: 'tool_call', id: 'todo', kind: 'other', verb: 'Update todos', verbKey: 'verb.todo', status, todoEntries: entries });
const shellSource = readFileSync(join(import.meta.dirname, '../src/webview/chat/Shell.tsx'), 'utf8');
const planBarSource = readFileSync(join(import.meta.dirname, '../src/webview/chat/PlanBar.tsx'), 'utf8');

describe('PlanBar dock visibility', () => {
  it('hides a finished list once its turn has ended', () => {
    const turns: Turn[] = [
      { role: 'user', text: 'do the work' },
      { role: 'agent', blocks: [done] },
    ];
    expect(dockPlan(turns, false)).toBeUndefined();
  });

  it('keeps a finished list pinned while that same turn is still running', () => {
    const turns: Turn[] = [
      { role: 'user', text: 'do the work' },
      { role: 'agent', blocks: [done] },
    ];
    expect(dockPlan(turns, true)).toBe(done.entries);
  });

  it('does not resurrect a finished list when a later prompt starts running', () => {
    const turns: Turn[] = [
      { role: 'user', text: 'do the work' },
      { role: 'agent', blocks: [done] },
      { role: 'user', text: 'rewrite the commit message' },
      { role: 'agent', blocks: [] },
    ];
    expect(dockPlan(turns, true)).toBeUndefined();
    expect(dockPlan(turns, false)).toBeUndefined();
  });

  it('keeps an unfinished list in the dock across a new prompt', () => {
    const turns: Turn[] = [
      { role: 'user', text: 'do the work' },
      { role: 'agent', blocks: [open] },
      { role: 'user', text: 'keep going' },
      { role: 'agent', blocks: [] },
    ];
    expect(dockPlan(turns, true)).toBe(open.entries);
    // Completing a later follow-up does not complete the preceding plan.
    expect(dockPlan(turns, false)).toBe(open.entries);
    expect(dockPlan(turns.slice(0, 2), false)).toBe(open.entries);
  });

  it('follows a new plan on the live turn instead of the finished predecessor', () => {
    const next = plan(entry('Rewrite the message', 'in_progress'));
    const turns: Turn[] = [
      { role: 'user', text: 'do the work' },
      { role: 'agent', blocks: [done] },
      { role: 'user', text: 'rewrite the commit message' },
      { role: 'agent', blocks: [next] },
    ];
    expect(dockPlan(turns, true)).toBe(next.entries);
  });

  it('docks a todo tool result when the agent sends no plan update', () => {
    // The built-in agent and OpenCode report the list only through their todo tool
    const row = todoRow(open.entries);
    const turns: Turn[] = [
      { role: 'user', text: 'do the work' },
      { role: 'agent', blocks: [row, { type: 'tool_call', id: 'run', kind: 'execute', verb: 'Run', status: 'completed' }] },
    ];
    expect(dockPlan(turns, true)).toBe(open.entries);
    expect(dockPlan(turns, false)).toBe(open.entries);
    // A finished list follows the same rule as a plan block
    expect(dockPlan([{ role: 'agent', blocks: [todoRow(done.entries)] }], false)).toBeUndefined();
    expect(dockPlan([{ role: 'agent', blocks: [todoRow(done.entries)] }], true)).toBe(done.entries);
  });

  it('takes the newest todo row of the turn and ignores rows still running', () => {
    const next = [entry('A', 'completed'), entry('B', 'in_progress')];
    const turns: Turn[] = [{ role: 'agent', blocks: [todoRow(open.entries), todoRow(next), todoRow([entry('C', 'pending')], 'in_progress')] }];
    expect(dockPlan(turns, true)).toBe(next);
  });

  it('prefers the turn\'s plan block over its todo rows and a later turn\'s list over an earlier one', () => {
    // Grok sends both; the plan block is updated in place, so it holds the newest list even above a later row
    const stale = todoRow([entry('A', 'in_progress'), entry('B', 'pending')]);
    expect(dockPlan([{ role: 'agent', blocks: [open, stale] }], true)).toBe(open.entries);
    const later = [entry('Next', 'pending')];
    const turns: Turn[] = [
      { role: 'agent', blocks: [open] },
      { role: 'user', text: 'keep going' },
      { role: 'agent', blocks: [todoRow(later)] },
    ];
    expect(dockPlan(turns, true)).toBe(later);
  });

  it('renders the live plan over the thread without shrinking its scroll viewport', () => {
    const thread = shellSource.indexOf('<Thread key=');
    const dock = shellSource.indexOf('data-plan-dock');
    const composer = shellSource.indexOf('<DockStack className=', dock);
    expect(thread).toBeGreaterThan(-1);
    expect(dock).toBeGreaterThan(thread);
    expect(composer).toBeGreaterThan(dock);
    expect(shellSource).toContain("'pointer-events-none absolute inset-x-0 bottom-0 z-10'");
    // Dock height reaches the transcript as a direct padding write on the content element. An
    // inherited custom property on the shared ancestor would restyle the whole transcript on
    // every resize frame, so no ancestor channel may exist.
    expect(shellSource).toContain('content.style.paddingBottom');
    // The clearance keeps the message gap on top of the dock height, so the last message sits one
    // message gap from the first dock surface with or without the plan
    expect(shellSource).toContain('calc(var(--msg-gap) + ${dockHeight}px)');
    expect(shellSource).not.toContain('threadArea');
    expect(planBarSource).toContain('pointer-events-none px-dock pb-(--dock-gap)');
    expect(planBarSource).toContain('pointer-events-auto');
  });
});
