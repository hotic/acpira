import type { Draft } from '@shared/transcript';
import { MAX_FILE_BYTES, MAX_IMAGE_BYTES, MAX_TEXT_BYTES, imageMimeOf } from '@shared/attachments';
import { t } from '../i18n';

export interface Collected {
  drafts: Draft[];
  // Why something was left out, one line each, for a toast
  refused: string[];
}

// Workbench drag formats (VS Code `DataTransfers` / `CodeDataTransfers`). A drag from an extension tree view (`TreeDragAndDropController`) arrives with
// `text/uri-list` and `ResourceURLs` blanked: `CustomTreeViewDragAndDrop.addExtensionProvidedTransferTypes` sets every declared `dragMimeTypes` entry to ''
// after `fillEditorsDragData` filled them, and the controller's own `handleDrag` data never reaches the DOM. `CodeFiles` (fs paths of file: items) and
// `CodeEditors` survive, so they are the fallbacks, as does `application/vnd.code.uri-list` (`DataTransfers.INTERNAL_URI_LIST`, every dragged URI, where
// `text/uri-list` holds only the first; written by `fillEditorsDragData` in VS Code 1.140). Types are compared lowercased because DataTransfer lowercases
// custom formats
const URI_LIST = 'text/uri-list';
const CODE_URI_LIST = 'application/vnd.code.uri-list';
const CODE_FILES = 'codefiles';
const RESOURCE_URLS = 'resourceurls';
const CODE_EDITORS = 'codeeditors';
const PAYLOAD_TYPES = new Set(['files', URI_LIST, CODE_URI_LIST, CODE_FILES, RESOURCE_URLS, CODE_EDITORS]);
// Formats only the workbench writes (tree MIME `application/vnd.code.tree.<id>`, `CodeEditors`, …): a drag carrying one came from VS Code itself
const WORKBENCH_PREFIX = 'application/vnd.code.';

// Whether a drag carries anything the composer can take: workbench resources (Explorer, extension trees, editor tabs) or real files (Finder / clipboard)
export function hasPayload(dt: DataTransfer | null): boolean {
  if (!dt) return false;
  return [...dt.types].some(type => PAYLOAD_TYPES.has(type.toLowerCase()));
}

// The types of a workbench drag that arrived without any format the composer reads, for the notice that reports it; undefined for any other drag
// (plain text, or one `hasPayload` takes). An extension tree drag that reaches the webview this way would otherwise drop without a trace
export function pathlessWorkbenchDrag(dt: DataTransfer | null): string[] | undefined {
  if (!dt || hasPayload(dt)) return undefined;
  const types = [...dt.types];
  return types.some(type => type.toLowerCase().startsWith(WORKBENCH_PREFIX)) ? types : undefined;
}

// Everything a paste or drop can carry, turned into drafts. Workbench drags arrive as file: URIs and take precedence: those are handed to the host as
// file drafts (it reads images itself). OS files only exist as blobs here (no path in a webview): accepted images go inline, small text files are embedded, anything else up to MAX_FILE_BYTES is carried as bytes
export async function collectDrafts(dt: DataTransfer, cwd: string): Promise<Collected> {
  const out: Collected = { drafts: [], refused: [] };
  const uris = workbenchUris(dt);
  if (uris.length) {
    for (const uri of uris) out.drafts.push({ kind: 'file', uri, name: nameOf(uri, cwd) });
    return out;
  }
  for (const f of Array.from(dt.files)) {
    const mimeType = imageMimeOf(f.name) ?? (f.type.startsWith('image/') && imageMimeOf(`.${f.type.slice(6)}`));
    if (mimeType) {
      if (f.size > MAX_IMAGE_BYTES) { out.refused.push(t('attach.tooBigImage', { name: f.name, mb: MAX_IMAGE_BYTES >> 20 })); continue; }
      out.drafts.push({ kind: 'image', mimeType, data: await base64Of(f), name: f.name === 'image.png' ? undefined : f.name });
      continue;
    }
    // Small plain text is embedded; binaries (video, archives …) and oversized text travel as bytes for the host to stage and link
    const text = f.size <= MAX_TEXT_BYTES ? await f.text() : undefined;
    if (text !== undefined && !text.includes('\0')) { out.drafts.push({ kind: 'text', name: f.name, text }); continue; }
    if (f.size > MAX_FILE_BYTES) { out.refused.push(t('attach.tooBigFile', { name: f.name, mb: MAX_FILE_BYTES >> 20 })); continue; }
    out.drafts.push({ kind: 'file', uri: `attachment:///${encodeURIComponent(f.name)}`, name: f.name, data: await base64Of(f) });
  }
  // A workbench drag that named only remote / virtual resources (or nothing readable) would otherwise drop without a trace
  if (!dt.files.length && [...dt.types].some(type => type.toLowerCase() !== 'files' && PAYLOAD_TYPES.has(type.toLowerCase()))) out.refused.push(t('attach.noLocalFiles'));
  return out;
}

// file: URIs of a workbench drag, from the first format that yields any. Non-file resources (remote / virtual file systems) are skipped: the host reads real paths only
export function workbenchUris(dt: DataTransfer): string[] {
  const read = (type: string) => { try { return dt.getData(type); } catch { return ''; } };
  const json = (type: string): unknown => { const raw = read(type); if (!raw) return undefined; try { return JSON.parse(raw); } catch { return undefined; } };
  const files = (list: unknown[]) => [...new Set(list.filter((u): u is string => typeof u === 'string' && /^file:/i.test(u)))];

  const lines = (type: string) => read(type).split(/\r?\n/).map(l => l.trim()).filter(l => l && !l.startsWith('#'));
  const candidates: (() => unknown[])[] = [
    // The workbench list first: it names every dragged item, the standard uri-list only the first
    () => lines(CODE_URI_LIST),
    () => lines(URI_LIST),
    () => { const v = json(CODE_FILES); return Array.isArray(v) ? v.map(p => typeof p === 'string' ? fileUriOf(p) : undefined) : []; },
    () => { const v = json(RESOURCE_URLS); return Array.isArray(v) ? v : []; },
    () => { const v = json(CODE_EDITORS); return Array.isArray(v) ? v.map(e => uriOfEditor(e)) : []; },
  ];
  for (const candidate of candidates) {
    const found = files(candidate());
    if (found.length) return found;
  }
  return [];
}

// An absolute fs path (POSIX or Windows) as a file: URI
function fileUriOf(path: string): string | undefined {
  if (!path) return undefined;
  const posix = path.replace(/\\/g, '/');
  // A UNC path (`\\server\share\a.ts`) carries its server as the URI authority, the form VS Code itself uses
  const unc = /^\/\/([^/]+)(\/.*)$/.exec(posix);
  if (unc) return `file://${unc[1]}${unc[2]!.split('/').map(encodeURIComponent).join('/')}`;
  const absolute = posix.startsWith('/') ? posix : /^[a-z]:\//i.test(posix) ? `/${posix}` : undefined;
  if (!absolute) return undefined;
  return `file://${absolute.split('/').map(encodeURIComponent).join('/').replace(/^\/([a-z])%3A/i, '/$1:')}`;
}

// `CodeEditors` entries carry `resource` as a serialized URI (`{ scheme, path, external? }`) or, in older builds, as a string
function uriOfEditor(editor: unknown): string | undefined {
  const resource = (editor as { resource?: unknown } | null)?.resource;
  if (typeof resource === 'string') return resource;
  if (!resource || typeof resource !== 'object') return undefined;
  const r = resource as { scheme?: unknown; path?: unknown; external?: unknown };
  if (typeof r.external === 'string') return r.external;
  return r.scheme === 'file' && typeof r.path === 'string' ? fileUriOf(r.path) : undefined;
}

// Path relative to the workspace when inside it, otherwise the file name
export function nameOf(uri: string, cwd: string): string {
  let path: string;
  try { path = decodeURIComponent(new URL(uri).pathname); } catch { return uri; }
  // Windows: the URI path is `/c:/repo/a.ts` while cwd is `C:\repo`; both become `c:/repo…`-style and compare case-insensitively
  const slashed = (p: string) => p.replace(/\\/g, '/').replace(/^\/([a-z]:)/i, '$1');
  path = slashed(path);
  const base = slashed(cwd);
  const root = base.endsWith('/') ? base : `${base}/`;
  const inside = /^[a-z]:/i.test(root) ? path.toLowerCase().startsWith(root.toLowerCase()) : path.startsWith(root);
  return inside ? path.slice(root.length) : path.split('/').pop() || path;
}

function base64Of(f: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve((r.result as string).split(',', 2)[1] ?? '');
    r.onerror = () => reject(r.error);
    r.readAsDataURL(f);
  });
}
