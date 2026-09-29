import { describe, expect, it } from 'vitest';
import { copiedSelection, markSelectionSent, offeredSelection, sameRange, selectionDraft, setEditorCopy, setEditorSelection } from './editorContext';

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
