import { t } from '../i18n';

// The command row names the program; the full command lives in the output card. DOM-free so the host tsconfig can type-check its test.

// `NAME=value` prefixes (unquoted, or one simple quoted value) only configure the program that follows
const ASSIGNMENT = /^[A-Za-z_]\w*=(?:"[^"]*"|'[^']*'|[^\s"'`$;&|]*)(?:\s+|$)/;
// A leading `cd dir &&` / `cd dir;` only sets the scene
const CD = /^cd\s+(?:"[^"]*"|'[^']*'|[^\s"'`$;&|]+)\s*(?:&&|;)\s*/;
// `bash -lc '…'` / `sh -c "…"` around the whole command: the quoted script is what runs
const SHELL_WRAPPER = /^(?:\/(?:usr\/)?bin\/)?(?:ba|z|da)?sh\s+-l?c\s+(["'])([\s\S]*)\1$/;
// Subcommand-like words (`run`, `typecheck`, `vitest@latest`); flags, paths, redirects and quotes end the summary
const WORD = /^[A-Za-z][\w.:@+-]*$/;
const MAX_WORDS = 3;

/** Program name plus up to two plain subcommand words, with ` …` when the command holds more than that. */
export function commandSummary(command: string): string {
  let text = command.trim();
  const wrapped = SHELL_WRAPPER.exec(text);
  if (wrapped && !wrapped[2]!.includes(wrapped[1]!)) text = wrapped[2]!.trim();
  const lines = text.split('\n');
  const head = lines[0]!.trim();
  let first = head;
  for (let match = ASSIGNMENT.exec(first) ?? CD.exec(first); match; match = ASSIGNMENT.exec(first) ?? CD.exec(first)) {
    first = first.slice(match[0].length).trimStart();
  }
  // Nothing but assignments: the line itself is the command
  if (!first) first = head;
  const words = first.split(/\s+/).filter(Boolean);
  if (!words.length) return '';
  // An absolute program path reads by its name (`/usr/bin/python3` → `python3`)
  const program = words[0]!.replace(/;$/, '');
  const kept = [program.startsWith('/') ? program.split('/').filter(Boolean).pop() ?? program : program];
  let used = 1;
  let ended = words[0]!.endsWith(';');
  for (const word of words.slice(1)) {
    if (ended || kept.length === MAX_WORDS) break;
    const bare = word.replace(/;$/, '');
    if (!WORD.test(bare)) break;
    kept.push(bare);
    used++;
    ended = bare !== word;
  }
  const more = used < words.length || lines.slice(1).some(line => line.trim());
  return more ? `${kept.join(' ')} …` : kept.join(' ');
}

/**
 * A command's run time for the row's trailing slot. Finished commands keep a tenth of a second below ten seconds;
 * `whole` is the live clock, which ticks in whole seconds.
 */
export function commandDuration(ms: number, whole = false): string {
  const clamped = Math.max(0, ms);
  if (!whole && clamped < 10_000) return t('turns.elapsed.s', { s: clamped < 100 ? '<0.1' : (clamped / 1000).toFixed(1) });
  const seconds = whole ? Math.floor(clamped / 1000) : Math.round(clamped / 1000);
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return m ? (s ? t('turns.elapsed.ms', { m, s }) : t('turns.elapsed.m', { m })) : t('turns.elapsed.s', { s });
}
