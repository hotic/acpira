import { describe, expect, it } from 'vitest';
import type { SessionView } from '../transcript';
import { applySession, reuse, reuseById } from '../reuse';

const clone = <T>(value: T): T => structuredClone(value);

function view(): SessionView {
  return {
    id: 's', agent: 'grok', title: 't', cwd: '/w', status: 'ready', running: true,
    createdAt: 'a', updatedAt: 'b', commands: [], controls: { modes: [], options: [] },
    turns: [
      { role: 'user', id: 'u1', text: 'hi' },
      { role: 'agent', stop: 'end_turn', blocks: [
        { type: 'thought', text: 'think', streaming: false },
        { type: 'tool_call', id: 'c1', kind: 'read', verb: 'Read', target: 'a.ts', status: 'completed', locations: [{ path: 'a.ts' }] },
        { type: 'text', markdown: 'done' },
      ] },
      { role: 'user', id: 'u2', text: 'more' },
      { role: 'agent', blocks: [{ type: 'text', markdown: 'strea', streaming: true }] },
    ],
  };
}

describe('reuse', () => {
  it('returns the previous reference for a deep-equal clone', () => {
    const previous = view();
    expect(reuse(previous, clone(previous))).toBe(previous);
  });

  it('keeps unchanged turns and blocks while the streaming turn takes the new values', () => {
    const previous = view();
    const next = clone(previous);
    const live = next.turns[3] as Extract<SessionView['turns'][number], { role: 'agent' }>;
    (live.blocks[0] as { markdown: string }).markdown = 'streaming';
    const merged = reuse(previous, next);
    expect(merged).not.toBe(previous);
    expect(merged.turns).not.toBe(previous.turns);
    expect(merged.turns[0]).toBe(previous.turns[0]);
    expect(merged.turns[1]).toBe(previous.turns[1]);
    expect(merged.turns[2]).toBe(previous.turns[2]);
    expect(merged.turns[3]).not.toBe(previous.turns[3]);
    expect((merged.turns[3] as typeof live).blocks[0]).toEqual({ type: 'text', markdown: 'streaming', streaming: true });
    expect(merged.controls).toBe(previous.controls);
  });

  it('keeps earlier blocks of a turn that grew', () => {
    const previous = view();
    const next = clone(previous);
    const live = next.turns[3] as Extract<SessionView['turns'][number], { role: 'agent' }>;
    live.blocks.push({ type: 'tool_call', id: 'c2', kind: 'execute', verb: 'Run', target: 'ls', status: 'in_progress' });
    const merged = reuse(previous, next) as typeof next;
    const before = previous.turns[3] as typeof live;
    expect((merged.turns[3] as typeof live).blocks[0]).toBe(before.blocks[0]);
    expect((merged.turns[3] as typeof live).blocks).toHaveLength(2);
  });

  it('does not equate a removed key with an undefined one', () => {
    expect(reuse({ a: 1, b: undefined } as Record<string, unknown>, { a: 1 })).toEqual({ a: 1 });
    expect(Object.keys(reuse({ a: 1, b: undefined } as Record<string, unknown>, { a: 1 }))).toEqual(['a']);
    const previous = { a: 1 } as Record<string, unknown>;
    expect(reuse(previous, { a: 1, b: undefined })).not.toBe(previous);
  });

  it('replaces arrays that shrank or changed type', () => {
    const previous = { list: [1, 2, 3], value: { x: 1 } as unknown };
    expect(reuse(previous, { list: [1, 2], value: { x: 1 } }).list).toEqual([1, 2]);
    expect(reuse(previous, { list: [1, 2, 3], value: 'text' }).value).toBe('text');
    expect(reuse(previous, { list: [1, 2, 3], value: null }).value).toBeNull();
  });
});

describe('reuseById', () => {
  const entry = (id: string, title = id) => ({ id, title, updatedAt: '2026-10-25' });

  // The session list's memoized rows rely on this: a session added or removed at the top shifts every index
  it('keeps unchanged entries by id when the list shifts', () => {
    const previous = [entry('a'), entry('b'), entry('c')];
    const added = reuseById(previous, clone([entry('n'), ...previous]));
    expect(added.slice(1)).toEqual(previous);
    added.slice(1).forEach((item, i) => expect(item).toBe(previous[i]));
    const removed = reuseById(previous, clone(previous.slice(1)));
    expect(removed[0]).toBe(previous[1]);
    expect(removed[1]).toBe(previous[2]);
  });

  it('takes the new object for a changed entry and the previous array when nothing changed', () => {
    const previous = [entry('a'), entry('b')];
    const renamed = reuseById(previous, [entry('a', 'renamed'), entry('b')]);
    expect(renamed[0]).toEqual(entry('a', 'renamed'));
    expect(renamed[1]).toBe(previous[1]);
    expect(reuseById(previous, clone(previous))).toBe(previous);
    // Same entries in another order: a new array of the old objects
    const swapped = reuseById(previous, clone([previous[1]!, previous[0]!]));
    expect(swapped).not.toBe(previous);
    expect(swapped[0]).toBe(previous[1]);
  });
});

describe('applySession', () => {
  it('drops an older running snapshot that arrives after settle', () => {
    const idle = { ...view(), running: false, rev: 4 };
    const stale = { ...view(), running: true, rev: 3 };
    expect(applySession(idle, stale)).toBe(idle);
  });

  it('applies a newer snapshot and a first paint without a current view', () => {
    const idle = { ...view(), running: false, rev: 4 };
    const next = { ...view(), running: true, rev: 5 };
    expect(applySession(idle, next).running).toBe(true);
    expect(applySession(undefined, idle)).toBe(idle);
  });

  it('applies unversioned snapshots so the LAB and an older host still update', () => {
    const current = { ...view(), running: false, rev: 4 };
    const unversioned = { ...view(), running: true };
    expect(applySession(current, unversioned).running).toBe(true);
    expect(applySession({ ...view(), running: false }, { ...view(), running: true, rev: 1 }).running).toBe(true);
  });

  it('replaces a snapshot from another session even when rev is lower', () => {
    const current = { ...view(), id: 'a', running: false, rev: 8 };
    const other = { ...view(), id: 'b', running: true, rev: 1 };
    expect(applySession(current, other)).toBe(other);
  });
});
