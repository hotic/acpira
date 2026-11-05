import { afterEach, describe, expect, it } from 'vitest';
import type { ContextReport } from '@acpira/shared/context';
import { setLocale } from '../i18n';
import { buildTree, ROOT_ID } from '../context/tree';
import { fitLabel, lay, squarify } from '../context/treemap';

afterEach(() => setLocale('en'));

const REPORT: ContextReport = {
  used: 1000, size: 200_000, rev: 'r', categories: [
    { id: 'system', tokens: 100, nodes: [{ id: 'sys:base', kind: 'base', tokens: 100 }] },
    { id: 'docs', tokens: 50, nodes: [{ id: 'doc:0', kind: 'doc', name: '/home/me/work/AGENTS.md', tokens: 50 }] },
    { id: 'chat', tokens: 850, nodes: [
      { id: 't:1', kind: 'turn', index: 1, title: 'fix the build', tokens: 850, children: [
        { id: 't:1:msg', kind: 'message', tokens: 10 },
        { id: 't:1:c:0', kind: 'call', toolKind: 'read', name: 'read', title: 'src/main.rs', tokens: 600, cached: 600 },
        { id: 't:1:c:1', kind: 'call', toolKind: 'read', name: 'read', title: 'a.rs +2', tokens: 200 },
        { id: 't:1:c:2', kind: 'call', toolKind: 'other', name: 'goal_update', tokens: 40 },
      ] },
    ] },
  ],
};

describe('context report tree', () => {
  it('nests categories over the agent nodes, labels them and resolves the files a leaf can open', () => {
    const { root, index, pathOf } = buildTree(REPORT, { home: '/home/me', cwd: '/home/me/work' });
    expect(root.children!.map(c => c.id)).toEqual(['system', 'docs', 'chat']);
    expect(root.tokens).toBe(1000);
    expect(index.get('t:1')!.label).toBe('Turn 1');
    expect(index.get('t:1')!.target).toBe('fix the build');
    expect(index.get('t:1')!.cached).toBe(600);
    // An instruction file shows relative to the project and opens by its absolute path
    expect(index.get('doc:0')).toMatchObject({ label: 'AGENTS.md', file: '/home/me/work/AGENTS.md' });
    // A read opens its file, resolved against the project; one naming several files opens none
    expect(index.get('t:1:c:0')).toMatchObject({ label: 'Read', target: 'src/main.rs', file: '/home/me/work/src/main.rs' });
    expect(index.get('t:1:c:1')!.file).toBeUndefined();
    // A tool without a verb of its own goes by its name
    expect(index.get('t:1:c:2')!.label).toBe('goal_update');
    expect(pathOf('t:1:c:0')).toEqual([ROOT_ID, 'chat', 't:1', 't:1:c:0']);
  });

  it('names a turn by its prompt when compaction left it unnumbered', () => {
    const r: ContextReport = { ...REPORT, categories: [{ id: 'chat', tokens: 5, nodes: [
      { id: 't:a', kind: 'turn', title: 'go on', tokens: 5, children: [{ id: 't:a:msg', kind: 'message', tokens: 5 }] },
    ] }] };
    const turn = buildTree(r).index.get('t:a')!;
    expect([turn.label, turn.target]).toEqual(['go on', undefined]);
  });
});

describe('context report map', () => {
  it('fills the box with areas in proportion to the values', () => {
    const rect = { x: 0, y: 0, w: 400, h: 300 };
    const boxes = squarify([30, 20, 10, 5, 0].map((value, i) => ({ value, data: i })), rect);
    expect(boxes.map(b => b.data).sort()).toEqual([0, 1, 2, 3]);
    const area = boxes.reduce((s, b) => s + b.w * b.h, 0);
    expect(area).toBeCloseTo(rect.w * rect.h);
    for (const b of boxes) {
      expect(b.w * b.h).toBeCloseTo((rect.w * rect.h * [30, 20, 10, 5][b.data]!) / 65);
      expect(b.x + b.w).toBeLessThanOrEqual(rect.w + 1e-6);
      expect(b.y + b.h).toBeLessThanOrEqual(rect.h + 1e-6);
    }
  });

  it('folds leaves too thin to read into one stub', () => {
    const r: ContextReport = { ...REPORT, categories: [{ id: 'tools', tokens: 0, nodes: [
      { id: 'big', kind: 'tool', name: 'bash', tokens: 10_000 },
      ...Array.from({ length: 6 }, (_, i) => ({ id: `t${i}`, kind: 'tool' as const, name: `t${i}`, tokens: 5 })),
    ] }] };
    const items = lay(buildTree(r).root, { x: 0, y: 0, w: 400, h: 300 });
    const stub = items.find(i => i.n.more);
    expect(stub?.n.more).toHaveLength(6);
    expect(items.some(i => i.n.id === 't0')).toBe(false);
  });

  it('labels a tile with the most it can fit', () => {
    const { index } = buildTree(REPORT);
    const call = index.get('t:1:c:0')!;
    const m = (s: string) => s.length * 7;
    expect(fitLabel(call, 500, m)).toEqual({ a: 'Read', b: 'src/main.rs' });
    expect(fitLabel(call, 60, m)).toEqual({ a: 'main.rs' });
    expect(fitLabel(call, 10, m)).toBeUndefined();
  });
});
