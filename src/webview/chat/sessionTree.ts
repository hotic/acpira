import type { SessionCategory, SessionSummary } from '@shared/transcript';

// The session list as a tree: pinned sessions on top, then one node per project (the session's cwd), each holding its
// user categories and the sessions filed nowhere ("loose"). Under the workspace scope there is a single node (the
// window's project) and the list shows no project header; under "all" every project is a collapsible group, the
// window's own first. A session sits in exactly one place: pinned, a category of its own project, or loose

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
  // Every non-pinned session of the project in view
  count: number;
}

export interface SessionTree {
  pinned: SessionSummary[];
  // Project groups with headers ("all" scope) or the single headerless node
  grouped: boolean;
  projects: ProjectNode[];
}

export interface TreeInput {
  // The sessions in view (scope, channel filter and search already applied), in list order
  shown: SessionSummary[];
  categories: SessionCategory[];
  collapsedProjects: string[];
  // The window's project folder; absent (the LAB) every project is foreign
  workspace?: string;
  grouped: boolean;
  // A search or channel filter is on: categories / projects without a match hide, the rest open
  filtering: boolean;
}

// The category a session is filed under, if that category still exists and belongs to the session's project
export function categoryOf(s: SessionSummary, categories: SessionCategory[]): SessionCategory | undefined {
  if (!s.category || s.pinned) return undefined;
  return categories.find(c => c.id === s.category && c.cwd === s.cwd);
}

export function buildSessionTree({ shown, categories, collapsedProjects, workspace, grouped, filtering }: TreeInput): SessionTree {
  const pinned = shown.filter(s => s.pinned);
  const rest = shown.filter(s => !s.pinned);
  const node = (cwd: string, sessions: SessionSummary[]): ProjectNode => {
    const own = categories.filter(c => c.cwd === cwd);
    const filed = new Map(own.map(c => [c.id, [] as SessionSummary[]]));
    const loose: SessionSummary[] = [];
    for (const s of sessions) {
      const c = categoryOf(s, own);
      if (c) filed.get(c.id)!.push(s);
      else loose.push(s);
    }
    const nodes = own
      .map(c => ({ category: c, sessions: filed.get(c.id)!, open: filtering || !c.collapsed }))
      .filter(n => !filtering || n.sessions.length > 0);
    return {
      cwd,
      current: cwd === workspace,
      open: filtering || !collapsedProjects.includes(cwd),
      categories: nodes,
      loose,
      count: sessions.length,
    };
  };

  if (!grouped) {
    // One project: the window's. A session from elsewhere still listed (the active one) stays loose with its tag
    const cwd = workspace ?? '';
    return { pinned, grouped, projects: [node(cwd, rest)] };
  }

  const byProject = new Map<string, SessionSummary[]>();
  for (const s of rest) byProject.set(s.cwd, [...byProject.get(s.cwd) ?? [], s]);
  // A project with categories but no session in view still shows (so its categories can be reached), unless filtering
  if (!filtering) for (const c of categories) if (!byProject.has(c.cwd)) byProject.set(c.cwd, []);
  const latest = (cwd: string) => byProject.get(cwd)!.reduce((m, s) => (s.updatedAt > m ? s.updatedAt : m), '');
  const order = [...byProject.keys()].sort((a, b) => Number(b === workspace) - Number(a === workspace) || latest(b).localeCompare(latest(a)));
  return { pinned, grouped, projects: order.map(cwd => node(cwd, byProject.get(cwd)!)) };
}

// Where a dragged session may land: a category of its own project or its own project's loose area.
// Pinned sessions are locked and ChatGPT mirrors have no categories, so neither is dragged at all
export function canFile(s: SessionSummary, target: { cwd: string }): boolean {
  return s.cwd === target.cwd;
}

export function draggable(s: SessionSummary): boolean {
  return !s.pinned && !s.external;
}
