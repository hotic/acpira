import { mkdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { devinAuthenticate, readDevinLogin } from './lib/devin';
import { RawAgent, RpcError, type RpcMessage } from './lib/rawAcp';
import { builtinAgent, type SpawnSpec } from './lib/sidecarBin';

// Raw-wire subagent probe. Speaks JSON-RPC over stdio itself (no SDK), so nothing an agent sends is validated away:
// the released SDK's session/update parser is a closed union and drops unknown `sessionUpdate` kinds.
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-subagents.ts <agent> [--cmd "<command line>"] [--no-caps] [--air] [--devin-meta] [--wait MS] [--out FILE] [prompt]
//
// <agent>: a built-in id (devin / grok / kimi / opencode / …, launched as the sidecar would: `acpira agents --json`) or a label for --cmd
// --cmd: run this command line instead of the built-in one (e.g. "npx -y @agentclientprotocol/claude-agent-acp@0.78.0")
// --no-caps: do not advertise clientCapabilities.subagents (RFD #1992) — the control run
// --air: also advertise _meta.jetbrains.air = { version: 1, capabilities: ['nativeSubagentSessions', 'sessionFailure', 'asyncTasks'] }
//   — claude-agent-acp's bridge for
//   SDKs that strip the draft `subagents` field (the shape `air-extension.js` `clientSupportsAirCapability` reads)
// --devin-meta: also advertise _meta['cognition.ai/subagentSupport'] = true (a string found in the devin binary; unverified)
// --import-local: Devin's ACP mode refuses local credentials; hand the CLI's own login (~/.local/share/devin/credentials.toml)
//   over through `authenticate` `_meta.api_key` the way the account layer does. The key never leaves this process
// --wait MS: keep listening this long after the prompt response (terminal updates may trail it); default 4000
// --out FILE: where the redacted JSONL log goes; default ~/.acpira/probe/subagents-<agent>-<iso>.jsonl
//
// Every line both ways is logged (strings over 160 chars truncated); permission requests are allowed once, elicitations cancelled.
// The summary at the end lists which session ids carried which update kinds and every _meta key seen — the evidence for
// which visibility tier an agent sits in (receipt / nested transcript / native child sessions).

const argv = process.argv.slice(2);
const flag = (f: string) => argv.includes(f);
const valued = (f: string) => { const i = argv.indexOf(f); return i >= 0 ? { idx: i + 1, value: argv[i + 1] } : undefined; };
const cmdOpt = valued('--cmd'), waitOpt = valued('--wait'), outOpt = valued('--out');
const valueIdx = new Set([cmdOpt, waitOpt, outOpt].flatMap(v => (v ? [v.idx] : [])));
const positional = argv.filter((a, i) => !a.startsWith('--') && !valueIdx.has(i));
const [agentId = 'devin', ...rest] = positional;
const waitMs = Number(waitOpt?.value ?? 4000);
const promptText = rest.join(' ') || [
  'Use two subagents in parallel, each with a short task:',
  '1) list the files directly inside the `src/shared` directory and report how many there are;',
  '2) list the files directly inside the `src/host/store` directory and report how many there are.',
  'Do not modify any files. When both return, reply with the two counts on one line.',
].join(' ');

let spec: SpawnSpec, agentEnv: Record<string, string> = {};
if (cmdOpt?.value) {
  const [command = '', ...args] = cmdOpt.value.split(/\s+/).filter(Boolean);
  spec = { command, args, verbatim: false };
} else {
  const def = builtinAgent(agentId);
  spec = def.spawn!;
  agentEnv = def.env ?? {};
}
const { command, args } = spec;
const stamp = new Date().toISOString().replace(/[:.]/g, '-');
const outFile = outOpt?.value ?? join(homedir(), '.acpira', 'probe', `subagents-${agentId}-${stamp}.jsonl`);
await mkdir(join(outFile, '..'), { recursive: true });

const TEXT_MAX = 160;
function redact(v: unknown): unknown {
  if (typeof v === 'string') return v.length > TEXT_MAX ? `${v.slice(0, TEXT_MAX)}…[${v.length} chars]` : v;
  if (Array.isArray(v)) return v.map(redact);
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v as Record<string, unknown>).map(([k, x]) => [k, k === 'data' && typeof x === 'string' ? `[${x.length} chars]` : redact(x)]));
  return v;
}

const log: string[] = [];
// A secret request (the local login handed over) goes into the log as its method only; the redactor cannot know the key is secret
const record = (dir: '→' | '←', msg: RpcMessage, secret?: boolean) => {
  log.push(JSON.stringify({ t: Date.now(), dir, msg: secret ? { id: msg.id, method: msg.method, params: '[redacted: api_key]' } : redact(msg) }));
};
console.error(`→ ${command} ${args.join(' ')}`);

// Evidence tallies
const kindsBySession = new Map<string, Map<string, number>>();
const metaKeys = new Set<string>();
const requestMethods = new Map<string, number>();
const extensionUpdates: unknown[] = [];
let rootSessionId: string | undefined;

function collectMeta(v: unknown, path = '') {
  if (!v || typeof v !== 'object') return;
  for (const [k, x] of Object.entries(v as Record<string, unknown>)) {
    if (k === '_meta' && x && typeof x === 'object') for (const mk of Object.keys(x as object)) metaKeys.add(`${path}._meta.${mk}`);
    if (x && typeof x === 'object' && k !== '_meta') collectMeta(x, path ? `${path}.${k}` : k);
  }
}

const short = (id: unknown) => (typeof id === 'string' ? id.slice(0, 12) : String(id));

const agent = new RawAgent(spec, process.cwd(), agentEnv, {
  onMessage: record,
  onStderr: l => { console.error(`\x1b[33mstderr\x1b[0m ${l}`); log.push(JSON.stringify({ t: Date.now(), dir: 'stderr', line: l.slice(0, 400) })); },
  onNonJson: line => { console.error(`\x1b[31mnon-json stdout\x1b[0m ${line.slice(0, 200)}`); log.push(JSON.stringify({ t: Date.now(), dir: 'stdout-raw', line: line.slice(0, 400) })); },
  onRequest: (method, params) => {
    collectMeta(params, method);
    requestMethods.set(method, (requestMethods.get(method) ?? 0) + 1);
    console.log(`\x1b[36m[request ${method}]\x1b[0m session ${short(params.sessionId)}${params.sessionId && params.sessionId !== rootSessionId ? ' \x1b[35m(child?)\x1b[0m' : ''} ${JSON.stringify(redact(params)).slice(0, 300)}`);
    if (method === 'session/request_permission') {
      const options = (params.options ?? []) as { optionId: string; kind: string }[];
      const allow = options.find(o => o.kind === 'allow_once') ?? options[0];
      return allow ? { outcome: { outcome: 'selected', optionId: allow.optionId } } : { outcome: { outcome: 'cancelled' } };
    }
    if (method === 'elicitation/create') return { action: 'cancel' };
    throw new RpcError(-32601, `Method not found: ${method}`);
  },
  onNotification: (method, params) => {
    collectMeta(params, method);
    if (method === 'session/update') {
      const sid = String(params.sessionId);
      const u = (params.update ?? {}) as Record<string, unknown>;
      const kind = String(u.sessionUpdate);
      const m = kindsBySession.get(sid) ?? new Map<string, number>();
      m.set(kind, (m.get(kind) ?? 0) + 1);
      kindsBySession.set(sid, m);
      const foreign = rootSessionId !== undefined && sid !== rootSessionId;
      const ext = /subagent|async_task|child|chain/i.test(kind);
      if (ext) extensionUpdates.push({ sessionId: sid, update: redact(u) });
      if (kind === 'agent_message_chunk' && !foreign) {
        const c = u.content as { type?: string; text?: string } | undefined;
        if (c?.type === 'text') process.stdout.write(c.text ?? '');
        return;
      }
      if (kind === 'agent_thought_chunk' && !foreign) return;
      const tag = ext ? '\x1b[32;1m' : foreign ? '\x1b[35m' : '\x1b[2m';
      console.log(`\n${tag}[${kind}]\x1b[0m session ${short(sid)}${foreign ? ' (child)' : ''} ${JSON.stringify(redact(u)).slice(0, ext ? 800 : 240)}`);
      return;
    }
    console.log(`\n\x1b[2m[notification ${method}]\x1b[0m ${JSON.stringify(redact(params)).slice(0, 300)}`);
  },
});
const exited = agent.exited.then(({ code, signal }) => { console.error(`exit code=${code} signal=${signal}`); });

async function finish(exitCode: number) {
  const summary = {
    agent: agentId, command: `${command} ${args.join(' ')}`,
    caps: { subagents: !flag('--no-caps'), air: flag('--air'), devinMeta: flag('--devin-meta') },
    rootSessionId,
    sessions: Object.fromEntries([...kindsBySession].map(([sid, m]) => [sid, Object.fromEntries(m)])),
    requestMethods: Object.fromEntries(requestMethods),
    metaKeys: [...metaKeys].sort(),
    extensionUpdates,
  };
  log.push(JSON.stringify({ t: Date.now(), dir: 'summary', summary }));
  await writeFile(outFile, log.join('\n') + '\n');
  console.log('\n\n=== summary ===');
  console.log(JSON.stringify(summary, null, 2));
  console.log(`\nlog: ${outFile}`);
  await Promise.race([agent.kill(), exited.then(() => {}), new Promise(r => setTimeout(r, 6000))]);
  process.exit(exitCode);
}

// A hung upstream (a gateway retrying 503s) still leaves its evidence: write what was captured before dying
for (const sig of ['SIGINT', 'SIGTERM'] as const) process.once(sig, () => { console.error(`\n${sig}: writing partial log`); void finish(130); });

try {
  const meta: Record<string, unknown> = {};
  // The same AIR block AgentProcess sends — the raw probe exists to observe what an agent emits given the capability
  if (flag('--air')) meta.jetbrains = { air: { version: 1, capabilities: ['nativeSubagentSessions', 'sessionFailure', 'asyncTasks'] } };
  if (flag('--devin-meta')) meta['cognition.ai/subagentSupport'] = true;
  const clientCapabilities: Record<string, unknown> = {
    fs: { readTextFile: false, writeTextFile: false },
    terminal: false,
    ...(flag('--no-caps') ? {} : { subagents: {} }),
    ...(Object.keys(meta).length ? { _meta: meta } : {}),
  };
  const init = await agent.request<Record<string, unknown>>('initialize', { protocolVersion: 1, clientInfo: { name: 'acpira-probe', version: '0' }, clientCapabilities });
  collectMeta(init, 'initialize.result');
  console.log('initialize →', JSON.stringify(redact(init), null, 1).slice(0, 1500));
  if (flag('--import-local')) {
    const login = await readDevinLogin();
    if (!login) throw new Error('no local devin login (credentials.toml)');
    const methods = (init.authMethods ?? []) as { id: string }[];
    await agent.request('authenticate', devinAuthenticate(methods[0]?.id, login), { secret: true });
    console.log('authenticate ok (local login)');
  }
  const s = await agent.request<{ sessionId: string; [k: string]: unknown }>('session/new', { cwd: process.cwd(), mcpServers: [] });
  rootSessionId = s.sessionId;
  console.log(`session/new → ${s.sessionId}`);
  console.log(`\nprompt → ${promptText}\n`);
  const started = Date.now();
  const r = await agent.request<Record<string, unknown>>('session/prompt', { sessionId: s.sessionId, prompt: [{ type: 'text', text: promptText }] });
  collectMeta(r, 'prompt.result');
  console.log(`\n\nprompt done in ${((Date.now() - started) / 1000).toFixed(1)}s → ${JSON.stringify(redact(r)).slice(0, 400)}`);
  await new Promise(res => setTimeout(res, waitMs));
  await finish(0);
} catch (e) {
  console.error('\n\x1b[31mprobe failed\x1b[0m', e);
  await finish(1);
}
