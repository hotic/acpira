import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import type { AgentTurn, GoalBlock, PermissionBlock, SessionView } from '@shared/transcript';
import { Host } from './lib/host';

// Host-path goal smoke (real CLI, real model): the dock's goal controls through the Rust sidecar, the way the webview drives them.
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-goal-host.ts [codex] [--bin PATH] [--keep]
//
// --bin: the codex-acp to launch (default: codex-acp on PATH); --keep: leave the temp project / CODEX_HOME / wire log in place
//
// Codex runs in a temp CODEX_HOME built from ~/.codex/config.toml (provider + model, `goals` feature on, no notify hook) and
// auth.json, so its sessions stay out of the real store. The agent is wrapped in a tee that writes the raw ndjson wire.
// Phase 1: set a goal only the probe can unblock (go.txt), so Codex's first turn ends with it active and automatic continuations
// follow (codex-acp 1.13.0 / 2.2.2: two, then Codex marks it blocked itself): they run as a turn holding the queue, pause stops further ones, go.txt + resume meets it (`complete` row, the goal
// leaves the view). Codex's pause does not interrupt a running turn, it only stops continuations (seen on codex-acp 1.13.0). Phase 2: set a goal, clear it while the turn runs. Every view change of interest is logged
// with its time, and the wire is summarised at the end (goal snapshots, threadStatus brackets, automatic continuations).
const argv = process.argv.slice(2);
const valued = (f: string) => { const i = argv.indexOf(f); return i >= 0 ? argv[i + 1] : undefined; };
const bin = valued('--bin') ?? 'codex-acp';
const keep = argv.includes('--keep');
const agentId = 'codex';

const work = mkdtempSync(join(tmpdir(), 'acpira-goal-probe-'));
const project = join(work, 'project');
mkdirSync(project);
writeFileSync(join(project, 'README.md'), '# goal probe\n');

// Temp CODEX_HOME: the real provider settings without the notify hook, project-specific trust or plugins
const codexHome = join(work, 'codex-home');
mkdirSync(codexHome);
const real = readFileSync(join(homedir(), '.codex/config.toml'), 'utf8');
const head = real.slice(0, real.search(/^\[(?!model_providers)/m)).replace(/^notify = \[[\s\S]*?\]\n/m, '');
writeFileSync(join(codexHome, 'config.toml'), `${head}\n[projects."${project}"]\ntrust_level = "trusted"\n\n[features]\ngoals = true\n`);
if (existsSync(join(homedir(), '.codex/auth.json'))) copyFileSync(join(homedir(), '.codex/auth.json'), join(codexHome, 'auth.json'));

// The tee wrapper: host → agent into in.ndjson, agent → host into out.ndjson
const wireIn = join(work, 'in.ndjson');
const wireOut = join(work, 'out.ndjson');
const wrapper = join(work, 'codex-acp-tee.sh');
writeFileSync(wrapper, `#!/bin/sh\ntee -a '${wireIn}' | '${bin}' "$@" | tee -a '${wireOut}'\n`);
chmodSync(wrapper, 0o755);

const checks: [string, boolean][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const t0 = Date.now();
const at = () => `${((Date.now() - t0) / 1000).toFixed(1).padStart(6)}s`;
const goalRows = (v: SessionView): GoalBlock[] => v.turns.flatMap(t => t.role === 'agent' ? t.blocks.filter((b): b is GoalBlock => b.type === 'goal') : []);
const lastAgent = (v: SessionView): AgentTurn | undefined => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const files = () => readdirSync(project).filter(f => f !== 'README.md').sort();

const host = await Host.start({
  cwd: project,
  defaultAgent: agentId,
  agents: { codex: { name: 'Codex', command: wrapper } },
  env: { CODEX_HOME: codexHome },
});
const m = await host.view();

// Timeline: one line whenever the summary of what the dock and transcript show changes
let lastLine = '';
const timeline = setInterval(() => {
  const v = m.active();
  if (!v) return;
  const g = v.goal;
  const line = `running=${v.running} goal=${g ? `${g.status}${g.tokensUsed !== undefined ? ` tok=${g.tokensUsed}` : ''}${g.timeUsedSeconds !== undefined ? ` t=${g.timeUsedSeconds}s` : ''}` : '-'} turns=${v.turns.length} rows=[${goalRows(v).map(r => r.event).join(',')}] queued=${v.queued?.length ?? 0} files=[${files().join(',')}]`;
  if (line !== lastLine) { console.log(`${at()} ${line}`); lastLine = line; }
  // Permission cards (none expected with approval_policy never) are allowed once
  for (const b of lastAgent(v)?.blocks ?? []) {
    if (b.type !== 'permission') continue;
    const opt = (b as PermissionBlock).options.find(o => o.kind === 'allow_once') ?? (b as PermissionBlock).options[0];
    if (opt) void m.handle({ type: 'permission', sessionId: v.id, blockId: b.id, optionId: opt.id });
  }
}, 150);

try {
  const opened = m.activeId;
  m.post({ type: 'newSession', agent: agentId });
  await m.until(() => !!m.activeId && m.activeId !== opened, 5000, 'the replacement session').catch(() => {});
  await m.until(() => ['ready', 'error', 'auth_required'].includes(m.active()?.status ?? ''), 90_000, 'session start');
  let v = m.active()!;
  const sessionId = v.id;
  check('session ready', v.status === 'ready', v.error);
  check('goalActions = set, pause, resume, clear', JSON.stringify(v.goalActions) === '["set","pause","resume","clear"]', JSON.stringify(v.goalActions));
  check('no goal before set', v.goal === undefined);

  // Phase 1: a goal the agent cannot meet alone (go.txt is the probe's to create), so its turn ends with the goal active and
  // Codex goes on by itself: the continuation runs as a turn, holds the queue, pause stops further ones, resume finishes it
  const conts = () => host.logs.filter(l => l.includes('goal continuation started')).length;
  const objective = 'The goal is met only when a file named go.txt exists in the current working directory. Never create, touch or write go.txt yourself, and never wait or poll for it with sleep. Each time you work on this goal, run `ls` once; if go.txt is missing, reply with one short sentence and end your turn. Once go.txt exists, create done.txt containing the word done; the goal is then met.';
  console.log(`${at()} → goal set`);
  m.post({ type: 'goal', sessionId, action: 'set', objective });
  await m.until(() => m.active()?.goal?.status === 'active', 60_000, 'the goal to turn active');
  v = m.active()!;
  check('set: goal active with the objective', v.goal?.objective === objective, v.goal?.objective.slice(0, 60));
  const hidden = v.turns.find(t => t.role === 'user');
  check('set: sent as a hidden /goal user turn', hidden?.role === 'user' && hidden.text.startsWith('/goal ') && hidden.auto === true && hidden.autoReason === 'goal', hidden?.role === 'user' ? hidden.text.slice(0, 40) : undefined);
  check('set: a set row', goalRows(v).some(r => r.event === 'set'));

  // The automatic continuation: Codex goes on right after the set prompt's own turn (threadStatus idle → active with no
  // prompt), so it shows inside the same running stretch; the host log marks it
  const turnsBefore = v.turns.length;
  await m.until(() => conts() > 0, 180_000, 'an automatic goal continuation');
  v = m.active()!;
  check('continuation: host opened it as a running turn', v.running, `log=${conts()} running=${v.running}`);
  check('continuation: no new user turn', v.turns.length === turnsBefore, `${turnsBefore} → ${v.turns.length}`);
  m.post({ type: 'send', text: 'Reply with just the word QUEUED-OK.' });
  await m.until(() => (m.active()?.queued?.length ?? 0) === 1 || !m.active()!.running, 10_000, 'the prompt to queue');
  check('continuation: a typed prompt waits in the queue', m.active()!.running && m.active()!.queued?.length === 1, `running=${m.active()!.running} queued=${m.active()!.queued?.length}`);

  console.log(`${at()} → goal pause`);
  m.post({ type: 'goal', sessionId, action: 'pause' });
  await m.until(() => m.active()?.goal?.status === 'paused', 30_000, 'the goal to pause');
  check('pause: goal paused, a paused row', goalRows(m.active()!).some(r => r.event === 'paused'));
  await m.until(() => { const s = m.active()!; return !s.running && !s.queued?.length && /QUEUED-OK/.test(lastAgent(s)?.blocks.map(b => b.type === 'text' ? b.markdown : '').join('') ?? ''); }, 180_000, 'the queued prompt to be answered');
  check('queue: the prompt went out once the continuation ended and was answered', true);
  const pausedConts = conts();
  await new Promise(r => setTimeout(r, 25_000));
  v = m.active()!;
  check('pause: no further continuation for 25 s', conts() === pausedConts && !v.running && v.goal?.status === 'paused', `log ${pausedConts} → ${conts()} running=${v.running} goal=${v.goal?.status}`);

  writeFileSync(join(project, 'go.txt'), 'go\n');
  console.log(`${at()} → go.txt written, goal resume`);
  m.post({ type: 'goal', sessionId, action: 'resume' });
  await m.until(() => m.active()?.goal?.status === 'active', 60_000, 'the goal to resume');
  v = m.active()!;
  check('resume: a resumed row', goalRows(v).some(r => r.event === 'resumed'), goalRows(v).map(r => r.event).join(','));
  check('resume: sent as a hidden /goal resume turn', v.turns.some(t => t.role === 'user' && t.text === '/goal resume' && t.autoReason === 'goal'));

  // Met: the goal leaves the view once its turn settles
  await m.until(() => { const s = m.active()!; return !s.running && (s.goal === undefined || s.goal.status === 'complete'); }, 300_000, 'the goal to be met');
  await new Promise(r => setTimeout(r, 3000));
  v = m.active()!;
  check('complete: done.txt on disk', files().includes('done.txt'), `[${files()}]`);
  check('complete: a complete row', goalRows(v).some(r => r.event === 'complete'), goalRows(v).map(r => r.event).join(','));
  check('complete: the goal left the view', v.goal === undefined, JSON.stringify(v.goal));
  check('complete: not running, queue empty', !v.running && !v.queued?.length);
  console.log(`${at()} host log: ${conts()} goal continuation(s) started`);

  // Phase 2: set → clear while running
  const before = goalRows(v).length;
  console.log(`${at()} → goal set (to clear)`);
  m.post({ type: 'goal', sessionId, action: 'set', objective: 'Run the shell command `sleep 30`, then create clear-me.txt containing the word late. The goal is met when clear-me.txt exists.' });
  await m.until(() => m.active()?.goal?.status === 'active' && m.active()!.running, 60_000, 'the second goal to run');
  await m.until(() => !!lastAgent(m.active()!)?.blocks.some(b => b.type === 'tool_call'), 120_000, 'the second goal\'s tool row');
  console.log(`${at()} → goal clear`);
  m.post({ type: 'goal', sessionId, action: 'clear' });
  await m.until(() => m.active()?.goal === undefined, 30_000, 'the goal to clear');
  check('clear: goal gone from the view', true);
  check('clear: a cleared row', goalRows(m.active()!).slice(before).some(r => r.event === 'cleared'), goalRows(m.active()!).slice(before).map(r => r.event).join(','));
  await m.until(() => !m.active()!.running, 180_000, 'the turn to settle after clear');
  await new Promise(r => setTimeout(r, 10_000));
  v = m.active()!;
  check('clear: no goal and idle 10 s later', v.goal === undefined && !v.running, `files=[${files()}]`);
  const rec = await host.record(sessionId);
  check('record: no goal persisted', !rec?.goal, JSON.stringify(rec?.goal));
} catch (e) {
  check(`probe ran through: ${String(e)}`, false);
  console.log(host.logs.slice(-30).join('\n'));
} finally {
  clearInterval(timeline);
  await host.dispose();
}

// What the agent said about goals and its thread on the wire
const wire = (p: string) => existsSync(p) ? readFileSync(p, 'utf8').split('\n').filter(Boolean).map(l => { try { return JSON.parse(l) as Record<string, any>; } catch { return null; } }).filter(Boolean) as Record<string, any>[] : [];
const out = wire(wireOut);
const init = out.find(x => x.result?.protocolVersion !== undefined)?.result;
console.log('\nwire: initialize goal capability', JSON.stringify(init?._meta?.goal ?? init?._meta?.jetbrains?.air?.goal ?? init?.agentCapabilities?._meta?.goal ?? init?.agentCapabilities?._meta?.jetbrains?.air?.goal));
for (const x of wire(wireIn)) if (x.method === '_session/goal' || (x.method === 'session/prompt' && JSON.stringify(x.params?.prompt).includes('/goal'))) console.log(`wire → ${x.method} ${JSON.stringify(x.method === '_session/goal' ? x.params : x.params.prompt.map((b: any) => b.text))}`);
for (const x of out) {
  const u = x.params?.update;
  if (u?.sessionUpdate !== 'session_info_update') continue;
  const goal = u._meta?.goal !== undefined ? u._meta.goal : u._meta?.jetbrains?.air?.goal;
  const thread = u._meta?.codex?.threadStatus?.type;
  if (goal !== undefined) console.log(`wire ← goal ${JSON.stringify(goal)}`);
  if (thread) console.log(`wire ← threadStatus ${thread}`);
}
console.log(`wire log: ${work}`);
if (!keep) rmSync(codexHome, { recursive: true, force: true });

const failed = checks.filter(([, ok]) => !ok).length;
console.log(`\n${checks.length - failed}/${checks.length} passed`);
process.exit(failed ? 1 : 0);
