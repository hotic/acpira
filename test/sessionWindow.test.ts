import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import type { HostMsg } from '@acpira/shared/protocol';
import type { AgentBlock, AgentTurn, SessionView, Turn } from '@acpira/shared/transcript';
import { applySessionPatch, type SessionPatch } from '@acpira/shared/sessionPatch';
import { applySessionEarlier, sessionTurnIndex, type SessionEarlier } from '@acpira/shared/sessionWindow';
import { Shell } from './sidecarShell';

const text = (markdown: string): AgentBlock => ({ type: 'text', markdown });
const user = (id: string): Turn => ({ role: 'user', id, text: id });
const agent = (...md: string[]): Turn => ({ role: 'agent', blocks: md.map(text) });

function view(turns: Turn[], earlier?: SessionView['earlier']): SessionView {
  return { id: 's', agent: 'fake', title: 't', cwd: '/w', status: 'ready', running: false, rev: 3, createdAt: 'a', updatedAt: 'b', commands: [], controls: { modes: [], options: [] }, turns, ...(earlier ? { earlier } : {}) };
}

describe('windowed session views', () => {
  it('a chunk goes in front of the turns, its blocks in front of the cut turn', () => {
    const current = view([agent('c', 'd'), user('u3')], { turns: 2, blocks: 2 });
    const next = applySessionEarlier(current, { id: 's', from: { turns: 2, blocks: 2 }, blocks: [text('a'), text('b')], turns: [user('u2')], earlier: { turns: 1, blocks: 0 } })!;
    expect(next.turns).toEqual([user('u2'), agent('a', 'b', 'c', 'd'), user('u3')]);
    expect(next.earlier).toEqual({ turns: 1, blocks: 0 });
    expect(next.rev).toBe(3);
    // turns the page already held keep their identity
    expect(next.turns[2]).toBe(current.turns[1]);
    // the last chunk leaves a whole view
    const whole = applySessionEarlier(next, { id: 's', from: { turns: 1, blocks: 0 }, turns: [user('u1')] })!;
    expect('earlier' in whole).toBe(false);
    expect(whole.turns.length).toBe(4);
  });

  it('a chunk cut for another window does not apply', () => {
    const current = view([agent('c'), user('u3')], { turns: 2, blocks: 1 });
    const c: SessionEarlier = { id: 's', from: { turns: 2, blocks: 1 }, blocks: [text('a')], turns: [] };
    expect(applySessionEarlier(undefined, c)).toBeUndefined();
    expect(applySessionEarlier({ ...current, id: 't' }, c)).toBeUndefined();
    expect(applySessionEarlier(current, { ...c, from: { turns: 2, blocks: 0 } })).toBeUndefined();
    expect(applySessionEarlier(view([user('u')]), c)).toBeUndefined();
    // blocks for a first turn that has none
    expect(applySessionEarlier(view([user('u')], { turns: 2, blocks: 1 }), c)).toBeUndefined();
  });

  it('a turn index counts from the first turn of the session', () => {
    expect(sessionTurnIndex(view([user('u')], { turns: 7, blocks: 3 }), 2)).toBe(9);
    expect(sessionTurnIndex(view([user('u')]), 2)).toBe(2);
  });

  // test/fixtures/engine-window.json (rust/crates/acpira-host/tests/engine/golden.rs): what the engine sends a windowed page
  it('replays what the engine sends and holds the recorded view after every step', () => {
    type Step = { view: SessionView; session?: SessionView; patch?: SessionPatch; chunk?: SessionEarlier };
    const fixture = JSON.parse(readFileSync(join(__dirname, 'fixtures/engine-window.json'), 'utf8')) as { first: SessionView; steps: Step[] };
    let held: SessionView | undefined = fixture.first;
    expect(held.earlier).toBeDefined();
    for (const [i, step] of fixture.steps.entries()) {
      if (step.session) held = step.session;
      else if (step.patch) held = applySessionPatch(held, step.patch);
      else if (step.chunk) held = applySessionEarlier(held, step.chunk);
      expect(held, `step ${i}`).toEqual(step.view);
    }
    expect(held!.earlier).toBeUndefined();
  });
});

// The real sidecar: a page that takes windows opens a long session at its tail, follows it by patches and loads the rest
// in bounded chunks, ending on the view a page taking whole views is sent
describe('windowed session views over the sidecar', () => {
  const shells: Shell[] = [];
  const dirs: string[] = [];
  afterEach(async () => {
    for (const s of shells.splice(0)) await s.kill();
    for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
  });

  it('opens a long session at its tail and loads the rest in bounded chunks', async () => {
    const home = mkdtempSync(join(tmpdir(), 'acpira-window-home-'));
    const cwd = mkdtempSync(join(tmpdir(), 'acpira-window-ws-'));
    dirs.push(home, cwd);
    const s = new Shell(home, cwd);
    shells.push(s);
    await s.hello();
    const until = async (ok: () => boolean) => { for (let i = 0; i < 400 && !ok(); i++) await new Promise(r => setTimeout(r, 50)); expect(ok()).toBe(true); };

    // A page taking whole views builds the session: three 300 KB exchanges
    let whole: SessionView | undefined;
    s.onMessage(m => {
      if (m.type !== 'hostMessage' || m.viewId !== 'W') return;
      const msg: HostMsg = m.message;
      if (msg.type === 'init') whole = msg.state.active;
      if (msg.type === 'session') whole = msg.session;
    });
    s.send({ type: 'attachView', viewId: 'W', host: 'sidebar' });
    s.view('W', { type: 'ready' });
    const id = (await s.hostMsg('W', 'init')).state.active!.id;
    const idleAt = (turns: number) => () => !!whole && whole.id === id && !whole.running && whole.turns.length === turns;
    for (let i = 0; i < 3; i++) {
      s.view('W', { type: 'send', sessionId: id, text: `say:${String(i).repeat(300_000)}` });
      await until(idleAt(2 * i + 2));
    }

    // The windowed page, as a phone keeps it
    let held: SessionView | undefined;
    const sizes: { type: string; bytes: number }[] = [];
    let unfit = 0;
    s.onMessage(m => {
      if (m.type !== 'hostMessage' || m.viewId !== 'M') return;
      const msg: HostMsg = m.message;
      if (msg.type !== 'init' && msg.type !== 'session' && msg.type !== 'sessionPatch' && msg.type !== 'sessionEarlier') return;
      sizes.push({ type: msg.type, bytes: JSON.stringify(msg).length });
      if (msg.type === 'init') held = msg.state.active;
      if (msg.type === 'session') held = msg.session;
      if (msg.type === 'sessionPatch') {
        const next = applySessionPatch(held, msg.patch);
        if (next) held = next; else unfit++;
      }
      if (msg.type === 'sessionEarlier') {
        const next = applySessionEarlier(held, msg.chunk);
        if (next) held = next; else unfit++;
      }
    });
    s.send({ type: 'attachView', viewId: 'M', host: 'editor', initial: id });
    s.view('M', { type: 'ready', patches: true, listPatches: true, window: true });
    await s.hostMsg('M', 'init');
    expect(held?.id).toBe(id);
    const opened = held!.earlier!;
    expect(opened.turns).toBeGreaterThan(0);
    const tail = (v: SessionView) => whole!.turns.slice(v.earlier?.turns ?? 0);
    expect(held!.turns).toEqual(tail(held!));

    // A new turn streams into the window as patches, indices counted from where the page's copy starts
    s.view('M', { type: 'send', sessionId: id, text: 'say:fresh||er' });
    await until(idleAt(8));
    await until(() => held?.turns.length === 8 - opened.turns && !held.running);
    expect(held!.earlier).toEqual(opened);
    expect(held!.turns).toEqual(tail(held!));
    expect(sizes.filter(x => x.type === 'session')).toEqual([]);

    // The rest, one chunk per ask, until the view is whole
    let asks = 0;
    while (held!.earlier) {
      const before = held!.earlier;
      s.view('M', { type: 'loadEarlier', sessionId: id });
      await until(() => held!.earlier?.turns !== before.turns || held!.earlier?.blocks !== before.blocks);
      asks++;
    }
    expect(asks).toBeGreaterThan(2);
    expect(held!.turns).toEqual(whole!.turns);
    expect(unfit).toBe(0);
    // Nothing the page was sent grows with the transcript: 1.8 MB of it, never more than one exchange at a time
    expect(Math.max(...sizes.map(x => x.bytes))).toBeLessThan(700_000);

    // Whole now: nothing more to ask for, and the page keeps following
    s.view('M', { type: 'loadEarlier', sessionId: id });
    s.view('M', { type: 'send', sessionId: id, text: 'say:last' });
    await until(idleAt(10));
    await until(() => held?.turns.length === 10 && !held.running);
    expect(held!.turns).toEqual(whole!.turns);
    expect((held!.turns.at(-1) as AgentTurn).blocks.some(b => b.type === 'text' && b.markdown.includes('last'))).toBe(true);
  });
});
