import { describe, expect, it } from 'vitest';
import { restoreInterruptedTurns } from '../src/host/acp/restoreTurns';
import type { Turn } from '../src/shared/transcript';

describe('restoring persisted execution state', () => {
  it('withdraws orphaned approvals and questions while keeping the task plan', () => {
    const turns: Turn[] = [{ role: 'agent', blocks: [
      { type: 'permission', id: 'p', title: 'Run command', options: [] },
      { type: 'question', id: 'q', questions: [] },
      { type: 'plan', entries: [{ title: 'Continue project', status: 'in_progress' }] },
    ] }];
    const restored = restoreInterruptedTurns(turns, new Date(5000).toISOString());
    expect(restored[0]).toMatchObject({ stop: 'cancelled', blocks: [
      { type: 'question', outcome: 'cancelled' },
      { type: 'plan', entries: [{ status: 'in_progress' }] },
    ] });
    expect(turns[0]).not.toHaveProperty('stop');
  });

  it('stops residual background commands in completed turns without changing the turn result', () => {
    const turns: Turn[] = [{ role: 'agent', startedAt: 1000, endedAt: 3000, stop: 'end_turn', blocks: [
      { type: 'tool_call', id: 'server', kind: 'execute', verb: 'Run', background: true, status: 'in_progress', startedAt: 2000 },
    ] }];
    expect(restoreInterruptedTurns(turns, new Date(9000).toISOString())[0]).toMatchObject({ stop: 'end_turn', endedAt: 3000,
      blocks: [{ status: 'cancelled', endedAt: 3000 }] });
  });

  it('preserves healthy history and uses the latest recorded activity when list time predates a long turn', () => {
    const complete: Turn = { role: 'agent', blocks: [{ type: 'text', markdown: 'Done' }], stop: 'end_turn' };
    const active: Turn = { role: 'agent', startedAt: 2000, blocks: [
      { type: 'tool_call', id: 'run', kind: 'execute', verb: 'Run', status: 'in_progress', startedAt: 7000 },
    ] };
    const restored = restoreInterruptedTurns([complete, active], new Date(1000).toISOString());
    expect(restored[0]).toBe(complete);
    expect(restored[1]).toMatchObject({ stop: 'cancelled', endedAt: 7000, blocks: [{ status: 'cancelled', endedAt: 7000 }] });
  });
});
