import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import type { HostMsg } from '../src/shared/protocol';
import type { AgentBlock, AgentTurn, SessionView, Turn } from '../src/shared/transcript';
import { applySessionPatch, mergeSessionPatches, type SessionPatch } from '../src/shared/sessionPatch';
import { applySession } from '../src/shared/reuse';
import { Shell } from './sidecarShell';

const text = (markdown: string): AgentBlock => ({ type: 'text', markdown });
const user = (id: string): Turn => ({ role: 'user', id, text: id });
const agent = (...md: string[]): Turn => ({ role: 'agent', blocks: md.map(text) });

function view(rev: number, turns: Turn[]): SessionView {
  return { id: 's', agent: 'fake', title: 't', cwd: '/w', status: 'ready', running: false, rev, createdAt: 'a', updatedAt: 'b', commands: [], controls: { modes: [], options: [] }, turns };
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

// What the sidecar computes (session_patch.rs), for building patches between arbitrary views in the merge test
function diff(prev: SessionView, next: SessionView): SessionPatch {
  let keep = 0;
  while (keep < prev.turns.length && keep < next.turns.length && same(prev.turns[keep], next.turns[keep])) keep++;
  const head = { ...next, turns: [] };
  const p: SessionPatch = { id: next.id, base: prev.rev!, view: head, keep, turns: next.turns.slice(keep) };
  const a = prev.turns.at(-1), b = next.turns.at(-1);
  if (keep + 1 === next.turns.length && keep + 1 === prev.turns.length && a?.role === 'agent' && b?.role === 'agent') {
    let kb = 0;
    while (kb < a.blocks.length && kb < b.blocks.length && same(a.blocks[kb], b.blocks[kb])) kb++;
    if (kb > 0) return { ...p, turns: [{ ...b, blocks: [] }], keepBlocks: kb, blocks: b.blocks.slice(kb) };
  }
  return p;
}

describe('session patches', () => {
  it('a streamed chunk keeps every earlier turn and block by reference', () => {
    const current = view(1, [user('u1'), agent('a'), user('u2'), agent('x', 'y')]);
    const next = applySessionPatch(current, { id: 's', base: 1, view: view(2, []), keep: 3, turns: [{ role: 'agent', blocks: [] }], keepBlocks: 1, blocks: [text('yz')] })!;
    expect(next.rev).toBe(2);
    expect(next.turns.slice(0, 3).every((t, i) => t === current.turns[i])).toBe(true);
    const last = next.turns[3] as AgentTurn;
    expect(last.blocks[0]).toBe((current.turns[3] as AgentTurn).blocks[0]);
    expect(last.blocks[1]).toEqual(text('yz'));
    // applySession then finds nothing else to rebuild
    expect(applySession(current, next).turns[0]).toBe(current.turns[0]);
  });

  it('a patch against a view the page does not hold does not apply', () => {
    const current = view(5, [user('u1'), agent('a')]);
    const p: SessionPatch = { id: 's', base: 4, view: view(6, []), keep: 1, turns: [agent('b')] };
    expect(applySessionPatch(current, p)).toBeUndefined();
    expect(applySessionPatch(undefined, { ...p, base: 5 })).toBeUndefined();
    expect(applySessionPatch({ ...current, id: 't' }, { ...p, base: 5 })).toBeUndefined();
    expect(applySessionPatch(current, { ...p, base: 5, keep: 3 })).toBeUndefined();
    expect(applySessionPatch(current, { ...p, base: 5, keep: 1, turns: [{ role: 'agent', blocks: [] }], keepBlocks: 4, blocks: [] })).toBeUndefined();
  });

  // Any two chained patches fold into one with the same effect: what the extension's post queue relies on while it is behind
  it('merging two chained patches equals applying them one after the other', () => {
    let seed = 7;
    const rand = (n: number) => { seed = (seed * 1103515245 + 12345) % 2 ** 31; return seed % n; };
    const mutate = (v: SessionView, rev: number): SessionView => {
      const turns = v.turns.slice();
      switch (rand(5)) {
        case 0: turns.push(rand(2) ? user(`u${rev}`) : agent(`n${rev}`)); break;
        case 1: if (turns.length) turns.pop(); break;
        case 2: {
          const last = turns.at(-1);
          if (last?.role === 'agent') turns[turns.length - 1] = { ...last, blocks: [...last.blocks.slice(0, Math.max(0, last.blocks.length - rand(2))), text(`c${rev}`)] };
          else turns.push(agent(`s${rev}`));
          break;
        }
        case 3: if (turns.length) { const i = rand(turns.length); turns[i] = agent(`r${rev}`); } break;
        default: break;
      }
      return view(rev, turns);
    };
    for (let round = 0; round < 400; round++) {
      let v0 = view(1, [user('u1'), agent('a', 'b')]);
      for (let i = rand(4); i > 0; i--) v0 = mutate(v0, 1);
      const v1 = mutate(v0, 2);
      const v2 = mutate(v1, 3);
      const a = diff(v0, v1), b = diff(v1, v2);
      expect(applySessionPatch(v0, a)?.turns).toEqual(v1.turns);
      const merged = mergeSessionPatches(a, b);
      expect(merged, `round ${round}`).toBeDefined();
      expect(applySessionPatch(v0, merged!)?.turns, `round ${round}`).toEqual(v2.turns);
      expect(applySessionPatch(v0, merged!)?.rev).toBe(3);
    }
  });

  it('patches that do not chain are not merged', () => {
    const a: SessionPatch = { id: 's', base: 1, view: view(2, []), keep: 0, turns: [] };
    expect(mergeSessionPatches(a, { ...a, base: 5, view: view(6, []) })).toBeUndefined();
    expect(mergeSessionPatches(a, { ...a, id: 't', base: 2 })).toBeUndefined();
  });
});

// The real sidecar: a page that asks for patches rebuilds exactly the view a page taking whole views is sent
describe('session patches over the sidecar', () => {
  const shells: Shell[] = [];
  const dirs: string[] = [];
  afterEach(async () => {
    for (const s of shells.splice(0)) await s.kill();
    for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
  });

  it('streams into a long transcript as small patches and lands on the same view', async () => {
    const home = mkdtempSync(join(tmpdir(), 'acpira-patch-home-'));
    const cwd = mkdtempSync(join(tmpdir(), 'acpira-patch-ws-'));
    dirs.push(home, cwd);
    const s = new Shell(home, cwd);
    shells.push(s);
    await s.hello();
    s.send({ type: 'attachView', viewId: 'P', host: 'sidebar' });
    s.view('P', { type: 'ready', patches: true });
    const init = await s.hostMsg('P', 'init');
    const id = init.state.active!.id;
    // The page as App.tsx keeps it
    let held: SessionView | undefined = init.state.active;
    let resyncs = 0;
    const sizes: { type: string; bytes: number }[] = [];
    s.onMessage(m => {
      if (m.type !== 'hostMessage' || m.viewId !== 'P') return;
      const msg: HostMsg = m.message;
      if (msg.type === 'session') { held = applySession(held, msg.session); sizes.push({ type: 'session', bytes: JSON.stringify(msg).length }); }
      if (msg.type === 'sessionPatch') {
        sizes.push({ type: 'sessionPatch', bytes: JSON.stringify(msg).length });
        const next = applySessionPatch(held, msg.patch);
        if (next) held = applySession(held, next);
        else { resyncs++; s.view('P', { type: 'resync' }); }
      }
    });
    const idleAt = (turns: number) => () => !!held && held.id === id && !held.running && held.turns.length === turns;
    const until = async (ok: () => boolean) => { for (let i = 0; i < 300 && !ok(); i++) await new Promise(r => setTimeout(r, 50)); expect(ok()).toBe(true); };

    s.view('P', { type: 'send', sessionId: id, text: `say:${'x'.repeat(200_000)}` });
    await until(idleAt(2));
    const before = sizes.length;
    s.view('P', { type: 'send', sessionId: id, text: `say:${Array.from({ length: 40 }, (_, i) => `c${i} `).join('||')}` });
    await until(idleAt(4));
    s.view('P', { type: 'send', sessionId: id, text: 'slow' });
    await until(() => (held?.turns.at(-1) as AgentTurn | undefined)?.blocks.some(b => b.type === 'text' && b.markdown.includes('5 ')) ?? false);
    s.view('P', { type: 'stop', sessionId: id });
    await until(idleAt(6));

    const later = sizes.slice(before);
    // how many pushes the 30 ms batch leaves depends on timing; every one of them is a patch
    expect(later.length).toBeGreaterThan(0);
    expect(later.every(x => x.type === 'sessionPatch')).toBe(true);
    // the 400 KB of history is never sent again while the new turns stream
    expect(Math.max(...later.map(x => x.bytes))).toBeLessThan(50_000);
    expect(resyncs).toBe(0);

    // A page taking whole views on the same session, and a resync, both land where the patches did
    s.send({ type: 'attachView', viewId: 'W', host: 'editor', initial: id });
    s.view('W', { type: 'ready' });
    const whole = (await s.hostMsg('W', 'init')).state.active!;
    expect(s.hostMsgs('W').some(m => m.type === 'sessionPatch')).toBe(false);
    expect({ ...held!, rev: 0 }).toEqual({ ...whole, rev: 0 });
    s.view('P', { type: 'resync' });
    const full = await s.hostMsg('P', 'session', m => m.session.id === id && m.session.rev! >= held!.rev!);
    expect(full.session.turns).toEqual(held!.turns);
  });
});
