import { describe, expect, it } from 'vitest';
import { editorSelectionOf } from '../src/host/editorSelection';
import { MAX_TEXT_BYTES } from '../src/shared/attachments';

// Mirrored by idea/frontend EditorRangeTest: both shells must label the same range the same way
describe('editor selection ranges', () => {
  it('turns 0-based positions into 1-based inclusive lines', () => {
    expect(editorSelectionOf('file:///w/a.ts', { line: 11, character: 2 }, { line: 18, character: 5 }, 'x')).toEqual({ uri: 'file:///w/a.ts', startLine: 12, endLine: 19, text: 'x' });
  });

  it('leaves out a last line the selection only touches at column 0', () => {
    expect(editorSelectionOf('file:///w/a.ts', { line: 11, character: 0 }, { line: 19, character: 0 }, 'x')?.endLine).toBe(19);
    expect(editorSelectionOf('file:///w/a.ts', { line: 11, character: 0 }, { line: 11, character: 0 }, 'x')?.endLine).toBe(12);
  });

  it('offers nothing blank or oversized', () => {
    expect(editorSelectionOf('file:///w/a.ts', { line: 0, character: 0 }, { line: 1, character: 0 }, '  \n')).toBeUndefined();
    expect(editorSelectionOf('file:///w/a.ts', { line: 0, character: 0 }, { line: 0, character: 1 }, 'x'.repeat(MAX_TEXT_BYTES + 1))).toBeUndefined();
  });
});
