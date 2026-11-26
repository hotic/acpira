import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { PermissionBlock, SessionView } from '@acpira/shared/transcript';
import { Host } from '../lib/host';

// An Antigravity orchestrator turn end to end through the sidecar (real `agy acp`, real model calls):
//   pnpm exec tsx scripts/probe/orchestrator-host.ts [--model gemini-pro-orchestrator-neon]
// The planner's plan hand-off must open a sidekick round on the root (the hand-off row itself never shows), the executor's
// calls and permission cards must live in that round, its model must come from the session's transcript, and the reply
// must stay on the root. Every permission card, root or round, is answered like a click.
const flags = process.argv.slice(2);
const modelAt = flags.indexOf('--model');
const model = (modelAt >= 0 ? flags[modelAt + 1] : undefined) ?? 'gemini-pro-orchestrator-neon';
const project = mkdtempSync(join(tmpdir(), 'acpira-orch-project-'));
const store = mkdtempSync(join(tmpdir(), 'acpira-orch-store-'));
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const until = async (pred: () => boolean, ms: number, what: string) => {
  const t0 = Date.now();
  while (!pred()) { if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`); await new Promise(r => setTimeout(r, 50)); }
};
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };

const host = await Host.start({ cwd: project, home: store, defaultAgent: 'antigravity' });
const m = await host.view();
let roundCards = 0;
const answered = new Set<string>();
const approver = setInterval(() => {
  const cur = m.active();
  if (!cur) return;
  const root = (lastAgent(cur)?.blocks ?? []).filter((b): b is PermissionBlock => b.type === 'permission');
  const round = (cur.subagents ?? []).flatMap(n => n.permissions ?? []);
  for (const p of [...root, ...round]) {
    if (answered.has(p.id)) continue;
    const opt = p.options.find(o => o.kind === 'allow_once') ?? p.options[0];
    if (!opt) continue;
    answered.add(p.id);
    if (round.includes(p)) roundCards++;
    console.log(`approving${round.includes(p) ? ' (round)' : ''}: ${p.title} → ${opt.id}(${opt.kind})`);
    void m.handle({ type: 'permission', sessionId: cur.id, blockId: p.id, optionId: opt.id });
  }
}, 100);

try {
  await m.newSession('antigravity');
  await until(() => ['ready', 'error', 'auth_required', 'readonly'].includes(m.active()?.status ?? ''), 90_000, 'session start');
  let v = m.active()!;
  check('session ready', v.status === 'ready', v.error);
  if (v.status !== 'ready') throw new Error('not ready');
  const ctl = v.controls.options.find(o => o.category === 'model');
  if (!ctl) throw new Error('no model control');
  await m.handle({ type: 'setConfig', configId: ctl.id, value: model });
  await until(() => m.active()!.controls.options.find(o => o.id === ctl.id)?.value === model, 30_000, `model → ${model}`);
  check(`model set to ${model}`, true);
  check('the host sidekick select is set aside', !m.active()!.controls.options.some(o => o.id.startsWith('host_sidekick')),
    m.active()!.controls.options.map(o => o.id).join(','));

  await m.handle({ type: 'send', text: 'Create fib.py in the current directory that prints the first 20 Fibonacci numbers, one per line, then run it and confirm the output is correct.' });
  v = m.active()!;
  const turn = lastAgent(v)!;
  const rounds = (v.subagents ?? []).filter(n => n.harness?.persona === 'sidekick');
  console.log('rounds:', JSON.stringify(rounds.map(n => ({ id: n.id, state: n.state, model: n.model, title: n.title, round: n.harness?.round, agent: n.harness?.agent, tools: n.toolCount, result: n.result?.slice(0, 120), steps: n.steps?.map(s => `${s.kind}:${s.verb} ${s.target ?? ''}`) })), null, 1));
  console.log('root blocks:', turn.blocks.map(b => b.type === 'tool_call' ? `tool:${b.verb} ${b.target ?? ''}${b.subagentId ? `→${b.subagentId.slice(0, 8)}` : ''}` : b.type).join(' | '));
  check('turn ended end_turn', turn.stop === 'end_turn', String(turn.stop));
  check('at least one sidekick round', rounds.length > 0, `${rounds.length}`);
  const first = rounds[0];
  if (first) {
    check('round runs in antigravity', first.harness?.agent === 'antigravity', first.harness?.agent);
    check('round settled after the prompt', first.state === 'completed', first.state);
    check('round names the transcript\'s execution model', !!first.model && !/orchestrator/i.test(first.model), first.model);
    check('round carries the plan as task and a title', !!first.task && !!first.title, first.title);
    check('round holds the executor\'s calls', first.toolCount > 0, `${first.toolCount}`);
    check('the round\'s anchor row sits on the root', turn.blocks.some(b => b.type === 'tool_call' && b.subagentId === first.id));
  }
  check('no exit_plan_mode row on the root', !turn.blocks.some(b => b.type === 'tool_call' && /plan_mode/.test(`${b.verb} ${b.target ?? ''}`)));
  check('reply text stays on the root', turn.blocks.some(b => b.type === 'text' && b.markdown.trim().length > 0));
  check('executor permission cards asked inside the round', roundCards > 0, `${roundCards} round card(s)`);
} catch (e) {
  check('scenario', false, String(e));
} finally {
  clearInterval(approver);
  await host.dispose();
}
const failed = checks.filter(([, ok]) => !ok);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
process.exit(failed.length ? 1 : 0);
