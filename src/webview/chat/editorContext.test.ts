import { describe, expect, it } from 'vitest';
import { contextDrafts, copiedSelection, markContextSent, markSelectionSent, offeredLiveContexts, offeredSelection, sameContext, sameRange, selectionDraft, setContextSelection, setEditorCopy, setEditorSelection } from './editorContext';

const copy = { uri: 'file:///w/src/a.ts', startLine: 12, endLine: 14, text: 'one\r\ntwo\r\nthree\r\n' };

describe('editor copies pasted into the composer', () => {
  it('matches the text copied from an editor across line endings and a trailing newline', () => {
    setEditorCopy(copy);
    expect(copiedSelection('one\ntwo\nthree')).toBe(copy);
    expect(copiedSelection('one\ntwo')).toBeUndefined();
  });

  it('leaves a one-line copy as plain text', () => {
    setEditorCopy({ ...copy, endLine: 12, text: 'identifier' });
    expect(copiedSelection('identifier')).toBeUndefined();
  });

  it('labels the range relative to the workspace and spots duplicates', () => {
    const d = selectionDraft(copy, '/w');
    expect(d).toEqual({ kind: 'selection', uri: copy.uri, name: 'src/a.ts', startLine: 12, endLine: 14, text: copy.text });
    expect(sameRange(d, { ...d })).toBe(true);
    // Another excerpt of the same lines is a second draft
    expect(sameRange(d, { ...d, text: 'changed' })).toBe(false);
    expect(sameRange(d, { ...d, endLine: 15 })).toBe(false);
  });

  it('names a Windows selection relative to a backslash workspace', () => {
    const win = { ...copy, uri: 'file:///c%3A/Repo/src/a.ts' };
    expect(selectionDraft(win, 'C:\\repo').name).toBe('src/a.ts');
    expect(selectionDraft(win, 'D:\\other').name).toBe('a.ts');
  });
});

describe('the live selection', () => {
  it('stays hidden once sent, and is offered again after it was cleared and picked again', () => {
    setEditorSelection(copy);
    expect(offeredSelection()).toBe(copy);
    markSelectionSent(copy);
    expect(offeredSelection()).toBeUndefined();
    setEditorSelection(copy);
    expect(offeredSelection()).toBeUndefined();
    setEditorSelection(undefined);
    setEditorSelection(copy);
    expect(offeredSelection()).toBe(copy);
  });

  it('treats a same-length selection that only differs past its first characters as a new one', () => {
    const head = 'x'.repeat(64);
    const a = { ...copy, text: `${head}A` };
    const b = { ...copy, text: `${head}B` };
    setEditorSelection(undefined);
    setEditorSelection(a);
    markSelectionSent(a);
    setEditorSelection(b);
    expect(offeredSelection()).toBe(b);
  });
});

describe('other tools\' live selections', () => {
  const canvas = { id: 'asgard.questCanvas', name: 'Quest canvas' };
  const browser = { id: 'browser', name: 'Browser' };
  const nodes = [{ label: '3 nodes', text: 'a\nb\nc', uri: 'file:///w/a.canvas' }];
  const ids = () => offeredLiveContexts().map(c => c.source.id);

  it('keeps one slot per source in arrival order and clears only the source that asked', () => {
    setContextSelection(canvas, nodes);
    setContextSelection(browser, [{ label: 'h1', text: '<h1>Hi</h1>' }]);
    expect(ids()).toEqual(['asgard.questCanvas', 'browser']);
    // A canvas update keeps its place before the browser's chip
    setContextSelection(canvas, [{ label: '1 node', text: 'a' }]);
    expect(ids()).toEqual(['asgard.questCanvas', 'browser']);
    setContextSelection(canvas, []);
    expect(ids()).toEqual(['browser']);
    setContextSelection(browser, undefined);
    expect(ids()).toEqual([]);
  });

  it('hides sent content until the source reports something else or clears', () => {
    setContextSelection(canvas, nodes);
    markContextSent(offeredLiveContexts()[0]!);
    expect(ids()).toEqual([]);
    setContextSelection(canvas, [...nodes]);
    expect(ids()).toEqual([]);
    setContextSelection(canvas, [{ label: '1 node', text: 'a' }]);
    expect(ids()).toEqual(['asgard.questCanvas']);
    markContextSent(offeredLiveContexts()[0]!);
    setContextSelection(canvas, undefined);
    setContextSelection(canvas, [{ label: '1 node', text: 'a' }]);
    expect(ids()).toEqual(['asgard.questCanvas']);
    setContextSelection(canvas, undefined);
  });

  it('goes out as text drafts that remember their source', () => {
    const [d] = contextDrafts(canvas, nodes);
    expect(d).toEqual({ kind: 'text', name: '3 nodes', text: 'a\nb\nc', context: { sourceId: 'asgard.questCanvas', sourceName: 'Quest canvas', uri: 'file:///w/a.canvas' } });
    expect(sameContext(d!, { ...d! })).toBe(true);
    expect(sameContext(d!, { ...d!, context: { ...d!.context!, sourceId: 'browser' } })).toBe(false);
    expect(sameContext(d!, { kind: 'text', name: d!.name, text: d!.text })).toBe(false);
  });
});
