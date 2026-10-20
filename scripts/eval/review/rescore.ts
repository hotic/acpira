import { copyFileSync, existsSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { targetsOf, type Target } from './bench';
import { JUDGE_MODEL, judge, type Verdict } from './judge';

// Rescores stored reviews after a target's definition was corrected (docs/dev/evals.md, Review bench): every run of the
// given targets has its reviews judged again with the current targets.json and the same judge, and its verdict files
// and results.ndjson line are rewritten. The first rescore of a folder keeps the original as results.before-rescore.ndjson
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/review/rescore.ts DIR --target id,… [--reviewers cell,…]

interface ReviewerRow { name: string; hits: number[]; others: number; findings: number; judgeError?: string }
interface Row { runId: string; set: 'dev' | 'holdout'; target: string; reviewers: string; bugs: number; hits: number[]; found: number; others: number; judge: string; dry?: boolean; perReviewer: ReviewerRow[] }

const arg = (k: string) => {
  const i = process.argv.indexOf(`--${k}`);
  return i > 0 ? process.argv[i + 1] : undefined;
};
const dir = resolve(process.argv[2] ?? '.');
const file = join(dir, 'results.ndjson');
const ids = new Set((arg('target') ?? '').split(',').filter(Boolean));
const cells = new Set((arg('reviewers') ?? '').split(',').filter(Boolean));
if (!existsSync(file) || !ids.size) {
  console.error('usage: rescore.ts DIR --target id,… [--reviewers cell,…]');
  process.exit(2);
}

const text = readFileSync(file, 'utf8');
const rows = text.split('\n').filter(Boolean).map(l => JSON.parse(l) as Row);
const targets = new Map<string, Target>();
for (const set of ['dev', 'holdout'] as const) for (const t of targetsOf(set).targets) targets.set(`${set}:${t.id}`, t);

for (const row of rows) {
  if (!ids.has(row.target) || (cells.size && !cells.has(row.reviewers)) || row.dry) continue;
  const t = targets.get(`${row.set}:${row.target}`);
  if (!t) throw new Error(`no ${row.set} target ${row.target}`);
  const before = `${row.found}/${row.bugs}`;
  const hits = new Set<number>();
  let others = 0;
  for (const r of row.perReviewer) {
    const reviewFile = join(dir, 'runs', row.runId, `review.${r.name}.md`);
    const review = existsSync(reviewFile) ? readFileSync(reviewFile, 'utf8') : '';
    const v: Verdict = review.trim() ? await judge(row.judge || JUDGE_MODEL, t, review) : { findings: [], hits: [], others: 0, judge: row.judge };
    // A failed judge call leaves the folder as it was
    if (v.error) throw new Error(`${row.runId} ${r.name}: the judge failed: ${v.error}`);
    writeFileSync(join(dir, 'runs', row.runId, `verdict.${r.name}.json`), JSON.stringify(v, null, 2));
    Object.assign(r, { hits: v.hits, others: v.others, findings: v.findings.length });
    delete r.judgeError;
    for (const h of v.hits) hits.add(h);
    others += v.others;
  }
  Object.assign(row, { hits: [...hits].sort(), found: hits.size, others });
  console.log(`[${row.runId}] ${before} -> ${row.found}/${row.bugs}`);
}

const backup = join(dir, 'results.before-rescore.ndjson');
if (!existsSync(backup)) copyFileSync(file, backup);
// A runner still appending to the file: the lines it added meanwhile go after the rewritten ones
const now = readFileSync(file, 'utf8');
if (!now.startsWith(text)) throw new Error('results.ndjson was rewritten during the rescore');
writeFileSync(`${file}.tmp`, rows.map(r => JSON.stringify(r)).join('\n') + '\n' + now.slice(text.length));
renameSync(`${file}.tmp`, file);
