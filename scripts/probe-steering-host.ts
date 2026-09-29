import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { AgentTurn, PermissionBlock, SessionView } from '@shared/transcript';
import { Host } from './lib/host';

// Host-path steering smoke (real CLI, real model): the queue's Steer button through the Rust sidecar, the way the webview drives it.
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-steering-host.ts <claude|codex|…> [--model TEXT]
//
// --model: switch the session's model control to the first value whose id or name contains TEXT (case-insensitive)
//
// A prompt runs `sleep 8` through the agent's shell tool; once its tool row shows up, two prompts are queued. The first is steered
// (`steerQueued`), the second stays queued. Checks: the view advertises canSteer, the steered prompt leaves the queue and becomes a
// steer block inside the running turn, the turn settles once with the reply after the steer honouring it, and the second prompt
// then goes out as a turn of its own. Permission cards are allowed once.
const argv = process.argv.slice(2);
const valued = (f: string) => { const i = argv.indexOf(f); return i >= 0 ? argv[i + 1] : undefined; };
const model = valued('--model');
const agentId = argv.find(a => !a.startsWith('--') && a !== model) ?? 'claude';
const MARKER = 'PINEAPPLE';

const project = mkdtempSync(join(tmpdir(), `acpira-${agentId}-steer-`));
writeFileSync(join(project, 'README.md'), '# steering probe\n');
const checks: [string, boolean][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const lastAgent = (v: SessionView): AgentTurn | undefined => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };

const host = await Host.start({ cwd: project, defaultAgent: agentId, settings: { steerQueued: true } });
const m = await host.view();
const approver = setInterval(() => {
  const cur = m.active();
  for (const b of (cur ? lastAgent(cur) : undefined)?.blocks ?? []) {
    if (b.type !== 'permission') continue;
    const opt = (b as PermissionBlock).options.find(o => o.kind === 'allow_once') ?? (b as PermissionBlock).options[0];
    if (opt) void m.handle({ type: 'permission', sessionId: cur!.id, blockId: b.id, optionId: opt.id });
  }
}, 100);

try {
  // The view opens on an untouched session of the default agent, which newSession may still replace after it resolves
  const opened = m.activeId;
  m.post({ type: 'newSession', agent: agentId });
  await m.until(() => !!m.activeId && m.activeId !== opened, 5000, 'the replacement session').catch(() => {});
  await m.until(() => ['ready', 'error', 'auth_required'].includes(m.active()?.status ?? ''), 90_000, 'session start');
  let v = m.active()!;
  check('session ready', v.status === 'ready', v.error);
  check('view advertises canSteer', v.canSteer === true);
  if (model) {
    const opt = v.controls.options.find(o => o.category === 'model' || o.id === 'model');
    const pick = opt?.options.find(o => `${o.id} ${o.name}`.toLowerCase().includes(model.toLowerCase()));
    if (!opt || !pick) throw new Error(`no model matching ${model}`);
    await m.handle({ type: 'setConfig', configId: opt.id, value: pick.id });
    console.log(`model → ${pick.id}`);
  }
  const sessionId = v.id;

  const t0 = Date.now();
  m.post({ type: 'send', text: 'Run exactly this shell command and wait for it to finish: `sleep 8 && echo sleep-done`. Then reply with one short sentence saying what it printed.' });
  await m.until(() => !!m.active()?.running && !!lastAgent(m.active()!)?.blocks.some(b => b.type === 'tool_call'), 90_000, 'the first tool row');
  console.log(`tool row after ${Date.now() - t0} ms`);
  m.post({ type: 'send', text: `Additional instruction: end your final reply with the exact word ${MARKER}.` });
  m.post({ type: 'send', text: 'Reply with just the word QUEUED-OK.' });
  await m.until(() => (m.active()?.queued?.length ?? 0) === 2, 10_000, 'two queued prompts');
  const [first, second] = m.active()!.queued!;
  m.post({ type: 'steerQueued', sessionId, id: first!.id });
  await m.until(() => lastAgent(m.active()!)?.blocks.some(b => b.type === 'steer') ?? false, 30_000, 'the steer block');
  v = m.active()!;
  console.log(`steer block after ${Date.now() - t0} ms, running=${v.running}`);
  check('steered prompt left the queue, the other stayed', v.queued?.length === 1 && v.queued[0]!.id === second!.id, JSON.stringify(v.queued?.map(q => q.text)));
  check('turn still running when the steer landed', v.running);

  // The steered turn settles, then the queued prompt goes out as a turn of its own
  await m.until(() => { const s = m.active()!; return !s.running && !s.queued?.length && s.turns.length >= 4 && lastAgent(s)?.stop !== undefined; }, 180_000, 'the queued turn to end');
  v = m.active()!;
  const roles = v.turns.map(t => t.role).join(',');
  check('turns: user, agent (steered), user, agent', roles === 'user,agent,user,agent', roles);
  const steered = v.turns[1] as AgentTurn;
  const at = steered.blocks.findIndex(b => b.type === 'steer');
  const after = steered.blocks.slice(at + 1).map(b => b.type === 'text' ? b.markdown : '').join('');
  check('one steer block in the steered turn', steered.blocks.filter(b => b.type === 'steer').length === 1);
  check('steered turn ended end_turn', steered.stop === 'end_turn', String(steered.stop));
  check(`reply after the steer ends with ${MARKER}`, after.includes(MARKER), after.slice(-160));
  const tail = lastAgent(v)!;
  const reply = tail.blocks.map(b => b.type === 'text' ? b.markdown : '').join('');
  check('queued prompt answered in its own turn', /QUEUED-OK/.test(reply) && tail.stop === 'end_turn', reply.slice(0, 80));
  console.log('steered turn blocks:', steered.blocks.map(b => b.type === 'tool_call' ? `tool_call(${b.id.slice(-6)} ${b.verb} ${b.target ?? ""})` : b.type).join(' '));
} catch (e) {
  check(`probe ran through: ${String(e)}`, false);
  const v = m.active();
  console.log(host.logs.slice(-25).join('\n'));
  if (v) console.log('view:', JSON.stringify({ running: v.running, queued: v.queued, turns: v.turns.map(t => t.role === 'agent' ? t.blocks.map(b => b.type) : t.role) }));
} finally {
  clearInterval(approver);
  await host.dispose();
}
const failed = checks.filter(([, ok]) => !ok).length;
console.log(`\n${checks.length - failed}/${checks.length} passed`);
process.exit(failed ? 1 : 0);
