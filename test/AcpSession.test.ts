import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';
import type { AgentBlock, PermissionBlock, SessionOption, ToolCallBlock } from '@shared/transcript';
import { captureTurnSettings } from '@shared/turnSettings';
import type { EditTurnRequest } from '@shared/protocol';
import { MAX_IMAGE_BYTES } from '@shared/attachments';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AcpSession, type CompactionPolicy, type SessionDeps } from '../src/host/acp/AcpSession';

// Launch test/fake-agent.ts via tsx as the agent; the registry holds a custom agent pointing at it
const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));

// Grok-style synthesized modes: not provided by the protocol, declared in the registry
const SYN_MODES: SessionOption[] = [
  { id: 'default', name: 'Agent' },
  { id: 'plan', name: 'Plan' },
  { id: 'yolo', name: 'Auto accept' },
];

function deps(cwd = '/tmp', compaction?: () => CompactionPolicy, modes?: SessionOption[]) {
  const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE], login: 'echo login', modes } });
  const logs: string[] = [];
  let changes = 0;
  // In-memory blob store: remembers what was written so tests can check the payload landed
  const blobs = new Map<string, Uint8Array>();
  const d: SessionDeps = {
    registry, log: (l: string) => logs.push(l), onChange: () => { changes++; }, compaction,
    blobs: {
      saveBlob: async (sid, ext, bytes) => { const name = `b${blobs.size}${ext}`; blobs.set(name, bytes); return { name, path: `/blobs/${sid}/${name}` }; },
      readBlob: async (_sid, name) => { const b = blobs.get(name); if (!b) throw new Error(`no blob ${name}`); return b; },
    },
  };
  return { d, logs, blobs, changes: () => changes, session: () => AcpSession.fresh('fake', cwd, d) };
}

// Wait until a condition holds (5s timeout by default)
async function until(pred: () => boolean, ms = 5000) {
  const t0 = Date.now();
  while (!pred()) {
    if (Date.now() - t0 > ms) throw new Error('timeout');
    await new Promise(r => setTimeout(r, 20));
  }
}

describe('AcpSession', () => {
  it('keeps empty slash receipts and observed settings across persistence without inventing assistant prose', async () => {
    const { session, d } = deps();
    const s = session();
    try {
      await s.start();
      await s.prompt('/silent');
      expect(s.view().turns.at(-1)).toMatchObject({ blocks: [], stop: 'end_turn', command: { name: 'silent' } });
      await s.prompt('/silent-plan');
      expect(s.view().turns.at(-1)).toMatchObject({ blocks: [], stop: 'end_turn', command: { name: 'silent-plan', mode: 'Plan' } });
      await s.prompt('/silent-plan');
      expect(s.view().turns.at(-1)).toMatchObject({ command: { name: 'silent-plan' } });
      expect(s.view().turns.at(-1)).not.toHaveProperty('command.mode');
      await s.prompt('/slash-error');
      expect(s.view().turns.at(-1)).toMatchObject({ stop: 'error', error: { message: expect.stringContaining('Unknown command') } });
      await s.prompt('ordinary message');
      expect(s.view().turns.at(-1)).not.toHaveProperty('command');
      const restored = new AcpSession(s.toRecord(), d);
      expect(restored.view().turns).toEqual(s.view().turns);
      restored.dispose();
    } finally { s.dispose(); }
  });

  it('opening an older session filters repeated completed plan snapshots from follow-up replies', () => {
    const { session, d } = deps();
    const original = session();
    const record = original.toRecord();
    record.turns = [
      { role: 'agent', blocks: [{ type: 'plan', entries: [{ title: 'Done', status: 'completed' }] }] },
      { role: 'user', text: 'Follow-up' },
      { role: 'agent', blocks: [{ type: 'text', markdown: 'Answer' }, { type: 'plan', entries: [{ title: 'Done', status: 'completed' }] }] },
    ];
    const restored = new AcpSession(record, d);
    expect(restored.view().turns[2]).toMatchObject({ blocks: [{ type: 'text', markdown: 'Answer' }] });
    expect(restored.toRecord().turns).toHaveLength(3);
    expect(record.turns[2]).toMatchObject({ blocks: [{ type: 'text' }, { type: 'plan' }] });
    restored.dispose();
    original.dispose();
  });

  it.each(['plan-grok', 'plan-devin', 'plan-devin-early'])('plan approval %s: show full plan and change execution model before approval', async prompt => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      await s.setMode('plan');
      const pending = s.prompt(prompt);
      await until(() => s.view().turns.some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'permission')));
      const blocks = s.view().turns.flatMap(t => t.role === 'agent' ? t.blocks : []);
      const permission = blocks.find(b => b.type === 'permission')!;
      const plan = blocks.find(b => b.type === 'plan_document')!;
      if (permission.type !== 'permission' || plan.type !== 'plan_document') throw new Error('Missing plan');
      expect(permission.planId).toBe(plan.id);
      expect(plan.markdown).toBe('# Demo plan\n\nCreate hello.txt.');
      expect(plan.path).toBe(prompt === 'plan-devin-early' ? undefined : '/Users/test/.devin/plans/demo.md');
      const allow = permission.options.find(o => o.kind === 'allow_once')!;
      s.resolvePermission(permission.id, 'invented');
      expect(s.view().running).toBe(true);
      await s.buildPlan(plan.id, { configId: 'model', value: 'm2' }, allow.id);
      await pending;
      expect(plan.status).toBe('approved');
      expect(plan.path).toBe('/Users/test/.devin/plans/demo.md');
      expect(s.view().turns.flatMap(t => t.role === 'agent' ? t.blocks.filter(b => b.type === 'plan_document') : [])).toEqual([plan]);
      expect(s.view().controls.modeId).toBe('agent');
      expect(JSON.stringify(s.view().turns)).toContain('APPROVED model=m2');
      expect(s.toRecord().turns.flatMap(t => t.role === 'agent' ? t.blocks : []).some(b => b.type === 'plan_document')).toBe(true);
      const count = s.view().turns.length;
      await s.buildPlan(plan.id, undefined, allow.id);
      expect(s.view().turns).toHaveLength(count);
    } finally { s.dispose(); }
  });

  it.each(['reject', 'cancel', 'dispose'])('Grok plan approval handles %s without leaving an orphaned permission', async action => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      await s.setMode('plan');
      const pending = s.prompt('plan-grok');
      await until(() => s.view().turns.some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'permission')));
      const b = s.view().turns.flatMap(t => t.role === 'agent' ? t.blocks : []).find(b => b.type === 'permission')!;
      if (b.type !== 'permission') throw new Error();
      if (action === 'reject') s.resolvePermission(b.id, 'rejected');
      else if (action === 'cancel') await s.cancel();
      else s.dispose();
      await pending;
      expect(s.view().running).toBe(false);
      expect(s.view().turns.flatMap(t => t.role === 'agent' ? t.blocks : []).some(b => b.type === 'permission')).toBe(false);
      expect(s.view().controls.modeId).toBe('plan');
    } finally { s.dispose(); }
  });

  it('Build a saved plan switches mode/model and dispatches its full content once', async () => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      await s.setMode('plan');
      await s.prompt('plan-file');
      const plan = s.view().turns.flatMap(t => t.role === 'agent' ? t.blocks : []).find(b => b.type === 'plan_document')!;
      if (plan.type !== 'plan_document') throw new Error();
      await Promise.all([s.buildPlan(plan.id, { configId: 'model', value: 'm2' }), s.buildPlan(plan.id)]);
      expect(s.view().turns).toHaveLength(4);
      expect(s.view().turns[2]).toMatchObject({ role: 'user', planId: plan.id, text: `Implement the following approved plan:\n\n${plan.markdown}` });
      expect(s.toRecord().turns[2]).toMatchObject({ planId: plan.id });
      expect(s.view().controls.modeId).toBe('agent');
      expect(s.view().controls.options.find(c => c.id === 'model')?.value).toBe('m2');
    } finally { s.dispose(); }
  });

  it('retrying failed plan execution keeps its internal instruction out of user messages', async () => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      await s.prompt('plan-file');
      const plan = s.view().turns.flatMap(t => t.role === 'agent' ? t.blocks : []).find(b => b.type === 'plan_document')!;
      if (plan.type !== 'plan_document') throw new Error('Missing plan');
      plan.markdown += '\n\nfail once';
      await s.buildPlan(plan.id);
      expect(s.view().turns.at(-1)).toMatchObject({ role: 'agent', stop: 'error' });
      await s.retryTurn();
      expect(s.view().turns).toHaveLength(4);
      expect(s.view().turns[2]).toMatchObject({ role: 'user', planId: plan.id,
        text: `Implement the following approved plan:\n\n${plan.markdown}` });
      expect(s.view().turns.at(-1)).toMatchObject({ role: 'agent', stop: 'end_turn' });
    } finally { s.dispose(); }
  });

  it('a rejected model selection leaves the plan approval waiting', async () => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      const pending = s.prompt('plan-devin');
      await until(() => s.view().turns.some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'permission')));
      const plan = s.view().turns.flatMap(t => t.role === 'agent' ? t.blocks : []).find(b => b.type === 'plan_document')!;
      if (plan.type !== 'plan_document') throw new Error();
      // An advertised model may still fail when the peer applies it.
      s.view().controls.options.find(c => c.id === 'model')!.options.push({ id: 'unavailable', name: 'Unavailable' });
      await expect(s.buildPlan(plan.id, { configId: 'model', value: 'unavailable' })).rejects.toThrow('Model unavailable');
      expect(s.view().running).toBe(true);
      expect(plan.status).toBe('ready');
      await s.cancel();
      await pending;
    } finally { s.dispose(); }
  });

  it('start session: receives modes and configOptions (model sorted before thought_level)', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    const v = s.view();
    expect(v.status).toBe('ready');
    expect(v.controls.modes.map(m => m.id)).toEqual(['agent', 'plan']);
    expect(v.controls.modeId).toBe('agent');
    expect(v.controls.options.map(o => o.id)).toEqual(['model', 'effort']);
    expect(v.controls.options[0]).toMatchObject({ category: 'model', value: 'm1' });
    expect(v.controls.options[0]!.options.map(o => o.id)).toEqual(['m1', 'm2']);
    expect(v.controls.options[1]).toMatchObject({ name: 'Reasoning', category: 'thought_level', value: 'high' });
    s.dispose();
  });

  it('one prompt turn: thought / plan / text merged into blocks, title and commands updated, echoed user_message_chunk not duplicated', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    await s.prompt('hi');
    const v = s.view();
    expect(v.running).toBe(false);
    expect(v.turns).toHaveLength(2);
    expect(v.turns[0]).toMatchObject({ role: 'user', text: 'hi' });
    const agent = v.turns[1]!;
    expect(agent.role).toBe('agent');
    if (agent.role !== 'agent') return;
    expect(agent.startedAt).toEqual(expect.any(Number));
    expect(agent.endedAt).toEqual(expect.any(Number));
    expect(agent.endedAt!).toBeGreaterThanOrEqual(agent.startedAt!);
    expect(agent.blocks.map(b => b.type)).toEqual(['thought', 'plan', 'text']);
    expect(agent.blocks[0]).toMatchObject({ type: 'thought', text: 'thinking hard', streaming: false });
    expect(agent.blocks[2]).toMatchObject({ type: 'text', markdown: 'hello world', streaming: false });
    expect(agent.activity).toBeUndefined();
    expect(v.title).toBe('Fake title');
    expect(v.commands).toEqual([{ name: 'compact', description: 'compact it' }]);
    s.dispose();
  });

  it('attachments: image and dropped text are written to the blob store and sent as image / resource blocks, a file goes as resource_link; the turn keeps only references', async () => {
    const { session, blobs } = deps();
    const s = session();
    await s.start();
    const png = Buffer.from('fake-png-bytes').toString('base64');
    await s.prompt('echo blocks', [
      { kind: 'image', mimeType: 'image/png', data: png, name: 'shot.png' },
      { kind: 'text', name: 'notes.md', text: '# notes' },
      { kind: 'file', uri: 'file:///repo/src/a.ts', name: 'src/a.ts' },
    ]);
    const v = s.view();
    expect(v.turns[0]).toMatchObject({
      role: 'user', text: 'echo blocks',
      attachments: [
        { kind: 'image', blob: 'b0.png', mimeType: 'image/png', name: 'shot.png' },
        { kind: 'text', blob: 'b1.txt', name: 'notes.md' },
        { kind: 'file', uri: 'file:///repo/src/a.ts', name: 'src/a.ts' },
      ],
    });
    expect(Buffer.from(blobs.get('b0.png')!).toString()).toBe('fake-png-bytes');
    expect(Buffer.from(blobs.get('b1.txt')!).toString()).toBe('# notes');
    // the fake agent echoes the block types and key fields it received
    const agent = v.turns[1]!;
    if (agent.role !== 'agent') throw new Error();
    const echoed = agent.blocks.find(b => b.type === 'text');
    expect(echoed).toMatchObject({ type: 'text', markdown: 'text · image:image/png · resource:file:///blobs/' + s.id + '/b1.txt:# notes · resource_link:file:///repo/src/a.ts:src/a.ts' });
    s.dispose();
  });

  it('attachments only: the text block is omitted and the title comes from what was attached', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    await s.prompt('', [{ kind: 'image', mimeType: 'image/png', data: 'AAAA' }, { kind: 'file', uri: 'file:///repo/README.md', name: 'README.md' }]);
    const v = s.view();
    expect(v.turns[0]).toMatchObject({ role: 'user', text: '' });
    expect(v.title).toBe('1 images, README.md');
    const agent = v.turns[1]!;
    if (agent.role !== 'agent') throw new Error();
    expect(agent.blocks.find(b => b.type === 'text')).toMatchObject({ markdown: 'image:image/png · resource_link:file:///repo/README.md:README.md' });
    // the echoed user_message_chunk (Grok sends the image back too) must not create a second user turn
    expect(v.turns.filter(t => t.role === 'user')).toHaveLength(1);
    s.dispose();
  });

  it('file draft pointing at an image on disk is read and sent as pixels; an oversized image draft is dropped with a note, the rest still goes', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-att-'));
    const png = join(dir, 'shot.png');
    writeFileSync(png, 'real-png-bytes');
    const { d, session, blobs } = deps();
    const notes: string[] = [];
    d.notify = t => notes.push(t);
    const s = session();
    await s.start();
    const huge = Buffer.alloc(MAX_IMAGE_BYTES + 1).toString('base64');
    await s.prompt('echo blocks', [
      { kind: 'file', uri: pathToFileURL(png).href, name: 'shot.png' },
      { kind: 'image', mimeType: 'image/png', data: huge, name: 'huge.png' },
    ]);
    const v = s.view();
    expect(v.turns[0]).toMatchObject({ role: 'user', attachments: [{ kind: 'image', blob: 'b0.png', mimeType: 'image/png', name: 'shot.png' }] });
    expect(Buffer.from(blobs.get('b0.png')!).toString()).toBe('real-png-bytes');
    const agent = v.turns[1]!;
    if (agent.role !== 'agent') throw new Error();
    expect(agent.blocks.find(b => b.type === 'text')).toMatchObject({ markdown: 'text · image:image/png' });
    expect(notes).toEqual([`huge.png exceeds ${MAX_IMAGE_BYTES >> 20} MB, skipped`]);
    s.dispose();
  });

  it('blob store failing does not lose the prompt: it still goes out inline, the attachment just has no preview', async () => {
    const { d, session } = deps();
    d.blobs = { saveBlob: async () => { throw new Error('disk full'); }, readBlob: async () => { throw new Error('nope'); } };
    const notes: string[] = [];
    d.notify = t => notes.push(t);
    const s = session();
    await s.start();
    await s.prompt('echo blocks', [{ kind: 'image', mimeType: 'image/png', data: 'AAAA', name: 'shot.png' }, { kind: 'text', name: 'n.md', text: 'x' }]);
    const v = s.view();
    expect(v.turns[0]).toMatchObject({ role: 'user', text: 'echo blocks', attachments: [{ kind: 'image', mimeType: 'image/png', name: 'shot.png' }, { kind: 'text', name: 'n.md' }] });
    const agent = v.turns[1]!;
    if (agent.role !== 'agent') throw new Error();
    expect(agent.blocks.find(b => b.type === 'text')).toMatchObject({ markdown: 'text · image:image/png · resource:attachment:///n.md:x' });
    expect(notes).toHaveLength(2);
    expect(notes[0]).toContain('disk full');
    s.dispose();
  });

  it('cancel while staging drops the prompt without a turn; a send arriving meanwhile is queued and goes out afterwards', async () => {
    const { d, session } = deps();
    let release!: () => void;
    const gate = new Promise<void>(r => { release = r; });
    const inner = d.blobs;
    d.blobs = { ...inner, saveBlob: async (...a) => { await gate; return inner.saveBlob(...a); } };
    const s = session();
    await s.start();
    const first = s.prompt('echo blocks', [{ kind: 'image', mimeType: 'image/png', data: 'AAAA' }]);
    expect(s.view().running).toBe(true);
    await s.cancel();
    await s.prompt('hi');
    expect(s.view().queued?.map(q => q.text)).toEqual(['hi']);
    release();
    await first;
    await until(() => !s.view().running && s.view().turns.length === 2);
    const v = s.view();
    expect(v.turns[0]).toMatchObject({ role: 'user', text: 'hi' });
    expect(v.queued).toBeUndefined();
    s.dispose();
  });

  it('dispose while staging: nothing is appended or sent afterwards', async () => {
    const { d, session, blobs } = deps();
    let release!: () => void;
    const gate = new Promise<void>(r => { release = r; });
    const inner = d.blobs;
    d.blobs = { ...inner, saveBlob: async (...a) => { await gate; return inner.saveBlob(...a); } };
    const s = session();
    await s.start();
    const p = s.prompt('echo blocks', [{ kind: 'image', mimeType: 'image/png', data: 'AAAA' }]);
    s.dispose();
    release();
    await p;
    expect(s.view().turns).toEqual([]);
    expect(s.view().running).toBe(false);
    // the blob had already been handed to the store by the time dispose landed; the manager removes the directory when it drops the session
    expect(blobs.size).toBe(1);
  });

  it('permission: card appears → approve → tool completes, diff normalized, usage arrives', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    const p = s.prompt('use tool');
    await until(() => s.view().turns.some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'permission')));
    const agent = s.view().turns[1]!;
    if (agent.role !== 'agent') throw new Error();
    const perm = agent.blocks.find(b => b.type === 'permission') as PermissionBlock;
    expect(perm.command).toBe('pnpm test');
    expect(perm.options.map(o => o.id)).toEqual(['allow', 'reject']);
    expect(agent.activity?.label).toBe('Awaiting approval');
    s.resolvePermission(perm.id, 'allow');
    await p;
    const v = s.view();
    const blocks = (v.turns[1] as { blocks: AgentBlock[] }).blocks;
    expect(blocks.some(b => b.type === 'permission')).toBe(false);
    const tc1 = blocks.find((b): b is ToolCallBlock => b.type === 'tool_call' && b.id === 'tc1')!;
    expect(tc1).toMatchObject({ kind: 'execute', verb: 'Run', target: 'pnpm test', targetMono: true, status: 'completed' });
    expect(tc1.content).toEqual({ type: 'text', text: '12 passed' });
    const tc2 = blocks.find((b): b is ToolCallBlock => b.type === 'tool_call' && b.id === 'tc2')!;
    expect(tc2).toMatchObject({ kind: 'edit', target: 'a.ts', diffStat: { add: 2, del: 1 } });
    expect(v.usage).toEqual({ used: 1234, size: 100000, cost: 0.01 });
    s.dispose();
  });

  it('synthesized modes: when the protocol omits modes, backfill from the registry, default to the first one, setMode goes through session/set_mode', async () => {
    mkdirSync('/tmp/acpira-no-modes', { recursive: true });
    const { session } = deps('/tmp/acpira-no-modes', undefined, SYN_MODES);
    const s = session();
    await s.start();
    const v = s.view();
    expect(v.status).toBe('ready');
    expect(v.controls.modes.map(m => m.id)).toEqual(['default', 'plan', 'yolo']);
    expect(v.controls.modeId).toBe('default');
    // configOptions unaffected, still land in controls.options
    expect(v.controls.options.map(o => o.id)).toEqual(['model', 'effort']);
    await s.setMode('plan');
    expect(s.view().controls.modeId).toBe('plan');
    await s.setMode('default');
    expect(s.view().controls.modeId).toBe('default');
    s.dispose();
  });

  it('synthesized modes: yolo auto-approves permission requests, no card shown', async () => {
    mkdirSync('/tmp/acpira-no-modes', { recursive: true });
    const { session } = deps('/tmp/acpira-no-modes', undefined, SYN_MODES);
    const s = session();
    await s.start();
    // plan → yolo: covers the "pull the CLI back to default first" path
    await s.setMode('plan');
    await s.setMode('yolo');
    expect(s.view().controls.modeId).toBe('yolo');
    await s.prompt('use tool');
    const v = s.view();
    const blocks = (v.turns[1] as { blocks: AgentBlock[] }).blocks;
    expect(blocks.some(b => b.type === 'permission')).toBe(false);
    const tc1 = blocks.find((b): b is ToolCallBlock => b.type === 'tool_call' && b.id === 'tc1')!;
    expect(tc1.status).toBe('completed');
    s.dispose();
  });

  it('synthesized modes: switching into yolo approves pending permissions too', async () => {
    mkdirSync('/tmp/acpira-no-modes', { recursive: true });
    const { session } = deps('/tmp/acpira-no-modes', undefined, SYN_MODES);
    const s = session();
    await s.start();
    const p = s.prompt('use tool');
    await until(() => s.view().turns.some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'permission')));
    await s.setMode('yolo');
    await p;
    const v = s.view();
    expect(v.running).toBe(false);
    const blocks = (v.turns[1] as { blocks: AgentBlock[] }).blocks;
    expect(blocks.some(b => b.type === 'permission')).toBe(false);
    const tc1 = blocks.find((b): b is ToolCallBlock => b.type === 'tool_call' && b.id === 'tc1')!;
    expect(tc1.status).toBe('completed');
    s.dispose();
  });

  it('session views carry a monotonic rev and leave running false after the prompt returns', async () => {
    const { session } = deps();
    const s = session();
    try {
      const before = s.view().rev ?? 0;
      await s.start();
      const ready = s.view().rev ?? 0;
      expect(ready).toBeGreaterThan(before);
      expect(s.view().rev).toBe(ready);
      await s.prompt('hi');
      const done = s.view();
      expect(done.running).toBe(false);
      expect(done.rev ?? 0).toBeGreaterThan(ready);
      const last = done.turns.at(-1);
      expect(last?.role === 'agent' && last.stop).toBe('end_turn');
    } finally { s.dispose(); }
  });

  it('cancel: text stops midway, the turn wraps up, can send again', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    const p = s.prompt('slow');
    await until(() => { const t = s.view().turns[1]; return t?.role === 'agent' && t.blocks.some(b => b.type === 'text'); });
    await s.cancel();
    await p;
    expect(s.view().running).toBe(false);
    await s.prompt('hi');
    expect(s.view().turns).toHaveLength(4);
    s.dispose();
  });

  it('prompt error: the turn ends with stop=error carrying code / kind / retryable, the session stays ready; retryTurn drops both turns and sends the same prompt again', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    await s.prompt('please fail');
    let v = s.view();
    expect(v.status).toBe('ready');
    expect(v.running).toBe(false);
    expect(v.error).toBeUndefined();
    expect(v.turns).toHaveLength(2);
    const agent = v.turns[1]!;
    if (agent.role !== 'agent') throw new Error();
    expect(agent.stop).toBe('error');
    expect(agent.error).toEqual({ message: 'Upstream error: quota exhausted', code: -32603, kind: 'upstream_error', retryable: true });
    await s.retryTurn();
    v = s.view();
    expect(v.turns).toHaveLength(2);
    expect(v.turns[0]).toMatchObject({ role: 'user', text: 'please fail' });
    const again = v.turns[1]!;
    if (again.role !== 'agent') throw new Error();
    expect(again.stop).toBe('end_turn');
    expect(again.blocks.some(b => b.type === 'text')).toBe(true);
    s.dispose();
  });

  it('retryTurn: attachments are rebuilt from their blobs; nothing happens after a normal end', async () => {
    const { session, blobs } = deps();
    const s = session();
    await s.start();
    await s.prompt('fail with picture', [{ kind: 'image', mimeType: 'image/png', data: Buffer.from('png!').toString('base64'), name: 'shot.png' }]);
    expect(blobs.size).toBe(1);
    await s.retryTurn();
    const v = s.view();
    expect(v.turns).toHaveLength(2);
    const user = v.turns[0]!;
    expect(user).toMatchObject({ role: 'user', text: 'fail with picture', attachments: [{ kind: 'image', mimeType: 'image/png', name: 'shot.png' }] });
    // The re-sent image is byte-for-byte the original (the fake store names blobs by count, the real one by content hash)
    const blob = user.role === 'user' && user.attachments?.[0]?.kind === 'image' ? user.attachments[0].blob : undefined;
    expect(Buffer.from(blobs.get(blob ?? '')!).toString()).toBe('png!');
    const agent = v.turns[1]!;
    if (agent.role !== 'agent') throw new Error();
    expect(agent.stop).toBe('end_turn');
    expect(agent.blocks[0]).toMatchObject({ type: 'text', markdown: 'text · image:image/png' });
    await s.retryTurn();
    expect(s.view().turns).toHaveLength(2);
    s.dispose();
  });

  it('short stops: refusal leaves an empty turn with stop=refusal, max_tokens keeps the text and stop=max_tokens; a normal turn records end_turn', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    await s.prompt('refuse this');
    await s.prompt('truncate this');
    await s.prompt('hi');
    const [, refused, , truncated, , ok] = s.view().turns;
    if (refused?.role !== 'agent' || truncated?.role !== 'agent' || ok?.role !== 'agent') throw new Error();
    expect(refused.stop).toBe('refusal');
    expect(refused.blocks).toEqual([]);
    expect(truncated.stop).toBe('max_tokens');
    expect(truncated.blocks.at(-1)).toMatchObject({ type: 'text', markdown: 'once upon a', streaming: false });
    expect(ok.stop).toBe('end_turn');
    expect(ok.error).toBeUndefined();
    s.dispose();
  });

  it('queue: a prompt sent while starting waits for ready then goes out', async () => {
    const { session } = deps();
    const s = session();
    try {
      await s.prompt('hi');
      expect(s.view().queued?.map(q => q.text)).toEqual(['hi']);
      await s.start();
      await until(() => (s.view().turns.some(t => t.role === 'agent') && !s.view().queued) || s.view().status !== 'ready');
      expect(s.view().status).toBe('ready');
      expect(s.view().queued).toBeUndefined();
      expect(s.view().turns[0]).toMatchObject({ role: 'user', text: 'hi' });
    } finally { s.dispose(); }
  });

  it('queue: sending another prompt while running auto-sends it after the turn ends', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    const p = s.prompt('slow');
    await until(() => s.view().running);
    await s.prompt('hi');
    expect(s.view().queued?.map(q => q.text)).toEqual(['hi']);
    await s.cancel();
    await p;
    await until(() => s.view().turns.length === 4 && !s.view().running);
    expect(s.view().queued).toBeUndefined();
    s.dispose();
  });

  // Several sends during one turn line up in order and go out one after another; attachments are staged at queue time so the
  // queue row shows them, and the flushed turn carries the same blobs. Removing / editing addresses an entry by id
  it('queue: several prompts keep their order, a queued image is staged at once, entries can be edited in place or removed', async () => {
    const { session, blobs } = deps();
    const s = session();
    try {
      await s.start();
      const p = s.prompt('slow');
      await until(() => s.view().running);
      await s.prompt('first', [{ kind: 'image', mimeType: 'image/png', data: 'AAAA', name: 'a.png' }]);
      await s.prompt('second');
      await s.prompt('third');
      let queued = s.view().queued!;
      expect(queued.map(q => q.text)).toEqual(['first', 'second', 'third']);
      expect(queued[0]!.attachments).toEqual([{ kind: 'image', blob: 'b0.png', mimeType: 'image/png', name: 'a.png' }]);
      expect(blobs.has('b0.png')).toBe(true);
      // Edit the first: new text, the image kept, a text draft added; the entry stays first
      await s.editQueued(queued[0]!.id, 'first edited', [0], [{ kind: 'text', name: 'n.md', text: 'x' }]);
      queued = s.view().queued!;
      expect(queued.map(q => q.text)).toEqual(['first edited', 'second', 'third']);
      expect(queued[0]!.attachments.map(a => a.kind)).toEqual(['image', 'text']);
      // Remove the middle one; removing something already gone is a no-op, editing it is an error
      s.dequeue(queued[1]!.id);
      expect(s.view().queued?.map(q => q.text)).toEqual(['first edited', 'third']);
      s.dequeue(queued[1]!.id);
      await expect(s.editQueued(queued[1]!.id, 'x', [], [])).rejects.toThrow();
      // Emptying an entry removes it
      await s.editQueued(queued[2]!.id, '   ', [], []);
      expect(s.view().queued?.map(q => q.text)).toEqual(['first edited']);
      await s.cancel();
      await p;
      await until(() => s.view().turns.length === 4 && !s.view().running);
      expect(s.view().queued).toBeUndefined();
      expect(s.view().turns[2]).toMatchObject({ role: 'user', text: 'first edited', attachments: [{ kind: 'image', mimeType: 'image/png' }, { kind: 'text', name: 'n.md' }] });
      expect(s.view().turns[2]).not.toHaveProperty('edited');
    } finally { s.dispose(); }
  });

  it('queue: send now cancels the active turn, sends the selected payload once, then preserves the remaining order', async () => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      const running = s.prompt('slow');
      await until(() => s.view().turns.length === 2);
      await s.prompt('first');
      await s.prompt('priority', [{ kind: 'image', mimeType: 'image/png', data: 'AAAA', name: 'priority.png' }]);
      await s.prompt('last');
      const selected = s.view().queued![1]!;
      await Promise.all([s.sendQueued(selected.id), s.sendQueued(selected.id)]);
      await running;
      await until(() => !s.isRunning && !s.view().queued);
      expect(s.view().turns.filter(t => t.role === 'user').map(t => t.text)).toEqual(['slow', 'priority', 'first', 'last']);
      expect(s.view().turns[1]).toMatchObject({ role: 'agent', stop: 'cancelled' });
      expect(s.view().turns[2]).toMatchObject({ role: 'user', attachments: selected.attachments });
      // A stale row must not interrupt the next unrelated turn.
      const next = s.prompt('slow again');
      await until(() => s.view().turns.length === 10);
      await s.sendQueued(selected.id);
      expect(s.isRunning).toBe(true);
      await s.cancel();
      await next;
    } finally { s.dispose(); }
  });

  it('queue: send now during attachment staging waits for the cancelled staging operation', async () => {
    const { d, session } = deps();
    let release!: () => void;
    const staging = new Promise<void>(resolve => { release = resolve; });
    const save = d.blobs.saveBlob;
    d.blobs.saveBlob = async (...args) => { await staging; return save(...args); };
    const s = session();
    try {
      await s.start();
      const running = s.prompt('original', [{ kind: 'image', mimeType: 'image/png', data: 'AAAA' }]);
      await s.prompt('priority');
      await s.sendQueued(s.view().queued![0]!.id);
      expect(s.view().turns).toEqual([]);
      expect(s.view().queued![0]).toMatchObject({ text: 'priority', sending: true });
      release();
      await running;
      await until(() => !s.isRunning);
      expect(s.view().turns.filter(t => t.role === 'user').map(t => t.text)).toEqual(['priority']);
      expect(s.view().queued).toBeUndefined();
    } finally { release(); s.dispose(); }
  });

  // The session list sorts by updatedAt: only the user's message may move a session, never the stream that follows it
  it('updatedAt: bumped once by the prompt, then stable across streamed updates and the turn end', async () => {
    const { d, session } = deps();
    const seen: string[] = [];
    d.onChange = s => { seen.push(s.view().updatedAt); };
    const s = session();
    await s.start();
    const before = s.view().updatedAt;
    await new Promise(r => setTimeout(r, 5));
    seen.length = 0;
    await s.prompt('hi');
    expect(seen.length).toBeGreaterThan(2);
    expect(seen[0]).not.toBe(before);
    expect(new Set(seen).size).toBe(1);
    expect(s.view().updatedAt).toBe(seen[0]);
    s.dispose();
  });

  it('switch mode / model / effort; rename and pin leave updatedAt untouched', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    await s.setMode('plan');
    expect(s.view().controls.modeId).toBe('plan');
    await s.setConfig('model', 'm2');
    expect(s.view().controls.options.find(o => o.id === 'model')?.value).toBe('m2');
    await s.setConfig('effort', 'low');
    expect(s.view().controls.options.find(o => o.id === 'effort')?.value).toBe('low');
    expect(s.view().controls.options.find(o => o.id === 'model')?.value).toBe('m2');
    await s.setConfig('nope', 'x');
    const before = s.view().updatedAt;
    s.rename('  改个名  ');
    s.setPinned(true);
    expect(s.view().title).toBe('改个名');
    expect(s.toRecord().pinned).toBe(true);
    expect(s.view().updatedAt).toBe(before);
    s.dispose();
  });

  it('resume: with acpSessionId goes through session/resume; unknown to the process → readonly', async () => {
    const { d, session } = deps();
    const s = session();
    await s.start();
    await s.prompt('hi');
    const record = s.toRecord();
    s.dispose();
    // new process answers invalidParams "unknown session" — the peer doesn't know the id, the same conclusion as session_not_found:
    // the transcript already ran, so it stays read-only instead of silently continuing on a fresh native context
    const s2 = new AcpSession(record, d);
    await s2.start();
    expect(s2.view().status).toBe('readonly');
    expect(s2.view().turns).toHaveLength(2);
    expect(s2.toRecord().acpSessionId).toBe(record.acpSessionId);
    // No fresh native session was opened, so the persisted command list stays until a peer replaces it
    expect(s2.view().commands).toEqual([{ name: 'compact', description: 'compact it' }]);
    s2.dispose();
  });

  it('resume: a failed restore attempt (not gone, not unsupported) lands on the error state and retry reconnects', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-flaky-resume-'));
    writeFileSync(join(dir, 'resume.lock'), '');
    const { d, session } = deps(dir);
    const s = session();
    await s.start();
    await s.prompt('hi');
    const record = s.toRecord();
    s.dispose();
    // resume answers -32603 while resume.lock exists: an internal error is not "can't resume" — the session goes to
    // the error Notice (Retry = full reconnect + resume), not to read-only
    const s2 = new AcpSession(record, d);
    await s2.start();
    expect(s2.view().status).toBe('error');
    expect(s2.view().error).toContain('transient restore failure');
    rmSync(join(dir, 'resume.lock'));
    await s2.retry();
    expect(s2.view().status).toBe('ready');
    expect(s2.view().turns).toHaveLength(2);
    expect(s2.toRecord().acpSessionId).toBe(record.acpSessionId);
    s2.dispose();
  });

  it('resume: a typed session_locked is reported as held elsewhere, retryable through the error Notice', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-locked-'));
    const { d, session } = deps(dir);
    const s = session();
    await s.start();
    await s.prompt('hi');
    const record = s.toRecord();
    s.dispose();
    const s2 = new AcpSession(record, d);
    await s2.start();
    expect(s2.view().status).toBe('error');
    expect(s2.view().error).toContain('held by another');
    expect(s2.view().turns).toHaveLength(2);
    s2.dispose();
  });

  it('resume: peer reports session_not_found — a session that never talked is replaced transparently, one with history stays read-only', async () => {
    mkdirSync('/tmp/acpira-gone', { recursive: true });
    const { d, logs, session } = deps('/tmp/acpira-gone');
    const s = session();
    await s.start();
    await s.prompt('hi');
    const record = s.toRecord();
    s.dispose();
    // The transcript already ran: swapping in a fresh native session would keep the old conversation on an empty
    // context (compaction included). Read-only, history kept, the native id retained so a later open can retry
    const s2 = new AcpSession(record, d);
    await s2.start();
    expect(s2.view().status).toBe('readonly');
    expect(s2.view().turns).toHaveLength(2);
    expect(s2.toRecord().acpSessionId).toBe(record.acpSessionId);
    expect(logs.filter(l => l.includes('session/new ok')).length).toBe(1);
    s2.dispose();
    // An empty session (Devin sweeps exactly those when its process exits) is replaced transparently — nothing visible lost its context
    const empty = session();
    await empty.start();
    const emptyRecord = empty.toRecord();
    empty.dispose();
    const s3 = new AcpSession(emptyRecord, d);
    await s3.start();
    expect(s3.view().status).toBe('ready');
    expect(s3.view().turns).toHaveLength(0);
    expect(logs.filter(l => l.includes('session/new ok')).length).toBe(3);
    // The replacement native session advertised nothing: the old connection's slash commands do not carry over
    expect(s3.view().commands).toEqual([]);
    s3.dispose();
  });

  it('a prompt answered session_not_found leaves ready: the error Notice reconnects instead of resending into the dead session', async () => {
    const { session } = deps();
    const s = session();
    await s.start();
    await s.prompt('hi');
    await s.prompt('prompt-session-gone');
    expect(s.view().turns.at(-1)).toMatchObject({ role: 'agent', stop: 'error' });
    // the native session is gone — resending over this connection could only fail the same way
    expect(s.view().status).toBe('error');
    // Retry = reconnect + resume; the fresh process doesn't know the id either → read-only history, still no silent context swap
    await s.retry();
    expect(s.view().status).toBe('readonly');
    expect(s.view().turns).toHaveLength(4);
    s.dispose();
  });

  it('auto compaction: usage over threshold at turn end and /compact available → auto-send an auto turn, compaction row in_progress→completed, usage drops; no resend if usage did not grow back', async () => {
    const { session } = deps('/tmp', () => ({ atTokens: 300_000, auto: true }));
    const s = session();
    await s.start();
    await s.prompt('big');
    // auto compaction is already queued (async) when prompt() returns; wait for it to finish
    await until(() => s.view().turns.length === 4 && !s.view().running);
    const v = s.view();
    expect(v.turns[2]).toEqual({ role: 'user', text: '/compact', auto: true });
    const t = v.turns[3]!;
    if (t.role !== 'agent') throw new Error();
    expect(t.blocks).toEqual([{ type: 'compaction', id: 'cp1', status: 'completed' }]);
    expect(v.usage?.used).toBeLessThan(300_000);
    expect(v.title).toBe('big');

    // grows again → compacts once more, but this time the fake agent can't compact (usage unchanged)
    await s.prompt('big');
    await until(() => s.view().turns.length === 8 && !s.view().running);
    const used = s.view().usage!.used;
    expect(used).toBeGreaterThan(300_000);
    // usage didn't grow back much: the next turn end doesn't resend /compact
    await s.prompt('hi');
    await new Promise(r => setTimeout(r, 200));
    expect(s.view().turns).toHaveLength(10);
    expect(s.view().usage!.used).toBe(used);
    s.dispose();
  });

  it('auto compaction runs before a follow-up queued during the over-threshold turn', async () => {
    const { session } = deps('/tmp', () => ({ atTokens: 300_000, auto: true }));
    const s = session();
    await s.start();
    const first = s.prompt('big');
    await s.prompt('follow-up');
    await first;
    await until(() => !s.isRunning && s.view().turns.some(t => t.role === 'user' && t.text === 'follow-up'));
    const users = s.view().turns.filter(t => t.role === 'user');
    expect(users).toMatchObject([
      { text: 'big' },
      { text: '/compact', auto: true },
      { text: 'follow-up' },
    ]);
    expect(s.view().usage?.used).toBeLessThan(300_000);
    s.dispose();
  });

  it('auto compaction runs before the next typed prompt when a previous turn left usage over the threshold', async () => {
    let auto = false;
    const { session } = deps('/tmp', () => ({ atTokens: 300_000, auto }));
    const s = session();
    await s.start();
    await s.prompt('big');
    expect(s.view().turns).toHaveLength(2);
    expect(s.view().usage!.used).toBeGreaterThan(300_000);
    auto = true;
    await s.prompt('hi');
    await until(() => !s.isRunning && s.view().turns.some(t => t.role === 'user' && t.text === 'hi'));
    const users = s.view().turns.filter(t => t.role === 'user');
    expect(users).toMatchObject([
      { text: 'big' },
      { text: '/compact', auto: true },
      { text: 'hi' },
    ]);
    s.dispose();
  });

  it('manual compaction: sends /compact when available; errors when not', async () => {
    const { session } = deps('/tmp', () => ({ atTokens: 300_000, auto: false }));
    const s = session();
    await s.start();
    await expect(s.compact()).rejects.toThrow('/compact');
    await s.prompt('big');
    await new Promise(r => setTimeout(r, 200));
    expect(s.view().turns).toHaveLength(2);
    await s.compact();
    expect(s.view().turns).toHaveLength(4);
    expect(s.view().turns[2]).toMatchObject({ role: 'user', text: '/compact' });
    s.dispose();
  });

  it('login: session/new fails with -32000 → auth_required → authenticate → retry succeeds', async () => {
    mkdirSync('/tmp/acpira-needs-auth', { recursive: true });
    const { session } = deps('/tmp/acpira-needs-auth');
    const s = session();
    await s.start();
    expect(s.view().status).toBe('auth_required');
    expect(s.view().authMethods?.[0]?.id).toBe('fake.login');
    // The reason the CLI logged to stderr right before -32000 is surfaced instead of a bare "log in"
    expect(s.view().error).toBe('provider managed:fake has no credential configured');
    await s.authenticate();
    await s.retry();
    expect(s.view().status).toBe('ready');
    expect(s.view().error).toBeUndefined();
    s.dispose();
  });
});


function historyEdit(s: AcpSession, turnIndex: number, text = 'inspect-history'): EditTurnRequest {
  const view = s.view();
  const turn = view.turns[turnIndex];
  if (turn?.role !== 'user') throw new Error('Expected user turn');
  return { sessionId: s.id, turnIndex, turnCount: view.turns.length, originalText: turn.text, turnId: turn.id,
    text, retainedAttachments: (turn.attachments ?? []).map((_, i) => i), attachments: [], settings: captureTurnSettings(view.controls) };
}

describe('historical message editing', () => {
  it.each([1, 2])('resending an unchanged message after %i empty failures keeps native compacted context', async attempts => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      await s.prompt('earlier-context');
      // The UI retains old tool output even after native compaction. It must
      // never be injected into an unchanged failed-message retry.
      const earlier = s.toRecord().turns[1]!;
      if (earlier.role !== 'agent') throw new Error('Missing history');
      earlier.blocks.push({ type: 'text', markdown: 'archived output '.repeat(250_000) });
      const text = attempts === 2 ? 'fail-twice' : 'please fail';
      for (let i = 0; i < attempts; i++) await s.prompt(text);
      expect(s.view().turns.at(-1)).toMatchObject({ stop: 'error', blocks: [] });
      const nativeId = s.toRecord().acpSessionId;
      await s.editTurn(historyEdit(s, 2, text));
      await until(() => !s.isRunning);
      expect(s.toRecord().acpSessionId).toBe(nativeId);
      expect(s.view().turns).toHaveLength(4);
      expect(s.view().turns[1]).toBe(earlier);
      expect(s.view().turns[2]).not.toHaveProperty('edited');
      expect(s.view().turns[3]).toMatchObject({ stop: 'end_turn' });
    } finally { s.dispose(); }
  });

  it('starts a fresh peer with only earlier context and applies mode/effort before resending', async () => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      await s.prompt('earlier-context');
      await s.prompt('replaced-original');
      await s.prompt('discarded-future');
      const oldPeer = s.toRecord().acpSessionId;
      const edit = historyEdit(s, 2);
      edit.settings.modeId = 'plan';
      edit.settings.config.effort = 'low';
      edit.settings.config.model = 'm2';
      await s.editTurn(edit);
      await until(() => !s.isRunning);
      expect(s.toRecord().acpSessionId).not.toBe(oldPeer);
      const turns = s.view().turns;
      expect(turns).toHaveLength(4);
      expect(turns[0]).toMatchObject({ text: 'earlier-context', settings: { config: { effort: 'high' } } });
      expect(turns[2]).toMatchObject({ text: 'inspect-history', settings: { modeId: 'plan', config: { effort: 'low', model: 'm2' } } });
      const reply = JSON.stringify(turns[3]);
      expect(reply).toContain('earlier-context');
      expect(reply).not.toContain('replaced-original');
      expect(reply).not.toContain('discarded-future');
      expect(reply).toContain('low');
      expect(reply).toContain('m2');
      expect(reply).toContain('plan');
    } finally { s.dispose(); }
  });

  it('rebuilds context again when retrying a failed edited prompt', async () => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      await s.prompt('earlier-context');
      await s.prompt('original');
      await s.editTurn(historyEdit(s, 2, 'please fail'));
      await until(() => !s.isRunning);
      expect(s.view().turns[3]).toMatchObject({ stop: 'error' });
      const failedPeer = s.toRecord().acpSessionId;
      await s.retryTurn();
      await until(() => !s.isRunning);
      expect(s.toRecord().acpSessionId).not.toBe(failedPeer);
      expect(s.view().turns).toHaveLength(4);
      expect(s.view().turns[0]).toMatchObject({ text: 'earlier-context' });
      expect(s.view().turns[2]).toMatchObject({ text: 'please fail', edited: true });
    } finally { s.dispose(); }
  });

  it('keeps retained image bytes, removes selected attachments and adds new ones', async () => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      await s.prompt('original', [
        { kind: 'image', name: 'old.png', mimeType: 'image/png', data: 'aGVsbG8=' },
        { kind: 'text', name: 'remove.txt', text: 'removed attachment content' },
      ]);
      const edit = historyEdit(s, 0);
      edit.retainedAttachments = [0];
      edit.attachments = [{ kind: 'text', name: 'new.txt', text: 'new attachment content' }];
      await s.editTurn(edit);
      await until(() => !s.isRunning);
      expect(s.view().turns).toHaveLength(2);
      expect(s.view().turns[0]).toMatchObject({ attachments: [{ kind: 'image', name: 'old.png' }, { kind: 'text', name: 'new.txt' }] });
      const reply = JSON.stringify(s.view().turns[1]);
      expect(reply).toContain('aGVsbG8=');
      expect(reply).toContain('new attachment content');
      expect(reply).not.toContain('removed attachment content');
    } finally { s.dispose(); }
  });

  it('preserves the original transcript and peer after stale edits or unavailable settings', async () => {
    const { session } = deps();
    const s = session();
    try {
      await s.start();
      await s.prompt('original');
      const before = JSON.stringify(s.view().turns);
      const peer = s.toRecord().acpSessionId;
      const stale = historyEdit(s, 0);
      stale.turnCount += 2;
      await expect(s.editTurn(stale)).rejects.toThrow();
      const invalid = historyEdit(s, 0);
      invalid.settings.config.model = 'unavailable';
      await expect(s.editTurn(invalid)).rejects.toThrow();
      expect(JSON.stringify(s.view().turns)).toBe(before);
      expect(s.toRecord().acpSessionId).toBe(peer);
      expect(s.view().controls.options.find(c => c.id === 'model')?.value).toBe('m1');
      expect(s.isRunning).toBe(false);
    } finally { s.dispose(); }
  });

  it('does not replace history when a retained blob is missing', async () => {
    const { session, blobs } = deps();
    const s = session();
    try {
      await s.start();
      await s.prompt('original', [{ kind: 'text', name: 'lost.txt', text: 'payload' }]);
      const before = JSON.stringify(s.view().turns);
      const peer = s.toRecord().acpSessionId;
      blobs.clear();
      await expect(s.editTurn(historyEdit(s, 0))).rejects.toThrow();
      expect(JSON.stringify(s.view().turns)).toBe(before);
      expect(s.toRecord().acpSessionId).toBe(peer);
      expect(s.isRunning).toBe(false);
    } finally { s.dispose(); }
  });

  it('rejects double submission and cancels before replacing history', async () => {
    const { session, d } = deps();
    const s = session();
    let release!: () => void;
    try {
      await s.start();
      await s.prompt('original', [{ kind: 'text', name: 'wait.txt', text: 'payload' }]);
      const before = JSON.stringify(s.view().turns);
      const read = d.blobs.readBlob;
      d.blobs.readBlob = async (...args) => { await new Promise<void>(r => { release = r; }); return read(...args); };
      const edit = historyEdit(s, 0);
      const pending = s.editTurn(edit);
      await expect(s.editTurn(edit)).rejects.toThrow();
      await s.cancel();
      release();
      await expect(pending).rejects.toThrow();
      expect(JSON.stringify(s.view().turns)).toBe(before);
      expect(s.isRunning).toBe(false);
    } finally { s.dispose(); }
  });
});
