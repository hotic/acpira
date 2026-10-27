// Writes the TS dictionaries (the single source) as JSON resources for the iOS app: `pnpm exec tsx scripts/export/ios-i18n.ts`.
// test/ios-i18n.test.ts fails when the committed JSON drifts from packages/shared/i18n
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { DICTS, LOCALES } from '@acpira/shared/i18n';
import { rustI18nJson } from './rust-i18n';

export const IOS_I18N_DIR = fileURLToPath(new URL('../../apps/ios/Packages/AcpiraKit/Sources/AcpiraUI/Resources/i18n/', import.meta.url));

// Same bytes as the Rust export, so one formatter covers both
export const iosI18nJson = rustI18nJson;

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  mkdirSync(IOS_I18N_DIR, { recursive: true });
  for (const locale of LOCALES) writeFileSync(`${IOS_I18N_DIR}${locale}.json`, iosI18nJson(DICTS[locale]));
}
