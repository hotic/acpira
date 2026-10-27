// Writes the iOS app's syntax highlighter: `pnpm exec tsx scripts/export/ios-shiki.ts`.
// shiki's core and its JavaScript regex engine become one script JavaScriptCore evaluates (`shiki.js`), and the grammars
// and themes the webview loads (packages/ui/chat/codeSyntax.ts) become JSON beside it, read when a language first shows.
// The engine options follow the webview's JavaScriptCore branch (ES2018 regexes, which JSC compiles at once), so a line
// gets the same tokens and colours on the phone as in the desktop's webview.
import { build } from 'esbuild';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../..', import.meta.url));
export const IOS_SHIKI_DIR = join(root, 'apps/ios/Packages/AcpiraKit/Sources/AcpiraUI/Resources/shiki');
// The webview's grammar choice (codeLanguage.ts), read from its source: the root workspace does not depend on
// @acpira/ui. The list names the grammars to ship; the maps become `languages.json`, which the app reads instead of a
// copy of its own
const LANGUAGE_SOURCE = readFileSync(join(root, 'packages/ui/chat/codeLanguage.ts'), 'utf8');
const LANGUAGES = [...LANGUAGE_SOURCE.match(/export const LANGUAGES = \[([^\]]+)\]/)![1]!.matchAll(/'([a-z]+)'/g)].map(m => m[1]!);
/** A `const name: Record<string, Language> = { key: 'grammar', … }` table of codeLanguage.ts */
function languageTable(name: string): Record<string, string> {
  const body = new RegExp(`const ${name}: Record<string, Language> = \\{([^}]+)\\}`).exec(LANGUAGE_SOURCE)![1]!;
  return Object.fromEntries([...body.matchAll(/(\w+): '([a-z]+)'/g)].map(m => [m[1]!, m[2]!]));
}
const THEMES = ['dark-plus', 'light-plus'] as const;

// The script's API, on `globalThis.acpiraShiki`: everything synchronous, JSON in and out, so Swift needs no promise
// handling. Grammars and themes arrive as JSON text from Swift; `tokenize` answers one line array per source line, each
// token `[text, colour]` in the theme's own colours (the merge into runs happens in Swift, as codeSyntax.ts does it)
const ENTRY = `
import { createHighlighterCoreSync } from 'shiki/core';
import { createJavaScriptRegexEngine, defaultJavaScriptRegexConstructor } from 'shiki/engine/javascript';

const regexConstructor = pattern => {
  try { return defaultJavaScriptRegexConstructor(pattern, { target: 'ES2018' }); }
  catch { return defaultJavaScriptRegexConstructor(pattern); }
};
let highlighter;
const loaded = new Set();
globalThis.acpiraShiki = {
  init(themesJSON) {
    highlighter = createHighlighterCoreSync({ themes: JSON.parse(themesJSON), langs: [], engine: createJavaScriptRegexEngine({ regexConstructor }) });
  },
  has(lang) { return loaded.has(lang); },
  load(lang, grammarsJSON) {
    highlighter.loadLanguageSync(JSON.parse(grammarsJSON));
    loaded.add(lang);
  },
  tokenize(code, lang, theme) {
    const lines = highlighter.codeToTokensBase(code, { lang, theme, tokenizeMaxLineLength: 20000 });
    return JSON.stringify(lines.map(line => line.map(t => [t.content, t.color ?? ''])));
  },
};
`;

/** Every file the app bundles under `Resources/shiki`, by its path there */
export async function iosShikiFiles(): Promise<Map<string, string>> {
  const files = new Map<string, string>();
  const bundle = await build({
    stdin: { contents: ENTRY, resolveDir: root, loader: 'js' },
    bundle: true, format: 'iife', platform: 'neutral', target: 'es2020', minify: true, write: false,
    // JavaScriptCore has no module loader and no `process`; the neutral platform still wants a main field order
    mainFields: ['module', 'main'],
  });
  files.set('shiki.js', bundle.outputFiles[0]!.text);
  files.set('languages.json', JSON.stringify({ languages: LANGUAGES, extensions: languageTable('extensions'), fenceAliases: languageTable('fenceAliases') }));
  for (const lang of LANGUAGES) {
    // A language module's default export: its grammar and the ones it embeds, in load order
    const mod = await import(`shiki/langs/${lang}.mjs`) as { default: unknown[] };
    files.set(`langs/${lang}.json`, JSON.stringify(mod.default));
  }
  for (const theme of THEMES) {
    const mod = await import(`shiki/themes/${theme}.mjs`) as { default: unknown };
    files.set(`themes/${theme}.json`, JSON.stringify(mod.default));
  }
  return files;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const files = await iosShikiFiles();
  rmSync(IOS_SHIKI_DIR, { recursive: true, force: true });
  for (const [path, text] of files) {
    mkdirSync(dirname(join(IOS_SHIKI_DIR, path)), { recursive: true });
    writeFileSync(join(IOS_SHIKI_DIR, path), text);
  }
  console.log(IOS_SHIKI_DIR);
}
