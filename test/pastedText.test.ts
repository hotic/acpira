import { describe, expect, it } from 'vitest';
import { collectPastedText, PASTE_TEXT_CHARS, PASTE_TEXT_LINES } from '../src/shared/pastedText';
import { MAX_TEXT_BYTES } from '../src/shared/attachments';
import { preparePrompt, restoreDrafts, type BlobStore } from '../src/host/acp/attachments';

const name = '粘贴的文本.txt';

describe('large text pastes', () => {
  it('leaves short text and ordinary slash commands in the native input', () => {
    for (const text of ['', '你好', '/compact', 'a'.repeat(PASTE_TEXT_CHARS - 1), Array(PASTE_TEXT_LINES - 1).fill('line').join('\n')]) {
      expect(collectPastedText(text, name)).toEqual({});
    }
  });

  it('attaches at the character or line threshold without trimming or normalizing', () => {
    for (const text of ['中'.repeat(PASTE_TEXT_CHARS), ...['\n', '\r\n', '\r'].map(newline => Array(PASTE_TEXT_LINES).fill('  内容  ').join(newline))]) {
      expect(collectPastedText(text, name)).toEqual({ draft: { kind: 'text', name, text } });
    }
  });

  it('enforces the host byte limit, including multibyte text', () => {
    expect(collectPastedText('a'.repeat(MAX_TEXT_BYTES), name).draft).toBeDefined();
    expect(collectPastedText('a'.repeat(MAX_TEXT_BYTES + 1), name)).toEqual({ tooBig: true });
    expect(collectPastedText('中'.repeat(Math.ceil(MAX_TEXT_BYTES / 3)), name)).toEqual({ tooBig: true });
  });

  it('stages the full text as an ACP resource and restores it for editing', async () => {
    const text = '  完整内容\r\n'.repeat(500);
    const { draft } = collectPastedText(text, name);
    let saved = new Uint8Array();
    const blobs: BlobStore = {
      async saveBlob(_session, ext, bytes) {
        expect(ext).toBe('.txt');
        saved = new Uint8Array(bytes);
        return { name: 'pasted.txt', path: '/tmp/pasted.txt' };
      },
      async readBlob() { return saved; },
    };
    const result = await preparePrompt('session', '', [draft!], blobs);
    expect(result.problems).toEqual([]);
    expect(result.blocks).toEqual([{ type: 'resource', resource: { uri: 'file:///tmp/pasted.txt', mimeType: 'text/plain', text } }]);
    expect(new TextDecoder().decode(saved)).toBe(text);
    expect(await restoreDrafts('session', result.attachments, blobs)).toEqual([draft]);
  });
});
