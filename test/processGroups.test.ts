import { describe, expect, it } from 'vitest';
import type { AgentBlock, DiffLine, ToolCallBlock } from '../src/shared/transcript';
import { DWELL_MS, editEntries, editReference, editSpan, foldStates, groupNames, groupProcess, itemSettled, type FoldInput } from '../src/webview/chat/processGroups';

const read = (id: string, path: string, status: ToolCallBlock['status'] = 'completed'): ToolCallBlock => ({
  type: 'tool_call', id, kind: 'read', verb: 'Read', status, target: path.split('/').pop(), locations: [{ path }],
});
const diff = (lines: DiffLine[]) => ({ type: 'diff' as const, lines });
const edit = (id: string, path: string, lines: DiffLine[] = []): ToolCallBlock => ({
  type: 'tool_call', id, kind: 'edit', verb: 'Edit', status: 'completed', target: path.split('/').pop(), locations: [{ path }],
  content: diff(lines), diffStat: { add: lines.filter(l => l.kind === 'add').length, del: lines.filter(l => l.kind === 'del').length },
});
const search: ToolCallBlock = { type: 'tool_call', id: 's', kind: 'search', verb: 'Search', status: 'completed', target: 'x' };
const thought: AgentBlock = { type: 'thought', text: 'hm', startedAt: 5 };

describe('process groups', () => {
  it('opens edited files without a preceding read, at the first changed line', () => {
    const block = edit('e', '/repo/my file.ts', [{ kind: 'add', newLine: 42, text: 'added' }]);
    expect(editReference([block])).toEqual({ path: '/repo/my file.ts', line: 42 });
    expect(editReference([block, edit('e2', '/repo/my file.ts', [{ kind: 'del', oldLine: 12, text: 'removed' }])]))
      .toEqual({ path: '/repo/my file.ts', line: 12 });
    expect(editReference([{ ...block, locations: undefined, target: 'edit file', content: {
      type: 'diff', lines: [], source: { path: '/repo/new.ts', oldText: '', newText: 'new' },
    } }])).toEqual({ path: '/repo/new.ts' });
  });

  it('uses stored locations or legacy paths and rejects ambiguous edit destinations', () => {
    const block = edit('e', '/repo/a.ts');
    expect(editReference([{ ...block, locations: [{ path: '/repo/a.ts', line: 9 }] }])).toEqual({ path: '/repo/a.ts', line: 9 });
    expect(editReference([{ ...block, locations: undefined, target: 'src/a.ts:17' }])).toEqual({ path: 'src/a.ts', line: 17 });
    expect(editReference([{ ...block, locations: undefined, target: 'Edited 2 files' }])).toBeUndefined();
    expect(editReference([{ ...block, locations: [{ path: '/repo/a.ts' }, { path: '/repo/b.ts' }] }])).toBeUndefined();
  });

  it('groups consecutive reads and consecutive edits, keyed by their first call', () => {
    const items = groupProcess([read('r1', '/a.ts'), read('r2', '/b.ts'), search, read('r3', '/c.ts'), thought, edit('e1', '/a.ts'), edit('e2', '/b.ts')]);
    expect(items.map(i => [i.type, i.id, i.type === 'group' ? i.kind : undefined])).toEqual([
      ['group', 'r1', 'read'], ['block', 's', undefined], ['block', 'r3', undefined], ['block', 'thought:5', undefined], ['group', 'e1', 'edit'],
    ]);
  });

  it('lets running calls join and keeps failures and empty reads on their own', () => {
    const empty: ToolCallBlock = { type: 'tool_call', id: 'empty', kind: 'read', verb: 'Read', status: 'completed' };
    const failed = { ...read('f', '/f.ts'), status: 'failed' as const };
    const items = groupProcess([read('r1', '/a.ts'), read('r2', '/b.ts', 'in_progress'), failed, read('r3', '/c.ts'), empty, read('r4', '/d.ts')]);
    expect(items.map(i => i.id)).toEqual(['r1', 'f', 'r3', 'empty', 'r4']);
    expect(items[0]).toMatchObject({ type: 'group', blocks: [{ id: 'r1' }, { id: 'r2' }] });
  });

  it('merges edits of one file into one entry and names each file once', () => {
    const blocks = [edit('e1', '/src/Turns.tsx'), edit('e2', '/src/prompt.rs'), edit('e3', '/src/Turns.tsx')];
    expect(editEntries(blocks).map(e => [e.key, e.blocks.map(b => b.id)])).toEqual([['/src/Turns.tsx', ['e1', 'e3']], ['/src/prompt.rs', ['e2']]]);
    expect(groupNames(blocks)).toEqual(['Turns.tsx', 'prompt.rs']);
  });

  it('spans the first to the last changed line, falling back to removed lines for a deletion', () => {
    const a = edit('a', '/t.ts', [{ kind: 'ctx', text: ' x', oldLine: 534, newLine: 534 }, { kind: 'add', text: '+y', newLine: 535 }, { kind: 'add', text: '+z', newLine: 536 }]);
    const b = edit('b', '/t.ts', [{ kind: 'del', text: '-q', oldLine: 549 }, { kind: 'add', text: '+r', newLine: 549 }]);
    expect(editSpan([a, b])).toBe('L535–549');
    expect(editSpan([edit('c', '/t.ts', [{ kind: 'del', text: '-gone', oldLine: 12 }])])).toBe('L12');
    expect(editSpan([edit('d', '/t.ts')])).toBeUndefined();
  });

  describe('folds', () => {
    const run = (blocks: AgentBlock[], over: Partial<FoldInput> = {}) => foldStates({
      items: groupProcess(blocks), now: 1000, live: true, autoExpand: true, following: true,
      doneAt: new Map(), first: false, held: new Set(), manual: () => undefined, ...over,
    });
    const openIds = (r: ReturnType<typeof run>) => [...r.folds].filter(([, open]) => open).map(([id]) => id);
    const reads = [read('r1', '/a.ts'), read('r2', '/b.ts')];

    it('opens the latest item and keeps a just-finished one open for the dwell', () => {
      const doneAt = new Map<string, number>();
      const first = run([...reads, thought], { doneAt, now: 1000 });
      expect(openIds(first)).toEqual(['r1', 'thought:5']);
      expect(first.wake).toBe(1000 + DWELL_MS);
      expect(openIds(run([...reads, thought], { doneAt, now: 1000 + DWELL_MS }))).toEqual(['thought:5']);
    });

    it('dates items finished before the turn mounted long ago, so nothing flashes open', () => {
      expect(openIds(run([...reads, thought], { first: true }))).toEqual(['thought:5']);
    });

    it('keeps a still-running call open and never opens a lone diff', () => {
      const cmd: ToolCallBlock = { type: 'tool_call', id: 'x', kind: 'execute', verb: 'Run', status: 'in_progress' };
      expect(openIds(run([cmd, thought, edit('e', '/a.ts')], { first: true }))).toEqual(['x']);
      // An edit group lists its files like any group
      expect(openIds(run([edit('e1', '/a.ts'), edit('e2', '/b.ts')]))).toEqual(['e1']);
    });

    it('closes everything once the turn is done or details are off, unless chosen by hand', () => {
      expect(openIds(run(reads, { live: false }))).toEqual([]);
      expect(openIds(run(reads, { autoExpand: false }))).toEqual([]);
      expect(openIds(run(reads, { live: false, manual: id => id === 'r1' || undefined }))).toEqual(['r1']);
      expect(openIds(run([...reads, thought], { first: true, manual: id => (id === 'thought:5' ? false : undefined) }))).toEqual([]);
    });

    it('holds what was open while the reader is away from the bottom', () => {
      expect(openIds(run([...reads, thought], { first: true, following: false, held: new Set(['r1']) }))).toEqual(['r1', 'thought:5']);
      expect(openIds(run(reads, { live: false, following: false, held: new Set(['r1']) }))).toEqual(['r1']);
    });
  });

  it('treats a background command and a finished thought as settled', () => {
    const cmd: ToolCallBlock = { type: 'tool_call', id: 'x', kind: 'execute', verb: 'Run', status: 'in_progress' };
    expect(itemSettled({ type: 'block', id: 'x', block: cmd })).toBe(false);
    expect(itemSettled({ type: 'block', id: 'x', block: { ...cmd, background: true } })).toBe(true);
    expect(itemSettled({ type: 'block', id: 't', block: { ...thought, streaming: true } as AgentBlock })).toBe(false);
    expect(itemSettled({ type: 'block', id: 't', block: thought })).toBe(true);
  });
});
