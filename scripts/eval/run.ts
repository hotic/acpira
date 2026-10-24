import { spawnSync } from 'node:child_process';
import { appendFileSync, cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import type { AgentTurn, PermissionBlock, QuestionBlock, SessionView } from '@acpira/shared/transcript';
import { FAKE, bundleFakeAgent } from '../../test/fakeAgentBundle';
import { Host, type View } from '../lib/host';
import { CONFIGS, type Config, nativeHelpers, setupHelpers } from './configs';
import { HARNESSES, type HarnessSetup, MODELS } from './harnesses';
import { deepseekKey, gatewayKey } from './keys';
import { type Call, OUTAGE, startMeter } from './meter';
import { type QuotaSnap, claudeUsage, quotaSpent, readQuota } from './native';
import { cost } from './prices';

// Harness comparison on long tasks (docs/dev/builtin-agent.md, "Harness comparison") and the Fusion comparison
// (docs/dev/evals.md, Fusion bench). Spends real model calls, and subscription quota for native-login configurations.
//
//   pnpm exec tsx scripts/eval/run.ts --model glm-5.3 --harness acpira,opencode,pi
//     [--task calc-lang,conf-bugs] [--reps 1] [--out DIR] [--timeout-min N] [--dump]
//   pnpm exec tsx scripts/eval/run.ts --config claude-opus,devin-opus-luna,acpira-orch
//     [--task …] [--reps 1] [--out DIR] [--timeout-min N] [--quota-floor claude.weekly=0.25,…] [--dry]
//
// Each run: a fresh copy of the task's repo (git initialised), a fresh HOME and sidecar home, the harness pointed at the
// metering proxy (or at its own login), one prompt through the real sidecar with every permission card allowed (the
// summoned helpers' too), the background work it started waited out, then the hidden tests. One line per run goes to
// DIR/results.ndjson and one per model call to DIR/calls.ndjson; work trees stay under DIR/runs. A configuration
// (configs.ts) adds session picks, helpers and their usage, and the quota its native logins spent. --dry swaps every
// agent for the fake one (a configuration's lead summons its helpers once) and spends nothing.
// ACPIRA_SIDECAR_BIN pins the engine under test (otherwise the workspace's debug build, rebuilt before every run)

const TASKS_DIR = resolve(import.meta.dirname, 'tasks');
const REPO = resolve(import.meta.dirname, '../..');
// A Rust task's hidden tests build first and bound each target themselves (rust-tests.sh)
const VERIFY_TIMEOUT_MS = 900_000;
const DRY = process.argv.includes('--dry');
// Background work counts as over once the session has stayed idle this long with no summoned round running: the
// automatic message delivering the last round starts within it
const QUIET_MS = 5000;

function arg(name: string, fallback?: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
}

// A task's starting tree is its `repo/` folder, or with `base` a revision of this repository (a task from its own
// history): `git archive` of it, so no later history comes along, then the task's `patch` (test scaffolding the hidden
// tests need) and the files under its `add/` (the task's own ISSUES.md or TASK.md). Its hidden tests are its `hidden/`
// folder, or with `hidden` those paths as the reference change left them, under `.eval_hidden/` at their own paths.
// `env`: set for the agent and the hidden tests on top of the run's HOME, for a task that needs the user's own toolchains
// (a Rust task's RUSTUP_HOME / CARGO_HOME); a value starting with `~/` is under the real home, since HOME is the run's
interface Task {
  id: string; kind: string; prompt: string; verify: string; timeoutMin: number; env?: Record<string, string>;
  base?: string; patch?: string; hidden?: { rev: string; paths: string[] };
}

const taskEnv = (t: Task): Record<string, string> =>
  Object.fromEntries(Object.entries(t.env ?? {}).map(([k, v]) => [k, v.startsWith('~/') ? join(homedir(), v.slice(2)) : v]));

function loadTask(id: string): Task {
  return JSON.parse(readFileSync(join(TASKS_DIR, id, 'task.json'), 'utf8')) as Task;
}

function sh(cmd: string, cwd: string, timeout = 60_000, env: Record<string, string> = {}) {
  return spawnSync('bash', ['-c', cmd], { cwd, timeout, encoding: 'utf8', env: { ...process.env, ...env }, maxBuffer: 64 << 20 });
}

function must(cmd: string, cwd: string) {
  const r = sh(cmd, cwd);
  if (r.status !== 0) throw new Error(`${cmd}: ${(r.stderr ?? '').trim().slice(-500)}`);
}

// `git archive` of a revision's paths (all of it without any) unpacked into a folder
const unpack = (rev: string, into: string, paths: string[] = []) =>
  must(`git -C '${REPO}' archive --format=tar '${rev}' ${paths.map(p => `'${p}'`).join(' ')} | tar -x -C '${into}'`, into);

function materialize(t: Task, work: string) {
  const dir = join(TASKS_DIR, t.id);
  if (!t.base) return cpSync(join(dir, 'repo'), work, { recursive: true });
  unpack(t.base, work);
  if (t.patch) must(`git apply '${join(dir, t.patch)}'`, work);
  if (existsSync(join(dir, 'add'))) cpSync(join(dir, 'add'), work, { recursive: true });
}

function hideTests(t: Task, work: string) {
  const into = join(work, '.eval_hidden');
  mkdirSync(into, { recursive: true });
  if (t.hidden) unpack(t.hidden.rev, into, t.hidden.paths);
  else cpSync(join(TASKS_DIR, t.id, 'hidden'), into, { recursive: true });
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

// Python unittest's summary, or the sum of cargo test's per-binary `test result:` lines (a build that fails has none)
function testCounts(out: string): { total: number; passed: number } {
  const cargo = [...out.matchAll(/^test result: \w+\. (\d+) passed; (\d+) failed;/gm)];
  if (cargo.length) {
    const passed = cargo.reduce((a, m) => a + Number(m[1]), 0);
    return { total: passed + cargo.reduce((a, m) => a + Number(m[2]), 0), passed };
  }
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

interface NodeOutcome { persona?: string; agent?: string; role?: string; state: string; tools: number; wallS?: number }

interface Outcome {
  timedOut: boolean; failure?: string; status?: string; stop?: string; error?: unknown;
  tools: number; cards: number; questions: number; harnessUsage?: unknown;
  // Delegated children: summoned rounds (persona, CLI) and the harness's own subagents (role)
  subagents?: NodeOutcome[];
  // Automatic turns after the prompt's own (deliveries of summoned rounds, compaction, retries)
  autoTurns?: number;
  // Token counts the harness reported itself, for one that bypasses the meter
  usage?: { requests: number; input: number; cacheRead: number; output: number; reasoning: number };
}

// A harness that is its own CLI (no ACP), run once in the work tree with the prompt; its stdout is kept as cli.out
function driveCli(cli: NonNullable<HarnessSetup['cli']>, o: { runId: string; dir: string; work: string; home: string; prompt: string; timeoutMin: number; env: Record<string, string> }): Outcome {
  console.log(`[${o.runId}] cli ${cli.cmd[0]}`);
  const r = spawnSync(cli.cmd[0]!, [...cli.cmd.slice(1), o.prompt], {
    cwd: o.work, encoding: 'utf8', timeout: o.timeoutMin * 60_000, killSignal: 'SIGTERM', maxBuffer: 256 << 20,
    env: { ...process.env, HOME: o.home, PYTHONDONTWRITEBYTECODE: '1', ...o.env, ...cli.env },
  });
  writeFileSync(join(o.dir, 'cli.out'), r.stdout ?? '');
  writeFileSync(join(o.dir, 'cli.err'), r.stderr ?? '');
  const timedOut = r.error?.message.includes('ETIMEDOUT') ?? false;
  const failure = r.error ? String(r.error) : r.status !== 0 ? `exit ${r.status}: ${(r.stderr ?? '').trim().slice(-300)}` : undefined;
  // agy --output-format json: { status, num_turns, usage: { input_tokens (uncached), cache_read_tokens, output_tokens
  // (thinking included), thinking_tokens } } (agy 1.3.5, 2026-10-15)
  let usage: Outcome['usage'];
  try {
    const j = JSON.parse(r.stdout ?? '') as { num_turns?: number; usage?: Record<string, number> };
    const u = j.usage ?? {};
    usage = { requests: j.num_turns ?? 0, input: (u.input_tokens ?? 0) + (u.cache_read_tokens ?? 0), cacheRead: u.cache_read_tokens ?? 0, output: u.output_tokens ?? 0, reasoning: u.thinking_tokens ?? 0 };
  } catch {
    usage = undefined;
  }
  return { timedOut, failure, stop: timedOut ? 'timeout' : r.status === 0 ? 'end_turn' : 'error', tools: 0, cards: 0, questions: 0, usage };
}

// Every permission is allowed once, the root's and the summoned children's alike; a question is skipped (the prompt
// asks for autonomous work). A card leaves its turn once answered, so each id is clicked only once
function approve(view: View) {
  const clicked = new Set<string>();
  const n = { cards: 0, questions: 0 };
  const allow = (sessionId: string, p: PermissionBlock) => {
    if (clicked.has(p.id)) return;
    const opt = p.options.find(x => x.kind === 'allow_once') ?? p.options.find(x => x.kind === 'allow_always') ?? p.options[0];
    if (!opt) return;
    clicked.add(p.id);
    n.cards++;
    view.post({ type: 'permission', sessionId, blockId: p.id, optionId: opt.id });
  };
  const skip = (sessionId: string, q: QuestionBlock) => {
    if (clicked.has(q.id) || q.outcome) return;
    clicked.add(q.id);
    n.questions++;
    view.post({ type: 'answer', sessionId, blockId: q.id, answers: {}, skip: true });
  };
  const timer = setInterval(() => {
    const v = view.active();
    if (!v) return;
    for (const b of lastAgent(v)?.blocks ?? []) {
      if (b.type === 'permission') allow(v.id, b as PermissionBlock);
      else if (b.type === 'question') skip(v.id, b as QuestionBlock);
    }
    for (const c of v.subagents ?? []) {
      for (const p of c.permissions ?? []) allow(v.id, p);
      if (c.question) skip(v.id, c.question);
    }
  }, 200);
  return { counts: n, stop: () => clearInterval(timer) };
}

// --quota-floor agent.window=fraction,…: no run starts on a native login whose window has less left (the matrix stops
// there and resumes later), so a configuration cannot use up the quota the user's own work needs
const FLOORS = (arg('quota-floor') ?? '').split(',').filter(Boolean).map(f => {
  const m = /^(\w+)\.(\w+)=([\d.]+)$/.exec(f);
  if (!m) throw new Error(`--quota-floor ${f}: expected agent.window=fraction`);
  return { agent: m[1]!, window: m[2]!, min: Number(m[3]) };
});

class FloorReached extends Error {}

function belowFloor(q: QuotaSnap): string | undefined {
  for (const f of FLOORS) {
    const w = q.agents[f.agent]?.windows.find(x => x.id === f.window);
    if (w && w.remaining < f.min) return `${f.agent} ${f.window} has ${Math.round(w.remaining * 100)}% left, under the floor of ${Math.round(f.min * 100)}%`;
  }
  return undefined;
}

const scrub = (setup: HarnessSetup) => {
  for (const f of setup.scrub ?? []) rmSync(f, { force: true });
};

// Sets each picked session option (by config id, else category) and waits until the session shows it. A value the
// option does not offer fails the run: a run on another model says nothing about the configuration
async function applyPicks(view: View, picks: Record<string, string>) {
  for (const [key, value] of Object.entries(picks)) {
    // A new session's options arrive after the view shows it (the agent answers session/new first), so wait for this
    // one, and for the value itself: one Devin session came up without the Fusion pair it was asked for, which a probe a
    // minute later listed again (Devin 3000.11.92, 2026-10-21)
    const find = () => {
      const options = view.active()?.controls.options ?? [];
      return options.find(x => x.id === key) ?? options.find(x => x.category === key);
    };
    await view.until(() => !!find()?.options.some(x => x.id === value), 60_000, `session option ${key} offering ${value}`).catch(() => {});
    const v = view.active()!;
    const opt = find();
    if (!opt) throw new Error(`no session option ${key} (have ${v.controls.options.map(x => x.id).join(', ')})`);
    if (!opt.options.some(x => x.id === value)) throw new Error(`${opt.id} offers no ${value}`);
    if (opt.value === value) continue;
    view.post({ type: 'setConfig', sessionId: v.id, configId: opt.id, value });
    await view.until(() => view.active()?.controls.options.find(x => x.id === opt.id)?.value === value, 60_000, `${opt.id} = ${value}`);
  }
}

// One prompt through the real sidecar, every permission card allowed, then whatever it left running in the
// background (summoned rounds and the automatic messages delivering them) waited out within the same time limit
async function driveSidecar(setup: HarnessSetup, o: {
  runId: string; work: string; home: string; acpiraHome: string; prompt: string; timeoutMin: number;
  picks: Record<string, string>; agents?: Record<string, unknown>; env: Record<string, string>;
}): Promise<Outcome> {
  const { work, home, acpiraHome } = o;
  const host = await Host.start({
    cwd: work, home: acpiraHome, defaultAgent: setup.agent, agents: o.agents,
    env: { HOME: home, PYTHONDONTWRITEBYTECODE: '1', ...o.env, ...setup.env },
  });
  const view = await host.view();
  const approver = approve(view);
  const deadline = Date.now() + o.timeoutMin * 60_000;
  let timedOut = false;
  let failure: string | undefined;
  let firstTurn = 0;
  try {
    if (setup.importLogin && !DRY) {
      const a = await view.addAccount(setup.agent, 'import').finally(() => scrub(setup));
      if (a.status !== 'success') throw new Error(`importing the ${setup.agent} login: ${a.status} ${a.error ?? ''}`);
    }
    await view.newSession(setup.agent);
    if (!DRY) await applyPicks(view, o.picks);
    const v0 = view.active()!;
    const shown = v0.controls.options.filter(x => x.category === 'model' || x.id in o.picks).map(x => `${x.id}=${x.value}`);
    console.log(`[${o.runId}] session ${v0.id} status=${v0.status} ${shown.join(' ')}`);
    firstTurn = v0.turns.length;
    try {
      await view.handle({ type: 'send', text: o.prompt }, deadline - Date.now());
      let quiet = 0;
      await view.until(() => {
        const v = view.active();
        const busy = !v || v.running || (v.queued?.length ?? 0) > 0 || (v.subagents ?? []).some(c => c.harness && c.state === 'running');
        if (busy) {
          quiet = 0;
          return false;
        }
        quiet ||= Date.now();
        return Date.now() - quiet >= QUIET_MS;
      }, Math.max(1, deadline - Date.now()), 'the background work');
    } catch (e) {
      timedOut = true;
      failure = String(e);
      const v = view.active();
      for (const c of (v?.subagents ?? []).filter(x => x.state === 'running')) view.post({ type: 'cancelSubagent', sessionId: v!.id, subagentId: c.id });
      view.post({ type: 'stop', sessionId: view.activeId });
      await view.until(() => !view.active()?.running, 30_000, 'the stopped turn').catch(() => {});
    }
  } catch (e) {
    failure = String(e);
  }
  approver.stop();
  const v = view.active();
  const turns = v?.turns.slice(firstTurn) ?? [];
  const agentTurns = turns.filter((t): t is AgentTurn => t.role === 'agent');
  const tools = agentTurns.reduce((a, t) => a + t.blocks.filter(b => b.type === 'tool_call').length, 0);
  const turn = lastAgent(v);
  const subagents = (v?.subagents ?? []).map(c => ({
    ...(c.harness ? { persona: c.harness.persona, agent: c.harness.agent } : { role: c.role }),
    state: c.state, tools: c.toolCount,
    ...(c.endedAt ? { wallS: Math.round((c.endedAt - c.announcedAt) / 1000) } : {}),
  }));
  const autoTurns = turns.filter(t => t.role === 'user' && t.auto).length;
  writeFileSync(join(o.work, '..', 'session.json'), JSON.stringify(v ?? null, null, 2));
  await host.dispose().catch(() => {});
  return {
    timedOut, failure, status: v?.status, stop: turn?.stop, error: turn?.error ?? v?.error, tools,
    cards: approver.counts.cards, questions: approver.counts.questions, harnessUsage: v?.usage as unknown,
    ...(subagents.length ? { subagents } : {}), ...(autoTurns ? { autoTurns } : {}),
  };
}

// One column of the matrix: a harness on a model (the harness comparison), or a named configuration
interface Cell { name: string; harness: string; model: string; config?: Config }

const runIdOf = (task: string, c: Cell, rep: number) =>
  (c.config ? `${task}.${c.name}.${rep}` : `${task}.${c.harness}.${c.model}.${rep}`).replace(/[^\w.-]/g, '_');

// What a dry run's lead does: summon the configuration's first group (else its first helper) once, so the roster, the
// relay and the delivery are all exercised; a lead without helpers just answers
function dryPrompt(c: Cell): string {
  const target = c.config?.groups?.[0]?.id ?? c.config?.helpers?.[0]?.id;
  return target ? `mcp-call:ask_agent ${JSON.stringify({ agent: target, prompt: 'relay: dry run', title: 'Dry run' })}` : 'hello';
}

const sumCalls = (calls: Call[]) => ({
  requests: calls.length,
  input: calls.reduce((a, c) => a + c.input, 0),
  cacheRead: calls.reduce((a, c) => a + c.cacheRead, 0),
  cacheWrite: calls.reduce((a, c) => a + c.cacheWrite, 0),
  output: calls.reduce((a, c) => a + c.output, 0),
  reasoning: calls.reduce((a, c) => a + c.reasoning, 0),
});

// The quota a native login spends, read around each run that uses one (the agents its lead and its helpers run in)
const nativeAgents = (setup: HarnessSetup, c?: Config) => [...new Set([
  ...(setup.unmetered && !setup.cli && ['claude', 'devin'].includes(setup.agent) ? [setup.agent] : []),
  ...nativeHelpers(c).map(h => h.agent),
])];

async function runOne(o: { task: Task; cell: Cell; rep: number; root: string; meterUrl: string; calls: Call[]; timeoutMin: number }) {
  const { cell } = o;
  const runId = runIdOf(o.task.id, cell, o.rep);
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
  materialize(o.task, work);
  sh('git init -q && git add -A && git -c user.name=eval -c user.email=eval@localhost commit -qm "Initial state"', work);

  const spec = MODELS[cell.model]!;
  const upstream = spec.upstream ?? 'gw';
  const setup = HARNESSES[cell.harness]!({ model: spec, base: `${o.meterUrl}/r/${runId}/${upstream}`, home, acpiraHome });
  const helped = cell.config ? setupHelpers(cell.config, { home, acpiraHome, baseFor: id => `${o.meterUrl}/r/${runId}.${id}/gw` }) : { picks: {}, env: {} };
  const picks = { ...cell.config?.pick, ...helped.picks };
  // A dry run's agents, lead and helpers alike, are the fake one under their own ids
  const agents = DRY ? Object.fromEntries([setup.agent, ...(cell.config?.helpers ?? []).map(h => h.agent)].map(id => [id, { name: `Fake ${id}`, command: process.execPath, args: [FAKE] }])) : undefined;
  const natives = DRY ? [] : nativeAgents(setup, cell.config);
  const quotaBefore = natives.length ? readQuota(natives) : undefined;
  const low = quotaBefore && belowFloor(quotaBefore);
  if (low) throw new FloorReached(`${runId} not started: ${low}`);
  const started = Date.now();
  const prompt = DRY ? dryPrompt(cell) : o.task.prompt;
  const env = taskEnv(o.task);
  const outcome = setup.cli
    ? driveCli(setup.cli, { runId, dir, work, home, prompt, timeoutMin: o.timeoutMin, env })
    : await driveSidecar(setup, { runId, work, home, acpiraHome, prompt, timeoutMin: o.timeoutMin, picks, agents, env: { ...env, ...helped.env } });
  const { timedOut, failure, tools } = outcome;
  const wallS = Math.round((Date.now() - started) / 1000);
  const quotaAfter = natives.length ? readQuota(natives) : undefined;
  // The run's sidecar home held an imported login's key: it goes once the run is over, with any copy the import read
  if (setup.importLogin) rmSync(join(acpiraHome, 'secrets.json'), { force: true });
  scrub(setup);

  // The hidden tests, copied in only now so the agent never saw them
  hideTests(o.task, work);
  const ver = await verifyLock(o.root, () => sh(o.task.verify, work, VERIFY_TIMEOUT_MS, { HOME: home, PYTHONDONTWRITEBYTECODE: '1', EVAL_DIR: import.meta.dirname, ...env }));
  const out = `${ver.stdout ?? ''}\n${ver.stderr ?? ''}`;
  writeFileSync(join(dir, 'verify.txt'), out);
  const counts = testCounts(out);
  const verifyTimedOut = ver.error?.message.includes('ETIMEDOUT') ?? false;

  const mine = o.calls.filter(c => c.run === runId);
  const lead = sumCalls(mine);
  // Claude Code on its own login: the tokens its transcripts recorded stand in for the meter's
  const native = setup.agent === 'claude' && setup.unmetered && !DRY ? claudeUsage(home) : undefined;
  // A Claude helper's tokens are its own transcripts under the run's HOME, which holds no other Claude session (no
  // Claude lead beside a Claude helper, setupHelpers)
  const nativeIds = new Set(nativeHelpers(cell.config).map(h => h.id));
  const helpers = Object.fromEntries([...(cell.config?.helpers ?? []).map(h => [h.id, h.model] as const), ...(cell.config?.sidekick ? [['sidekick', cell.config.sidekick] as const] : [])]
    .map(([id, model]) => {
      const u = nativeIds.has(id) && !DRY ? { ...claudeUsage(home), reasoning: 0, nativeUsage: true } : sumCalls(o.calls.filter(c => c.run === `${runId}.${id}`));
      return [id, { ...u, costUsd: Math.round(cost({ model, ...u }) * 1000) / 1000 }];
    })
    .filter(([, u]) => (u as { requests: number }).requests > 0));
  const result = {
    runId, task: o.task.id, harness: cell.harness, model: cell.model, ...(cell.config ? { config: cell.name } : {}), rep: o.rep,
    pass: ver.status === 0 && counts.total > 0, testsPassed: counts.passed, testsTotal: counts.total, verifyTimedOut,
    wallS, timedOut, failure, status: outcome.status, stop: outcome.stop, error: outcome.error,
    tools, cards: outcome.cards, questions: outcome.questions,
    callErrors: mine.filter(c => c.error).length,
    ...lead,
    ...(native ? { ...native, reasoning: 0, nativeUsage: true } : {}),
    ...(outcome.usage ? { ...outcome.usage, cacheWrite: 0 } : {}),
    harnessUsage: outcome.harnessUsage,
    ...(setup.unmetered && !outcome.usage && !native ? { unmetered: true } : {}),
    ...(Object.keys(helpers).length ? { helpers } : {}),
    ...(outcome.subagents ? { subagents: outcome.subagents } : {}),
    ...(outcome.autoTurns ? { autoTurns: outcome.autoTurns } : {}),
    ...(quotaBefore && quotaAfter ? { quota: quotaSpent(quotaBefore, quotaAfter), quotaAt: { before: quotaBefore, after: quotaAfter } } : {}),
    ...(DRY ? { dry: true } : {}),
  };
  const outage = !result.pass && o.calls.some(c => (c.run === runId || c.run.startsWith(`${runId}.`)) && c.error && OUTAGE.test(c.error));
  appendFileSync(join(o.root, outage ? 'infra.ndjson' : 'results.ndjson'), JSON.stringify(result) + '\n');
  if (outage) console.log(`[${runId}] gateway outage: recorded in infra.ndjson, not counted`);
  console.log(`[${runId}] pass=${result.pass} tests=${counts.passed}/${counts.total} wall=${wallS}s requests=${result.requests} in=${result.input} cached=${result.cacheRead} out=${result.output} stop=${result.stop}${outcome.subagents ? ` subagents=${outcome.subagents.length}` : ''}${result.quota ? ` quota=${JSON.stringify(result.quota)}` : ''}${failure ? ` failure=${failure}` : ''}`);
  return result;
}

async function main() {
  const model = arg('model');
  const harnesses = (arg('harness') ?? '').split(',').filter(Boolean);
  const configs = (arg('config') ?? '').split(',').filter(Boolean);
  const tasks = (arg('task') ?? readdirSync(TASKS_DIR).filter(d => existsSync(join(TASKS_DIR, d, 'task.json'))).join(',')).split(',');
  const reps = Number(arg('reps', '1'));
  const byHarness = !!model && !!MODELS[model] && harnesses.length > 0 && harnesses.every(h => HARNESSES[h]);
  const byConfig = configs.length > 0 && configs.every(c => CONFIGS[c]);
  if (byHarness === byConfig) {
    console.error(`usage: run.ts --model ${Object.keys(MODELS).join('|')} --harness ${Object.keys(HARNESSES).join(',')} [--task …] [--reps N] [--out DIR]`);
    console.error(`   or: run.ts --config ${Object.keys(CONFIGS).join(',')} [--task …] [--reps N] [--out DIR] [--dry]`);
    process.exit(2);
  }
  const cells: Cell[] = byConfig
    ? configs.map(name => ({ name, harness: CONFIGS[name]!.harness, model: CONFIGS[name]!.model, config: CONFIGS[name] }))
    : harnesses.map(h => ({ name: h, harness: h, model: model! }));
  if (DRY) bundleFakeAgent();
  const root = resolve(arg('out') ?? join('/tmp/acpira-eval', new Date().toISOString().replace(/[:.]/g, '-')));
  mkdirSync(root, { recursive: true });
  // A run already in results.ndjson is skipped, so an interrupted matrix resumes where it stopped
  const resultsFile = join(root, 'results.ndjson');
  const done = new Set(existsSync(resultsFile) ? readFileSync(resultsFile, 'utf8').split('\n').filter(Boolean).map(l => (JSON.parse(l) as { runId: string }).runId) : []);
  // The gateway's key for every lead and helper on it, DeepSeek's own only when a lead runs there
  const keys: Record<string, string> = {};
  if (DRY) keys.gw = 'dry';
  else if (cells.some(c => MODELS[c.model]!.upstream !== 'ds' || c.config?.helpers?.length || c.config?.sidekick)) keys.gw = gatewayKey();
  if (!DRY && cells.some(c => MODELS[c.model]!.upstream === 'ds')) {
    keys.ds = deepseekKey() ?? '';
    if (!keys.ds) throw new Error('no DeepSeek key (EVAL_DS_KEY or EVAL_DS_KEY_FILE)');
  }
  const meter = await startMeter({ log: join(root, 'calls.ndjson'), keys, dump: process.argv.includes('--dump') ? join(root, 'bodies') : undefined });
  console.log(`results in ${root}${DRY ? ' (dry)' : ''}`);
  try {
    for (let rep = 1; rep <= reps; rep++) {
      for (const t of tasks) {
        const task = loadTask(t);
        for (const cell of cells) {
          if (done.has(runIdOf(task.id, cell, rep))) continue;
          const timeoutMin = Number(arg('timeout-min', String(task.timeoutMin)));
          await runOne({ task, cell, rep, root, meterUrl: meter.url, calls: meter.calls, timeoutMin });
        }
      }
    }
  } catch (e) {
    if (!(e instanceof FloorReached)) throw e;
    console.log(`stopped at the quota floor: ${e.message}`);
  } finally {
    await meter.stop();
  }
}

await main();
