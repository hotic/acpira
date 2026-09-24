import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import type { AgentBlock, AgentTurn, SessionView, Turn } from '../src/shared/transcript';
import { FAKE, Shell } from './sidecarShell';

// Session-loop contract at the envelope level against the Rust sidecar: only what a shell / webview can observe is asserted

const lastAgent = (v: SessionView): AgentTurn | undefined => {
  const t = v.turns.at(-1);
  return t?.role === 'agent' ? t : undefined;
};
const blocks = (v: SessionView): AgentBlock[] => lastAgent(v)?.blocks ?? [];
// Run the fake agent on node directly (not the tsx wrapper): a SIGKILL must reach the process that ignores SIGTERM, not its parent
const LOADER = fileURLToPath(new URL('../node_modules/tsx/dist/loader.mjs', import.meta.url));
const pidsWith = (mark: string): string[] => {
  try { return execFileSync('pgrep', ['-f', mark], { encoding: 'utf8' }).split('\n').filter(Boolean); } catch { return []; }
};
const idle = (turns: number) => (m: { session: SessionView }) => !m.session.running && m.session.turns.length === turns;

describe('sidecar session contract', () => {
  const shells: Shell[] = [];
  const dirs: string[] = [];
  afterEach(async () => {
    for (const s of shells.splice(0)) await s.kill();
    for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
  });

  function dirsFor(suffix = '') {
    const home = mkdtempSync(join(tmpdir(), 'acpira-contract-home-'));
    const cwd = mkdtempSync(join(tmpdir(), `acpira-contract-ws-${suffix}`));
    dirs.push(home, cwd);
    return { home, cwd };
  }

  function shell(at = dirsFor()) {
    const s = new Shell(at.home, at.cwd);
    shells.push(s);
    return s;
  }

  async function started(s: Shell, agentOver: Record<string, unknown> = {}) {
    await s.hello({ client: { name: 'contract', version: '0', capabilities: [] } }, agentOver);
    const init = await s.open('V');
    const id = init.state.active!.id;
    await s.hostMsg('V', 'session', m => m.session.id === id && m.session.status === 'ready');
    return id;
  }

  // The sidecar exits as soon as shutdown returns, so its agent processes must be gone by then: a stubborn one is SIGKILLed after the
  // shutdown grace instead of being left behind with its SIGTERM → SIGKILL timer never firing
  it.skipIf(process.platform === 'win32')('shutdown ends every agent process first, even one that ignores SIGTERM', async () => {
    const mark = `acpira-stubborn-${randomUUID()}`;
    const s = shell();
    await started(s, { command: process.execPath, args: ['--import', LOADER, FAKE, mark], env: { FAKE_STUBBORN: '1' } });
    expect(pidsWith(mark).length).toBeGreaterThan(0);
    await s.kill();
    expect(s.exitCode).toBe(0);
    expect(pidsWith(mark)).toEqual([]);
  });

  it('a permission card answered by the webview lets the tool run; usage and the diff land in the turn', async () => {
    const s = shell();
    const id = await started(s);
    s.view('V', { type: 'send', sessionId: id, text: 'tool' });
    const asked = await s.hostMsg('V', 'session', m => blocks(m.session).some(b => b.type === 'permission'));
    const card = blocks(asked.session).find(b => b.type === 'permission');
    expect(card).toMatchObject({ type: 'permission', command: 'pnpm test', options: [{ id: 'allow', kind: 'allow_once' }, { id: 'reject', kind: 'reject_once' }] });
    s.view('V', { type: 'permission', sessionId: id, blockId: card!.id, optionId: 'allow' });
    const done = await s.hostMsg('V', 'session', idle(2));
    const b = blocks(done.session);
    expect(b.some(x => x.type === 'permission')).toBe(false);
    expect(b.find(x => x.type === 'tool_call' && x.id === 'tc1')).toMatchObject({ status: 'completed' });
    expect(b.find(x => x.type === 'tool_call' && x.id === 'tc2')).toMatchObject({ kind: 'edit', content: { type: 'diff' } });
    expect(b.some(x => x.type === 'text' && x.markdown === 'tests passed')).toBe(true);
    expect(done.session.usage).toMatchObject({ used: 1234, size: 100000 });
    expect(lastAgent(done.session)?.stop).toBe('end_turn');
  });

  it('stop cancels a streaming turn; a message sent meanwhile queues and goes out after the cancel', async () => {
    const s = shell();
    const id = await started(s);
    s.view('V', { type: 'send', sessionId: id, text: 'slow' });
    await s.hostMsg('V', 'session', m => m.session.running && blocks(m.session).some(b => b.type === 'text'));
    s.view('V', { type: 'send', sessionId: id, text: 'hi' });
    const queued = await s.hostMsg('V', 'session', m => (m.session.queued?.length ?? 0) === 1);
    expect(queued.session.queued![0]).toMatchObject({ text: 'hi' });
    s.view('V', { type: 'stop', sessionId: id });
    const done = await s.hostMsg('V', 'session', idle(4), 15_000);
    const turns = done.session.turns;
    expect(turns[1]).toMatchObject({ role: 'agent', stop: 'cancelled' });
    expect(turns[2]).toMatchObject({ role: 'user', text: 'hi' });
    expect((turns[3] as AgentTurn).blocks.some(b => b.type === 'text' && b.markdown === 'hello world')).toBe(true);
    expect(done.session.queued).toBeUndefined();
  });

  it('structured questions: Grok private request and Kimi form elicitation become one question card', async () => {
    const s = shell();
    const id = await started(s);
    s.view('V', { type: 'send', sessionId: id, text: 'ask-grok' });
    const asked = await s.hostMsg('V', 'session', m => blocks(m.session).some(b => b.type === 'question'));
    const q = blocks(asked.session).find(b => b.type === 'question');
    if (q?.type !== 'question') throw new Error('no question block');
    expect(q.questions.map(x => [x.kind, x.options.map(o => o.label)])).toEqual([['single', ['report', 'notes']], ['multiple', ['src', 'docs']]]);
    s.view('V', { type: 'answer', sessionId: id, blockId: q.id, answers: { [q.questions[0]!.id]: 'notes', [q.questions[1]!.id]: ['docs'] } });
    const done = await s.hostMsg('V', 'session', idle(2));
    const settled = blocks(done.session).find(b => b.type === 'question');
    expect(settled).toMatchObject({ outcome: 'answered' });
    const reply = blocks(done.session).filter(b => b.type === 'text').map(b => (b.type === 'text' ? b.markdown : '')).join('');
    expect(reply).toContain('"outcome":"accepted"');
    expect(reply).toContain('notes');

    s.view('V', { type: 'send', sessionId: id, text: 'ask-kimi' });
    const form = await s.hostMsg('V', 'session', m => m.session.turns.length === 4 && blocks(m.session).some(b => b.type === 'question'));
    const fq = blocks(form.session).find(b => b.type === 'question');
    if (fq?.type !== 'question') throw new Error('no form question');
    expect(fq.questions.map(x => x.kind)).toEqual(['single', 'multiple']);
    s.view('V', { type: 'answer', sessionId: id, blockId: fq.id, answers: {}, skip: true });
    const skipped = await s.hostMsg('V', 'session', idle(4));
    expect(blocks(skipped.session).find(b => b.type === 'question')).toMatchObject({ outcome: 'skipped' });
  });

  it('an agent image is stored as a content-named session blob', async () => {
    const at = dirsFor();
    const s = shell(at);
    const id = await started(s);
    s.view('V', { type: 'send', sessionId: id, text: 'image' });
    const done = await s.hostMsg('V', 'session', idle(2));
    const img = blocks(done.session).find(b => b.type === 'image');
    if (img?.type !== 'image') throw new Error('no image block');
    expect(img.blob).toMatch(/^[0-9a-f]+\.png$/);
    await expect.poll(() => existsSync(join(at.home, 'sessions', id, img.blob!))).toBe(true);
  });

  it('/compact runs the agent command and records a completed compaction', async () => {
    const s = shell();
    const id = await started(s);
    s.view('V', { type: 'send', sessionId: id, text: 'hi' });
    await s.hostMsg('V', 'session', m => idle(2)(m) && m.session.commands.some(c => c.name === 'compact'));
    s.view('V', { type: 'compact', sessionId: id });
    const done = await s.hostMsg('V', 'session', m => idle(4)(m));
    expect(done.session.turns[2]).toMatchObject({ role: 'user', text: '/compact' });
    expect(blocks(done.session).find(b => b.type === 'compaction')).toMatchObject({ status: 'completed' });
  });

  it('a failed turn is retried in place; an edit replaces the message and marks it edited', async () => {
    const s = shell();
    const id = await started(s);
    s.view('V', { type: 'send', sessionId: id, text: 'fail' });
    const failed = await s.hostMsg('V', 'session', m => idle(2)(m) && lastAgent(m.session)?.error !== undefined);
    expect(lastAgent(failed.session)?.stop).toBe('error');
    s.view('V', { type: 'retryTurn', sessionId: id });
    const retried = await s.hostMsg('V', 'session', m => idle(2)(m) && lastAgent(m.session)?.error === undefined && lastAgent(m.session)?.stop === 'end_turn');
    const user = retried.session.turns[0] as Extract<Turn, { role: 'user' }>;
    expect(user.text).toBe('fail');

    s.view('V', { type: 'editTurn', requestId: 'e1', edit: {
      sessionId: id, turnIndex: 0, turnCount: 2, originalText: 'fail', turnId: user.id, text: 'hello again',
      retainedAttachments: [], attachments: [], settings: user.settings ?? { config: {} },
    } });
    const result = await s.hostMsg('V', 'editTurnResult', m => m.requestId === 'e1');
    expect(result.error).toBeUndefined();
    const edited = await s.hostMsg('V', 'session', m => idle(2)(m) && m.session.turns[0]?.role === 'user' && m.session.turns[0].text === 'hello again');
    expect(edited.session.turns[0]).toMatchObject({ edited: true });
    expect(lastAgent(edited.session)?.stop).toBe('end_turn');
  });

  it('list operations: rename, pin, soft delete with undo, and fork from an agent turn', async () => {
    const s = shell();
    const id = await started(s);
    s.view('V', { type: 'send', sessionId: id, text: 'hi' });
    await s.hostMsg('V', 'session', idle(2));

    s.view('V', { type: 'renameSession', id, title: '  Renamed  ' });
    await s.hostMsg('V', 'sessions', m => m.sessions.some(x => x.id === id && x.title === 'Renamed'));
    s.view('V', { type: 'pinSession', id, pinned: true });
    await s.hostMsg('V', 'sessions', m => m.sessions.some(x => x.id === id && x.pinned === true));

    s.view('V', { type: 'forkSession', sessionId: id, turnIndex: 1 });
    const fork = await s.hostMsg('V', 'session', m => m.session.id !== id && m.session.turns.length === 2);
    const forkId = fork.session.id;
    expect(fork.session.turns[0]).toMatchObject({ role: 'user', text: 'hi' });
    expect(fork.session.title).not.toBe('Renamed');
    await s.hostMsg('V', 'sessions', m => m.sessions.some(x => x.id === forkId) && m.sessions.some(x => x.id === id));

    s.view('V', { type: 'deleteSession', id });
    await s.hostMsg('V', 'sessions', m => !m.sessions.some(x => x.id === id));
    s.view('V', { type: 'restoreSession', id });
    await s.hostMsg('V', 'sessions', m => m.sessions.some(x => x.id === id && x.title === 'Renamed'));
  });

  it('a record written by one sidecar reopens in the next one with its transcript', async () => {
    const at = dirsFor();
    // A native store on disk, so the second process can resume the session the first one created
    const agentOver = { env: { FAKE_SESSION_DIR: join(at.home, 'native') } };
    mkdirSync(agentOver.env.FAKE_SESSION_DIR);
    const first = shell(at);
    const id = await started(first, agentOver);
    first.view('V', { type: 'send', sessionId: id, text: 'hi' });
    await first.hostMsg('V', 'session', idle(2));
    await first.kill();

    const second = shell(at);
    await second.hello({ client: { name: 'contract', version: '0', capabilities: [] } }, agentOver);
    const init = await second.open('V', 'sidebar', { mostRecent: true });
    expect(init.state.active?.id).toBe(id);
    expect(init.state.active?.turns).toHaveLength(2);
    expect(init.state.sessions.find(x => x.id === id)?.title).toBe('Fake title');
    const ready = await second.hostMsg('V', 'session', m => m.session.id === id && m.session.status === 'ready');
    expect(ready.session.turns[1]).toMatchObject({ role: 'agent', stop: 'end_turn' });
  });
});
