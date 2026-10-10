import { spawnSync } from 'node:child_process';
import { appendFileSync, cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import type { AgentTurn, PermissionBlock, QuestionBlock, SessionView } from '@shared/transcript';
import { Host } from '../lib/host';
import { HARNESSES, MODELS } from './harnesses';
import { type Call, startMeter } from './meter';

// Harness comparison on long tasks (docs/dev/builtin-agent.md, "Harness comparison"). Spends real model calls.
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/run.ts --model glm-5.3 --harness acpira,opencode,pi
//     [--task calc-lang,conf-bugs] [--reps 1] [--out DIR] [--timeout-min N] [--dump]
//
// Each run: a fresh copy of the task's repo (git initialised), a fresh HOME and sidecar home, the harness pointed at the
// metering proxy, one prompt through the real sidecar with every permission card allowed, then the hidden tests. One
// line per run goes to DIR/results.ndjson and one per model call to DIR/calls.ndjson; work trees stay under DIR/runs.
// ACPIRA_SIDECAR_BIN pins the engine under test (otherwise the workspace's debug build, rebuilt before every run)

const TASKS_DIR = resolve(import.meta.dirname, 'tasks');
// The gateway itself out of capacity (seen 2026-10-11 on every model in turn): a run that failed with one of these says
// nothing about the harness, so it goes to infra.ndjson instead and is run again on the next launch
const OUTAGE = /no eligible upstream channel|no route configured for model|rate limited or quota exhausted/;
const VERIFY_TIMEOUT_MS = 300_000;

function arg(name: string, fallback?: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
}

// The gateway key, read where Codex keeps it; never printed or written to a run's files
function gatewayKey(): string {
  if (process.env.EVAL_GW_KEY) return process.env.EVAL_GW_KEY;
  const toml = readFileSync(join(homedir(), '.codex/config.toml'), 'utf8');
  const section = toml.split(/^\[/m).find(s => s.startsWith('model_providers.asgard]'));
  const m = section && /experimental_bearer_token\s*=\s*"([^"]+)"/.exec(section);
  if (!m) throw new Error('no gateway key (EVAL_GW_KEY or ~/.codex/config.toml model_providers.asgard)');
  return m[1]!;
}

// DeepSeek's own key, only when a model goes there: EVAL_DS_KEY, or the file EVAL_DS_KEY_FILE names; never printed
function deepseekKey(): string | undefined {
  if (process.env.EVAL_DS_KEY) return process.env.EVAL_DS_KEY;
  return process.env.EVAL_DS_KEY_FILE ? readFileSync(process.env.EVAL_DS_KEY_FILE, 'utf8').trim() : undefined;
}

interface Task { id: string; kind: string; prompt: string; verify: string; timeoutMin: number }

function loadTask(id: string): Task {
  return JSON.parse(readFileSync(join(TASKS_DIR, id, 'task.json'), 'utf8')) as Task;
}

function sh(cmd: string, cwd: string, timeout = 60_000, env: Record<string, string> = {}) {
  return spawnSync('bash', ['-c', cmd], { cwd, timeout, encoding: 'utf8', env: { ...process.env, ...env }, maxBuffer: 64 << 20 });
}

// Hidden tests run one at a time across every runner process (a timing test must not compete with another verify);
// mkdir is the atomic test-and-set, and a lock older than the verify timeout is taken as abandoned
async function verifyLock<T>(root: string, f: () => T): Promise<T> {
  const lock = join(root, '..', '.verify.lock');
  for (;;) {
    try {
      mkdirSync(lock);
      break;
    } catch {
      const age = Date.now() - (statSync(lock, { throwIfNoEntry: false })?.mtimeMs ?? Date.now());
      if (age > VERIFY_TIMEOUT_MS + 60_000) rmSync(lock, { recursive: true, force: true });
      await new Promise(r => setTimeout(r, 1000));
    }
  }
  try {
    return f();
  } finally {
    rmSync(lock, { recursive: true, force: true });
  }
}

// unittest's summary: tests run and how many failed or errored
function unittestCounts(out: string): { total: number; passed: number } {
  const ran = /Ran (\d+) tests?/.exec(out);
  const total = ran ? Number(ran[1]) : 0;
  const failed = /FAILED \(([^)]*)\)/.exec(out);
  let bad = 0;
  if (failed) for (const m of failed[1]!.matchAll(/(failures|errors)=(\d+)/g)) bad += Number(m[2]);
  return { total, passed: Math.max(0, total - bad) };
}

function lastAgent(v: SessionView | undefined): AgentTurn | undefined {
  const t = v?.turns[v.turns.length - 1];
  return t?.role === 'agent' ? t : undefined;
}

async function runOne(o: { task: Task; harness: string; model: string; rep: number; root: string; meterUrl: string; calls: Call[]; timeoutMin: number }) {
  const runId = `${o.task.id}.${o.harness}.${o.model}.${o.rep}`.replace(/[^\w.-]/g, '_');
  const dir = join(o.root, 'runs', runId);
  const work = join(dir, 'work');
  const home = join(dir, 'home');
  const acpiraHome = join(dir, 'acpira');
  // A run interrupted before it reported leaves its tree behind; start over from the task's repo
  rmSync(dir, { recursive: true, force: true });
  // Its dumped bodies too: the numbering restarts with each runner process
  const bodies = join(o.root, 'bodies');
  for (const f of existsSync(bodies) ? readdirSync(bodies) : []) if (f.startsWith(`${runId}.`)) rmSync(join(bodies, f));
  for (const d of [work, home, acpiraHome]) mkdirSync(d, { recursive: true });
  cpSync(join(TASKS_DIR, o.task.id, 'repo'), work, { recursive: true });
  sh('git init -q && git add -A && git -c user.name=eval -c user.email=eval@localhost commit -qm "Initial state"', work);

  const setup = HARNESSES[o.harness]!({ model: MODELS[o.model]!, base: `${o.meterUrl}/r/${runId}/${MODELS[o.model]!.upstream ?? 'gw'}`, home, acpiraHome });
  const started = Date.now();
  const host = await Host.start({
    cwd: work, home: acpiraHome, defaultAgent: setup.agent,
    env: { HOME: home, PYTHONDONTWRITEBYTECODE: '1', ...setup.env },
  });
  const view = await host.view();
  // Every permission is allowed once; a question is skipped (the prompt asks for autonomous work). A card leaves the turn
  // once answered, so each id is clicked only once
  const clicked = new Set<string>();
  let cards = 0;
  let questions = 0;
  const approver = setInterval(() => {
    const v = view.active();
    for (const b of lastAgent(v)?.blocks ?? []) {
      if (b.type !== 'permission' && b.type !== 'question') continue;
      if (clicked.has(b.id)) continue;
      if (b.type === 'permission') {
        const p = b as PermissionBlock;
        const opt = p.options.find(x => x.kind === 'allow_once') ?? p.options.find(x => x.kind === 'allow_always') ?? p.options[0];
        if (!opt) continue;
        clicked.add(b.id);
        cards++;
        view.post({ type: 'permission', sessionId: v!.id, blockId: p.id, optionId: opt.id });
      } else if (!(b as QuestionBlock).outcome) {
        clicked.add(b.id);
        questions++;
        view.post({ type: 'answer', sessionId: v!.id, blockId: b.id, answers: {}, skip: true });
      }
    }
  }, 200);

  let timedOut = false;
  let failure: string | undefined;
  try {
    await view.newSession(setup.agent);
    const v0 = view.active()!;
    const modelOpt = v0.controls.options.find(x => x.category === 'model');
    console.log(`[${runId}] session ${v0.id} status=${v0.status} model=${JSON.stringify(modelOpt?.value ?? null)}`);
    try {
      await view.handle({ type: 'send', text: o.task.prompt }, o.timeoutMin * 60_000);
    } catch (e) {
      timedOut = true;
      failure = String(e);
      view.post({ type: 'stop', sessionId: view.activeId });
      await view.until(() => !view.active()?.running, 30_000, 'the stopped turn').catch(() => {});
    }
  } catch (e) {
    failure = String(e);
  }
  clearInterval(approver);
  const v = view.active();
  const turn = lastAgent(v);
  const tools = turn?.blocks.filter(b => b.type === 'tool_call').length ?? 0;
  const wallS = Math.round((Date.now() - started) / 1000);
  await host.dispose().catch(() => {});

  // The hidden tests, copied in only now so the agent never saw them
  cpSync(join(TASKS_DIR, o.task.id, 'hidden'), join(work, '.eval_hidden'), { recursive: true });
  const ver = await verifyLock(o.root, () => sh(o.task.verify, work, VERIFY_TIMEOUT_MS, { HOME: home, PYTHONDONTWRITEBYTECODE: '1' }));
  const out = `${ver.stdout ?? ''}\n${ver.stderr ?? ''}`;
  writeFileSync(join(dir, 'verify.txt'), out);
  const counts = unittestCounts(out);
  const verifyTimedOut = ver.error?.message.includes('ETIMEDOUT') ?? false;

  const mine = o.calls.filter(c => c.run === runId);
  const sum = (k: 'input' | 'cacheRead' | 'cacheWrite' | 'output' | 'reasoning') => mine.reduce((a, c) => a + c[k], 0);
  const result = {
    runId, task: o.task.id, harness: o.harness, model: o.model, rep: o.rep,
    pass: ver.status === 0 && counts.total > 0, testsPassed: counts.passed, testsTotal: counts.total, verifyTimedOut,
    wallS, timedOut, failure, status: v?.status, stop: turn?.stop, error: turn?.error ?? v?.error,
    tools, cards, questions,
    requests: mine.length, callErrors: mine.filter(c => c.error).length,
    input: sum('input'), cacheRead: sum('cacheRead'), cacheWrite: sum('cacheWrite'), output: sum('output'), reasoning: sum('reasoning'),
    harnessUsage: v?.usage,
  };
  const outage = !result.pass && mine.some(c => c.error && OUTAGE.test(c.error));
  appendFileSync(join(o.root, outage ? 'infra.ndjson' : 'results.ndjson'), JSON.stringify(result) + '\n');
  if (outage) console.log(`[${runId}] gateway outage: recorded in infra.ndjson, not counted`);
  console.log(`[${runId}] pass=${result.pass} tests=${counts.passed}/${counts.total} wall=${wallS}s requests=${result.requests} in=${result.input} cached=${result.cacheRead} out=${result.output} stop=${result.stop}${failure ? ` failure=${failure}` : ''}`);
  return result;
}

async function main() {
  const model = arg('model');
  const harnesses = (arg('harness') ?? '').split(',').filter(Boolean);
  const tasks = (arg('task') ?? readdirSync(TASKS_DIR).filter(d => existsSync(join(TASKS_DIR, d, 'task.json'))).join(',')).split(',');
  const reps = Number(arg('reps', '1'));
  if (!model || !MODELS[model] || !harnesses.length || harnesses.some(h => !HARNESSES[h])) {
    console.error(`usage: run.ts --model ${Object.keys(MODELS).join('|')} --harness ${Object.keys(HARNESSES).join(',')} [--task …] [--reps N] [--out DIR]`);
    process.exit(2);
  }
  const root = resolve(arg('out') ?? join('/tmp/acpira-eval', new Date().toISOString().replace(/[:.]/g, '-')));
  mkdirSync(root, { recursive: true });
  // A run already in results.ndjson is skipped, so an interrupted matrix resumes where it stopped
  const resultsFile = join(root, 'results.ndjson');
  const done = new Set(existsSync(resultsFile) ? readFileSync(resultsFile, 'utf8').split('\n').filter(Boolean).map(l => (JSON.parse(l) as { runId: string }).runId) : []);
  const upstream = MODELS[model]!.upstream ?? 'gw';
  const keys: Record<string, string> = upstream === 'ds' ? { ds: deepseekKey() ?? '' } : { gw: gatewayKey() };
  if (!Object.values(keys)[0]) throw new Error('no DeepSeek key (EVAL_DS_KEY or EVAL_DS_KEY_FILE)');
  const meter = await startMeter({ log: join(root, 'calls.ndjson'), keys, dump: process.argv.includes('--dump') ? join(root, 'bodies') : undefined });
  console.log(`results in ${root}`);
  try {
    for (let rep = 1; rep <= reps; rep++) {
      for (const t of tasks) {
        const task = loadTask(t);
        for (const harness of harnesses) {
          if (done.has(`${task.id}.${harness}.${model}.${rep}`.replace(/[^\w.-]/g, '_'))) continue;
          const timeoutMin = Number(arg('timeout-min', String(task.timeoutMin)));
          await runOne({ task, harness, model, rep, root, meterUrl: meter.url, calls: meter.calls, timeoutMin });
        }
      }
    }
  } finally {
    await meter.stop();
  }
}

await main();
