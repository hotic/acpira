import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { IOS_REMEND_FILE, iosRemendSwift } from '../scripts/export/ios-remend';

// The iOS remend port replays these; regenerate with `pnpm exec tsx scripts/export/ios-remend.ts`
describe('ios remend export', () => {
  it('matches the installed remend', async () => {
    expect(readFileSync(IOS_REMEND_FILE, 'utf8')).toBe(await iosRemendSwift());
  });
});
