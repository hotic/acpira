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

// Session list, settings, prefs (list agent): the language picked in the settings

/** The dictionaries a language pick offers, as the iOS app's L10n.locales */
export const LOCALES: string[] = ['en', 'zh-CN', 'zh-TW', 'ja', 'ko', 'de', 'fr', 'es', 'ru'];

let current: string = 'en';

/** The dictionary a system language tag selects: its own when the app has it, else English */
export function preferredLocale(language: string): string {
  if (language.startsWith('zh')) {
    return language.includes('Hant') || language.endsWith('TW') || language.endsWith('HK') ? 'zh-TW' : 'zh-CN';
  }
  const base = language.split('-')[0];
  return LOCALES.includes(base) ? base : 'en';
}

/** Switches the table: `auto` follows the system language */
export function setLocale(id: string, systemLanguage: string): void {
  current = id === 'auto' || !LOCALES.includes(id) ? preferredLocale(systemLanguage) : id;
  english = load('en');
  table = current === 'en' ? english : load(current);
}

/** The dictionary in use */
export function currentLocale(): string {
  return current;
}
