import type { ProjectMeta, SessionCategory, SessionSummary } from '@acpira/shared/transcript';
import { isRemoteFolder } from '@acpira/shared/remote';

// The session list as a tree: pinned sessions on top, then one node per project (the session's cwd), each holding its
// user categories and the sessions filed nowhere ("loose"). Under the workspace scope there is a single node (the
// window's project) and the list shows no project header; under "all" every project is a collapsible group, the
// window's own first. A session sits in exactly one place: pinned, a category of its own project, or loose. Grouped with
// `pinInProject`, pinned sessions leave the top section and lead their own project instead. Grouped with `isChat`, chats
// (sessions in a folder of their own under InitState.chats) belong to no project and gather in `chats`

export interface CategoryNode {
  category: SessionCategory;
  sessions: SessionSummary[];
  open: boolean;
}

export interface ProjectNode {
  cwd: string;
  // The window's own project
  current: boolean;
  open: boolean;
  categories: CategoryNode[];
  loose: SessionSummary[];
  // The project's pinned sessions, listed before everything else (only with `pinInProject`; otherwise empty)
  pinned: SessionSummary[];
  // Every session of the project in view (its pinned ones only when they are listed here)
  count: number;
}

export interface SessionTree {
  pinned: SessionSummary[];
  // Project groups with headers ("all" scope) or the single headerless node
  grouped: boolean;
  projects: ProjectNode[];
  // Grouped with `isChat`: the chats in view, in list order (with `pinInProject` their pinned ones first)
  chats: SessionSummary[];
}

export interface TreeInput {
  // The sessions in view (scope, channel filter and search already applied), in list order
  shown: SessionSummary[];
  categories: SessionCategory[];
  collapsedProjects: string[];
  // The window's project folder; absent (the LAB) every project is foreign
  workspace?: string;
  grouped: boolean;
  // A search or a view filter is on: categories / projects without a match hide
  filtering: boolean;
  // A search is on: every group opens so each match is in sight (filters keep the user's folds). Defaults to `filtering`
  expand?: boolean;
  // A project shell's registered folders (SessionCategories.projects, the desktop app): grouped, they lead in this order,
  // a folder without a session included, and any other folder a session ran in follows by latest activity
  registered?: string[];
  // Folders the project shell took off its list (SessionCategories.removedProjects): their sessions show only in a search,
  // as do those of host folders that are not registered (`offList`)
  removed?: string[];
  // Grouped: pinned sessions sit first in their own project rather than in one section on top
  pinInProject?: boolean;
  // Grouped: whether a folder is a chat's (`isChatFolder`); those sessions leave the project groups for `chats`
  isChat?: (cwd: string) => boolean;
  // The project a session's folder belongs to (`projectOf` over SessionCategories.projectMeta): a project of several
  // folders groups the sessions of each. Absent: every folder is its own project
  projectOf?: (cwd: string) => string;
}

// The project a folder belongs to: the one whose primary folder it is or that lists it as a further folder; else the
// folder stands for itself (mirrors SessionCategories::project_of in the engine)
export function projectOf(meta: Record<string, ProjectMeta> | undefined, folder: string): string {
  if (!meta || meta[folder]) return folder;
  for (const [key, m] of Object.entries(meta)) if (m.extra?.includes(folder)) return key;
  return folder;
}

// A chat's folder: directly inside `root` (InitState.chats), in either separator
export function isChatFolder(root: string | undefined, cwd: string | undefined): boolean {
  if (!root || !cwd) return false;
  const cut = Math.max(cwd.lastIndexOf('/'), cwd.lastIndexOf('\\'));
  return cut > 0 && cut < cwd.length - 1 && cwd.slice(0, cut) === root;
}

// Whether a project shell keeps a folder off its list: one it took off, or a host folder it never added.
// Search can return summaries outside the host's ordinary project subscription.
export function offList(cwd: string, registered?: string[], removed?: string[]): boolean {
  return !!removed?.includes(cwd) || (!!registered && isRemoteFolder(cwd) && !registered.includes(cwd));
}

// Apply project visibility before pinning or grouping. A view filter may reveal removed folders, but only a search
// reveals host folders that were never added.
export function projectSessions(shown: SessionSummary[], { registered, removed, filtering, expand = filtering, projectOf = same }: Pick<TreeInput, 'registered' | 'removed' | 'filtering' | 'expand' | 'projectOf'>): SessionSummary[] {
  if (filtering && expand) return shown;
  return shown.filter(s => !offList(projectOf(s.cwd), registered, filtering ? undefined : removed));
}

// The category a session is filed under, if that category still exists and belongs to the session's project
const same = (cwd: string) => cwd;

export function categoryOf(s: SessionSummary, categories: SessionCategory[], projectOf: (cwd: string) => string = same): SessionCategory | undefined {
  if (!s.category || s.pinned) return undefined;
  const project = projectOf(s.cwd);
  return categories.find(c => c.id === s.category && c.cwd === project);
}

export function buildSessionTree({ shown, categories, collapsedProjects, workspace, grouped, filtering, registered, removed, pinInProject, isChat, expand = filtering, projectOf = same }: TreeInput): SessionTree {
  const listed = grouped ? projectSessions(shown, { registered, removed, filtering, expand, projectOf }) : shown;
  // Pinning into projects only means something when there are project groups to pin into
  const inProject = grouped && !!pinInProject;
  const pinned = inProject ? [] : listed.filter(s => s.pinned);
  const rest = inProject ? listed : listed.filter(s => !s.pinned);
  const node = (cwd: string, sessions: SessionSummary[]): ProjectNode => {
    const own = categories.filter(c => c.cwd === cwd);
    const filed = new Map(own.map(c => [c.id, [] as SessionSummary[]]));
    const loose: SessionSummary[] = [];
    const pins: SessionSummary[] = [];
    for (const s of sessions) {
      if (s.pinned) { pins.push(s); continue; }
      const c = categoryOf(s, own, projectOf);
      if (c) filed.get(c.id)!.push(s);
      else loose.push(s);
    }
    const nodes = own
      .map(c => ({ category: c, sessions: filed.get(c.id)!, open: expand || !c.collapsed }))
      .filter(n => !filtering || n.sessions.length > 0);
    return {
      cwd,
      current: cwd === workspace,
      open: expand || !collapsedProjects.includes(cwd),
      categories: nodes,
      loose,
      pinned: pins,
      count: sessions.length,
    };
  };

  if (!grouped) {
    // One project: the window's. A session from elsewhere still listed (the active one) stays loose with its tag
    const cwd = workspace ?? '';
    return { pinned, grouped, projects: [node(cwd, rest)], chats: [] };
  }

  const chat = (s: SessionSummary) => !!isChat?.(s.cwd);
  const chats = [...rest.filter(s => s.pinned && chat(s)), ...rest.filter(s => !s.pinned && chat(s))];
  const byProject = new Map<string, SessionSummary[]>();
  for (const s of rest) if (!chat(s)) { const key = projectOf(s.cwd); byProject.set(key, [...byProject.get(key) ?? [], s]); }
  // A project with categories but no session in view still shows (so its categories can be reached), unless filtering
  if (!filtering) for (const c of categories) if (!byProject.has(c.cwd) && !offList(c.cwd, registered)) byProject.set(c.cwd, []);
  if (!filtering) for (const cwd of registered ?? []) if (!byProject.has(cwd)) byProject.set(cwd, []);
  const latest = (cwd: string) => byProject.get(cwd)!.reduce((m, s) => (s.updatedAt > m ? s.updatedAt : m), '');
  // Unregistered folders rank after every registered one (Infinity - Infinity is NaN, which falls through to the next key)
  const rank = (cwd: string) => { const i = registered?.indexOf(cwd) ?? -1; return i < 0 ? Infinity : i; };
  const order = [...byProject.keys()].sort((a, b) => rank(a) - rank(b) || Number(b === workspace) - Number(a === workspace) || latest(b).localeCompare(latest(a)));
  return { pinned, grouped, projects: order.map(cwd => node(cwd, byProject.get(cwd)!)), chats };
}

// Where a session is moved to: another project's folder, optionally one of that project's categories
export interface MoveTarget {
  cwd: string;
  category?: string;
}

// What dropping a session on a zone (a project's category, or its loose area / header when `category` is absent) does.
// Within its own project it is filed (null takes it out of its category); onto another project it moves there, filed under
// that project's category when dropped on one. Undefined: the drop changes nothing.
// Pinned sessions are locked, so they are not dragged at all
export type DropAction =
  | { kind: 'file'; category: string | null }
  | { kind: 'move'; to: MoveTarget };

export function dropAction(s: SessionSummary, target: MoveTarget, categories: SessionCategory[], projectOf: (cwd: string) => string = same): DropAction | undefined {
  // Within a project of several folders a session stays in its own folder: filing it never moves it
  if (projectOf(s.cwd) !== target.cwd) return { kind: 'move', to: target };
  if (target.category) return target.category === categoryOf(s, categories, projectOf)?.id ? undefined : { kind: 'file', category: target.category };
  return categoryOf(s, categories, projectOf) ? { kind: 'file', category: null } : undefined;
}

export function draggable(s: SessionSummary): boolean {
  return !s.pinned;
}
