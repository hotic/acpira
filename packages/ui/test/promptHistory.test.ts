import { describe, expect, it } from 'vitest';
import type { Turn } from '@acpira/shared/transcript';
import { promptHistory, PromptRecall } from '../chat/promptHistory';

describe('promptHistory', () => {
  it('keeps typed prompts, steers and queued messages in order, not internal instructions or replies', () => {
    const turns: Turn[] = [
      { role: 'user', text: '  first\nline  ', attachments: [{ kind: 'file', uri: 'file:///repo/a.ts', name: 'a.ts' }] },
      { role: 'agent', blocks: [{ type: 'text', markdown: 'reply' }, { type: 'steer', id: 's', text: 'follow up' }] },
      { role: 'user', text: '/compact', auto: true },
      { role: 'user', text: 'continue', auto: true, autoReason: 'retry' },
      { role: 'user', text: 'build the plan', planId: 'plan' },
      { role: 'user', text: '', attachments: [{ kind: 'image', mimeType: 'image/png', blob: 'image.png' }] },
      { role: 'user', text: ' \n ' },
      { role: 'user', text: '/compact' },
    ];
    expect(promptHistory(turns, [{ id: 'q', text: 'queued', attachments: [] }]))
      .toEqual(['  first\nline  ', 'follow up', '/compact', 'queued']);
  });

  it('derives history only from the supplied session, retaining repeated messages', () => {
    expect(promptHistory([{ role: 'user', text: 'A' }, { role: 'user', text: 'A' }])).toEqual(['A', 'A']);
    expect(promptHistory([{ role: 'user', text: 'B' }])).toEqual(['B']);
    expect(promptHistory([])).toEqual([]);
  });
});

describe('PromptRecall', () => {
  const history = ['older', 'newer'];

  it('browses both ways without wrapping, then returns to the empty draft', () => {
    const recall = new PromptRecall();
    expect(recall.move(history, '', -1)).toBe('newer');
    expect(recall.move(history, 'newer', -1)).toBe('older');
    expect(recall.move(history, 'older', -1)).toBe('older');
    expect(recall.move(history, 'older', 1)).toBe('newer');
    expect(recall.move(history, 'newer', 1)).toBe('');
    expect(recall.move(history, '', 1)).toBeUndefined();
  });

  it('does not replace an existing draft or start with Down', () => {
    const recall = new PromptRecall();
    expect(recall.move(history, 'unsent draft', -1)).toBeUndefined();
    expect(recall.move(history, ' ', -1)).toBeUndefined();
    expect(recall.move(history, '', 1)).toBeUndefined();
    expect(recall.move([], '', -1)).toBeUndefined();
  });

  it('stops browsing after edits, including edits undone back to the recalled words', () => {
    const recall = new PromptRecall();
    expect(recall.move(history, '', -1)).toBe('newer');
    expect(recall.move(history, 'newer edited', -1)).toBeUndefined();
    expect(recall.move(history, 'newer edited', 1)).toBeUndefined();
    expect(recall.move(history, '', -1)).toBe('newer');
    recall.reset();
    expect(recall.move(history, 'newer', -1)).toBeUndefined();
    expect(recall.move(history, 'newer', 1)).toBeUndefined();
  });

  it('holds the browsing snapshot while live messages change, refreshing on the next recall', () => {
    const recall = new PromptRecall();
    expect(recall.move(history, '', -1)).toBe('newer');
    const updated = [...history, 'latest'];
    expect(recall.move(updated, 'newer', -1)).toBe('older');
    expect(recall.move(updated, 'older', 1)).toBe('newer');
    expect(recall.move(updated, 'newer', 1)).toBe('');
    expect(recall.move(updated, '', -1)).toBe('latest');
  });

  it('keeps each mounted composer cursor independent and resets after sending', () => {
    const a = new PromptRecall();
    const b = new PromptRecall();
    expect(a.move(history, '', -1)).toBe('newer');
    expect(b.move(['other session'], '', -1)).toBe('other session');
    expect(a.move(history, 'newer', -1)).toBe('older');
    a.reset();
    expect(a.move([...history, 'sent'], '', -1)).toBe('sent');
  });
});
