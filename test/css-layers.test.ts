import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// Cascade layering contract for src/webview/styles (see docs/dev/webview.md): an unlayered rule beats every Tailwind utility
// regardless of specificity, so a stray `.foo { padding: … }` silently overrides the call-site className.
// Top-level rules must therefore be layered (`@layer`, `@utility`, `@theme`, `@keyframes`) or set custom properties
// only, except in the files that exist precisely to beat other unlayered stylesheets.
const DIR = join(import.meta.dirname, '../src/webview/styles');
const UNLAYERED_BY_DESIGN: Record<string, string> = {
  'base.css': 'code / kbd / focus overrides must beat the unlayered host webview stylesheet',
  'prose.css': 'markdown rules must beat the host webview stylesheet and katex.min.css',
  'motion.css': 'keyframes, rail / stream animation hooks and the reduced-motion kill switch',
};

interface Block { prelude: string; body: string }

// Top-level blocks of a stylesheet, comments stripped; nested braces stay inside `body`.
function topLevelBlocks(css: string): Block[] {
  const src = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const blocks: Block[] = [];
  let depth = 0, start = 0, bodyStart = 0;
  for (let i = 0; i < src.length; i++) {
    const ch = src[i];
    if (ch === '{') { if (depth++ === 0) bodyStart = i + 1; }
    else if (ch === '}') {
      if (--depth === 0) {
        blocks.push({ prelude: src.slice(start, bodyStart - 1).trim(), body: src.slice(bodyStart, i) });
        start = i + 1;
      }
    } else if (ch === ';' && depth === 0) start = i + 1;
  }
  return blocks;
}

// Declarations directly inside a block (nested rules excluded).
function ownDeclarations(body: string): string[] {
  const out: string[] = [];
  let depth = 0, cur = '';
  for (const ch of body) {
    if (ch === '{') depth++;
    else if (ch === '}') depth--;
    else if (ch === ';' && depth === 0) { if (cur.trim()) out.push(cur.trim()); cur = ''; continue; }
    if (depth === 0) cur += ch;
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}

const LAYERED = /^@(layer|utility|theme|keyframes|import|property)\b/;

describe('webview stylesheet layering', () => {
  const files = readdirSync(DIR).filter(f => f.endsWith('.css'));

  it('covers the style directory', () => {
    expect(files).toEqual(expect.arrayContaining(['index.css', 'tokens.css', 'chat.css', ...Object.keys(UNLAYERED_BY_DESIGN)]));
  });

  for (const file of files.filter(f => !(f in UNLAYERED_BY_DESIGN))) {
    it(`${file} has no unlayered rule that could shadow a utility`, () => {
      const offenders = topLevelBlocks(readFileSync(join(DIR, file), 'utf8'))
        .filter(b => !LAYERED.test(b.prelude))
        .filter(b => ownDeclarations(b.body).some(d => !d.startsWith('--')))
        .map(b => b.prelude);
      expect(offenders).toEqual([]);
    });
  }

  it('flags an unlayered rule and accepts a variables-only block', () => {
    const blocks = topLevelBlocks(':root { --x: 1; } .foo { --y: 2; padding: 0; } @utility bar { padding: 0; } /* .baz { margin: 0 } */');
    const offenders = blocks.filter(b => !LAYERED.test(b.prelude)).filter(b => ownDeclarations(b.body).some(d => !d.startsWith('--')));
    expect(offenders.map(b => b.prelude)).toEqual(['.foo']);
  });
});
