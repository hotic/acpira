import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { mkdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { dataHome, readCredentials } from '../src/host/accounts/devin';

// Raw-wire subagent probe. Speaks JSON-RPC over stdio itself (no SDK), so nothing an agent sends is validated away:
// the released SDK's session/update parser is a closed union and drops unknown `sessionUpdate` kinds.
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-subagents.ts <agent> [--cmd "<command line>"] [--no-caps] [--air] [--devin-meta] [--wait MS] [--out FILE] [prompt]
//
// <agent>: a registry id (devin / grok / kimi / opencode / …) or a label for --cmd
// --cmd: run this command line instead of the registry's (e.g. "npx -y @agentclientprotocol/claude-agent-acp@0.78.0")
// --no-caps: do not advertise clientCapabilities.subagents (RFD #1992) — the control run
// --air: also advertise _meta.jetbrains.air = { version: 1, capabilities: ['nativeSubagentSessions'] } — claude-agent-acp's bridge for
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

let command: string, args: string[];
if (cmdOpt?.value) {
  [command = '', ...args] = cmdOpt.value.split(/\s+/).filter(Boolean);
} else {
  const registry = new AgentRegistry();
  const def = registry.get(agentId);
  const bin = await registry.resolveBinary(agentId);
  if (!bin) { console.error(`command not found: ${def.command}`); process.exit(1); }
  command = bin; args = def.args;
}
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
const record = (dir: '→' | '←', msg: unknown) => { log.push(JSON.stringify({ t: Date.now(), dir, msg: redact(msg) })); };

console.error(`→ ${command} ${args.join(' ')}`);
const child = spawn(command, args, { cwd: process.cwd(), stdio: ['pipe', 'pipe', 'pipe'], env: process.env });
createInterface({ input: child.stderr }).on('line', l => { console.error(`\x1b[33mstderr\x1b[0m ${l}`); log.push(JSON.stringify({ t: Date.now(), dir: 'stderr', line: l.slice(0, 400) })); });

let seq = 0;
const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
function send(msg: Record<string, unknown>) { record('→', msg); child.stdin.write(JSON.stringify(msg) + '\n'); }
function request(method: string, params: unknown): Promise<unknown> {
  const id = ++seq;
  send({ jsonrpc: '2.0', id, method, params });
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
}
function respond(id: unknown, result: unknown) { send({ jsonrpc: '2.0', id, result }); }
function respondError(id: unknown, code: number, message: string) { send({ jsonrpc: '2.0', id, error: { code, message } }); }

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

createInterface({ input: child.stdout }).on('line', line => {
  if (!line.trim()) return;
  let msg: Record<string, unknown>;
  try { msg = JSON.parse(line); } catch { console.error(`\x1b[31mnon-json stdout\x1b[0m ${line.slice(0, 200)}`); log.push(JSON.stringify({ t: Date.now(), dir: 'stdout-raw', line: line.slice(0, 400) })); return; }
  record('←', msg);
  if ('method' in msg) {
    const method = msg.method as string;
    const params = (msg.params ?? {}) as Record<string, unknown>;
    collectMeta(params, method);
    if ('id' in msg) {
      requestMethods.set(method, (requestMethods.get(method) ?? 0) + 1);
      console.log(`\x1b[36m[request ${method}]\x1b[0m session ${short(params.sessionId)}${params.sessionId && params.sessionId !== rootSessionId ? ' \x1b[35m(child?)\x1b[0m' : ''} ${JSON.stringify(redact(params)).slice(0, 300)}`);
      if (method === 'session/request_permission') {
        const options = (params.options ?? []) as { optionId: string; kind: string }[];
        const allow = options.find(o => o.kind === 'allow_once') ?? options[0];
        if (allow) respond(msg.id, { outcome: { outcome: 'selected', optionId: allow.optionId } });
        else respond(msg.id, { outcome: { outcome: 'cancelled' } });
      } else if (method === 'elicitation/create') {
        respond(msg.id, { action: 'cancel' });
      } else {
        respondError(msg.id, -32601, `Method not found: ${method}`);
      }
      return;
    }
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
    return;
  }
  if ('id' in msg) {
    const p = pending.get(msg.id as number);
    pending.delete(msg.id as number);
    if (!p) return;
    if ('error' in msg && msg.error) p.reject(msg.error); else p.resolve(msg.result);
  }
});

const exited = new Promise<void>(resolve => child.once('exit', (code, signal) => { console.error(`exit code=${code} signal=${signal}`); resolve(); }));

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
  if (child.exitCode === null) { child.kill(); setTimeout(() => child.kill('SIGKILL'), 5000).unref(); }
  await Promise.race([exited, new Promise(r => setTimeout(r, 6000))]);
  process.exit(exitCode);
}

// A hung upstream (a gateway retrying 503s) still leaves its evidence: write what was captured before dying
for (const sig of ['SIGINT', 'SIGTERM'] as const) process.once(sig, () => { console.error(`\n${sig}: writing partial log`); void finish(130); });

try {
  const meta: Record<string, unknown> = {};
  if (flag('--air')) meta.jetbrains = { air: { version: 1, capabilities: ['nativeSubagentSessions'] } };
  if (flag('--devin-meta')) meta['cognition.ai/subagentSupport'] = true;
  const clientCapabilities: Record<string, unknown> = {
    fs: { readTextFile: false, writeTextFile: false },
    terminal: false,
    ...(flag('--no-caps') ? {} : { subagents: {} }),
    ...(Object.keys(meta).length ? { _meta: meta } : {}),
  };
  const init = await request('initialize', { protocolVersion: 1, clientInfo: { name: 'acpira-probe', version: '0' }, clientCapabilities }) as Record<string, unknown>;
  collectMeta(init, 'initialize.result');
  console.log('initialize →', JSON.stringify(redact(init), null, 1).slice(0, 1500));
  if (flag('--import-local')) {
    const cred = await readCredentials(join(dataHome(), 'devin', 'credentials.toml'));
    if (!cred) throw new Error('no local devin login (credentials.toml)');
    const methods = (init.authMethods ?? []) as { id: string }[];
    const authMeta: Record<string, string> = { api_key: cred.secret };
    if (cred.meta?.api_server_url) authMeta.api_server_url = cred.meta.api_server_url;
    // Logged through `send`; the redactor cannot know this key is secret, so the request goes out unrecorded
    const id = ++seq;
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method: 'authenticate', params: { methodId: methods[0]?.id ?? 'devin-browser', _meta: authMeta } }) + '\n');
    log.push(JSON.stringify({ t: Date.now(), dir: '→', msg: { id, method: 'authenticate', params: '[redacted: api_key]' } }));
    await new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
    console.log('authenticate ok (local login)');
  }
  const s = await request('session/new', { cwd: process.cwd(), mcpServers: [] }) as { sessionId: string; [k: string]: unknown };
  rootSessionId = s.sessionId;
  console.log(`session/new → ${s.sessionId}`);
  console.log(`\nprompt → ${promptText}\n`);
  const started = Date.now();
  const r = await request('session/prompt', { sessionId: s.sessionId, prompt: [{ type: 'text', text: promptText }] }) as Record<string, unknown>;
  collectMeta(r, 'prompt.result');
  console.log(`\n\nprompt done in ${((Date.now() - started) / 1000).toFixed(1)}s → ${JSON.stringify(redact(r)).slice(0, 400)}`);
  await new Promise(res => setTimeout(res, waitMs));
  await finish(0);
} catch (e) {
  console.error('\n\x1b[31mprobe failed\x1b[0m', e);
  await finish(1);
}
