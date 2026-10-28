// Which grammar a file or a fenced block gets, without the highlighter: the page only needs these answers, while the shiki
// highlighter and its grammars (`codeSyntax.ts`, about 1.3 MB) live in the highlighting worker alone

// A run of a line in one colour. Only the scheme it was tokenized for has its colour: each scheme costs a whole pass of
// the grammar, so the highlighter tokenizes for the one on screen and a switch asks again
export interface CodeToken { text: string; light?: string; dark?: string }
export type CodeTheme = 'light' | 'dark';

// The grammars the highlighter loads (`codeSyntax.ts` maps each id to its shiki module)
export const LANGUAGES = ['python', 'tsx', 'typescript', 'javascript', 'jsx', 'json', 'jsonc', 'css', 'html', 'shellscript', 'yaml', 'markdown', 'rust', 'go', 'sql'] as const;
export type Language = (typeof LANGUAGES)[number];

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
  if ((LANGUAGES as readonly string[]).includes(name)) return name as Language;
  // Own-property lookups only: a fence named `constructor` must not resolve through Object.prototype
  if (Object.hasOwn(extensions, name)) return extensions[name]!;
  return Object.hasOwn(fenceAliases, name) ? fenceAliases[name]! : null;
}
