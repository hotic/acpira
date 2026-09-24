import { execSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import type { PermissionBlock, PlanDocumentBlock, SessionView, ToolCallBlock } from '@shared/transcript';
import { Host } from './lib/host';
import { builtinAgent } from './lib/sidecarBin';

// Generic host-path smoke for any registered agent (real CLI, real model):
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-agent-host.ts <agent> [--attach] [--shell] [--write] [--plan] [--image] [--background] [--failure] [--env KEY=VALUE]... [--raw]
// new session → controls / commands / no ghost turn → "pong" prompt → optionally a dropped text attachment (--attach: goes as an
// embedded resource or a marked-up text block depending on promptCapabilities.embeddedContext) → optionally a shell command
// (--shell: the tool row must carry the command and its output, permission cards are answered like a click).
//   --write      mode that asks before edits → "create note.txt" → a _meta.permission card is answered through its quick allow
//   --plan       plan mode → plan_document linked to a pending permission → reject resolves it without writing hello.txt
//   --image      drops a red PNG → "view it" → an agent/tool image lands in the session blob store on disk
//   --background asks for a backgrounded `sleep` → the tool row keeps running past end_turn, settles completed; a second
//                long task is stopped through _session/async_task/stop and the sleep process is gone afterwards
//   --failure    expects the first prompt to fail: asserts the structured AIR sessionFailure view instead of pong
//   --env K=V    repeatable: extra env for the agent process (e.g. ANTHROPIC_BASE_URL=http://127.0.0.1:9)
//   --raw        dump the last agent turn's blocks as JSON after each scenario
// Point the CLI's own store elsewhere first when its sessions must not pile up (DSH_HOME=…, PI_CODING_AGENT_DIR=…).
const [agentId = 'opencode', ...flags] = process.argv.slice(2);
const attach = flags.includes('--attach');
const shell = flags.includes('--shell');
const write = flags.includes('--write');
const plan = flags.includes('--plan');
const image = flags.includes('--image');
const background = flags.includes('--background');
const failure = flags.includes('--failure');
const raw = flags.includes('--raw');
const env: Record<string, string> = {};
for (let i = 0; i < flags.length; i++) {
  if (flags[i] !== '--env') continue;
  const kv = flags[i + 1] ?? '';
  const eq = kv.indexOf('=');
  if (eq > 0) env[kv.slice(0, eq)] = kv.slice(eq + 1);
  i++;
}
const project = mkdtempSync(join(tmpdir(), `acpira-${agentId}-project-`));
const store = mkdtempSync(join(tmpdir(), `acpira-${agentId}-store-`));
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const until = async (pred: () => boolean, ms: number, what: string) => {
  const t0 = Date.now();
  while (!pred()) { if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`); await new Promise(r => setTimeout(r, 50)); }
};
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const text = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'text').map(b => b.type === 'text' ? b.markdown : '').join('') ?? '';

// --env merges into the agent's own env: the built-in def is cloned with the overrides since a custom entry
// replaces the whole definition
const agents = Object.keys(env).length
  ? (() => { const d = builtinAgent(agentId); return { [agentId]: { name: d.name, command: d.command, args: d.args, env: { ...d.env, ...env } } }; })()
  : undefined;
if (agents) console.log('env overrides:', Object.keys(env).join(', '));

// The Rust sidecar driven over the envelope protocol, the way the webview drives it (scripts/lib/host.ts)
const host = await Host.start({ cwd: project, home: store, defaultAgent: agentId, agents });
const m = await host.view();
// --write / --plan answer their permission cards deliberately, so the click-through approver stands down there
const autoApprove = !write && !plan;
const approver = setInterval(() => {
  if (!autoApprove) return;
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
// What the webview's quick buttons would offer: the first allow_once / reject_once in wire order, never a label guess
const quickPair = (p: PermissionBlock) => ({
  allow: p.options.find(o => o.kind === 'allow_once'),
  reject: p.options.find(o => o.kind === 'reject_once'),
});
const dump = (v: SessionView) => { if (raw) console.log('raw blocks:', JSON.stringify(lastAgent(v)?.blocks ?? null, null, 1).slice(0, 8000)); };

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
  if (failure) {
    // --failure: the structured AIR sessionFailure view — turn error with failureId/actions plus the notice rows
    const agent = lastAgent(v);
    const notices = v.turns.flatMap(t => t.role === 'agent' ? t.blocks : []).filter(b => b.type === 'notice');
    console.log('sessionFailure view:', JSON.stringify({
      stop: agent?.stop, error: agent?.error,
      notices: notices.map(n => n.type === 'notice'
        ? { id: n.id, revision: n.revision, severity: n.severity, category: n.category, title: n.title, details: n.details, actions: n.actions }
        : n),
    }));
    check('turn ended as error', agent?.stop === 'error', String(agent?.stop));
    check('structured failureId on the turn error', !!agent?.error?.failureId, agent?.error?.failureId ?? String(agent?.error?.kind));
    check('failure actions declared', (agent?.error?.actions?.length ?? 0) > 0, JSON.stringify(agent?.error?.actions));
    check('notice rows present', notices.length > 0, `${notices.length} notice(s)`);
  } else {
    check('pong reply', /pong/i.test(text(v)), text(v).slice(0, 80));
    check('turn ended end_turn', lastAgent(v)?.stop === 'end_turn', String(lastAgent(v)?.stop));
  }
  console.log('commands:', v.commands.length, v.commands.slice(0, 8).map(c => `/${c.name}`).join(' '));
  console.log('usage:', JSON.stringify(v.usage ?? null), 'turn usage:', JSON.stringify(lastAgent(v)?.usage ?? null));

  // Boolean config options arrive only because the client advertised session.configOptions.boolean; each one is
  // flipped on then off through session/set_config_option with a real boolean on the wire (codex `fast-mode`)
  for (const b of v.controls.options.filter(o => o.type === 'boolean')) {
    console.log(`boolean control: ${b.id} (${b.name}) = ${b.value}`);
    const next = b.value === 'true' ? 'false' : 'true';
    await m.handle({ type: 'setConfig', configId: b.id, value: next });
    await until(() => m.active()!.controls.options.find(o => o.id === b.id)?.value === next, 15_000, `${b.id} → ${next}`);
    check(`boolean ${b.id} follows to ${next}`, true, String(m.active()!.controls.options.find(o => o.id === b.id)?.value));
    await m.handle({ type: 'setConfig', configId: b.id, value: b.value! });
    await until(() => m.active()!.controls.options.find(o => o.id === b.id)?.value === b.value, 15_000, `${b.id} → ${b.value}`);
    check(`boolean ${b.id} flips back to ${b.value}`, true, String(m.active()!.controls.options.find(o => o.id === b.id)?.value));
  }
  if (!v.controls.options.some(o => o.type === 'boolean')) console.log('boolean control: none advertised');

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
    // codex-acp answers with a rawOutput { formatted_output, exit_code } receipt — the row must render the text, not the JSON
    check('shell output is plain text, not JSON', exec?.content?.type === 'text' && !/^\s*\{/.test(exec.content.text));
    check('reply relays the output', /acpira-shell-ok/.test(text(v)), text(v).slice(0, 80));
    dump(v);
  }

  if (write) {
    // A mode that asks before edits: Claude 'default', Codex 'read-only'; anything else runs whatever it advertises.
    // Codex read-only only gates files OUTSIDE its writable roots — and its workspaceWrite policy keeps $TMPDIR
    // and /tmp writable — so its note.txt lands in $HOME, outside both. Claude's default asks either way and
    // writes inside the project.
    const want = agentId === 'claude' ? 'default' : agentId === 'codex' ? 'read-only' : undefined;
    if (want && v.controls.modes.some(x => x.id === want)) {
      await m.handle({ type: 'setMode', id: want });
      console.log('mode set to', m.active()!.controls.modeId);
    } else console.log(`mode ${want} not advertised; modes:`, v.controls.modes.map(x => x.id).join(','));
    const target = agentId === 'codex' ? join(homedir(), `acpira-codex-note-${process.pid}.txt`) : join(project, 'note.txt');
    const send = m.handle({ type: 'send', text: `Create a file named ${target === join(project, 'note.txt') ? 'note.txt' : target} containing exactly: acpira` });
    // The card must be observable before it is answered
    try {
      await until(() => lastAgent(m.active()!)?.blocks.some(b => b.type === 'permission') ?? false, 120_000, 'permission card');
    } catch (e) {
      // No card is itself a result (e.g. a read-only refusal): dump what the agent actually did, then fail loudly
      v = m.active()!;
      console.log('reply on timeout:', text(v).slice(0, 200));
      dump(v);
      throw e;
    }
    const perm = lastAgent(m.active()!)!.blocks.find(b => b.type === 'permission') as PermissionBlock;
    console.log('permission:', JSON.stringify({ title: perm.title, description: perm.description, defaultToNo: perm.defaultToNo, options: perm.options }));
    check('_meta.permission title on the card', !!perm.title, perm.title);
    const quick = quickPair(perm);
    check('quick allow + quick reject by kind', !!quick.allow && !!quick.reject, perm.options.map(o => `${o.id}(${o.kind})`).join(' '));
    if (!quick.allow) throw new Error('no allow_once option to click');
    console.log(`clicking quick allow → ${quick.allow.id}`);
    void m.handle({ type: 'permission', sessionId: m.active()!.id, blockId: perm.id, optionId: quick.allow.id });
    await send;
    await until(() => existsSync(target), 15_000, 'note.txt');
    check('note.txt contents are exactly "acpira"', readFileSync(target, 'utf8') === 'acpira', readFileSync(target, 'utf8'));
    rmSync(target, { force: true });
    dump(m.active()!);
  }

  if (plan) {
    // Claude's plan mode is a mode; Codex exposes planning as the collaboration_mode config option
    if (agentId === 'claude' && v.controls.modes.some(x => x.id === 'plan')) {
      await m.handle({ type: 'setMode', id: 'plan' });
      console.log('mode set to', m.active()!.controls.modeId);
    }
    if (agentId === 'codex' && v.controls.options.some(o => o.id === 'collaboration_mode')) {
      await m.handle({ type: 'setConfig', configId: 'collaboration_mode', value: 'plan' });
      console.log('collaboration_mode set to', m.active()!.controls.options.find(o => o.id === 'collaboration_mode')?.value);
    }
    const send = m.handle({ type: 'send', text: 'Plan (do not implement) adding a file hello.txt that says hi, then request approval to implement.' });
    await until(() => {
      const a = lastAgent(m.active()!);
      return !!a?.blocks.some(b => b.type === 'plan_document') && a.blocks.some(b => b.type === 'permission');
    }, 180_000, 'plan document + pending permission');
    let agent = lastAgent(m.active()!)!;
    const doc = agent.blocks.find(b => b.type === 'plan_document') as PlanDocumentBlock;
    const perm = agent.blocks.find(b => b.type === 'permission') as PermissionBlock;
    console.log('plan:', JSON.stringify({ doc: { id: doc.id, title: doc.title, status: doc.status }, perm: { id: perm.id, planId: perm.planId, title: perm.title, options: perm.options.map(o => `${o.id}(${o.kind})`) } }));
    check('plan_document linked to the pending permission', !!perm.planId && perm.planId === doc.id, `planId=${perm.planId} doc=${doc.id}`);
    const reject = quickPair(perm).reject ?? perm.options.find(o => o.kind.startsWith('reject'));
    if (!reject) throw new Error('no reject option to click');
    console.log(`clicking ${reject.id}(${reject.kind})`);
    void m.handle({ type: 'permission', sessionId: m.active()!.id, blockId: perm.id, optionId: reject.id });
    await send;
    agent = lastAgent(m.active()!)!;
    check('permission card resolved', !agent.blocks.some(b => b.type === 'permission'));
    const docAfter = agent.blocks.find((b): b is PlanDocumentBlock => b.type === 'plan_document' && b.id === doc.id);
    check('plan status rejected', docAfter?.status === 'rejected', String(docAfter?.status));
    check('hello.txt was not written', !existsSync(join(project, 'hello.txt')));
    dump(m.active()!);
  }

  if (background) {
    // AIR asyncTasks: a backgrounded shell keeps its tool row running past end_turn and settles from the
    // task's own state updates; a long task is stopped through _session/async_task/stop
    const tasks = (vv: SessionView) => vv.turns.flatMap(t => t.role === 'agent' ? t.blocks : [])
      .filter((b): b is ToolCallBlock => b.type === 'tool_call' && b.asyncTask !== undefined);
    const sleepCount = () => {
      try { return execSync('pgrep -f "sleep 120" || true').toString().trim().split('\n').filter(Boolean).length; }
      catch { return 0; }
    };
    const baseline = sleepCount();
    await m.handle({ type: 'send', text: 'Run exactly the shell command `sleep 20 && echo done` — no `&`, no nohup, no redirection — as a background command, do not wait for it to finish, and reply with just: started' });
    v = m.active()!;
    dump(v);
    const row = tasks(v)[0];
    console.log('async tasks:', JSON.stringify(tasks(v).map(t => ({ id: t.id, status: t.status, background: t.background, task: t.asyncTask }))));
    check('async task attached to a tool row', !!row, `${tasks(v).length} task row(s)`);
    if (row) {
      check('task running after the turn ended', ['running', 'paused'].includes(row.asyncTask!.state), row.asyncTask!.state);
      check('tool row kept running (end_turn did not sweep it)', row.status === 'in_progress', row.status);
      check('row flagged background', row.background === true);
      // A missed terminal update is a FAIL of its own, not an abort: the stop scenario below still runs
      await until(() => tasks(m.active()!).some(t => t.id === row.id && t.asyncTask!.state === 'completed'), 45_000, 'async task completed')
        .catch(e => console.log(String(e)));
      const done = tasks(m.active()!).find(t => t.id === row.id)!;
      check('task reached completed', done.asyncTask!.state === 'completed', done.asyncTask!.state);
      check('row settled completed with the task', done.status === 'completed', done.status);
    }
    // A second, long task: stopped through the host's stopAsyncTask route, then the sleep must be gone
    // Both prompts are spelled out: codex may detach a bare "in the background" request itself (seen:
    // `(sleep 20 && echo done) >/tmp/….log 2>&1 &`), which returns at once and never becomes a background
    // terminal; the adapter only tracks a command that keeps its own shell running
    await m.handle({ type: 'send', text: 'Run exactly the shell command `sleep 120` — no `&`, no nohup, no redirection — as a background command, do not wait for it to finish, and reply with just: started' });
    v = m.active()!;
    dump(v);
    // Whether the first task's terminal update only arrives once the next prompt drives the adapter again
    if (row) console.log('first task after the next prompt:', tasks(v).find(t => t.id === row.id)?.asyncTask?.state);
    const live = tasks(v).filter(t => t.id !== row?.id && ['running', 'paused'].includes(t.asyncTask!.state));
    const t2 = live[0];
    check('second background task running', !!t2, `${live.length} live task(s)`);
    if (t2) {
      console.log('stopping task', t2.asyncTask!.id, 'canStop', t2.asyncTask!.canStop);
      await m.handle({ type: 'stopAsyncTask', sessionId: v.id, taskId: t2.asyncTask!.id });
      await until(() => tasks(m.active()!).some(t => t.id === t2.id && t.asyncTask!.state === 'stopped'), 15_000, 'task stopped')
        .catch(e => console.log(String(e)));
      const stopped = tasks(m.active()!).find(t => t.id === t2.id)!;
      check('task reached stopped', stopped.asyncTask!.state === 'stopped', stopped.asyncTask!.state);
      check('row cancelled with the task', stopped.status === 'cancelled', stopped.status);
      await until(() => sleepCount() <= baseline, 10_000, 'sleep exited').catch(e => console.log(String(e)));
      check('sleep process exited', sleepCount() <= baseline, `${sleepCount()} vs baseline ${baseline}`);
    }
  }

  if (image) {
    // A 1x1 red PNG for the agent to look at through its own image tooling
    writeFileSync(join(project, 'red.png'), Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64'));
    await m.handle({ type: 'send', text: 'Open red.png with your file/image viewing tool and tell me its dominant color in one word.' });
    v = m.active()!;
    const agent = lastAgent(v)!;
    const block = agent.blocks.find(b => b.type === 'image');
    const toolImg = agent.blocks.filter((b): b is ToolCallBlock => b.type === 'tool_call')
      .flatMap(b => b.contents ?? (b.content ? [b.content] : []))
      .find(c => c.type === 'image');
    const blob = block?.type === 'image' ? block.blob : toolImg?.type === 'image' ? toolImg.blob : undefined;
    const uri = block?.type === 'image' ? block.uri : toolImg?.type === 'image' ? toolImg.uri : undefined;
    check('agent emitted an image block or tool image content', !!(block || toolImg));
    const path = blob ? host.blobPath(v.id, blob) : undefined;
    check('image blob exists on disk', !!path && existsSync(path), `${path ?? 'no blob'}${uri ? ` · uri=${uri}` : ''}`);
    console.log('reply:', text(v).slice(0, 120));
    dump(v);
  }
} catch (e) {
  console.log('probe aborted:', e instanceof Error ? e.message : e);
  check('probe ran to the end', false, e instanceof Error ? e.message : String(e));
} finally {
  clearInterval(approver);
  await host.dispose();
}
const failed = checks.filter(c => !c[1]);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
console.log('log tail:\n' + host.logs.filter(l => !/^stderr: /.test(l) || /error|warn|fail/i.test(l)).slice(-15).join('\n'));
rmSync(store, { recursive: true, force: true });
rmSync(project, { recursive: true, force: true });
process.exit(failed.length ? 1 : 0);
