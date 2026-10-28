import { createHighlighterCore, type GrammarState, type ThemedToken } from 'shiki/core';
import { createJavaScriptRegexEngine, defaultJavaScriptRegexConstructor } from 'shiki/engine/javascript';
import type { CodeTheme, CodeToken, Language } from './codeLanguage';

// The highlighter runs in the highlighting worker only (`codeDiff.worker.ts`); the page asks it through `diffHighlight.ts`
// and takes its grammar answers from `codeLanguage.ts`, which these re-exports keep reachable from here
export { codeLanguage, fenceLanguage, type CodeTheme, type CodeToken, type Language } from './codeLanguage';

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

// JavaScriptCore (WKWebView, WebKitGTK) compiles the `v`-flag regexes of the default target slowly: every grammar rule
// cost a long pause on its first use, and a first 20-line TypeScript file took ~0.9 s to colour. Its `u`-flag (ES2018)
// regexes compile at once and give the same tokens. V8 is the other way round (ES2018 regexes run up to 2.6x slower
// there), so Chromium hosts keep the default; a pattern ES2018 cannot express keeps it too
const agent = globalThis.navigator?.userAgent ?? '';
const javaScriptCore = /AppleWebKit/.test(agent) && !/Chrom|Edg/.test(agent);
const regexConstructor = (pattern: string) => {
  try {
    return defaultJavaScriptRegexConstructor(pattern, { target: 'ES2018' });
  } catch {
    return defaultJavaScriptRegexConstructor(pattern);
  }
};
const getHighlighter = () => highlighter ??= createHighlighterCore({
  themes: [import('shiki/themes/dark-plus.mjs'), import('shiki/themes/light-plus.mjs')],
  langs: [],
  engine: createJavaScriptRegexEngine(javaScriptCore ? { regexConstructor } : {}),
});
let highlighter: ReturnType<typeof createHighlighterCore> | undefined;
const loading = new Map<Language, Promise<void>>();
function sourceLines(text: string): string[] {
  if (!text) return [];
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  if (lines.at(-1) === '') lines.pop();
  return lines;
}

// The default foreground of light-plus and dark-plus matches the shell's own --fg-1, so a token in it renders as a plain
// text node, without a span or an inline style
const DEFAULT_COLOR: Record<CodeTheme, string> = { light: '#000000', dark: '#D4D4D4' };

// Merge adjacent tokens on a line that share a colour (absorbing pure-whitespace tokens into their neighbour, since
// whitespace has no ink) so each line mounts far fewer DOM nodes and text runs.
function mergeTokens(line: ThemedToken[], theme: CodeTheme): CodeToken[] {
  const out: CodeToken[] = [];
  for (const { content: text, color: raw } of line) {
    if (!text) continue;
    const prev = out[out.length - 1];
    if (prev && !/\S/.test(text)) {
      prev.text += text;
      continue;
    }
    const color = raw && raw.toUpperCase() !== DEFAULT_COLOR[theme] ? raw : undefined;
    if (prev && prev[theme] === color) prev.text += text;
    else out.push(color ? { text, [theme]: color } : { text });
  }
  return out;
}

const THEMES: Record<CodeTheme, string> = { light: 'light-plus', dark: 'dark-plus' };
// A long source holds the worker for seconds (a 90 KB TSX file takes over a second), so with `pause` it is tokenized in
// slices of about SLICE_MS, the grammar state carried from one slice to the next (the same tokens as one pass, and a slice
// costs next to nothing beyond its lines). Slices are measured in characters, which a line's cost follows where a count of
// lines does not (a blank line costs nothing), and grow at most twofold from one to the next: sized by lines, a run of
// blank lines sized the slice after it to the rest of a dense file, which held the worker for seconds. A blank line costs
// nothing and a line past MAX_LINE is skipped, so neither takes part in a slice's rate: counted, a 50 KB first line grew
// the next slice to 100 KB of dense code (half a second in V8), and a run of blank lines doubled the budget slice after
// slice. `pause` is
// awaited after every slice, the last one too (a diff's many short hunks are one request): the worker takes its queued
// messages in there once its run has gone a turn's time, and it throws to end the run. `progress` sees the lines
// tokenized so far after each slice but the last
const SLICE_MS = 8;
const FIRST_SLICE = 512;
// Lines this long (minified code, one-line JSON) stay plain, as in VS Code: shiki stops a line at its time limit and gives
// the rest the last token's colour, after holding the worker for a second, and the row filled with spans
const MAX_LINE = 20_000;

// Tokenize each complete side before diffing so multiline strings and comments
// keep their grammar state across line boundaries and omitted context. `theme`: the scheme whose colours the tokens get
export async function tokenizeCode(text: string, language: Language | null, theme: CodeTheme, pause?: () => Promise<void>,
  progress?: (lines: readonly CodeToken[][]) => void): Promise<CodeToken[][]> {
  if (!language) return sourceLines(text).map(line => [{ text: line }]);
  const h = await getHighlighter();
  let task = loading.get(language);
  if (!task) { task = h.loadLanguage(grammars[language]()); loading.set(language, task); }
  await task;
  const source = text.replace(/\r\n/g, '\n');
  const lines = pause ? source.split('\n') : [source];
  const out: CodeToken[][] = [];
  let state: GrammarState | undefined;
  for (let at = 0, budget = FIRST_SLICE; at < lines.length;) {
    const started = performance.now();
    // The slice: whole lines up to `budget` characters, at least one; `tokenized` leaves out blank lines and lines past
    // MAX_LINE
    let end = at;
    let chars = 0;
    let tokenized = 0;
    while (end < lines.length && (end === at || chars < budget)) {
      const size = lines[end++]!.length + 1;
      chars += size;
      if (size <= MAX_LINE && /\S/.test(lines[end - 1]!)) tokenized += size;
    }
    const tokens = h.codeToTokensBase(lines.slice(at, end).join('\n'), { lang: language, theme: THEMES[theme], grammarState: state, tokenizeMaxLineLength: MAX_LINE });
    for (const line of tokens) out.push(mergeTokens(line, theme));
    at = end;
    if (!pause) break;
    if (at < lines.length) {
      state = h.getLastGrammarState(tokens);
      // A slice of blank or skipped lines alone says nothing of the rate, and keeps the budget
      if (tokenized) budget = Math.max(1, Math.min(budget * 2, Math.round(tokenized * SLICE_MS / Math.max(1, performance.now() - started))));
      progress?.(out);
    }
    await pause();
  }
  return out;
}

