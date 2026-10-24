import type { en } from './en';

// The key set is whatever en defines; a key missing from another locale is a type error in that locale's file
export type MsgKey = keyof typeof en;

// CLDR plural categories a dictionary may add as `<key>#<category>` variants of a counted string (param `n` or `count`);
// the plain key stays the fallback form. Only the categories of the shipped locales are listed (see pluralCategory)
export type PluralCategory = 'one' | 'few' | 'many';

// A translated dictionary besides en and zh-CN: any subset of the en keys (a missing key falls back to en) plus plural variants
export type LocaleDict = Partial<Record<MsgKey | `${MsgKey}#${PluralCategory}`, string>>;
