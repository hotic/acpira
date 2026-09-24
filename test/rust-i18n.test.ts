import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { en, zhCN } from '../src/shared/i18n';
import { RUST_I18N_DIR, rustI18nJson } from '../scripts/export-rust-i18n';

// The Rust host embeds these files; regenerate with `pnpm exec tsx scripts/export-rust-i18n.ts`
describe('rust i18n export', () => {
  it('matches the TS dictionaries', () => {
    expect(readFileSync(`${RUST_I18N_DIR}en.json`, 'utf8')).toBe(rustI18nJson(en));
    expect(readFileSync(`${RUST_I18N_DIR}zh-CN.json`, 'utf8')).toBe(rustI18nJson(zhCN));
  });
});
