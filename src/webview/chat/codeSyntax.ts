import { createHighlighterCore } from 'shiki/core';
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript';

export interface CodeToken { text: string; light?: string; dark?: string }

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
};
export type Language = keyof typeof grammars;
const extensions: Record<string, Language> = {
  py: 'python', pyw: 'python', tsx: 'tsx', ts: 'typescript', mts: 'typescript', cts: 'typescript',
  js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'jsx', json: 'json', jsonc: 'jsonc',
  css: 'css', html: 'html', htm: 'html', sh: 'shellscript', bash: 'shellscript', zsh: 'shellscript',
  yaml: 'yaml', yml: 'yaml', md: 'markdown', rs: 'rust', go: 'go', sql: 'sql',
};

// No language is guessed from arbitrary output or extensionless filenames.
export function codeLanguage(path: string): Language | null {
  const basename = path.split(/[\\/]/).at(-1) ?? '';
  const suffix = /[^.]\.([^.]+)$/.exec(basename)?.[1]?.toLowerCase();
  return suffix ? extensions[suffix] ?? null : null;
}

// Fence names that differ from both the grammar ids and the file extensions above
const fenceAliases: Record<string, Language> = {
  shell: 'shellscript', console: 'shellscript', terminal: 'shellscript',
  node: 'javascript', ecmascript: 'javascript', golang: 'go', postgres: 'sql', postgresql: 'sql',
};

// A fenced block is coloured only when its info string names a known grammar; an unlabelled
// or `text` fence (commit messages, logs, prose) stays plain instead of being guessed at.
export function fenceLanguage(tag: string | undefined): Language | null {
  const name = tag?.trim().toLowerCase();
  if (!name) return null;
  if (Object.hasOwn(grammars, name)) return name as Language;
  // Own-property lookups only: a fence named `constructor` must not resolve through Object.prototype
  if (Object.hasOwn(extensions, name)) return extensions[name]!;
  return Object.hasOwn(fenceAliases, name) ? fenceAliases[name]! : null;
}

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

