import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { PermissionBlock, SessionView, ToolCallBlock } from '@shared/transcript';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { SessionManager } from '../src/host/SessionManager';
import { TranscriptStore } from '../src/host/store/TranscriptStore';

// Generic host-path smoke for any registered agent (real CLI, real model):
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-agent-host.ts <agent> [--attach] [--shell]
// new session → controls / commands / no ghost turn → "pong" prompt → optionally a dropped text attachment (--attach: goes as an
// embedded resource or a marked-up text block depending on promptCapabilities.embeddedContext) → optionally a shell command
// (--shell: the tool row must carry the command and its output, permission cards are answered like a click).
// Point the CLI's own store elsewhere first when its sessions must not pile up (DSH_HOME=…, PI_CODING_AGENT_DIR=…).
const [agentId = 'opencode', ...flags] = process.argv.slice(2);
const attach = flags.includes('--attach');
const shell = flags.includes('--shell');
const project = mkdtempSync(join(tmpdir(), `acpira-${agentId}-project-`));
const store = mkdtempSync(join(tmpdir(), `acpira-${agentId}-store-`));
const logs: string[] = [];
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const until = async (pred: () => boolean, ms: number, what: string) => {
  const t0 = Date.now();
  while (!pred()) { if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`); await new Promise(r => setTimeout(r, 50)); }
};
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const text = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'text').map(b => b.type === 'text' ? b.markdown : '').join('') ?? '';

const m = new SessionManager({
  registry: new AgentRegistry(), store: new TranscriptStore(store),
  log: l => logs.push(l), cwd: () => project, defaultAgent: () => agentId,
  runInTerminal: () => {}, toast: (l, t) => console.log(`toast ${l}: ${t}`),
});
const approver = setInterval(() => {
  const cur = m.active();
  const turn = cur ? lastAgent(cur) : undefined;
  for (const b of turn?.blocks ?? []) {
    if (b.type !== 'permission') continue;
    const p = b as PermissionBlock;
    const opt = p.options.find(o => o.kind === 'allow_once') ?? p.options[0];
    if (!opt) continue;
    console.log(`approving: ${p.title} → ${opt.id}(${opt.kind}) · ${p.options.map(o => `${o.id}(${o.kind})`).join(' ')}`);
    void m.handle({ type: 'permission', sessionId: cur!.id, blockId: b.id, optionId: opt.id });
  }
}, 100);

try {
  await m.newSession(agentId);
  await until(() => ['ready', 'error', 'auth_required', 'readonly'].includes(m.active()?.status ?? ''), 90_000, 'session start');
  let v = m.active()!;
  console.log('status:', v.status, v.error ?? '');
  console.log('controls:', JSON.stringify({ modes: v.controls.modes.map(x => x.id), modeId: v.controls.modeId, options: v.controls.options.map(o => `${o.id}[${o.category}]=${JSON.stringify(o.value)}(${o.options.length})`) }));
  check('session ready', v.status === 'ready', v.error);
  check('no ghost turn after start', v.turns.length === 0, `${v.turns.length} turns`);
  if (v.status !== 'ready') throw new Error('not ready');

  await m.handle({ type: 'send', text: 'Reply with exactly the word pong and nothing else.' });
  v = m.active()!;
  check('pong reply', /pong/i.test(text(v)), text(v).slice(0, 80));
  check('turn ended end_turn', lastAgent(v)?.stop === 'end_turn', String(lastAgent(v)?.stop));
  console.log('commands:', v.commands.length, v.commands.slice(0, 8).map(c => `/${c.name}`).join(' '));
  console.log('usage:', JSON.stringify(v.usage ?? null), 'turn usage:', JSON.stringify(lastAgent(v)?.usage ?? null));

  if (attach) {
    await m.handle({ type: 'send', text: 'The attached note contains a secret word. Reply with only that word.', attachments: [{ kind: 'text', name: 'note.txt', text: 'The secret word is marmalade.' }] });
    v = m.active()!;
    check('text attachment understood', /marmalade/i.test(text(v)), text(v).slice(0, 80));
    const user = v.turns[v.turns.length - 2];
    check('attachment kept on the user turn', user?.role === 'user' && user.attachments?.[0]?.kind === 'text');
  }

  if (shell) {
    await m.handle({ type: 'send', text: 'Run the shell command `echo acpira-shell-ok` and reply with its output only.' });
    v = m.active()!;
    const tools = (lastAgent(v)?.blocks ?? []).filter((b): b is ToolCallBlock => b.type === 'tool_call');
    console.log('tools:', JSON.stringify(tools.map(t => ({ kind: t.kind, verb: t.verb, target: t.target, status: t.status, content: t.content?.type, text: t.content?.type === 'text' ? t.content.text.slice(0, 80) : undefined }))));
    const exec = tools.find(t => t.kind === 'execute');
    check('shell tool row with command', !!exec && /echo/.test(exec.target ?? ''), exec?.target);
    check('shell output captured on the row', exec?.content?.type === 'text' && /acpira-shell-ok/.test(exec.content.text), exec?.content?.type === 'text' ? exec.content.text.slice(0, 60) : String(exec?.content));
    check('reply relays the output', /acpira-shell-ok/.test(text(v)), text(v).slice(0, 80));
  }
} catch (e) {
  console.log('probe aborted:', e instanceof Error ? e.message : e);
} finally {
  clearInterval(approver);
  await m.dispose();
}
const failed = checks.filter(c => !c[1]);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
console.log('log tail:\n' + logs.filter(l => !/^stderr: /.test(l) || /error|warn|fail/i.test(l)).slice(-15).join('\n'));
rmSync(store, { recursive: true, force: true });
rmSync(project, { recursive: true, force: true });
process.exit(failed.length ? 1 : 0);
