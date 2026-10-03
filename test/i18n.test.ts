import { describe, expect, it } from 'vitest';
import { DICTS, en, LANGUAGES, LOCALES, pluralCategory, resolveLocale, translate, zhCN } from '../src/shared/i18n';

const holes = (s: string) => [...new Set([...s.matchAll(/\{(\w+)\}/g)].map(m => m[1]))].sort();
const PLURAL_SUFFIX = /#(one|few|many)$/;

describe('i18n dictionaries', () => {
  it('en and zh-CN have exactly the en key set, no empty strings', () => {
    const keys = Object.keys(en).sort();
    expect(Object.keys(zhCN).sort()).toEqual(keys);
    for (const dict of [zhCN, en] as Record<string, string>[]) for (const k of keys) expect(dict[k], k).toBeTruthy();
  });
  it('every other locale only uses en keys (plus plural variants of them) and no empty strings', () => {
    for (const locale of LOCALES) {
      for (const [k, v] of Object.entries(DICTS[locale])) {
        expect(k.replace(PLURAL_SUFFIX, '') in en, `${locale} ${k}`).toBe(true);
        expect(v, `${locale} ${k}`).toBeTruthy();
      }
    }
  });
  it('every shipped locale is a complete translation today', () => {
    for (const locale of LOCALES) {
      const missing = Object.keys(en).filter(k => !(k in DICTS[locale]) && !k.startsWith('settings.language.'));
      // Strings identical to en are left out of a dictionary on purpose (brand names, units, MCP …)
      expect(missing.length, `${locale}: ${missing.slice(0, 5).join(', ')}`).toBeLessThan(40);
    }
  });
  it('placeholders match en; plural variants use a subset of them', () => {
    for (const locale of LOCALES) {
      for (const [k, v] of Object.entries(DICTS[locale]) as [string, string][]) {
        const base = en[k.replace(PLURAL_SUFFIX, '') as keyof typeof en];
        if (PLURAL_SUFFIX.test(k)) expect(holes(base), `${locale} ${k}`).toEqual(expect.arrayContaining(holes(v)));
        else expect(holes(v), `${locale} ${k}`).toEqual(holes(base));
      }
    }
  });
  it('plural variants only sit on keys whose en string is counted ({n} or {count})', () => {
    for (const locale of LOCALES) {
      for (const k of Object.keys(DICTS[locale]).filter(k => PLURAL_SUFFIX.test(k))) {
        const base = en[k.replace(PLURAL_SUFFIX, '') as keyof typeof en];
        expect(/\{(n|count)\}/.test(base), `${locale} ${k}`).toBe(true);
      }
    }
  });
  it('translate fills params and leaves unknown holes alone', () => {
    expect(translate('zh-CN', 'session.deleted', { title: 'x' })).toBe('已删除「x」');
    expect(translate('en', 'turns.readFiles', { n: 3 })).toBe('Read 3 files');
    expect(translate('en', 'host.doing', { verb: 'Read' })).toBe('Read {target}');
    expect(translate('ja', 'session.deleted', { title: 'x' })).toBe('「x」を削除しました');
  });
  it('a key missing from a locale falls back to en', () => {
    expect(translate('ja', 'settings.tab.mcp')).toBe('MCP');
    expect(translate('de', 'settings.language.ja')).toBe('日本語');
  });
  it('picks the plural variant from the count', () => {
    expect(translate('ru', 'turns.readFiles', { n: 1 })).toBe('Прочитан 1 файл');
    expect(translate('ru', 'turns.readFiles', { n: 3 })).toBe('Прочитано 3 файла');
    expect(translate('ru', 'turns.readFiles', { n: 5 })).toBe('Прочитано 5 файлов');
    expect(translate('ru', 'turns.readFiles', { n: 11 })).toBe('Прочитано 11 файлов');
    expect(translate('ru', 'turns.readFiles', { n: 21 })).toBe('Прочитан 21 файл');
    expect(translate('ru', 'turns.readFiles', { n: 22 })).toBe('Прочитано 22 файла');
    expect(translate('ru', 'host.forkContextTrimmed', { count: '2' })).toContain('2 самых ранних хода');
    expect(translate('es', 'host.images', { n: 1 })).toBe('1 imagen');
    expect(translate('es', 'host.images', { n: 0 })).toBe('0 imágenes');
    expect(translate('fr', 'host.images', { n: 0 })).toBe('0 image');
    expect(translate('de', 'turns.readFiles', { n: 1 })).toBe('1 Datei gelesen');
  });
  it('pluralCategory follows CLDR for counts (mirrored by the Rust crate)', () => {
    const ru = [0, 1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 101, 111, 112].map(n => pluralCategory('ru', n));
    expect(ru).toEqual(['many', 'one', 'few', 'few', 'many', 'many', 'many', 'many', 'one', 'few', 'many', 'one', 'many', 'many']);
    expect([0, 1, 2].map(n => pluralCategory('fr', n))).toEqual(['one', 'one', undefined]);
    expect([0, 1, 2].map(n => pluralCategory('de', n))).toEqual([undefined, 'one', undefined]);
    expect(pluralCategory('ja', 1)).toBeUndefined();
    expect(pluralCategory('ru', 1.5)).toBeUndefined();
  });
  it('resolveLocale: explicit wins, auto follows the host language, unknown → en', () => {
    expect(resolveLocale('en', 'zh-cn')).toBe('en');
    expect(resolveLocale('auto', 'zh-cn')).toBe('zh-CN');
    expect(resolveLocale('auto', 'zh-Hans-CN')).toBe('zh-CN');
    expect(resolveLocale('auto', 'zh-TW')).toBe('zh-TW');
    expect(resolveLocale('auto', 'zh-hk')).toBe('zh-TW');
    expect(resolveLocale('auto', 'zh-Hant-TW')).toBe('zh-TW');
    expect(resolveLocale('auto', 'ja')).toBe('ja');
    expect(resolveLocale('auto', 'ko-KR')).toBe('ko');
    expect(resolveLocale('auto', 'es-419')).toBe('es');
    expect(resolveLocale('auto', 'de-AT')).toBe('de');
    expect(resolveLocale('auto', 'fr_CA')).toBe('fr');
    expect(resolveLocale('auto', 'ru')).toBe('ru');
    expect(resolveLocale('auto', 'pt-br')).toBe('en');
    expect(resolveLocale('auto', 'en-US')).toBe('en');
    expect(resolveLocale(undefined, undefined)).toBe('en');
    expect(LOCALES).toEqual(['zh-CN', 'zh-TW', 'en', 'ja', 'ko', 'es', 'de', 'fr', 'ru']);
  });
  it('every language has a picker label', () => {
    for (const l of LANGUAGES) expect(`settings.language.${l}` in en, l).toBe(true);
  });
});
