import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { IOS_WEB_DIR, iosWebFiles } from '../scripts/export/ios-web';

// The iOS app bundles these; regenerate with `pnpm exec tsx scripts/export/ios-web.ts`
describe('ios web runtimes export', () => {
  it('matches the installed mermaid and Tailwind browser builds', () => {
    const files = iosWebFiles();
    expect(readdirSync(IOS_WEB_DIR).sort()).toEqual([...files.keys()].sort());
    for (const [name, text] of files) expect(readFileSync(join(IOS_WEB_DIR, name), 'utf8') === text, name).toBe(true);
  });
});
