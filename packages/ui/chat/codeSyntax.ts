import { createHighlighterCore } from 'shiki/core';
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript';
import type { CodeToken, Language } from './codeLanguage';

// The highlighter runs in the highlighting worker only (`codeDiff.worker.ts`); the page asks it through `diffHighlight.ts`
// and takes its grammar answers from `codeLanguage.ts`, which these re-exports keep reachable from here
export { codeLanguage, fenceLanguage, type CodeToken, type Language } from './codeLanguage';

const grammars = {
  python: () => import('shiki/langs/python.mjs'),
  tsx: () => import('shiki/langs/tsx.mjs'),
  typescript: () => import('shiki/langs/typescript.mjs'),
  javascript: () => import('shiki/langs/javascript.mjs'),
  jsx: () => import('shiki/langs/jsx.mjs'),
  json: () => import('shiki/langs/json.mjs'),
  jsonc: () => import('shiki/langs/jsonc.mjs'),
  css: () => import('shiki/langs/css.mjs'),
  html: () => import('shiki/langs/html.mjs'),
  shellscript: () => import('shiki/langs/shellscript.mjs'),
  yaml: () => import('shiki/langs/yaml.mjs'),
  markdown: () => import('shiki/langs/markdown.mjs'),
  rust: () => import('shiki/langs/rust.mjs'),
  go: () => import('shiki/langs/go.mjs'),
  sql: () => import('shiki/langs/sql.mjs'),
} satisfies Record<Language, () => Promise<unknown>>;

const getHighlighter = () => highlighter ??= createHighlighterCore({
  themes: [import('shiki/themes/dark-plus.mjs'), import('shiki/themes/light-plus.mjs')],
  langs: [],
  engine: createJavaScriptRegexEngine(),
});
let highlighter: ReturnType<typeof createHighlighterCore> | undefined;
const loading = new Map<Language, Promise<void>>();
function sourceLines(text: string): string[] {
  if (!text) return [];
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  if (lines.at(-1) === '') lines.pop();
  return lines;
}

// Tokenize each complete side before diffing so multiline strings and comments
// keep their grammar state across line boundaries and omitted context.
export async function tokenizeCode(text: string, language: Language | null): Promise<CodeToken[][]> {
  if (!language) return sourceLines(text).map(line => [{ text: line }]);
  const h = await getHighlighter();
  let task = loading.get(language);
  if (!task) { task = h.loadLanguage(grammars[language]()); loading.set(language, task); }
  await task;
  return h.codeToTokensWithThemes(text.replace(/\r\n/g, '\n'), { lang: language, themes: { light: 'light-plus', dark: 'dark-plus' } })
    .map(line => line.map(token => ({ text: token.content, light: token.variants.light?.color, dark: token.variants.dark?.color })));
}

