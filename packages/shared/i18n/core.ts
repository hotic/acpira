import { en } from './en';
import type { LocaleDict, MsgKey, PluralCategory } from './keys';

export type { LocaleDict, MsgKey, PluralCategory } from './keys';

// Locales the UI ships; `auto` (the acpira.language setting's default) follows the host's display language.
// The order is the order of the language picker; rust/crates/acpira-shared/src/i18n.rs mirrors it
export type Locale = 'zh-CN' | 'zh-TW' | 'en' | 'ja' | 'ko' | 'es' | 'de' | 'fr' | 'ru';
export type Language = 'auto' | Locale;
export const LOCALES: Locale[] = ['zh-CN', 'zh-TW', 'en', 'ja', 'ko', 'es', 'de', 'fr', 'ru'];
export const LANGUAGES: Language[] = ['auto', ...LOCALES];

export type Params = Record<string, string | number>;

// The dictionaries this page holds: en always (the source, and every missing key's fallback). index.ts registers every
// locale up front; the webview imports this module only and reads the one it shows through loadLocale (load.ts), so
// the other eight stay out of its main bundle. Translating into a locale not loaded yet gives en
const dicts: Partial<Record<Locale, LocaleDict>> = { en };
export function registerLocale(locale: Locale, dict: LocaleDict) { dicts[locale] = dict; }
export function hasLocale(locale: Locale): boolean { return dicts[locale] !== undefined; }

// CLDR cardinal rules for the shipped locales, integers only (counts); mirrored by `plural_category` in the Rust crate.
// undefined means the plain key ("other")
export function pluralCategory(locale: Locale, n: number): PluralCategory | undefined {
  if (!Number.isInteger(n) || n < 0) return undefined;
  switch (locale) {
    case 'ru': {
      const d = n % 10, h = n % 100;
      if (d === 1 && h !== 11) return 'one';
      return d >= 2 && d <= 4 && (h < 12 || h > 14) ? 'few' : 'many';
    }
    case 'fr': return n < 2 ? 'one' : undefined;
    case 'en': case 'es': case 'de': return n === 1 ? 'one' : undefined;
    default: return undefined;
  }
}

function countOf(params: Params): number | undefined {
  const v = params.n ?? params.count;
  const n = typeof v === 'string' && /^\d+$/.test(v) ? Number(v) : v;
  return typeof n === 'number' ? n : undefined;
}

// Looks the key up in the locale (its plural variant first when the params carry a count), falls back to en
// (the source dictionary), then to the key itself; {name} placeholders are filled from params
export function translate(locale: Locale, key: MsgKey, params?: Params): string {
  const dict: LocaleDict = dicts[locale] ?? en;
  const n = params && countOf(params);
  const category = n === undefined ? undefined : pluralCategory(locale, n);
  const s = (category ? dict[`${key}#${category}`] : undefined) ?? dict[key] ?? en[key] ?? key;
  return params ? s.replace(/\{(\w+)\}/g, (m, k: string) => (k in params ? String(params[k]) : m)) : s;
}

// Maps the setting plus the host's display language (vscode.env.language / navigator.language / a JVM language tag,
// e.g. "zh-cn", "zh-Hant-TW", "en-US", "ja") onto a shipped locale: Chinese splits by script / region, others by language
export function resolveLocale(language: Language | undefined, hostLanguage: string | undefined): Locale {
  if (language && language !== 'auto') return language;
  const [lang = '', ...rest] = (hostLanguage ?? '').toLowerCase().replace(/_/g, '-').split('-');
  if (lang === 'zh') return rest.some(s => s === 'hant' || s === 'tw' || s === 'hk' || s === 'mo') ? 'zh-TW' : 'zh-CN';
  return LOCALES.find(l => l === lang) ?? 'en';
}

export function isLocale(v: unknown): v is Locale {
  return typeof v === 'string' && (LOCALES as string[]).includes(v);
}

export function isLanguage(v: unknown): v is Language {
  return typeof v === 'string' && (LANGUAGES as string[]).includes(v);
}

export { en };
