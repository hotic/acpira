import { mkdtemp, readFile, realpath, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { ChatGptBridgeStore } from '../src/host/external/ChatGptBridgeStore';
import { chatgptSessionId, OUTPUT_LIMIT, parseChatGptEvent, STALE_AFTER_MS } from '../src/host/external/chatgptEvents';
import { chatgptBinding } from '../src/host/external/chatgptBinding';

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const fn of cleanups.splice(0)) await fn(); });
async function setup() {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'acpira-chatgpt-')));
  let now = 1_800_000_000_000;
  const dir = join(root, 'bridges', 'chatgpt');
  const store = new ChatGptBridgeStore(dir, () => {}, () => now, '/extension/dist/chatgpt-bridge.cjs');
  await store.init();
  cleanups.push(async () => { await store.dispose(); await rm(root, { recursive: true, force: true }); });
  const view = await store.open('test-conversation-a', root, 'ChatGPT test');
  let seq = 0;
  const send = (body: object) => store.accept(view.id, { id: `event-${++seq}`, turnId: 'turn-a', ...body });
  return { root, dir, store, view, send, clock: (ms: number) => { now += ms; } };
}

describe('ChatGPT external session bridge', () => {
  it('keeps a distinct channel, project and stable source identity without starting ACP', async () => {
    const { store, root, view } = await setup();
    expect(view).toMatchObject({ agent: 'chatgpt', status: 'readonly', running: false, cwd: root });
    expect(view.external?.connectionPrompt).toContain(view.id);
    expect((await store.open('test-conversation-a', root)).id).toBe(view.id);
    const other = await store.open('test-conversation-b', root);
    expect(other.id).not.toBe(view.id);
    expect(store.summaries()).toHaveLength(2);
    await expect(store.open('test-conversation-a', tmpdir())).rejects.toThrow('another project');
    expect(chatgptSessionId('../../escape')).toMatch(/^chatgpt-[a-f0-9]{32}$/);
  });

  it('renders real turn, visible messages, streaming output and completion receipts', async () => {
    const { store, view, send, clock } = await setup();
    await send({ type: 'turn_start', text: 'Inspect this test project' });
    await send({ type: 'message', messageId: 'progress', phase: 'commentary', text: 'Checking' });
    await send({ type: 'message', messageId: 'progress', phase: 'commentary', text: 'Checking the project' });
    await send({ type: 'tool_start', callId: 'call-a', name: 'Shell', kind: 'execute', target: 'echo ok', input: { command: 'echo ok' } });
    expect(store.view(view.id)?.running).toBe(true);
    await send({ type: 'tool_output', callId: 'call-a', text: 'ok\n' });
    await expect(send({ type: 'turn_end', stop: 'end_turn' })).rejects.toThrow('no completion receipt');
    clock(2000);
    await send({ type: 'tool_end', callId: 'call-a', status: 'completed', detail: 'exit 0' });
    await send({ type: 'message', messageId: 'answer', phase: 'final', text: 'The check passed.' });
    await send({ type: 'turn_end', stop: 'end_turn' });
    const result = store.view(view.id)!;
    expect(result.running).toBe(false);
    expect(result.turns[0]).toMatchObject({ role: 'user', text: 'Inspect this test project' });
    const turn = result.turns[1]!;
    if (turn.role !== 'agent') throw new Error('Missing agent turn');
    expect(turn.blocks).toHaveLength(3);
    expect(turn.blocks[0]).toMatchObject({ type: 'text', markdown: 'Checking the project' });
    expect(turn.blocks[1]).toMatchObject({ type: 'tool_call', status: 'completed', meta: 'exit 0', content: { text: expect.stringContaining('ok\n') } });
    expect(turn.stop).toBe('end_turn');
  });

  it('does not invent completion after a disconnect and resumes on a heartbeat', async () => {
    const { store, view, send, clock } = await setup();
    await send({ type: 'turn_start', text: 'A long operation' });
    const before = store.view(view.id)!.rev!;
    clock(STALE_AFTER_MS + 1); await store.refresh();
    const stale = store.view(view.id)!;
    expect(stale).toMatchObject({ running: false, external: { state: 'stale' } });
    expect(stale.rev).toBeGreaterThan(before);
    expect(stale.turns[1]).not.toHaveProperty('stop');
    expect(stale.turns[1]).not.toHaveProperty('endedAt');
    await send({ type: 'heartbeat' });
    expect(store.view(view.id)).toMatchObject({ running: true, external: { state: 'receiving' } });
    expect(store.view(view.id)!.rev).toBeGreaterThan(stale.rev!);
  });

  it('deduplicates retry receipts but rejects conflicting IDs and hidden-message phases', async () => {
    const { store, view } = await setup();
    const e = { id: 'one', turnId: 'turn-a', type: 'turn_start', text: 'hello' };
    await store.accept(view.id, e);
    const revision = store.view(view.id)!.rev;
    await store.accept(view.id, e);
    expect(store.view(view.id)!.rev).toBe(revision);
    await expect(store.accept(view.id, { ...e, text: 'different' })).rejects.toThrow('different content');
    expect(() => parseChatGptEvent({ id: '__proto__', turnId: 'x', type: 'heartbeat' })).toThrow();
    await expect(store.accept(view.id, { id: 'two', turnId: 'turn-a', type: 'message', messageId: 'm', text: 'not allowed', phase: 'analysis' })).rejects.toThrow('visible');
    expect(store.view(view.id)!.turns).toHaveLength(2);
  });

  it('merges concurrent writers and replays into another host without overwriting events', async () => {
    const { root, dir, store, view, send } = await setup();
    const second = new ChatGptBridgeStore(dir);
    await second.init(); cleanups.unshift(() => second.dispose());
    await send({ type: 'turn_start', text: 'concurrency test' });
    await Promise.all(Array.from({ length: 16 }, (_, i) => (i % 2 ? store : second).accept(view.id, {
      id: `parallel-${i}`, turnId: 'turn-a', type: 'message', messageId: `message-${i}`, phase: 'commentary', text: `result ${i}`,
    })));
    await store.refresh(); await second.refresh();
    expect(store.view(view.id)?.turns).toEqual(second.view(view.id)?.turns);
    const last = store.view(view.id)?.turns.at(-1);
    expect(last?.role === 'agent' && last.blocks.length).toBe(16);
    if (process.platform !== 'win32') expect((await stat(join(dir, `${view.id}.json`))).mode & 0o777).toBe(0o600);
    expect(root).toBeTruthy();
  });

  it('preserves deletion across writer retries, allows undo and later removes transcript data', async () => {
    const { store, dir, view, send, clock } = await setup();
    await send({ type: 'turn_start', text: 'private test fixture' });
    await store.delete(view.id);
    expect(store.view(view.id)).toBeUndefined();
    await expect(send({ type: 'heartbeat' })).rejects.toThrow('deleted');
    await store.restore(view.id); expect(store.view(view.id)).toBeDefined();
    await store.delete(view.id); clock(31_000); await store.refresh();
    await expect(store.restore(view.id)).rejects.toThrow('expired');
    const disk = JSON.parse(await readFile(join(dir, `${view.id}.json`), 'utf8'));
    expect(disk.turns).toEqual([]); expect(disk.receipts).toEqual({});
    await expect(store.open('test-conversation-a', disk.cwd)).rejects.toThrow('deleted');
  });

  it('marks capped output explicitly and only accepts real successful diff receipts', async () => {
    const { store, view, send } = await setup();
    await send({ type: 'turn_start', text: 'test output limits' });
    await send({ type: 'tool_start', callId: 'x', name: 'write', kind: 'edit' });
    await send({ type: 'tool_output', callId: 'x', text: 'x'.repeat(OUTPUT_LIMIT + 20) });
    const turn = store.view(view.id)!.turns.at(-1)!;
    if (turn.role !== 'agent') throw new Error('Missing turn');
    expect(turn.blocks[0]).toMatchObject({ content: { text: expect.stringContaining('[Output truncated') } });
    await expect(send({ type: 'tool_end', callId: 'x', status: 'failed', diff: { path: '/a', oldText: 'a', newText: 'b' } })).rejects.toThrow('cannot claim');
    await send({ type: 'tool_end', callId: 'x', status: 'completed', diff: { path: '/a', oldText: 'a', newText: 'b' } });
    const final = store.view(view.id)!.turns.at(-1)!;
    expect(final.role === 'agent' && final.blocks[0]).toMatchObject({ content: { type: 'diff', source: { oldText: 'a', newText: 'b' } } });
  });

  it('rejects symlink records and malformed event ordering without mutating other files', async () => {
    const { root, store, dir, view, send } = await setup();
    await expect(send({ type: 'tool_output', callId: 'x', text: 'late' })).rejects.toThrow('not active');
    const outside = join(root, 'untouched.json'); await writeFile(outside, '{}');
    const id = chatgptSessionId('symlink'); await symlink(outside, join(dir, `${id}.json`));
    await expect(store.open('symlink', root)).rejects.toThrow('Invalid');
    expect(await readFile(outside, 'utf8')).toBe('{}');
    expect(store.view(view.id)?.turns).toEqual([]);
  });

  it('quotes paths in binding instructions and never substitutes a Codex invocation', async () => {
    const { view } = await setup();
    const text = chatgptBinding(view, "/a path/it's here/bridge.cjs", '/test home', 'darwin');
    expect(text).toContain("'\\''");
    expect(text).toContain('prompt'); expect(text).toContain('exec'); expect(text).toContain('explicitly bridged');
    expect(text).not.toContain('codex exec');
  });
});
