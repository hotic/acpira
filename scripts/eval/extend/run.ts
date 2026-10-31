#!/usr/bin/env tsx
import { spawnSync } from 'node:child_process';
import { appendFileSync, chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import type { HostMsg } from '@acpira/shared/protocol';
import type { PluginSlot } from '@acpira/shared/plugins';
import type { AgentTurn, ConfigControl, PermissionBlock, QuestionBlock, SessionOption, SessionView } from '@acpira/shared/transcript';
import { isReasoningControl } from '@acpira/shared/composerControls';
import { FAKE, bundleFakeAgent } from '../../../test/fakeAgentBundle';
import { Host, type View } from '../../lib/host';
import { HARNESSES, MODELS } from '../harnesses';
import { deepseekKey, gatewayKey } from '../keys';
import { type Call, OUTAGE, startMeter } from '../meter';
import { cost } from '../prices';
import { CLAUDE_BIN, ocClaudeLogin, ocConfigDir, ocInstall, ocPrepare, ocServer, ocWrite, ocWriterConfig, OC_VERSION, runTmp } from './opencode';
import { checkJail, jailedBin, setupJail, shortTmp, shortTmpPath } from './jail';
import { installDriver } from './tui';
import { ENGINE_TOOLS, changes, pluginIds, startFx, TASKS, type AcpiraProbe, type ArmId, type Ctx, type Group, type RaceTask, type Verdict } from './tasks';

// The extension race (docs/dev/evals.md, Extension race): one extension task built several ways (an Acpira plugin, an
// MCP server, a project hook, a source change, OpenCode's own ways), each against a pinned harness, then checked
// through the real harness with no model (the fake agent drives Acpira's engine, a scripted model drives OpenCode).
// `--dry` puts each arm's reference build in place instead of a model, which is the bench's own test and the reachable
// surface of each harness; a real run hands the task's prompt to a writer on `--model` (spends model calls).
//
//   tsx scripts/eval/extend/run.ts --dry [--task fx-tool,lock-gate] [--arm plugin,opencode] [--group plugins,opencode]
//   tsx scripts/eval/extend/run.ts --model deepseek-v4.1-flash [--writer acpira] [--repairs 2] [--reps N] [--out DIR] [--work DIR] …
//       [--engine-today PATH] [--engine-plugins PATH] [--with-source] [--today-rev REV] [--timeout-min N]
//
// Groups: `today` is the engine before plugins (`--engine-today`, a build of `--today-rev`, default 4fd49f76), `plugins`
// this tree's (`--engine-plugins`, default the workspace debug build), `opencode` a pinned OpenCode (OC_VERSION, installed
// under ~/.cache/opencode-<version>, OPENCODE_BIN overrides). A group without
// its harness is skipped. Source arms need `--with-source` and have no dry reference.
//
// The writer is the harness extending itself, the way a user asks in a new window: Acpira's built-in agent for Acpira's
// groups (`--writer` picks another harness of harnesses.ts, e.g. claude-max), OpenCode's own `opencode run` for
// OpenCode, outside Acpira so it never sees Acpira's MCP server and its plugin guide. Every check runs first against the
// instance that was running while the writer worked (`effect: hot`), and only when that fails against a fresh one
// (`effect: restart`). A failed check sends its symptom back to the writer, up to `--repairs` more rounds

const argv = process.argv.slice(2);
const arg = (name: string) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 ? argv[i + 1] : undefined;
};
const list = (name: string) => arg(name)?.split(',').filter(Boolean);
const DRY = argv.includes('--dry');
const MODEL = arg('model');
const WRITER = arg('writer') ?? 'acpira';
const REPAIRS = Number(arg('repairs') ?? 2);
const REPS = Number(arg('reps') ?? 1);
// `--rep-from N`: start at rep N, to run again only the reps a broken bench lost
const REP_FROM = Number(arg('rep-from') ?? 1);
// The writers' reasoning level, the same on both harnesses: the highest level each model offers (GPT's `ultra` is a
// subagent mode rather than a level, and Argon stops at high). `--effort` overrides
const TOP_EFFORT: Record<string, string> = {
  'deepseek-v4.1-flash': 'max', 'deepseek-flash': 'max', 'kimi-k3.1-pro': 'max', 'gpt-6.1-sol': 'max', 'gemini-4-argon': 'high',
  'claude-opus-5.5': 'max', 'claude-sonnet-5.5': 'max',
};
// `--level mid`: the middle level each model offers, quicker to watch (DeepSeek and Kimi offer low / high / max)
const MID_EFFORT: Record<string, string> = {
  'deepseek-v4.1-flash': 'high', 'deepseek-flash': 'high', 'kimi-k3.1-pro': 'high', 'gpt-6.1-sol': 'medium', 'gemini-4-argon': 'medium',
  'claude-opus-5.5': 'medium', 'claude-sonnet-5.5': 'medium',
};
const EFFORT = arg('effort') ?? (MODEL ? (arg('level') === 'mid' ? MID_EFFORT : TOP_EFFORT)[MODEL] : undefined);
// `--writer claude-max`: Claude Code on the user's subscription writes on both sides (Acpira's `claude` agent, OpenCode's
// Claude Agent provider), the model picked by its Claude Code alias; unmetered
const CLAUDE_SUB = WRITER === 'claude-max';
const CLAUDE_ALIAS: Record<string, string> = { 'claude-opus-5.5': 'opus', 'claude-sonnet-5.5': 'sonnet' };
// `--dump`: every request body the meter forwards, kept under OUT/bodies (to check what level a writer sent)
const DUMP = argv.includes('--dump');
const WITH_SOURCE = argv.includes('--with-source');
const TODAY_REV = arg('today-rev') ?? '4fd49f76';
const ROOT = resolve(new URL('../../..', import.meta.url).pathname);
const OUT = resolve(arg('out') ?? join(ROOT, 'output', 'eval-extend', new Date().toISOString().replace(/[:.]/g, '-')));
// Where the runs happen: outside this repository, so a writer finds nothing of Acpira's source tree (a user has only the
// installed app). Results and each run's transcript still go to OUT
const WORK = resolve(arg('work') ?? join(tmpdir(), 'acpira-extend', basename(OUT)));
// The time a writer's round gets, and how long a check waits for a reply
const ROUND_MIN = Number(arg('timeout-min') ?? 20);
const CHECK_MS = 90_000;
// How long a running instance gets to pick a change up before the check counts it as needing a restart (OpenCode
// reloads plugins from a file watcher, Acpira on the card's click)
const HOT_WAIT_MS = 15_000;

// Checks the running instance until it passes or the window closes; `ms` is how long the change took to show
async function settle(check: () => Promise<Verdict>): Promise<Verdict & { ms: number }> {
  const t0 = Date.now();
  for (;;) {
    const v = await check();
    if (v.pass || Date.now() - t0 > HOT_WAIT_MS) return { ...v, ms: Date.now() - t0 };
    await new Promise(r => setTimeout(r, 1500));
  }
}

// Resolved now: the bench moves into its work folder before it looks for them
const ENGINES: Record<Exclude<Group, 'opencode'>, string | undefined> = {
  today: arg('engine-today') && resolve(arg('engine-today')!),
  plugins: resolve(arg('engine-plugins') ?? process.env.ACPIRA_SIDECAR_BIN ?? join(ROOT, 'rust', 'target', 'debug', 'acpira')),
};
const REVS: Record<Exclude<Group, 'opencode'>, string> = { today: TODAY_REV, plugins: 'HEAD' };

if (!DRY && !(MODEL && MODELS[MODEL])) throw new Error(`a real run needs --model ${Object.keys(MODELS).join('|')} (it spends model calls); --dry runs the reference builds`);
if (!DRY && !HARNESSES[WRITER]) throw new Error(`--writer ${WRITER}: not a harness of harnesses.ts`);
if (!DRY && !EFFORT) throw new Error(`--effort: no level known for ${MODEL}`);
if (!DRY && CLAUDE_SUB && !CLAUDE_ALIAS[MODEL!]) throw new Error(`--writer claude-max: no Claude Code alias for ${MODEL}`);

// hot: the running instance picked it up by itself; reload: after the harness's own reload command; restart: only in a
// fresh instance
type Effect = 'hot' | 'reload' | 'restart';

// Tasks whose OpenCode check reads the terminal interface
const SCREEN_TASKS = new Set(['warm-theme', 'review-button', 'status-widget']);

interface Round { round: number; ms: number; pass: boolean; effect?: Effect; effectMs?: number; detail: string; symptom?: string; failure?: string }

interface Result {
  task: string; arm: ArmId; group: Group; rep: number; dry: boolean;
  model?: string; writer?: string;
  // The writer's reasoning level: read back from the Acpira session's control, or the OpenCode variant it was given
  effort?: string;
  // Speed: the first round's writing (the writer's turn, or the reference), and the request to the first passing check
  // with every round, rebuild and check in between
  buildMs: number; firstUseMs?: number;
  // The writer's time over every round it took (the model's share of the wait, checks and restarts left out)
  writeMs: number;
  // Accuracy: rounds the writer took (1 = right the first time), whether it ever passed
  rounds: number; firstTry: boolean; pass: boolean; detail: string;
  // The way can do only part of the request (the arm's `partial`)
  partial?: string;
  // Elegance: took effect in the running instance or only after a restart; cards and questions the user answered
  // effectMs: from the end of the writing to the running instance showing it
  effect?: Effect; effectMs?: number; cards: number; questions: number; cardOpenMs: number;
  history: Round[];
  // Source arms: the rebuild after each round
  rebuild?: { ms: number; ok: boolean };
  files: number; lines: number;
  // Plugin arms: switched off on the page in one step, the check must fail again
  rollback?: { pass: boolean; detail: string };
  // Arms both Acpira groups offer: the same build checked against the other group's engine
  other?: { group: Group; pass: boolean; detail: string };
  // The writer's model calls (the meter), when it is metered
  usage?: { requests: number; input: number; cacheRead: number; output: number; costUsd: number; modelMs: number; slowestMs: number };
  error?: string;
}

// Permission cards are allowed once (a card leaves its turn once answered); a plugin card is applied (the user's
// click), any other question skipped. Each card's open time is kept: what a user would sit through is the count, the
// bench adds no wait
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
      for (const card of (t as AgentTurn).pluginCards ?? []) {
        const key = `card:${card.id}`;
        if (!seen.has(key)) seen.set(key, Date.now());
        click(key, () => { n.cards++; view.post({ type: 'pluginCard', sessionId: v.id, card: card.id, op: 'apply' }); });
      }
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
          click(q.id, () => {
            n.questions++;
            view.post({ type: 'answer', sessionId: v.id, blockId: q.id, answers: {}, skip: true });
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

const promptLog = (c: Ctx) => join(c.home, '..', 'fake-prompts.jsonl');

// The engine and the fake agent as installed copies outside the repository: the plugin guide names the running binary,
// and a path into the source tree would lead a writer there
let fakeBin = FAKE;
function install(bin: string, name: string): string {
  if (!bin.startsWith(ROOT)) return bin;
  const to = join(WORK, 'app', name);
  mkdirSync(dirname(to), { recursive: true });
  copyFileSync(bin, to);
  chmodSync(to, 0o755);
  return to;
}

// One engine over the run's homes: the fake agent is always registered, so checks never reach a model. The writer
// works in `view`, the checks in a view of their own, so the writer's session stays where it was between rounds
async function engine(bin: string, c: Ctx, env: Record<string, string> = {}) {
  // The engine hosts the writer (the built-in agent's shell is its child), so the whole engine runs in the run's sandbox
  process.env.ACPIRA_SIDECAR_BIN = jailedBin(bin, dirname(c.home));
  // A loaded machine (another build, the sandbox's first look at the binary) can hold a start past the handshake's
  // wait: that is the bench's trouble, not the writer's, so a start is tried three times, each failed one ended
  let host: Host;
  for (let attempt = 1; ; attempt++) {
    try {
      host = await Host.start({
        cwd: c.project, home: c.acpiraHome, defaultAgent: 'fake' as never,
        agents: { fake: { name: 'Fake', command: process.execPath, args: [fakeBin] } },
        env: { HOME: c.home, TMPDIR: runTmp(c), CLAUDE_CODE_TMPDIR: shortTmp(dirname(c.home)), FAKE_PROMPT_LOG: promptLog(c), ...env },
      });
      break;
    } catch (e) {
      if (attempt >= 3) throw e;
      console.log(`engine start ${attempt} failed (${String(e).split('\n')[0]}), trying again`);
    }
  }
  let theme: string | undefined;
  let pieces: PluginSlot[] = [];
  host.shell.onMessage(m => {
    if (m.type !== 'hostMessage') return;
    const msg = m.message as HostMsg;
    if (msg.type === 'pluginTheme') theme = msg.css;
    else if (msg.type === 'pluginUi') pieces = msg.slots;
  });
  const opened = async () => [await host.view(), await host.view()] as const;
  const [view, checks] = await opened().catch(async (e: unknown) => { await host.dispose(); throw e; });
  // Prompts as the fake agent received them; its session ids are its own, so the check's are the lines after its ask
  const logLines = () => (existsSync(promptLog(c)) ? readFileSync(promptLog(c), 'utf8').split('\n').filter(Boolean).map(l => JSON.parse(l) as { text: string }) : []);
  let asked = 0;
  const probe: AcpiraProbe = {
    kind: 'acpira',
    async ask(prompt) {
      if (c.work === c.project) await checks.newSession('fake' as never);
      else {
        // A session in another folder: the view first shows whatever session it had, the new one arrives on a later push
        const before = checks.activeId;
        const where = realpathSync(c.work);
        checks.post({ type: 'newSession', agent: 'fake' as never, cwd: c.work });
        await checks.until(() => {
          const v = checks.active();
          return !!v && v.id !== before && v.turns.length === 0 && !!v.cwd && realpathSync(v.cwd) === where;
        }, 30_000, `a new session in ${c.work}`);
      }
      asked = logLines().length;
      const approver = approve(checks);
      try {
        await checks.handle({ type: 'send', text: prompt }, CHECK_MS);
      } finally {
        approver.stop();
      }
      return reply(checks.active());
    },
    session: () => checks.active(),
    theme: () => theme,
    pieces: () => pieces,
    received() {
      return logLines().slice(asked).pop()?.text ?? '';
    },
  };
  return { host, view, probe };
}

const sh = (cmd: string, cwd: string) => spawnSync('/bin/sh', ['-c', cmd], { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
const commitAll = (d: string) => sh('git init -q 2>/dev/null; git add -A && git -c user.name=bench -c user.email=bench@localhost commit -qm base --allow-empty', d);

// The run's homes: HOME, the engine's data root, a project in git (its diff is the run's footprint), the rate service
function prepare(dir: string, fxUrl: string, source: boolean, group: Group, plainArm: boolean): Ctx {
  const project = join(dir, 'project');
  const elsewhere = join(dir, 'elsewhere');
  const c: Ctx = { home: join(dir, 'home'), acpiraHome: join(dir, 'home', '.acpira'), project, fxUrl, work: plainArm ? elsewhere : project };
  for (const d of [project, elsewhere]) {
    mkdirSync(d, { recursive: true });
    writeFileSync(join(d, 'README.md'), '# demo\n');
    commitAll(d);
  }
  // The plugins folder is a repository too, its bookkeeping and the scaffold's generated types left out, so the
  // footprint counts what the plugin's author wrote. An OpenCode run's home has no Acpira in it
  if (group !== 'opencode') {
    mkdirSync(join(c.acpiraHome, 'plugins'), { recursive: true });
    writeFileSync(join(c.acpiraHome, 'plugins', '.keep'), '');
    commitAll(join(c.acpiraHome, 'plugins'));
    writeFileSync(join(c.acpiraHome, 'plugins', '.git', 'info', 'exclude'), '.*\nstate.json\nchanges.jsonl\nschedules.json\nacpira.d.ts\n');
  }
  // The files lock-gate's check writes, which a later round's footprint would otherwise count
  for (const d of [project, elsewhere]) writeFileSync(join(d, '.git', 'info', 'exclude'), '/Cargo.lock\n/notes.txt\n');
  if (group === 'opencode') ocPrepare(c);
  if (source && group !== 'opencode') {
    c.source = join(dir, 'source');
    mkdirSync(c.source);
    const r = sh(`git -C "${ROOT}" archive ${REVS[group]} | tar -x -C "${c.source}" && git init -q && git add -A && git -c user.name=bench -c user.email=bench@localhost commit -qm base`, c.source);
    if (r.status !== 0) throw new Error(`source tree of ${REVS[group]}: ${r.stderr}`);
  }
  return c;
}

const other = (g: Group): Group => (g === 'today' ? 'plugins' : 'today');
const footprint = (c: Ctx) => changes([c.project, join(c.acpiraHome, 'plugins'), ocConfigDir(c), ...(c.source ? [c.source] : [])]);

// What the user says when a round did not work: the symptom they would see, and nothing of the check behind it
const followUp = (symptom: string | undefined) => `还不行：${symptom ?? '没看到效果'}。`;

interface Meterish { url: string; calls: Call[] }

function usageOf(calls: Call[], run: string) {
  const mine = calls.filter(c => c.run === run);
  if (!mine.length || !MODEL) return undefined;
  const u = {
    requests: mine.length, input: mine.reduce((a, c) => a + c.input, 0), cacheRead: mine.reduce((a, c) => a + c.cacheRead, 0),
    cacheWrite: mine.reduce((a, c) => a + c.cacheWrite, 0), output: mine.reduce((a, c) => a + c.output, 0),
  };
  // modelMs: the calls' own durations summed, so a harness's overhead is the writing time less this; slowest: one outlier
  // call (a gateway stall) shows here instead of being read as the harness's slowness
  const modelMs = mine.reduce((a, c) => a + c.ms, 0);
  const slowestMs = Math.max(...mine.map(c => c.ms));
  return { requests: u.requests, input: u.input, cacheRead: u.cacheRead, output: u.output, costUsd: Math.round(cost({ model: MODEL, ...u }) * 1000) / 1000, modelMs, slowestMs };
}

// The writer session's model (subscription Claude) and reasoning level, picked in its controls as a user would and read
// back; a level the session does not offer fails the run instead of leaving it at the default. The built-in agent was
// configured with that level as its only one, so it starts there
async function pickWriter(view: View): Promise<string> {
  const find = (pred: (c: ConfigControl) => boolean) => view.active()?.controls.options.find(pred);
  const pick = async (what: string, pred: (c: ConfigControl) => boolean, want: (o: SessionOption) => boolean) => {
    await view.until(() => !!find(pred), 60_000, `the session's ${what} control`);
    const control = find(pred)!;
    const option = control.options.find(want);
    if (!option) throw new Error(`no ${what} to pick among ${control.options.map(o => o.id).join(', ')}`);
    if (control.value !== option.id) {
      view.post({ type: 'setConfig', sessionId: view.active()!.id, configId: control.id, value: option.id });
      await view.until(() => find(pred)?.value === option.id, 60_000, `${what} = ${option.id}`);
    }
    return option;
  };
  if (CLAUDE_SUB) {
    const alias = CLAUDE_ALIAS[MODEL!]!;
    await pick('model', c => c.category === 'model', o => o.id === alias);
  }
  const level = await pick('reasoning level', isReasoningControl, o => o.id === EFFORT || o.name.toLowerCase() === EFFORT);
  // Full access, as OpenCode's `--auto`: the built-in agent's approval select, else the agent's bypass mode. The plugin
  // card stays, it is the one confirmation activation always asks for
  const full = (o: SessionOption) => o.kind === 'full_access' || ['full', 'bypassPermissions', 'yolo'].includes(o.id);
  if (find(c => c.category === '_permission_mode')) await pick('approval level', c => c.category === '_permission_mode', full);
  else {
    await view.until(() => !!view.active()?.controls.modes.some(full), 60_000, 'a full-access mode');
    const mode = view.active()!.controls.modes.find(full)!;
    if (view.active()!.controls.modeId !== mode.id) {
      view.post({ type: 'setMode', sessionId: view.active()!.id, id: mode.id });
      await view.until(() => view.active()?.controls.modeId === mode.id, 60_000, `mode = ${mode.id}`);
    }
  }
  return level.id;
}

async function runAcpira(task: RaceTask, armId: ArmId, group: Exclude<Group, 'opencode'>, c: Ctx, r: Result, runId: string, art: string, meter?: Meterish) {
  const arm = task.arms.find(a => a.id === armId && a.groups.includes(group))!;
  // The writer's model, on the meter (a native-login writer such as claude-max is not metered)
  let writerAgent = 'fake';
  let writerEnv: Record<string, string> = {};
  if (!DRY) {
    const spec = MODELS[MODEL!]!;
    const setup = HARNESSES[WRITER]!({ model: spec, base: `${meter!.url}/r/${runId}/${spec.upstream ?? 'gw'}`, home: c.home, acpiraHome: c.acpiraHome, effort: EFFORT });
    writerAgent = setup.agent;
    writerEnv = { ...setup.env, ...(CLAUDE_SUB ? { CLAUDE_CODE_EXECUTABLE: CLAUDE_BIN } : {}) };
  }
  const t0 = Date.now();
  let e = await engine(ENGINES[group]!, c, writerEnv);
  const fresh: { host: Host }[] = [];
  try {
    if (!ENGINE_TOOLS.size) {
      const listed = JSON.parse(await e.probe.ask('mcp-call:__list__ {}') || '{}') as { tools?: string[] };
      for (const t of listed.tools ?? []) ENGINE_TOOLS.add(t);
    }
    await e.view.newSession(writerAgent as never);
    const writerSession = e.view.active()!.id;
    if (!DRY) r.effort = await pickWriter(e.view);
    for (let round = 1; round <= (DRY ? 1 : 1 + REPAIRS); round++) {
      const last = r.history[r.history.length - 1];
      const prompt = DRY ? arm.reference!(c) : round === 1 ? arm.prompt(c) : followUp(last?.symptom);
      const r0 = Date.now();
      const approver = approve(e.view);
      let failure: string | undefined;
      try {
        if (e.view.activeId !== writerSession) await e.view.selectSession(writerSession);
        await e.view.handle({ type: 'send', sessionId: writerSession, text: prompt }, ROUND_MIN * 60_000);
      } catch (err) {
        failure = String(err);
        e.view.post({ type: 'stop', sessionId: writerSession });
        await e.view.until(() => !e.view.active()?.running, 30_000, 'the stopped turn').catch(() => {});
      } finally {
        approver.stop();
      }
      const ms = Date.now() - r0;
      if (round === 1) r.buildMs = Date.now() - t0;
      r.writeMs += ms;
      r.cards += approver.counts.cards;
      r.questions += approver.counts.questions;
      r.cardOpenMs += approver.counts.openMs;
      if (arm.build && c.source) {
        const b0 = Date.now();
        const b = sh(arm.build, c.source);
        r.rebuild = { ms: Date.now() - b0, ok: b.status === 0 };
      }
      // The footprint is the writing's alone, measured before a check writes files of its own
      Object.assign(r, footprint(c));
      // The theme layer arrives on its own push after the activation
      if (task.id === 'warm-theme' && armId === 'plugin') await e.view.until(() => !!e.probe.theme(), 5000, 'the theme layer').catch(() => {});
      if (task.id === 'status-widget') await e.view.until(() => e.probe.pieces().length > 0, 5000, 'the interface pieces').catch(() => {});
      // A source arm's engine change runs only in the tree's own binary, which is a restart by nature
      const ownBin = armId === 'source' && task.id === 'fx-tool' && c.source ? join(c.source, 'rust', 'target', 'debug', 'acpira') : undefined;
      let v: Verdict & { ms?: number } = ownBin ? { pass: false, detail: 'needs the rebuilt engine' } : await settle(() => task.accept(c, armId, e.probe));
      let effect: Effect | undefined = v.pass ? 'hot' : undefined;
      const effectMs = v.pass ? v.ms : undefined;
      if (!v.pass) {
        const f = await engine(ownBin ?? ENGINES[group]!, c, writerEnv);
        fresh.push(f);
        const again = await task.accept(c, armId, f.probe);
        await f.host.dispose();
        if (again.pass) { v = again; effect = 'restart'; } else if (!ownBin) v = { ...v, detail: `${v.detail} (after a restart too: ${again.detail})` }; else v = again;
      }
      r.history.push({ round, ms, pass: v.pass, effect, ...(effectMs !== undefined ? { effectMs } : {}), detail: v.detail, ...(v.symptom ? { symptom: v.symptom } : {}), ...(failure ? { failure } : {}) });
      Object.assign(r, { pass: v.pass, detail: v.detail, rounds: round, effect, effectMs });
      if (v.pass) {
        r.firstUseMs = Date.now() - t0;
        r.firstTry = round === 1;
        break;
      }
    }
    if (arm.rollback === 'plugin' && r.pass) {
      const ids = pluginIds(c.acpiraHome);
      const offs = ids.map(id => sh(`"${process.env.ACPIRA_SIDECAR_BIN}" plugin disable ${id} --home "${c.acpiraHome}"`, c.project));
      // The engine sees the state on its next poll
      await new Promise(res => setTimeout(res, 2500));
      const again = await task.accept(c, armId, e.probe);
      const failed = offs.find(o => o.status !== 0);
      r.rollback = { pass: !failed && !again.pass, detail: failed ? failed.stderr.trim() : `${ids.join(', ')} off: ${again.detail}` };
    }
    if (arm.groups.includes(other(group)) && ENGINES[other(group) as 'today'] && existsSync(ENGINES[other(group) as 'today']!) && armId !== 'source') {
      await e.host.dispose();
      e = await engine(ENGINES[other(group) as 'today']!, c, writerEnv);
      const v2 = await task.accept(c, armId, e.probe);
      r.other = { group: other(group), pass: v2.pass, detail: v2.detail };
    }
  } finally {
    writeFileSync(join(art, 'session.json'), JSON.stringify(e.view.active() ?? null, null, 2));
    await e.host.dispose();
    for (const f of fresh) await f.host.dispose().catch(() => {});
  }
}

async function runOpenCode(task: RaceTask, c: Ctx, r: Result, runId: string, art: string, meter?: Meterish) {
  const arm = task.arms.find(a => a.id === 'opencode')!;
  if (!DRY) {
    ocWriterConfig(c, MODELS[MODEL!]!, `${meter!.url}/r/${runId}/${MODELS[MODEL!]!.upstream ?? 'gw'}`, EFFORT!, CLAUDE_SUB ? CLAUDE_ALIAS[MODEL!] : undefined);
    if (CLAUDE_SUB) {
      // Claude Code's login in the run's HOME (keychain and account file, as for Acpira's side), then OpenCode's
      HARNESSES['claude-max']!({ model: MODELS[MODEL!]!, base: '', home: c.home, acpiraHome: c.acpiraHome });
      await ocClaudeLogin(c, CLAUDE_ALIAS[MODEL!]!);
    }
    r.effort = EFFORT;
  }
  // The instance a user has open while the writer works: a server on the scripted model, started before the build, with
  // its terminal interface open when the task shows on screen
  const live = await ocServer(c);
  if (SCREEN_TASKS.has(task.id)) await live.probe.open();
  const t0 = Date.now();
  let session: string | undefined;
  try {
    for (let round = 1; round <= (DRY ? 1 : 1 + REPAIRS); round++) {
      const last = r.history[r.history.length - 1];
      const r0 = Date.now();
      let failure: string | undefined;
      if (DRY) arm.reference!(c);
      else {
        const w = await ocWrite(c, round === 1 ? arm.prompt(c) : followUp(last?.symptom), { model: MODELS[MODEL!]!, session, timeoutMs: ROUND_MIN * 60_000 });
        session = w.session ?? session;
        failure = w.failure;
        writeFileSync(join(art, `writer.${round}.out`), w.out);
      }
      const ms = Date.now() - r0;
      if (round === 1) r.buildMs = Date.now() - t0;
      r.writeMs += ms;
      Object.assign(r, footprint(c));
      let v: Verdict & { ms?: number } = await settle(() => task.accept(c, 'opencode', live.probe));
      let effect: Effect | undefined = v.pass ? 'hot' : undefined;
      const effectMs = v.pass ? v.ms : undefined;
      if (!v.pass) {
        await live.probe.reload();
        const again = await task.accept(c, 'opencode', live.probe);
        if (again.pass) { v = again; effect = 'reload'; }
      }
      if (!v.pass) {
        const f = await ocServer(c);
        try {
          const again = await task.accept(c, 'opencode', f.probe);
          if (again.pass) { v = again; effect = 'restart'; } else v = { ...v, detail: `${v.detail} (after a restart too: ${again.detail})` };
        } finally {
          await f.stop();
        }
      }
      r.history.push({ round, ms, pass: v.pass, effect, ...(effectMs !== undefined ? { effectMs } : {}), detail: v.detail, ...(v.symptom ? { symptom: v.symptom } : {}), ...(failure ? { failure } : {}) });
      Object.assign(r, { pass: v.pass, detail: v.detail, rounds: round, effect, effectMs });
      if (v.pass) {
        r.firstUseMs = Date.now() - t0;
        r.firstTry = round === 1;
        break;
      }
    }
  } finally {
    await live.stop();
  }
}

async function runOne(task: RaceTask, armId: ArmId, group: Group, rep: number, fxUrl: string, meter?: Meterish): Promise<Result> {
  const runId = `${task.id}.${armId}.${group}.${DRY ? 'dry' : MODEL}.${rep}`.replace(/[^\w.-]/g, '_');
  const dir = join(WORK, 'runs', runId);
  const art = join(OUT, 'runs', runId);
  mkdirSync(dir, { recursive: true });
  mkdirSync(art, { recursive: true });
  const c = prepare(dir, fxUrl, armId === 'source', group, (armId === 'plugin' || armId === 'opencode') && !!task.everywhere);
  const r: Result = {
    task: task.id, arm: armId, group, rep, dry: DRY, ...(DRY ? {} : { model: MODEL, writer: group === 'opencode' ? (CLAUDE_SUB ? 'opencode-claude-agent' : 'opencode') : WRITER }),
    buildMs: 0, writeMs: 0, rounds: 0, firstTry: false, pass: false, detail: '', cards: 0, questions: 0, cardOpenMs: 0, history: [], files: 0, lines: 0,
    ...(task.arms.find(a => a.id === armId)?.partial ? { partial: task.arms.find(a => a.id === armId)!.partial } : {}),
  };
  try {
    if (group === 'opencode') await runOpenCode(task, c, r, runId, art, meter);
    else await runAcpira(task, armId, group, c, r, runId, art, meter);
  } catch (err) {
    r.error = String(err);
  } finally {
    reap(c.home);
    rmSync(shortTmpPath(dir), { recursive: true, force: true });
  }
  if (meter) r.usage = usageOf(meter.calls, runId);
  return r;
}

// Whatever a writer left running under the run's HOME (a daemon its shell started, e.g. OpenCode's background service
// behind `opencode plugin list`, which holds one fixed port for the whole machine, so a later run's writer would reach
// this run's); macOS and Linux ps both print a process's environment after its command with -E / e, megabytes of it
function reap(home: string) {
  const r = spawnSync('ps', process.platform === 'darwin' ? ['-A', '-E', '-ww', '-o', 'pid=,command='] : ['-A', 'eww', '-o', 'pid=,command='], { encoding: 'utf8', maxBuffer: 256 << 20 });
  const mark = `HOME=${home}`;
  const pids = (r.stdout ?? '').split('\n')
    .filter(l => l.split(' ').includes(mark))
    .map(l => Number(l.trim().split(' ')[0]))
    .filter(p => p && p !== process.pid);
  for (const p of pids) {
    try {
      process.kill(p, 'SIGKILL');
    } catch {
      // already gone
    }
  }
  if (pids.length) console.log(`  reaped ${pids.length} process(es) left under ${basename(dirname(home))}`);
}

async function main() {
  const tasks = TASKS.filter(t => !list('task') || list('task')!.includes(t.id));
  const arms = list('arm') as ArmId[] | undefined;
  const groups = (list('group') as Group[] | undefined) ?? ['today', 'plugins', 'opencode'];
  bundleFakeAgent();
  mkdirSync(OUT, { recursive: true });
  mkdirSync(WORK, { recursive: true });
  // Nothing a writer inherits points into this repository: pnpm's INIT_CWD and npm_* variables, PWD, the loader in
  // NODE_OPTIONS, node_modules/.bin on PATH
  for (const [k, v] of Object.entries(process.env)) if (k !== 'PATH' && v?.includes(ROOT)) delete process.env[k];
  process.env.PATH = (process.env.PATH ?? '').split(':').filter(p => !p.startsWith(ROOT)).join(':');
  fakeBin = install(FAKE, 'fake-agent.mjs');
  installDriver(join(WORK, 'app'));
  // The bench's own line in a process list, which a writer may read: no path into this repository
  process.title = 'node';
  // Every process starts outside it too (the engine inherits this folder; inside the sandbox the repository is unreadable)
  process.chdir(WORK);
  const unjailed = setupJail(join(WORK, 'app'), ROOT, join(WORK, 'runs'));
  if (unjailed) console.log(`warning: ${unjailed}`);
  checkJail(ROOT);
  for (const g of ['today', 'plugins'] as const) if (ENGINES[g] && existsSync(ENGINES[g]!)) ENGINES[g] = install(ENGINES[g]!, join(g, 'acpira'));
  console.log(`runs in ${WORK}`);
  const fx = await startFx();
  // The writers' calls go through the metering proxy (meter.ts), the gateway's or DeepSeek's key added there
  const meter = DRY ? undefined : await startMeter({
    log: join(OUT, 'calls.ndjson'),
    ...(DUMP ? { dump: join(OUT, 'bodies') } : {}),
    keys: MODELS[MODEL!]!.upstream === 'ds' ? { ds: deepseekKey() ?? '' } : { gw: gatewayKey() },
  });
  // The meter holds the keys now; a writer's environment carries none
  for (const k of ['EVAL_GW_KEY', 'EVAL_DS_KEY', 'EVAL_DS_KEY_FILE']) delete process.env[k];
  const results: Result[] = [];
  const skipped: string[] = [];
  // OpenCode 2, pinned and installed on first use when the opencode group is asked for
  const ocMissing = groups.includes('opencode') ? ocInstall() : 'not asked for';
  if (groups.includes('opencode')) console.log(ocMissing ? `opencode group skipped: ${ocMissing}` : `opencode ${OC_VERSION}`);
  try {
    for (const task of tasks) for (const arm of task.arms) for (const group of arm.groups) {
      if (arms && !arms.includes(arm.id)) continue;
      if (!groups.includes(group)) continue;
      const name = `${task.id}/${arm.id}@${group}`;
      if (group === 'opencode' ? !!ocMissing : !ENGINES[group] || !existsSync(ENGINES[group]!)) { skipped.push(`${name}: no ${group} harness`); continue; }
      if (arm.id === 'source' && !WITH_SOURCE) { skipped.push(`${name}: needs --with-source`); continue; }
      // An arm with no way to do the task is part of the surface: the report shows why
      if (arm.none) appendFileSync(join(OUT, 'none.jsonl'), `${JSON.stringify({ task: task.id, arm: arm.id, group, reason: arm.none })}\n`);
      if (DRY && !arm.reference) { skipped.push(`${name}: no reference${arm.none ? ` (${arm.none})` : ''}`); continue; }
      for (let rep = REP_FROM; rep <= REPS; rep++) {
        const r = await runOne(task, arm.id, group, rep, fx.url, meter);
        // The model's service failing, or the bench failing before the writer had a round (an engine that did not start)
        const outage = !r.pass && (r.rounds === 0 || !!meter?.calls.some(c => c.run.startsWith(`${task.id}.${arm.id}.${group}.`) && c.error && OUTAGE.test(c.error)));
        results.push(r);
        appendFileSync(join(OUT, outage ? 'infra.jsonl' : 'results.jsonl'), `${JSON.stringify(r)}\n`);
        const extra = [
          r.effect && `effect ${r.effect}`, !DRY && `${r.rounds} round${r.rounds === 1 ? '' : 's'}`,
          r.rollback && `rollback ${r.rollback.pass ? 'ok' : 'FAILED'}`, r.other && `on ${r.other.group} ${r.other.pass ? 'pass' : 'fail'}`,
          r.usage && `${r.usage.requests} calls $${r.usage.costUsd}`,
        ].filter(Boolean).join(', ');
        console.log(`${r.pass ? 'PASS' : 'FAIL'} ${name}#${rep} build ${r.buildMs} ms${r.firstUseMs ? `, first use ${r.firstUseMs} ms` : ''}, ${r.cards} cards, ${r.files} files / ${r.lines} lines${extra ? `, ${extra}` : ''}: ${r.error ?? r.detail}`);
      }
    }
  } finally {
    fx.server.close();
    await meter?.stop();
  }
  for (const s of skipped) console.log(`skip ${s}`);
  console.log(`results in ${OUT}${DRY ? ' (dry)' : ''}`);
  // A dry run is the bench's own test: every reference must pass, roll back and carry over
  if (DRY && results.some(r => !r.pass || r.error || (r.rollback && !r.rollback.pass) || (r.other && !r.other.pass))) process.exitCode = 1;
}

await main();
