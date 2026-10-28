import { describe, expect, it } from 'vitest';
import { codeLanguage, tokenizeCode } from '../packages/ui/chat/codeSyntax';
import { diffCopyText } from '../packages/ui/chat/codeDiff';
import { highlightDiff } from '../packages/ui/chat/codeDiffHighlight';
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
    const rows = await highlightDiff(lines, 'dark', { path: 'answer.ts', omitted: true });
    expect(rows.some(row => row.tokens.some(token => token.dark))).toBe(true);
  });

  it('preserves Python multiline grammar across omitted lines and both source sides', async () => {
    const { oldText, newText, lines } = diffCase('py-multiline');
    const rows = await highlightDiff(lines, 'dark', { path: 'example.py', oldText, newText });
    const added = rows.find(row => row.kind === 'add')!;
    const deleted = rows.find(row => row.kind === 'del')!;
    expect(added.tokens.map(token => token.text).join('')).toBe('new text');
    expect(added.tokens[0]?.dark).toBe('#CE9178');
    expect(deleted.tokens[0]?.dark).toBe('#CE9178');
  });

  it.each([['file.tsx', 'tsx'], ['file.py', 'py'], ['file.json', 'json']])('uses real syntax tokens for %s in either theme', async (path, name) => {
    const { oldText, newText, lines } = diffCase(name);
    const dark = (await highlightDiff(lines, 'dark', { path, oldText, newText })).flatMap(row => row.tokens);
    const light = (await highlightDiff(lines, 'light', { path, oldText, newText })).flatMap(row => row.tokens);
    expect(new Set(dark.map(token => token.dark)).size).toBeGreaterThan(2);
    // Each pass colours its own scheme only, and the schemes differ
    expect(dark.some(token => token.light) || light.some(token => token.dark)).toBe(false);
    expect(new Set(light.map(token => token.light))).not.toEqual(new Set(dark.map(token => token.dark)));
  });

  it('keeps unknown filenames plain and recovers visible historical syntax', async () => {
    expect(codeLanguage('NOTES')).toBeNull();
    const { lines } = diffCase('answer');
    const plain = await highlightDiff(lines, 'dark', undefined, 'NOTES');
    expect(plain[0]?.tokens).toEqual([{ text: 'const answer = "yes";' }]);
    const legacy = await highlightDiff(lines.map(({ kind, text }) => ({ kind, text })), 'dark', undefined, 'answer.ts');
    expect(legacy[0]?.tokens.some(token => token.dark)).toBe(true);
  });

  it('carries grammar state across the slices of a long source', async () => {
    // A block comment opens before the first slice ends (512 characters) and closes long after it; a template literal follows
    const filler = Array.from({ length: 2 }, (_, i) => `const value${i} = ${i} * 2;`);
    const text = [...filler, '/* a comment', ...Array.from({ length: 40 }, (_, i) => ` * line ${i} const x = 1;`), ' */',
      'const s = `start', ...Array.from({ length: 40 }, (_, i) => `${i} \${value${i % 2}} text`), 'end`;', 'export const done = true;'].join('\n');
    let pauses = 0;
    const sliced = await tokenizeCode(text, 'typescript', 'dark', async () => { pauses++; });
    // A pause follows every slice: two or more mean the source was cut
    expect(pauses).toBeGreaterThan(1);
    expect(sliced).toEqual(await tokenizeCode(text, 'typescript', 'dark'));
  });

  it('keeps the slice budget past a skipped long line or a run of blank lines', async () => {
    // Neither costs the grammar anything, so neither may grow the slice after it: counted, a 50 KB first line sized the
    // next slice to all the dense code behind it
    const code = Array.from({ length: 200 }, (_, i) => `export const value${i} = compute(${i}, "text ${i}");`);
    for (const head of ['x'.repeat(50_000), '\n'.repeat(5_000)]) {
      const lines = [head, ...code].join('\n').split('\n');
      const ends: number[] = [];
      await tokenizeCode(lines.join('\n'), 'typescript', 'dark', async () => {}, done => { ends.push(done.length); });
      // The first slice that holds code is no larger than the first slice's budget, and one line past it
      const at = ends.findIndex((end, i) => lines.slice(ends[i - 1] ?? 0, end).some(line => /\S/.test(line) && line.length < 20_000));
      expect(at).toBeGreaterThanOrEqual(0);
      expect(lines.slice(ends[at - 1] ?? 0, ends[at]).join('\n').length).toBeLessThanOrEqual(512 + 80);
    }
  });

  it('retains empty-file copy', () => {
    const { lines } = diffCase('to-empty');
    expect(diffCopyText(lines, { path: 'empty', oldText: 'old', newText: '' })).toBe('');
  });
});
