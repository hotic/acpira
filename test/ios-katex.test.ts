import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { IOS_KATEX_FILE, KATEX_FONTS, iosKatexSwift } from '../scripts/export/ios-katex';

// The iOS math typesetter reads these; regenerate with `pnpm exec tsx scripts/export/ios-katex.ts`
describe('ios katex export', () => {
  it('matches the installed katex metrics', () => {
    expect(readFileSync(IOS_KATEX_FILE, 'utf8')).toBe(iosKatexSwift());
  });
  it('ships the installed katex faces', () => {
    const katex = dirname(createRequire(import.meta.url).resolve('katex/package.json'));
    for (const font of KATEX_FONTS) {
      const bundled = readFileSync(join(dirname(IOS_KATEX_FILE), 'Fonts', `KaTeX_${font}.ttf`));
      expect(bundled.equals(readFileSync(join(katex, 'dist/fonts', `KaTeX_${font}.ttf`))), font).toBe(true);
    }
  });
});
