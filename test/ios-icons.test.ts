import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { IOS_ICONS_FILE, IOS_MARKS_FILE, iosIconsSwift, iosMarksSwift } from '../scripts/export/ios-icons';

// The iOS app draws these; regenerate with `pnpm exec tsx scripts/export/ios-icons.ts`
describe('ios icon export', () => {
  it('matches the lucide icons the webview imports', async () => {
    expect(readFileSync(IOS_ICONS_FILE, 'utf8')).toBe(await iosIconsSwift());
  });
  it('matches the vendor marks and brand rules', () => {
    expect(readFileSync(IOS_MARKS_FILE, 'utf8')).toBe(iosMarksSwift());
  });
});
