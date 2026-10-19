import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { ContextSlots, MAX_CONTEXT_ITEMS, parseContextPayload } from '../src/host/contextApi';
import { MAX_TEXT_BYTES } from '../src/shared/attachments';

const source = { id: 'asgard.questCanvas', name: 'Quest canvas' };
const item = { label: 'a.canvas 3 nodes', text: 'node a\nnode b', uri: 'file:///w/a.canvas' };

describe('context API payloads', () => {
  it('accepts the documented shape and tidies labels and comments', () => {
    expect(parseContextPayload({ version: 1, source, owner: 'file:///w/a.canvas', items: [{ ...item, label: ' a.canvas\n3 nodes ', comment: '  check this ' }] }))
      .toEqual({ source, owner: 'file:///w/a.canvas', items: [{ ...item, comment: 'check this' }] });
    // Missing items mean a clear
    expect(parseContextPayload({ source }).items).toEqual([]);
  });

  it('names what is wrong', () => {
    expect(() => parseContextPayload(undefined)).toThrow(/object/);
    expect(() => parseContextPayload({ version: 2, source, items: [] })).toThrow(/version 2/);
    expect(() => parseContextPayload({ source: { id: 'bad id', name: 'x' }, items: [] })).toThrow(/source\.id/);
    expect(() => parseContextPayload({ source: { id: 'ok', name: '  ' }, items: [] })).toThrow(/source\.name/);
    expect(() => parseContextPayload({ source, items: [{ label: 'x', text: ' ' }] })).toThrow(/items\[0\]\.text/);
    expect(() => parseContextPayload({ source, items: [{ label: '', text: 'x' }] })).toThrow(/items\[0\]\.label/);
    expect(() => parseContextPayload({ source, items: Array.from({ length: MAX_CONTEXT_ITEMS + 1 }, () => item) })).toThrow(/at most/);
    const half = 'x'.repeat(MAX_TEXT_BYTES / 2 + 1);
    expect(() => parseContextPayload({ source, items: [{ label: 'a', text: half }, { label: 'b', text: half }] })).toThrow(/KB in total/);
  });
});

describe('live context slots', () => {
  const set = (slots: ContextSlots, owner: string, items = [item]) => slots.set(parseContextPayload({ source, owner, items }));

  it('lets any owner take the slot but only the filling owner clear it', () => {
    const slots = new ContextSlots();
    set(slots, 'A');
    expect(set(slots, 'B', [{ ...item, label: 'b' }])).toMatchObject({ type: 'contextSelection', items: [{ label: 'b' }] });
    // A was closed after B took over: B's selection stays
    expect(set(slots, 'A', [])).toBeUndefined();
    expect(slots.replay()).toMatchObject([{ items: [{ label: 'b' }] }]);
    expect(set(slots, 'B', [])).toEqual({ type: 'contextSelection', source });
    expect(slots.replay()).toEqual([]);
  });

  it('drops slots whose owner went away, and hides without forgetting', () => {
    const slots = new ContextSlots();
    set(slots, 'A');
    slots.set(parseContextPayload({ source: { id: 'other', name: 'Other' }, items: [item] }));
    expect(slots.hidden()).toHaveLength(2);
    expect(slots.replay()).toHaveLength(2);
    expect(slots.owners()).toEqual(new Set(['A']));
    expect(slots.release(new Set(['A']))).toEqual([{ type: 'contextSelection', source }]);
    expect(slots.replay().map(m => m.type === 'contextSelection' && m.source.id)).toEqual(['other']);
  });
});

describe('the extension manifest', () => {
  // A command only activates the extension from another extension's executeCommand when it is contributed
  it('contributes every command activate() registers, and hides the context API from the palette', () => {
    const pkg = JSON.parse(readFileSync('package.json', 'utf8')) as {
      contributes: { commands: { command: string }[]; menus: { commandPalette: { command: string; when: string }[] } };
    };
    const contributed = new Set(pkg.contributes.commands.map(c => c.command));
    const registered = [...readFileSync('src/host/extension.ts', 'utf8').matchAll(/register(?:TextEditor)?Command\('([^']+)'/g)].map(m => m[1]!);
    expect(registered.filter(c => !contributed.has(c))).toEqual([]);
    for (const c of ['acpira.context.set', 'acpira.context.add']) {
      expect(pkg.contributes.menus.commandPalette).toContainEqual({ command: c, when: 'false' });
    }
  });
});
