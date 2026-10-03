// Writes the TS dictionaries (the single source) as JSON for the Rust host: `pnpm exec tsx scripts/export-rust-i18n.ts`.
// test/rust-i18n.test.ts fails when the committed JSON drifts from src/shared/i18n
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { DICTS, LOCALES } from '../src/shared/i18n';

export const RUST_I18N_DIR = fileURLToPath(new URL('../rust/crates/acpira-shared/i18n/', import.meta.url));

export function rustI18nJson(dict: Record<string, string | undefined>): string {
  return `${JSON.stringify(dict, null, 2)}\n`;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  for (const locale of LOCALES) writeFileSync(`${RUST_I18N_DIR}${locale}.json`, rustI18nJson(DICTS[locale]));
}
