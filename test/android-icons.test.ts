import { describe, expect, it } from 'vitest';
import { androidIconsJson, readAndroidIcons } from '../scripts/export/android-icons';

// The Android app draws these; regenerate with `pnpm exec tsx scripts/export/android-icons.ts`
describe('android icon export', () => {
  it('matches the lucide icons the webview imports and the vendor marks', async () => {
    expect(readAndroidIcons()).toBe(await androidIconsJson());
  });
});
