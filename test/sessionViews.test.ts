import { describe, expect, it } from 'vitest';
import type { SessionView, Turn } from '@acpira/shared/transcript';
import type { SessionPatch } from '@acpira/shared/sessionPatch';
import { SessionViews, VIEW_CACHE_ENTRIES } from '../packages/ui/sessionViews';

const user = (id: string): Turn => ({ role: 'user', id, text: id });
function view(id: string, rev: number, turns: Turn[]): SessionView {
  return { id, agent: 'fake', title: 't', cwd: '/w', status: 'ready', running: false, rev, createdAt: 'a', updatedAt: 'b', commands: [], controls: { modes: [], options: [] }, turns };
}
const ids = (v: SessionView) => v.turns.map(t => t.role === 'user' ? t.id : '');
// A patch that keeps the base's turns and appends `turns`
const patch = (id: string, base: number, rev: number, keep: number, turns: Turn[]): SessionPatch => ({ id, base, view: view(id, rev, []), keep, turns });

describe('page session view cache', () => {
  it('switching back to a kept session applies the patch against its kept view', () => {
    const v = new SessionViews();
    v.whole(view('a', 3, [user('a1')]));
    v.whole(view('b', 1, [user('b1')]));
    const back = v.patch(patch('a', 3, 4, 1, [user('a2')]));
    expect(back).not.toBe('stale');
    expect(ids(back as SessionView)).toEqual(['a1', 'a2']);
    expect(v.current?.id).toBe('a');
    expect(v.kept()).toEqual(['b', 'a']);
  });

  it('a whole view of another session is taken even at a lower rev than its kept copy', () => {
    const v = new SessionViews();
    v.whole(view('a', 9, [user('old')]));
    v.whole(view('b', 1, []));
    // a reopened instance counts its revs from 0 again
    const next = v.whole(view('a', 0, [user('new')]));
    expect(next.rev).toBe(0);
    expect(ids(next)).toEqual(['new']);
  });

  it('a patch against a view the page does not hold drops the kept copy and asks for the whole view', () => {
    const v = new SessionViews();
    v.whole(view('a', 3, [user('a1')]));
    v.whole(view('b', 1, []));
    expect(v.patch(patch('a', 5, 6, 1, [user('a2')]))).toBeUndefined();
    expect(v.kept()).toEqual(['b']);
    expect(v.current?.id).toBe('b');
    expect(v.patch(patch('c', 0, 1, 0, []))).toBeUndefined();
  });

  it('a patch older than the view on screen is stale', () => {
    const v = new SessionViews();
    v.whole(view('a', 5, [user('a1')]));
    expect(v.patch(patch('a', 3, 4, 1, [user('x')]))).toBe('stale');
    expect(v.current?.rev).toBe(5);
  });

  it('keeps the most recently shown sessions within its bound', () => {
    const v = new SessionViews();
    for (let i = 0; i <= VIEW_CACHE_ENTRIES; i++) v.whole(view(`s${i}`, 1, []));
    expect(v.kept()).toHaveLength(VIEW_CACHE_ENTRIES);
    expect(v.kept()[0]).toBe('s1');
    expect(v.current?.id).toBe(`s${VIEW_CACHE_ENTRIES}`);
  });

  it('init starts the cache over with the active view', () => {
    const v = new SessionViews();
    v.whole(view('a', 1, []));
    v.whole(view('b', 1, []));
    expect(v.init(view('c', 0, []))?.id).toBe('c');
    expect(v.kept()).toEqual(['c']);
    expect(v.init(undefined)).toBeUndefined();
    expect(v.current).toBeUndefined();
  });

  it('optimistically selects a kept session and absorbs an in-flight patch of the previous session in the background', () => {
    const v = new SessionViews();
    const a1 = user('a1');
    const b1 = user('b1');
    v.whole(view('a', 3, [a1]));
    v.whole(view('b', 1, [b1]));

    // Optimistic switch back to 'a' returns its kept view at once.
    const selected = v.select('a');
    expect(selected?.id).toBe('a');
    expect(v.current?.id).toBe('a');

    // A late streaming patch for 'b' (the session just left) updates 'b' in the cache without flipping the screen back.
    expect(v.patch(patch('b', 1, 2, 1, [user('b2')]))).toBe('stale');
    expect(v.current?.id).toBe('a');

    // When the host's patch for 'a' lands, kept turns preserve their object references.
    const confirmed = v.patch(patch('a', 3, 3, 1, [])) as SessionView;
    expect(confirmed.id).toBe('a');
    expect(confirmed.turns[0]).toBe(a1);

    // Switching back to 'b' later sees the background patch that was absorbed.
    const backToB = v.select('b');
    expect(backToB && ids(backToB)).toEqual(['b1', 'b2']);
  });
});
