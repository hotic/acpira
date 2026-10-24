import { describe, expect, it } from 'vitest';
import type { QueuedPrompt, Turn } from '@shared/transcript';
import { heldPrompt } from './heldPrompt';

const entry = (id: string, text = id): QueuedPrompt => ({ id, text, attachments: [] });

describe('heldPrompt', () => {
  it('shows the head of a queue held while the session starts as a sent prompt with a working reply', () => {
    const view = heldPrompt([], false, 'starting', [entry('a', 'hello'), entry('b')]);
    expect(view.running).toBe(true);
    expect(view.turns).toEqual([{ role: 'user', id: 'a', text: 'hello' }, { role: 'agent', blocks: [] }]);
    expect(view.queued?.map(q => q.id)).toEqual(['b']);
  });

  it('does the same on a ready session whose queue waits for the controls replay', () => {
    const turns: Turn[] = [{ role: 'user', text: 'x' }, { role: 'agent', blocks: [], stop: 'end_turn' }];
    const view = heldPrompt(turns, false, 'ready', [entry('a')]);
    expect(view.turns).toHaveLength(4);
    expect(view.queued).toBeUndefined();
  });

  it('leaves a running turn, a claimed entry and a failed session alone', () => {
    const turns: Turn[] = [];
    const queued = [entry('a')];
    for (const view of [
      heldPrompt(turns, true, 'ready', queued),
      heldPrompt(turns, false, 'ready', [{ ...entry('a'), sending: true }]),
      heldPrompt(turns, false, 'error', queued),
      heldPrompt(turns, false, 'ready', undefined),
    ]) expect(view.turns).toBe(turns);
  });
});
