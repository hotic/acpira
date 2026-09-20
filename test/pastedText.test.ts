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

describe('prompt capabilities', () => {
  const blobs: BlobStore = {
    async saveBlob() { return { name: 'blob', path: '/tmp/blob' }; },
    async readBlob() { return new Uint8Array(); },
  };
  const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

  it('embeddedContext: false sends dropped text as marked-up plain text, still staging the blob', async () => {
    const result = await preparePrompt('session', 'hi', [{ kind: 'text', name: 'notes.txt', text: 'contents' }], blobs, { embeddedContext: false });
    expect(result.problems).toEqual([]);
    expect(result.blocks).toEqual([
      { type: 'text', text: 'hi' },
      { type: 'text', text: '[Attachment: notes.txt]\ncontents\n[End of attachment: notes.txt]' },
    ]);
    expect(result.attachments).toEqual([{ kind: 'text', blob: 'blob', name: 'notes.txt' }]);
  });

  it('image: false drops pasted images with a problem instead of a block', async () => {
    const result = await preparePrompt('session', 'hi', [{ kind: 'image', name: 'shot.png', mimeType: 'image/png', data: png }], blobs, { image: false });
    expect(result.blocks).toEqual([{ type: 'text', text: 'hi' }]);
    expect(result.attachments).toEqual([]);
    expect(result.problems).toHaveLength(1);
    expect(result.problems[0]).toContain('shot.png');
  });

  it('imagesRegardless overrides a false image capability (Grok)', async () => {
    const result = await preparePrompt('session', 'hi', [{ kind: 'image', name: 'shot.png', mimeType: 'image/png', data: png }], blobs, { image: false, imagesRegardless: true });
    expect(result.problems).toEqual([]);
    expect(result.blocks[1]).toEqual({ type: 'image', mimeType: 'image/png', data: png });
    expect(result.attachments).toEqual([{ kind: 'image', blob: 'blob', mimeType: 'image/png', name: 'shot.png' }]);
  });

  it('no caps at all keeps the historical behaviour', async () => {
    const result = await preparePrompt('session', '', [{ kind: 'image', name: 'shot.png', mimeType: 'image/png', data: png }], blobs);
    expect(result.blocks[0]?.type).toBe('image');
  });
});
