#!/usr/bin/env tsx
import { spawnSync } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import type { HostMsg } from '@acpira/shared/protocol';
import type { PermissionBlock, QuestionBlock, SessionView } from '@acpira/shared/transcript';
import { FAKE, bundleFakeAgent } from '../../../test/fakeAgentBundle';
import { Host, type View } from '../../lib/host';
import { changes, startFx, TASKS, type ArmId, type Ctx, type Group, type Probe, type RaceTask } from './tasks';

// The extension race (docs/dev/evals.md, Extension race): one extension task built several ways (a plugin, an MCP
// server, a project hook, a source change), each against a pinned engine, then checked through the real engine with
// the fake agent, so the verdict costs no model call. `--dry` puts each arm's reference build in place instead of a
// model and exercises the whole path; a real run hands the task's prompt to `--agent` (spends model calls).
//
//   tsx scripts/eval/extend/run.ts --dry [--task fx-tool,lock-gate] [--arm plugin,mcp] [--group plugins]
//       [--engine-today PATH] [--engine-plugins PATH] [--reps N] [--out DIR] [--with-source] [--today-rev REV]
//
// Groups: `today` is the engine before plugins (`--engine-today`, a build of `--today-rev`, default 4fd49f76), `plugins`
// this tree's (`--engine-plugins`, default the workspace debug build). A group without an engine is skipped. Source arms
// need `--with-source` (a `git archive` of the group's revision, then that tree's build) and have no dry reference yet

const argv = process.argv.slice(2);
const arg = (name: string) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 ? argv[i + 1] : undefined;
};
const list = (name: string) => arg(name)?.split(',').filter(Boolean);
const DRY = argv.includes('--dry');
const AGENT = arg('agent');
const REPS = Number(arg('reps') ?? 1);
const WITH_SOURCE = argv.includes('--with-source');
const TODAY_REV = arg('today-rev') ?? '4fd49f76';
const ROOT = resolve(new URL('../../..', import.meta.url).pathname);
const OUT = resolve(arg('out') ?? join(ROOT, 'output', 'eval-extend', new Date().toISOString().replace(/[:.]/g, '-')));
// The time a real build gets, and how long a check waits for a reply
const BUILD_MIN = Number(arg('timeout-min') ?? 30);
const CHECK_MS = 90_000;

const ENGINES: Record<Group, string | undefined> = {
  today: arg('engine-today'),
  plugins: arg('engine-plugins') ?? process.env.ACPIRA_SIDECAR_BIN ?? join(ROOT, 'rust', 'target', 'debug', 'acpira'),
};
const REVS: Record<Group, string> = { today: TODAY_REV, plugins: 'HEAD' };

if (!DRY && !AGENT) throw new Error('a real run needs --agent <id> (it spends model calls); --dry runs the reference builds');

interface Result {
  task: string; arm: ArmId; group: Group; rep: number; dry: boolean;
  // Build: the agent's turn (or the reference) until its end; cards answered and the time they stayed open
  buildMs: number; cards: number; questions: number; cardOpenMs: number;
  // Source arms: the rebuild after the turn
  rebuild?: { ms: number; ok: boolean };
  // Prompt to the first passing check (build + rebuild + check)
  firstUseMs?: number;
  pass: boolean; detail: string;
  files: number; lines: number;
  // Plugin arms: switched off on the page in one step, the check must fail again
  rollback?: { pass: boolean; detail: string };
  // Arms both groups offer: the same build checked against the other group's engine
  other?: { group: Group; pass: boolean; detail: string };
  error?: string;
}

// Permission cards are allowed once (a card leaves its turn once answered); a plugin's activation card is answered `enable` (the user's click), any other
// question skipped. Each card's open time is kept: what a user would sit through is the count, the bench adds no wait
function approve(view: View) {
  const seen = new Map<string, number>();
  const done = new Set<string>();
  const n = { cards: 0, questions: 0, openMs: 0 };
  const click = (id: string, post: () => void) => {
    if (done.has(id)) return;
    done.add(id);
    n.openMs += Date.now() - (seen.get(id) ?? Date.now());
    post();
  };
  const scan = (v: SessionView) => {
    for (const t of v.turns) {
      if (t.role !== 'agent') continue;
      for (const b of t.blocks) {
        if (b.type === 'permission') {
          const p = b as PermissionBlock;
          if (!seen.has(p.id)) seen.set(p.id, Date.now());
          const opt = p.options.find(x => x.kind === 'allow_once') ?? p.options.find(x => x.kind === 'allow_always') ?? p.options[0];
          if (opt) click(p.id, () => { n.cards++; view.post({ type: 'permission', sessionId: v.id, blockId: p.id, optionId: opt.id }); });
        } else if (b.type === 'question') {
          const q = b as QuestionBlock;
          if (q.outcome) continue;
          if (!seen.has(q.id)) seen.set(q.id, Date.now());
          const plugin = q.questions.some(x => x.id === 'plugin');
          click(q.id, () => {
            n.questions++;
            view.post(plugin
              ? { type: 'answer', sessionId: v.id, blockId: q.id, answers: { plugin: 'enable' } }
              : { type: 'answer', sessionId: v.id, blockId: q.id, answers: {}, skip: true });
          });
        }
      }
    }
  };
  const timer = setInterval(() => { const v = view.active(); if (v) scan(v); }, 200);
  return { counts: n, stop: () => clearInterval(timer) };
}

// The text of the last agent turn
function reply(v: SessionView | undefined): string {
  const last = [...(v?.turns ?? [])].reverse().find(t => t.role === 'agent');
  if (!last || last.role !== 'agent') return '';
  return last.blocks.filter(b => b.type === 'text').map(b => (b as { markdown: string }).markdown).join('');
}

// One engine over the run's homes: the fake agent is always registered, so checks never reach a model
async function engine(group: Group, c: Ctx, bin?: string) {
  process.env.ACPIRA_SIDECAR_BIN = bin ?? ENGINES[group]!;
  const host = await Host.start({
    cwd: c.project, home: c.acpiraHome, defaultAgent: (AGENT ?? 'fake') as never,
    agents: { fake: { name: 'Fake', command: process.execPath, args: [FAKE] } },
    env: { HOME: c.home },
  });
  let theme: string | undefined;
  host.shell.onMessage(m => {
    if (m.type !== 'hostMessage') return;
    const msg = m.message as HostMsg;
    if (msg.type === 'pluginTheme') theme = msg.css;
  });
  const view = await host.view();
  const probe: Probe = {
    async ask(prompt) {
      await view.newSession('fake' as never);
      const approver = approve(view);
      try {
        await view.handle({ type: 'send', text: prompt }, CHECK_MS);
      } finally {
        approver.stop();
      }
      return reply(view.active());
    },
    theme: () => theme,
  };
  return { host, view, probe };
}

const sh = (cmd: string, cwd: string) => spawnSync('/bin/sh', ['-c', cmd], { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });

// The run's homes: HOME, the engine's data root, a project in git (its diff is the run's footprint), the rate service
function prepare(dir: string, fxUrl: string, source: boolean, group: Group): Ctx {
  const c: Ctx = { home: join(dir, 'home'), acpiraHome: join(dir, 'home', '.acpira'), project: join(dir, 'project'), fxUrl };
  mkdirSync(join(c.acpiraHome, 'plugins'), { recursive: true });
  mkdirSync(c.project, { recursive: true });
  writeFileSync(join(c.project, 'README.md'), '# demo\n');
  // The plugins folder is a repository too, its bookkeeping left out, so the footprint counts the plugin's own files
  writeFileSync(join(c.acpiraHome, 'plugins', '.keep'), '');
  for (const d of [c.project, join(c.acpiraHome, 'plugins')]) {
    sh('git init -q && git add -A && git -c user.name=bench -c user.email=bench@localhost commit -qm base', d);
  }
  writeFileSync(join(c.acpiraHome, 'plugins', '.git', 'info', 'exclude'), '.*\nstate.json\nchanges.jsonl\n');
  if (source) {
    c.source = join(dir, 'source');
    mkdirSync(c.source);
    const r = sh(`git -C "${ROOT}" archive ${REVS[group]} | tar -x -C "${c.source}" && git init -q && git add -A && git -c user.name=bench -c user.email=bench@localhost commit -qm base`, c.source);
    if (r.status !== 0) throw new Error(`source tree of ${REVS[group]}: ${r.stderr}`);
  }
  return c;
}

const other = (g: Group): Group => (g === 'today' ? 'plugins' : 'today');

async function runOne(task: RaceTask, armId: ArmId, group: Group, rep: number, fxUrl: string): Promise<Result> {
  const arm = task.arms.find(a => a.id === armId)!;
  const dir = join(OUT, 'runs', `${task.id}.${armId}.${group}.${rep}`);
  mkdirSync(dir, { recursive: true });
  const c = prepare(dir, fxUrl, armId === 'source', group);
  const r: Result = { task: task.id, arm: armId, group, rep, dry: DRY, buildMs: 0, cards: 0, questions: 0, cardOpenMs: 0, pass: false, detail: '', files: 0, lines: 0 };
  const t0 = Date.now();
  let e = await engine(group, c);
  try {
    // Build: the reference's files plus its prompt, or the task's prompt to the agent under test
    const prompt = DRY ? arm.reference!(c) : arm.prompt(c);
    await e.view.newSession((DRY ? 'fake' : AGENT) as never);
    const approver = approve(e.view);
    try {
      await e.view.handle({ type: 'send', text: prompt }, BUILD_MIN * 60_000);
    } finally {
      approver.stop();
    }
    Object.assign(r, { buildMs: Date.now() - t0, cards: approver.counts.cards, questions: approver.counts.questions, cardOpenMs: approver.counts.openMs });
    let checkBin: string | undefined;
    if (arm.build && c.source) {
      const b0 = Date.now();
      const b = sh(arm.build, c.source);
      r.rebuild = { ms: Date.now() - b0, ok: b.status === 0 };
      // A change to the engine is checked against the tree's own binary
      if (task.id === 'fx-tool') checkBin = join(c.source, 'rust', 'target', 'debug', 'acpira');
    }
    if (checkBin) {
      await e.host.dispose();
      e = await engine(group, c, checkBin);
    }
    // The theme layer arrives on its own push after the activation
    if (task.id === 'warm-theme' && armId === 'plugin') await e.view.until(() => !!e.probe.theme(), 5000, 'the theme layer').catch(() => {});
    // The footprint is the build's alone: a check writes files of its own
    Object.assign(r, changes([c.project, join(c.acpiraHome, 'plugins'), ...(c.source ? [c.source] : [])]));
    const v = await task.accept(c, armId, e.probe);
    Object.assign(r, v);
    if (v.pass) r.firstUseMs = Date.now() - t0;
    if (arm.rollback === 'plugin') {
      const id = { 'fx-tool': 'fx', 'warm-theme': 'warm', 'lock-gate': 'lockgate' }[task.id]!;
      const off = sh(`"${process.env.ACPIRA_SIDECAR_BIN}" plugin disable ${id} --home "${c.acpiraHome}"`, c.project);
      // The engine sees the state on its next poll
      await new Promise(res => setTimeout(res, 2500));
      const again = await task.accept(c, armId, e.probe);
      r.rollback = { pass: off.status === 0 && !again.pass, detail: off.status === 0 ? again.detail : off.stderr.trim() };
    }
    if (arm.groups.includes(other(group)) && ENGINES[other(group)] && !checkBin) {
      await e.host.dispose();
      e = await engine(other(group), c);
      const v2 = await task.accept(c, armId, e.probe);
      r.other = { group: other(group), ...v2 };
    }
  } catch (err) {
    r.error = String(err);
  } finally {
    await e.host.dispose();
  }
  return r;
}

async function main() {
  const tasks = TASKS.filter(t => !list('task') || list('task')!.includes(t.id));
  const arms = list('arm') as ArmId[] | undefined;
  const groups = (list('group') as Group[] | undefined) ?? ['today', 'plugins'];
  bundleFakeAgent();
  mkdirSync(OUT, { recursive: true });
  const fx = await startFx();
  const results: Result[] = [];
  const skipped: string[] = [];
  try {
    for (const task of tasks) for (const arm of task.arms) for (const group of arm.groups) {
      if (arms && !arms.includes(arm.id)) continue;
      if (!groups.includes(group)) continue;
      const name = `${task.id}/${arm.id}@${group}`;
      if (!ENGINES[group] || !existsSync(ENGINES[group]!)) { skipped.push(`${name}: no ${group} engine`); continue; }
      if (arm.id === 'source' && !WITH_SOURCE) { skipped.push(`${name}: needs --with-source`); continue; }
      if (DRY && !arm.reference) { skipped.push(`${name}: no dry reference`); continue; }
      for (let rep = 1; rep <= REPS; rep++) {
        const r = await runOne(task, arm.id, group, rep, fx.url);
        results.push(r);
        appendFileSync(join(OUT, 'results.jsonl'), `${JSON.stringify(r)}\n`);
        const extra = [r.rollback && `rollback ${r.rollback.pass ? 'ok' : 'FAILED'}`, r.other && `on ${r.other.group} ${r.other.pass ? 'pass' : 'fail'}`].filter(Boolean).join(', ');
        console.log(`${r.pass ? 'PASS' : 'FAIL'} ${name}#${rep} build ${r.buildMs} ms, ${r.cards} cards, ${r.questions} questions, ${r.files} files / ${r.lines} lines${extra ? `, ${extra}` : ''}: ${r.error ?? r.detail}`);
      }
    }
  } finally {
    fx.server.close();
  }
  for (const s of skipped) console.log(`skip ${s}`);
  console.log(`results in ${OUT}${DRY ? ' (dry)' : ''}`);
  // A dry run is the bench's own test: every reference must pass, roll back and carry over
  if (DRY && results.some(r => !r.pass || r.error || (r.rollback && !r.rollback.pass) || (r.other && !r.other.pass))) process.exitCode = 1;
}

await main();
