import { describe, expect, it } from 'vitest';
import type { AgentTurn, PlanBlock, Turn } from '../src/shared/transcript';
import { applyUpdate, emptyState, endTurn } from '../src/host/acp/normalize';
import { restorePlanSnapshots } from '../src/host/acp/planSnapshots';
import { dockPlan } from '../src/webview/chat/dockPlan';

const completed = { content: 'Implement feature', priority: 'medium' as const, status: 'completed' as const };
const snapshot = (status = completed.status) => ({ sessionUpdate: 'plan' as const, entries: [{ ...completed, status }] });
const done: PlanBlock = { type: 'plan', entries: [{ title: completed.content, priority: completed.priority, status: 'completed' }] };

describe('session plan snapshots', () => {
  it('ignores identical snapshots across follow-ups without interrupting prose or showing the dock', () => {
    const state = emptyState();
    applyUpdate(state, snapshot());
    endTurn(state, 'end_turn');
    for (let i = 0; i < 3; i++) {
      state.turns.push({ role: 'user', text: 'A follow-up question' });
      // A snapshot arriving before the reply must not create an empty agent turn.
      expect(applyUpdate(state, snapshot())).toBe(false);
      expect(state.turns.at(-1)!.role).toBe('user');
      applyUpdate(state, { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Reply' } });
      expect(applyUpdate(state, snapshot())).toBe(false);
      applyUpdate(state, { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: ' continued.' } });
      expect((state.turns.at(-1) as AgentTurn).blocks).toEqual([{ type: 'text', markdown: 'Reply continued.', streaming: true }]);
      expect(dockPlan(state.turns, true)).toBeUndefined();
      endTurn(state, 'end_turn');
    }
  });

  it('keeps real progress even when the new turn ends at the previous completed state', () => {
    const state = emptyState();
    applyUpdate(state, snapshot());
    endTurn(state, 'end_turn');
    state.turns.push({ role: 'user', text: 'Run the same work again' });
    applyUpdate(state, { sessionUpdate: 'plan', entries: [{ ...completed, status: 'in_progress' }] });
    expect(dockPlan(state.turns, true)?.entries[0]?.status).toBe('in_progress');
    applyUpdate(state, snapshot());
    endTurn(state, 'end_turn');
    const restored = restorePlanSnapshots(JSON.parse(JSON.stringify(state.turns)) as Turn[]);
    expect(restored.flatMap(t => t.role === 'agent' ? t.blocks.filter(b => b.type === 'plan') : [])).toHaveLength(2);
  });

  it('retains changes to content, priority, order and list clearing', () => {
    const state = emptyState();
    applyUpdate(state, snapshot());
    for (const entries of [
      [{ ...completed, content: 'Another feature' }],
      [{ ...completed, priority: 'high' as const }],
      [completed, { ...completed, content: 'Second' }],
      [{ ...completed, content: 'Second' }, completed],
      [],
    ]) expect(applyUpdate(state, { sessionUpdate: 'plan', entries })).toBe(true);
    expect(applyUpdate(state, { sessionUpdate: 'plan', entries: [] })).toBe(false);
  });

  it('removes legacy completed echoes without mutating records or changing turn indices', () => {
    const first: AgentTurn = { role: 'agent', blocks: [done], stop: 'end_turn' };
    const reply: AgentTurn = { role: 'agent', blocks: [{ type: 'text', markdown: 'Answer.' }, structuredClone(done)], stop: 'end_turn' };
    const turns: Turn[] = [first, { role: 'user', text: 'Follow-up' }, reply];
    const restored = restorePlanSnapshots(turns);
    expect(restored).toHaveLength(turns.length);
    expect(restored[0]).toBe(first);
    expect(restored[1]).toBe(turns[1]);
    expect((restored[2] as AgentTurn).blocks).toEqual([reply.blocks[0]]);
    expect(reply.blocks).toHaveLength(2);
    expect(dockPlan(restored, true)).toBeUndefined();
  });

  it('preserves legacy todo edits and updates after an empty snapshot', () => {
    const turns: Turn[] = [
      { role: 'agent', blocks: [done] },
      { role: 'agent', blocks: [{ type: 'tool_call', id: 'todo', kind: 'other', verb: 'Todo', verbKey: 'verb.todo', status: 'completed' }, done] },
      { role: 'agent', blocks: [{ type: 'plan', entries: [] }] },
      { role: 'agent', blocks: [done] },
    ];
    expect(restorePlanSnapshots(turns)).toEqual(turns);
  });
});
