import type { SlashCommand } from '@shared/transcript';

// Pure helpers behind the / command completion (Slash.tsx renders them; a lowercase `slash.ts` would clash with it on a case-insensitive disk); kept DOM-free so the host tsconfig can type-check their tests

// A / command token under the caret: the prompt starts with `/` and the caret is still inside that first token.
// `query` is what has been typed after the slash. A `/` anywhere else (paths, `a/b`) is plain text
export interface SlashSpan {
  query: string;
}

export function commandAt(text: string, caret: number): SlashSpan | undefined {
  const m = /^\/(\S*)$/.exec(text.slice(0, caret));
  return m ? { query: m[1]! } : undefined;
}

// Commands matching the typed query: name prefixes first, then names containing it or descriptions with a word starting with it; each tier keeps
// the agent's order. Case-insensitive so `/Comp` still finds `compact`. Descriptions match by word start, not substring, so a path typed at the
// start of a prompt (`/tmp/…`) does not keep hitting the middle of unrelated words
export function matchCommands(commands: readonly SlashCommand[], query: string): SlashCommand[] {
  const q = query.toLowerCase();
  if (!q) return [...commands];
  const prefix = commands.filter(c => c.name.toLowerCase().startsWith(q));
  const wordStart = (s: string) => s.toLowerCase().split(/[^\p{L}\p{N}]+/u).some(w => w.startsWith(q));
  const rest = commands.filter(c => !prefix.includes(c) && (c.name.toLowerCase().includes(q) || wordStart(c.description)));
  return [...prefix, ...rest];
}

// The command the prompt names when it is exactly `/name` (optionally followed by whitespace), for showing its input hint
// while the arguments are still empty. Anything typed after the name means the user is past the hint
export function commandHint(commands: readonly SlashCommand[], text: string): string | undefined {
  const m = /^\/(\S+)\s*$/.exec(text);
  return m ? commands.find(c => c.name === m[1])?.input?.hint : undefined;
}
