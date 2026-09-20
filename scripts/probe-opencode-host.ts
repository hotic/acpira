import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { PermissionBlock, SessionView, ToolCallBlock } from '@shared/transcript';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { SessionManager } from '../src/host/SessionManager';
import { TranscriptStore } from '../src/host/store/TranscriptStore';

// Host-path acceptance for the OpenCode integration, against the real `opencode acp` (needs a configured provider):
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-opencode-host.ts [--keep] [--model provider/model]
// Drives SessionManager the way the webview does: new session → controls (mode / model / effort) → set effort + mode → one prompt →
// a two-file write (permission cards answered like a click; both diffs must land on the tool block) → a second manager on a
// fresh store lists the native sessions and imports the first one, replaying its history through session/load.
// Every OpenCode session created here persists in OpenCode's own store (that is what session/list reads); --keep leaves the temp project.
const keep = process.argv.includes('--keep');
// --model <provider/model>: switch the session's model before the first prompt (the default route may be slow or unavailable)
const modelArg = process.argv.indexOf('--model');
const model = modelArg >= 0 ? process.argv[modelArg + 1] : undefined;
const project = mkdtempSync(join(tmpdir(), 'acpira-oc-project-'));
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const until = async (pred: () => boolean, ms: number, what: string) => {
  const t0 = Date.now();
  while (!pred()) { if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`); await new Promise(r => setTimeout(r, 50)); }
};
const logs: string[] = [];
function makeManager(store: string) {
  return new SessionManager({
    registry: new AgentRegistry(), store: new TranscriptStore(store),
    log: l => logs.push(l), cwd: () => project, defaultAgent: () => 'opencode',
    runInTerminal: () => {}, toast: (l, text) => console.log(`toast ${l}: ${text}`),
  });
}
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const text = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'text').map(b => b.type === 'text' ? b.markdown : '').join('') ?? '';

const storeA = mkdtempSync(join(tmpdir(), 'acpira-oc-store-a-'));
const m1 = makeManager(storeA);
await m1.newSession('opencode');
await until(() => m1.active()?.status === 'ready', 60_000, 'ready');
let v = m1.active()!;
const record = m1.handle.bind(m1);
console.log('controls:', JSON.stringify({ modes: v.controls.modes.map(m => m.id), modeId: v.controls.modeId, options: v.controls.options.map(o => `${o.id}[${o.category}]=${o.value}(${o.options.length})`) }));
check('modes come from the mode config option', v.controls.modes.some(m => m.id === 'build') && v.controls.modes.some(m => m.id === 'plan'), v.controls.modes.map(m => m.id).join(','));
check('model + effort options present', v.controls.options.some(o => o.id === 'model') && v.controls.options.some(o => o.id === 'effort'));
await record({ type: 'setConfig', configId: 'effort', value: 'high' });
v = m1.active()!;
check('effort switched to high', v.controls.options.find(o => o.id === 'effort')?.value === 'high', String(v.controls.options.find(o => o.id === 'effort')?.value));
await record({ type: 'setMode', id: 'plan' });
v = m1.active()!;
check('mode switched to plan through set_config_option', v.controls.modeId === 'plan', String(v.controls.modeId));
await record({ type: 'setMode', id: 'build' });
if (model) {
  await record({ type: 'setConfig', configId: 'model', value: model });
  v = m1.active()!;
  check(`model switched to ${model}`, v.controls.options.find(o => o.id === 'model')?.value === model, String(v.controls.options.find(o => o.id === 'model')?.value));
}

await record({ type: 'send', text: 'Reply with exactly the word pong and nothing else.' });
await until(() => !m1.active()!.running && m1.active()!.turns.length >= 2, 120_000, 'first reply');
v = m1.active()!;
check('first reply is pong', /pong/i.test(text(v)), text(v).slice(0, 60));
check('commands advertised (after the first turn)', v.commands.length > 0, `${v.commands.length} commands`);
check('turn usage stamped', !!lastAgent(v)?.usage, JSON.stringify(lastAgent(v)?.usage ?? null).slice(0, 160));
check('session usage from usage_update', !!v.usage, JSON.stringify(v.usage ?? null));

// Two file writes in one turn; approve every permission card the way the webview click does
const sendPromise = record({ type: 'send', text: `Using your file-writing tool, create exactly two new files in ${project}: a.txt with the single line "alpha" and b.txt with the single line "beta". Do not ask for confirmation; after both writes reply with the single word done.` });
const approved = new Set<string>();
const approver = setInterval(() => {
  const cur = m1.active();
  const turn = cur ? lastAgent(cur) : undefined;
  for (const b of turn?.blocks ?? []) {
    if (b.type !== 'permission' || approved.has(b.id)) continue;
    const opt = (b as PermissionBlock).options.find(o => o.kind === 'allow_once') ?? (b as PermissionBlock).options[0];
    if (!opt) continue;
    approved.add(b.id);
    console.log(`approving permission ${b.id}: ${b.title} → ${opt.id}(${opt.kind}) · options ${(b as PermissionBlock).options.map(o => `${o.id}(${o.kind})`).join(' ')}`);
    void record({ type: 'permission', sessionId: cur!.id, blockId: b.id, optionId: opt.id });
  }
}, 100);
try {
  await sendPromise;
  await until(() => !m1.active()!.running, 180_000, 'write turn');
} finally { clearInterval(approver); }
v = m1.active()!;
const tools = (lastAgent(v)?.blocks ?? []).filter((b): b is ToolCallBlock => b.type === 'tool_call');
console.log('tools:', JSON.stringify(tools.map(t => ({ kind: t.kind, target: t.target, status: t.status, content: t.content?.type, contents: t.contents?.length, diffStat: t.diffStat, locations: t.locations }))));
const edits = tools.filter(t => t.kind === 'edit');
const diffsSeen = edits.reduce((n, t) => n + (t.contents ? t.contents.filter(c => c.type === 'diff').length : t.content?.type === 'diff' ? 1 : 0), 0);
check('two file writes recorded', edits.length >= 2 || diffsSeen >= 2, `${edits.length} edit tools, ${diffsSeen} diffs, permissions approved ${approved.size}`);
check('write turn finished cleanly', lastAgent(v)?.stop === 'end_turn', String(lastAgent(v)?.stop));

const nativeId = m1.sessions().find(s => s.id === v.id)?.acpSessionId;
console.log('native session id:', nativeId);
await m1.dispose();

// A second host on a fresh store: the session exists only on OpenCode's side now
const storeB = mkdtempSync(join(tmpdir(), 'acpira-oc-store-b-'));
const m2 = makeManager(storeB);
const listed = await m2.listNativeSessions('opencode');
console.log('native list:', JSON.stringify(listed.map(s => ({ id: s.sessionId.slice(0, 14), title: s.title, updatedAt: s.updatedAt, localId: s.localId })).slice(0, 5)));
const mine = listed.find(s => s.sessionId === nativeId);
check('session/list shows the session with no local owner', !!mine && !mine.localId, mine ? `title ${mine.title}` : 'not listed');
if (mine) {
  const viewer = m2.attach();
  await m2.importNativeSession(viewer, 'opencode', mine);
  await until(() => viewer.active()?.status === 'ready' || viewer.active()?.status === 'error', 60_000, 'import');
  const iv = viewer.active()!;
  const turnsBefore = iv.turns.length;
  console.log('imported:', JSON.stringify({ status: iv.status, error: iv.error, title: iv.title, turns: iv.turns.map(t => t.role === 'user' ? `user:${t.text.slice(0, 30)}` : `agent:${t.blocks.map(b => b.type).join('+')}:${t.stop}`) }));
  check('import replayed the history', iv.status === 'ready' && iv.turns.filter(t => t.role === 'user').length >= 2, `${iv.turns.length} turns`);
  check('replayed turns are sealed', iv.turns.every(t => t.role === 'user' || (t.stop !== undefined && t.blocks.every(b => !('streaming' in b) || !b.streaming))));
  const again = await m2.listNativeSessions('opencode');
  check('re-listing marks it imported', again.find(s => s.sessionId === nativeId)?.localId === iv.id);
  // The latest native reply was "done", so the question must point at the first turn or a literal-minded model answers with that
  await m2.handle({ type: 'send', sessionId: iv.id, text: 'In this conversation, what single word did you reply to the very first message with? Answer with that single word only.' });
  await until(() => !viewer.active()!.running && viewer.active()!.turns.length > turnsBefore, 240_000, 'follow-up on imported session');
  const fv = viewer.active()!;
  check('imported session continues the native context', /pong/i.test(text(fv)), text(fv).slice(0, 60));
}
await m2.dispose();

const failed = checks.filter(c => !c[1]);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
if (failed.length) console.log('interesting log lines:', logs.filter(l => /failed|error|ignored|close/i.test(l)).slice(-20).join('\n'));
if (!keep) { rmSync(storeA, { recursive: true, force: true }); rmSync(storeB, { recursive: true, force: true }); rmSync(project, { recursive: true, force: true }); }
process.exit(failed.length ? 1 : 0);
