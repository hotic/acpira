import { createContext } from 'react';

export type FileLink = { path: string; line?: number };

export const OpenToolFileContext = createContext<((path: string, line?: number) => void) | undefined>(undefined);

// Hash form survives rehype-sanitize / rehype-harden: file: is a hard block and becomes ` [blocked]`.
export const FILE_HREF_PREFIX = '#acpira-file:';

const SKIP_SCHEMES = /^(?:https?|mailto|javascript|data|vbscript|blob|tel):/i;
const WEB_TLD = /^(?:com|org|net|io|edu|gov|cn|co|dev|info|xyz|me|ai)$/i;
const LINE_HASH = /^(.*)#L(\d+)(?:-\d+)?$/i;
const LINE_COLON = /^(.+?):(\d+)(?:[–-]\d+)?(?::\d+)?$/;

export function encodeFileHref(path: string, line?: number): string {
  return `${FILE_HREF_PREFIX}${encodeURIComponent(path)}${line != null ? `:${line}` : ''}`;
}

export function decodeFileHref(href: string): FileLink | undefined {
  if (!href.startsWith(FILE_HREF_PREFIX)) return;
  const payload = href.slice(FILE_HREF_PREFIX.length);
  if (!payload) return;
  const numbered = /^(.*):(\d+)$/.exec(payload);
  try {
    if (numbered) return { path: decodeURIComponent(numbered[1]!), line: Number(numbered[2]) };
    return { path: decodeURIComponent(payload) };
  } catch {
    return;
  }
}

// Workspace paths in agent markdown / inline code, including file:// which harden would otherwise drop.
export function parseFileLink(text: string): FileLink | undefined {
  const raw = text.trim();
  if (!raw || /\s/.test(raw)) return;
  const hashed = decodeFileHref(raw);
  if (hashed) return hashed.path ? hashed : undefined;
  if (raw.startsWith('#') || SKIP_SCHEMES.test(raw)) return;
  if (/^file:/i.test(raw)) return parseFileUrl(raw);
  return parsePathAndLine(raw);
}

// A whole Markdown link inside inline code (`[Shell.tsx:12](file:///repo/Shell.tsx)`): the target is the file, the label is what was
// meant to show. A line number comes from the target first, then from the label.
export type WrappedLink = { label: string; file: FileLink };

const WRAPPED_LINK = /^\[([^[\]]+)\]\(<?([^\s()<>]+)>?\)$/;

export function parseWrappedLink(text: string): WrappedLink | undefined {
  const hit = WRAPPED_LINK.exec(text.trim());
  if (!hit) return;
  const label = hit[1]!.trim();
  const target = parseFileLink(hit[2]!);
  if (!label || !target) return;
  const line = target.line ?? parsePathAndLine(label)?.line;
  return { label, file: line != null ? { path: target.path, line } : { path: target.path } };
}

export function toFileHref(url: string): string {
  const file = parseFileLink(url);
  return file ? encodeFileHref(file.path, file.line) : url;
}

// Attacher: must run after rehype-raw and before sanitize/harden so file:// is already a hash when those run.
export function rewriteFileHrefs() {
  return (tree: HastNode) => walk(tree);
}

interface HastNode {
  type: string;
  tagName?: string;
  properties?: { href?: unknown };
  children?: HastNode[];
}

function walk(node: HastNode): void {
  if (node.tagName === 'a' && typeof node.properties?.href === 'string') {
    const href = node.properties.href;
    if (!href.startsWith(FILE_HREF_PREFIX)) {
      const file = parseFileLink(href);
      if (file) node.properties.href = encodeFileHref(file.path, file.line);
    }
  }
  node.children?.forEach(walk);
}

function parseFileUrl(href: string): FileLink | undefined {
  let path: string | undefined;
  let hash = '';
  try {
    const uri = new URL(href);
    if (uri.protocol !== 'file:') return;
    path = decodeURIComponent(uri.pathname);
    if (uri.host && uri.host !== 'localhost') path = `//${uri.host}${path}`;
    else if (/^\/[A-Za-z]:/.test(path)) path = path.slice(1);
    hash = uri.hash;
  } catch {
    const stripped = href.replace(/^file:\/\//i, '');
    path = decodeURIComponent(stripped);
  }
  if (!path) return;
  const fromPath = parsePathAndLine(hash ? `${path}${hash}` : path);
  if (fromPath) return fromPath;
  const line = lineFromHash(hash);
  return isFilePath(path) ? { path, line } : undefined;
}

function parsePathAndLine(text: string): FileLink | undefined {
  const hashed = LINE_HASH.exec(text);
  if (hashed) {
    const path = hashed[1]!;
    return isFilePath(path) ? { path, line: Number(hashed[2]) } : undefined;
  }
  const colon = LINE_COLON.exec(text);
  if (colon && isFilePath(colon[1]!)) return { path: colon[1]!, line: Number(colon[2]) };
  return isFilePath(text) ? { path: text } : undefined;
}

function lineFromHash(hash: string): number | undefined {
  const hit = /^#L(\d+)/i.exec(hash);
  return hit ? Number(hit[1]) : undefined;
}

function isFilePath(path: string): boolean {
  if (!path) return false;
  if (/^(?:\.{0,2}\/|[A-Za-z]:[\\/]|\/)/.test(path)) return true;
  if (path.startsWith('.') && /[\\/]/.test(path)) return true;
  if (/[\\/]/.test(path) && /\.[A-Za-z][\w-]*$/.test(path)) return true;
  const ext = /^[^\s/\\]+\.([A-Za-z][\w-]{1,15})$/.exec(path);
  return !!ext && !WEB_TLD.test(ext[1]!);
}
