import { readRaw } from './Assets';

// The webview's dictionaries (packages/shared locales, synced as the iOS app's Resources/i18n): the same keys and
// wording on every client. English fills what a locale lacks; `{name}` placeholders take the params.

let table: Record<string, string> = {};
let english: Record<string, string> = {};

function load(locale: string): Record<string, string> {
  const text = readRaw(`i18n/${locale}.json`);
  return text === undefined ? {} : JSON.parse(text) as Record<string, string>;
}

/** Picks the dictionary for a system language tag (zh-Hans-CN → zh-CN, anything else English) */
export function initL10n(language: string): void {
  english = load('en');
  table = language.startsWith('zh') ? load('zh-CN') : english;
}

export function t(key: string, params?: Map<string, string>): string {
  let text: string = table[key] ?? english[key] ?? key;
  params?.forEach((value: string, name: string) => {
    text = text.replace(`{${name}}`, value);
  });
  return text;
}
