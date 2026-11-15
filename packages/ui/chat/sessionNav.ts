import { inWorkspace, type SessionScope } from '@acpira/shared/settings';
import type { SessionCategories, SessionSection, SessionSummary } from '@acpira/shared/transcript';
import { getLocale } from '../i18n';
import { webviewState } from '../vscodeApi';
import { buildSessionTree, isChatFolder, projectOf as projectOfMeta, projectSessions, type ProjectNode } from './sessionTree';
import { filterCount, groupSessions, passes, readView, sortSessions, VIEW_KEY } from './sessionView';

export const needsAttention = (s: SessionSummary): boolean =>
  !!s.unread || s.state === 'unread' || s.state === 'waiting' || s.state === 'error';

export interface SessionNavInput {
  sessions: SessionSummary[];
  categories?: SessionCategories;
  activeId?: string;
  workspace?: string;
  scope?: SessionScope;
  desktop?: boolean;
  chats?: string;
}

// Sessions in the top-to-bottom visual order of SessionList (pinned, sections, projects with their categories, then chats),
// respecting scope, archive state and the user's saved list view
export function orderedSessions({ sessions, categories: categoryState, activeId, workspace, scope = 'workspace', desktop = false, chats }: SessionNavInput): SessionSummary[] {
  const grouped = desktop || (scope === 'all' && !!workspace);
  const view = readView(webviewState(VIEW_KEY));
  const filtering = filterCount(view) > 0;
  const meta = desktop ? categoryState?.projectMeta : undefined;
  const projectOf = (cwd: string) => projectOfMeta(meta, cwd);
  const registered = desktop ? categoryState?.projects ?? [] : undefined;
  const removed = desktop ? categoryState?.removedProjects : undefined;
  const here = (s: SessionSummary) => !!workspace && (inWorkspace(s, workspace) || projectOf(s.cwd) === workspace);
  const inScope = (s: SessionSummary) => desktop || scope === 'all' || here(s) || s.id === activeId;
  const listed = desktop ? projectSessions(sessions, { registered, removed, filtering, expand: false, projectOf }) : sessions;
  const at = new Date();
  const shown = sortSessions(listed.filter(s => inScope(s) && passes(s, view, at)), view.order, getLocale());

  const group = !grouped && view.group === 'project' ? 'none' : view.group;
  if (group !== 'project') {
    const pinned = shown.filter(s => s.pinned);
    const unpinned = group === 'none'
      ? shown.filter(s => !s.pinned)
      : groupSessions(shown.filter(s => !s.pinned), group, view.order, at).flatMap(g => g.sessions);
    return [...pinned, ...unpinned];
  }

  const pinInProject = grouped && view.pinned === 'project';
  const isChat = (cwd: string) => desktop && isChatFolder(chats, cwd);
  const categories = categoryState?.categories ?? [];
  const tree = buildSessionTree({
    shown, categories, collapsedProjects: categoryState?.collapsedProjects ?? [],
    workspace, grouped, filtering, expand: false, registered, removed, pinInProject, isChat, projectOf,
  });

  const projectRows = (p: ProjectNode): SessionSummary[] => [
    ...p.pinned,
    ...p.loose,
    ...p.categories.flatMap(c => c.sessions),
  ];

  const sections: SessionSection[] = desktop ? categoryState?.sections ?? [] : [];
  const sectionOf = (cwd: string) => sections.find(x => x.projects.includes(cwd));
  const sectioned = sections.flatMap(section =>
    section.projects.map(cwd => tree.projects.find(p => p.cwd === cwd)).filter((p): p is ProjectNode => !!p).flatMap(projectRows),
  );
  const unsectioned = (sections.length ? tree.projects.filter(p => !sectionOf(p.cwd)) : tree.projects).flatMap(projectRows);
  return [...tree.pinned, ...sectioned, ...unsectioned, ...tree.chats];
}

// Steps forward (+1) or backward (-1) around `list` from `activeId`
export function stepSession(list: readonly SessionSummary[], activeId: string | undefined, delta: 1 | -1): string | undefined {
  if (!list.length) return undefined;
  const at = activeId ? list.findIndex(s => s.id === activeId) : -1;
  if (at < 0) return delta > 0 ? list[0]!.id : list[list.length - 1]!.id;
  if (list.length === 1) return undefined;
  return list[(at + delta + list.length) % list.length]!.id;
}

// Next session after `activeId` that needs the user: open permission / question cards (`waiting`) take priority over
// errored or unread finished turns, cycling in list order on repeated presses (Codex ⌘⌥A)
export function nextAttentionSession(list: readonly SessionSummary[], activeId: string | undefined): string | undefined {
  if (!list.length) return undefined;
  const at = activeId ? list.findIndex(s => s.id === activeId) : -1;
  const pick = (match: (s: SessionSummary) => boolean): string | undefined => {
    for (let step = 1; step <= list.length; step++) {
      const candidate = list[(at + step + list.length) % list.length]!;
      if (candidate.id !== activeId && match(candidate)) return candidate.id;
    }
    return undefined;
  };
  return pick(s => s.state === 'waiting') ?? pick(needsAttention);
}
