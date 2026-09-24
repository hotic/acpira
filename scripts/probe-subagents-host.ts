import { cpSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { AgentBlock, PermissionBlock, SessionView, ToolCallBlock, Turn } from '@shared/transcript';
import type { SubagentSummary } from '@shared/subagents';
import { Host } from './lib/host';

// Subagent end-to-end through the production host path (the Rust sidecar over the envelope protocol → real CLI):
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-subagents-host.ts <claude|codex|devin>
// claude and codex run through the built-in registry (the official adapters resolve from PATH — point it at the
// local install, e.g. PATH=/tmp/acp-adapters/node_modules/.bin:$PATH); devin runs through the account layer,
// binding the local CLI login.
// One prompt asks for two parallel subagents; the script then observes each node's transcript, prints summaries and
// transcripts, and re-opens the transcript store to verify the persisted node states. Permission cards — root or child —
// are auto-allowed like a click.
const [agentId = 'claude'] = process.argv.slice(2);
if (agentId !== 'claude' && agentId !== 'codex' && agentId !== 'devin') { console.error('usage: probe-subagents-host.ts <claude|codex|devin>'); process.exit(1); }
const src = fileURLToPath(new URL('../src', import.meta.url));
const project = mkdtempSync(join(tmpdir(), `acpira-sub-${agentId}-project-`));
cpSync(join(src, 'shared'), join(project, 'src/shared'), { recursive: true });
cpSync(join(src, 'host/store'), join(project, 'src/host/store'), { recursive: true });
const store = mkdtempSync(join(tmpdir(), `acpira-sub-${agentId}-store-`));
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const until = async (pred: () => boolean, ms: number, what: string) => {
  const t0 = Date.now();
  while (!pred()) { if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`); await new Promise(r => setTimeout(r, 50)); }
};
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const blockTag = (b: AgentBlock) =>
  `${b.type === 'tool_call' ? b.kind : b.type}:${b.type === 'tool_call' ? b.status : '-'}:${'id' in b ? b.id : '-'}`;
const turnTags = (turns: Turn[]) => turns.map((t, i) => ({
  turn: i, role: t.role,
  blocks: t.role === 'agent' ? t.blocks.map(b => blockTag(b)) : [t.text?.slice(0, 60) ?? '-'],
}));

// Built-in registry: claude / codex resolve the official adapters from PATH, devin the local CLI
const m = await Host.start({ cwd: project, home: store, defaultAgent: agentId });
// `subagent` messages are per view (each webview observes its own child)
const viewer = await m.view();
if (agentId === 'devin') {
  const action = await viewer.addAccount('devin', 'import');
  console.log('account:', action.status === 'success' ? 'imported the local login' : `NO LOCAL LOGIN (${action.status}${action.error ? `: ${action.error}` : ''})`);
  if (action.status !== 'success') { await m.dispose(); process.exit(1); }
}
const childStreams = new Map<string, { rev: number; running: boolean; turns: Turn[] }>();
viewer.onSubagent(ev => childStreams.set(`${ev.sessionId}:${ev.subagentId}`, { rev: ev.rev, running: ev.running, turns: ev.turns }));

// Permission cards live on the root turn or inside a child node's summary; both answer through the same message
const approver = setInterval(() => {
  const cur = viewer.active();
  if (!cur) return;
  const cards: PermissionBlock[] = [
    ...(lastAgent(cur)?.blocks ?? []).filter((b): b is PermissionBlock => b.type === 'permission'),
    ...(cur.subagents ?? []).flatMap(n => n.permissions ?? []),
  ];
  for (const p of cards) {
    const opt = p.options.find(o => o.kind === 'allow_once') ?? p.options[0];
    if (!opt) continue;
    console.log(`approving: ${p.title} → ${opt.id}(${opt.kind})`);
    void viewer.handle({ type: 'permission', sessionId: cur.id, blockId: p.id, optionId: opt.id });
  }
}, 100);

const deadline = setTimeout(() => { console.log('deadline: 300 s budget spent'); void m.dispose().finally(() => process.exit(2)); }, 300_000);
const PROMPT = agentId === 'codex'
  ? 'Spawn two subagents in parallel (use your agent-spawning / collaboration capability, not sequential work): 1) one lists the files directly inside the `src/shared` directory and reports the count; 2) one lists the files directly inside the `src/host/store` directory and reports the count. Do not modify any files. When both report back, reply with the two counts on one line.'
  : 'Use two subagents in parallel, each with a short task: 1) list the files directly inside the `src/shared` directory and report how many there are; 2) list the files directly inside the `src/host/store` directory and report how many there are. Do not modify any files. When both return, reply with the two counts on one line.';
let sessionId: string | undefined;
let liveNodes: SubagentSummary[] = [];

try {
  await viewer.newSession(agentId);
  await until(() => ['ready', 'error', 'auth_required', 'readonly'].includes(viewer.active()?.status ?? ''), 90_000, 'session start');
  let v = viewer.active()!;
  sessionId = v.id;
  console.log('status:', v.status, v.error ?? '');
  check('session ready', v.status === 'ready', v.error);
  if (v.status !== 'ready') throw new Error('not ready');

  await viewer.handle({ type: 'send', text: PROMPT });
  v = viewer.active()!;
  const nodes: SubagentSummary[] = v.subagents ?? [];
  liveNodes = nodes;
  console.log('nodes:', nodes.length);
  for (const n of nodes) {
    const { permissions, question, ...rest } = n;
    console.log('node:', JSON.stringify(rest));
  }
  check('>= 2 subagent nodes', nodes.length >= 2, String(nodes.length));
  check('all nodes terminal', nodes.every(n => n.state !== 'running'), nodes.filter(n => n.state === 'running').map(n => n.id).join(' '));
  const localNodes = nodes.filter(n => n.stateSource === 'local');
  check('no locally-synthesized state', localNodes.length === 0, localNodes.map(n => `${n.id}(${n.state})`).join(' '));

  const root = lastAgent(v);
  const rootTools = (root?.blocks ?? []).filter((b): b is ToolCallBlock => b.type === 'tool_call');
  console.log('root blocks:', JSON.stringify((root?.blocks ?? []).map(b => `${blockTag(b)}${b.type === 'tool_call' && b.subagentId ? `(${b.subagentId})` : ''}`)));

  // Observe each node until its transcript event arrives
  const childToolIds = new Set<string>();
  for (const n of nodes) {
    const key = `${v.id}:${n.id}`;
    const before = childStreams.get(key)?.rev ?? -1;
    await viewer.handle({ type: 'observeSubagent', sessionId: v.id, subagentId: n.id });
    await until(() => (childStreams.get(key)?.rev ?? -1) > before || (childStreams.get(key)?.turns.length ?? 0) > 0, 5_000, `transcript for ${n.id}`).catch(e => console.log('observe timeout:', n.id, e instanceof Error ? e.message : e));
    const stream = childStreams.get(key);
    console.log(`child ${n.id} transcript:`, JSON.stringify(turnTags(stream?.turns ?? [])));
    for (const t of stream?.turns ?? []) {
      if (t.role !== 'agent') continue;
      for (const b of t.blocks) if (b.type === 'tool_call') childToolIds.add(b.id);
    }
    check(`child ${n.id} has >= 1 tool_call`, childToolIds.size > 0 && (stream?.turns ?? []).some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'tool_call')));
  }
  const leaked = rootTools.filter(b => childToolIds.has(b.id));
  check('no child tool_call on the root turn', leaked.length === 0, leaked.map(b => b.id).join(' '));

  if (agentId === 'claude') {
    check('all nodes session visibility', nodes.every(n => n.visibility === 'session'), nodes.map(n => n.visibility).join(' '));
    check('all nodes peer.sessionId', nodes.every(n => n.peer.sessionId !== undefined));
    check('all nodes completed', nodes.every(n => n.state === 'completed'), nodes.map(n => `${n.id}:${n.state}`).join(' '));
    check('all nodes model', nodes.every(n => n.model !== undefined), nodes.map(n => n.model).join(' '));
    const bare = rootTools.filter(b => b.subagentId === undefined);
    check('root tool_calls all carry subagentId', bare.length === 0, bare.map(b => `${b.id}(${b.kind})`).join(' '));
  } else if (agentId === 'codex') {
    // codex-acp's wire is the same subagent_spawned / subagent_state_update pair claude sends; capabilities {}
    // means no per-child stop control, and nothing links the spawn to a root tool row (peer.toolCallId stays empty)
    check('all nodes session visibility', nodes.every(n => n.visibility === 'session'), nodes.map(n => n.visibility).join(' '));
    check('all nodes peer.sessionId', nodes.every(n => n.peer.sessionId !== undefined));
    check('all nodes terminal', nodes.every(n => n.state !== 'running'), nodes.map(n => `${n.id}:${n.state}`).join(' '));
    check('no per-child stop control (capabilities {})', nodes.every(n => n.controls.cancel === false), nodes.map(n => `${n.id}:${n.controls.cancel}`).join(' '));
    // A generation reopen must land as a fresh node, never an orphan or a wrongly-failed one
    const gens = nodes.filter(n => /:generation:\d+$/.test(n.peer.sessionId ?? ''));
    if (gens.length) check('generation reopen nodes are well-formed', gens.every(n => n.state !== 'failed' || n.stateSource === 'agent'), gens.map(n => `${n.peer.sessionId}:${n.state}`).join(' '));
  } else {
    check('all nodes nested visibility', nodes.every(n => n.visibility === 'nested'), nodes.map(n => n.visibility).join(' '));
    check('all nodes role', nodes.every(n => n.role !== undefined), nodes.map(n => n.role).join(' '));
    check('all nodes model', nodes.every(n => n.model !== undefined), nodes.map(n => n.model).join(' '));
    check('all nodes result', nodes.every(n => n.result !== undefined));
    check('all nodes background', nodes.every(n => n.background === true), nodes.map(n => String(n.background)).join(' '));
    const titles = new Set(nodes.map(n => n.title));
    const awaiting = rootTools.find(b => b.verbKey === 'verb.awaitSubagent' && b.target !== undefined && titles.has(b.target));
    check('await-subagent row on root turn', awaiting !== undefined, awaiting ? `${awaiting.id} → ${awaiting.target}` : 'none');
    const delegations = rootTools.filter(b => b.id.startsWith('run_subagent:') || b.verbKey === 'verb.delegate');
    check('delegation rows carry subagentId', delegations.length >= nodes.length && delegations.every(b => b.subagentId !== undefined), delegations.map(b => `${b.id}:${b.subagentId}`).join(' '));
  }
  console.log('usage:', JSON.stringify(v.usage ?? null));
} catch (e) {
  console.log('probe aborted:', e instanceof Error ? e.message : e);
  check('probe ran to the end', false, e instanceof Error ? e.message : String(e));
} finally {
  clearInterval(approver);
  clearTimeout(deadline);
  await m.dispose();
}

// Persistence: the store survives the session; node ids and states must round-trip
const saved = sessionId ? await m.record(sessionId) as { subagents?: SubagentSummary[] } | null : null;
console.log('persisted:', JSON.stringify(saved?.subagents?.map(n => ({ id: n.id, state: n.state, source: n.stateSource })) ?? 'no record'));
check('persisted nodes match live ids + states',
  saved !== null && (saved.subagents ?? []).length === liveNodes.length
    && liveNodes.every(n => saved.subagents!.some(s => s.id === n.id && s.state === n.state)),
  `live ${liveNodes.map(n => `${n.id}:${n.state}`).join(' ')} vs saved ${saved?.subagents?.map(n => `${n.id}:${n.state}`).join(' ') ?? 'none'}`);

const failed = checks.filter(c => !c[1]);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
console.log('store:', store);
console.log('project:', project);
console.log('log tail:\n' + m.logs.filter(l => !/^stderr: /.test(l) || /error|warn|fail|subagent/i.test(l)).slice(-30).join('\n'));
process.exit(failed.length ? 1 : 0);
