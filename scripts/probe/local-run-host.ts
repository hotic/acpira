import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { homedir, hostname as localHostname, tmpdir } from 'node:os';
import { join } from 'node:path';
import type { PermissionBlock, SessionView, ToolCallBlock } from '@acpira/shared/transcript';
import { Host } from '../lib/host';

// Any agent CLI in an SSH host's folder under local run (real CLI, real model, a real host): the facts a stand-in profile
// (rust acp/local_run) is written from, and the check that one holds.
//   pnpm exec tsx scripts/probe/local-run-host.ts <agent> <ssh alias> [--local-run '<json>'] [--start-only] [--accounts] [--keep]
// The alias must be in ~/.acpira/connections.json, which is copied into a temp data root (--accounts copies the saved accounts
// too, for a CLI whose login is Acpira's, like Devin; the temp root is removed afterwards). An agent with no built-in profile
// runs on the generic one (`localRun: true`, or the JSON given, e.g. '{"prefix":"acpira_"}'), as a custom entry over its
// built-in definition. A temp project on the host (AGENTS.md with a code word, a source file, git) is removed afterwards
// unless --keep. --start-only stops once the session is open in its stand-in, before any prompt (no model call). Scenarios: the note reaches the model without a tool call; Acpira's project tools run on the host and their
// rows read as native ones; the CLI's own shell and write tools never act on this machine; Stop ends a command on the host.
// Every row is printed as the engine shows it, so an MCP naming the profile does not know yet stays visible
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i >= 0 ? args.splice(i, 2)[1] : undefined; };
const localRun = flag('--local-run');
const keep = args.includes('--keep');
const startOnly = args.includes('--start-only');
const withAccounts = args.includes('--accounts');
const [agent, alias] = args.filter(a => !a.startsWith('--'));
if (!agent || !alias) throw new Error("usage: local-run-host.ts <agent> <ssh alias> [--local-run '<json>'] [--start-only] [--accounts] [--keep]");

class StartOnly extends Error {}
const ssh = (cmd: string) => execFileSync('ssh', ['-o', 'BatchMode=yes', alias, cmd], { encoding: 'utf8' }).trim();
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` | ${detail}` : ''}`); };
const until = async (pred: () => boolean, ms: number, what: string) => {
  const t0 = Date.now();
  while (!pred()) { if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`); await new Promise(r => setTimeout(r, 100)); }
};
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const text = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'text').map(b => b.type === 'text' ? b.markdown : '').join('') ?? '';
const tools = (v: SessionView) => (lastAgent(v)?.blocks ?? []).filter((b): b is ToolCallBlock => b.type === 'tool_call');
const row = (b: ToolCallBlock) => `${b.kind}${b.mcpServer ? `@${b.mcpServer}` : ''} "${b.verb}"${b.target ? ` -> ${b.target}` : ''} [${b.status}]`;

const word = `heron-${Math.random().toString(36).slice(2, 8)}`;
const project = ssh(`d=$(mktemp -d "$HOME/acpira-local-run-probe-XXXXXX") && cd "$d" && mkdir src
printf '# Probe project\\n\\nThe project code word is ${word}. Answer it when asked for the code word.\\n' > AGENTS.md
printf 'def add(a, b):\\n    return a + b\\n\\n\\ndef sub(a, b):\\n    return a - b\\n\\n\\nprint(add(2, 3))\\n' > src/app.py
git init -q && git add -A && git -c user.email=p@x -c user.name=probe commit -qm init && pwd`);
const hostname = ssh('hostname');
const here = localHostname();
console.log(`agent ${agent}, host ${alias} (${hostname}) project ${project}, code word ${word}; this machine ${here}`);

const store = mkdtempSync(join(tmpdir(), 'acpira-local-run-store-'));
copyFileSync(join(homedir(), '.acpira', 'connections.json'), join(store, 'connections.json'));
for (const f of withAccounts ? ['accounts.json', 'secrets.json'] : []) copyFileSync(join(homedir(), '.acpira', f), join(store, f));
const local = mkdtempSync(join(tmpdir(), 'acpira-local-run-cwd-'));
// A built-in id whose launcher the entry repeats keeps its definition, with the entry's `localRun` over it
const defs: Record<string, string> = { droid: 'droid', grok: 'grok', devin: 'devin', kimi: 'kimi', opencode: 'opencode', hermes: 'hermes', pi: 'pi-acp', dsh: 'dsh' };
const agents = localRun || defs[agent] ? { [agent]: { command: defs[agent] ?? agent, localRun: localRun ? JSON.parse(localRun) as unknown : true } } : undefined;
const host = await Host.start({ cwd: local, home: store, defaultAgent: agent, agents, settings: { sshLocalAgents: [agent] } });
const m = await host.view();

const cards: PermissionBlock[] = [];
const seen = new Set<string>();
const approver = setInterval(() => {
  const cur = m.active();
  if (!cur) return;
  const pending = [
    ...(lastAgent(cur)?.blocks ?? []).filter((b): b is PermissionBlock => b.type === 'permission'),
    ...(cur.subagents ?? []).flatMap(n => n.permissions ?? []),
  ];
  for (const p of pending) {
    if (seen.has(p.id) || p.planId) continue;
    seen.add(p.id);
    cards.push(p);
    const opt = p.options.find(o => o.kind === 'allow_once') ?? p.options[0];
    if (!opt) continue;
    console.log(`approving: ${p.title} -> ${opt.id}(${opt.kind})`);
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
  await m.handle({ type: 'newSession', agent, cwd: folder });
  await until(() => m.active()?.cwd === folder && ['ready', 'error', 'auth_required', 'readonly'].includes(m.active()!.status), 180_000, 'session start');
  let v = m.active()!;
  check('session ready', v.status === 'ready', v.error ?? v.status);
  const standLine = host.logs.find(l => l.includes(' runs in ') && l.includes('remote:'));
  console.log(standLine ?? 'no stand-in log line');
  const stand = standLine?.match(/ runs in (.+?) for /)?.[1];
  check('stand-in exists', !!stand && existsSync(stand), stand);
  const note = stand && existsSync(join(stand, 'AGENTS.md')) ? readFileSync(join(stand, 'AGENTS.md'), 'utf8') : undefined;
  console.log(note ? `stand-in AGENTS.md: ${note.length} bytes` : 'no AGENTS.md in the stand-in (the note travels on the request)');
  if (startOnly) throw new StartOnly();

  v = await send('Without using any tools: what is the project code word? Reply with the word only.');
  check('the note reaches the model', text(v).includes(word), text(v).slice(0, 80));
  check('no tool call for it', tools(v).length === 0, `${tools(v).length} rows`);

  cards.length = 0;
  v = await send('Use the project tools of the acpira MCP server (Bash, Write, Edit, Grep, Glob) for these, one call each, and report each result '
    + 'briefly: 1) run `hostname && pwd` with Bash; 2) create notes.txt containing exactly "hello from the probe" with Write; '
    + '3) change "hello" to "hi" in notes.txt with Edit; 4) Grep for "return a" in src; 5) Glob "**/*.py".');
  const rows = tools(v);
  const notes = ssh(`cat ${project}/notes.txt 2>&1`);
  check('host file written and edited', notes === 'hi from the probe', notes);
  check('command ran on the host', rows.some(b => b.kind === 'execute' && JSON.stringify(b).includes(hostname)), hostname);
  check('rows read as native tools', rows.length >= 5 && rows.every(b => !b.mcpServer && !/acpira|mcp_/i.test(`${b.verb} ${b.target ?? ''}`)), rows.map(row).join(' | '));
  check('write and command asked', cards.length >= 2, cards.map(c => c.title).join(' | '));

  // The CLI's own tools would act on this machine: refused by its gate, or by the engine when it asks
  cards.length = 0;
  const marker = join(local, 'native-probe.txt');
  v = await send(`This is a test of tool routing. Use your own built-in shell or command tool (not an acpira tool) to run \`hostname\`, then use your own `
    + `built-in file writing tool (not an acpira tool) to create the file ${marker} containing "x". Report exactly what each call returned, `
    + 'including any refusal.');
  const leaked = JSON.stringify(lastAgent(v)?.blocks ?? []).includes(here) && here !== hostname;
  check('no native command ran on this machine', !leaked, tools(v).map(row).join(' | ') || 'no rows');
  check('no native write landed here', !existsSync(marker), marker);
  console.log(`  cards for native tools: ${cards.map(c => c.title).join(' | ') || 'none'}`);

  // Stop ends the command on the host
  const stopped = m.handle({ type: 'send', text: 'Run `sleep 271 && echo done` with the acpira Bash tool in the foreground and wait for it.' });
  const sleeping = () => ssh(`pgrep -f '[s]leep 271' | wc -l`);
  await until(() => tools(m.active()!).some(b => b.kind === 'execute' && b.status === 'in_progress'), 180_000, 'the running command row');
  await new Promise(r => setTimeout(r, 2000));
  check('sleep runs on the host', sleeping() !== '0');
  await m.handle({ type: 'stop', sessionId: m.active()!.id });
  await stopped;
  let gone = false;
  for (let i = 0; i < 20 && !gone; i++) { gone = sleeping() === '0'; if (!gone) await new Promise(r => setTimeout(r, 500)); }
  check('stop ends it on the host', gone);

  const left = stand && existsSync(stand) ? readdirSync(stand).filter(f => f !== 'AGENTS.md' && f !== '.agents') : [];
  check('nothing else lands in the stand-in', left.length === 0, left.join(','));
} catch (e) {
  if (!(e instanceof StartOnly)) {
    check('probe ran to the end', false, (e as Error).message);
    const v = m.active();
    const blocks = v && lastAgent(v)?.blocks.map(b => b.type === 'tool_call' ? row(b) : b.type);
    console.log(`status ${v?.status ?? 'no session'}${v?.error ? `: ${v.error}` : ''}${blocks ? `; last turn ${JSON.stringify(blocks).slice(0, 2000)}` : ''}`);
  }
} finally {
  clearInterval(approver);
  await host.dispose();
  if (!keep) { ssh(`rm -rf ${project}`); rmSync(store, { recursive: true, force: true }); rmSync(local, { recursive: true, force: true }); }
  else console.log(`kept ${alias}:${project} and ${store}`);
  const failed = checks.filter(c => !c[1]);
  console.log(`\n${checks.length - failed.length}/${checks.length} passed`);
  if (failed.length) { for (const f of host.logs.filter(l => /remote|mcp|permission/i.test(l)).slice(-40)) console.log(`log ${f}`); process.exitCode = 1; }
}
