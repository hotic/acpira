import { createContext, useContext } from 'react';
import { translate, en, type Locale, type MsgKey, type Params } from '@acpira/shared/i18n';

// Webview strings. Components read `t` from module state (no prop threading through twenty components). A new locale
// from the host re-renders the tree in place, without a remount: a component that memo keeps from rendering (the
// transcript rows) or a cache of translated text (useMemo) reads LocaleContext through useLocale, which reaches it anyway.
// The context also serves the components that need the locale itself (date formatting)
let current: Locale = 'en';

export function setLocale(locale: Locale) { current = locale; }
export function getLocale(): Locale { return current; }

export function t(key: MsgKey, params?: Params): string {
  return translate(current, key, params);
}

// For keys built at runtime (e.g. `notice.method.<agent>:<methodId>`): the translation when the dictionary knows the key, otherwise the fallback text
export function tOr(key: string, fallback: string, params?: Params): string {
  return key in en ? translate(current, key as MsgKey, params) : fallback;
}

export const LocaleContext = createContext<Locale>('en');
export const useLocale = () => useContext(LocaleContext);

// Constructing an Intl.DateTimeFormat resolves locale data every time, and the session list formats a date per row on
// every render: one formatter per locale and option set, kept for the page's lifetime (a handful of shapes in all)
const dateFormats = new Map<string, Intl.DateTimeFormat>();
export function dateFormat(locale: string, options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${locale} ${JSON.stringify(options)}`;
  let format = dateFormats.get(key);
  if (!format) dateFormats.set(key, format = new Intl.DateTimeFormat(locale, options));
  return format;
}
