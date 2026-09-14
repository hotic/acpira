import { presentCommand } from '@shared/commandPresentation';
import type { Locale } from '@shared/i18n';
import type { SlashCommand } from '@shared/transcript';

// Pure helpers behind the / command completion (Slash.tsx renders them; a lowercase `slash.ts` would clash with it on a case-insensitive disk); kept DOM-free so the host tsconfig can type-check their tests

// A / command token under the caret: the `/` sits at the start or after whitespace and the query runs up to the caret without
// whitespace (the same shape as the `@` mention). `query` is what has been typed after the slash, `start` the index of the `/`.
// A `/` inside a word (`a/b`, `https://`) is plain text
export interface SlashSpan {
  start: number;
  query: string;
}

export function commandAt(text: string, caret: number): SlashSpan | undefined {
  const m = /(^|\s)\/(\S*)$/.exec(text.slice(0, caret));
  return m ? { start: caret - m[2]!.length - 1, query: m[2]! } : undefined;
}

// The prompt after picking `name` for the span under the caret: the token becomes `/name ` (whatever follows the caret inside
// it goes too) and the text after it is kept as the arguments
export function completeCommand(text: string, span: SlashSpan, caret: number, name: string): { text: string; caret: number } {
  const head = `${text.slice(0, span.start)}/${name} `;
  return { text: head + text.slice(caret).replace(/^\S*/, '').trimStart(), caret: head.length };
}

// Commands matching the typed query: name prefixes first, then names containing it or descriptions with a word starting with it; each tier keeps
// the agent's order. Case-insensitive so `/Comp` still finds `compact`. Descriptions match by word start, not substring, so a path typed at the
// start of a prompt (`/tmp/…`) does not keep hitting the middle of unrelated words
export function matchCommands(commands: readonly SlashCommand[], query: string, locale: Locale = 'en'): SlashCommand[] {
  const q = query.toLowerCase();
  if (!q) return [...commands];
  const prefix = commands.filter(c => c.name.toLowerCase().startsWith(q));
  const wordStart = (s: string) => s.toLowerCase().split(/[^\p{L}\p{N}]+/u).some(w => w.startsWith(q));
  const rest = commands.filter(c => !prefix.includes(c) && (c.name.toLowerCase().includes(q) || wordStart(c.description) || wordStart(presentCommand(c, locale).description)));
  return [...prefix, ...rest];
}

// The command the prompt names when its last token is `/name` (optionally followed by whitespace), wherever that token sits,
// for showing its input hint while the arguments are still empty. Anything typed after the name means the user is past the hint
export function commandHint(commands: readonly SlashCommand[], text: string, locale: Locale = 'en'): string | undefined {
  const m = /(^|\s)\/(\S+)\s*$/.exec(text);
  const command = m ? commands.find(c => c.name === m[2]) : undefined;
  return command ? presentCommand(command, locale).input?.hint : undefined;
}

export interface CommandMark {
  start: number;
  name: string;
}

// Every token in the text that names an advertised command, for the composer mirror to paint: `/` at the start or after
// whitespace, the name running to whitespace or the end — the same boundary rules as `commandName`, so `a/b`, `/tmp/x`
// and partial names stay plain. A mid-sentence command gets the same pill as a leading one, like Cursor's composer
export function commandMarks(commands: readonly SlashCommand[], text: string): CommandMark[] {
  if (!commands.length || !text.includes('/')) return [];
  const marks: CommandMark[] = [];
  for (const m of text.matchAll(/(^|\s)\/([\p{L}\p{N}][\p{L}\p{N}_.:-]*)(?=\s|$)/gu))
    if (commands.some(c => c.name === m[2])) marks.push({ start: m.index + m[1]!.length, name: m[2]! });
  return marks;
}
