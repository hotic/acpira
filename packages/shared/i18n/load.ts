import { hasLocale, registerLocale, type Locale, type LocaleDict } from './core';

// The webview's dictionaries besides en, each its own chunk in the installed bundle: read from the package when the
// page first shows that language (nothing is downloaded). Settles once the locale is registered; a failed read leaves
// the page on en and is retried by the next call
const LOADERS: Record<Exclude<Locale, 'en'>, () => Promise<LocaleDict>> = {
  'zh-CN': () => import('./zh-CN').then(m => m.zhCN),
  'zh-TW': () => import('./zh-TW').then(m => m.zhTW),
  ja: () => import('./ja').then(m => m.ja),
  ko: () => import('./ko').then(m => m.ko),
  es: () => import('./es').then(m => m.es),
  de: () => import('./de').then(m => m.de),
  fr: () => import('./fr').then(m => m.fr),
  ru: () => import('./ru').then(m => m.ru),
};

const pending = new Map<Locale, Promise<void>>();
export function loadLocale(locale: Locale): Promise<void> {
  if (hasLocale(locale) || locale === 'en') return Promise.resolve();
  let p = pending.get(locale);
  if (!p) {
    p = LOADERS[locale]().then(d => registerLocale(locale, d), () => {});
    p.finally(() => pending.delete(locale));
    pending.set(locale, p);
  }
  return p;
}
