import { describe, expect, it } from 'vitest';
import { applyChatGptEvent, chatgptSessionId, chatgptView, STALE_AFTER_MS, type ChatGptRecord } from '../src/host/external/chatgptEvents';
import { commanderFacts } from '../src/host/external/desktopCommanderStatus';

function fixture() {
  const at = new Date(0).toISOString();
  let r: ChatGptRecord = { version: 1, id: chatgptSessionId('fixture'), sourceKey: 'fixture', title: 'Fixture', cwd: '/fixture',
    createdAt: at, updatedAt: at, lastEventAt: at, revision: 1, turns: [], receipts: {} };
  let seq = 0;
  return { send: (body: object, now = 1) => r = applyChatGptEvent(r, { id: `e${++seq}`, turnId: 'a', ...body }, now), get: () => r };
}

describe('ChatGPT continuation and truthful transport status', () => {
  it('does not call an empty receiver connected', () => {
    expect(chatgptView(fixture().get(), 0).external?.state).toBe('unbound');
  });
  it('retries the same prompt without duplicating or restarting a completed turn', () => {
    const f = fixture(); f.send({ type: 'turn_start', text: 'hello' });
    f.send({ type: 'turn_end', stop: 'end_turn' });
    f.send({ type: 'turn_start', text: 'hello' });
    expect(f.get().turns).toHaveLength(2);
    expect(f.get().activeTurnId).toBeUndefined();
    expect(() => f.send({ type: 'turn_start', text: 'changed' })).toThrow('different text');
  });
  it('requires the exact previous turn and preserves its unconfirmed outcome', () => {
    const f = fixture(); f.send({ type: 'turn_start', text: 'first' });
    expect(() => f.send({ type: 'turn_start', turnId: 'b', text: 'next' })).toThrow('previous-turn');
    expect(() => f.send({ type: 'turn_start', turnId: 'b', text: 'next', previousTurnId: 'wrong' })).toThrow();
    f.send({ type: 'turn_start', turnId: 'b', text: 'next', previousTurnId: 'a' });
    const view = chatgptView(f.get(), 2);
    expect(view.turns).toHaveLength(4);
    expect(view.turns[1]).toMatchObject({ observation: 'unknown' });
    expect(view.turns[1]).not.toHaveProperty('stop');
    expect(view.turns[1]).not.toHaveProperty('endedAt');
    expect(view.running).toBe(true);
  });
  it('routes late command output to its original turn without renewing the new turn lease', () => {
    const f = fixture(); f.send({ type: 'turn_start', text: 'first' });
    f.send({ type: 'tool_start', callId: 'shell', name: 'Shell', kind: 'execute' });
    f.send({ type: 'turn_start', turnId: 'b', text: 'next', previousTurnId: 'a' }, 2);
    f.send({ type: 'heartbeat' }, STALE_AFTER_MS + 9);
    f.send({ type: 'tool_output', callId: 'shell', text: 'late output' }, STALE_AFTER_MS + 10);
    const view = chatgptView(f.get(), STALE_AFTER_MS + 10);
    expect(view.external?.state).toBe('stale');
    expect(view.turns[1]).toMatchObject({ blocks: [{ content: { text: 'late output' }, observation: 'unknown' }] });
    expect(view.turns[3]).toMatchObject({ blocks: [] });
    f.send({ type: 'tool_end', callId: 'shell', status: 'completed' });
    f.send({ type: 'turn_end', stop: 'end_turn' });
    expect(f.get().activeTurnId).toBe('b');
    expect(f.get().turns[1]).toMatchObject({ stop: 'end_turn' });
  });
  it('resumes observation without replaying prompts or tools', () => {
    const f = fixture(); f.send({ type: 'turn_start', text: 'first' });
    expect(chatgptView(f.get(), STALE_AFTER_MS + 10).external?.state).toBe('stale');
    f.send({ type: 'turn_resume' }, STALE_AFTER_MS + 20);
    expect(chatgptView(f.get(), STALE_AFTER_MS + 21).running).toBe(true);
    expect(f.get().turns).toHaveLength(2);
    f.send({ type: 'turn_end', stop: 'end_turn' });
    expect(() => f.send({ type: 'turn_resume' })).toThrow('unfinished');
  });
  it('does not interpret stopping generation as terminating a local process', () => {
    const f = fixture(); f.send({ type: 'turn_start', text: 'first' });
    f.send({ type: 'tool_start', callId: 'shell', name: 'Shell', kind: 'execute' });
    f.send({ type: 'turn_end', stop: 'cancelled' });
    expect(chatgptView(f.get(), 2).turns[1]).toMatchObject({ stop: 'cancelled', blocks: [{ status: 'in_progress', observation: 'unknown' }] });
    f.send({ type: 'tool_end', callId: 'shell', status: 'completed' });
    expect(chatgptView(f.get(), 3).turns[1]).toMatchObject({ stop: 'cancelled', blocks: [{ status: 'completed' }] });
  });
  it('never claims paired from a process, executable or stale configuration', () => {
    expect(commanderFacts(false, true, false)).toMatchObject({ installation: 'unknown', pairing: 'unknown', evidence: 'configuration' });
    expect(commanderFacts(true, true, true)).toMatchObject({ installation: 'detected', pairing: 'unknown' });
    expect(commanderFacts(false, false, false)).toMatchObject({ installation: 'not_detected', pairing: 'unknown' });
    expect(commanderFacts(false, false, undefined)).toMatchObject({ installation: 'unknown', process: 'unknown' });
  });
});
