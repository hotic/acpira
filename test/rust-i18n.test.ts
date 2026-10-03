import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { DICTS, LOCALES } from '../src/shared/i18n';
import { RUST_I18N_DIR, rustI18nJson } from '../scripts/export-rust-i18n';

// The Rust host embeds these files; regenerate with `pnpm exec tsx scripts/export-rust-i18n.ts`
describe('rust i18n export', () => {
  it('matches the TS dictionaries', () => {
    for (const locale of LOCALES) expect(readFileSync(`${RUST_I18N_DIR}${locale}.json`, 'utf8'), locale).toBe(rustI18nJson(DICTS[locale]));
  });
});
