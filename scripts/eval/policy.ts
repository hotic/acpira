import { mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { DUTY_BRIEFS } from '@shared/subagents';

// The strategy file the evolve loop tunes (evolve.ts) and the review bench reads (`review/run.ts --policy`): the handoff
// pack's recipe and the reviewer's standing brief. Pure data; a key left out keeps the shipped default. Only what the
// review bench exercises is here: the scout and executor briefs would need Fusion runs to judge, and `readPaths` lists
// what the author's session read, which the bench's lead has none of

export interface Recipe { functionContext: boolean; fullTextLines: number; refs: boolean; refsMax: number; maxBytes: number }
export interface Policy { handoff?: Partial<Recipe>; briefs?: { reviewer?: string } }

// Mirror of Recipe::default (rust/crates/acpira-host/src/handoff.rs)
export const DEFAULT_RECIPE: Recipe = { functionContext: true, fullTextLines: 400, refs: false, refsMax: 60, maxBytes: 60_000 };

const BRIEF_MAX = 2000;
// Recipe::clamped: the pack's cap stays within half the text attachment limit (attachments.rs MAX_TEXT_BYTES)
const LIMITS = { fullTextLines: [0, 5000], refsMax: [0, 500], maxBytes: [2000, 512 * 1024] } as const;

// What the proposer reads: every knob, its range and what it changes
export const SCHEMA = {
  handoff: {
    functionContext: 'boolean: widen each diff hunk to its whole function (git diff -W); false = three-line hunks',
    fullTextLines: `integer ${LIMITS.fullTextLines.join('..')}: changed files up to this many lines go into the pack whole; 0 = never`,
    refs: 'boolean: list where the symbols whose definition lines changed are used elsewhere (path:line and the line)',
    refsMax: `integer ${LIMITS.refsMax.join('..')}: reference lines in all, at most`,
    maxBytes: `integer ${LIMITS.maxBytes.join('..')}: the pack's size cap; over it the pack degrades (whole files first, then function context, then references)`,
  },
  briefs: {
    reviewer: `string, at most ${BRIEF_MAX} characters, English: the standing instructions every summoned reviewer gets after its task`,
  },
};

// The candidate as the bench should run it, or what is wrong with it: unknown keys, wrong types and values out of range
// are refused rather than clamped, so the ledger records what really ran
export function validate(value: unknown): { policy?: Policy; error?: string } {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return { error: 'the policy is not an object' };
  const v = value as Record<string, unknown>;
  for (const k of Object.keys(v)) if (k !== 'handoff' && k !== 'briefs') return { error: `unknown key ${k}` };
  const out: Policy = {};
  if (v.handoff !== undefined) {
    if (!v.handoff || typeof v.handoff !== 'object' || Array.isArray(v.handoff)) return { error: 'handoff is not an object' };
    const h = v.handoff as Record<string, unknown>;
    const recipe: Partial<Recipe> = {};
    for (const [k, x] of Object.entries(h)) {
      if (k === 'functionContext' || k === 'refs') {
        if (typeof x !== 'boolean') return { error: `handoff.${k} is not a boolean` };
        recipe[k] = x;
      } else if (k === 'fullTextLines' || k === 'refsMax' || k === 'maxBytes') {
        const [lo, hi] = LIMITS[k];
        if (typeof x !== 'number' || !Number.isInteger(x) || x < lo || x > hi) return { error: `handoff.${k} is not an integer in ${lo}..${hi}` };
        recipe[k] = x;
      } else {
        return { error: `unknown key handoff.${k}` };
      }
    }
    out.handoff = recipe;
  }
  if (v.briefs !== undefined) {
    if (!v.briefs || typeof v.briefs !== 'object' || Array.isArray(v.briefs)) return { error: 'briefs is not an object' };
    const b = v.briefs as Record<string, unknown>;
    for (const k of Object.keys(b)) if (k !== 'reviewer') return { error: `unknown key briefs.${k}` };
    if (b.reviewer !== undefined) {
      if (typeof b.reviewer !== 'string' || !b.reviewer.trim() || b.reviewer.length > BRIEF_MAX) return { error: `briefs.reviewer is not a text of 1..${BRIEF_MAX} characters` };
      out.briefs = { reviewer: b.reviewer.trim() };
    }
  }
  return { policy: out };
}

// Every knob spelled out, defaults filled in: what two policies are compared by
export function effective(p: Policy): { handoff: Recipe; briefs: { reviewer: string } } {
  return { handoff: { ...DEFAULT_RECIPE, ...p.handoff }, briefs: { reviewer: p.briefs?.reviewer ?? DUTY_BRIEFS.reviewer } };
}

export const samePolicy = (a: Policy, b: Policy) => JSON.stringify(effective(a)) === JSON.stringify(effective(b));

// Text cut into pieces of at most `width` characters at spaces, each piece but the last keeping its trailing space
function pieces(text: string, width: number): string[] {
  const out: string[] = [];
  let line = '';
  for (const word of text.split(' ')) {
    if (line && line.length + word.length + 1 > width) {
      out.push(`${line} `);
      line = word;
    } else {
      line = line ? `${line} ${word}` : word;
    }
  }
  out.push(line);
  return out;
}

const rustString = (s: string) => s.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
const tsString = (s: string) => s.replace(/\\/g, '\\\\').replace(/'/g, "\\'");

// A function replacement takes the text literally (no `$` patterns)
function replaceOnce(text: string, re: RegExp, by: string | (() => string), what: string): string {
  if (!re.test(text)) throw new Error(`defaults patch: ${what} not found`);
  return typeof by === 'string' ? text.replace(re, by) : text.replace(re, by);
}

// The shipped defaults changed to this policy, as a patch against the repository at `repo` (applied by hand once
// approved): Recipe::default, and the reviewer brief in its three places (the Rust constant, its TypeScript mirror and
// the fixture that checks both). Empty when the policy changes nothing
export function defaultsPatch(p: Policy, repo: string): string {
  const e = effective(p);
  const files: Record<string, (t: string) => string> = {};
  const r = e.handoff;
  if (JSON.stringify(r) !== JSON.stringify(DEFAULT_RECIPE)) {
    const big = (n: number) => String(n).replace(/\B(?=(\d{3})+(?!\d))/g, '_');
    files['rust/crates/acpira-host/src/handoff.rs'] = t => replaceOnce(t,
      /Recipe \{ function_context: \w+, full_text_lines: [\d_]+, refs: \w+, refs_max: [\d_]+, read_paths: (\w+), max_bytes: [\d_]+ \}/,
      `Recipe { function_context: ${r.functionContext}, full_text_lines: ${r.fullTextLines}, refs: ${r.refs}, refs_max: ${r.refsMax}, read_paths: $1, max_bytes: ${big(r.maxBytes)} }`,
      'Recipe::default');
  }
  const brief = e.briefs.reviewer;
  if (brief !== DUTY_BRIEFS.reviewer) {
    files['rust/crates/acpira-shared/src/subagents.rs'] = t => {
      const lines = pieces(rustString(brief), 110);
      const body = lines.map((l, i) => (i === 0 ? `"${l}` : l) + (i === lines.length - 1 ? '";' : '\\')).join('\n');
      return replaceOnce(t, /pub const REVIEWER_BRIEF: &str = "(?:[^"\\]|\\.)*";/s, () => `pub const REVIEWER_BRIEF: &str = ${body}`, 'REVIEWER_BRIEF');
    };
    files['packages/shared/subagents.ts'] = t => {
      const lines = pieces(tsString(brief), 110);
      const body = lines.map((l, i) => (i === 0 ? `'${l}'` : `    + '${l}'`)).join('\n');
      return replaceOnce(t, /  reviewer: '(?:[^'\\]|\\.)*'(?:\n    \+ '(?:[^'\\]|\\.)*')*,/, () => `  reviewer: ${body},`, 'DUTY_BRIEFS.reviewer');
    };
    files['test/fixtures/duty-presets.json'] = t => {
      const j = JSON.parse(t) as { reviewer: { brief: string } };
      j.reviewer.brief = brief;
      return JSON.stringify(j, null, 2) + '\n';
    };
  }
  if (!Object.keys(files).length) return '';
  // A copy of each file before and after, diffed with git's own paths
  const tmp = mkdtempSync(join(tmpdir(), 'acpira-defaults-'));
  try {
    for (const [path, edit] of Object.entries(files)) {
      const before = readFileSync(join(repo, path), 'utf8');
      for (const [side, text] of [['a', before], ['b', edit(before)]] as const) {
        mkdirSync(dirname(join(tmp, side, path)), { recursive: true });
        writeFileSync(join(tmp, side, path), text);
      }
    }
    const d = spawnSync('git', ['diff', '--no-index', '--no-color', 'a', 'b'], { cwd: tmp, encoding: 'utf8' });
    return (d.stdout ?? '').replace(/^(---|\+\+\+) (a|b)\/(a|b)\//gm, '$1 $2/').replace(/^diff --git a\/a\/(\S+) b\/b\/\S+$/gm, 'diff --git a/$1 b/$1');
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}
