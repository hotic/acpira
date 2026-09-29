import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { RawAgent, RpcError, type RpcMessage } from './lib/rawAcp';
import { builtinAgent } from './lib/sidecarBin';

// Raw-wire `_session/steering` probe: what an adapter really does with a steer, mid-turn and idle. Spends real model calls.
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-steering.ts <claude|codex|…> [--model TEXT] [--idle-wait MS] [--out FILE]
//
// --model: switch the session's model configOption to the first value whose id or name contains TEXT (case-insensitive)
//
// 1. initialize: prints `_meta.steering`
// 2. mid-turn: a prompt that runs `sleep 10` through the agent's shell tool; once the first tool call shows up, a steer asks the
//    final reply to end with a marker word. Checks: the outcome, that the one `session/prompt` settles once, and whether the
//    reply after the steer carries the marker
// 3. idle: a steer with `idleBehavior: promptRequired` after the turn settled. `promptRequired` is the contract; `startedNewTurn`
//    means the adapter started a turn nobody's `session/prompt` owns, so every update after it is logged with its time
//    (--idle-wait, default 30000) to see whether anything marks that turn's end
// 4. a plain prompt afterwards, to see the session still takes one
//
// The temp cwd is empty; permission requests are allowed once. The JSONL log (strings clipped) lands in ~/.acpira/probe/.

const argv = process.argv.slice(2);
const valued = (f: string) => { const i = argv.indexOf(f); return i >= 0 ? argv[i + 1] : undefined; };
const agentId = argv.find(a => !a.startsWith('--') && ![valued('--idle-wait'), valued('--out'), valued('--model')].includes(a)) ?? 'claude';
const model = valued('--model');
const idleWait = Number(valued('--idle-wait') ?? 30_000);
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const outFile = valued('--out') ?? join(homedir(), '.acpira', 'probe', `steering-${agentId}-${stamp}.jsonl`);
await mkdir(join(outFile, '..'), { recursive: true });
const cwd = await mkdtemp(join(tmpdir(), 'acpira-steer-'));
await writeFile(join(cwd, 'README.md'), '# steering probe\n');

const MARKER = 'PINEAPPLE';
const t0 = Date.now();
const at = () => `${((Date.now() - t0) / 1000).toFixed(2)}s`;
const clip = (v: unknown): unknown => typeof v === 'string' ? (v.length > 200 ? `${v.slice(0, 200)}…` : v)
  : Array.isArray(v) ? v.map(clip) : v && typeof v === 'object' ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, clip(x)])) : v;
const log: string[] = [];
const note = (what: string, extra?: unknown) => {
  console.log(`\x1b[36m[${at()}] ${what}\x1b[0m${extra === undefined ? '' : ` ${JSON.stringify(clip(extra))}`}`);
  log.push(JSON.stringify({ t: Date.now() - t0, note: what, extra: clip(extra) }));
};

// What the root session streamed, split around the steer so the reply to it can be checked on its own
type Phase = 'before' | 'after' | 'idle' | 'final';
let phase = 'before' as Phase;
const text: Record<Phase, string> = { before: '', after: '', idle: '', final: '' };
const updates: { t: number; phase: string; kind: string }[] = [];
let firstTool: (() => void) | undefined;

const def = builtinAgent(agentId);
const agent = new RawAgent(def.spawn!, cwd, def.env ?? {}, {
  onMessage: (dir: '→' | '←', msg: RpcMessage) => log.push(JSON.stringify({ t: Date.now() - t0, dir, msg: clip(msg) })),
  onStderr: l => log.push(JSON.stringify({ t: Date.now() - t0, dir: 'stderr', line: l.slice(0, 300) })),
  onRequest: (method, params) => {
    if (method === 'session/request_permission') {
      const options = (params.options ?? []) as { optionId: string; kind: string }[];
      const allow = options.find(o => o.kind === 'allow_once') ?? options[0];
      note('permission → allow once', (params.toolCall as { title?: string } | undefined)?.title);
      return allow ? { outcome: { outcome: 'selected', optionId: allow.optionId } } : { outcome: { outcome: 'cancelled' } };
    }
    if (method === 'elicitation/create') return { action: 'cancel' };
    throw new RpcError(-32601, `Method not found: ${method}`);
  },
  onNotification: (method, params) => {
    if (method !== 'session/update') return;
    const u = (params.update ?? {}) as Record<string, unknown>;
    const kind = String(u.sessionUpdate);
    updates.push({ t: Date.now() - t0, phase, kind });
    if (kind === 'agent_message_chunk') {
      const c = u.content as { type?: string; text?: string } | undefined;
      if (c?.type === 'text') { text[phase] += c.text ?? ''; process.stdout.write(c.text ?? ''); }
    } else if (kind === 'tool_call') {
      note(`tool_call ${String(u.title ?? '')}`);
      firstTool?.();
      firstTool = undefined;
    } else if (kind === 'user_message_chunk') {
      note('user_message_chunk (echo)', (u.content as { text?: string } | undefined)?.text);
    } else if (phase === 'idle' && kind !== 'agent_thought_chunk') {
      note(`idle update ${kind}`, u);
    }
  },
});

const summary: Record<string, unknown> = { agent: agentId, cwd };
async function finish(code: number) {
  const byPhase = (p: string) => updates.filter(u => u.phase === p);
  summary.updateKinds = Object.fromEntries((['before', 'after', 'idle', 'final'] as const).map(p => [p, [...new Set(byPhase(p).map(u => u.kind))]]));
  summary.lastIdleUpdateAt = byPhase('idle').at(-1)?.t;
  log.push(JSON.stringify({ summary }));
  await writeFile(outFile, `${log.join('\n')}\n`);
  console.log(`\n\n=== summary ===\n${JSON.stringify(summary, null, 2)}\nlog: ${outFile}`);
  await agent.kill();
  process.exit(code);
}
for (const sig of ['SIGINT', 'SIGTERM'] as const) process.once(sig, () => void finish(130));

const steer = (sessionId: string, message: string) => agent.request<Record<string, unknown>>('_session/steering', {
  sessionId, prompt: [{ type: 'text', text: message }], _meta: { steering: { idleBehavior: 'promptRequired' } },
});

try {
  const init = await agent.request<Record<string, unknown>>('initialize', {
    protocolVersion: 1, clientInfo: { name: 'acpira-probe', version: '0' },
    clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
  });
  const info = init.agentInfo as { name?: string; version?: string } | undefined;
  summary.agentInfo = info;
  summary.steeringMeta = (init._meta as { steering?: unknown } | undefined)?.steering ?? null;
  note('initialize', { agentInfo: info, steering: summary.steeringMeta });
  type Option = { id: string; category?: string; currentValue?: unknown; options?: ({ value: string; name: string } | { options: { value: string; name: string }[] })[] };
  const created = await agent.request<{ sessionId: string; configOptions?: Option[] }>('session/new', { cwd, mcpServers: [] });
  const { sessionId } = created;
  const modelOption = created.configOptions?.find(o => o.category === 'model' || o.id === 'model');
  note(`session/new ${sessionId}`, { model: modelOption?.currentValue });
  if (model && modelOption) {
    const values = (modelOption.options ?? []).flatMap(o => ('options' in o ? o.options : [o]));
    const pick = values.find(v => `${v.value} ${v.name}`.toLowerCase().includes(model.toLowerCase()));
    if (!pick) throw new Error(`no model matching ${model}: ${values.map(v => v.value).join(', ')}`);
    await agent.request('session/set_config_option', { sessionId, configId: modelOption.id, value: pick.value });
    note(`model → ${pick.value}`);
  }
  summary.model = model ?? modelOption?.currentValue;

  // Mid-turn
  const toolSeen = new Promise<void>(resolve => { firstTool = resolve; });
  const promptSent = Date.now();
  let prompts = 0;
  const running = agent.request<Record<string, unknown>>('session/prompt', { sessionId, prompt: [{ type: 'text', text:
    'Run exactly this shell command and wait for it to finish: `sleep 10 && echo sleep-done`. Then reply with one short sentence saying what it printed.' }] })
    .then(r => { prompts++; note('session/prompt answered', r); return r; });
  running.catch(() => {});
  await Promise.race([toolSeen, new Promise(r => setTimeout(r, 20_000))]);
  await new Promise(r => setTimeout(r, 1500));
  phase = 'after';
  const steerSent = Date.now();
  const mid = await steer(sessionId, `Additional instruction from the user: end your final reply with the exact word ${MARKER}.`)
    .catch((e: unknown) => ({ error: e instanceof RpcError ? { code: e.code, message: e.message } : String(e) }));
  const steerAnsweredMs = Date.now() - steerSent;
  note('mid-turn steer answered', mid);
  const r1 = await running;
  summary.midTurn = {
    steerOutcome: mid,
    steerAnsweredMs,
    stopReason: r1.stopReason,
    promptMs: Date.now() - promptSent,
    promptResponses: prompts,
    markerInReplyAfterSteer: text.after.includes(MARKER),
    replyAfterSteer: text.after.slice(-300),
  };

  // Idle
  await new Promise(r => setTimeout(r, 1500));
  phase = 'idle';
  const idle = await steer(sessionId, `Reply with just the word IDLE-${MARKER}.`)
    .catch((e: unknown) => ({ error: e instanceof RpcError ? { code: e.code, message: e.message } : String(e) }));
  note('idle steer answered', idle);
  await new Promise(r => setTimeout(r, (idle as { outcome?: string }).outcome === 'startedNewTurn' ? idleWait : 3000));
  summary.idle = { steerOutcome: idle, textAfter: text.idle.slice(-300), updates: updates.filter(u => u.phase === 'idle').length };

  // The session afterwards
  phase = 'final';
  const finalSent = Date.now();
  const r2 = await agent.request<Record<string, unknown>>('session/prompt', { sessionId, prompt: [{ type: 'text', text: 'Reply with just OK.' }] });
  summary.after = { stopReason: r2.stopReason, ms: Date.now() - finalSent, text: text.final.slice(-200) };
  await finish(0);
} catch (e) {
  console.error('\nprobe failed', e);
  summary.failure = String(e);
  await finish(1);
}
