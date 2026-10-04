import { describe, expect, it, vi } from 'vitest';
import { MAX_FILE_BYTES, MAX_TEXT_BYTES } from '@shared/attachments';
import { collectDrafts, hasPayload, pathlessWorkbenchDrag, workbenchUris } from './drafts';

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
