import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';
import { IOS_SHIKI_DIR, iosShikiFiles } from '../scripts/export/ios-shiki';

const listed = (dir: string): string[] => readdirSync(dir, { withFileTypes: true, recursive: true })
  .filter(entry => entry.isFile()).map(entry => relative(IOS_SHIKI_DIR, join(entry.parentPath, entry.name)));

// The iOS highlighter bundles these; regenerate with `pnpm exec tsx scripts/export/ios-shiki.ts`
describe('ios shiki export', () => {
  it('matches the installed shiki and the webview grammar choice', async () => {
    const files = await iosShikiFiles();
    expect(listed(IOS_SHIKI_DIR).sort()).toEqual([...files.keys()].sort());
    for (const [path, text] of files) expect(readFileSync(join(IOS_SHIKI_DIR, path), 'utf8') === text, path).toBe(true);
  });
});
