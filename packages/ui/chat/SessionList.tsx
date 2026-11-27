import { Fragment, memo, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState, type CSSProperties, type DragEvent, type KeyboardEvent, type ReactNode } from 'react';
import { ArrowDown, ArrowUp, Copy, FolderOpen, FolderPlus, Layers, Pencil, Plus, Search, Server, SquarePen, Trash2, Undo2, X } from 'lucide-react';
import type { AgentId, AgentInfo, CategoryOp, NativeSessionInfo, SessionCategories, SessionCategory, SessionSection, SessionSummary } from '@acpira/shared/transcript';
import type { ExportFormat, NativeSessionsState, SessionHit } from '@acpira/shared/protocol';
import { inWorkspace, type SessionScope } from '@acpira/shared/settings';
import { launchable } from '@acpira/shared/agentOrder';
import { reuse } from '@acpira/shared/reuse';
import { parseRemoteFolder } from '@acpira/shared/remote';
import { cn } from '../ui/cn';
import { LazyPanelContext } from '../ui/Collapsible';
import { DropdownMenu } from '../ui/DropdownMenu';
import { IconButton } from '../ui/Button';
import { OptionContent } from '../ui/Panel';
import { useScrollFade } from '../ui/useScrollFade';
import { t, useLocale } from '../i18n';
import { macOS, shortcutLabel, useDesktop, windowsOS } from '../desktop';
import { AgentMark } from './AgentMark';
import { useAppearance } from '../appearance';
import { CategoryAddIcon } from './CategoryIcon';
import { Submenu, type MoveProject, type SessionCopyKind } from './SessionMenu';
import { setWebviewState, webviewState } from '../vscodeApi';
import { matchesTitle, searchTerms } from './sessionSearch';
import { needsAttention } from './sessionNav';
import { NAV_LEAD, NAV_ROW, NavDivider, NavEmpty, NavHint, NavSearch } from './NavRows';
import { buildSessionTree, categoryOf, draggable, dropAction, isChatFolder, offList, projectOf as projectOfMeta, projectSessions, type CategoryNode, type MoveTarget, type ProjectNode, type SessionTree } from './sessionTree';
import { clearFilters, filterCount, groupSessions, passes, readView, sortSessions, VIEW_KEY, type FlatGroup, type ListView, type TimeBucket } from './sessionView';
import { ViewMenu } from './ViewMenu';
import { clockRoot } from '../chrome/styleClock';
import { useConnections } from '../remote/api';
import { RemoteFolderIcon } from '../remote/RemoteFolderIcon';
import { startOfDay, projectName, timeWidth, fmtTime } from './sessionTime';
import { Fold, LEAD_ROWS, CHUNK_ROWS, RowChunk, FlipRows } from './SessionListMotion';
import { aggregate, RowIndent, ROW_INDENT_CLASS, Item, RenameInput, RowActions } from './SessionRows';
import { PinDivider, SECTION_ACT, ROW_ACT, Section, FOLDED_KEY, storedFolds, SectionLabel, SectionActions, EmptyProjectRow, pages, MoreRow, GroupHeader, MORE_MARKS, MORE_PAGE } from './SessionListSections';
import { ImportSessions } from './ImportSessions';
// Other modules have always read the project name from here
export { projectName };

// Titles and agent names match locally on every key; the conversation search runs host-side once typing pauses
const SEARCH_DEBOUNCE = 180;

// Desktop: an open project lists its latest sessions, the rest behind one row (MoreRow), which previews this many agents
const LATEST = 5;
// Pure running / background work stays outside "more" up to this many; sessions needing attention have no cap.
const LIVE_KEEP = 5;
const isLive = (s: SessionSummary) => s.state === 'working' || s.state === 'background';

// What decides where every row sits; FlipRows measures only when it changed
function layoutOf(tree: SessionTree, folded: string[], more: Map<string, number>, active?: string): string {
  const ids = (list: SessionSummary[]) => list.map(s => s.id).join();
  // Attention and pure live work have different caps; changing either can move rows across "more" (cutRows).
  const live = (list: SessionSummary[]) => list.filter(s => isLive(s) || needsAttention(s)).map(s => `${s.id}:${needsAttention(s) ? 'attention' : 'live'}`).join();
  return [
    folded.join(),
    live(tree.chats),
    ...tree.projects.map(p => `${live(p.loose)};${p.categories.map(n => live(n.sessions)).join(',')}`),
    active,
    ids(tree.pinned),
    `chats>${more.get(CHATS) ?? 0}>${ids(tree.chats)}`,
    ...tree.projects.map(p =>
      `${p.cwd}>${p.open}${more.get(p.cwd) ?? 0}>${ids(p.pinned)}>${ids(p.loose)}>${p.categories.map(n => `${n.category.id}${n.open}${more.get(`cat:${n.category.id}`) ?? 0}:${ids(n.sessions)}`).join(';')}`),
  ].join('|');
}

// The OS file manager's own name for showing a folder
const revealLabel = () => t(macOS ? 'session.project.revealFinder' : windowsOS ? 'session.project.revealExplorer' : 'session.project.revealFolder');

export interface SessionListProps {
  sessions: SessionSummary[];
  agents: AgentInfo[];
  activeId?: string;
  // The workspace folder this window shows; with it the list can be scoped to the sessions opened there (acpira.sessionScope) and
  // sessions from elsewhere offer "move here". Without it (LAB) every session shows
  workspace?: string;
  scope?: SessionScope;
  // When opened in an overlay the search box auto-focuses; the drawer is permanent and doesn't steal focus
  autoFocus?: boolean;
  // Docked lists fill the available column; history popovers keep their bounded height.
  fill?: boolean;
  // The session the view is on, for the import popover's default agent
  activeAgent?: AgentId;
  // What the import popover shows: the last listing the host answered (keyed by the agent it belongs to)
  nativeSessions?: NativeSessionsState;
  // Present when the host can list an agent's own sessions: the filter row gets an "import" button
  onListNative?: (agent: AgentId) => void;
  onImportNative?: (agent: AgentId, s: NativeSessionInfo) => void;
  onSelect: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
  onPin: (id: string, pinned: boolean) => void;
  // The row menu's "archive" (beside delete) and "mark as unread"; absent where the host does not offer them
  onArchive?: (id: string, archived: boolean) => void;
  onUnread?: (id: string, unread: boolean) => void;
  // Re-home a session: into this window's workspace (no target: the "move here" action) or, dragged onto another project's
  // group under "all", into that project (and its category when dropped on one)
  onMove?: (id: string, to?: MoveTarget) => void;
  // Writes the session as Markdown or JSON under exports/ (absent only where the host does not offer it)
  onExport?: (id: string, format: ExportFormat) => void;
  onCopy?: (id: string, what: SessionCopyKind) => void;
  // Searches the saved conversations too; content hits join the title matches with a snippet under their title
  onSearch?: (query: string) => Promise<SessionHit[]>;
  // User categories (shared by every window through the host). With onFile + onCategoryOp the list shows them, files
  // sessions by drag or the row menu, and offers "new category"; without them it is the plain pinned + flat list
  categories?: SessionCategories;
  onFile?: (id: string, category: string | null) => void;
  // `file`: with a create, the session to file under the new category in the same step
  onCategoryOp?: (op: CategoryOp, file?: string) => void;
  // "New session in this category" (desktop offers it on every project's categories; IDE shells only on the window's own)
  onNewInCategory?: (category: string, cwd?: string) => void;
  // The docked left column's navigation layout (Codex / Claude desktop style): new session, search and import as rows on
  // top, then quiet section labels (pinned, projects / sessions) carrying the filter and "new category"
  nav?: boolean;
  // The nav layout's "new session" row: picks the agent from the same menu as the header's plus
  onNew?: (agent: AgentId) => void;
  // Desktop: "new session" (the column's row, a project's pen) opens the start page at once, in that project when given;
  // the agent is picked there
  onNewIn?: (cwd?: string) => void;
  // Desktop: the Projects label's folder button, the system folder dialog (the folder picked joins the list's end)
  onAddProject?: () => void;
  // Desktop, once the engine answered for SSH hosts: the same menu's "remote folder" entries, a folder on that host
  // (the remote folder picker), or a new host (no alias: Settings → Connections)
  onAddRemote?: (alias?: string) => void;
  // Desktop: a project header's "edit project", its name and folders (ProjectDialog)
  onEditProject?: (cwd: string) => void;
  // A project header's menu: show the folder in the OS file manager, copy its path
  onRevealPath?: (path: string) => void;
  onCopyPath?: (path: string) => void;
  // A project just added: its header flashes once and its group scrolls into view
  fresh?: string;
  // The view menu's "mark all as read" (absent where the host does not offer it)
  onMarkAllRead?: () => void;
  // Desktop: where chats' folders live (InitState.chats). Chats belong to no project and get a section of their own
  chats?: string;
  // Desktop: the chats section's pen, a new chat on the start page
  onNewChat?: () => void;
}

// The drag payload's type: WebKit starts no drag without data. Telling a row's drag goes by `held`, not by this type
const DRAG_TYPE = 'application/x-acpira-session';
// A drop zone's marks, set by the drag handlers straight on the DOM (`paint`), never by a render. `data-hot` frames the
// zone: a thin light outline drawn inset (never shifts layout) around the whole block, header and children. `data-insert`
// ("top" / "bottom") draws the insert line on that edge in the frame's colour, so every drop reads in one style; that
// zone needs `relative`
const FRAME = 'transition-shadow data-hot:ring-1 data-hot:ring-inset data-hot:ring-fg-1/70';
const LINE = 'data-insert:before:pointer-events-none data-insert:before:absolute data-insert:before:inset-x-2 data-insert:before:z-10 data-insert:before:h-0.5 data-insert:before:rounded-full data-insert:before:bg-fg-1/70 data-[insert=top]:before:-top-px data-[insert=bottom]:before:-bottom-px';

// A project is held by its folder; a section by its id
type Held = { kind: 'session' | 'category' | 'project' | 'section'; id: string };
type Over =
  | { kind: 'category'; id: string; cwd: string }
  // A project's loose area; for a session of another project, anywhere on that project's group (header included)
  | { kind: 'loose'; cwd: string }
  // Reordering categories: the dragged one goes before `before` (absent: after the project's last)
  | { kind: 'gap'; cwd: string; before?: string }
  // Placing a project (desktop): into `section` (absent: under Projects), before the project `before` (absent: at the end)
  | { kind: 'slot'; section?: string; before?: string }
  // Reordering sections (desktop): the dragged one goes before `before` (absent: after the last section), with the
  // insert line drawn on `anchor`'s `edge` so the mark stays under the pointer across expanded sections
  | { kind: 'sectionGap'; before?: string; anchor: string; edge: 'top' | 'bottom' };
// The [data-section] of the Projects section, the one holding the projects filed in no section
const REST = 'rest';
// The `more` key of the chats section (a project's is its folder)
const CHATS = ' chats';

const sameOver = (a?: Over, b?: Over) => JSON.stringify(a) === JSON.stringify(b);
const newCategoryId = () => `c-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
const newSectionId = () => `s-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

const separator = <DropdownMenu.Separator className="my-1 h-px bg-line" />;

// One empty list for every render without categories, so what is derived from it keeps its identity
const NO_CATEGORIES: SessionCategory[] = [];

// The previous value while the new one is deep-equal to it (shared/reuse): something derived from the session list, which
// every push replaces, keeps its identity until it really changes
function useReused<T>(value: T): T {
  const kept = useRef(value);
  kept.current = reuse(kept.current, value);
  return kept.current;
}

// A function of a string, computed once per argument: callers asking for the same key get the same object back
function perKey<T>(compute: (key: string) => T): (key: string) => T {
  const cache = new Map<string, T>();
  return key => {
    if (!cache.has(key)) cache.set(key, compute(key));
    return cache.get(key)!;
  };
}

// Session list: search and agent filters stay visible even without history; pinned sessions get their own section on top.
// Below them the sessions are a tree (sessionTree.ts): under the workspace scope the window's project, its user categories
// then its loose sessions; under "all" one collapsible group per project (folder glyph), each holding its categories (inbox
// glyph) and loose sessions. Each item: vendor mark · title · time; on hover the time cross-fades into the actions — pin,
// plus "move here" for a session from another project — and the row's right-click menu holds the rest. Project, category
// and custom section headers open their "…" menu on right-click too.
// Sessions file by native HTML5 drag (the browser's own translucent drag image) into a category of their own project or back
// to its loose area; under "all", dropped on another project's group (or one of its categories) they move to that project.
// Pinned rows are locked and do not drag; category and section headers drag to reorder.
// Every change moves: folds open on the panel curve with their rows stepping in (Fold), rows that change place slide
// (FlipRows). The desktop app (DesktopContext) lists its registered projects in their own order, each open project
// showing its latest loose sessions (the rest behind one row) followed by its categories.
// Deletion applies immediately, undo lives on the Toast at the shell's bottom
export const SessionList = memo(function SessionList({ sessions, agents, activeId, workspace, scope = 'all', autoFocus, fill, activeAgent, nativeSessions, onListNative, onImportNative, onSelect, onRename, onDelete, onPin, onArchive, onUnread, onMove, onExport, onCopy, onSearch, categories: categoryState, onFile, onCategoryOp, onNewInCategory, nav, onNew, onNewIn, onAddProject, onAddRemote, onEditProject, onRevealPath, onCopyPath, fresh, onMarkAllRead, chats, onNewChat }: SessionListProps) {
  const locale = useLocale();
  const desktop = useDesktop();
  const projectMarks = useAppearance().projectMarks === 'on';
  // The SSH hosts the add menu offers folders on; undefined until the engine answered for them
  const connections = useConnections();
  const remoteHosts = onAddRemote ? connections : undefined;
  const [query, setQuery] = useState('');
  // The view menu's choices (grouping, ordering, what rows show, filters), kept per window across reloads
  const [view, setViewState] = useState<ListView>(() => readView(webviewState(VIEW_KEY)));
  const setView = (next: ListView) => { setViewState(next); setWebviewState(VIEW_KEY, next); };
  const filters = filterCount(view);
  // One channel picked: the import popover starts on it, as it did with the old channel filter
  const agentFilter = view.agents.length === 1 ? view.agents[0] : undefined;
  // A section being renamed; set right after creating one, before the host has pushed it back
  const [editingSection, setEditingSection] = useState<string>();
  const [editing, setEditing] = useState<string>();
  // A category being renamed; set right after creating one, before the host has pushed it back
  const [editingCategory, setEditingCategory] = useState<string>();
  const [held, setHeld] = useState<Held>();
  // The hovered zone and the elements marked for it: a ref, not state, so hovering never re-renders the list
  const over = useRef<Over>(undefined);
  const marked = useRef<HTMLElement[]>([]);
  // Nav layout: the sections folded by their label, kept in the webview's state across reloads
  const [folded, setFolded] = useState<Section[]>(storedFolds);
  // Desktop: per project, how many sessions past the latest few are shown. A click on the "more" row adds a page; with
  // nothing left to show it folds them all away again
  const listFade = useScrollFade<HTMLDivElement>();
  // The desktop column's list fades at its edges as well as carrying the style clock
  const listRoot = useCallback((el: HTMLDivElement | null) => {
    const clock = clockRoot(el);
    const fade = desktop ? listFade(el) : undefined;
    return () => { clock?.(); fade?.(); };
  }, [desktop, listFade]);
  const [more, setMore] = useState<Map<string, number>>(() => new Map());
  const stepMore = (cwd: string, total: number) => setMore(prev => {
    const next = new Map(prev);
    const shown = prev.get(cwd) ?? 0;
    if (shown >= total) next.delete(cwd);
    else next.set(cwd, shown + MORE_PAGE);
    return next;
  });
  // Like Codex, a fold forgets its pages: reopening a project (or the chats) starts from the latest few again. Cleared on the
  // way open, not shut, so the closing fold keeps its height while it animates
  const resetMore = (key: string) => setMore(prev => {
    if (!prev.has(key)) return prev;
    const next = new Map(prev);
    next.delete(key);
    return next;
  });
  const nameOf = (id: string) => agents.find(a => a.id === id)?.name ?? id;
  const here = (s: SessionSummary) => !workspace || inWorkspace(s, workspace);
  const filing = !!onFile && !!onCategoryOp;
  const categories = filing ? categoryState?.categories ?? NO_CATEGORIES : NO_CATEGORIES;

  // The rows follow the field on a deferred copy: the field shows each key at once, and the list renders in slices React
  // drops for the next key, so a first letter matching hundreds of sessions no longer holds the frame it was typed in
  const listQuery = useDeferredValue(query);
  const terms = useMemo(() => searchTerms(listQuery), [listQuery]);
  const q = terms.join(' ');
  // The content search goes out for the field's own text, not the list's deferred copy: the engine searches while the
  // list renders, where asking after that render put the two one after the other
  const asked = useMemo(() => searchTerms(query).join(' '), [query]);
  // Content hits belong to the query they answered; a reply for an older query is ignored rather than mixed in
  const [hits, setHits] = useState<{ q: string; snippets: Map<string, string>; summaries: SessionSummary[] }>();
  useEffect(() => {
    if (!onSearch || !asked) { setHits(undefined); return; }
    let live = true;
    const timer = setTimeout(() => {
      void onSearch(asked).then(list => {
        if (live) setHits({ q: asked, snippets: new Map(list.map(h => [h.id, h.snippet])), summaries: list.flatMap(h => h.summary ? [h.summary] : []) });
      });
    }, SEARCH_DEBOUNCE);
    return () => { live = false; clearTimeout(timer); };
  }, [asked, onSearch]);
  const snippets = hits?.q === q ? hits.snippets : undefined;
  const searching = !!onSearch && !!q && !snippets;
  // Search-only remote summaries never enter the persistent list and are released when the query clears.
  const searched = useMemo(() => {
    if (!q || hits?.q !== q || !hits.summaries.length) return sessions;
    const known = new Set(sessions.map(s => s.id));
    return [...sessions, ...hits.summaries.filter(s => !known.has(s.id))];
  }, [sessions, hits, q]);
  // Scope keeps the active session even in another project; project visibility and view filters still apply.
  const inScope = (s: SessionSummary) => scope === 'all' || here(s) || s.id === activeId;
  // Before the list arrives the desktop app registers nothing yet, rather than every folder (`offList`)
  const registered = desktop ? categoryState?.projects ?? [] : undefined;
  const removed = desktop ? categoryState?.removedProjects : undefined;
  const filtering = !!q || filters > 0;
  // Pins and flat views follow the same project visibility as the project tree.
  // Desktop: a project of several folders (SessionCategories.projectMeta) lists the sessions of each, under its own name
  const meta = desktop ? categoryState?.projectMeta : undefined;
  const projectOf = useCallback((cwd: string) => projectOfMeta(meta, cwd), [meta]);
  const nameOfProject = (cwd: string) => meta?.[cwd]?.name || projectName(cwd);
  const listed = desktop ? projectSessions(searched, { registered, removed, filtering, expand: !!q, projectOf }) : searched;
  const at = new Date();
  const shown = sortSessions(listed.filter(s => inScope(s) && passes(s, view, at)
    && (!q || matchesTitle(s.title, nameOf(s.agent), terms) || !!snippets?.has(s.id))), view.order, locale);

  // Grouped by date, channel or not at all, the rows leave the project tree for one flat list under group labels
  const flat = view.group !== 'project';
  const flatGroups: FlatGroup[] = view.group === 'project' ? [] : view.group === 'none'
    ? [{ key: '', sessions: shown.filter(s => !s.pinned) }].filter(g => g.sessions.length > 0)
    : groupSessions(shown.filter(s => !s.pinned), view.group, view.order, at);
  // The channels the view menu offers: those the list's sessions ran on, in the agents' order (unknown ones last)
  const channels = useMemo<AgentInfo[]>(() => {
    const ids = new Set(sessions.map(s => s.agent));
    return [...agents.filter(a => ids.has(a.id)), ...[...ids].filter(id => !agents.some(a => a.id === id)).map(id => ({ id, name: id }) as AgentInfo)];
  }, [sessions, agents]);
  // The desktop app always groups by project, its registered folders leading in their own order
  const grouped = desktop || (scope === 'all' && !!workspace);
  // View menu → Show → Pinned: grouped by project, pinned rows lead their own project instead of one section on top
  const pinInProject = grouped && !flat && view.pinned === 'project';
  // Desktop: a chat's folder is no project, so it never heads a group or becomes a destination
  const isChat = (cwd: string) => desktop && isChatFolder(chats, cwd);
  const tree = buildSessionTree({
    shown, categories, collapsedProjects: filing ? categoryState?.collapsedProjects ?? [] : [],
    workspace, grouped, filtering, expand: !!q, registered, removed, pinInProject, isChat, projectOf,
  });
  const pinnedHere = (s: SessionSummary) => !!s.pinned && pinInProject;
  // Every project the list knows (the sessions' folders, the categories' own, the sections' and the registered ones, not
  // those off the list): custom sections first in their own order, then the remaining projects (the window's first, then
  // registered order, then most recently used). Taken from all sessions, not the filtered view, so a search does not hide
  // one. Kept while equal: every push brings a new session list
  const projects = useReused(useMemo<MoveProject[]>(() => {
    const latest = new Map<string, string>();
    for (const s of sessions) { const cwd = projectOf(s.cwd); if (cwd && (latest.get(cwd) ?? '') < s.updatedAt) latest.set(cwd, s.updatedAt); }
    for (const c of categoryState?.categories ?? []) if (c.cwd && !latest.has(c.cwd)) latest.set(c.cwd, '');
    for (const cwd of registered ?? []) if (!latest.has(cwd)) latest.set(cwd, '');
    for (const sec of categoryState?.sections ?? []) for (const cwd of sec.projects) if (cwd && !latest.has(cwd)) latest.set(cwd, '');
    const valid = new Set([...latest.keys()].filter(c => !offList(c, registered, removed) && !(desktop && isChatFolder(chats, c))));
    // A project on an SSH host carries the host's name; a local one carries nothing (its full path is the row's tooltip)
    const toMove = (cwd: string, sec?: SessionSection): MoveProject => {
      const alias = parseRemoteFolder(cwd)?.alias;
      return {
        cwd,
        name: meta?.[cwd]?.name || projectName(cwd),
        current: cwd === workspace,
        ...(alias ? { hint: alias } : {}),
        ...(sec ? { sectionId: sec.id, sectionName: sec.name } : {}),
      };
    };
    const placed = new Set<string>();
    const sectioned: MoveProject[] = [];
    for (const sec of categoryState?.sections ?? []) {
      for (const cwd of sec.projects) {
        if (valid.has(cwd) && !placed.has(cwd)) {
          placed.add(cwd);
          sectioned.push(toMove(cwd, sec));
        }
      }
    }
    const rank = (cwd: string) => { const i = registered?.indexOf(cwd) ?? -1; return i < 0 ? Infinity : i; };
    const cwds = [...valid].filter(c => !placed.has(c)).sort((a, b) => Number(b === workspace) - Number(a === workspace) || rank(a) - rank(b) || latest.get(b)!.localeCompare(latest.get(a)!));
    return [...sectioned, ...cwds.map(cwd => toMove(cwd))];
  }, [sessions, categoryState, registered, removed, workspace, desktop, chats, projectOf, meta]));
  // A row's slices of the menus' lists, one array per project, so a row gets the same one on every render
  const projectsBesides = useMemo(() => perKey(cwd => projects.filter(p => p.cwd !== cwd)), [projects]);
  const categoriesIn = useMemo(() => perKey(cwd => categories.filter(c => c.cwd === cwd)), [categories]);
  // Under the workspace scope only the window's own categories show (a foreign active session stays loose)
  const visibleCategories = tree.projects.some(p => p.categories.length > 0);
  // Desktop, grouped by project: the user's sections, each holding its projects in its own order; the projects in none stay
  // under Projects. A filter or search hides the sections it leaves empty
  const sections: SessionSection[] = desktop && filing && !flat ? categoryState?.sections ?? [] : [];
  const sectionOf = (cwd: string) => sections.find(x => x.projects.includes(cwd));
  const sectionNodes = sections
    .map(section => ({ section, projects: section.projects.map(cwd => tree.projects.find(p => p.cwd === cwd)).filter((p): p is ProjectNode => !!p) }))
    .filter(n => !filtering || n.projects.length > 0);
  const unsectioned = sections.length ? tree.projects.filter(p => !sectionOf(p.cwd)) : tree.projects;

  const now = new Date();
  const dayOf = (iso: string) => Math.floor((startOfDay(now) - startOfDay(new Date(iso))) / 86_400_000);
  const times = new Map(shown.map(s => [s.id, fmtTime(s.updatedAt, dayOf(s.updatedAt), locale)]));
  // The longest time sizes the shared time column
  const timeCh = Math.max(0, ...[...times.values()].map(timeWidth));

  // Creating a category opens its name for editing as soon as the host pushes it back
  const createCategory = (cwd: string, file?: string) => {
    const id = newCategoryId();
    onCategoryOp?.({ op: 'create', id, name: '', cwd }, file);
    if (grouped && categoryState?.collapsedProjects.includes(cwd)) onCategoryOp?.({ op: 'collapseProject', cwd, collapsed: false });
    setEditingCategory(id);
  };
  // Likewise a section, `project` filed into it in the same edit (from a project's "move to section")
  const createSection = (project?: string) => {
    const id = newSectionId();
    onCategoryOp?.({ op: 'createSection', id, name: '', ...(project ? { project } : {}) });
    setEditingSection(id);
  };

  // ---- Native drag and drop. One handler pair on the list: the hovered zone comes from the event target's nearest
  // [data-drop] ("category:<id>" / "loose:<cwd>") or, for a category / section being reordered, [data-category] / [data-section]
  const resolve = (h: Held, target: Element, y: number): Over | undefined => {
    if (h.kind === 'section') {
      const block = target.closest<HTMLElement>('[data-section]');
      const hovered = block?.dataset.section;
      if (!block || !hovered) return undefined;
      const ids = sectionNodes.map(n => n.section.id);
      // Dropping onto Projects ("rest") appends after the last custom section unless this one is already last
      if (hovered === REST) {
        const last = ids.at(-1);
        return last && last !== h.id ? { kind: 'sectionGap', anchor: last, edge: 'bottom' } : undefined;
      }
      if (hovered === h.id || !ids.includes(hovered)) return undefined;
      const siblings = ids.filter(id => id !== h.id);
      const box = block.getBoundingClientRect();
      const after = y > box.top + box.height / 2;
      const before = after ? siblings[siblings.indexOf(hovered) + 1] : hovered;
      return { kind: 'sectionGap', ...(before ? { before } : {}), anchor: hovered, edge: after ? 'bottom' : 'top' };
    }
    if (h.kind === 'project') {
      // Over a project: before it (its upper half) or after it, in whichever section it sits; over a section's label or
      // empty row: at that section's end
      const block = target.closest<HTMLElement>('[data-project]');
      const hovered = block?.dataset.project;
      if (block && hovered !== undefined) {
        if (hovered === h.id) return undefined;
        const section = sectionOf(hovered)?.id;
        const run = (section ? sectionNodes.find(n => n.section.id === section)?.projects ?? [] : unsectioned).map(p => p.cwd).filter(c => c !== h.id);
        const box = block.getBoundingClientRect();
        const before = y < box.top + box.height / 2 ? hovered : run[run.indexOf(hovered) + 1];
        return { kind: 'slot', ...(section ? { section } : {}), ...(before ? { before } : {}) };
      }
      const zone = target.closest<HTMLElement>('[data-section]')?.dataset.section;
      if (!zone) return undefined;
      return zone === REST ? { kind: 'slot' } : { kind: 'slot', section: zone };
    }
    if (h.kind === 'category') {
      const moving = categories.find(c => c.id === h.id);
      const block = target.closest<HTMLElement>('[data-category]');
      const hovered = categories.find(c => c.id === block?.dataset.category);
      if (!moving || !block || !hovered || hovered.cwd !== moving.cwd || hovered.id === moving.id) return undefined;
      const box = block.getBoundingClientRect();
      const siblings = categories.filter(c => c.cwd === hovered.cwd && c.id !== moving.id);
      const next = siblings[siblings.indexOf(hovered) + 1];
      return { kind: 'gap', cwd: hovered.cwd, before: y > box.top + box.height / 2 ? next?.id : hovered.id };
    }
    const zone = target.closest<HTMLElement>('[data-drop]')?.dataset.drop;
    if (!zone) return undefined;
    const split = zone.indexOf(':');
    const [kind, ref] = [zone.slice(0, split), zone.slice(split + 1)];
    if (kind === 'category') {
      const c = categories.find(x => x.id === ref);
      return c && { kind: 'category', id: ref, cwd: c.cwd };
    }
    return { kind: 'loose', cwd: ref };
  };
  const heldSession = held?.kind === 'session' ? sessions.find(s => s.id === held.id) : undefined;
  const begin = (h: Held) => (e: DragEvent) => {
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData(DRAG_TYPE, h.id);
    setHeld(h);
  };
  // Marks the zone a drop would land in. Straight on the DOM, in the dragover that found it: re-rendering a list of a
  // thousand sessions per zone change left the frame behind the pointer (and WebKit's cursor with it). React leaves the
  // data attributes alone, since no prop names them
  const unpaint = () => {
    for (const el of marked.current) { delete el.dataset.hot; delete el.dataset.insert; }
    marked.current = [];
  };
  const paint = (root: HTMLElement, next?: Over) => {
    unpaint();
    if (!next) return;
    const find = (attr: string, value?: string) => value === undefined ? null : root.querySelector<HTMLElement>(`[${attr}="${CSS.escape(value)}"]`);
    const mark = (el: HTMLElement | null, insert?: 'top' | 'bottom') => {
      if (!el) return;
      if (insert) el.dataset.insert = insert; else el.dataset.hot = '';
      marked.current.push(el);
    };
    if (next.kind === 'category') mark(find('data-category', next.id));
    // Only a session of this project lights its loose area; one from elsewhere frames the whole project group
    else if (next.kind === 'loose') mark(heldSession && projectOf(heldSession.cwd) === next.cwd ? find('data-loose', next.cwd) : onMove && heldSession ? find('data-group', next.cwd) : null);
    else if (next.kind === 'gap') {
      if (next.before) mark(find('data-category', next.before), 'top');
      else mark(find('data-category', tree.projects.find(p => p.cwd === next.cwd)?.categories.at(-1)?.category.id), 'bottom');
    } else if (next.kind === 'sectionGap') {
      mark(find('data-section', next.anchor), next.edge);
    } else {
      // A held project: the line above the project it would precede, or under its run's last one; landing in another
      // section (or out of one) also frames where it lands, as a session landing in another project does
      const run = next.section ? sectionNodes.find(n => n.section.id === next.section)?.projects ?? [] : unsectioned;
      if (next.before) mark(find('data-project', next.before), 'top');
      else mark(find('data-project', run.at(-1)?.cwd), 'bottom');
      if (held?.kind === 'project' && sectionOf(held.id)?.id !== next.section) mark(find('data-section-body', next.section ?? REST));
    }
  };
  const end = () => { setHeld(undefined); over.current = undefined; unpaint(); };
  const onDragOver = (e: DragEvent) => {
    // `held` alone tells a row's drag: the desktop app's WKWebView may list "Files" and leave the payload's type out of
    // `types` (the column's folder frame lit while no zone did)
    if (!held) return;
    const target = e.target as Element;
    // The margin between two zones (above a project) belongs to none: the pointer keeps the zone it came from on its way
    // to the next, instead of the frame blinking off and the cursor flipping to "no drop" in between
    const between = held.kind !== 'section' && !!over.current && !target.closest('[data-drop]') && !!target.querySelector(':scope > [data-drop]');
    const next = between ? over.current : resolve(held, target, e.clientY);
    // Moving to another project needs the host's move; without it only same-project zones take the drop
    const ok = !!next && (next.kind === 'gap' || next.kind === 'slot' || next.kind === 'sectionGap' || !heldSession || projectOf(heldSession.cwd) === next.cwd || !!onMove);
    if (ok) e.preventDefault();
    e.dataTransfer.dropEffect = ok ? 'move' : 'none';
    // A push that re-mounted a marked block took its marks along: paint again
    if (!sameOver(next, over.current) || marked.current.some(el => !el.isConnected)) {
      over.current = next;
      paint(e.currentTarget as HTMLElement, next);
    }
  };
  const onDrop = (e: DragEvent) => {
    if (!held) return;
    e.preventDefault();
    const at = over.current;
    const zone = heldSession && at && at.kind !== 'gap' && at.kind !== 'slot' && at.kind !== 'sectionGap' ? { cwd: at.cwd, ...(at.kind === 'category' ? { category: at.id } : {}) } : undefined;
    const action = heldSession && zone ? dropAction(heldSession, zone, categories, projectOf) : undefined;
    if (action?.kind === 'file') onFile?.(heldSession!.id, action.category);
    if (action?.kind === 'move') onMove?.(heldSession!.id, action.to);
    if (held.kind === 'category' && at?.kind === 'gap') onCategoryOp?.({ op: 'reorder', id: held.id, ...(at.before ? { before: at.before } : {}) });
    if (held.kind === 'project' && at?.kind === 'slot') placeProject(held.id, at);
    if (held.kind === 'section' && at?.kind === 'sectionGap') {
      const currentNext = sections[sections.findIndex(s => s.id === held.id) + 1]?.id;
      if (at.before !== currentNext) onCategoryOp?.({ op: 'reorderSection', id: held.id, ...(at.before ? { before: at.before } : {}) });
    }
    end();
  };
  // A dropped project: filed in its section at the slot, or out of any section and placed among the rest. The two ops of
  // the second case touch different fields, so they compose in either order
  const placeProject = (cwd: string, slot: { section?: string; before?: string }) => {
    const before = slot.before ? { before: slot.before } : {};
    if (slot.section) { onCategoryOp?.({ op: 'setProjectSection', cwd, section: slot.section, ...before }); return; }
    if (sectionOf(cwd)) onCategoryOp?.({ op: 'setProjectSection', cwd });
    onCategoryOp?.({ op: 'reorderProject', cwd, ...before });
  };

  // What a row's controls do, read through the ref when they are used: rows hold the same object on every render, so a
  // row whose own props a push left alone skips its render (Item is memoized), where every listed row used to re-render
  const rowActions = useRef<RowActions>(null!);
  const patchSearch = (id: string, patch?: Partial<SessionSummary>) => setHits(prev => prev && ({
    ...prev, summaries: patch ? prev.summaries.map(s => s.id === id ? { ...s, ...patch } : s) : prev.summaries.filter(s => s.id !== id),
  }));
  rowActions.current = {
    // A search-only row (a host session outside the list) gets no push: opening it reads it here too
    select: s => { onSelect(s.id); if (s.state === 'unread' || s.unread) patchSearch(s.id, { state: s.state === 'unread' ? undefined : s.state, unread: false }); },
    edit: s => setEditing(s.id),
    rename: (s, title) => { setEditing(undefined); if (title.trim() && title.trim() !== s.title) { onRename(s.id, title); patchSearch(s.id, { title }); } },
    delete: s => { setEditing(undefined); onDelete(s.id); patchSearch(s.id); },
    pin: s => { onPin(s.id, !s.pinned); patchSearch(s.id, { pinned: !s.pinned }); },
    archive: (s, archived) => { onArchive?.(s.id, archived); patchSearch(s.id, { archived }); },
    unread: (s, unread) => { onUnread?.(s.id, unread); patchSearch(s.id, { state: unread ? 'unread' : undefined, unread }); },
    move: (s, to) => onMove?.(s.id, to),
    export: (s, format) => onExport?.(s.id, format),
    copy: (s, what) => onCopy?.(s.id, what),
    file: (s, category) => onFile?.(s.id, category),
    newCategory: s => createCategory(projectOf(s.cwd), s.id),
    dragStart: (s, e) => begin({ kind: 'session', id: s.id })(e),
    dragEnd: end,
  };
  const renderItem = (s: SessionSummary, inCategory = false) => {
    const rowIndent: RowIndent | undefined = flat || isChat(s.cwd) || (s.pinned && !pinnedHere(s))
      ? undefined
      : tree.grouped
        ? (inCategory ? 'nested' : 'project')
        : (inCategory ? 'category' : undefined);
    return (
      // A chat's row sits flush under its section label, like a pinned one, and names no project
      <Item
        key={s.id}
        session={s}
        agentName={nameOf(s.agent)}
        terms={terms}
        snippet={snippets?.get(s.id)}
        active={s.id === activeId}
        time={times.get(s.id) ?? ''}
        // Grouped, a row's project is its group, so only pinned rows (which sit above every group) name theirs. The desktop
        // app names none: the title gets the room, the tooltip still names it
        // Grouped by date or channel the rows leave their project's group, so they name it unless it is the window's own
        project={isChat(s.cwd) ? (flat ? t('session.group.chats') : undefined) : flat ? (desktop || !here(s) ? nameOfProject(projectOf(s.cwd)) : undefined) : desktop || here(s) || (grouped && (!s.pinned || pinnedHere(s))) ? undefined : nameOfProject(projectOf(s.cwd))}
        model={view.model ? s.model?.name : undefined}
        indent={rowIndent}
        editing={editing === s.id}
        dragging={held?.kind === 'session' && held.id === s.id}
        actions={rowActions}
        drag={filing && draggable(s) && (grouped || here(s)) && editing !== s.id}
        moveHere={!!onMove && !!workspace && !desktop && !here(s)}
        moveTo={!!onMove}
        exportable={!!onExport}
        copyable={!!onCopy}
        archivable={!!onArchive}
        markable={!!onUnread}
        fileable={filing && !isChat(s.cwd)}
        projects={projectsBesides(projectOf(s.cwd))}
        categories={categoriesIn(projectOf(s.cwd))}
      />
    );
  };

  // The rows of one run, a long one by chunk (RowChunk). `run` names the run among its siblings' keys
  const rowsOf = (list: SessionSummary[], run = 'chunk', inCategory = false) => {
    const draw = (s: SessionSummary) => renderItem(s, inCategory);
    if (q || list.length <= LEAD_ROWS + CHUNK_ROWS) return list.map(draw);
    const rows: ReactNode[] = list.slice(0, LEAD_ROWS).map(draw);
    for (let at = LEAD_ROWS, n = 0; at < list.length; at += CHUNK_ROWS, n++) {
      const part = list.slice(at, at + CHUNK_ROWS);
      rows.push(
        <RowChunk key={`${run}:${n}`} count={part.length} first={n === 0} keep={part.some(x => x.id === editing || (held?.kind === 'session' && x.id === held.id))}>
          {() => part.map(draw)}
        </RowChunk>,
      );
    }
    return rows;
  };

  // A category: a collapsible tree row (leading chevron, medium-muted label, count at the tail) whose sessions step in
  // one notch so their agent mark aligns with the category name. The whole block is one drop zone and lights up as one;
  // a collapsed category stays collapsed under a drag (no spring-open), so its frame is just the header
  const renderCategory = (node: CategoryNode) => {
    const c = node.category;
    const catKey = `cat:${c.id}`;
    const newHere = onNewInCategory && (desktop || c.cwd === workspace) ? () => onNewInCategory(c.id, desktop ? c.cwd : undefined) : undefined;
    const sessionIndent: RowIndent = tree.grouped ? 'nested' : 'category';
    return (
      <div key={c.id} role="group" aria-label={c.name} data-category={c.id} data-drop={`category:${c.id}`}
        className={cn('relative mt-0.5 flex flex-col rounded-lg', FRAME, LINE)}>
        <GroupHeader
          sub
          indent={tree.grouped}
          name={c.name}
          count={node.sessions.length}
          open={node.open}
          state={node.open ? undefined : aggregate(node.sessions)}
          dragging={held?.kind === 'category' && held.id === c.id}
          editing={editingCategory === c.id}
          onDragStart={!filtering && editingCategory !== c.id ? begin({ kind: 'category', id: c.id }) : undefined}
          onDragEnd={end}
          onToggle={() => { if (!node.open) resetMore(catKey); onCategoryOp?.({ op: 'collapse', id: c.id, collapsed: node.open }); }}
          onRename={name => { setEditingCategory(undefined); if (name.trim() && name.trim() !== c.name) onCategoryOp?.({ op: 'rename', id: c.id, name }); }}
          onNewSession={newHere}
          menu={<>
            <DropdownMenu.Item onClick={() => setEditingCategory(c.id)}><OptionContent icon={<Pencil strokeWidth={1.5} />}>{t('session.category.rename')}</OptionContent></DropdownMenu.Item>
            {newHere && <DropdownMenu.Item onClick={newHere}><OptionContent icon={<SquarePen strokeWidth={1.5} />}>{t('session.category.newSession')}</OptionContent></DropdownMenu.Item>}
            {separator}
            <DropdownMenu.Item className="text-danger" onClick={() => onCategoryOp?.({ op: 'delete', id: c.id })}>
              <OptionContent icon={<Trash2 strokeWidth={1.5} />}>{t('session.category.delete')}</OptionContent>
            </DropdownMenu.Item>
          </>}
        />
        <Fold open={node.open}>
          {node.sessions.length
            ? cutRows(catKey, node.sessions, sessionIndent)
            : <div className={cn('flex min-h-row items-center pr-2 text-3 text-fg-3', ROW_INDENT_CLASS[sessionIndent])}>{t('session.category.empty')}</div>}
        </Fold>
      </div>
    );
  };

  // A project's body: its loose sessions (the zone that takes a session out of its category), then its categories.
  // Desktop: the latest few of `list`, the active one when it is older than those, the rest folded behind one row whose
  // page count `key` keeps; a search lists every match, while the view menu's filters keep the cut
  const cutRows = (key: string, list: SessionSummary[], indent?: RowIndent) => {
    const inCategory = indent === 'category' || indent === 'nested';
    const cut = desktop && !q && view.per ? view.per : list.length;
    const extra = more.get(key) ?? 0;
    // Keep attention and the active session outside "more", in list order, without opening all rows before them.
    // Only pure running / background work is capped; unread replies still surface while background work continues.
    const beyond = list.slice(cut + extra);
    const live = beyond.filter(s => isLive(s) && !needsAttention(s));
    const kept = new Set(beyond.filter(needsAttention));
    if (live.length <= LIVE_KEEP) for (const s of live) kept.add(s);
    const active = beyond.find(s => s.id === activeId);
    if (active) kept.add(active);
    const late = beyond.filter(s => kept.has(s));
    const rest = late.length ? list.slice(cut).filter(s => !kept.has(s)) : list.slice(cut);
    const shown = Math.min(extra, rest.length);
    // A flat keyed array, not a fragment: a project's pinned rows are its siblings, so a row pinned or unpinned in place
    // keeps its element (renderBody)
    return [
      ...rowsOf(list.slice(0, cut), key, inCategory),
      // One fold per page, so each click opens only the page it adds (a closed fold releases its rows)
      ...(rest.length > 0 ? pages(rest).map((page, i) => <Fold key={`fold:${i}`} open={shown > i * MORE_PAGE}>{page.map(s => renderItem(s, inCategory))}</Fold>) : []),
      ...late.map(s => renderItem(s, inCategory)),
      rest.length > 0 && <MoreRow key="more" group={key} indent={indent} open={shown >= rest.length} rest={rest.slice(shown)} nameOf={nameOf} onToggle={() => stepMore(key, rest.length)} />,
    ];
  };
  const renderBody = (p: ProjectNode) => {
    // A filed session of this project being dragged needs somewhere to land even when nothing is loose
    const strip = !p.loose.length && !!heldSession && projectOf(heldSession.cwd) === p.cwd && !!categoryOf(heldSession, categories, projectOf);
    return <>
      {/* Pinned first, outside the "more" cut (pinned rows never drag). They share one keyed list with the loose rows:
          under separate parents a row pinned or unpinned in place was remounted, and the new element, painted unhovered
          under the pointer, flashed its time and turned its mark over again as the hover came back */}
      {(p.pinned.length > 0 || p.loose.length > 0 || strip) && (
        <div data-drop={`loose:${p.cwd}`} data-loose={p.cwd} className={cn('flex flex-col rounded-lg', FRAME)}>
          {[
            ...p.pinned.map(s => renderItem(s)),
            // The pinned rows end at a short faded rule, the IDE list's, instead of a pin glyph on each row's tail
            !flat && <PinDivider key="pin-divider" on={p.pinned.length > 0 && (p.loose.length > 0 || strip || p.categories.length > 0)} />,
            ...cutRows(p.cwd, p.loose, tree.grouped ? 'project' : undefined),
          ]}
          {strip && <div className={cn('flex min-h-row items-center pr-2 text-3 text-fg-3', tree.grouped ? 'pl-6' : 'pl-2')}>{t('session.category.remove')}</div>}
        </div>
      )}
      {p.categories.map(renderCategory)}
      {desktop && !p.count && !p.categories.length && <EmptyProjectRow onNew={onNewIn ? () => onNewIn(p.cwd) : undefined} />}
    </>;
  };

  // A project header's menu, on its "…" and on right-click: show the folder, copy its path, new category; the desktop app
  // adds edit project, move to section, and "remove from the list" for any folder, registered or listed for its sessions
  const projectMenu = (cwd: string): ReactNode => {
    const items = [
      onEditProject && desktop && filing && <DropdownMenu.Item key="edit" onClick={() => onEditProject(cwd)}><OptionContent icon={<Pencil strokeWidth={1.5} />}>{t('session.project.edit')}</OptionContent></DropdownMenu.Item>,
      filing && <DropdownMenu.Item key="category" onClick={() => createCategory(cwd)}><OptionContent icon={<CategoryAddIcon />}>{t('session.category.new')}</OptionContent></DropdownMenu.Item>,
      onRevealPath && <DropdownMenu.Item key="reveal" onClick={() => onRevealPath(cwd)}><OptionContent icon={<FolderOpen strokeWidth={1.5} />}>{revealLabel()}</OptionContent></DropdownMenu.Item>,
      onCopyPath && <DropdownMenu.Item key="copy" title={cwd} onClick={() => onCopyPath(cwd)}><OptionContent icon={<Copy strokeWidth={1.5} />}>{t('session.project.copyPath')}</OptionContent></DropdownMenu.Item>,
    ].filter(Boolean);
    const remove = desktop && filing;
    if (!items.length && !remove) return undefined;
    const current = sectionOf(cwd);
    return <>
      {/* Desktop: "move to section", the sections ticked where the project is, then a new one or out of it */}
      {remove && <Submenu side="inline-end" icon={<Layers strokeWidth={1.5} />} label={t('session.section.moveTo')} value={current?.name}>{[
        ...(categoryState?.sections ?? []).map(x => (
          <DropdownMenu.Item key={x.id} onClick={() => onCategoryOp?.({ op: 'setProjectSection', cwd, section: x.id })}>
            <OptionContent checked={x.id === current?.id} checkSlot={!!current}>{x.name}</OptionContent>
          </DropdownMenu.Item>
        )),
        ...(categoryState?.sections?.length ? [<DropdownMenu.Separator key="sep" className="my-1 h-px bg-line" />] : []),
        <DropdownMenu.Item key="new" onClick={() => createSection(cwd)}><OptionContent icon={<Plus strokeWidth={1.5} />}>{t('session.section.newEllipsis')}</OptionContent></DropdownMenu.Item>,
        ...(current ? [<DropdownMenu.Item key="out" onClick={() => onCategoryOp?.({ op: 'setProjectSection', cwd })}><OptionContent icon={<Undo2 strokeWidth={1.5} />}>{t('session.section.remove')}</OptionContent></DropdownMenu.Item>] : []),
      ]}</Submenu>}
      {items}
      {remove && items.length > 0 && separator}
      {remove && <DropdownMenu.Item onClick={() => onCategoryOp?.({ op: 'removeProject', cwd })}><OptionContent icon={<X strokeWidth={1.5} />}>{t('session.project.remove')}</OptionContent></DropdownMenu.Item>}
    </>;
  };

  // `list`: the run the project sits in (a section's or the unsectioned ones), whose previous entry sets the spacing
  const renderProject = (p: ProjectNode, i: number, list: ProjectNode[]) => {
    const all = [...p.pinned, ...p.categories.flatMap(n => n.sessions), ...p.loose];
    // Preview the most-used agents across the folded project's sessions, including idle ones; off by default (Appearance `projectMarks`)
    const counts = new Map<AgentId, number>();
    if (!p.open && projectMarks) for (const s of all) counts.set(s.agent, (counts.get(s.agent) ?? 0) + 1);
    const marks = [...counts].sort((a, b) => b[1] - a[1]).slice(0, MORE_MARKS);
    // A session of another project dropped anywhere on the group (its categories keep their own zones) moves here: the
    // group is framed (`data-group`); a held project's insert line sits on its edge
    return (
      // Desktop: folded projects stack at the row pitch like a tree; only an open project's sessions leave room after them.
      // That room lives inside Fold (`pb-2`) instead of `mt-2` on the next project, so it animates on the panel's grid curve
      // rather than jumping 8 px in one frame and firing competing FlipRows slides.
      <div key={p.cwd} role="group" aria-label={nameOfProject(p.cwd)} data-drop={`loose:${p.cwd}`} data-group={p.cwd} data-project={desktop ? p.cwd : undefined}
        className={cn('relative flex flex-col rounded-lg', FRAME, LINE, i > 0 && !desktop && 'mt-2')}>
        <GroupHeader
          project
          name={nameOfProject(p.cwd)}
          title={p.cwd}
          folder={p.cwd}
          // Desktop: the sheet's header names the current project, and a project row carries no count
          badge={p.current && !desktop ? t('session.project.current') : undefined}
          count={desktop ? undefined : p.count}
          open={p.open}
          state={p.open ? undefined : aggregate(all)}
          preview={marks.map(([id]) => (
            <AgentMark key={id} id={id} name={nameOf(id)} className="size-3" />
          ))}
          fresh={fresh === p.cwd}
          onToggle={filing ? () => { if (!p.open) resetMore(p.cwd); onCategoryOp?.({ op: 'collapseProject', cwd: p.cwd, collapsed: p.open }); } : undefined}
          onNewCategory={filing && !desktop ? () => createCategory(p.cwd) : undefined}
          onNewSession={desktop && onNewIn ? () => onNewIn(p.cwd) : undefined}
          newSessionLabel={t('session.project.newSession')}
          menu={projectMenu(p.cwd)}
          // Desktop: a project drags into another section, out of one, or to another place in its own
          dragging={held?.kind === 'project' && held.id === p.cwd}
          onDragStart={desktop && filing && !filtering ? begin({ kind: 'project', id: p.cwd }) : undefined}
          onDragEnd={end}
        />
        {/* Desktop rows sit a step in (Item `nudge`), elsewhere the marks line up under the folder glyph; the heavier project name sets the group apart */}
        <Fold open={p.open} className={desktop && i < list.length - 1 ? 'pb-2' : undefined}>{renderBody(p)}</Fold>
      </div>
    );
  };

  const rest = flat ? flatGroups.length > 0 : unsectioned.some(p => p.pinned.length || p.loose.length || p.categories.length) || (desktop && unsectioned.length > 0);
  const empty = searching ? t('session.searching') : q ? t('session.noMatch') : filters ? t('session.view.noMatch') : scope === 'workspace' && workspace ? t('session.noneWorkspace') : t('session.none');

  // The panel layout's search field (the nav layout's is a NavSearch row)
  const searchInput = (
    <input
      autoFocus={autoFocus}
      value={query}
      onChange={e => setQuery(e.target.value)}
      placeholder={onSearch ? t('session.searchContent') : t('session.search')}
      aria-label={onSearch ? t('session.searchContent') : t('session.search')}
      className="min-w-0 flex-1 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3"
    />
  );
  const importer = onListNative && onImportNative && (
    <ImportSessions
      agents={agents} agentFilter={agentFilter} activeAgent={activeAgent}
      native={nativeSessions}
      onList={onListNative}
      onImport={onImportNative}
      onSelect={onSelect}
      row={nav}
    />
  );
  // Desktop: in the rows' geometry (an empty lead slot, then the text where titles start), not centred in the column
  const emptyRow = !shown.length && !visibleCategories && !(desktop && (rest || sectionNodes.length > 0)) && (desktop
    ? <NavEmpty>{empty}</NavEmpty>
    : <div className="mt-1 flex min-h-row items-center justify-center px-2 text-2 text-fg-3">{empty}</div>);
  // Grouped by date or channel: one quiet label per group (none when not grouped at all), its rows at the label's inset
  const flatLabel = (g: FlatGroup) => view.group === 'time' ? t(`session.when.${g.key as TimeBucket}`) : view.group === 'agent' ? nameOf(g.key) : '';
  const restRows = flat
    ? flatGroups.map((g, i) => (
      <div key={g.key || 'all'} role="group" aria-label={flatLabel(g) || undefined} className={cn('flex flex-col', i > 0 && 'mt-gap')}>
        {flatLabel(g) && <div className="flex min-h-row items-center gap-gap px-2 text-3 text-fg-3"><span className="min-w-0 truncate">{flatLabel(g)}</span></div>}
        {rowsOf(g.sessions)}
      </div>
    ))
    : tree.grouped ? unsectioned.map(renderProject) : tree.projects.map(p => <Fragment key={p.cwd}>{renderBody(p)}</Fragment>);
  // What the filters keep, named in one quiet line with its reset, so a filtered list never passes for the whole one
  const filterNames = [
    ...view.status.map(x => t(`session.view.status.${x}`)),
    ...view.agents.map(nameOf),
    ...view.since === 'all' ? [] : [t(`session.view.since.${view.since}`)],
  ];
  const summary = filters > 0 && (
    <div className="flex min-h-row shrink-0 items-center gap-gap px-2 text-3 text-fg-3">
      {nav && <span className="w-lead shrink-0" />}
      <span className="min-w-0 flex-1 truncate" title={filterNames.join(t('common.listSep'))}>{t('session.view.active', { list: filterNames.join(t('common.listSep')) })}</span>
      <button type="button" onClick={() => setView(clearFilters(view))} className="shrink-0 rounded-sm px-1 transition-colors hover:text-fg-1 focus-visible:text-fg-1">{t('session.view.reset')}</button>
    </div>
  );
  const viewMenu = (cls: string) => (
    <ViewMenu view={view} onChange={setView} agents={channels} desktop={desktop} pins={grouped} onMarkAllRead={onMarkAllRead} className={cls} />
  );
  const onEscape = (e: KeyboardEvent<HTMLDivElement>) => { if (e.key === 'Escape' && editing) { e.stopPropagation(); setEditing(undefined); } };
  const ids = (list: SessionSummary[]) => list.map(x => x.id).join();
  const layout = [
    layoutOf(tree, folded, more, desktop ? activeId : undefined), view.group, view.order, view.per,
    flatGroups.map(g => `${g.key}:${ids(g.sessions)}`).join(';'),
    sectionNodes.map(n => `${n.section.id}${n.section.collapsed ?? ''}:${n.projects.map(p => p.cwd).join()}`).join(';'),
  ].join('|');

  // Nav layout: the actions are rows of the column itself, and each section names itself in a quiet label. "New category"
  // has one home per scope: the section label when the list is the window's own project, the project rows when grouped.
  // Desktop: "new session" opens the start page, search and new session show their shortcuts, and the Projects label's
  // actions (add a folder, import an agent's own sessions, the filter) come out on hover
  if (nav) {
    const fresh = launchable(agents);
    const flip = (k: Section) => setFolded(prev => { const next = prev.includes(k) ? prev.filter(x => x !== k) : [...prev, k]; setWebviewState(FOLDED_KEY, next); return next; });
    // Desktop: the chats under their own label, which folds them and carries the pen for a new one; empty, the section is
    // that action's row. A search or filter that leaves no chat hides it
    const chatOpen = !!q || !folded.includes('chats');
    const chatSection = desktop && !!chats && !flat && (tree.chats.length > 0 || !filtering) && (
      <div className="flex flex-col" role="group" aria-label={t('session.group.chats')}>
        <SectionLabel open={chatOpen} onToggle={() => { if (!chatOpen) resetMore(CHATS); flip('chats'); }} quiet actions={onNewChat && (
          <button type="button" title={t('session.chat.new')} aria-label={t('session.chat.new')} onClick={onNewChat} className={SECTION_ACT}>
            <SquarePen className="size-icon" strokeWidth={1.5} />
          </button>
        )}>
          {t('session.group.chats')}
        </SectionLabel>
        <Fold open={chatOpen}>
          {tree.chats.length ? cutRows(CHATS, tree.chats) : <EmptyProjectRow flush label={t('session.chat.new')} onNew={onNewChat} />}
        </Fold>
      </div>
    );
    return (
      <div className="flex min-h-0 flex-1 flex-col" onKeyDown={onEscape}>
        <div className="flex shrink-0 flex-col">
          {desktop
            ? onNewIn && (
              <button type="button" data-land="agents" data-part="session-new" className={NAV_ROW} onClick={() => onNewIn()}>
                <span className={NAV_LEAD}><SquarePen className="size-icon" strokeWidth={1.5} /></span>
                <span className="min-w-0 flex-1 truncate">{t('session.new')}</span><NavHint>{shortcutLabel('N')}</NavHint>
              </button>
            )
            : onNew && fresh.length > 0 && (
              <DropdownMenu.Root>
                <DropdownMenu.Trigger render={<button type="button" data-land="agents" data-part="session-new" className={NAV_ROW}><span className={NAV_LEAD}><SquarePen className="size-icon" strokeWidth={1.5} /></span><span className="min-w-0 truncate">{t('session.new')}</span></button>} />
                <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="start" width="md"><DropdownMenu.Popup>
                  <div className="scroll-none flex max-h-pop flex-col overflow-y-auto">
                    {fresh.map(a => <DropdownMenu.Item key={a.id} disabled={a.available === false} title={a.available === false ? t('agent.notInstalled') : undefined} onClick={() => onNew(a.id)}>
                      <OptionContent icon={<AgentMark id={a.id} name={a.name} />}>{a.name}</OptionContent>
                    </DropdownMenu.Item>)}
                  </div>
                </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
              </DropdownMenu.Root>
            )}
          {/* Desktop: the row reads like the one above it until focused, its shortcut at the end */}
          <NavSearch part="session-search" value={query} onChange={setQuery} autoFocus={autoFocus} quiet={desktop} hint={desktop && shortcutLabel('K')}
            placeholder={desktop ? t('session.searchShort') : onSearch ? t('session.searchContent') : t('session.search')}
            label={onSearch ? t('session.searchContent') : t('session.search')} />
          {!desktop && importer}
        </div>
        <NavDivider />
        {/* The scroller bleeds through the panel's padding like the other layout's, so the thumb rides the panel edge. Its gutter
            stays reserved: folding a section must not take the scrollbar away and shift every right-aligned time and action.
            The right padding gives way to that gutter, so the rows keep equal margins on both sides. Desktop: the column runs
            to the window's foot, so the list fades out at its bottom edge, and under the divider on top once it scrolled (rows
            cut off by a hard edge there read as sliced). The first section label drops its mt-gap so the divider sits centred
            between the action rows above and the list below */}
        <div ref={listRoot} className={cn('scroll-thin scroll-stable -mx-column-pad flex min-h-0 flex-col overflow-y-auto pl-column-pad pr-list-gutter', desktop ? 'flex-1 pb-gap list-fade' : 'pb-1')}
          role="listbox" aria-label={t('session.listAria')}
          style={timeCh ? { '--session-time': `${timeCh}ch` } as CSSProperties : undefined}
          onDragOver={onDragOver} onDrop={onDrop}>
          <LazyPanelContext.Provider value>
            {summary}
            <FlipRows on={!q} layout={layout}>
              {tree.pinned.length > 0 && (
                <div className="flex flex-col" role="group" aria-label={t('session.group.pinned')}>
                  <SectionLabel first={!summary} open={!folded.includes('pinned')} onToggle={() => flip('pinned')}>{t('session.group.pinned')}</SectionLabel>
                  <Fold open={!folded.includes('pinned')}>{tree.pinned.map(s => renderItem(s))}</Fold>
                </div>
              )}
              {sectionNodes.map(({ section: x, projects: list }, i) => {
                const sectionMenu = <>
                  <DropdownMenu.Item onClick={() => setEditingSection(x.id)}><OptionContent icon={<Pencil strokeWidth={1.5} />}>{t('common.rename')}</OptionContent></DropdownMenu.Item>
                  {i > 0 && <DropdownMenu.Item onClick={() => onCategoryOp?.({ op: 'reorderSection', id: x.id, before: sections[sections.indexOf(x) - 1]!.id })}><OptionContent icon={<ArrowUp strokeWidth={1.5} />}>{t('session.section.moveUp')}</OptionContent></DropdownMenu.Item>}
                  {i < sectionNodes.length - 1 && <DropdownMenu.Item onClick={() => onCategoryOp?.({ op: 'reorderSection', id: x.id, ...(sections[sections.indexOf(x) + 2] ? { before: sections[sections.indexOf(x) + 2]!.id } : {}) })}><OptionContent icon={<ArrowDown strokeWidth={1.5} />}>{t('session.section.moveDown')}</OptionContent></DropdownMenu.Item>}
                  {separator}
                  <DropdownMenu.Item className="text-danger" onClick={() => onCategoryOp?.({ op: 'deleteSection', id: x.id })}>
                    <OptionContent icon={<Trash2 strokeWidth={1.5} />}>{t('session.section.delete')}</OptionContent>
                  </DropdownMenu.Item>
                </>;
                return (
                  <div key={x.id} className={cn('relative flex flex-col', LINE, held?.kind === 'section' && held.id === x.id && 'opacity-40')} role="group" aria-label={x.name} data-section={x.id}>
                    <SectionLabel
                      first={!summary && !tree.pinned.length && i === 0}
                      section={x.id}
                      open={!!q || !x.collapsed}
                      onToggle={() => onCategoryOp?.({ op: 'collapseSection', id: x.id, collapsed: !x.collapsed })}
                      quiet
                      rename={editingSection === x.id ? <RenameInput initial={x.name} label={t('common.rename')} onDone={name => { setEditingSection(undefined); if (name.trim() && name.trim() !== x.name) onCategoryOp?.({ op: 'renameSection', id: x.id, name }); }} /> : undefined}
                      onDragStart={!filtering && editingSection !== x.id && sectionNodes.length > 1 ? begin({ kind: 'section', id: x.id }) : undefined}
                      onDragEnd={end}
                      menu={sectionMenu}
                      actions={<SectionActions menu={sectionMenu} />}>
                      {x.name}
                    </SectionLabel>
                    {/* The section's projects are one zone: a project held in from elsewhere frames it */}
                    <Fold open={!!q || !x.collapsed}>
                      <div data-section-body={x.id} className={cn('flex flex-col rounded-lg', FRAME)}>
                        {list.length
                          ? list.map(renderProject)
                          : <div className="flex min-h-row items-center gap-gap px-2 text-3 text-fg-3"><span className="w-lead shrink-0" />{t('session.section.empty')}</div>}
                      </div>
                    </Fold>
                  </div>
                );
              })}
              <div className="contents" data-section={desktop ? REST : undefined}>
              <SectionLabel first={!summary && !tree.pinned.length && !sectionNodes.length} open={!folded.includes('rest')} onToggle={() => flip('rest')} quiet={desktop} keep={filters > 0 || (desktop && !tree.projects.length)} actions={<>
                {filing && workspace && !tree.grouped && (
                  <button type="button" title={t('session.category.new')} aria-label={t('session.category.new')} onClick={() => createCategory(workspace)} className={SECTION_ACT}>
                    <CategoryAddIcon />
                  </button>
                )}
                {/* Desktop: "+" adds a project folder (here or on an SSH host) or a section; with neither sections to edit nor
                    hosts, the folder button alone */}
                {desktop && onAddProject && (filing || remoteHosts
                  ? <DropdownMenu.Root>
                    <DropdownMenu.Trigger render={<button type="button" title={t('session.add')} aria-label={t('session.add')} className={SECTION_ACT}><Plus className="size-icon" strokeWidth={1.5} /></button>} />
                    <DropdownMenu.Portal><DropdownMenu.Positioner side="inline-end" align="start" width="sm"><DropdownMenu.Popup>
                      <DropdownMenu.Item onClick={onAddProject}><OptionContent icon={<FolderPlus strokeWidth={1.5} />}>{t('session.project.add')}</OptionContent></DropdownMenu.Item>
                      {remoteHosts && onAddRemote && <Submenu side="inline-end" icon={<RemoteFolderIcon />} label={t('session.project.remote')}>{[
                        ...remoteHosts.map(c => (
                          <DropdownMenu.Item key={c.alias} title={c.address} onClick={() => onAddRemote(c.alias)}>
                            <OptionContent icon={<Server strokeWidth={1.5} />}>{c.alias}</OptionContent>
                          </DropdownMenu.Item>
                        )),
                        ...(remoteHosts.length ? [<DropdownMenu.Separator key="sep" className="my-1 h-px bg-line" />] : []),
                        <DropdownMenu.Item key="host" onClick={() => onAddRemote()}><OptionContent icon={<Plus strokeWidth={1.5} />}>{t('session.project.addHost')}</OptionContent></DropdownMenu.Item>,
                      ]}</Submenu>}
                      {filing && <DropdownMenu.Item onClick={() => createSection()}><OptionContent icon={<Layers strokeWidth={1.5} />}>{t('session.section.new')}</OptionContent></DropdownMenu.Item>}
                    </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
                  </DropdownMenu.Root>
                  : <button type="button" title={t('session.project.add')} aria-label={t('session.project.add')} onClick={onAddProject} className={SECTION_ACT}>
                    <FolderPlus className="size-icon" strokeWidth={1.5} />
                  </button>)}
                {/* Desktop: importing an agent's own sessions sits with the folder and the filter (it left the start page's bar) */}
                {desktop && onListNative && onImportNative && (
                  <ImportSessions agents={agents} agentFilter={agentFilter} activeAgent={activeAgent} native={nativeSessions}
                    onList={onListNative} onImport={onImportNative} onSelect={onSelect} action />
                )}
                {viewMenu(SECTION_ACT)}
              </>}>
                {tree.grouped && !flat ? t('session.group.projects') : t('session.listAria')}
              </SectionLabel>
              <Fold open={!folded.includes('rest')}>
                <div data-section-body={REST} className={cn('flex flex-col rounded-lg', FRAME)}>
                  {emptyRow}
                  {rest && restRows}
                </div>
              </Fold>
              </div>
              {/* Desktop: the chats, sessions that belong to no project, under the projects as Codex lists them */}
              {chatSection}
            </FlipRows>
          </LazyPanelContext.Provider>
        </div>
      </div>
    );
  }

  return (
    <div className={cn('flex flex-col', fill ? 'min-h-0 flex-1' : 'max-h-[60vh]')} onKeyDown={onEscape}>
      <div className="flex items-center gap-1 px-1 pt-1">
        {/* No focus fill: the panel autofocuses the field, so a fill would sit there permanently as an off-centre box.
            Like the inline rename input, the caret alone marks it */}
        <label data-part="session-search" className="flex h-ctl min-w-0 flex-1 cursor-text items-center gap-2 px-2 text-fg-3">
          <Search className="size-icon shrink-0" strokeWidth={1.5} />
          {searchInput}
        </label>
        {/* One header row: new category and import sit beside the view menu, which ends the row. A second row for
            two icons read as a stray empty line */}
        {filing && workspace && (
          <IconButton title={t('session.category.new')} aria-label={t('session.category.new')} onClick={() => createCategory(workspace)}>
            <CategoryAddIcon />
          </IconButton>
        )}
        {importer}
        {viewMenu(ROW_ACT)}
      </div>
      {summary}
      {/* The scroller bleeds through its container's padding (docked panel px-column-pad, popover p-1) and pads it back, so the
          thumb rides the panel edge, the divider runs edge to edge and the rows stay where they were. The gutter stays
          reserved, so a search that ends the overflow does not shift the times */}
      <div ref={clockRoot} className={cn('scroll-thin scroll-stable mt-1 flex min-h-0 flex-col overflow-y-auto border-t border-line pb-1', fill ? '-mx-column-pad pl-column-pad pr-list-gutter' : '-mx-1 pl-1 pr-0')} role="listbox" aria-label={t('session.listAria')}
        style={timeCh ? { '--session-time': `${timeCh}ch` } as CSSProperties : undefined}
        onDragOver={onDragOver} onDrop={onDrop}>
        <LazyPanelContext.Provider value>
          <FlipRows on={!q} layout={layout}>
            {/* Empty state takes exactly one item row (pt-1 + min-h-row) so the popover keeps its height whether the filter matches 0 or 1 session */}
            {emptyRow}
            {tree.pinned.length > 0 && (
              <div className="flex flex-col pt-1" role="group" aria-label={t('session.group.pinned')}>
                {tree.pinned.map(s => renderItem(s))}
              </div>
            )}
            <PinDivider on={tree.pinned.length > 0 && !!rest} />
            {rest && <div className="flex flex-col pt-1">{restRows}</div>}
          </FlipRows>
        </LazyPanelContext.Provider>
      </div>
    </div>
  );
});

