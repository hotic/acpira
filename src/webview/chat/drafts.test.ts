import { describe, expect, it } from 'vitest';
import { collectDrafts, hasPayload, workbenchUris } from './drafts';

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

  it('ignores plain text drags', () => {
    expect(hasPayload(transfer({ 'text/plain': 'hello' }))).toBe(false);
  });
});
