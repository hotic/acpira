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
  if (!q) return mergeSkillCopies([...commands]);
  const prefix = commands.filter(c => c.name.toLowerCase().startsWith(q));
  const wordStart = (s: string) => s.toLowerCase().split(/[^\p{L}\p{N}]+/u).some(w => w.startsWith(q));
  const rest = commands.filter(c => !prefix.includes(c) && (c.name.toLowerCase().includes(q) || wordStart(c.description) || wordStart(presentCommand(c, locale).description)));
  return mergeSkillCopies([...prefix, ...rest]);
}

// Devin lists a skill found in both `~/.agents/skills` and `~/.claude/skills` twice (`agents:dig`, `claude:dig`). The list keeps
// the first copy in ranking order, so `/claude:` still surfaces the claude copies; the picked name is sent verbatim
const SKILL_SCOPE = /^(?:agents|claude):/;

function mergeSkillCopies(commands: SlashCommand[]): SlashCommand[] {
  const seen = new Set<string>();
  return commands.filter(c => {
    if (!SKILL_SCOPE.test(c.name)) return true;
    const base = c.name.replace(SKILL_SCOPE, '');
    return !seen.has(base) && !!seen.add(base);
  });
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
  // `@` marks a summoned subagent persona (`@name`); absent means a `/name` command
  sigil?: '@';
}

// Every token in the text that names an advertised command, for the composer mirror to paint: `/` at the start or after
// whitespace, the name running to whitespace or the end — the same boundary rules as `commandName`, so `a/b`, `/tmp/x`
// and partial names stay plain. A mid-sentence command gets the same pill as a leading one, like Cursor's composer
export function commandMarks(commands: readonly SlashCommand[], text: string): CommandMark[] {
  if (!commands.length || !text.includes('/')) return [];
  const marks: CommandMark[] = [];
  for (const m of text.matchAll(/(^|\s)\/(\$?[\p{L}\p{N}][\p{L}\p{N}_.:-]*)(?=\s|$)/gu))
    if (commands.some(c => c.name === m[2])) marks.push({ start: m.index + m[1]!.length, name: m[2]! });
  return marks;
}

// Characters that may continue a name, so `@code` inside `@code-review` or `@a/b` never counts as a summon of `code` / `a`
const NAME_TAIL = /[\p{L}\p{N}_.:/@-]/u;

// Every `@name` token naming a persona the ask_agent tool can summon (settings → Subagents), painted like a command. The `@`
// sits at the start or after whitespace, like the mention list's trigger; the name must match exactly and end at whitespace,
// the end or punctuation, so `@审查，` lights up while `@审查员` (a different name) does not. Persona names may contain
// spaces, so the names are tried longest first instead of tokenising the text
export function summonMarks(names: readonly string[], text: string): CommandMark[] {
  if (!names.length || !text.includes('@')) return [];
  const sorted = [...new Set(names)].filter(Boolean).sort((a, b) => b.length - a.length);
  const marks: CommandMark[] = [];
  for (let i = text.indexOf('@'); i >= 0; i = text.indexOf('@', i + 1)) {
    if (i > 0 && !/\s/.test(text[i - 1]!)) continue;
    const name = sorted.find(n => text.startsWith(n, i + 1) && !NAME_TAIL.test(text[i + 1 + n.length] ?? ''));
    if (name) { marks.push({ start: i, name, sigil: '@' }); i += name.length; }
  }
  return marks;
}

// The composer mirror's and the sent message's marks: commands and summons together, in text order
export function promptMarks(commands: readonly SlashCommand[], summons: readonly string[], text: string): CommandMark[] {
  const summoned = summonMarks(summons, text);
  const commanded = commandMarks(commands, text);
  if (!summoned.length) return commanded;
  // A summon owns its whole span; a `/name` inside a persona name (`@a /b` named "a /b") is not painted twice
  const free = commanded.filter(c => !summoned.some(s => c.start > s.start && c.start <= s.start + s.name.length));
  return [...summoned, ...free].sort((a, b) => a.start - b.start);
}

// How far a mark's pill may reach past its text on each side, as a CSS length for `--mark-room` (`.prompt-mark`).
// The pill's background overhangs by margin / padding that cancel out, so the composer mirror keeps every glyph where the
// textarea has it; that overhang must fit in the whitespace beside the mark or the pill touches its neighbour (one space
// is narrower than two full overhangs). Each side's room is the whitespace run there (`--mark-space` per character) minus
// `--mark-clear`, halved when another mark shares it; a text edge or a line break leaves the full `--mark-overhang`.
// The pill keeps the smaller side on both, so a run of marks reads as even pills with even gaps.
// Undefined when neither side is limited.
export function markRoom(text: string, marks: readonly CommandMark[], i: number): string | undefined {
  const m = marks[i]!;
  const end = m.start + m.name.length + 1;
  const prev = marks[i - 1];
  const next = marks[i + 1];
  const sides = [
    side(text.slice(prev ? prev.start + prev.name.length + 1 : 0, m.start), 'before', !!prev),
    side(text.slice(end, next ? next.start : text.length), 'after', !!next),
  ].filter((x): x is string => x !== undefined);
  return sides.length ? `min(var(--mark-overhang), ${sides.join(', ')})` : undefined;
}

// One side's room: the whitespace touching the mark in `gap` (the text up to the neighbouring mark or the text's edge)
function side(gap: string, at: 'before' | 'after', markBeyond: boolean): string | undefined {
  const run = (at === 'before' ? /[^\S\n]*$/ : /^[^\S\n]*/).exec(gap)![0];
  const edge = gap.length === run.length && !markBeyond;
  // The text's edge or a line break beyond the whitespace: nothing to touch on this line
  const broken = at === 'before' ? gap[gap.length - run.length - 1] === '\n' : gap[run.length] === '\n';
  if (edge || broken) return undefined;
  const shared = markBeyond && gap.length === run.length;
  const room = `${run.length} * var(--mark-space) - var(--mark-clear)`;
  return shared ? `calc((${room}) / 2)` : `calc(${room})`;
}
