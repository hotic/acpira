import { describe, expect, it } from 'vitest';
import type { SessionSummary } from '@acpira/shared/transcript';
import { clearFilters, DEFAULT_VIEW, filterCount, groupSessions, passes, readView, sortSessions, type ListView } from '../chat/sessionView';

const now = new Date(2026, 9, 23, 15, 0);
const ago = (days: number, hours = 0) => new Date(now.getTime() - (days * 24 + hours) * 3_600_000).toISOString();
const s = (id: string, patch: Partial<SessionSummary> = {}): SessionSummary => ({ id, title: id, agent: 'claude', cwd: '/w', updatedAt: ago(0, 1), ...patch });
const view = (patch: Partial<ListView>): ListView => ({ ...DEFAULT_VIEW, ...patch });

describe('session list view', () => {
  it('reads a stored view value by value, falling back on anything it does not know', () => {
    expect(readView(undefined)).toEqual(DEFAULT_VIEW);
    expect(readView({ group: 'time', order: 'nope', per: 10, model: 'yes', pinned: 'project', status: ['unread', 'bogus', 'unread'], agents: ['codex', 3], since: 'week' }))
      .toEqual({ ...DEFAULT_VIEW, group: 'time', per: 10, pinned: 'project', status: ['unread'], agents: ['codex'], since: 'week' });
  });

  it('keeps sessions that match any pick of a kind and every kind picked', () => {
    const list = [
      s('run', { state: 'working' }),
      s('bg', { state: 'background', unread: true }),
      s('done', { state: 'unread', agent: 'codex', model: { id: 'gpt-6-astra', name: 'GPT-6 Astra' } }),
      s('old', { updatedAt: ago(10) }),
      s('shelf', { state: 'unread', archived: true }),
    ];
    const kept = (v: ListView) => list.filter(x => passes(x, v, now)).map(x => x.id);
    // Archived sessions stay out of every view until "archived" is picked, which is one more alternative of its kind
    expect(kept(DEFAULT_VIEW)).toEqual(['run', 'bg', 'done', 'old']);
    expect(kept(view({ status: ['archived'] }))).toEqual(['shelf']);
    expect(kept(view({ status: ['archived', 'working'] }))).toEqual(['run', 'shelf']);
    // "Unread" takes the background session with a reply nobody looked at too
    expect(kept(view({ status: ['unread'] }))).toEqual(['bg', 'done']);
    expect(kept(view({ status: ['working', 'unread'], agents: ['codex'] }))).toEqual(['done']);
    expect(kept(view({ since: 'week' }))).toEqual(['run', 'bg', 'done']);
    expect(kept(view({ since: 'month' }))).toEqual(['run', 'bg', 'done', 'old']);
    const busy = view({ status: ['working'], agents: ['codex', 'claude'], since: 'today' });
    expect(filterCount(busy)).toBe(4);
    expect(clearFilters(busy)).toEqual(DEFAULT_VIEW);
  });

  it('orders by activity, by start (falling back to activity on old index entries) or by title', () => {
    const list = [s('b', { title: 'beta 10', updatedAt: ago(1), createdAt: ago(1) }), s('a', { title: 'Alpha', updatedAt: ago(0, 2), createdAt: ago(5) }), s('c', { title: 'beta 9', updatedAt: ago(2) })];
    const ids = (order: ListView['order']) => sortSessions(list, order, 'en').map(x => x.id);
    expect(ids('updated')).toEqual(['a', 'b', 'c']);
    expect(ids('created')).toEqual(['b', 'c', 'a']);
    expect(ids('title')).toEqual(['a', 'c', 'b']);
  });

  it('groups by day buckets or by agent, leaving empty groups out', () => {
    const list = [s('t', { updatedAt: ago(0, 1) }), s('y', { updatedAt: ago(1) }), s('w', { updatedAt: ago(4), agent: 'codex' }), s('o', { updatedAt: ago(60) })];
    expect(groupSessions(list, 'time', 'updated', now).map(g => [g.key, g.sessions.map(x => x.id)])).toEqual([['today', ['t']], ['yesterday', ['y']], ['week', ['w']], ['older', ['o']]]);
    expect(groupSessions(list, 'agent', 'updated', now).map(g => [g.key, g.sessions.length])).toEqual([['claude', 3], ['codex', 1]]);
  });
});
