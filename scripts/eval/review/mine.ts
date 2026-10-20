import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

// Review bench, step 1 (docs/dev/evals.md): candidate targets from this repository's own history. Every small fix
// commit is blamed back, line by line, to the commit that introduced what it fixed; that commit's change is what a
// reviewer gets, and the fix is the answer. Spends nothing; the list is curated by hand into targets.
//
//   pnpm exec tsx scripts/eval/review/mine.ts [--repo DIR] [--out FILE] [--max-fix-lines 40] [--max-intro-lines 900]

const ROOT = resolve(import.meta.dirname, '../../..');

function arg(name: string, fallback: string): string {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1]! : fallback;
}

const repo = resolve(arg('repo', ROOT));
const out = resolve(arg('out', join(ROOT, 'output/review-bench/candidates.json')));
const MAX_FIX = Number(arg('max-fix-lines', '40'));
const MAX_INTRO = Number(arg('max-intro-lines', '900'));

function git(args: string[]): string {
  const r = spawnSync('git', ['-C', repo, ...args], { encoding: 'utf8', maxBuffer: 256 << 20 });
  if (r.status !== 0) throw new Error(`git ${args.join(' ')}: ${r.stderr}`);
  return r.stdout;
}

// Code a reviewer reads; tests, docs, fixtures, translations and generated catalogues are not where a fix's defect lives
const SOURCE = /\.(rs|ts|tsx|kt|swift|mjs|cjs|js)$/;
const NOT_SOURCE = /(^|\/)(test|tests|fixtures|docs|lab|i18n)\/|\.test\.tsx?$|\.preview\.|assets\/model-catalog\.json$/;
const isSource = (p: string) => SOURCE.test(p) && !NOT_SOURCE.test(p);

interface Stat { path: string; added: number; removed: number }

function numstat(range: string[]): Stat[] {
  return git(['diff', '--numstat', '-M', ...range]).split('\n').filter(Boolean).map(l => {
    const [a, r, ...p] = l.split('\t');
    return { path: p.join('\t').replace(/^.*=> /, '').replace(/[{}]/g, ''), added: Number(a) || 0, removed: Number(r) || 0 };
  });
}

// The old-side line ranges a diff touches: removed lines, or for a pure insertion the line it follows (so a missing
// check is blamed on the code it should have guarded)
function oldRanges(fix: string, path: string): [number, number][] {
  const ranges: [number, number][] = [];
  for (const m of git(['diff', '-U0', `${fix}^`, fix, '--', path]).matchAll(/^@@ -(\d+)(?:,(\d+))? \+\d+(?:,\d+)? @@/gm)) {
    const start = Number(m[1]);
    const len = m[2] === undefined ? 1 : Number(m[2]);
    if (len > 0) ranges.push([start, start + len - 1]);
    else if (start > 0) ranges.push([start, start]);
  }
  return ranges;
}

// Commits that last touched those lines in the fix's parent, by how many lines each
function blame(fix: string, path: string, ranges: [number, number][]): Map<string, number> {
  const counts = new Map<string, number>();
  if (!ranges.length) return counts;
  const r = spawnSync('git', ['-C', repo, 'blame', '--porcelain', ...ranges.flatMap(([a, b]) => ['-L', `${a},${b}`]), `${fix}^`, '--', path], { encoding: 'utf8', maxBuffer: 64 << 20 });
  if (r.status !== 0) return counts;
  for (const m of r.stdout.matchAll(/^([0-9a-f]{40}) \d+ \d+(?: \d+)?$/gm)) counts.set(m[1]!, (counts.get(m[1]!) ?? 0) + 1);
  return counts;
}

const subjectOf = (sha: string) => git(['log', '-1', '--format=%s', sha]).trim();
const bodyOf = (sha: string) => git(['log', '-1', '--format=%b', sha]).trim();
const dateOf = (sha: string) => git(['log', '-1', '--format=%cs', sha]).trim();
const hasParent = (sha: string) => spawnSync('git', ['-C', repo, 'rev-parse', '--verify', '-q', `${sha}^`]).status === 0;

const fixes = git(['log', '--no-merges', '--format=%H%x09%s']).split('\n').filter(Boolean)
  .map(l => l.split('\t') as [string, string])
  .filter(([, s]) => /^fix(\(|:|!)/.test(s));

const candidates = [];
for (const [fix, subject] of fixes) {
  if (!hasParent(fix)) continue;
  const stats = numstat([`${fix}^`, fix]).filter(s => isSource(s.path));
  const lines = stats.reduce((n, s) => n + s.added + s.removed, 0);
  if (!stats.length || lines > MAX_FIX || stats.length > 3) continue;
  const blamed = new Map<string, number>();
  for (const s of stats) for (const [sha, n] of blame(fix, s.path, oldRanges(fix, s.path))) blamed.set(sha, (blamed.get(sha) ?? 0) + n);
  const ranked = [...blamed].sort((a, b) => b[1] - a[1]);
  const top = ranked[0];
  if (!top || !hasParent(top[0])) continue;
  const intro = top[0];
  const introStats = numstat([`${intro}^`, intro]);
  const introLines = introStats.filter(s => isSource(s.path)).reduce((n, s) => n + s.added + s.removed, 0);
  candidates.push({
    fix, fixSubject: subject, fixBody: bodyOf(fix), fixDate: dateOf(fix),
    fixFiles: stats.map(s => `${s.path} +${s.added} -${s.removed}`), fixLines: lines,
    intro, introSubject: subjectOf(intro), introDate: dateOf(intro),
    introLines, introFiles: introStats.length,
    // How much of what the fix touched the top commit wrote; a split blame is a weaker target
    blameShare: Math.round((top[1] / ranked.reduce((n, [, c]) => n + c, 0)) * 100) / 100,
    others: ranked.slice(1, 4).map(([sha, n]) => `${sha.slice(0, 8)} ${n}`),
    tooBig: introLines > MAX_INTRO,
  });
}

mkdirSync(join(out, '..'), { recursive: true });
writeFileSync(out, JSON.stringify(candidates, null, 2) + '\n');
const usable = candidates.filter(c => !c.tooBig);
console.log(`${fixes.length} fix commits, ${candidates.length} small ones blamed back, ${usable.length} with an introducing change under ${MAX_INTRO} source lines → ${out}`);
