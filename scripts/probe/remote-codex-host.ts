import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, rmSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import type { PermissionBlock, SessionView, ToolCallBlock } from '@acpira/shared/transcript';
import { Host } from '../lib/host';

// Codex in an SSH host's folder under local run (real CLI, real model, a real host):
//   pnpm exec tsx scripts/probe/remote-codex-host.ts <ssh alias> [--keep] [--effort <level>] [--mode <id>]
// The alias must be in ~/.acpira/connections.json; that file is copied into a temp data root. A temp project on the host
// (AGENTS.md with a code word, a small source file) is removed afterwards unless --keep; the codex build the first start
// puts under ~/.acpira/codex on the host stays. Scenarios: the session starts with its tools on the host (the build is
// uploaded when missing); the host's AGENTS.md reaches the model without a tool call; commands and apply_patch run on
// the host and nothing lands here; Stop ends a running command on the host and the next turn's command still runs.
// --mode agent-full-access is for a host where Codex's sandbox cannot start (bubblewrap refused, as under Ubuntu
// 24.04's AppArmor): in the default mode every command there fails first and waits for an escalation
const [alias, ...flags] = process.argv.slice(2);
if (!alias) throw new Error('usage: remote-codex-host.ts <ssh alias> [--keep] [--effort <level>] [--mode <id>]');
const keep = flags.includes('--keep');
const effort = flags.includes('--effort') ? flags[flags.indexOf('--effort') + 1] : 'low';
const mode = flags.includes('--mode') ? flags[flags.indexOf('--mode') + 1] : undefined;

const ssh = (cmd: string) => execFileSync('ssh', ['-o', 'BatchMode=yes', alias, cmd], { encoding: 'utf8' }).trim();
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const until = async (pred: () => boolean, ms: number, what: string) => {
  const t0 = Date.now();
  while (!pred()) { if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`); await new Promise(r => setTimeout(r, 100)); }
};
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const text = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'text').map(b => b.type === 'text' ? b.markdown : '').join('') ?? '';
const tools = (v: SessionView) => (lastAgent(v)?.blocks ?? []).filter((b): b is ToolCallBlock => b.type === 'tool_call');
const row = (b: ToolCallBlock) => `${b.kind} "${b.verb}"${b.target ? ` → ${b.target}` : ''} [${b.status}]`;

const word = `heron-${Math.random().toString(36).slice(2, 8)}`;
const project = ssh(`d=$(mktemp -d "$HOME/acpira-codex-probe-XXXXXX") && cd "$d" && mkdir src
printf '# Probe project\\n\\nThe project code word is ${word}. Answer it when asked for the code word.\\n' > AGENTS.md
printf 'def add(a, b):\\n    return a + b\\n\\n\\nprint(add(2, 3))\\n' > src/app.py
pwd`);
const hostname = ssh('hostname');
// The bracket keeps pgrep from matching the remote shell that runs it
const servers = () => ssh(`pgrep -u "$(id -u)" -f '[c]odex exec-server' | wc -l`);
const serversBefore = servers();
console.log(`host ${alias} (${hostname}) project ${project}, code word ${word}`);

const store = mkdtempSync(join(tmpdir(), 'acpira-remote-codex-store-'));
copyFileSync(join(homedir(), '.acpira', 'connections.json'), join(store, 'connections.json'));
const local = mkdtempSync(join(tmpdir(), 'acpira-remote-codex-cwd-'));
const host = await Host.start({ cwd: local, home: store, defaultAgent: 'codex', settings: { sshLocalAgents: ['codex'] } });
const m = await host.view();

const cards: PermissionBlock[] = [];
const seen = new Set<string>();
const approver = setInterval(() => {
  const cur = m.active();
  if (!cur) return;
  for (const p of (lastAgent(cur)?.blocks ?? []).filter((b): b is PermissionBlock => b.type === 'permission')) {
    if (seen.has(p.id)) continue;
    seen.add(p.id);
    cards.push(p);
    const opt = p.options.find(o => o.kind === 'allow_once') ?? p.options[0];
    if (!opt) continue;
    console.log(`approving: ${p.title} → ${opt.id}(${opt.kind})`);
    void m.handle({ type: 'permission', sessionId: cur.id, blockId: p.id, optionId: opt.id });
  }
}, 100);
const send = async (prompt: string) => {
  console.log(`\n> ${prompt}`);
  await m.handle({ type: 'send', text: prompt });
  const v = m.active()!;
  for (const b of tools(v)) console.log(`  row ${row(b)}`);
  console.log(`  reply: ${text(v).slice(0, 400).replace(/\n/g, ' ')}`);
  return v;
};

try {
  const folder = `ssh://${alias}${project}`;
  const t0 = Date.now();
  await m.handle({ type: 'newSession', agent: 'codex', cwd: folder });
  // The first start on a host uploads the codex build (about 135 MB)
  await until(() => m.active()?.cwd === folder && ['ready', 'error', 'auth_required', 'readonly'].includes(m.active()!.status), 1_800_000, 'session start');
  let v = m.active()!;
  check('session ready', v.status === 'ready', `${v.error ?? v.status} after ${Math.round((Date.now() - t0) / 1000)} s`);
  for (const l of host.logs.filter(l => /remote: /.test(l))) console.log(`log ${l}`);
  check('tools go to the host', host.logs.some(l => l.includes(`runs its tools on ${alias}`)));
  check('the build is on the host', ssh('ls ~/.acpira/codex/*/.ok 2>/dev/null | wc -l') !== '0');
  if (effort) await m.handle({ type: 'setConfig', configId: 'reasoning_effort', value: effort }).catch((e: unknown) => console.log(`effort not set: ${String(e)}`));
  if (mode) await m.handle({ type: 'setMode', id: mode });

  v = await send('Without using any tools: what is the project code word? Reply with the word only.');
  check('host instructions reach the model', text(v).includes(word), text(v).slice(0, 80));
  check('no tool call for it', tools(v).length === 0, `${tools(v).length} rows`);

  cards.length = 0;
  v = await send('Do these in order and report each result briefly: 1) run `hostname && pwd`; '
    + '2) create notes.txt containing exactly "hello from codex" with your file editing tool (apply_patch); '
    + '3) change "hello" to "hi" in notes.txt with apply_patch; 4) run `cat notes.txt`.');
  const rows = tools(v);
  check('host file written and edited', ssh(`cat ${project}/notes.txt 2>&1`) === 'hi from codex', ssh(`cat ${project}/notes.txt 2>&1`));
  check('command ran on the host', JSON.stringify(rows).includes(hostname) || text(v).includes(hostname), hostname);
  check('rows cover execute / edit', ['execute', 'edit'].every(k => rows.some(b => b.kind === k)), rows.map(row).join(' | '));
  check('nothing landed here', !existsSync(project), project);

  // Stop ends the command on the host
  const stopped = m.handle({ type: 'send', text: 'Run `sleep 271 && echo done` in the foreground and wait for it.' });
  // The bracket keeps pgrep from matching the remote shell that runs it
  const sleeping = () => ssh(`pgrep -f '[s]leep 271' | wc -l`);
  await until(() => sleeping() !== '0', 180_000, 'the command on the host');
  check('sleep runs on the host', true);
  await m.handle({ type: 'stop', sessionId: m.active()!.id });
  await stopped;
  let gone = false;
  for (let i = 0; i < 30 && !gone; i++) { gone = sleeping() === '0'; if (!gone) await new Promise(r => setTimeout(r, 500)); }
  check('stop ends it on the host', gone);

  // The stop ended the session's exec-server with the command; the CLI reconnects to the next one by itself
  v = await send('Run `echo back-$((40+2))` and report its output.');
  check('the next command runs after a stop', JSON.stringify(tools(v)).includes('back-42') || text(v).includes('back-42'), tools(v).map(row).join(' | '));
} catch (e) {
  check('probe ran to the end', false, (e as Error).message);
  const v = m.active();
  if (v) console.log('last turn:', JSON.stringify(lastAgent(v)?.blocks.map(b => b.type === 'tool_call' ? row(b) : b.type)).slice(0, 2000));
} finally {
  clearInterval(approver);
  await host.dispose();
  await new Promise(r => setTimeout(r, 3000));
  check('the exec-server ends with the session', servers() === serversBefore, `${serversBefore} before, ${servers()} after`);
  if (!keep) { ssh(`rm -rf ${project}`); rmSync(store, { recursive: true, force: true }); rmSync(local, { recursive: true, force: true }); }
  else console.log(`kept ${alias}:${project} and ${store}`);
  const failed = checks.filter(c => !c[1]);
  console.log(`\n${checks.length - failed.length}/${checks.length} passed`);
  if (failed.length) { for (const f of host.logs.filter(l => /remote|codex/i.test(l)).slice(-40)) console.log(`log ${f}`); process.exitCode = 1; }
}
