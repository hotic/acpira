import { describe, expect, it } from 'vitest';
import { diffLineNumbers } from '../src/webview/chat/diffLineNumbers';
import { diffCase } from './fixtures/engine';

describe('diff source positions', () => {
  it('recomputes both source positions of a stored diff across omitted context and inserted lines', () => {
    const { lines } = diffCase('splice30');
    expect(lines[0]?.kind).toBe('hunk');
    // Stored transcripts without positions must retain the same visible numbers
    expect(diffLineNumbers(lines.map(({ kind, text }) => ({ kind, text })))).toEqual(lines);
  });

  it('numbers new files and the bounded replacement output', () => {
    for (const name of ['new-ab', 'bounded450']) {
      const { lines } = diffCase(name);
      expect(diffLineNumbers(lines.map(({ kind, text }) => ({ kind, text })))).toEqual(lines);
    }
  });

  it.each(['@@ … 90 unchanged lines … @@', '@@ … 90 行未变 … @@'])('restores localized historical omissions: %s', text => {
    expect(diffLineNumbers([{ kind: 'hunk', text }, { kind: 'del', text: '-old' }, { kind: 'add', text: '+new' }]).slice(1))
      .toMatchObject([{ oldLine: 91 }, { newLine: 91 }]);
  });

  it('keeps unknown positions blank until the source supplies an explicit position', () => {
    const rows = diffLineNumbers([
      { kind: 'hunk', text: '@@ unknown omission @@' },
      { kind: 'add', text: '+unknown' },
      { kind: 'add', text: '+known', newLine: 80 },
      { kind: 'add', text: '+next' },
    ]);
    expect(rows.map(l => l.newLine)).toEqual([undefined, undefined, 80, 81]);
  });
});
