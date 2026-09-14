import { randomUUID } from 'node:crypto';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AcpSession, type SessionDeps } from '../src/host/acp/AcpSession';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { DevinAccountProvider } from '../src/host/accounts/devin';
import { captureTurnSettings } from '../src/shared/turnSettings';
import { groupModels } from '../src/shared/models';
import { EDIT_CONTEXT_MAX_BYTES } from '../src/host/limits';
import { compactionForDisplay } from '../src/webview/chat/compactionDisplay';

// pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-edit-turn.ts grok|devin|kimi [--plan]
// Exercises the production session class in an isolated directory. Local Devin
// credentials remain in memory; output includes only selections and probe replies.
const agent = process.argv[2] ?? 'grok';
const cwd = await mkdtemp(join(tmpdir(), `acpira-edit-${agent}-`));
const registry = new AgentRegistry();
const binary = await registry.resolveBinary(agent);
if (!binary) throw new Error(`CLI unavailable: ${agent}`);
const deps: SessionDeps = {
  registry, log: () => {}, onChange: s => {
    // This probe requests text only. Decline unexpected permission requests.
    for (const turn of s.view().turns) if (turn.role === 'agent') for (const b of turn.blocks) {
      if (b.type !== 'permission') continue;
      const option = b.options.find(o => o.kind === 'reject_once');
      if (option) s.resolvePermission(b.id, option.id);
    }
  },
  blobs: {
    saveBlob: async (_sid, ext, bytes) => { const name = randomUUID() + ext; const path = join(cwd, name); await writeFile(path, bytes); return { name, path }; },
    readBlob: async (_sid, name) => readFile(join(cwd, name)),
  },
};
if (agent === 'devin') {
  const provider = new DevinAccountProvider(cwd, async () => binary);
  const credential = await provider.importLocal();
  if (!credential) throw new Error('Local Devin login unavailable');
  deps.accounts = { spawnEnv: async () => undefined, authenticate: async (_agent, _account, proc) => provider.authenticate!(proc, credential) };
}
const s = AcpSession.fresh(agent, cwd, deps, agent === 'devin' ? 'probe-local' : undefined);
async function oversizedEdit() {
  const retained = `KEEP_${randomUUID().slice(0, 8)}`;
  const attached = `ATTACH_${randomUUID().slice(0, 8)}`;
  const facts = Array.from({ length: 160 }, (_, i) => `Synthetic record ${i}: item-${i} has value ${i * 7} and category ${i % 9}.`).join('\n');
  await s.prompt(`This is a text-only protocol test. Do not use tools or touch files. Remember the retained code ${retained} and these synthetic records. Reply only SEED_OK.\n${facts}`);
  const seed = s.view().turns.at(-1);
  if (seed?.role !== 'agent' || seed.stop !== 'end_turn') throw new Error(`Seed failed: ${seed?.role === 'agent' ? seed.error?.message ?? seed.stop : 'no reply'}`);
  const commandsDeadline = Date.now() + 5000;
  while (!s.canCompact && Date.now() < commandsDeadline) await new Promise(r => setTimeout(r, 50));
  if (!s.canCompact) throw new Error('Agent did not advertise /compact');
  const usageBeforeCompact = s.view().usage;
  await s.compact();
  const compact = s.view().turns.at(-1);
  if (compact?.role !== 'agent') throw new Error('Missing compaction reply');
  const compaction = compactionForDisplay(compact, s.isRunning).blocks.find(b => b.type === 'compaction');
  const compactionReply = compact.blocks.filter(b => b.type === 'text').map(b => b.markdown).join('');
  const compactEvidence = compaction?.status === 'completed' ? 'completion-event-or-text' : agent === 'grok' && !compaction && compact.stop === 'end_turn' ? 'rpc-completed-without-status' : 'not-completed';
  console.log(JSON.stringify({ agent, phase: 'compact-result', evidence: compactEvidence, status: compaction?.status, stop: compact.stop,
    reply: compactionReply, blocks: compact.blocks, usageBefore: usageBeforeCompact, usageAfter: s.view().usage }));
  if (compactEvidence === 'not-completed' || compact.stop !== 'end_turn') throw new Error('Compaction did not complete successfully');
  await s.prompt('Protocol-only test. Do not use tools or modify files. Reply only BEFORE_EDIT_OK.', [
    { kind: 'text', name: 'kept.txt', text: 'Synthetic kept attachment: KEEP_ATTACHMENT_OK.' },
    { kind: 'text', name: 'removed.txt', text: 'Synthetic attachment excluded from the next prompt.' },
  ]);
  const originalReply = s.view().turns.at(-1);
  if (originalReply?.role !== 'agent' || originalReply.stop !== 'end_turn') throw new Error(`Original prompt failed: ${originalReply?.role === 'agent' ? originalReply.error?.message ?? originalReply.stop : 'no reply'}`);
  seed.blocks.push({ type: 'text', markdown: 'Synthetic archived UI-only output.\n'.repeat(30_000) });
  const before = s.toRecord().acpSessionId;
  const previous = JSON.stringify(s.view().turns);
  const historyBytes = Buffer.byteLength(previous, 'utf8');
  if (historyBytes <= EDIT_CONTEXT_MAX_BYTES) throw new Error('Probe did not exercise oversized history');
  const turnIndex = s.view().turns.length - 2;
  const turn = s.view().turns[turnIndex];
  if (turn?.role !== 'user') throw new Error('Missing editable user turn');
  const settings = captureTurnSettings(s.view().controls);
  for (const control of s.view().controls.options) {
    if (control.category !== 'thought_level') continue;
    const next = control.options.find(o => o.id === 'low' && o.id !== control.value) ?? control.options.find(o => o.id !== control.value);
    if (next) settings.config[control.id] = next.id;
  }
  if (process.argv.includes('--plan')) {
    if (!s.view().controls.modes.some(mode => mode.id === 'plan')) throw new Error('Plan mode unavailable');
    settings.modeId = 'plan';
  }
  await s.editTurn({ sessionId: s.id, turnIndex, turnCount: s.view().turns.length, turnId: turn.id, originalText: turn.text,
    text: 'Protocol-only test. Do not use tools or modify files. Report the retained code from the first user message and the marker in the NEW attachment new.txt. Reply only RETAINED_CODE|NEW_ATTACHMENT_MARKER using the actual values.',
    retainedAttachments: [0], attachments: [{ kind: 'text', name: 'new.txt', text: `The new attachment marker is ${attached}.` }], settings });
  while (s.isRunning) await new Promise(r => setTimeout(r, 50));
  const view = s.view();
  const last = view.turns.at(-1);
  const sent = view.turns.at(-2);
  const reply = last?.role === 'agent' ? last.blocks.filter(b => b.type === 'text').map(b => b.markdown).join('') : '';
  const actual = captureTurnSettings(view.controls);
  const sameSettings = actual.modeId === settings.modeId && Object.entries(settings.config).every(([id, value]) => actual.config[id] === value);
  const sameHistory = JSON.stringify(view.turns.slice(0, -2)) === previous;
  const attachments = sent?.role === 'user' ? sent.attachments?.map(a => a.name) : undefined;
  const result = { agent, scenario: 'oversized-after-compact', nativeKept: s.toRecord().acpSessionId === before,
    historyBytes, sameHistory, sameSettings, settings: actual, attachments, stop: last?.role === 'agent' ? last.stop : undefined,
    error: last?.role === 'agent' ? last.error : undefined, reply, retainedContext: reply.includes(retained), newAttachment: reply.includes(attached),
    ordinaryTurn: sent?.role === 'user' && !sent.edited, usage: view.usage };
  console.log(JSON.stringify(result));
  if (!result.nativeKept || !sameHistory || !sameSettings || result.stop !== 'end_turn' || !result.retainedContext
    || !result.newAttachment || !result.ordinaryTurn || JSON.stringify(attachments) !== JSON.stringify(['kept.txt', 'new.txt'])) process.exitCode = 1;
}

const timeout = setTimeout(() => { console.error(JSON.stringify({ agent, error: 'probe timeout' })); s.dispose(); process.exit(1); }, 180_000);
try {
  await s.start();
  if (s.view().status !== 'ready') throw new Error(`Session status: ${s.view().status}`);
  const model = process.argv.find(arg => arg.startsWith('--model='))?.slice('--model='.length);
  if (model) {
    const control = s.view().controls.options.find(c => c.category === 'model');
    if (!control?.options.some(o => o.id === model)) throw new Error(`Model unavailable: ${model}`);
    await s.setConfig(control.id, model);
    if (s.view().controls.options.find(c => c.id === control.id)?.value !== model) throw new Error(`Model selection not applied: ${model}`);
  }
  console.log(JSON.stringify({ agent, phase: 'ready', runtime: s.runtimeInfo(), settings: captureTurnSettings(s.view().controls) }));
  if (process.argv.includes('--oversized')) {
    await oversizedEdit();
  } else {
    const retained = `KEEP_${randomUUID().slice(0, 8)}`;
    const removed = `DROP_${randomUUID().slice(0, 8)}`;
    await s.prompt(`This is a text-only protocol test. Do not use any tools or touch files. Remember the retained code ${retained}. Reply only OK.`);
    await s.prompt(`Text-only test, no tools. Remember the discarded code ${removed}. Reply only OK.`);
    const before = s.toRecord().acpSessionId;
    const turn = s.view().turns[2];
    if (turn?.role !== 'user') throw new Error('Missing second user turn');
    const settings = captureTurnSettings(s.view().controls);
    for (const control of s.view().controls.options) {
      if (control.category === 'thought_level') {
        const next = control.options.find(o => o.id !== control.value);
        if (next) settings.config[control.id] = next.id;
      } else if (control.category === 'model') {
        const family = groupModels(control.options).find(f => f.variants.some(v => v.id === control.value));
        const current = family?.variants.find(v => v.id === control.value);
        const next = family?.variants.find(v => v.effort !== current?.effort && v.fast === current?.fast && v.long === current?.long);
        if (next) settings.config[control.id] = next.id;
      }
    }
    if (process.argv.includes('--plan')) {
      const plan = s.view().controls.modes.find(m => m.id === 'plan');
      if (!plan) throw new Error('Plan mode unavailable');
      settings.modeId = plan.id;
    }
    await s.editTurn({ sessionId: s.id, turnIndex: 2, turnCount: s.view().turns.length, turnId: turn.id, originalText: turn.text,
      text: 'Text-only test. Do not use any tools. What retained code was given in the earlier conversation? Was any discarded code provided before this message? Reply as CODE|yes or CODE|no, using the actual retained code.',
      retainedAttachments: [], attachments: [], settings });
    while (s.isRunning) await new Promise(r => setTimeout(r, 50));
    const last = s.view().turns.at(-1);
    const reply = last?.role === 'agent' ? last.blocks.filter(b => b.type === 'text').map(b => b.markdown).join('') : '';
    const result = { agent, freshPeer: s.toRecord().acpSessionId !== before, turns: s.view().turns.length,
      settings: captureTurnSettings(s.view().controls), stop: last?.role === 'agent' ? last.stop : undefined, reply,
      retainedContext: reply.includes(retained), removedFuture: !reply.includes(removed) && /\|\s*no/i.test(reply) };
    console.log(JSON.stringify(result));
    if (!result.freshPeer || result.turns !== 4 || result.stop !== 'end_turn' || !result.retainedContext || !result.removedFuture) process.exitCode = 1;
  }
} finally { clearTimeout(timeout); s.dispose(); }
