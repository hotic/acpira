// Weighted markdown tells for peeking at text attachments: pasted docs are usually markdown, pasted code and logs are not.
// A fence or a table separator alone is decisive; the weaker marks need company — a `#` line is every shell comment too.
const SIGNALS: [RegExp, number][] = [
  // fenced code block
  [/^`{3,}|^~{3,}/m, 3],
  // GFM table: a row with pipes over a |---|---| separator line
  [/\|[^\n]*\n[ \t]*\|? *:?-{3,}:? *(?:\| *:?-{3,}:? *)+\|?[ \t]*$/m, 3],
  // [text](url) and ![alt](url)
  [/!?\[[^\][\n]+\]\([^()\s]+\)/, 2],
  // - [ ] task checkbox
  [/^\s*[-*+][ \t]+\[[ xX]\][ \t]+\S/m, 2],
  // # heading — also the shape of a shell / python comment
  [/^#{1,6}[ \t]+\S/m, 1],
  // > quote
  [/^[ \t]*>[ \t]*\S/m, 1],
  // - / * / 1. list item
  [/^\s*(?:[-*+]|\d{1,3}[.)])[ \t]+\S/m, 1],
  // --- / *** / ___ rule; a first-line --- is frontmatter or a plain divider, not evidence
  [/\n {0,3}(?:-{3,}|\*{3,}|_{3,})[ \t]*$/m, 1],
  // `inline code`
  [/`[^`\n]+`/, 1],
];

const BOLD = /\*\*[^\s*][^*\n]*\*\*/g;
const LIST_ITEM = /^\s*(?:[-*+]|\d{1,3}[.)])[ \t]+\S/gm;

export function looksLikeMarkdown(text: string): boolean {
  let score = 0;
  for (const [signal, weight] of SIGNALS) if (signal.test(text)) score += weight;
  // Emphasis counts only in pairs — a lone `**ptr` dereference isn't markdown — and a pair of them is a strong tell
  if ((text.match(BOLD)?.length ?? 0) >= 2) score += 2;
  // Three or more list lines make a real list, not a stray "- " inside prose
  if ((text.match(LIST_ITEM)?.length ?? 0) >= 3) score += 1;
  return score >= 3;
}
