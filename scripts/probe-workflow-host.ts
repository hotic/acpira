import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { PermissionBlock, SessionView, ToolCallBlock } from '@shared/transcript';
import { Host } from './lib/host';

// A Claude dynamic workflow (ultracode) end-to-end through the production host path (the Rust sidecar → claude-agent-acp):
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-workflow-host.ts [--model ID] [--effort LEVEL] [--out DIR]
// --agent codex sets Codex's `ultra` effort instead and asks for two tiny delegated children. One deliberately tiny ultracode prompt (two agents, one word each) on a cheap model / low effort; permission cards are
// allowed like a click. The script waits for the workflow agents to appear as subagent nodes, for the run to end after the
// prompt returned, and writes the SessionView seen while the agents ran and the final one (`running.json` / `final.json`,
// which the LAB page `workflow-tree` renders). Spends real model calls.
const args = process.argv.slice(2);
const opt = (name: string, fallback: string) => { const i = args.indexOf(name); return i >= 0 && args[i + 1] ? args[i + 1]! : fallback; };
const agentId = opt('--agent', 'claude') as 'claude' | 'codex';
const model = opt('--model', agentId === 'codex' ? 'gpt-6-astra' : 'opus');
const effort = opt('--effort', agentId === 'codex' ? 'ultra' : 'low');
const out = opt('--out', join(tmpdir(), 'acpira-workflow'));
mkdirSync(out, { recursive: true });

const project = mkdtempSync(join(tmpdir(), 'acpira-workflow-project-'));
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const save = (name: string, v: SessionView) => { writeFileSync(join(out, name), JSON.stringify(v, null, 2)); console.log(`saved ${join(out, name)}`); };

const m = await Host.start({ cwd: project, defaultAgent: agentId });
const view = await m.view();
const deadline = setTimeout(() => { console.log('deadline: 300 s budget spent'); void m.dispose().finally(() => process.exit(2)); }, 300_000);

// Permission cards (the Workflow tool asks once) are answered like a click, once each
const answered = new Set<string>();
const approver = setInterval(() => {
  const cur = view.active();
  const last = cur?.turns[cur.turns.length - 1];
  if (!cur || last?.role !== 'agent') return;
  for (const p of last.blocks.filter((b): b is PermissionBlock => b.type === 'permission' && !answered.has(b.id))) {
    const o = p.options.find(x => x.kind === 'allow_once') ?? p.options[0];
    if (!o) continue;
    answered.add(p.id);
    console.log(`approving: ${p.title} → ${o.id}`);
    void view.handle({ type: 'permission', sessionId: cur.id, blockId: p.id, optionId: o.id });
  }
}, 100);

const nodes = () => (view.active()?.subagents ?? []).filter(n => n.peer.agentId?.includes('#'));
const workflowRow = (): ToolCallBlock | undefined => view.active()?.turns.flatMap(t => t.role === 'agent' ? t.blocks : [])
  .find((b): b is ToolCallBlock => b.type === 'tool_call' && b.asyncTask?.taskType === 'workflow');

try {
  await view.newSession(agentId);
  await view.until(() => ['ready', 'error', 'auth_required'].includes(view.active()?.status ?? ''), 90_000, 'session start');
  const v = view.active()!;
  check('session ready', v.status === 'ready', v.error);
  if (v.status !== 'ready') throw new Error('not ready');
  const controls = () => view.active()!.controls?.options ?? [];
  const show = () => controls().map(c => `${c.id}=${String(c.value)}`).join(' ');
  console.log('controls:', show());
  for (const c of controls()) console.log(`  ${c.id}: ${c.options.map(o => o.id).join(', ')}`);
  // Pick by value or by a substring of it (`opus` → the catalogue's opus entry), and wait for the control to show it
  for (const [id, want] of [['model', model], [agentId === 'codex' ? 'reasoning_effort' : 'effort', effort]] as const) {
    const c = controls().find(x => x.id === id);
    const values = c?.options.map(o => o.id) ?? [];
    const value = values.find(x => x === want) ?? values.find(x => x.includes(want));
    if (!value) throw new Error(`${id}: no option matches ${want} (${values.join(', ')})`);
    // A draft shows the remembered controls before its process attaches, and a pick made then is dropped: retry until it sticks
    for (let i = 0; controls().find(x => x.id === id)?.value !== value; i++) {
      if (i === 10) throw new Error(`${id}=${value} never applied`);
      view.post({ type: 'setConfig', configId: id, value });
      await view.until(() => controls().find(x => x.id === id)?.value === value, 3000, `${id}=${value}`).catch(() => {});
    }
  }
  console.log('after:', show());
  if (args.includes('--dry')) throw new Error('dry run: no prompt sent');

  if (agentId === 'codex') {
    // Codex `ultra` is an effort level ("maximum reasoning with automatic task delegation"): its delegation is Codex's own
    // subagent spawn, so the nodes are the `session` children the regular subagent path already handles
    const all = () => view.active()?.subagents ?? [];
    const prompt = '简单测试一下 ultra 的自动委派功能，不要浪费额度，仅供测试用：派两个子代理并行，第一个只回复单词 alpha，'
      + '第二个只回复单词 beta，不读文件、不调用任何工具。两个都返回后用一行报告两个结果。';
    const t0 = Date.now();
    let sendError: Error | undefined;
    const sent = view.handle({ type: 'send', text: prompt }, 240_000).catch((e: Error) => { sendError = e; });
    await view.until(() => all().length > 0 || !!sendError || !view.active()?.running, 240_000, 'the first subagent node');
    if (all().length) { check('ultra delegates to subagent nodes', true, `${all().length} after ${Date.now() - t0} ms`); save('codex-running.json', view.active()!); }
    await sent;
    if (sendError) throw sendError;
    await view.until(() => all().every(n => n.state !== 'running'), 120_000, 'the children to end').catch(() => {});
    const fin = view.active()!;
    save('codex-final.json', fin);
    for (const n of all()) console.log(`node ${n.visibility} · ${n.title} · model=${n.model} · state=${n.state} · tools=${n.toolCount} · result=${JSON.stringify(n.result?.slice(0, 80))}`);
    const last = fin.turns[fin.turns.length - 1];
    console.log('reply:', last?.role === 'agent' ? last.blocks.filter(b => b.type === 'text').map(b => (b as { markdown: string }).markdown).join(' ').slice(-200) : '-');
    check('ultra delegated', all().length >= 1, `${all().length} node(s)`);
    check('every child completed', all().length > 0 && all().every(n => n.state === 'completed'));
  } else {
    // The keyword turns ultracode on for this human turn; the prompt keeps the run as small as a workflow gets
    const prompt = 'ultracode 简单测试一下 ultracode 功能，不要浪费额度，仅供测试用：启动一个只有两个 agent 的最小工作流，'
      + '第一个 agent 只回复单词 alpha，第二个只回复单词 beta，不读文件、不调用任何工具。工作流结束后用一行报告两个结果。';
    const t0 = Date.now();
    let sendError: Error | undefined;
    const sent = view.handle({ type: 'send', text: prompt }, 240_000).catch((e: Error) => { sendError = e; });
    await view.until(() => nodes().length > 0 || !!sendError || view.active()?.status === 'error', 240_000, 'the first workflow agent node');
    if (!nodes().length) throw sendError ?? new Error(`no workflow node: ${view.active()?.error ?? 'the turn ended without one'}`);
    check('workflow agents appear as subagent nodes', true, `${nodes().length} after ${Date.now() - t0} ms`);
    save('running.json', view.active()!);
    await sent;
    if (sendError) throw sendError;
    console.log(`prompt returned after ${Date.now() - t0} ms; nodes: ${nodes().map(n => `${n.title}:${n.state}`).join(', ')}`);
    check('no agent is swept disconnected when the prompt returns', !nodes().some(n => n.state === 'disconnected'));
    await view.until(() => {
      const row = workflowRow();
      return !!row && row.asyncTask!.state !== 'running' && nodes().every(n => n.state !== 'running');
    }, 240_000, 'the workflow run to end');
    // The follow-up reply the adapter sends after the run lands a moment later
    await new Promise(r => setTimeout(r, 4000));
    const fin = view.active()!;
    save('final.json', fin);
    const row = workflowRow();
    check('workflow row ends with its task', !!row && row.asyncTask!.state === 'completed', `${row?.status} / ${row?.asyncTask?.state}`);
    check('the workflow row is not claimed by a node', !!row && !row.subagentId);
    for (const n of nodes()) {
      console.log(`node ${n.peer.agentId} · ${n.title} · role=${n.role} · model=${n.model} · state=${n.state} · tools=${n.toolCount} · result=${JSON.stringify(n.result)}`);
    }
    check('every agent completed with a result', nodes().length >= 2 && nodes().every(n => n.state === 'completed' && !!n.result));
  }
} catch (e) {
  check('run', false, (e as Error).message);
  for (const l of m.logs.slice(-25)) console.log(`  sidecar: ${l.slice(0, 300)}`);
  const cur = view.active();
  if (cur) save('error.json', cur);
} finally {
  clearInterval(approver);
  clearTimeout(deadline);
  await m.dispose();
}
const failed = checks.filter(c => !c[1]);
console.log(failed.length ? `\n${failed.length} FAILED` : '\nALL PASS');
process.exit(failed.length ? 1 : 0);
