import { zhCN } from './zh-CN';
import { zhTW } from './zh-TW';
import { en } from './en';
import { ja } from './ja';
import { ko } from './ko';
import { es } from './es';
import { de } from './de';
import { fr } from './fr';
import { ru } from './ru';
import { LOCALES, registerLocale, type Locale, type LocaleDict } from './core';

// Every locale at once, for the extension host, the scripts and the tests. The webview imports core.ts instead and
// loads one locale at a time (load.ts)
export * from './core';

// en and zh-CN are complete (`satisfies Record<MsgKey, string>`); the others may lag behind and fall back to en per key
export const DICTS: Record<Locale, LocaleDict> = { 'zh-CN': zhCN, 'zh-TW': zhTW, en, ja, ko, es, de, fr, ru };
for (const l of LOCALES) registerLocale(l, DICTS[l]);

export { zhCN, zhTW, ja, ko, es, de, fr, ru };
