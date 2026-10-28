import { describe, expect, it } from 'vitest';
import type { SessionCategory, SessionSummary } from '@acpira/shared/transcript';
import { buildSessionTree, categoryOf, draggable, dropAction, isChatFolder } from '../chat/sessionTree';

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

  it('a view filter hides empty groups but keeps the folds; only a search opens every group', () => {
    const shown = [s('a', { category: 'ssh' }), s('b', { cwd: '/work/old' })];
    const filtered = buildSessionTree({ ...base, shown, grouped: true, filtering: true, expand: false, collapsedProjects: ['/work/old'] });
    expect(filtered.projects.map(p => [p.cwd, p.open, p.categories.map(n => [n.category.id, n.open])])).toEqual([
      [WS, true, [['ssh', false]]],
      ['/work/old', false, []],
    ]);
    const searched = buildSessionTree({ ...base, shown, grouped: true, filtering: true, collapsedProjects: ['/work/old'] });
    expect(searched.projects.map(p => [p.open, p.categories.map(n => n.open)])).toEqual([[true, [true]], [true, []]]);
  });

  it('pinned into projects: each pinned session leads its own project, a project holding only pins still shows', () => {
    const shown = [s('a', { pinned: true, category: 'ui' }), s('b'), s('c', { pinned: true, cwd: '/work/pins' })];
    const tree = buildSessionTree({ ...base, shown, grouped: true, pinInProject: true });
    expect(tree.pinned).toEqual([]);
    expect(tree.projects.map(p => [p.cwd, p.pinned.map(x => x.id), p.loose.map(x => x.id), p.count])).toEqual([
      [WS, ['a'], ['b'], 2],
      ['/work/pins', ['c'], [], 1],
      [OTHER, [], [], 0],
    ]);
    // Without project groups there is nothing to pin into: the section on top stays
    expect(buildSessionTree({ ...base, shown, grouped: false, pinInProject: true }).pinned.map(x => x.id)).toEqual(['a', 'c']);
  });

  it('a project shell lists its registered folders first, in their order and empty ones included, then the rest', () => {
    const shown = [s('a', { cwd: '/work/old', updatedAt: at(9) }), s('b', { cwd: '/work/new', updatedAt: at(2) }), s('c', { updatedAt: at(5) })];
    const tree = buildSessionTree({ ...base, categories: [], workspace: undefined, shown, grouped: true, registered: ['/work/old', '/work/empty', WS] });
    expect(tree.projects.map(p => [p.cwd, p.count])).toEqual([['/work/old', 1], ['/work/empty', 0], [WS, 1], ['/work/new', 1]]);
    // A search shows only the folders it matched, registered or not
    const found = buildSessionTree({ ...base, categories: [], workspace: undefined, shown: [shown[1]!], grouped: true, filtering: true, registered: ['/work/old', '/work/empty'] });
    expect(found.projects.map(p => p.cwd)).toEqual(['/work/new']);
  });

  it('a folder taken off the project list leaves it with its sessions, which a search still finds', () => {
    const shown = [s('a', { cwd: '/work/gone', updatedAt: at(9) }), s('b', { cwd: '/work/gone', pinned: true }), s('c', { updatedAt: at(5) })];
    const input = { ...base, categories: [], workspace: undefined, shown, grouped: true, registered: [WS], removed: ['/work/gone'] };
    const tree = buildSessionTree(input);
    expect(tree.projects.map(p => p.cwd)).toEqual([WS]);
    // A pinned session stays pinned
    expect(tree.pinned.map(x => x.id)).toEqual(['b']);
    expect(buildSessionTree({ ...input, filtering: true }).projects.map(p => p.cwd)).toEqual([WS, '/work/gone']);
  });

  it('a project shell groups only the host folders it added; the rest of a host list shows in a search', () => {
    const added = 'ssh://gen1/home/u/app';
    const other = 'ssh://gen1/home/u/scratch';
    const shown = [s('a', { cwd: added, updatedAt: at(9) }), s('b', { cwd: other, updatedAt: at(8) }), s('c', { cwd: '/work/local', updatedAt: at(7) })];
    const input = { ...base, categories: [], workspace: undefined, shown, grouped: true, registered: [WS, added] };
    // A local folder a session ran in still joins the list on its own
    expect(buildSessionTree(input).projects.map(p => p.cwd)).toEqual([WS, added, '/work/local']);
    expect(buildSessionTree({ ...input, filtering: true }).projects.map(p => p.cwd)).toEqual([added, '/work/local', other]);
    // A view filter is no search: the folders nobody added stay out
    expect(buildSessionTree({ ...input, filtering: true, expand: false }).projects.map(p => p.cwd)).toEqual([added, '/work/local']);
    // An IDE list registers nothing and keeps every folder
    expect(buildSessionTree({ ...input, registered: undefined }).projects.map(p => p.cwd)).toEqual(['/work/local', other, added]);
  });

  it('filtering hides empty categories and empty projects and opens everything else', () => {
    const shown = [s('a', { category: 'ssh' })];
    const tree = buildSessionTree({ ...base, shown, grouped: true, filtering: true, collapsedProjects: [WS] });
    expect(tree.projects.map(p => p.cwd)).toEqual([WS]);
    expect(tree.projects[0]!.open).toBe(true);
    expect(tree.projects[0]!.categories.map(n => [n.category.id, n.open])).toEqual([['ssh', true]]);
  });

  it('gathers chats, whose folders sit directly in the chats root, outside every project', () => {
    const root = '/home/u/.acpira/chats';
    const isChat = (cwd: string) => isChatFolder(root, cwd);
    expect([`${root}/2026-10-28-a1`, 'C:\\h\\chats\\x', `${root}/a/b`, root, `${root}/`, '/elsewhere/a1'].map(c => isChatFolder(c.startsWith('C:') ? 'C:\\h\\chats' : root, c)))
      .toEqual([true, true, false, false, false, false]);
    const shown = [s('a', { cwd: `${root}/one` }), s('b'), s('c', { cwd: `${root}/two`, pinned: true }), s('d', { cwd: `${root}/three` })];
    const tree = buildSessionTree({ ...base, shown, grouped: true, isChat, registered: [WS] });
    expect(tree.chats.map(x => x.id)).toEqual(['a', 'd']);
    expect(tree.pinned.map(x => x.id)).toEqual(['c']);
    // No chat folder heads a group (the other project shows for its category)
    expect(tree.projects.map(p => p.cwd)).toEqual([WS, OTHER]);
    // Pinned into projects, a pinned chat leads the chats
    expect(buildSessionTree({ ...base, shown, grouped: true, isChat, pinInProject: true }).chats.map(x => x.id)).toEqual(['c', 'a', 'd']);
    // One headerless project has no chats of its own
    expect(buildSessionTree({ ...base, shown, grouped: false, isChat }).chats).toEqual([]);
  });

  it('drags only unpinned local sessions', () => {
    expect(draggable(s('a'))).toBe(true);
    expect(draggable(s('a', { pinned: true }))).toBe(false);
    expect(draggable(s('a', { external: true }))).toBe(false);
  });

  it('files a dropped session within its own project and moves it into another', () => {
    // Own project: into a category, out of one, or nothing to do
    expect(dropAction(s('a'), { cwd: WS, category: 'ui' }, categories)).toEqual({ kind: 'file', category: 'ui' });
    expect(dropAction(s('a', { category: 'ui' }), { cwd: WS, category: 'ui' }, categories)).toBeUndefined();
    expect(dropAction(s('a', { category: 'ui' }), { cwd: WS }, categories)).toEqual({ kind: 'file', category: null });
    expect(dropAction(s('a'), { cwd: WS }, categories)).toBeUndefined();
    // A dangling category id reads as unfiled, so the loose area has nothing to undo
    expect(dropAction(s('a', { category: 'gone' }), { cwd: WS }, categories)).toBeUndefined();
    // Another project: its group moves the session there, one of its categories files it there too
    expect(dropAction(s('a', { category: 'ui' }), { cwd: OTHER }, categories)).toEqual({ kind: 'move', to: { cwd: OTHER } });
    expect(dropAction(s('a'), { cwd: OTHER, category: 'rel' }, categories)).toEqual({ kind: 'move', to: { cwd: OTHER, category: 'rel' } });
  });
});
