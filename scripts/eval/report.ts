import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { MODELS } from './harnesses';

// Aggregates run.ts results: per model × harness the pass rate, the share of hidden tests passed, what the runs cost at
// catalogue prices (failed runs included, so cost per pass charges the failures too), requests, cache hit and stops.
// With --dump bodies, "est. k" is the prompt size estimated from the request bodies (JSON bytes / 3.4, calibrated on a
// prose prompt), a ruler independent of what the provider reports (this gateway's Qwen counts miss most of the history)
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/report.ts DIR [DIR…] [--by-task]

interface Result {
  runId: string; dir: string; task: string; harness: string; model: string; pass: boolean; testsPassed: number; testsTotal: number; wallS: number;
  timedOut: boolean; stop?: string; requests: number; callErrors: number; tools: number;
  input: number; cacheRead: number; cacheWrite: number; output: number; reasoning: number;
}

interface Price { input: number; output: number; cacheRead?: number; cacheWrite?: number }

const catalog = JSON.parse(readFileSync(resolve(import.meta.dirname, '../../rust/crates/acpira-shared/assets/model-catalog.json'), 'utf8')) as { models: { id: string; cost?: Price }[] };

function price(model: string): Price {
  const id = MODELS[model]?.catalog ?? model;
  const p = catalog.models.find(m => m.id === id)?.cost;
  if (!p) throw new Error(`no catalogue price for ${id}`);
  return p;
}

// USD: uncached input, cache reads and writes, output (reasoning is part of output on every wire read here)
export function cost(r: Pick<Result, 'model' | 'input' | 'cacheRead' | 'cacheWrite' | 'output'>): number {
  const p = price(r.model);
  const uncached = Math.max(0, r.input - r.cacheRead - r.cacheWrite);
  return (uncached * p.input + r.cacheRead * (p.cacheRead ?? p.input) + r.cacheWrite * (p.cacheWrite ?? p.input) + r.output * p.output) / 1e6;
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
  const k = [r.model, r.harness, byTask ? r.task : ''].join('|');
  groups.set(k, [...(groups.get(k) ?? []), r]);
}

const fmt = (n: number, d = 0) => n.toFixed(d);
console.log(`| model | harness${byTask ? ' | task' : ''} | runs | pass | tests | cost $ | $ / pass | requests | input k | est. k | cache hit | output k | wall min | stops |`);
console.log(`|---|---|${byTask ? '---|' : ''}---|---|---|---|---|---|---|---|---|---|---|---|`);
for (const [k, rs] of [...groups].sort()) {
  const [model, harness, task] = k.split('|');
  const passes = rs.filter(r => r.pass).length;
  const usd = rs.reduce((a, r) => a + cost(r), 0);
  const tests = rs.reduce((a, r) => a + (r.testsTotal ? r.testsPassed / r.testsTotal : 0), 0) / rs.length;
  const input = rs.reduce((a, r) => a + r.input, 0);
  const cached = rs.reduce((a, r) => a + r.cacheRead, 0);
  const est = rs.map(estimated);
  const estK = est.every(e => e !== undefined) ? fmt(est.reduce((a, e) => a + e!, 0) / rs.length / 1000) : '-';
  const stops = [...new Set(rs.map(r => (r.timedOut ? 'timeout' : r.stop ?? '?')))].join(',');
  console.log(`| ${model} | ${harness}${byTask ? ` | ${task}` : ''} | ${rs.length} | ${passes}/${rs.length} | ${fmt(tests * 100)}% | ${fmt(usd, 3)} | ${passes ? fmt(usd / passes, 3) : '-'} | ${fmt(rs.reduce((a, r) => a + r.requests, 0) / rs.length, 1)} | ${fmt(input / rs.length / 1000)} | ${estK} | ${input ? fmt((cached / input) * 100) : 0}% | ${fmt(rs.reduce((a, r) => a + r.output, 0) / rs.length / 1000, 1)} | ${fmt(rs.reduce((a, r) => a + r.wallS, 0) / rs.length / 60, 1)} | ${stops} |`);
}
