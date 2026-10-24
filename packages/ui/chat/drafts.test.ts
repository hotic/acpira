import { describe, expect, it, vi } from 'vitest';
import { MAX_FILE_BYTES, MAX_TEXT_BYTES } from '@shared/attachments';
import { collectDrafts, collectImageSources, hasPayload, hiddenText, imageSources, pastedText, pathlessWorkbenchDrag, workbenchUris } from './drafts';

// Minimal DataTransfer: lowercased types like the DOM one, no files
function transfer(data: Record<string, string>): DataTransfer {
  const lower = Object.fromEntries(Object.entries(data).map(([k, v]) => [k.toLowerCase(), v]));
  return {
    types: Object.keys(lower),
    files: [] as unknown as FileList,
    getData: (type: string) => lower[type.toLowerCase()] ?? '',
  } as unknown as DataTransfer;
}

// What `CustomTreeViewDragAndDrop.onDragStart` leaves in the DOM transfer for a tree whose controller declares text/uri-list
const extensionTreeDrag = (paths: string[]) => transfer({
  'text/uri-list': '',
  'text/plain': '',
  ResourceURLs: '',
  CodeFiles: JSON.stringify(paths),
  CodeEditors: JSON.stringify(paths.map(path => ({ resource: { $mid: 1, scheme: 'file', path } }))),
  'application/vnd.code.tree.asgard.explorer': '',
});

describe('workbench drags', () => {
  it('takes the uri-list of the built-in Explorer', () => {
    const dt = transfer({ 'text/uri-list': 'file:///w/a.ts\r\n# comment\r\nfile:///w/b%20c.ts' });
    expect(hasPayload(dt)).toBe(true);
    expect(workbenchUris(dt)).toEqual(['file:///w/a.ts', 'file:///w/b%20c.ts']);
  });

  it('falls back to CodeFiles when an extension tree blanked the uri-list', async () => {
    const dt = extensionTreeDrag(['/w/src/a.ts', '/w/docs/b c.md']);
    expect(hasPayload(dt)).toBe(true);
    expect(workbenchUris(dt)).toEqual(['file:///w/src/a.ts', 'file:///w/docs/b%20c.md']);
    const { drafts, refused } = await collectDrafts(dt, '/w');
    expect(drafts).toEqual([
      { kind: 'file', uri: 'file:///w/src/a.ts', name: 'src/a.ts' },
      { kind: 'file', uri: 'file:///w/docs/b%20c.md', name: 'docs/b c.md' },
    ]);
    expect(refused).toEqual([]);
  });

  it('reads ResourceURLs and CodeEditors when CodeFiles is absent', () => {
    expect(workbenchUris(transfer({ ResourceURLs: JSON.stringify(['file:///w/a.ts', 'vscode-remote://h/x']) }))).toEqual(['file:///w/a.ts']);
    expect(workbenchUris(transfer({ CodeEditors: JSON.stringify([{ resource: { scheme: 'file', path: '/w/e.ts' } }, { resource: 'file:///w/f.ts' }]) })))
      .toEqual(['file:///w/e.ts', 'file:///w/f.ts']);
  });

  it('turns Windows paths into file URIs', () => {
    expect(workbenchUris(transfer({ CodeFiles: JSON.stringify(['C:\\w\\a b.ts']) }))).toEqual(['file:///C:/w/a%20b.ts']);
    // A network share keeps its server as the URI authority
    expect(workbenchUris(transfer({ CodeFiles: JSON.stringify(['\\\\server\\share\\a b.ts']) }))).toEqual(['file://server/share/a%20b.ts']);
  });

  it('reports a drag of remote resources instead of dropping it silently', async () => {
    const dt = transfer({ 'text/uri-list': '', CodeEditors: JSON.stringify([{ resource: { scheme: 'sftp', path: '/srv/a.ts' } }]) });
    const { drafts, refused } = await collectDrafts(dt, '/w');
    expect(drafts).toEqual([]);
    expect(refused).toHaveLength(1);
  });

  it('prefers the workbench uri-list, which names every dragged item', () => {
    // fillEditorsDragData (VS Code 1.140) puts only the first URI into text/uri-list; an extension tree also blanks that one
    const all = 'file:///w/a.ts\r\nfile:///w/b.ts';
    expect(workbenchUris(transfer({ 'text/uri-list': 'file:///w/a.ts', 'application/vnd.code.uri-list': all }))).toEqual(['file:///w/a.ts', 'file:///w/b.ts']);
    const blanked = transfer({ 'text/uri-list': '', 'application/vnd.code.uri-list': all, 'application/vnd.code.tree.asgard.explorer': '' });
    expect(hasPayload(blanked)).toBe(true);
    expect(workbenchUris(blanked)).toEqual(['file:///w/a.ts', 'file:///w/b.ts']);
  });

  it('names the types of a workbench drag that carried no path', () => {
    const tree = transfer({ 'application/vnd.code.tree.asgard.explorer': '{"id":"asgard.explorer"}' });
    expect(hasPayload(tree)).toBe(false);
    expect(pathlessWorkbenchDrag(tree)).toEqual(['application/vnd.code.tree.asgard.explorer']);
    // Drags the composer takes, and plain text, are not reported
    expect(pathlessWorkbenchDrag(extensionTreeDrag(['/w/a.ts']))).toBeUndefined();
    expect(pathlessWorkbenchDrag(transfer({ 'text/plain': 'hello' }))).toBeUndefined();
  });

  it('ignores plain text drags', () => {
    expect(hasPayload(transfer({ 'text/plain': 'hello' }))).toBe(false);
  });
});

// An OS file as the webview sees it: bytes only, `size` overridable so the caps can be hit without allocating them
function osFile(name: string, bytes: Uint8Array | string, size?: number): File {
  const buf = typeof bytes === 'string' ? new TextEncoder().encode(bytes) : bytes;
  return { name, type: '', size: size ?? buf.byteLength, text: async () => new TextDecoder().decode(buf), arrayBuffer: async () => buf.buffer } as unknown as File;
}
const osTransfer = (files: File[]) => ({ types: ['Files'], files, getData: () => '' }) as unknown as DataTransfer;

describe('OS files', () => {
  // Node has no FileReader; this one answers readAsDataURL from the stub's bytes
  vi.stubGlobal('FileReader', class {
    result: string | null = null;
    onload: (() => void) | null = null;
    onerror: (() => void) | null = null;
    readAsDataURL(f: File) {
      void f.arrayBuffer().then(b => { this.result = `data:;base64,${Buffer.from(b).toString('base64')}`; this.onload?.(); });
    }
  });

  it('embeds small text and carries binaries and oversized text as bytes', async () => {
    const mp4 = new Uint8Array([0, 0, 0, 0x18, 0x66, 0x74, 0x79, 0x70]);
    const { drafts, refused } = await collectDrafts(osTransfer([
      osFile('notes.md', '# hi'),
      osFile('录制.mp4', mp4),
      osFile('big.log', 'x', MAX_TEXT_BYTES + 1),
    ]), '/w');
    expect(refused).toEqual([]);
    expect(drafts).toEqual([
      { kind: 'text', name: 'notes.md', text: '# hi' },
      { kind: 'file', uri: `attachment:///${encodeURIComponent('录制.mp4')}`, name: '录制.mp4', data: Buffer.from(mp4).toString('base64') },
      { kind: 'file', uri: 'attachment:///big.log', name: 'big.log', data: Buffer.from('x').toString('base64') },
    ]);
  });

  it('refuses a file over the byte cap', async () => {
    const { drafts, refused } = await collectDrafts(osTransfer([osFile('huge.mov', new Uint8Array([0]), MAX_FILE_BYTES + 1)]), '/w');
    expect(drafts).toEqual([]);
    expect(refused).toHaveLength(1);
    expect(refused[0]).toContain('huge.mov');
  });
});

describe('mixed pastes', () => {
  const png = Buffer.from([0x89, 0x50, 0x4e, 0x47]).toString('base64');
  const clip = (data: Record<string, string>, files: File[] = []) =>
    ({ types: [...Object.keys(data), ...(files.length ? ['Files'] : [])], files, getData: (type: string) => data[type] ?? '' }) as unknown as DataTransfer;

  it('keeps the text that came with a screenshot', () => {
    expect(pastedText(clip({ 'text/plain': 'why does this fail?' }, [osFile('image.png', 'x')]))).toBe('why does this fail?');
  });

  it('drops the file names a Finder copy puts beside the files', () => {
    expect(pastedText(clip({ 'text/plain': 'a.png\nb c.pdf' }, [osFile('a.png', 'x'), osFile('b c.pdf', 'y')]))).toBe('');
    expect(pastedText(clip({ 'text/plain': '/Users/me/a.png' }, [osFile('a.png', 'x')]))).toBe('');
  });

  it("drops the address a browser's Copy Image puts in text/plain", () => {
    const dt = clip({ 'text/plain': 'https://e.com/a.png?x=1&y=2', 'text/html': '<img src="https://e.com/a.png?x=1&amp;y=2">' });
    expect(pastedText(dt)).toBe('');
  });

  it('finds the images of a copied selection in its HTML', async () => {
    const html = `<p>look</p><img alt="" class="size-full" src="data:image/png;base64,${png}"><img src='data:image/svg+xml,%3Csvg%2F%3E'><img src="data:image/png;base64,${png}">`;
    const dt = clip({ 'text/plain': 'look', 'text/html': html });
    expect(pastedText(dt)).toBe('look');
    const sources = imageSources(dt);
    expect(sources).toHaveLength(2);
    // SVG is not an image the models take
    const { drafts, refused } = await collectImageSources(sources);
    expect(refused).toEqual([]);
    expect(drafts).toEqual([{ kind: 'image', mimeType: 'image/png', data: png }]);
  });

  it('takes the text of a mixed message that only carries it in HTML', () => {
    // Qt-style fragment: head and style, comments, source line breaks, <br>, entities, an inline image
    const html = '<html><head><style>p { margin: 0 }</style></head><body><!--StartFragment--><p>你好，\n  这是&nbsp;文字 &amp; 图</p>'
      + '<img src="file:///tmp/a.png"><p>第二行<br />第三行&#x1F600;</p><!--EndFragment--></body></html>';
    expect(pastedText(clip({ 'text/html': html }, [osFile('image.png', 'x')]))).toBe('你好， 这是 文字 & 图\n第二行\n第三行😀');
    // text/plain holding only the attached files' paths does not hide it either
    expect(pastedText(clip({ 'text/plain': '/tmp/a.png', 'text/html': html }, [osFile('a.png', 'x')]))).toContain('第二行');
    // An image-only HTML flavour still types nothing
    expect(pastedText(clip({ 'text/html': '<img src="https://e.com/a.png">' }))).toBe('');
  });

  it('reads back the text Chromium withholds beside pasted files', async () => {
    const onlyFiles = (names: string[]) => clip({}, names.map(n => osFile(n, 'x')));
    const read = vi.fn(async () => [{
      types: ['text/html'],
      getType: async () => new Blob(['<p>看这张图</p><img src="file:///tmp/a.png">'], { type: 'text/html' }),
    }]);
    // Only clipboard is mocked: replacing the whole navigator global leaks into the fetch path of later tests
    Object.defineProperty(navigator, 'clipboard', { value: { read }, configurable: true });
    try {
      // A bare screenshot (synthetic image.png) or a paste that already has text never touches the clipboard
      expect(hiddenText(onlyFiles(['image.png']))).toBeUndefined();
      expect(hiddenText(clip({ 'text/plain': 'x' }, [osFile('a.png', 'x')]))).toBeUndefined();
      expect(read).not.toHaveBeenCalled();
      expect(await hiddenText(onlyFiles(['a.png']))).toBe('看这张图');
      read.mockRejectedValueOnce(new DOMException('denied', 'NotAllowedError'));
      expect(await hiddenText(onlyFiles(['a.png']))).toBe('');
    } finally {
      Reflect.deleteProperty(navigator, 'clipboard');
    }
  });

  it('never requests a pasted image on a loopback port, which the page may reach for open_browser', async () => {
    const fetched: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => { fetched.push(url); return new Response(new Blob([], { type: 'image/png' })); }));
    try {
      const { drafts } = await collectImageSources(['http://localhost:8080/admin/drop.png', 'http://127.0.0.1:3000/x.png', 'http://[::1]/y.png', 'https://example.com/z.png']);
      expect(fetched).toEqual(['https://example.com/z.png']);
      expect(drafts).toHaveLength(1);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
