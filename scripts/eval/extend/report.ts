#!/usr/bin/env tsx
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

// The extension race's report (docs/dev/evals.md, Extension race), from one or more result folders of run.ts:
//
//   - Surface: what each way can do at all, from the reference builds of a dry run (hot, only after a restart, partly,
//     or not at all and why), with the one-step rollback and the size of the reference.
//   - Speed: a plain request to the first passing check (median over the runs that passed), and the writer's first round.
//   - Accuracy: right the first time, right after the repair rounds, rounds taken.
//
//   tsx scripts/eval/extend/report.ts DIR [DIR …] [--out report.md]
//
// The report stays where the results are (output/ is not in git): the comparison with other harnesses is internal

interface Round { round: number; ms: number; pass: boolean; effect?: string; detail: string }
interface Result {
  task: string; arm: string; group: string; rep: number; dry: boolean; model?: string; writer?: string; effort?: string;
  buildMs: number; writeMs?: number; firstUseMs?: number; rounds: number; firstTry: boolean; pass: boolean; detail: string; partial?: string;
  effect?: 'hot' | 'reload' | 'restart'; cards: number; history: Round[]; files: number; lines: number;
  rollback?: { pass: boolean }; usage?: { requests: number; costUsd: number }; error?: string;
}
interface None { task: string; arm: string; group: string; reason: string }

const argv = process.argv.slice(2);
const outAt = argv.indexOf('--out');
const outFile = outAt >= 0 ? argv[outAt + 1] : undefined;
const dirs = argv.filter((a, i) => !a.startsWith('--') && !(outAt >= 0 && i === outAt + 1)).map(d => resolve(d));

const lines = <T>(file: string): T[] => (existsSync(file) ? readFileSync(file, 'utf8').split('\n').filter(Boolean).map(l => JSON.parse(l) as T) : []);
const results = dirs.flatMap(d => lines<Result>(join(d, 'results.jsonl')));
const nones = dirs.flatMap(d => lines<None>(join(d, 'none.jsonl')));
// Runs lost to the bench or the model's service, never to the writer (run.ts `outage`): listed, not counted
const infra = dirs.flatMap(d => lines<Result>(join(d, 'infra.jsonl')));

// One column per way of extending: the harness and the means
const WAYS: Record<string, string> = {
  'plugin@plugins': 'Acpira 插件',
  'opencode@opencode': 'OpenCode 2',
  'mcp@plugins': 'Acpira MCP',
  'hooks@plugins': 'Acpira hooks',
  'mcp@today': '旧版 MCP',
  'hooks@today': '旧版 hooks',
  'source@today': '旧版改源码',
};
const TASK_NAMES: Record<string, string> = {
  'fx-tool': '加工具（汇率换算）',
  'warm-theme': '改外观（暖灰配色）',
  'lock-gate': '加规矩（禁改 .lock）',
  'review-button': '加按钮（一键评审）',
  'status-widget': '加组件（状态条）',
};
const wayOf = (r: { arm: string; group: string }) => `${r.arm}@${r.group}`;
// A row's model: the model at its reasoning level, and whether Claude Code wrote on both sides (the subscription runs);
// runs from before the level was set keep the bare name, so they never mix with these
const modelOf = (r: Result) => `${r.model ?? '?'}${r.effort ? ` ${r.effort}` : ''}${r.writer?.includes('claude') ? '（Claude Code 订阅）' : ''}`;
const tasks = [...new Set([...results.map(r => r.task), ...nones.map(n => n.task)])].sort((a, b) => Object.keys(TASK_NAMES).indexOf(a) - Object.keys(TASK_NAMES).indexOf(b));
const ways = Object.keys(WAYS).filter(w => results.some(r => wayOf(r) === w) || nones.some(n => wayOf(n) === w));

const median = (xs: number[]) => {
  if (!xs.length) return undefined;
  const s = [...xs].sort((a, b) => a - b);
  return s.length % 2 ? s[(s.length - 1) / 2]! : (s[s.length / 2 - 1]! + s[s.length / 2]!) / 2;
};
const secs = (ms?: number) => (ms === undefined ? '—' : ms < 10_000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.round(ms / 1000)} s`);
const pct = (n: number, d: number) => (d ? `${Math.round((n / d) * 100)}%` : '—');

const out: string[] = [];

// Surface: the dry runs' reference builds
const dry = results.filter(r => r.dry);
if (dry.length || nones.length) {
  out.push('## 可实现面（专家参考实现，免模型）', '');
  out.push(`| 任务 | ${ways.map(w => WAYS[w]).join(' | ')} |`, `|---|${ways.map(() => '---').join('|')}|`);
  for (const t of tasks) {
    const cells = ways.map(w => {
      const none = nones.find(n => n.task === t && wayOf(n) === w);
      const runs = dry.filter(r => r.task === t && wayOf(r) === w);
      if (!runs.length) return none ? `✗ ${none.reason}` : '';
      const r = runs[runs.length - 1]!;
      if (!r.pass) return `✗ 参考实现未通过：${(r.error ?? r.detail).slice(0, 60)}`;
      const how = r.effect === 'hot' ? '热生效' : r.effect === 'reload' ? '手动 reload 后生效' : '需重启';
      const back = r.rollback ? (r.rollback.pass ? '，一步回滚' : '，回滚失败') : '';
      const part = r.partial ? `（部分：${r.partial}）` : '';
      return `${r.partial ? '◐' : '✓'} ${how}${back}${part}，${r.files} 个文件 ${r.lines} 行`;
    });
    out.push(`| ${TASK_NAMES[t] ?? t} | ${cells.join(' | ')} |`);
  }
  out.push('');
}

// Speed and accuracy: the real runs, per model and way
const real = results.filter(r => !r.dry);
if (real.length) {
  const models = [...new Set(real.map(modelOf))];
  out.push('## 写的速度与准确度（真模型，一句人话）', '');
  out.push('| 模型 | 方式 | 任务 | 次数 | 首轮就对 | 修完能用 | 平均轮数 | 写完用时（中位） | 首次可用（中位，含校验） | 生效 | 花费 |', '|---|---|---|---|---|---|---|---|---|---|---|');
  for (const m of models) for (const w of ways) for (const t of [...tasks, '合计']) {
    const runs = real.filter(r => modelOf(r) === m && wayOf(r) === w && (t === '合计' || r.task === t));
    if (!runs.length || (t === '合计' && new Set(runs.map(r => r.task)).size < 2)) continue;
    const passed = runs.filter(r => r.pass);
    const effects = [...new Set(passed.map(r => r.effect).filter(Boolean))].map(e => (e === 'hot' ? '热' : e === 'reload' ? 'reload' : '重启')).join('/');
    const spent = runs.reduce((a, r) => a + (r.usage?.costUsd ?? 0), 0);
    out.push(`| ${m} | ${WAYS[w]} | ${t === '合计' ? '**合计**' : TASK_NAMES[t] ?? t} | ${runs.length} | ${pct(runs.filter(r => r.firstTry).length, runs.length)} | ${pct(passed.length, runs.length)} | ${(runs.reduce((a, r) => a + r.rounds, 0) / runs.length).toFixed(1)} | ${secs(median(passed.map(r => r.writeMs ?? r.buildMs)))} | ${secs(median(passed.map(r => r.firstUseMs!)))} | ${effects || '—'} | ${spent ? `$${spent.toFixed(2)}` : '—'} |`);
  }
  out.push('');
  // Head to head: Acpira's plugin against OpenCode's own way, one row per task (medians over the runs that passed)
  const [a, o] = ['plugin@plugins', 'opencode@opencode'];
  if (real.some(r => wayOf(r) === a) && real.some(r => wayOf(r) === o)) {
    const effectOf = (rs: Result[]) => [...new Set(rs.filter(r => r.pass).map(r => r.effect))].map(e => (e === 'hot' ? '热' : e === 'reload' ? 'reload' : e ? '重启' : '—')).join('/') || '—';
    const passOf = (rs: Result[]) => `${rs.filter(r => r.pass).length}/${rs.length}`;
    const useOf = (rs: Result[]) => median(rs.filter(r => r.pass).map(r => r.firstUseMs!));
    const callsOf = (rs: Result[]) => median(rs.filter(r => r.pass && r.usage).map(r => r.usage!.requests));
    // Only the models both sides ran
    for (const m of models.filter(m => real.some(r => modelOf(r) === m && wayOf(r) === a) && real.some(r => modelOf(r) === m && wayOf(r) === o))) {
      out.push(`### 对照：${m}`, '');
      out.push('| 任务 | 跑通 Acpira / OpenCode | 首次可用 Acpira | 首次可用 OpenCode | OpenCode 用时倍数 | 模型调用 Acpira / OpenCode | 生效 Acpira / OpenCode |', '|---|---|---|---|---|---|---|');
      // The total adds up the tasks both sides passed
      const sum = { a: 0, o: 0, ca: 0, co: 0, n: 0 };
      for (const t of tasks) {
        const ra = real.filter(r => modelOf(r) === m && wayOf(r) === a && r.task === t);
        const ro = real.filter(r => modelOf(r) === m && wayOf(r) === o && r.task === t);
        if (!ra.length || !ro.length) continue;
        const [ua, uo] = [useOf(ra), useOf(ro)];
        const ratio = ua && uo ? `${(uo / ua).toFixed(1)}×` : '—';
        if (ua && uo) Object.assign(sum, { a: sum.a + ua, o: sum.o + uo, ca: sum.ca + (callsOf(ra) ?? 0), co: sum.co + (callsOf(ro) ?? 0), n: sum.n + 1 });
        out.push(`| ${TASK_NAMES[t] ?? t} | ${passOf(ra)} / ${passOf(ro)} | ${secs(ua)} | ${secs(uo)} | ${ratio} | ${callsOf(ra) ?? '—'} / ${callsOf(ro) ?? '—'} | ${effectOf(ra)} / ${effectOf(ro)} |`);
      }
      if (sum.n > 1) out.push(`| **合计**（${sum.n} 项都跑通） | | ${secs(sum.a)} | ${secs(sum.o)} | ${(sum.o / sum.a).toFixed(1)}× | ${sum.ca} / ${sum.co} | |`);
      out.push('');
    }
  }
  const failures = real.filter(r => !r.pass);
  if (failures.length) {
    out.push('### 没跑通的', '');
    for (const r of failures) out.push(`- ${modelOf(r)} ${WAYS[wayOf(r)]} ${TASK_NAMES[r.task] ?? r.task} #${r.rep}：${(r.error ?? r.detail).slice(0, 200)}`);
    out.push('');
  }
}

if (infra.length) {
  out.push('### 评测故障（不计入，已补跑）', '');
  for (const r of infra) out.push(`- ${modelOf(r)} ${WAYS[wayOf(r)] ?? wayOf(r)} ${TASK_NAMES[r.task] ?? r.task} #${r.rep}：${(r.error ?? r.detail).split('\n')[0]!.slice(0, 160)}`);
  out.push('');
}

const text = out.join('\n');
console.log(text);
if (outFile) writeFileSync(outFile, `${text}\n`);
