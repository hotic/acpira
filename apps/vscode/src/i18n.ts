import { translate, en, type Locale, type MsgKey, type Params } from '@shared/i18n';

// Host-side strings: one process-wide locale, set on activation and whenever acpira.language changes.
// Only user-visible text (toasts, error texts, default titles) goes through here; log lines stay as they are
let current: Locale = 'en';

export function setHostLocale(locale: Locale) { current = locale; }

export function t(key: MsgKey, params?: Params): string {
  return translate(current, key, params);
}

// For strings that may carry a dictionary key (e.g. the synthetic Grok mode descriptions): the translation when known, otherwise the string itself
export function tOr(key: string, params?: Params): string {
  return key in en ? translate(current, key as MsgKey, params) : key;
}

