import { describe, expect, it } from 'vitest';
import { LIVE_MIN, buildTrace, domain, segments, tickStep } from '../packages/shared/trace';
import type { AgentTurn, ToolCallBlock } from '../packages/shared/transcript';
import type { SubagentSummary } from '../packages/shared/subagents';

const O = 1_000_000;
const tool = (id: string, from: number, to?: number, extra: Partial<ToolCallBlock> = {}): ToolCallBlock => ({
  type: 'tool_call', id, kind: 'execute', verb: 'Run', status: to === undefined ? 'in_progress' : 'completed',
  startedAt: O + from, ...(to !== undefined ? { endedAt: O + to } : {}), ...extra,
});
const node = (id: string, extra: Partial<SubagentSummary> = {}): SubagentSummary => ({
  id, turnIndex: 1, visibility: 'session', state: 'completed', stateSource: 'agent', controls: { cancel: false },
  announcedAt: O + 1000, endedAt: O + 5000, peer: {}, toolCount: 0, ...extra,
});

describe('buildTrace', () => {
  it('puts a subagent where its delegation call was, with its own calls and nested children under it', () => {
    const turn: AgentTurn = { role: 'agent', startedAt: O, endedAt: O + 9000, stop: 'end_turn',
      blocks: [tool('a', 100, 600), tool('d', 900, 6000, { kind: 'other', subagentId: 'x' })] };
    const trace = buildTrace({
      turn, turnIndex: 1, subagents: [node('x'), node('y', { parentId: 'x', announcedAt: O + 3000, endedAt: O + 4000 })],
      childTurns: id => (id === 'x' ? [{ role: 'agent', blocks: [tool('b', 1200, 2000, { status: 'failed' })] }] : undefined),
    })!;
    // The delegation call 'd' became subagent x and keeps its id
    expect(trace.root.children.map(c => c.id)).toEqual(['a', 'd']);
    const x = trace.root.children[1]!;
    // The delegation call's own start wins over the announcement
    expect(x.start).toBe(900);
    expect(x.children.map(c => c.id)).toEqual(['b', 'subagent:y']);
    expect(trace.root.size).toBe(4);
    expect(trace.stats).toMatchObject({ tools: 2, subagents: 2, errors: 1 });
  });

  it('closes a call the ended turn left open, but not a background command', () => {
    const turn: AgentTurn = { role: 'agent', startedAt: O, endedAt: O + 5000, stop: 'cancelled',
      blocks: [tool('open', 100), tool('bg', 200, undefined, { background: true })] };
    const [open, bg] = buildTrace({ turn, turnIndex: 0 })!.root.children;
    expect(open).toMatchObject({ state: 'cancelled', end: 5000 });
    expect(bg!.state).toBe('running');
    expect(bg!.end).toBeUndefined();
  });

  it('has no trace for a turn without any clock', () => {
    expect(buildTrace({ turn: { role: 'agent', blocks: [{ type: 'tool_call', id: 't', kind: 'read', verb: 'Read', status: 'completed' }] }, turnIndex: 0 })).toBeUndefined();
  });
});

describe('segments', () => {
  it('merges overlapping children under the strongest tone and fills the gaps with model time', () => {
    const turn: AgentTurn = { role: 'agent', startedAt: O, endedAt: O + 10_000, stop: 'end_turn',
      blocks: [tool('a', 1000, 3000), tool('b', 2000, 4000, { status: 'failed' }), tool('c', 6000, 7000)] };
    const root = buildTrace({ turn, turnIndex: 0 })!.root;
    expect(segments(root, 10_000)).toEqual([
      { from: 0, to: 1000, tone: 'model' },
      { from: 1000, to: 4000, tone: 'fail' },
      { from: 4000, to: 6000, tone: 'model' },
      { from: 6000, to: 7000, tone: 'tool' },
      { from: 7000, to: 10_000, tone: 'model' },
    ]);
  });
});

describe('domain', () => {
  it('sweeps a fixed window first, then grows with the clock', () => {
    expect(domain(2000, true)).toBe(LIVE_MIN);
    expect(domain(60_000, true)).toBe(60_000);
    // Finished, the axis is the trace
    expect(domain(2000, false)).toBe(2000);
  });

  it('picks the smallest step that keeps the ticks within the limit', () => {
    expect(tickStep(11_500)).toBe(5000);
    expect(tickStep(11_500, 2)).toBe(10_000);
  });
});

describe('open segments', () => {
  it('marks what runs up to now: a running child, and model time after the last child of a running span', () => {
    const turn: AgentTurn = { role: 'agent', startedAt: O, blocks: [tool('a', 1000, 2000), tool('b', 3000)] };
    const root = buildTrace({ turn, turnIndex: 0 })!.root;
    expect(segments(root, 5000)).toEqual([
      { from: 0, to: 1000, tone: 'model' },
      { from: 1000, to: 2000, tone: 'tool' },
      { from: 2000, to: 3000, tone: 'model' },
      { from: 3000, to: 5000, tone: 'tool', open: true },
    ]);
    const quiet: AgentTurn = { role: 'agent', startedAt: O, blocks: [tool('a', 1000, 2000)] };
    expect(segments(buildTrace({ turn: quiet, turnIndex: 0 })!.root, 5000).at(-1)).toEqual({ from: 2000, to: 5000, tone: 'model', open: true });
  });
});
