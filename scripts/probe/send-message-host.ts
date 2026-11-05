import { cpSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { SessionView, ToolCallBlock, Turn } from '@acpira/shared/transcript';
import type { SubagentSummary } from '@acpira/shared/subagents';
import { Host, type View } from '../lib/host';

// Claude's SendMessage resumes of a finished background subagent, end to end through the sidecar and claude-agent-acp:
//   pnpm exec tsx scripts/probe/send-message-host.ts
// 1. one prompt launches a background subagent; 2. a SendMessage resumes it in the same adapter process; 3. the host is
// restarted (the session comes back through session/resume in a new adapter process) and a SendMessage resumes it
// again. After each resume the SendMessage row must hold the run's node, the run must end, and its transcript must read
// as the agent's thread with the run's own work in it. Spends model calls; permission cards are auto-allowed
const repo = fileURLToPath(new URL('../..', import.meta.url));
const project = mkdtempSync(join(tmpdir(), 'acpira-sendmsg-project-'));
cpSync(join(repo, 'packages/shared'), join(project, 'src/shared'), { recursive: true });
cpSync(join(repo, 'apps/vscode/src/store'), join(project, 'src/host/store'), { recursive: true });
const store = mkdtempSync(join(tmpdir(), 'acpira-sendmsg-store-'));
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const until = async (pred: () => boolean, ms: number, what: string) => {
  const t0 = Date.now();
  while (!pred()) { if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`); await new Promise(r => setTimeout(r, 200)); }
};
const agentBlocks = (v: SessionView) => v.turns.flatMap(t => t.role === 'agent' ? t.blocks : []);
const tools = (v: SessionView) => agentBlocks(v).filter((b): b is ToolCallBlock => b.type === 'tool_call');
const turnTags = (turns: Turn[]) => turns.map(t => t.role === 'user'
  ? `user: ${t.text.slice(0, 50).replace(/\n/g, ' ')}`
  : `agent: ${t.blocks.map(b => b.type === 'tool_call' ? `${b.kind}(${b.target ?? ''})` : b.type).join(' ')}`);

const PROMPT_LAUNCH = 'Launch exactly one subagent with the Agent tool: run_in_background true, subagent_type general-purpose, description '
  + '"File counter", prompt: "Use the Glob tool with the pattern src/shared/* and reply with only the number of entries it found." '
  + 'Do no other work yourself. Wait until it reports, then reply with its number only.';
const resumePrompt = (pattern: string) => 'Use the SendMessage tool to resume the "File counter" subagent you launched earlier '
  + `(its agent id as \`to\`) with this message: "Now use the Glob tool with the pattern ${pattern} and reply with only the number of entries it found." `
  + 'Do not do the work yourself. Wait until it reports, then reply with its number only.';

const autoApprove = (viewer: View) => {
  const answered = new Set<string>();
  return setInterval(() => {
  const cur = viewer.active();
  if (!cur) return;
  const cards = [...agentBlocks(cur).filter(b => b.type === 'permission'), ...(cur.subagents ?? []).flatMap(n => n.permissions ?? [])];
  for (const p of cards) {
    if (p.type !== 'permission' || answered.has(p.id)) continue;
    answered.add(p.id);
    const opt = p.options.find(o => o.kind === 'allow_once') ?? p.options[0];
    if (opt) void viewer.handle({ type: 'permission', sessionId: cur.id, blockId: p.id, optionId: opt.id });
  }
  }, 200);
};

// The SendMessage row of the last turn, its node, and that node's transcript once the run ended
async function checkResume(label: string, viewer: View, transcripts: Map<string, Turn[]>, firstRun: SubagentSummary) {
  let v = viewer.active()!;
  const turn = v.turns[v.turns.length - 1];
  const rows = turn?.role === 'agent' ? turn.blocks.filter((b): b is ToolCallBlock => b.type === 'tool_call' && b.target === 'SendMessage') : [];
  console.log(`${label} SendMessage rows:`, JSON.stringify(rows.map(r => ({ id: r.id, status: r.status, subagentId: r.subagentId }))));
  const row = rows.find(r => r.subagentId !== undefined) ?? rows[rows.length - 1];
  check(`${label}: a SendMessage call was made`, row !== undefined);
  if (!row) return;
  check(`${label}: the SendMessage row holds a subagent`, row.subagentId !== undefined);
  const nodeOf = () => viewer.active()!.subagents?.find(n => n.id === row.subagentId);
  if (!row.subagentId || !nodeOf()) return;
  await until(() => nodeOf()?.state !== 'running', 300_000, `${label} run to end`).catch(e => console.log(String(e)));
  v = viewer.active()!;
  const node = nodeOf()!;
  const { permissions, question, ...rest } = node;
  console.log(`${label} node:`, JSON.stringify(rest));
  check(`${label}: the run continues the first agent`, node.peer.sidechainId === firstRun.peer.sessionId, `${node.peer.sidechainId} vs ${firstRun.peer.sessionId}`);
  check(`${label}: the run completed`, node.state === 'completed', node.state);
  check(`${label}: no stand-in row beside it`, !tools(v).some(b => b.id === `subagent:${node.id}`));
  const key = `${v.id}:${node.id}`;
  await viewer.handle({ type: 'observeSubagent', sessionId: v.id, subagentId: node.id });
  await until(() => (transcripts.get(key)?.some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'tool_call')) ?? false), 15_000, `${label} transcript`).catch(e => console.log(String(e)));
  const turns = transcripts.get(key) ?? [];
  console.log(`${label} transcript:`, JSON.stringify(turnTags(turns), null, 1));
  const users = turns.filter(t => t.role === 'user').length;
  check(`${label}: the transcript reads as the agent's thread`, users >= 2, `${users} user turns`);
  const last = turns.reduce((at, t, i) => (t.role === 'user' ? i : at), -1);
  check(`${label}: the run's own work is in it`, turns.slice(last + 1).some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'tool_call')));
  // Claude Code writes the closing reply after the run is reported over
  await until(() => (transcripts.get(key) ?? []).slice(last + 1).some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'text')), 15_000, `${label} closing reply`).catch(e => console.log(String(e)));
  check(`${label}: the run's closing reply is in it`, (transcripts.get(key) ?? []).slice(last + 1).some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'text')));
  await until(() => nodeOf()?.result !== undefined, 15_000, `${label} result`).catch(e => console.log(String(e)));
  check(`${label}: the run has a result`, nodeOf()?.result !== undefined, nodeOf()?.result);
  const head = viewer.active()!.subagents?.find(n => n.id === firstRun.id);
  check(`${label}: the first run keeps its own task`, head?.task === firstRun.task, head?.task?.slice(0, 60));
}

const hosts: Host[] = [];
const deadline = setTimeout(() => { console.log('deadline: 15 min budget spent'); void Promise.all(hosts.map(h => h.dispose())).finally(() => process.exit(2)); }, 900_000);
const logs: string[] = [];
try {
  const h1 = await Host.start({ cwd: project, home: store, defaultAgent: 'claude' });
  hosts.push(h1);
  const m1 = await h1.view();
  const t1 = new Map<string, Turn[]>();
  m1.onSubagent(ev => t1.set(`${ev.sessionId}:${ev.subagentId}`, ev.turns));
  const a1 = autoApprove(m1);
  await m1.newSession('claude');
  await m1.until(() => ['ready', 'error', 'auth_required'].includes(m1.active()?.status ?? ''), 90_000, 'session start');
  check('session ready', m1.active()!.status === 'ready', m1.active()!.error);
  const localId = m1.activeId!;
  await m1.handle({ type: 'send', text: PROMPT_LAUNCH });
  const first = m1.active()!.subagents?.find(n => n.visibility === 'session' && n.peer.sidechainId === undefined);
  console.log('first run:', JSON.stringify(first && { id: first.id, state: first.state, peer: first.peer, title: first.title }));
  check('a background subagent ran', first !== undefined && first.state === 'completed', first?.state);
  if (!first) throw new Error('no subagent');
  await m1.handle({ type: 'send', text: resumePrompt('src/host/store/*') });
  await checkResume('in-process resume', m1, t1, first);
  clearInterval(a1);
  logs.push(...h1.logs);
  await h1.dispose();

  // A new host: the session comes back through session/resume in a new adapter process
  const h2 = await Host.start({ cwd: project, home: store, defaultAgent: 'claude' });
  hosts.push(h2);
  const m2 = await h2.view();
  const t2 = new Map<string, Turn[]>();
  m2.onSubagent(ev => t2.set(`${ev.sessionId}:${ev.subagentId}`, ev.turns));
  const a2 = autoApprove(m2);
  await m2.selectSession(localId);
  await m2.until(() => ['ready', 'error', 'readonly'].includes(m2.active()?.status ?? ''), 90_000, 'restore');
  check('restored through session/resume', h2.logs.some(l => l.includes('session/resume ok')));
  await m2.handle({ type: 'send', text: resumePrompt('src/shared/*.ts') });
  await checkResume('resume after restart', m2, t2, first);
  console.log('engine lines:', h2.logs.filter(l => /resumed without an announce|sidechain|dropping unattributed/.test(l)).slice(0, 5).join(' | '));
  clearInterval(a2);
  logs.push(...h2.logs);
} catch (e) {
  check('probe ran to the end', false, e instanceof Error ? e.message : String(e));
} finally {
  clearTimeout(deadline);
  await Promise.all(hosts.map(h => h.dispose()));
}
const failed = checks.filter(c => !c[1]);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
console.log('store:', store, '· project:', project);
process.exit(failed.length ? 1 : 0);
