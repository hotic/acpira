import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// Review bench report (docs/dev/evals.md): per cell (variant × reviewers) the hit rate over the known bugs, the
// findings that match none, tools, time and cost; the noise between repeated runs of one cell; and every pair of cells
// compared target by target, the comparison the adoption rule reads (a cell wins on hits beyond the noise, then on time)
//
//   pnpm exec tsx scripts/eval/review/report.ts DIR [DIR…] [--by-target]

interface PerReviewer { name: string; state: string; tools: number; wallS: number; costUsd?: number; unmetered?: boolean; requests: number }
interface Row {
  runId: string; target: string; variant: string; reviewers: string; rep: number;
  bugs: number; found: number; others: number; wallS: number; timedOut: boolean; failure?: string; perReviewer: PerReviewer[];
}

const dirs = process.argv.slice(2).filter(a => !a.startsWith('--'));
const byTarget = process.argv.includes('--by-target');
const rows: Row[] = dirs.flatMap(d => readFileSync(join(d, 'results.ndjson'), 'utf8').split('\n').filter(Boolean).map(l => JSON.parse(l) as Row));

const cellOf = (r: Row) => `${r.variant} ${r.reviewers}`;
const median = (xs: number[]) => {
  if (!xs.length) return 0;
  const s = [...xs].sort((a, b) => a - b);
  return s.length % 2 ? s[(s.length - 1) / 2]! : (s[s.length / 2 - 1]! + s[s.length / 2]!) / 2;
};
const fmt = (n: number, d = 0) => n.toFixed(d);

const cells = new Map<string, Row[]>();
for (const r of rows) cells.set(cellOf(r), [...(cells.get(cellOf(r)) ?? []), r]);

console.log('| cell | runs | hit rate | found / bugs | others / run | tools / run | wall min | $ / run | failed |');
console.log('|---|---|---|---|---|---|---|---|---|');
for (const [cell, rs] of [...cells].sort()) {
  const found = rs.reduce((a, r) => a + r.found, 0);
  const bugs = rs.reduce((a, r) => a + r.bugs, 0);
  const tools = median(rs.map(r => r.perReviewer.reduce((a, p) => a + p.tools, 0)));
  const wall = median(rs.map(r => r.wallS)) / 60;
  const metered = rs.every(r => r.perReviewer.every(p => !p.unmetered));
  const usd = rs.reduce((a, r) => a + r.perReviewer.reduce((b, p) => b + (p.costUsd ?? 0), 0), 0) / rs.length;
  const failed = rs.filter(r => r.timedOut || r.failure || r.perReviewer.some(p => p.state !== 'completed')).length;
  console.log(`| ${cell} | ${rs.length} | ${fmt((found / Math.max(1, bugs)) * 100)}% | ${found} / ${bugs} | ${fmt(rs.reduce((a, r) => a + r.others, 0) / rs.length, 1)} | ${fmt(tools)} | ${fmt(wall, 1)} | ${metered ? fmt(usd, 2) : `${fmt(usd, 2)}+`} | ${failed} |`);
}

// Noise: within one cell, how often two runs of the same target disagree on a bug (found in one, missed in the other)
console.log('\n| cell | targets run twice or more | bug outcomes that flip between runs |');
console.log('|---|---|---|');
for (const [cell, rs] of [...cells].sort()) {
  const byT = new Map<string, Row[]>();
  for (const r of rs) byT.set(r.target, [...(byT.get(r.target) ?? []), r]);
  let pairs = 0;
  let flips = 0;
  for (const runs of byT.values()) {
    for (let i = 0; i < runs.length; i++) for (let k = i + 1; k < runs.length; k++) {
      pairs += runs[i]!.bugs;
      flips += Math.abs(runs[i]!.found - runs[k]!.found);
    }
  }
  const twice = [...byT.values()].filter(x => x.length > 1).length;
  if (twice) console.log(`| ${cell} | ${twice} | ${fmt((flips / Math.max(1, pairs)) * 100)}% |`);
}

// Paired: per target, the mean bugs found per run in each cell; a target counts as a win for the cell ahead on it
const mean = (rs: Row[]) => rs.reduce((a, r) => a + r.found, 0) / rs.length;
const names = [...cells.keys()].sort();
if (names.length > 1) {
  console.log('\n| cell A | cell B | shared targets | A ahead | B ahead | tied | mean found A − B | median wall A − B (min) |');
  console.log('|---|---|---|---|---|---|---|---|');
  for (let i = 0; i < names.length; i++) for (let k = i + 1; k < names.length; k++) {
    const a = cells.get(names[i]!)!;
    const b = cells.get(names[k]!)!;
    const shared = [...new Set(a.map(r => r.target))].filter(t => b.some(r => r.target === t));
    let ahead = 0;
    let behind = 0;
    let diff = 0;
    for (const t of shared) {
      const d = mean(a.filter(r => r.target === t)) - mean(b.filter(r => r.target === t));
      diff += d;
      if (d > 0) ahead++;
      else if (d < 0) behind++;
    }
    const wall = (rs: Row[]) => median(rs.filter(r => shared.includes(r.target)).map(r => r.wallS)) / 60;
    console.log(`| ${names[i]} | ${names[k]} | ${shared.length} | ${ahead} | ${behind} | ${shared.length - ahead - behind} | ${fmt(diff / Math.max(1, shared.length), 2)} | ${fmt(wall(a) - wall(b), 1)} |`);
  }
}

if (byTarget) {
  const targets = [...new Set(rows.map(r => r.target))].sort();
  console.log(`\n| target | ${names.join(' | ')} |`);
  console.log(`|---|${names.map(() => '---|').join('')}`);
  for (const t of targets) {
    const cellsFor = names.map(n => {
      const rs = cells.get(n)!.filter(r => r.target === t);
      return rs.length ? rs.map(r => `${r.found}/${r.bugs}`).join(' ') : '-';
    });
    console.log(`| ${t} | ${cellsFor.join(' | ')} |`);
  }
}
