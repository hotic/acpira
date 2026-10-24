import { describe, expect, it } from 'vitest';
import { applyGlyphRuns, createGlyphRuns, seedGlyphRuns, GLYPH_CLASS } from '../chat/glyphRuns';

type Node = { type: string; value?: string; tagName?: string; properties?: Record<string, unknown>; children?: Node[] };
const text = (value: string): Node => ({ type: 'text', value });
const el = (tagName: string, children: Node[]): Node => ({ type: 'element', tagName, properties: {}, children });
const tree = (...children: Node[]) => ({ type: 'root' as const, children: children as never[] });
// The paragraph's children as [kind, text]: 'f' a fading span, 't' a plain text node
const shape = (root: { children: Node[] }) => (root.children[0]!.children ?? []).map(n => n.type === 'text' ? ['t', n.value] : ((n.properties?.className as string[] | undefined) ?? []).includes(GLYPH_CLASS) ? ['f', n.children?.[0]?.value] : [n.tagName, undefined]);

describe('glyph runs', () => {
  it('wraps each pass in one span and turns finished runs into text at the same index', () => {
    const s = createGlyphRuns();
    const a = tree(el('p', [text('你好')]));
    applyGlyphRuns(s, a, 0, 200);
    expect(shape(a)).toEqual([['f', '你好']]);
    const b = tree(el('p', [text('你好世界')]));
    applyGlyphRuns(s, b, 100, 200);
    expect(shape(b)).toEqual([['f', '你好'], ['f', '世界']]);
    const c = tree(el('p', [text('你好世界！')]));
    applyGlyphRuns(s, c, 250, 200);
    // The first run finished: plain text, still child 0, so the fading spans keep their indexes
    expect(shape(c)).toEqual([['t', '你好'], ['f', '世界'], ['f', '！']]);
  });

  it('splits across inline elements by character offset and leaves code alone', () => {
    const s = createGlyphRuns();
    applyGlyphRuns(s, tree(el('p', [text('ab')])), 0, 200);
    const b = tree(el('p', [text('ab'), el('strong', [text('cd')])]), el('pre', [el('code', [text('x = 1')])]));
    applyGlyphRuns(s, b, 300, 200);
    expect(shape(b)).toEqual([['t', 'ab'], ['strong', undefined]]);
    const [p, pre] = b.children as Node[];
    expect(p!.children![1]!.children!.map(n => n.type)).toEqual(['element']);
    expect(pre!.children![0]!.children).toEqual([text('x = 1')]);
  });

  it('shows a resumed block at once and fades only what follows', () => {
    const s = createGlyphRuns();
    seedGlyphRuns(s);
    const a = tree(el('p', [text('already here')]));
    applyGlyphRuns(s, a, 0, 200);
    expect(shape(a)).toEqual([['t', 'already here']]);
    const b = tree(el('p', [text('already here, then')]));
    applyGlyphRuns(s, b, 10, 200);
    expect(shape(b)).toEqual([['t', 'already here'], ['f', ', then']]);
  });

  it('keeps its runs when the rendered text shrinks for a pass', () => {
    const s = createGlyphRuns();
    applyGlyphRuns(s, tree(el('p', [text('abc**')])), 0, 200);
    const b = tree(el('p', [text('abc')]));
    applyGlyphRuns(s, b, 50, 200);
    expect(shape(b)).toEqual([['f', 'abc']]);
    expect(s.runs).toHaveLength(1);
  });
});
