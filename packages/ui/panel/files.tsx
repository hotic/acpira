import type { GitMark } from '@acpira/shared/protocol';
import { File, FileCode2, FileJson, FileText, Image as ImageIcon } from 'lucide-react';
import { cn } from '../ui/cn';

// Small pieces the tree, the viewer and the review share

export const nameOf = (path: string) => path.slice(path.lastIndexOf('/') + 1);
export const dirOf = (path: string) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');

// The absolute path of `rel` under the project folder, in the folder's own separators
export function absPath(root: string, rel: string): string {
  if (/^([a-z]:)?[\\/]/i.test(rel)) return rel;
  const sep = root.includes('\\') && !root.includes('/') ? '\\' : '/';
  return `${root.replace(/[\\/]+$/, '')}${sep}${sep === '\\' ? rel.replace(/\//g, '\\') : rel}`;
}

// `path` (absolute, as a tool call names it) relative to the project folder, undefined when outside it
export function relPath(root: string, path: string): string | undefined {
  const slashed = (p: string) => p.replace(/\\/g, '/');
  const base = slashed(root).replace(/\/+$/, '');
  const p = slashed(path);
  const windows = /^[a-z]:/i.test(base);
  const inside = windows ? p.toLowerCase().startsWith(`${base.toLowerCase()}/`) : p.startsWith(`${base}/`);
  return inside ? p.slice(base.length + 1) : undefined;
}

export function FileGlyph({ name, className }: { name: string; className?: string }) {
  const ext = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1).toLowerCase() : '';
  const Icon = /^(tsx?|jsx?|mjs|cjs|rs|kt|kts|swift|css|scss|html|py|go|sh|java|c|cc|cpp|h|hpp|rb|php|lua|zig|vue|svelte)$/.test(ext) ? FileCode2
    : /^(md|mdx|txt|log|rst)$/.test(ext) ? FileText
    : /^(json|jsonc|toml|ya?ml|lock|xml|ini)$/.test(ext) ? FileJson
    : /^(png|jpe?g|gif|svg|webp|icns|ico|avif|bmp)$/.test(ext) ? ImageIcon
    : File;
  return <Icon strokeWidth={1.5} className={cn('size-icon shrink-0', className)} />;
}

// git marks as the tree shows them: severity for a folder's dot (its most severe descendant), the letter's colour
const SEVERITY: Record<GitMark, number> = { D: 3, M: 2, A: 1, U: 1, I: 0 };
export const MARK_TEXT: Record<GitMark, string> = { M: 'text-warn', A: 'text-ok', U: 'text-ok', D: 'text-danger', I: 'text-fg-3' };
export const MARK_DOT: Record<GitMark, string> = { M: 'bg-warn', A: 'bg-ok', U: 'bg-ok', D: 'bg-danger', I: 'bg-fg-3' };

// A path's mark: its own, or the one of a folder above it listed as a whole (`dist/` ignored, `new/` untracked); a folder
// takes its most severe descendant's, ignored ones aside. Built once per status, so a row looks its mark up instead of
// walking every mark
export type MarkLookup = (path: string, dir: boolean) => GitMark | undefined;
export function markIndex(marks: Record<string, GitMark>): MarkLookup {
  const wholes = Object.keys(marks).filter(p => p.endsWith('/'));
  const folders = new Map<string, GitMark>();
  for (const [p, m] of Object.entries(marks)) {
    if (m === 'I') continue;
    const parts = p.replace(/\/$/, '').split('/');
    for (let i = 1; i < parts.length; i++) {
      const dir = parts.slice(0, i).join('/');
      const held = folders.get(dir);
      if (!held || SEVERITY[m] > SEVERITY[held]) folders.set(dir, m);
    }
  }
  return (path, dir) => {
    const self = dir ? `${path}/` : path;
    for (const w of wholes) if (self.startsWith(w)) return marks[w];
    return dir ? folders.get(path) : marks[path];
  };
}

// Folders first, then files, without case: the order an editor's explorer uses (the engine lists folders the same way)
export interface TreeNode { name: string; path: string; dir: boolean; children: TreeNode[] }
export function buildTree(paths: string[]): TreeNode[] {
  const root: TreeNode = { name: '', path: '', dir: true, children: [] };
  for (const path of paths) {
    const parts = path.split('/').filter(Boolean);
    let at = root;
    parts.forEach((name, i) => {
      let child = at.children.find(c => c.name === name);
      if (!child) {
        child = { name, path: parts.slice(0, i + 1).join('/'), dir: i < parts.length - 1, children: [] };
        at.children.push(child);
      }
      at = child;
    });
  }
  const order = (n: TreeNode) => {
    n.children.sort((a, b) => (a.dir !== b.dir ? (a.dir ? -1 : 1) : a.name.toLowerCase().localeCompare(b.name.toLowerCase())));
    n.children.forEach(order);
  };
  order(root);
  return root.children;
}
