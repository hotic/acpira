import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

// Tailwind's `group-has-*` compiles to `:where(.group):has(…) *`. Chromium cannot narrow that: any DOM change under a
// `.group` (a composer re-render on every keystroke) restyles the group's whole subtree, i.e. the shell with the full
// transcript (docs/dev/webview.md, Render budget). Use `has-*` on the element it styles, a sibling form, or `[:focus-visible>&]`.
const ROOT = join(import.meta.dirname, '..');
const SKIP = new Set(['node_modules', 'test']);

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(e => {
    if (SKIP.has(e.name)) return [];
    const path = join(dir, e.name);
    if (e.isDirectory()) return sources(path);
    return /\.(tsx?|css)$/.test(e.name) ? [path] : [];
  });
}

describe('webview :has() selectors', () => {
  it('no group-has-* variant in packages/ui', () => {
    const hits = sources(ROOT).flatMap(file => readFileSync(file, 'utf8').split('\n')
      .flatMap((line, i) => /\bgroup-has-/.test(line) ? [`${relative(ROOT, file)}:${i + 1}`] : []));
    expect(hits).toEqual([]);
  });

  // WKWebView fails to invalidate `:has(… :last-child …)` when a later sibling is appended after the matching child,
  // leaving the ancestor styled as if the old child were still last (e.g. an auto `/compact` turn appended to an exchange).
  it('no :last-child inside :has() in packages/ui', () => {
    const hits = sources(ROOT).flatMap(file => readFileSync(file, 'utf8').split('\n')
      .flatMap((line, i) => {
        const code = line.replace(/\/\/.*$/, '');
        return /(?::has\([^)]*:last-child|\bhas-\[[^\]]*:last-child)/.test(code) ? [`${relative(ROOT, file)}:${i + 1}`] : [];
      }));
    expect(hits).toEqual([]);
  });
});
