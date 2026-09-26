import { afterEach, describe, expect, it, vi } from 'vitest';
import { copyImage } from './copyImage';

type Item = { data: Record<string, Promise<Blob>> };

const PNG_BYTES = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);

function stubClipboard() {
  const written: Blob[] = [];
  const write = vi.fn(async (items: Item[]) => { written.push(await items[0]!.data['image/png']!); });
  vi.stubGlobal('ClipboardItem', class { constructor(readonly data: Record<string, Promise<Blob>>) {} });
  vi.stubGlobal('navigator', { clipboard: { write } });
  return { write, written };
}

function stubFetch(body: Blob) {
  vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, blob: async () => body })));
}

function stubCanvas() {
  const drawImage = vi.fn();
  vi.stubGlobal('document', { createElement: () => ({
    getContext: () => ({ drawImage }),
    toBlob: (callback: (blob: Blob) => void) => callback(new Blob(['converted'], { type: 'image/png' })),
  }) });
  return drawImage;
}

afterEach(() => vi.unstubAllGlobals());

describe('copyImage', () => {
  it('writes PNG bytes as-is with the clipboard type set', async () => {
    const { write, written } = stubClipboard();
    stubFetch(new Blob([PNG_BYTES], { type: 'application/octet-stream' }));

    await copyImage('/blobs/image.png', 'image/png');
    expect(write).toHaveBeenCalledOnce();
    expect(written[0]?.type).toBe('image/png');
    expect(new Uint8Array(await written[0]!.arrayBuffer())).toEqual(PNG_BYTES);
  });

  it('converts a JPEG mislabelled as PNG instead of trusting the declared type', async () => {
    const { written } = stubClipboard();
    const close = vi.fn();
    stubFetch(new Blob([new Uint8Array([0xff, 0xd8, 0xff, 0xe0])]));
    vi.stubGlobal('createImageBitmap', vi.fn(async () => ({ width: 12, height: 8, close })));
    const drawImage = stubCanvas();

    await copyImage('/blobs/image.png', 'image/png');
    expect(drawImage).toHaveBeenCalledOnce();
    expect(close).toHaveBeenCalledOnce();
    expect(await written[0]!.text()).toBe('converted');
  });

  it('rasterizes SVG through an image element, since createImageBitmap rejects it', async () => {
    const { written } = stubClipboard();
    const bitmap = vi.fn();
    const sources: string[] = [];
    stubFetch(new Blob(['<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>'], { type: 'image/svg+xml' }));
    vi.stubGlobal('createImageBitmap', bitmap);
    vi.stubGlobal('FileReader', class {
      result: string | null = null;
      onload: (() => void) | null = null;
      readAsDataURL() { this.result = 'data:image/svg+xml;base64,AAAA'; this.onload?.(); }
    });
    vi.stubGlobal('Image', class {
      naturalWidth = 4;
      naturalHeight = 4;
      set src(value: string) { sources.push(value); }
      decode() { return Promise.resolve(); }
    });
    const drawImage = stubCanvas();

    await copyImage('/blobs/icon.svg', 'image/svg+xml');
    expect(bitmap).not.toHaveBeenCalled();
    expect(sources).toEqual(['data:image/svg+xml;base64,AAAA']);
    expect(drawImage).toHaveBeenCalledOnce();
    expect(await written[0]!.text()).toBe('converted');
  });

  it('rejects when the image request fails', async () => {
    const { write } = stubClipboard();
    write.mockImplementation(async (items: Item[]) => { await items[0]!.data['image/png']; });
    vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false, status: 404 })));

    await expect(copyImage('/blobs/missing.png', 'image/png')).rejects.toThrow('404');
  });
});
