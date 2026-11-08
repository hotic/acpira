// The command and summon pills of a sent prompt (apps/ios Logic/SlashCommands.swift promptMarks, slashCommands.ts
// CommandMark): every `/command` the agent advertises (or the turn recorded) and every `@name` of a summonable persona.
// Offsets are string indices.

/** One marked token: where its sigil sits, how long it runs, and whether it summons a persona */
export class PromptMark {
  start: number = 0;
  length: number = 0;
  summon: boolean = false;
}

/** A run of the prompt as the bubble draws it: plain text or one mark */
export class PromptRun {
  text: string = '';
  /** '', command, summon */
  mark: string = '';
}

function mark(start: number, length: number, summon: boolean): PromptMark {
  const m = new PromptMark();
  m.start = start;
  m.length = length;
  m.summon = summon;
  return m;
}

/**
 * What may sit right before a sigil for it to count (SIGIL_LEAD): whitespace, or CJK text and its full-width
 * punctuation. Latin letters, digits and ASCII punctuation keep `a@b.com`, `a/b` and `https://` plain
 */
function leads(c: string): boolean {
  return /\s/.test(c) || /[ᄀ-ᇿ⺀-⿟　-ヿ㄰-㆏ㇰ-ㇿ㐀-䶿一-鿿가-힯豈-﫿＀-￯]/.test(c);
}

function nameChar(c: string): boolean {
  return /[\p{L}\p{N}]/u.test(c);
}

/** commandMarks: `/name` (or `/$name`) tokens whose name the agent knows, ending at whitespace or the text's end */
export function commandMarks(names: string[], text: string): PromptMark[] {
  if (names.length === 0 || !text.includes('/')) {
    return [];
  }
  const known = new Set<string>(names);
  const marks: PromptMark[] = [];
  let i = 0;
  while (i < text.length) {
    if (text.charAt(i) === '/' && (i === 0 || leads(text.charAt(i - 1)))) {
      let j = i + 1;
      if (j < text.length && text.charAt(j) === '$') {
        j++;
      }
      if (j < text.length && nameChar(text.charAt(j))) {
        while (j < text.length && (nameChar(text.charAt(j)) || '_.:-'.includes(text.charAt(j)))) {
          j++;
        }
        if ((j === text.length || /\s/.test(text.charAt(j))) && known.has(text.slice(i + 1, j))) {
          marks.push(mark(i, j - i, false));
          i = j;
          continue;
        }
      }
    }
    i++;
  }
  return marks;
}

/** summonMarks: `@name` of a persona, longest names first; a match must not run on into more name characters */
export function summonMarks(names: string[], text: string): PromptMark[] {
  if (names.length === 0 || !text.includes('@')) {
    return [];
  }
  const sorted = names.filter((n: string) => n.length > 0).sort((a: string, b: string) => b.length - a.length);
  const tail = (c: string): boolean => nameChar(c) || '_.:/@-'.includes(c);
  const marks: PromptMark[] = [];
  let i = 0;
  while (i < text.length) {
    if (text.charAt(i) === '@' && (i === 0 || leads(text.charAt(i - 1)))) {
      const hit = sorted.find((name: string) => {
        const end = i + 1 + name.length;
        return end <= text.length && text.slice(i + 1, end) === name && (end === text.length || !tail(text.charAt(end)));
      });
      if (hit !== undefined) {
        marks.push(mark(i, hit.length + 1, true));
        i += hit.length + 1;
        continue;
      }
    }
    i++;
  }
  return marks;
}

/** promptMarks: both kinds in text order; a summon owns its whole span, so a `/name` inside one is no command */
export function promptMarks(commands: string[], summons: string[], text: string): PromptMark[] {
  const summoned = summonMarks(summons, text);
  const commanded = commandMarks(commands, text);
  const free = commanded.filter((c: PromptMark) =>
    !summoned.some((s: PromptMark) => c.start > s.start && c.start < s.start + s.length));
  return summoned.concat(free).sort((a: PromptMark, b: PromptMark) => a.start - b.start);
}

/** The prompt cut into plain runs and marked tokens, in order */
export function promptRuns(text: string, commands: string[], summons: string[]): PromptRun[] {
  const out: PromptRun[] = [];
  let at = 0;
  for (const m of promptMarks(commands, summons, text)) {
    if (m.start > at) {
      const plain = new PromptRun();
      plain.text = text.slice(at, m.start);
      out.push(plain);
    }
    const run = new PromptRun();
    run.text = text.slice(m.start, m.start + m.length);
    run.mark = m.summon ? 'summon' : 'command';
    out.push(run);
    at = m.start + m.length;
  }
  if (at < text.length) {
    const rest = new PromptRun();
    rest.text = text.slice(at);
    out.push(rest);
  }
  return out;
}
