import { describe, expect, it } from 'vitest';
import type { SessionCategory, SessionSummary } from '../src/shared/transcript';
import { buildSessionTree, canFile, categoryOf, draggable } from '../src/webview/chat/sessionTree';

const WS = '/work/acpira';
const OTHER = '/work/bot';
const at = (h: number) => new Date(Date.UTC(2026, 9, 6, 12 - h)).toISOString();
const s = (id: string, extra: Partial<SessionSummary> = {}): SessionSummary => ({ id, title: id, agent: 'claude', cwd: WS, updatedAt: at(1), ...extra });
const categories: SessionCategory[] = [
  { id: 'ui', name: 'UI', cwd: WS },
  { id: 'ssh', name: 'SSH', cwd: WS, collapsed: true },
  { id: 'rel', name: 'Release', cwd: OTHER },
];
const base = { categories, collapsedProjects: [], workspace: WS, filtering: false };

describe('session tree', () => {
  it('places a session pinned, in a category of its own project, or loose', () => {
    const shown = [
      s('a', { category: 'ui' }),
      s('b', { pinned: true, category: 'ui' }),
      // Another project's category, or one that no longer exists, reads as unfiled
      s('c', { category: 'rel' }),
      s('d', { category: 'gone' }),
      s('e'),
    ];
    const tree = buildSessionTree({ ...base, shown, grouped: false });
    expect(tree.pinned.map(x => x.id)).toEqual(['b']);
    expect(tree.projects).toHaveLength(1);
    const p = tree.projects[0]!;
    expect(p.categories.map(n => [n.category.id, n.sessions.map(x => x.id), n.open])).toEqual([['ui', ['a'], true], ['ssh', [], false]]);
    expect(p.loose.map(x => x.id)).toEqual(['c', 'd', 'e']);
    expect(categoryOf(shown[1]!, categories)).toBeUndefined();
    expect(categoryOf(shown[2]!, categories)).toBeUndefined();
  });

  it('groups by project under "all": the current one first, the rest by latest activity, category-only projects included', () => {
    const shown = [s('a', { cwd: '/work/old', updatedAt: at(9) }), s('b', { cwd: '/work/new', updatedAt: at(2) }), s('c', { updatedAt: at(5) })];
    const tree = buildSessionTree({ ...base, shown, grouped: true, collapsedProjects: ['/work/old'] });
    expect(tree.projects.map(p => [p.cwd, p.current, p.open, p.count])).toEqual([
      [WS, true, true, 1],
      ['/work/new', false, true, 1],
      ['/work/old', false, false, 1],
      // Only a category lives here: still listed so the category can be reached
      [OTHER, false, true, 0],
    ]);
  });

  it('filtering hides empty categories and empty projects and opens everything else', () => {
    const shown = [s('a', { category: 'ssh' })];
    const tree = buildSessionTree({ ...base, shown, grouped: true, filtering: true, collapsedProjects: [WS] });
    expect(tree.projects.map(p => p.cwd)).toEqual([WS]);
    expect(tree.projects[0]!.open).toBe(true);
    expect(tree.projects[0]!.categories.map(n => [n.category.id, n.open])).toEqual([['ssh', true]]);
  });

  it('drags only unpinned local sessions, and only into their own project', () => {
    expect(draggable(s('a'))).toBe(true);
    expect(draggable(s('a', { pinned: true }))).toBe(false);
    expect(draggable(s('a', { external: true }))).toBe(false);
    expect(canFile(s('a'), { cwd: WS })).toBe(true);
    expect(canFile(s('a'), { cwd: OTHER })).toBe(false);
  });
});
