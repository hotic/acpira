import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, readdirSync, rmSync, statSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import type { PermissionBlock, PlanDocumentBlock, SessionView, ToolCallBlock } from '@acpira/shared/transcript';
import { Host } from '../lib/host';

// Claude Code in an SSH host's folder under local run (real CLI, real model, a real host):
//   pnpm exec tsx scripts/probe/remote-claude-host.ts <ssh alias> [--keep]
// The alias must be in ~/.acpira/connections.json with mode "local"; that file is copied into a temp data root. A temp project
// on the host (CLAUDE.md with a code word, a small source file, git) is removed afterwards unless --keep. Scenarios: the host's
// instructions reach the model without a tool call; Bash / Write / Edit / Grep / Glob run on the host and their rows read as
// Claude's own; a Task subagent's tools reach the host too; plan mode keeps the plan file on this machine without a card and a
// rejected plan writes nothing; Stop ends a running command on the host; the local stand-in folder stays empty
const [alias, ...flags] = process.argv.slice(2);
if (!alias) throw new Error('usage: remote-claude-host.ts <ssh alias> [--keep]');
const keep = flags.includes('--keep');

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
const row = (b: ToolCallBlock) => `${b.kind}${b.mcpServer ? `@${b.mcpServer}` : ''} "${b.verb}"${b.target ? ` → ${b.target}` : ''} [${b.status}]`;

const word = `heron-${Math.random().toString(36).slice(2, 8)}`;
const project = ssh(`d=$(mktemp -d "$HOME/acpira-claude-probe-XXXXXX") && cd "$d" && mkdir src
printf '# Probe project\\n\\nThe project code word is ${word}. Answer it when asked for the code word.\\n' > CLAUDE.md
printf 'def add(a, b):\\n    return a + b\\n\\n\\ndef sub(a, b):\\n    return a - b\\n\\n\\nprint(add(2, 3))\\n' > src/app.py
git init -q && git add -A && git -c user.email=p@x -c user.name=probe commit -qm init && pwd`);
const hostname = ssh('hostname');
console.log(`host ${alias} (${hostname}) project ${project}, code word ${word}`);

const store = mkdtempSync(join(tmpdir(), 'acpira-remote-claude-store-'));
copyFileSync(join(homedir(), '.acpira', 'connections.json'), join(store, 'connections.json'));
const local = mkdtempSync(join(tmpdir(), 'acpira-remote-claude-cwd-'));
const host = await Host.start({ cwd: local, home: store, defaultAgent: 'claude' });
const m = await host.view();

// Click-through for every card but plan approvals, which the plan scenario answers itself
const cards: PermissionBlock[] = [];
const seen = new Set<string>();
const approver = setInterval(() => {
  const cur = m.active();
  if (!cur) return;
  // A subagent's cards sit in its node's summary; both answer through the same message
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
  // The view starts on an empty local Claude session (the default agent), so wait for the host folder's own session
  const folder = `ssh://${alias}${project}`;
  await m.handle({ type: 'newSession', agent: 'claude', cwd: folder });
  await until(() => m.active()?.cwd === folder && ['ready', 'error', 'auth_required', 'readonly'].includes(m.active()!.status), 180_000, 'session start');
  let v = m.active()!;
  check('session ready', v.status === 'ready', v.error ?? v.status);
  const standLine = host.logs.find(l => l.includes('Claude Code runs in'));
  console.log(standLine ?? 'no stand-in log line');
  const stand = join(store, 'remote', 'hosts', alias, project.replace(/^\//, ''));
  check('stand-in exists', existsSync(stand), stand);

  v = await send('Without using any tools: what is the project code word? Reply with the word only.');
  check('host instructions reach the model', text(v).includes(word), text(v).slice(0, 80));
  check('no tool call for it', tools(v).length === 0, `${tools(v).length} rows`);

  cards.length = 0;
  v = await send('Do these in order with your tools, one call each, and report each result briefly: '
    + '1) run `hostname && pwd` with Bash; 2) create notes.txt containing exactly "hello from claude" with Write; '
    + '3) change "hello" to "hi" in notes.txt with Edit; 4) Grep for "return a" in src; 5) Glob "**/*.py".');
  const rows = tools(v);
  check('host file written and edited', ssh(`cat ${project}/notes.txt 2>&1`) === 'hi from claude', ssh(`cat ${project}/notes.txt 2>&1`));
  check('command ran on the host', rows.some(b => b.kind === 'execute' && JSON.stringify(b).includes(hostname)), hostname);
  check('rows read as native tools', rows.length >= 5 && rows.every(b => !b.mcpServer && !/acpira|mcp__/i.test(`${b.verb} ${b.target ?? ''}`)), rows.map(row).join(' | '));
  check('rows cover execute / edit / search', ['execute', 'edit', 'search'].every(k => rows.some(b => b.kind === k)), [...new Set(rows.map(b => b.kind))].join(','));
  check('write and command asked', cards.length >= 2, cards.map(c => c.title).join(' | '));
  check('cards carry no MCP naming', cards.every(c => !/mcp__|acpira/i.test(c.title)), cards.map(c => c.title).join(' | '));

  cards.length = 0;
  v = await send('Use the Task tool with a general-purpose subagent to count the lines of src/app.py with `wc -l` and report its number. Reply with the number only.');
  const lines = ssh(`wc -l < ${project}/src/app.py`);
  check('a subagent reaches the host', text(v).includes(lines), `${lines} vs ${text(v).slice(0, 60)}`);
  check('subagent cards read as native', cards.every(c => !/mcp__/i.test(c.title)), cards.map(c => c.title).join(' | ') || 'no card');

  // Plan mode: the plan file is Claude's own and stays here without a card; rejecting the plan writes nothing
  const plans = join(homedir(), '.claude', 'plans');
  const t0 = Date.now();
  await m.handle({ type: 'setMode', id: 'plan' });
  await until(() => m.active()!.controls.modeId === 'plan', 15_000, 'mode → plan');
  cards.length = 0;
  const planned = m.handle({ type: 'send', text: 'Plan (do not implement) adding a file plan.txt that says x, then request approval to implement.' });
  await until(() => { const a = lastAgent(m.active()!); return !!a?.blocks.some(b => b.type === 'permission' && (b as PermissionBlock).planId); }, 300_000, 'the plan card');
  const a = lastAgent(m.active()!)!;
  const perm = a.blocks.find(b => b.type === 'permission' && (b as PermissionBlock).planId) as PermissionBlock;
  const doc = a.blocks.find(b => b.type === 'plan_document') as PlanDocumentBlock | undefined;
  check('plan document linked', !!doc && perm.planId === doc.id);
  check('no card for the plan file', cards.length === 0, cards.map(c => c.title).join(' | '));
  const fresh = existsSync(plans) ? readdirSync(plans).filter(f => statSync(join(plans, f)).mtimeMs >= t0) : [];
  check('plan file written here', fresh.length > 0, fresh.join(','));
  check('no plan file on the host', ssh(`find ~/.claude/plans -newermt '-10 minutes' 2>/dev/null | wc -l`) === '0');
  const reject = perm.options.find(o => o.kind === 'reject_once') ?? perm.options.find(o => o.kind.startsWith('reject'))!;
  void m.handle({ type: 'permission', sessionId: m.active()!.id, blockId: perm.id, optionId: reject.id });
  await planned;
  check('rejected plan wrote nothing', ssh(`test -e ${project}/plan.txt && echo yes || echo no`) === 'no');
  await m.handle({ type: 'setMode', id: 'default' });
  await until(() => m.active()!.controls.modeId === 'default', 15_000, 'mode → default');

  // Stop ends the command on the host
  const stopped = m.handle({ type: 'send', text: 'Run `sleep 271 && echo done` with Bash in the foreground and wait for it.' });
  // The bracket keeps pgrep from matching the remote shell that runs it
  const sleeping = () => ssh(`pgrep -f '[s]leep 271' | wc -l`);
  await until(() => tools(m.active()!).some(b => b.kind === 'execute' && b.status === 'in_progress'), 180_000, 'the running command row');
  await new Promise(r => setTimeout(r, 2000));
  check('sleep runs on the host', sleeping() !== '0');
  await m.handle({ type: 'stop', sessionId: m.active()!.id });
  await stopped;
  let gone = false;
  for (let i = 0; i < 20 && !gone; i++) { gone = sleeping() === '0'; if (!gone) await new Promise(r => setTimeout(r, 500)); }
  check('stop ends it on the host', gone);
  const after = tools(m.active()!).find(b => b.kind === 'execute');
  check('row settles', !!after && after.status !== 'in_progress', after?.status);

  const left = existsSync(stand) ? readdirSync(stand) : [];
  check('stand-in stays empty', left.length === 0, left.join(','));
} catch (e) {
  check('probe ran to the end', false, (e as Error).message);
  const v = m.active();
  if (v) console.log('last turn:', JSON.stringify(lastAgent(v)?.blocks.map(b => b.type === 'tool_call' ? row(b) : b.type)).slice(0, 2000));
} finally {
  clearInterval(approver);
  await host.dispose();
  if (!keep) { ssh(`rm -rf ${project}`); rmSync(store, { recursive: true, force: true }); rmSync(local, { recursive: true, force: true }); }
  else console.log(`kept ${alias}:${project} and ${store}`);
  const failed = checks.filter(c => !c[1]);
  console.log(`\n${checks.length - failed.length}/${checks.length} passed`);
  if (failed.length) { for (const f of host.logs.filter(l => /remote|claude|mcp/i.test(l)).slice(-40)) console.log(`log ${f}`); process.exitCode = 1; }
}
