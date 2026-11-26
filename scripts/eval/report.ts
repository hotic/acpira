import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { cost } from './prices';
import { isHoldoutTask } from './suites';

// Aggregates run.ts results: per model × harness (or per configuration) the pass rate, flip rate across
// paired repetitions, share of hidden tests passed, total cost per passed task (failed runs, helpers and
// reviews included), requests, Lead cache hit %, guardrails, critique outcomes, wall time and quota.
// With --dump bodies, "est. k" is the prompt size estimated from the request bodies (JSON bytes / 3.4).
//
//   pnpm exec tsx scripts/eval/report.ts DIR [DIR…] [--by-task] [--split]

interface Result {
  runId: string; dir: string; task: string; harness: string; model: string; pass: boolean; testsPassed: number; testsTotal: number; wallS: number;
  timedOut: boolean; stop?: string; requests: number; callErrors: number; tools: number;
  input: number; cacheRead: number; cacheWrite: number; output: number; reasoning: number;
  unmetered?: boolean;
  config?: string;
  helpers?: Record<string, { requests: number; costUsd: number }>;
  quota?: Record<string, Record<string, number>>;
  subagents?: { persona?: string; role?: string; state: string; tools: number }[];
  guardrails?: { editCalls: number; editNotFound: number; outputRereads: number; bashRepeats: number };
  critique?: { ran: number; approved: number; revised: number; skipped: number; invalid: number };
  cacheGap?: { over300s: number; over300sCacheDrop: number; maxGapS: number };
}

const dirs = process.argv.slice(2).filter(a => !a.startsWith('--'));
const byTask = process.argv.includes('--by-task');
const bySplit = process.argv.includes('--split');
const rows: Result[] = dirs.flatMap(d => readFileSync(join(d, 'results.ndjson'), 'utf8').split('\n').filter(Boolean).map(l => ({ ...(JSON.parse(l) as Result), dir: d })));

const BYTES_PER_TOKEN = 3.4;
function estimated(r: Result): number | undefined {
  const dir = join(r.dir, 'bodies');
  if (!existsSync(dir)) return undefined;
  const files = readdirSync(dir).filter(f => f.startsWith(`${r.runId}.`)).map(f => statSync(join(dir, f)));
  if (!files.length) return undefined;
  const last = Math.max(...files.map(f => f.mtimeMs));
  return files.filter(f => f.mtimeMs >= last - (r.wallS + 30) * 1000).reduce((a, f) => a + f.size, 0) / BYTES_PER_TOKEN;
}

const groups = new Map<string, Result[]>();
for (const r of rows) {
  const split = bySplit ? (isHoldoutTask(r.task) ? 'holdout' : 'dev') : '';
  const k = [r.model, r.config ?? r.harness, split, byTask ? r.task : ''].join('|');
  groups.set(k, [...(groups.get(k) ?? []), r]);
}

const fmt = (n: number, d = 0) => n.toFixed(d);

function quotaText(rs: Result[]): string {
  const sums = new Map<string, number>();
  for (const r of rs) for (const [agent, ws] of Object.entries(r.quota ?? {})) for (const [w, p] of Object.entries(ws)) sums.set(`${agent} ${w}`, (sums.get(`${agent} ${w}`) ?? 0) + p);
  return [...sums].map(([k, v]) => `${k} ${(v / rs.length).toFixed(1)}`).join(', ') || '-';
}

function helperText(rs: Result[]): string {
  const rounds = rs.reduce((a, r) => a + (r.subagents ?? []).filter(x => x.persona).length, 0);
  const who = [...new Set(rs.flatMap(r => (r.subagents ?? []).flatMap(x => (x.persona ? [x.persona] : []))))];
  return rounds ? `${(rounds / rs.length).toFixed(1)} (${who.join(', ')})` : '-';
}

// Fraction of multi-rep tasks within the group whose pass/fail outcome flipped across repetitions
function flipRateText(rs: Result[]): string {
  const perTask = new Map<string, boolean[]>();
  for (const r of rs) perTask.set(r.task, [...(perTask.get(r.task) ?? []), r.pass]);
  const multi = [...perTask.values()].filter(v => v.length >= 2);
  if (!multi.length) return '-';
  const flipped = multi.filter(v => v.some(Boolean) && v.some(x => !x)).length;
  return `${fmt((flipped / multi.length) * 100)}%`;
}

function guardrailText(rs: Result[]): string {
  const withG = rs.filter(r => r.guardrails);
  if (!withG.length) return '-';
  const miss = withG.reduce((a, r) => a + (r.guardrails?.editNotFound ?? 0), 0);
  const reread = withG.reduce((a, r) => a + (r.guardrails?.outputRereads ?? 0), 0);
  const repeat = withG.reduce((a, r) => a + (r.guardrails?.bashRepeats ?? 0), 0);
  const avgTools = rs.reduce((a, r) => a + r.tools, 0) / rs.length;
  return `${miss}m/${reread}r/${repeat}b (${fmt(avgTools, 0)}t)`;
}

function critiqueText(rs: Result[]): string {
  const ran = rs.reduce((a, r) => a + (r.critique?.ran ?? 0), 0);
  const rev = rs.reduce((a, r) => a + (r.critique?.revised ?? 0), 0);
  const inv = rs.reduce((a, r) => a + (r.critique?.invalid ?? 0), 0);
  const skip = rs.reduce((a, r) => a + (r.critique?.skipped ?? 0), 0);
  if (ran + skip === 0) return '-';
  return `${ran}ran/${rev}rev/${inv}inv/${skip}skip`;
}

const hasCritique = rows.some(r => r.critique);
const extraHead = `${bySplit ? ' | split' : ''}${byTask ? ' | task' : ''}`;
const extraSep = `${bySplit ? '---|' : ''}${byTask ? '---|' : ''}`;
console.log(`| model | harness${extraHead} | runs | pass | flip | tests | cost $ | $ / pass | requests | input k | est. k | lead cache | output k | wall min | guardrails |${hasCritique ? ' critique |' : ''} stops | rounds / run | quota / run |`);
console.log(`|---|---|${extraSep}---|---|---|---|---|---|---|---|---|---|---|---|---|${hasCritique ? '---|' : ''}---|---|---|`);
for (const [k, rs] of [...groups].sort()) {
  const [model, harness, split, task] = k.split('|');
  const passes = rs.filter(r => r.pass).length;
  const metered = !rs.some(r => r.unmetered);
  const usd = rs.reduce((a, r) => a + cost(r) + Object.values(r.helpers ?? {}).reduce((b, h) => b + h.costUsd, 0), 0);
  const tests = rs.reduce((a, r) => a + (r.testsTotal ? r.testsPassed / r.testsTotal : 0), 0) / rs.length;
  const input = rs.reduce((a, r) => a + r.input, 0);
  const cached = rs.reduce((a, r) => a + r.cacheRead, 0);
  const est = rs.map(estimated);
  const estK = est.every(e => e !== undefined) ? fmt(est.reduce((a, e) => a + e!, 0) / rs.length / 1000) : '-';
  const stops = [...new Set(rs.map(r => (r.timedOut ? 'timeout' : r.stop ?? '?')))].join(',');
  const extraVal = `${bySplit ? ` | ${split}` : ''}${byTask ? ` | ${task}` : ''}`;
  console.log(`| ${model} | ${harness}${extraVal} | ${rs.length} | ${passes}/${rs.length} | ${flipRateText(rs)} | ${fmt(tests * 100)}% | ${metered ? fmt(usd, 3) : '-'} | ${metered && passes ? fmt(usd / passes, 3) : '-'} | ${fmt(rs.reduce((a, r) => a + r.requests, 0) / rs.length, 1)} | ${fmt(input / rs.length / 1000)} | ${estK} | ${input ? fmt((cached / input) * 100) : 0}% | ${fmt(rs.reduce((a, r) => a + r.output, 0) / rs.length / 1000, 1)} | ${fmt(rs.reduce((a, r) => a + r.wallS, 0) / rs.length / 60, 1)} | ${guardrailText(rs)} |${hasCritique ? ` ${critiqueText(rs)} |` : ''} ${stops} | ${helperText(rs)} | ${quotaText(rs)} |`);
}
