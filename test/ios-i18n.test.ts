import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { DICTS, LOCALES } from '@acpira/shared/i18n';
import { IOS_I18N_DIR, iosI18nJson } from '../scripts/export/ios-i18n';

// The iOS app bundles these files; regenerate with `pnpm exec tsx scripts/export/ios-i18n.ts`
describe('ios i18n export', () => {
  it('matches the TS dictionaries', () => {
    for (const locale of LOCALES) expect(readFileSync(`${IOS_I18N_DIR}${locale}.json`, 'utf8'), locale).toBe(iosI18nJson(DICTS[locale]));
  });
});
