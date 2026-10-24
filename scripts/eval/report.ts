import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { cost } from './prices';

// Aggregates run.ts results: per model × harness (or per configuration) the pass rate, the share of hidden tests passed,
// what the runs cost at catalogue prices (failed runs included, so cost per pass charges the failures too; a
// configuration's helpers included), requests, cache hit, stops, and for native logins the quota spent per run.
// With --dump bodies, "est. k" is the prompt size estimated from the request bodies (JSON bytes / 3.4, calibrated on a
// prose prompt), a ruler independent of what the provider reports (this gateway's Qwen counts miss most of the history)
//
//   pnpm exec tsx scripts/eval/report.ts DIR [DIR…] [--by-task]

interface Result {
  runId: string; dir: string; task: string; harness: string; model: string; pass: boolean; testsPassed: number; testsTotal: number; wallS: number;
  timedOut: boolean; stop?: string; requests: number; callErrors: number; tools: number;
  input: number; cacheRead: number; cacheWrite: number; output: number; reasoning: number;
  // The harness reached its own backend, so no call went through the meter: tokens and cost are unknown
  unmetered?: boolean;
  // A named configuration (configs.ts) and its helpers' metered usage
  config?: string;
  helpers?: Record<string, { requests: number; costUsd: number }>;
  // Percentage points of each native login's quota window the run spent
  quota?: Record<string, Record<string, number>>;
  subagents?: { persona?: string; role?: string; state: string; tools: number }[];
}

const dirs = process.argv.slice(2).filter(a => !a.startsWith('--'));
const byTask = process.argv.includes('--by-task');
const rows: Result[] = dirs.flatMap(d => readFileSync(join(d, 'results.ndjson'), 'utf8').split('\n').filter(Boolean).map(l => ({ ...(JSON.parse(l) as Result), dir: d })));

// Bytes of the run's dumped request bodies / 3.4; only the latest attempt at a run (files within its wall time of the last)
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
  const k = [r.model, r.config ?? r.harness, byTask ? r.task : ''].join('|');
  groups.set(k, [...(groups.get(k) ?? []), r]);
}

const fmt = (n: number, d = 0) => n.toFixed(d);
// Mean points spent per run of each native window, e.g. "claude 5h 2.5"
function quotaText(rs: Result[]): string {
  const sums = new Map<string, number>();
  for (const r of rs) for (const [agent, ws] of Object.entries(r.quota ?? {})) for (const [w, p] of Object.entries(ws)) sums.set(`${agent} ${w}`, (sums.get(`${agent} ${w}`) ?? 0) + p);
  return [...sums].map(([k, v]) => `${k} ${(v / rs.length).toFixed(1)}`).join(', ') || '-';
}

// Summoned rounds per run and the personas that ran them
function helperText(rs: Result[]): string {
  const rounds = rs.reduce((a, r) => a + (r.subagents ?? []).filter(x => x.persona).length, 0);
  const who = [...new Set(rs.flatMap(r => (r.subagents ?? []).flatMap(x => (x.persona ? [x.persona] : []))))];
  return rounds ? `${(rounds / rs.length).toFixed(1)} (${who.join(', ')})` : '-';
}

console.log(`| model | harness${byTask ? ' | task' : ''} | runs | pass | tests | cost $ | $ / pass | requests | input k | est. k | cache hit | output k | wall min | stops | rounds / run | quota / run |`);
console.log(`|---|---|${byTask ? '---|' : ''}---|---|---|---|---|---|---|---|---|---|---|---|---|---|`);
for (const [k, rs] of [...groups].sort()) {
  const [model, harness, task] = k.split('|');
  const passes = rs.filter(r => r.pass).length;
  const metered = !rs.some(r => r.unmetered);
  const usd = rs.reduce((a, r) => a + cost(r) + Object.values(r.helpers ?? {}).reduce((b, h) => b + h.costUsd, 0), 0);
  const tests = rs.reduce((a, r) => a + (r.testsTotal ? r.testsPassed / r.testsTotal : 0), 0) / rs.length;
  const input = rs.reduce((a, r) => a + r.input, 0);
  const cached = rs.reduce((a, r) => a + r.cacheRead, 0);
  const est = rs.map(estimated);
  const estK = est.every(e => e !== undefined) ? fmt(est.reduce((a, e) => a + e!, 0) / rs.length / 1000) : '-';
  const stops = [...new Set(rs.map(r => (r.timedOut ? 'timeout' : r.stop ?? '?')))].join(',');
  console.log(`| ${model} | ${harness}${byTask ? ` | ${task}` : ''} | ${rs.length} | ${passes}/${rs.length} | ${fmt(tests * 100)}% | ${metered ? fmt(usd, 3) : '-'} | ${metered && passes ? fmt(usd / passes, 3) : '-'} | ${fmt(rs.reduce((a, r) => a + r.requests, 0) / rs.length, 1)} | ${fmt(input / rs.length / 1000)} | ${estK} | ${input ? fmt((cached / input) * 100) : 0}% | ${fmt(rs.reduce((a, r) => a + r.output, 0) / rs.length / 1000, 1)} | ${fmt(rs.reduce((a, r) => a + r.wallS, 0) / rs.length / 60, 1)} | ${stops} | ${helperText(rs)} | ${quotaText(rs)} |`);
}
