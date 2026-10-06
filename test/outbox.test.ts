import { describe, expect, it } from 'vitest';
import type { HostMsg } from '../src/shared/protocol';
import type { SessionView, Turn } from '../src/shared/transcript';
import type { SessionPatch } from '../src/shared/sessionPatch';
import { PostQueue, enqueue } from '../src/host/outbox';

const user = (id: string): Turn => ({ role: 'user', id, text: id });
function view(rev: number, turns: Turn[], id = 's'): SessionView {
  return { id, agent: 'fake', title: 't', cwd: '/w', status: 'ready', running: true, rev, createdAt: 'a', updatedAt: 'b', commands: [], controls: { modes: [], options: [] }, turns };
}
const session = (rev: number, turns: Turn[] = [user('u1')], id = 's'): HostMsg => ({ type: 'session', session: view(rev, turns, id) });
const patch = (base: number, keep: number, turns: Turn[], id = 's'): HostMsg => ({ type: 'sessionPatch', patch: { id, base, view: view(base + 1, [], id), keep, turns } satisfies SessionPatch });
const list = (n: number): HostMsg => ({ type: 'sessions', sessions: Array.from({ length: n }, (_, i) => ({ id: `x${i}`, title: '', agent: 'fake', cwd: '/w', updatedAt: '' })) });
const types = (q: HostMsg[]) => q.map(m => m.type);

describe('webview post queue', () => {
  it('a whole view replaces the session pushes and list copies waiting since the last barrier', () => {
    const q: HostMsg[] = [];
    enqueue(q, session(1));
    enqueue(q, { type: 'editTurnResult', requestId: 'r' });
    enqueue(q, session(2));
    enqueue(q, list(1));
    enqueue(q, patch(2, 1, [user('u2')]));
    enqueue(q, list(2));
    enqueue(q, session(9));
    // rev 1 stays ahead of the barrier; after it only the newest state of each kind is left
    expect(types(q)).toEqual(['session', 'editTurnResult', 'sessions', 'session']);
    expect((q[0] as Extract<HostMsg, { type: 'session' }>).session.rev).toBe(1);
    expect((q[2] as Extract<HostMsg, { type: 'sessions' }>).sessions).toHaveLength(2);
    expect((q[3] as Extract<HostMsg, { type: 'session' }>).session.rev).toBe(9);
  });

  it('a whole view of another session drops the views switched away from but keeps patches the page can apply', () => {
    const q: HostMsg[] = [];
    // a: a patch against the copy the page keeps; b: a whole view and a patch built on it; then the page goes to c
    enqueue(q, patch(4, 1, [user('a2')], 'a'));
    enqueue(q, session(1, [user('b1')], 'b'));
    enqueue(q, patch(1, 1, [user('b2')], 'b'));
    enqueue(q, session(1, [user('c1')], 'c'));
    expect(q.map(m => m.type === 'session' ? `session:${m.session.id}` : m.type === 'sessionPatch' ? `patch:${m.patch.id}` : m.type))
      .toEqual(['patch:a', 'session:c']);
  });

  it('a patch folds into the whole view or the patch queued before it', () => {
    const q: HostMsg[] = [session(1)];
    enqueue(q, patch(1, 1, [user('u2')]));
    expect(types(q)).toEqual(['session']);
    expect((q[0] as Extract<HostMsg, { type: 'session' }>).session).toMatchObject({ rev: 2, turns: [user('u1'), user('u2')] });

    const p: HostMsg[] = [patch(4, 1, [user('a')])];
    enqueue(p, list(1));
    enqueue(p, patch(5, 2, [user('b')]));
    expect(types(p)).toEqual(['sessionPatch', 'sessions']);
    expect((p[0] as Extract<HostMsg, { type: 'sessionPatch' }>).patch).toMatchObject({ base: 4, keep: 1, turns: [user('a'), user('b')] });
    // one that does not chain waits behind
    enqueue(p, patch(9, 0, []));
    expect(types(p)).toEqual(['sessionPatch', 'sessions', 'sessionPatch']);
  });

  it('posts one message at a time and collapses what arrives meanwhile', async () => {
    const posted: HostMsg[] = [];
    const acks: (() => void)[] = [];
    const queue = new PostQueue(m => { posted.push(m); return new Promise<void>(r => acks.push(r)); });
    queue.push(session(1));
    await Promise.resolve();
    for (let rev = 2; rev <= 20; rev++) queue.push(session(rev));
    expect(posted).toHaveLength(1);
    expect(queue.pending).toBe(1);
    acks.shift()!();
    await new Promise(r => setTimeout(r, 0));
    expect(posted.map(m => (m as Extract<HostMsg, { type: 'session' }>).session.rev)).toEqual([1, 20]);
  });

  it('a post that is never acknowledged does not stall the rest', async () => {
    const posted: HostMsg[] = [];
    const queue = new PostQueue(m => { posted.push(m); return new Promise<void>(() => {}); }, () => {}, 20);
    queue.push(session(1));
    queue.push({ type: 'editTurnResult', requestId: 'r' });
    await new Promise(r => setTimeout(r, 80));
    expect(types(posted)).toEqual(['session', 'editTurnResult']);
  });

  it('a reloading page is not held up by a post still in flight for the old one', async () => {
    const posted: HostMsg[] = [];
    const queue = new PostQueue(m => { posted.push(m); return new Promise<void>(() => {}); });
    queue.push(session(1));
    await Promise.resolve();
    queue.push(session(2));
    queue.clear();
    queue.push({ type: 'editTurnResult', requestId: 'init' });
    await Promise.resolve();
    expect(types(posted)).toEqual(['session', 'editTurnResult']);
  });

  it('a message cleared before its post starts is never sent', async () => {
    const posted: HostMsg[] = [];
    const queue = new PostQueue(m => { posted.push(m); return Promise.resolve(true); });
    queue.push(session(1));
    queue.clear();
    queue.push({ type: 'editTurnResult', requestId: 'init' });
    await new Promise(r => setTimeout(r, 0));
    expect(types(posted)).toEqual(['editTurnResult']);
  });

  it('reports a post the webview refused', async () => {
    const results: [string, boolean][] = [];
    const queue = new PostQueue(m => m.type === 'session' ? Promise.resolve(false) : Promise.reject(new Error('gone')), () => {}, undefined, (m, ok) => results.push([m.type, ok]));
    queue.push(session(1));
    queue.push(list(1));
    await new Promise(r => setTimeout(r, 0));
    expect(results).toEqual([['session', false], ['sessions', false]]);
    const fine = new PostQueue(() => Promise.resolve(true), () => {}, undefined, (m, ok) => results.push([m.type, ok]));
    fine.push(session(2));
    await new Promise(r => setTimeout(r, 0));
    expect(results.at(-1)).toEqual(['session', true]);
  });
});
