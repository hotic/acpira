import { describe, expect, it } from 'vitest';
import { codeLanguage } from '../src/webview/chat/codeSyntax';
import { diffCopyText, highlightDiff } from '../src/webview/chat/codeDiff';
import { agentTurn, diffCase } from './fixtures/engine';

describe('production code details', () => {
  it('keeps exact source for copying while omitting unchanged display context', () => {
    const block = agentTurn('edit-values').blocks[0];
    const diff = block?.type === 'tool_call' ? block.content : undefined;
    if (diff?.type !== 'diff') throw new Error('Missing diff');
    expect(diff.lines.some(line => line.kind === 'hunk')).toBe(true);
    const source = diff.source;
    if (!source || source.omitted) throw new Error('Missing source');
    expect(diffCopyText(diff.lines, source)).toBe(source.newText);
    expect(diffCopyText(diff.lines)).not.toContain('value0 =');
    // a page view's source comes without its texts: the visible lines stand in until the host is asked
    expect(diffCopyText(diff.lines, { path: source.path, omitted: true })).toBe(diffCopyText(diff.lines));
  });

  it('highlights a page view whose source omits the file texts from the visible segments', async () => {
    const { lines } = diffCase('answer');
    const rows = await highlightDiff(lines, { path: 'answer.ts', omitted: true });
    expect(rows.some(row => row.tokens.some(token => token.dark))).toBe(true);
  });

  it('preserves Python multiline grammar across omitted lines and both source sides', async () => {
    const { oldText, newText, lines } = diffCase('py-multiline');
    const rows = await highlightDiff(lines, { path: 'example.py', oldText, newText });
    const added = rows.find(row => row.kind === 'add')!;
    const deleted = rows.find(row => row.kind === 'del')!;
    expect(added.tokens.map(token => token.text).join('')).toBe('new text');
    expect(added.tokens[0]?.dark).toBe('#CE9178');
    expect(deleted.tokens[0]?.dark).toBe('#CE9178');
  });

  it.each([['file.tsx', 'tsx'], ['file.py', 'py'], ['file.json', 'json']])('uses real syntax tokens for %s in both themes', async (path, name) => {
    const { oldText, newText, lines } = diffCase(name);
    const rows = await highlightDiff(lines, { path, oldText, newText });
    expect(new Set(rows.flatMap(row => row.tokens.map(token => token.dark))).size).toBeGreaterThan(2);
    expect(rows.flatMap(row => row.tokens).some(token => token.dark !== token.light)).toBe(true);
  });

  it('keeps unknown filenames plain and recovers visible historical syntax', async () => {
    expect(codeLanguage('NOTES')).toBeNull();
    const { lines } = diffCase('answer');
    const plain = await highlightDiff(lines, undefined, 'NOTES');
    expect(plain[0]?.tokens).toEqual([{ text: 'const answer = "yes";' }]);
    const legacy = await highlightDiff(lines.map(({ kind, text }) => ({ kind, text })), undefined, 'answer.ts');
    expect(legacy[0]?.tokens.some(token => token.dark)).toBe(true);
  });

  it('retains empty-file copy', () => {
    const { lines } = diffCase('to-empty');
    expect(diffCopyText(lines, { path: 'empty', oldText: 'old', newText: '' })).toBe('');
  });
});
