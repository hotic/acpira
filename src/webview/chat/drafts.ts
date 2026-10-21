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

// `<img src>` values of a paste's HTML flavour. A selection copied from a web page or from this transcript carries its images only there,
// beside the text in text/plain. Parsed with a pattern rather than DOMParser so nothing in the markup is ever instantiated
const IMG_SRC = /<img\b[^>]*?\ssrc\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))/gi;
const ENTITIES: Record<string, string> = { amp: '&', quot: '"', apos: "'", '#39': "'", lt: '<', gt: '>', nbsp: ' ' };
function decodeEntities(s: string): string {
  return s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+\d*);/gi, (whole, e: string) => {
    if (e[0] !== '#') return ENTITIES[e.toLowerCase()] ?? whole;
    const code = e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
    return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : whole;
  });
}
function srcsOf(html: string): string[] {
  const found = Array.from(html.matchAll(IMG_SRC), m => decodeEntities(m[1] ?? m[2] ?? m[3] ?? '').trim());
  return [...new Set(found.filter(Boolean))];
}
export function imageSources(dt: DataTransfer): string[] {
  const html = dt.getData('text/html');
  return html ? srcsOf(html) : [];
}

// The readable text of an HTML flavour, for pastes that carry their words only there (a Qt chat app's mixed message has text/html and
// images but no text/plain). Pattern-based like IMG_SRC: source whitespace collapses, <br> and block ends break lines, every tag drops
const BLOCK_END = /<br\b[^>]*>|<\/(?:p|div|li|tr|h[1-6]|blockquote|pre|table|ul|ol|section|article|header|footer)\s*>/gi;
export function htmlText(html: string): string {
  const body = html
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<(head|script|style|title|template)\b[\s\S]*?<\/\1\s*>/gi, '')
    .replace(/\s+/g, ' ')
    .replace(BLOCK_END, '\n')
    .replace(/<[^>]*>/g, '');
  return decodeEntities(body).split('\n').map(l => l.trim()).join('\n').replace(/\n{3,}/g, '\n\n').trim();
}

// The text a paste should type. A file copied in Finder also puts its name in text/plain, and a browser's "Copy image" may put the image's
// address there; neither is message text, so either comes back empty. Without anything left in text/plain the HTML flavour's text is used
export function pastedText(dt: DataTransfer): string {
  return typedText(dt.getData('text/plain'), dt.getData('text/html'), Array.from(dt.files, f => f.name));
}
function typedText(text: string, html: string, files: string[]): string {
  const plain = plainText(text, html, files);
  if (plain.trim() || !html) return plain;
  return htmlText(html) || plain;
}
function plainText(text: string, html: string, files: string[]): string {
  const lines = text.split(/\r\n|\r|\n/).map(l => l.trim()).filter(Boolean);
  if (!lines.length) return text;
  const names = new Set(files);
  if (names.size && lines.every(l => names.has(l.split(/[\\/]/).pop()!))) return '';
  if (lines.length === 1 && html && srcsOf(html).includes(lines[0]!)) return '';
  return text;
}

// Chromium on macOS gives a paste only its files once the pasteboard also holds file URLs (a mixed message copied from WeChat: the images
// as files beside HTML and text), so text/plain and text/html read empty. The async Clipboard API still reads them; VS Code grants its
// webviews clipboard-read, elsewhere a refusal just leaves the text out. Only for real files: a bare screenshot arrives as the synthetic
// image.png, and Finder copies come back as their own names, which typedText drops. Undefined when nothing can be hidden
export function hiddenText(dt: DataTransfer): Promise<string> | undefined {
  const files = Array.from(dt.files, f => f.name);
  if (!files.length || [...dt.types].some(type => type !== 'Files') || files.every(n => n === 'image.png')) return undefined;
  if (typeof navigator === 'undefined' || !navigator.clipboard?.read) return undefined;
  return readClipboardText(files);
}
async function readClipboardText(files: string[]): Promise<string> {
  try {
    const items = await navigator.clipboard.read();
    const read = async (type: string) => {
      const item = items.find(i => i.types.includes(type));
      return item ? (await item.getType(type)).text() : '';
    };
    return typedText(await read('text/plain'), await read('text/html'), files);
  } catch {
    return '';
  }
}

// Types text at the caret like a native paste: execCommand keeps the textarea's undo stack and fires the input event React reads;
// setRangeText plus a synthetic input event where the command is refused
export function typeInto(el: HTMLTextAreaElement | null, text: string): void {
  if (!el || !text) return;
  el.focus();
  if (document.execCommand('insertText', false, text)) return;
  el.setRangeText(text, el.selectionStart, el.selectionEnd, 'end');
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

// Image drafts for `imageSources`: data: URLs decode in place (the CSP's connect-src keeps fetch() from them), anything else is fetched,
// which reaches this webview's own resources (sent images in the blob store) and quietly skips what the CSP or the network refuses
export async function collectImageSources(sources: string[]): Promise<Collected> {
  const out: Collected = { drafts: [], refused: [] };
  for (const src of sources) {
    const image = await readImageSource(src).catch(() => undefined);
    if (!image) continue;
    if (image.bytes > MAX_IMAGE_BYTES) { out.refused.push(t('attach.tooBigImage', { name: t('common.image'), mb: MAX_IMAGE_BYTES >> 20 })); continue; }
    out.drafts.push({ kind: 'image', mimeType: image.mimeType, data: image.data });
  }
  return out;
}

// The chat page's CSP reaches loopback ports (open_browser's probe, `bridge.ts`), so a pasted <img> pointing at one is not
// fetched unless it is the page's own origin (the browser harness): a paste never sends a request to a local service
const LOOPBACK_HOST = /^(localhost|0\.0\.0\.0|127(\.\d{1,3}){3}|\[::1?\])$/i;
function fetchable(src: string): boolean {
  if (/^blob:/i.test(src)) return true;
  if (!/^https?:/i.test(src)) return false;
  try {
    const url = new URL(src);
    return !LOOPBACK_HOST.test(url.hostname) || url.origin === globalThis.location?.origin;
  } catch {
    return false;
  }
}

async function readImageSource(src: string): Promise<{ mimeType: string; data: string; bytes: number } | undefined> {
  const accepted = (type: string) => (type.startsWith('image/') ? imageMimeOf(`.${type.slice(6)}`) : undefined);
  const inline = /^data:([^;,]*)((?:;[^;,]*)*?)(;base64)?,(.*)$/is.exec(src);
  if (inline) {
    const [, type = '', , base64, payload = ''] = inline;
    const mimeType = accepted(type.toLowerCase());
    if (!mimeType) return undefined;
    // Base64 size without decoding; a percent-encoded payload is re-encoded
    if (base64) return { mimeType, data: payload, bytes: Math.floor(payload.length * 3 / 4) };
    const raw = decodeURIComponent(payload);
    return { mimeType, data: btoa(raw), bytes: raw.length };
  }
  if (!fetchable(src)) return undefined;
  const response = await fetch(src);
  if (!response.ok) return undefined;
  const blob = await response.blob();
  let path = src;
  try { path = new URL(src).pathname; } catch { /* keep the raw source */ }
  const mimeType = accepted(blob.type.toLowerCase()) ?? imageMimeOf(path);
  if (!mimeType) return undefined;
  if (blob.size > MAX_IMAGE_BYTES) return { mimeType, data: '', bytes: blob.size };
  return { mimeType, data: await base64Of(blob), bytes: blob.size };
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

function base64Of(f: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve((r.result as string).split(',', 2)[1] ?? '');
    r.onerror = () => reject(r.error);
    r.readAsDataURL(f);
  });
}
