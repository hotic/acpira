import { Component, createRef, Fragment, memo, useEffect, useMemo, useRef, useState, type CSSProperties, type DragEvent, type KeyboardEvent, type ReactNode, type RefObject } from 'react';
import { ArrowDown, ArrowUp, Check, ChevronDown, ChevronRight, Circle, Copy, Ellipsis, Folder, FolderInput, FolderOpen, FolderPlus, Import, Layers, ListFilter, LoaderCircle, Pencil, Pin, PinOff, Plus, Search, Server, SquarePen, Trash2, Undo2, X } from 'lucide-react';
import type { AgentId, AgentInfo, CategoryOp, NativeSessionInfo, SessionCategories, SessionCategory, SessionSection, SessionSummary } from '@acpira/shared/transcript';
import type { ExportFormat, NativeSessionsState, SessionHit } from '@acpira/shared/protocol';
import { inWorkspace, type SessionScope } from '@acpira/shared/settings';
import { launchable } from '@acpira/shared/agentOrder';
import { foreignBrand } from '@acpira/shared/models';
import { reuse } from '@acpira/shared/reuse';
import { cn } from '../ui/cn';
import { Collapsible, LazyPanelContext } from '../ui/Collapsible';
import { Command } from '../ui/Command';
import { DropdownMenu } from '../ui/DropdownMenu';
import { CARET_TURN, IconButton } from '../ui/Button';
import { OptionContent, PanelHeader } from '../ui/Panel';
import { useScrollFade } from '../ui/useScrollFade';
import { Popover } from '../ui/Popover';
import { Row } from '../ui/Row';
import { imeKey } from '../ui/ime';
import { dateFormat, t, useLocale } from '../i18n';
import { macOS, shortcutLabel, useDesktop, windowsOS } from '../desktop';
import { AgentMark } from './AgentMark';
import { CategoryAddIcon } from './CategoryIcon';
import { ModelMark } from './ModelMark';
import { RowContextMenu, SessionContextMenu, Submenu, type MoveProject, type SessionCopyKind, type SessionMenuItemsProps } from './SessionMenu';
import { setWebviewState, webviewState } from '../vscodeApi';
import { markParts, matchesTitle, searchTerms } from './sessionSearch';
import { buildSessionTree, categoryOf, draggable, dropAction, type CategoryNode, type MoveTarget, type ProjectNode, type SessionTree } from './sessionTree';
import { clearFilters, filterCount, groupSessions, passes, readView, sortSessions, VIEW_KEY, type FlatGroup, type ListView, type TimeBucket } from './sessionView';
import { ViewMenu } from './ViewMenu';
import { CONNECTION_STATE_NAME, useConnections, useFolderHost } from '../remote/api';
import { RemoteFolderIcon } from '../remote/RemoteFolderIcon';

// A running session shows a spinning ring (the one place a spinner is allowed: a list has no verb to shimmer); the other states are plain dots.
// Unread (a turn that finished unwatched) is a slightly larger dot in the strongest foreground with a halo rippling out of it;
// motion=none / reduced motion drop the halo and leave the dot lit. Not the accent: its default brand amber is indistinguishable from waiting's warn
const STATE_DOT: Record<Exclude<NonNullable<SessionSummary['state']>, 'working' | 'background'>, string> = {
  waiting: 'size-1.5 bg-warn waiting-pulse',
  unread: 'size-unread-dot bg-fg-strong unread-halo',
  error: 'size-1.5 bg-danger error-blink',
};

// Titles and agent names match locally on every key; the conversation search runs host-side once typing pauses
const SEARCH_DEBOUNCE = 180;

// Desktop: an open project lists its latest sessions, the rest behind one row (MoreRow), which previews this many agents
const LATEST = 5;
const MORE_MARKS = 3;
// Each click on the "more" row shows this many more, as Codex does; once all are out it reads "show less"
const MORE_PAGE = 10;

// Ring and dots share one fixed slot so every mark sits on the same centre line, and each names its meaning on hover.
// The ring is drawn smaller than the slot: at full size its arc outweighed even an 8 px dot
// Background (the turn ended, a task it started still runs) is the working ring at rest: a closed circle in the faint
// foreground with a small dot orbiting it slowly, so it reads as "alive, not done, nothing to do". A reply that came in
// meanwhile lights it in the strongest foreground with a dot inside, never the unread dot itself, which means finished
interface Mark { state: NonNullable<SessionSummary['state']>; unread?: boolean }
function StateMark({ state, unread }: Mark) {
  const label = t(state === 'background' && unread ? 'session.state.backgroundUnread' : `session.state.${state}`);
  return <span className="flex size-3 shrink-0 items-center justify-center" title={label} aria-label={label} role="img">
    {state === 'working'
      ? <LoaderCircle className="size-2.5 animate-spin live-spin text-fg-2" strokeWidth={2.5} />
      : state === 'background'
        ? <span className="relative flex size-2.5 items-center justify-center">
          <Circle className={cn('size-2.5', unread ? 'text-fg-strong' : 'text-fg-3')} strokeWidth={2.5} />
          <span className="bg-orbit absolute inset-px">
            <span className={cn('absolute top-0 left-1/2 size-0.75 -translate-1/2 rounded-full', unread ? 'bg-fg-strong' : 'bg-fg-2')} />
          </span>
          {unread && <span className="absolute size-1 rounded-full bg-fg-strong" />}
        </span>
        : <span className={cn('rounded-full', STATE_DOT[state])} />}
  </span>;
}

// Rows move and step in through WAAPI, which the stylesheet's kill switches (motion=none, reduced motion) do not reach
const still = (el: Element) => !!el.closest('[data-motion="none"]') || matchMedia('(prefers-reduced-motion: reduce)').matches;
const MOVE_EASE = 'cubic-bezier(.2,.7,.2,1)';
// The rows that move: sessions, project headers, the desktop's "more" row
const MOVING_ROWS = '[data-session], [data-project-row], [data-more]';

// A fold whose rows step in one after another as it opens, while the panel grows on Collapsible's curve. It wraps the
// panel rather than sitting in it, so it stays mounted while a lazy panel releases its rows
function FoldRows({ open, children }: { open: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const was = useRef(open);
  useEffect(() => {
    const el = ref.current;
    if (open && !was.current && el && !still(el)) {
      el.querySelectorAll<HTMLElement>(MOVING_ROWS).forEach((row, i) => {
        row.animate([{ opacity: 0, translate: '0 -4px' }, { opacity: 1, translate: '0 0' }],
          { duration: 220, delay: Math.min(i, 10) * 18, easing: MOVE_EASE, fill: 'backwards' });
      });
    }
    was.current = open;
  }, [open]);
  return <div ref={ref} className="flex flex-col" data-fold data-open={open || undefined}>{children}</div>;
}

// A fold of the list: the panel on Collapsible's curve (released once closed, LazyPanelContext) with its rows stepping in
function Fold({ open, children }: { open: boolean; children: ReactNode }) {
  return <FoldRows open={open}><Collapsible.Section open={open}><div className="flex flex-col">{children}</div></Collapsible.Section></FoldRows>;
}

// Rows that change place in a commit (a new session at the top of its project, one moved, pinned or unpinned, a project
// added) slide from where they were instead of jumping. Only a genuinely new row fades in: an existing row that changes
// group stays opaque, since fading what was already visible reads as the whole list blinking on an update. Positions
// are read right before React writes the DOM, so a slide still in flight restarts from where the row is on screen. Only a
// commit that changed `layout` measures: the shell re-renders the list on every stream chunk. Off while searching, where
// rows come and go with every key.
// After the commit the rows go through three passes that never interleave: end the slides in flight, read every row, start
// the new animations. Reading a row after an animation started (or a slide was cancelled) makes the browser restyle the
// document first, so cancelling, measuring and animating row by row cost a restyle per row: with ~500 rows listed that
// froze the column for most of a second on every new or deleted session (lab/session-list-perf). Only rows on screen
// before or after the commit animate; the others land where they belong out of sight
type FlipFirst = { tops: Map<string, number>; open: Set<Element> };
const flipKey = (el: HTMLElement) => el.dataset.session ?? (el.hasAttribute('data-project-row') ? `p:${el.title}` : `m:${el.closest('[role="group"]')?.getAttribute('aria-label')}`);
class FlipRows extends Component<{ on: boolean; layout: string; children: ReactNode }> {
  box = createRef<HTMLDivElement>();
  getSnapshotBeforeUpdate(prev: Readonly<{ on: boolean; layout: string }>): FlipFirst | null {
    const box = this.box.current;
    if (!prev.on || !this.props.on || prev.layout === this.props.layout || !box || still(box)) return null;
    const tops: FlipFirst['tops'] = new Map();
    for (const el of box.querySelectorAll<HTMLElement>(MOVING_ROWS)) tops.set(flipKey(el), el.getBoundingClientRect().top);
    return { tops, open: new Set(box.querySelectorAll('[data-fold][data-open]')) };
  }
  componentDidUpdate(_prev: unknown, _state: unknown, first: FlipFirst | null) {
    const box = this.box.current;
    if (!first || !box) return;
    // One lookup for the whole list: el.getAnimations() per row restyles again after every cancel
    for (const a of box.getAnimations({ subtree: true })) if (a.id === 'flip') a.cancel();
    // The visible band: the list's scroller (the listbox around this box) within the window
    const clip = box.closest('[role="listbox"]')?.getBoundingClientRect();
    const [above, below] = [Math.max(0, clip?.top ?? 0), Math.min(innerHeight, clip?.bottom ?? innerHeight)];
    const seen = (top: number, height: number) => top + height > above && top < below;
    const slides: [HTMLElement, number][] = [];
    const entries: HTMLElement[] = [];
    for (const el of box.querySelectorAll<HTMLElement>(MOVING_ROWS)) {
      const { top, height } = el.getBoundingClientRect();
      const was = first.tops.get(flipKey(el));
      if (was !== undefined) {
        if (Math.abs(was - top) > 0.5 && (seen(top, height) || seen(was, height))) slides.push([el, was - top]);
        continue;
      }
      // The rows of a fold opening in this commit are FoldRows'
      const fold = el.closest('[data-fold]');
      if (fold && !first.open.has(fold)) continue;
      if (seen(top, height)) entries.push(el);
    }
    for (const [el, dy] of slides) el.animate([{ translate: `0 ${dy}px` }, { translate: '0 0' }], { id: 'flip', duration: 260, easing: MOVE_EASE });
    for (const el of entries) el.animate([{ opacity: 0, translate: '0 -4px' }, { opacity: 1, translate: '0 0' }], { id: 'flip', duration: 220, easing: MOVE_EASE });
  }
  render() { return <div ref={this.box} className="contents">{this.props.children}</div>; }
}

// What decides where every row sits; FlipRows measures only when it changed
function layoutOf(tree: SessionTree, folded: string[], more: Map<string, number>, active?: string): string {
  const ids = (list: SessionSummary[]) => list.map(s => s.id).join();
  return [folded.join(), active, ids(tree.pinned), ...tree.projects.map(p =>
    `${p.cwd}>${p.open}${more.get(p.cwd) ?? 0}>${ids(p.pinned)}>${ids(p.loose)}>${p.categories.map(n => `${n.category.id}${n.open}:${ids(n.sessions)}`).join(';')}`)].join('|');
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
  // "New session in this category" (only offered for the window's own project, where a new session starts)
  onNewInCategory?: (category: string) => void;
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
  // A project header's menu: show the folder in the OS file manager, copy its path
  onRevealPath?: (path: string) => void;
  onCopyPath?: (path: string) => void;
  // A project just added: its header flashes once and its group scrolls into view
  fresh?: string;
  // The view menu's "mark all as read" (absent where the host does not offer it)
  onMarkAllRead?: () => void;
}

// The drag payload's type: only rows of this list carry it, so files or text dragged in are left alone
const DRAG_TYPE = 'application/x-acpira-session';
// The drop target's frame: a thin light outline drawn inset (never shifts layout) around the whole block, header and children
const FRAME = 'ring-1 ring-inset ring-fg-1/70';

type Held = { kind: 'session' | 'category'; id: string };
type Over =
  | { kind: 'category'; id: string; cwd: string }
  // A project's loose area; for a session of another project, anywhere on that project's group (header included)
  | { kind: 'loose'; cwd: string }
  // Reordering categories: the dragged one goes before `before` (absent: after the project's last)
  | { kind: 'gap'; cwd: string; before?: string };

const sameOver = (a?: Over, b?: Over) => JSON.stringify(a) === JSON.stringify(b);
const newCategoryId = () => `c-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
const newSectionId = () => `s-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

// What a collapsed group still tells about its sessions: the most urgent state inside. A finished reply outranks
// background work, which asks nothing of the user
const STATE_RANK: NonNullable<SessionSummary['state']>[] = ['working', 'waiting', 'error', 'unread', 'background'];
function aggregate(sessions: SessionSummary[]): Mark | undefined {
  const state = STATE_RANK.find(state => sessions.some(s => s.state === state));
  return state && { state, unread: state === 'background' && sessions.some(s => s.state === 'background' && s.unread) };
}

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
// plus "move here" for a session from another project — and the row's right-click menu holds the rest. Project and category
// headers open their "…" menu on right-click too.
// Sessions file by native HTML5 drag (the browser's own translucent drag image) into a category of their own project or back
// to its loose area; under "all", dropped on another project's group (or one of its categories) they move to that project.
// Pinned rows are locked and do not drag; category headers drag to reorder within their project.
// Every change moves: folds open on the panel curve with their rows stepping in (Fold), rows that change place slide
// (FlipRows). The desktop app (DesktopContext) lists its registered projects in their own order without categories, each
// open project its latest sessions with the rest behind one row.
// Deletion applies immediately, undo lives on the Toast at the shell's bottom
export const SessionList = memo(function SessionList({ sessions, agents, activeId, workspace, scope = 'all', autoFocus, fill, activeAgent, nativeSessions, onListNative, onImportNative, onSelect, onRename, onDelete, onPin, onMove, onExport, onCopy, onSearch, categories: categoryState, onFile, onCategoryOp, onNewInCategory, nav, onNew, onNewIn, onAddProject, onAddRemote, onRevealPath, onCopyPath, fresh, onMarkAllRead }: SessionListProps) {
  const locale = useLocale();
  const desktop = useDesktop();
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
  const [over, setOver] = useState<Over>();
  // Nav layout: the sections folded by their label, kept in the webview's state across reloads
  const [folded, setFolded] = useState<Section[]>(storedFolds);
  // Desktop: per project, how many sessions past the latest few are shown. A click on the "more" row adds a page; with
  // nothing left to show it folds them all away again
  const listFade = useScrollFade<HTMLDivElement>();
  const [more, setMore] = useState<Map<string, number>>(() => new Map());
  const stepMore = (cwd: string, total: number) => setMore(prev => {
    const next = new Map(prev);
    const shown = prev.get(cwd) ?? 0;
    if (shown >= total) next.delete(cwd);
    else next.set(cwd, shown + MORE_PAGE);
    return next;
  });
  const nameOf = (id: string) => agents.find(a => a.id === id)?.name ?? id;
  const here = (s: SessionSummary) => !workspace || inWorkspace(s, workspace);
  const filing = !!onFile && !!onCategoryOp;
  // The desktop app has no categories: a session filed in an IDE window reads as loose there
  const categories = filing && !desktop ? categoryState?.categories ?? NO_CATEGORIES : NO_CATEGORIES;

  const terms = useMemo(() => searchTerms(query), [query]);
  const q = terms.join(' ');
  // Content hits belong to the query they answered; a reply for an older query is ignored rather than mixed in
  const [hits, setHits] = useState<{ q: string; snippets: Map<string, string> }>();
  useEffect(() => {
    if (!onSearch || !q) return;
    let live = true;
    const timer = setTimeout(() => {
      void onSearch(q).then(list => { if (live) setHits({ q, snippets: new Map(list.map(h => [h.id, h.snippet])) }); });
    }, SEARCH_DEBOUNCE);
    return () => { live = false; clearTimeout(timer); };
  }, [q, onSearch]);
  const snippets = hits?.q === q ? hits.snippets : undefined;
  const searching = !!onSearch && !!q && !snippets;
  // The active session always stays listed (its row is the highlight), even when it belongs to another project
  const inScope = (s: SessionSummary) => scope === 'all' || here(s) || s.id === activeId;
  const at = new Date();
  const shown = sortSessions(sessions.filter(s => inScope(s) && passes(s, view, at)
    && (!q || matchesTitle(s.title, nameOf(s.agent), terms) || !!snippets?.has(s.id))), view.order, locale);

  const filtering = !!q || filters > 0;
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
  const registered = desktop ? categoryState?.projects : undefined;
  const removed = desktop ? categoryState?.removedProjects : undefined;
  // View menu → Show → Pinned: grouped by project, pinned rows lead their own project instead of one section on top
  const pinInProject = grouped && !flat && view.pinned === 'project';
  const tree = buildSessionTree({
    shown, categories, collapsedProjects: filing ? categoryState?.collapsedProjects ?? [] : [],
    workspace, grouped, filtering, expand: !!q, registered, removed, pinInProject,
  });
  const pinnedHere = (s: SessionSummary) => !!s.pinned && pinInProject;
  // Every project the list knows (the sessions' folders, the categories' own and the registered ones, not the removed
  // ones), the window's first, then the most recently used: the "move to project" destinations. Taken from all sessions,
  // not the filtered view, so a search does not hide one. Kept while equal: every push brings a new session list
  const projects = useReused(useMemo<MoveProject[]>(() => {
    const latest = new Map<string, string>();
    for (const s of sessions) if (s.cwd && (latest.get(s.cwd) ?? '') < s.updatedAt) latest.set(s.cwd, s.updatedAt);
    for (const c of categoryState?.categories ?? []) if (c.cwd && !latest.has(c.cwd)) latest.set(c.cwd, '');
    for (const cwd of registered ?? []) if (!latest.has(cwd)) latest.set(cwd, '');
    const cwds = [...latest.keys()].filter(c => !removed?.includes(c)).sort((a, b) => Number(b === workspace) - Number(a === workspace) || latest.get(b)!.localeCompare(latest.get(a)!));
    const names = cwds.map(projectName);
    return cwds.map((cwd, i) => {
      const name = names[i]!;
      const shared = names.indexOf(name) !== names.lastIndexOf(name);
      return { cwd, name, current: cwd === workspace, ...(shared ? { hint: cwd.slice(0, cwd.lastIndexOf(name)).replace(/[\\/]+$/, '') || cwd } : {}) };
    });
  }, [sessions, categoryState, registered, removed, workspace]));
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
  // [data-drop] ("category:<id>" / "loose:<cwd>") or, for a category being reordered, [data-category]
  const resolve = (h: Held, target: Element, y: number): Over | undefined => {
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
  const end = () => { setHeld(undefined); setOver(undefined); };
  const onDragOver = (e: DragEvent) => {
    if (!held || !e.dataTransfer.types.includes(DRAG_TYPE)) return;
    const next = resolve(held, e.target as Element, e.clientY);
    // Moving to another project needs the host's move; without it only same-project zones take the drop
    const ok = !!next && (next.kind === 'gap' || !heldSession || heldSession.cwd === next.cwd || !!onMove);
    if (ok) e.preventDefault();
    e.dataTransfer.dropEffect = ok ? 'move' : 'none';
    if (!sameOver(next, over)) setOver(next);
  };
  const onDrop = (e: DragEvent) => {
    if (!held) return;
    e.preventDefault();
    const zone = heldSession && over && over.kind !== 'gap' ? { cwd: over.cwd, ...(over.kind === 'category' ? { category: over.id } : {}) } : undefined;
    const action = heldSession && zone ? dropAction(heldSession, zone, categories) : undefined;
    if (action?.kind === 'file') onFile?.(heldSession!.id, action.category);
    if (action?.kind === 'move') onMove?.(heldSession!.id, action.to);
    if (held.kind === 'category' && over?.kind === 'gap') onCategoryOp?.({ op: 'reorder', id: held.id, ...(over.before ? { before: over.before } : {}) });
    end();
  };

  // What a row's controls do, read through the ref when they are used: rows hold the same object on every render, so a
  // row whose own props a push left alone skips its render (Item is memoized), where every listed row used to re-render
  const rowActions = useRef<RowActions>(null!);
  rowActions.current = {
    select: s => onSelect(s.id),
    edit: s => setEditing(s.id),
    rename: (s, title) => { setEditing(undefined); if (title.trim() && title.trim() !== s.title) onRename(s.id, title); },
    delete: s => { setEditing(undefined); onDelete(s.id); },
    pin: s => onPin(s.id, !s.pinned),
    move: (s, to) => onMove?.(s.id, to),
    export: (s, format) => onExport?.(s.id, format),
    copy: (s, what) => onCopy?.(s.id, what),
    file: (s, category) => onFile?.(s.id, category),
    newCategory: s => createCategory(s.cwd, s.id),
    dragStart: (s, e) => begin({ kind: 'session', id: s.id })(e),
    dragEnd: end,
  };
  const renderItem = (s: SessionSummary) => (
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
      project={flat ? (desktop || !here(s) ? projectName(s.cwd) : undefined) : desktop || here(s) || (grouped && (!s.pinned || pinnedHere(s))) ? undefined : projectName(s.cwd)}
      model={view.model ? s.model?.name : undefined}
      // Pinned inside its project, the row has no section saying so: a pin glyph before the time does
      pinMark={pinnedHere(s)}
      // Desktop: a project's sessions sit a step in, their agent mark under the project name, so the group reads as a level
      nudge={desktop && (!s.pinned || pinnedHere(s)) && !flat}
      editing={editing === s.id}
      dragging={held?.kind === 'session' && held.id === s.id}
      actions={rowActions}
      drag={filing && draggable(s) && (grouped || here(s)) && editing !== s.id}
      moveHere={!!onMove && !!workspace && !desktop && !s.external && !here(s)}
      moveTo={!!onMove && !s.external}
      exportable={!!onExport}
      copyable={!!onCopy}
      fileable={filing && !desktop && !s.external}
      projects={projectsBesides(s.cwd)}
      categories={categoriesIn(s.cwd)}
    />
  );

  // A category: a quiet sub-label and, open, its sessions at the same inset (one level of nesting is the project's alone).
  // The whole block is one drop zone and lights up as one; a collapsed category stays collapsed under a drag (no
  // spring-open), so its frame is just the header
  const renderCategory = (node: CategoryNode, siblings: CategoryNode[]) => {
    const c = node.category;
    const hot = over?.kind === 'category' && over.id === c.id;
    const gapBefore = over?.kind === 'gap' && over.before === c.id;
    const gapAfter = over?.kind === 'gap' && !over.before && over.cwd === c.cwd && siblings.at(-1) === node;
    const newHere = onNewInCategory && c.cwd === workspace ? () => onNewInCategory(c.id) : undefined;
    return (
      <div key={c.id} role="group" aria-label={c.name} data-category={c.id} data-drop={`category:${c.id}`}
        className={cn('relative flex flex-col rounded-lg transition-shadow', hot && FRAME)}>
        {gapBefore && <InsertLine edge="top" />}
        <GroupHeader
          sub
          name={c.name}
          count={node.sessions.length}
          open={node.open}
          state={node.open ? undefined : aggregate(node.sessions)}
          dragging={held?.kind === 'category' && held.id === c.id}
          editing={editingCategory === c.id}
          onDragStart={!filtering && editingCategory !== c.id ? begin({ kind: 'category', id: c.id }) : undefined}
          onDragEnd={end}
          onToggle={() => onCategoryOp?.({ op: 'collapse', id: c.id, collapsed: node.open })}
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
            ? node.sessions.map(renderItem)
            : <div className="flex min-h-row items-center px-2 text-3 text-fg-3">{t('session.category.empty')}</div>}
        </Fold>
        {gapAfter && <InsertLine edge="bottom" />}
      </div>
    );
  };

  // A project's body: its loose sessions (the zone that takes a session out of its category), then its categories. Rows share
  // one inset, so a category runs on to the next sub-label or project: loose sessions after the last one would read as its.
  // Desktop: the latest few sessions, then the active one when it is older than those, the rest folded behind one row; a
  // search lists every match, while the view menu's filters keep the cut (they only narrow what it cuts)
  const renderBody = (p: ProjectNode) => {
    // Only a session of this project lights the loose area; one from elsewhere frames the whole project group
    const looseHot = over?.kind === 'loose' && over.cwd === p.cwd && heldSession?.cwd === p.cwd;
    // A filed session of this project being dragged needs somewhere to land even when nothing is loose
    const strip = !p.loose.length && !!heldSession && heldSession.cwd === p.cwd && !!categoryOf(heldSession, categories);
    const cut = desktop && !q && view.per ? view.per : p.loose.length;
    // An active session past the cut joins the head on its own instead of stretching the head down to it
    const late = p.loose.slice(cut).find(s => s.id === activeId);
    const head = late ? [...p.loose.slice(0, cut), late] : p.loose.slice(0, cut);
    const rest = late ? p.loose.slice(cut).filter(s => s !== late) : p.loose.slice(cut);
    const shown = Math.min(more.get(p.cwd) ?? 0, rest.length);
    return <>
      {/* Pinned first, outside the "more" cut and the loose drop zone (pinned rows never drag) */}
      {p.pinned.map(renderItem)}
      {(p.loose.length > 0 || strip) && (
        <div data-drop={`loose:${p.cwd}`} className={cn('flex flex-col rounded-lg transition-shadow', looseHot && FRAME)}>
          {head.map(renderItem)}
          {rest.length > 0 && <>
            {/* One fold per page, so each click opens only the page it adds (a closed fold releases its rows) */}
            {pages(rest).map((page, i) => <Fold key={i} open={shown > i * MORE_PAGE}>{page.map(renderItem)}</Fold>)}
            <MoreRow open={shown >= rest.length} rest={rest.slice(shown)} nameOf={nameOf} onToggle={() => stepMore(p.cwd, rest.length)} />
          </>}
          {strip && <div className="flex min-h-row items-center px-2 text-3 text-fg-3">{t('session.category.remove')}</div>}
        </div>
      )}
      {p.categories.map(n => renderCategory(n, p.categories))}
      {desktop && !p.count && <EmptyProjectRow onNew={onNewIn ? () => onNewIn(p.cwd) : undefined} />}
    </>;
  };

  // A project header's menu, on its "…" and on right-click: show the folder, copy its path; an IDE window adds "new
  // category", the desktop app "remove from the list" for any folder, registered or listed for its sessions (they stay,
  // found by a search)
  const projectMenu = (cwd: string): ReactNode => {
    const items = [
      filing && !desktop && <DropdownMenu.Item key="category" onClick={() => createCategory(cwd)}><OptionContent icon={<CategoryAddIcon />}>{t('session.category.new')}</OptionContent></DropdownMenu.Item>,
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
    // A session of another project dropped anywhere on the group (its categories keep their own zones) moves here
    const incoming = !!onMove && !!heldSession && heldSession.cwd !== p.cwd && over?.kind === 'loose' && over.cwd === p.cwd;
    return (
      // Desktop: folded projects stack at the row pitch like a tree; only an open project's sessions leave room after them
      <div key={p.cwd} role="group" aria-label={projectName(p.cwd)} data-drop={`loose:${p.cwd}`}
        className={cn('flex flex-col rounded-lg transition-shadow', i > 0 && (!desktop || list[i - 1]?.open) && 'mt-2', incoming && FRAME)}>
        <GroupHeader
          project
          name={projectName(p.cwd)}
          title={p.cwd}
          folder={p.cwd}
          // Desktop: the sheet's header names the current project, and a project row carries no count
          badge={p.current && !desktop ? t('session.project.current') : undefined}
          count={desktop ? undefined : p.count}
          open={p.open}
          state={p.open ? undefined : aggregate(all)}
          fresh={fresh === p.cwd}
          onToggle={filing ? () => onCategoryOp?.({ op: 'collapseProject', cwd: p.cwd, collapsed: p.open }) : undefined}
          onNewCategory={filing && !desktop ? () => createCategory(p.cwd) : undefined}
          onNewSession={desktop && onNewIn ? () => onNewIn(p.cwd) : undefined}
          newSessionLabel={t('session.project.newSession')}
          menu={projectMenu(p.cwd)}
        />
        {/* Desktop rows sit a step in (Item `nudge`), elsewhere the marks line up under the folder glyph; the heavier project name sets the group apart */}
        <Fold open={p.open}>{renderBody(p)}</Fold>
      </div>
    );
  };

  const rest = flat ? flatGroups.length > 0 : unsectioned.some(p => p.pinned.length || p.loose.length || p.categories.length) || (desktop && unsectioned.length > 0);
  const empty = searching ? t('session.searching') : q ? t('session.noMatch') : filters ? t('session.view.noMatch') : scope === 'workspace' && workspace ? t('session.noneWorkspace') : t('session.none');

  const searchInput = (
    <input
      autoFocus={autoFocus}
      value={query}
      onChange={e => setQuery(e.target.value)}
      // Desktop: the row reads like the one above it until focused, its shortcut at the end
      placeholder={desktop ? t('session.searchShort') : onSearch ? t('session.searchContent') : t('session.search')}
      aria-label={onSearch ? t('session.searchContent') : t('session.search')}
      className={cn('min-w-0 flex-1 bg-transparent text-2 text-fg-1 outline-none', desktop ? 'placeholder:text-fg-1 focus:placeholder:text-fg-3' : 'placeholder:text-fg-3')}
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
    ? <div className="flex min-h-row items-center gap-gap px-2 text-3 text-fg-3"><span className="w-lead shrink-0" />{empty}</div>
    : <div className="mt-1 flex min-h-row items-center justify-center px-2 text-2 text-fg-3">{empty}</div>);
  // Grouped by date or channel: one quiet label per group (none when not grouped at all), its rows at the label's inset
  const flatLabel = (g: FlatGroup) => view.group === 'time' ? t(`session.when.${g.key as TimeBucket}`) : view.group === 'agent' ? nameOf(g.key) : '';
  const restRows = flat
    ? flatGroups.map((g, i) => (
      <div key={g.key || 'all'} role="group" aria-label={flatLabel(g) || undefined} className={cn('flex flex-col', i > 0 && 'mt-gap')}>
        {flatLabel(g) && <div className="flex min-h-row items-center gap-gap px-2 text-3 text-fg-3"><span className="min-w-0 truncate">{flatLabel(g)}</span></div>}
        {g.sessions.map(renderItem)}
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
    return (
      <div className="flex min-h-0 flex-1 flex-col" onKeyDown={onEscape}>
        <div className="flex shrink-0 flex-col">
          {desktop
            ? onNewIn && (
              <button type="button" className={NAV_ROW} onClick={() => onNewIn()}>
                <span className={NAV_LEAD}><SquarePen className="size-icon" strokeWidth={1.5} /></span>
                <span className="min-w-0 flex-1 truncate">{t('session.new')}</span><Hint>{shortcutLabel('N')}</Hint>
              </button>
            )
            : onNew && fresh.length > 0 && (
              <DropdownMenu.Root>
                <DropdownMenu.Trigger render={<button type="button" className={NAV_ROW}><span className={NAV_LEAD}><SquarePen className="size-icon" strokeWidth={1.5} /></span><span className="min-w-0 truncate">{t('session.new')}</span></button>} />
                <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="start" width="md"><DropdownMenu.Popup>
                  <div className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
                    {fresh.map(a => <DropdownMenu.Item key={a.id} disabled={a.available === false} title={a.available === false ? t('agent.notInstalled') : undefined} onClick={() => onNew(a.id)}>
                      <OptionContent icon={<AgentMark id={a.id} name={a.name} />}>{a.name}</OptionContent>
                    </DropdownMenu.Item>)}
                  </div>
                </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
              </DropdownMenu.Root>
            )}
          <label className={cn(NAV_ROW, 'cursor-text text-fg-3 focus-within:bg-hover')}>
            <span className={NAV_LEAD}><Search className="size-icon" strokeWidth={1.5} /></span>
            {searchInput}
            {desktop && <Hint>{shortcutLabel('K')}</Hint>}
          </label>
          {!desktop && importer}
        </div>
        {/* The scroller bleeds through the panel's padding like the other layout's, so the thumb rides the panel edge. Its gutter
            stays reserved: folding a section must not take the scrollbar away and shift every right-aligned time and action.
            The right padding gives way to that gutter, so the rows keep equal margins on both sides. Desktop: the column runs
            to the window's foot, so the list fades out at its bottom edge, and under the rows on top once it scrolled (rows
            cut off by a hard edge there read as sliced) */}
        <div ref={desktop ? listFade : undefined} className={cn('scroll-thin scroll-stable mt-gap-half -mx-gap-half flex min-h-0 flex-col overflow-y-auto pl-gap-half pr-list-gutter', desktop ? 'flex-1 pb-gap list-fade' : 'pb-1')}
          role="listbox" aria-label={t('session.listAria')}
          style={timeCh ? { '--session-time': `${timeCh}ch` } as CSSProperties : undefined}
          onDragOver={onDragOver} onDrop={onDrop}>
          <LazyPanelContext.Provider value>
            {summary}
            <FlipRows on={!q} layout={layout}>
              {tree.pinned.length > 0 && (
                <div className="flex flex-col" role="group" aria-label={t('session.group.pinned')}>
                  <SectionLabel open={!folded.includes('pinned')} onToggle={() => flip('pinned')}>{t('session.group.pinned')}</SectionLabel>
                  <Fold open={!folded.includes('pinned')}>{tree.pinned.map(renderItem)}</Fold>
                </div>
              )}
              {sectionNodes.map(({ section: x, projects: list }, i) => (
                <div key={x.id} className="flex flex-col" role="group" aria-label={x.name}>
                  <SectionLabel open={!!q || !x.collapsed} onToggle={() => onCategoryOp?.({ op: 'collapseSection', id: x.id, collapsed: !x.collapsed })} quiet
                    rename={editingSection === x.id ? <RenameInput initial={x.name} label={t('common.rename')} onDone={name => { setEditingSection(undefined); if (name.trim() && name.trim() !== x.name) onCategoryOp?.({ op: 'renameSection', id: x.id, name }); }} /> : undefined}
                    actions={<SectionActions
                      onRename={() => setEditingSection(x.id)}
                      onUp={i > 0 ? () => onCategoryOp?.({ op: 'reorderSection', id: x.id, before: sections[sections.indexOf(x) - 1]!.id }) : undefined}
                      onDown={i < sectionNodes.length - 1 ? () => onCategoryOp?.({ op: 'reorderSection', id: x.id, ...(sections[sections.indexOf(x) + 2] ? { before: sections[sections.indexOf(x) + 2]!.id } : {}) }) : undefined}
                      onDelete={() => onCategoryOp?.({ op: 'deleteSection', id: x.id })} />}>
                    {x.name}
                  </SectionLabel>
                  <Fold open={!!q || !x.collapsed}>
                    {list.length
                      ? list.map(renderProject)
                      : <div className="flex min-h-row items-center gap-gap px-2 text-3 text-fg-3"><span className="w-lead shrink-0" />{t('session.section.empty')}</div>}
                  </Fold>
                </div>
              ))}
              <SectionLabel open={!folded.includes('rest')} onToggle={() => flip('rest')} quiet={desktop} keep={filters > 0 || (desktop && !tree.projects.length)} actions={<>
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
                {emptyRow}
                {rest && restRows}
              </Fold>
            </FlipRows>
          </LazyPanelContext.Provider>
        </div>
      </div>
    );
  }

  return (
    <div className={cn('flex flex-col', fill ? 'min-h-0 flex-1' : 'max-h-[60vh]')} onKeyDown={onEscape}>
      <div className="flex items-center gap-1 px-1 pt-1">
        <label className="flex h-ctl min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-fg-3 focus-within:bg-hover">
          <Search className="size-icon shrink-0" strokeWidth={1.5} />
          {searchInput}
        </label>
        {/* The view menu ends the search row, so the row below is only there for what fills it */}
        {viewMenu(ROW_ACT)}
      </div>
      {((filing && workspace) || importer) && (
        <div className="flex min-w-0 shrink-0 items-center gap-1 px-1 pt-1">
          <span className="ml-auto flex shrink-0 items-center">
            {filing && workspace && (
              <IconButton title={t('session.category.new')} aria-label={t('session.category.new')} onClick={() => createCategory(workspace)}>
                <CategoryAddIcon />
              </IconButton>
            )}
            {importer}
          </span>
        </div>
      )}
      {summary}
      {/* The scroller bleeds through its container's padding (docked panel p-gap-half, popover p-1) and pads it back, so the
          thumb rides the panel edge, the divider runs edge to edge and the rows stay where they were. The gutter stays
          reserved, so a search that ends the overflow does not shift the times */}
      <div className={cn('scroll-thin scroll-stable mt-1 flex min-h-0 flex-col overflow-y-auto border-t border-line pb-1', fill ? '-mx-gap-half pl-gap-half pr-list-gutter' : '-mx-1 pl-1 pr-0')} role="listbox" aria-label={t('session.listAria')}
        style={timeCh ? { '--session-time': `${timeCh}ch` } as CSSProperties : undefined}
        onDragOver={onDragOver} onDrop={onDrop}>
        <LazyPanelContext.Provider value>
          <FlipRows on={!q} layout={layout}>
            {/* Empty state takes exactly one item row (pt-1 + min-h-row) so the popover keeps its height whether the filter matches 0 or 1 session */}
            {emptyRow}
            {tree.pinned.length > 0 && (
              <div className="flex flex-col pt-1" role="group" aria-label={t('session.group.pinned')}>
                {tree.pinned.map(renderItem)}
              </div>
            )}
            {/* Keep the divider mounted so both pinning and unpinning can transition. */}
            <div
              role="presentation"
              className={cn(
                'shrink-0 bg-linear-to-r from-transparent via-line to-transparent transition-[height,margin,opacity,scale] duration-(--dur-open) ease-out',
                tree.pinned.length > 0 && rest ? 'my-1 h-px scale-x-100 opacity-100' : 'my-0 h-0 scale-x-0 opacity-0',
              )}
            />
            {rest && <div className="flex flex-col pt-1">{restRows}</div>}
          </FlipRows>
        </LazyPanelContext.Provider>
      </div>
    </div>
  );
});

// The nav layout's action rows share the session rows' box (height, inset, radius, lead slot), so the column reads as one list
const NAV_ROW = 'flex min-h-row w-full items-center gap-gap rounded-md px-2 text-left text-2 text-fg-1 transition-colors hover:bg-hover focus-visible:bg-hover data-[popup-open]:bg-hover';
const NAV_LEAD = 'flex size-lead shrink-0 items-center justify-center text-fg-2';
const SECTION_ACT = 'inline-flex size-ctl-sm items-center justify-center rounded-md text-fg-3 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 data-[popup-open]:bg-hover data-[popup-open]:text-fg-1';
// The filter row's view button, an IconButton's box (ui/Button)
const ROW_ACT = 'inline-flex size-ctl shrink-0 items-center justify-center rounded-md text-fg-2 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 data-[popup-open]:bg-active data-[popup-open]:text-fg-1';
// A header's hover actions: they cross-fade in over its resting count / state, in the same cell, and take clicks only then
const REVEAL = 'pointer-events-none opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100 group-data-[menu-open]:pointer-events-auto group-data-[menu-open]:opacity-100';
// The concealed tail also stops taking the pointer: at opacity 0 it paints (and hit-tests) above the fully opaque actions,
// so once their fade ended the pointer fell through to it, the button lost its hover fill and a click selected the row
const CONCEAL = 'transition-opacity duration-150 group-hover:pointer-events-none group-hover:opacity-0 group-focus-within:pointer-events-none group-focus-within:opacity-0 group-data-[menu-open]:pointer-events-none group-data-[menu-open]:opacity-0';
// The resting tail and the hover actions stacked in one grid cell, so the title's room never changes under the pointer
const STACK = "grid [grid-template-areas:'x'] *:[grid-area:x]";

type Section = 'pinned' | 'rest';
const FOLDED_KEY = 'sessionSectionsFolded';
function storedFolds(): Section[] {
  const v = webviewState(FOLDED_KEY);
  return Array.isArray(v) ? v.filter((x): x is Section => x === 'pinned' || x === 'rest') : [];
}

// A section's quiet label in the nav layout (Pinned, Projects / Sessions), its actions at the end. The label folds the
// section and reads the same either way: no count, and the chevron (turned to the state) only on hover or focus, so
// folding never makes anything appear or vanish. The text starts where the rows' glyphs do, not at the rows' padding.
// `quiet` (desktop): the actions come out on hover or focus too, and stay while they hold a choice (`keep`: a channel
// filter is on) or one of them has its popup open
// `rename`: a section being renamed shows this input in place of its label
function SectionLabel({ children, actions, open, quiet, keep, rename, onToggle }: { children: ReactNode; actions?: ReactNode; open: boolean; quiet?: boolean; keep?: boolean; rename?: ReactNode; onToggle: () => void }) {
  return (
    <div className="group mt-gap flex min-h-row items-center gap-gap px-2 text-3 text-fg-3">
      {rename ?? <button type="button" aria-expanded={open} onClick={onToggle}
        className="inline-flex min-w-0 items-center gap-1 rounded-sm px-[calc((var(--spacing-lead)-var(--spacing-icon))/2)] transition-colors hover:text-fg-1 focus-visible:text-fg-1">
        <span className="min-w-0 truncate">{children}</span>
        <ChevronRight strokeWidth={1.5} className={cn('size-3 shrink-0 opacity-0 transition-[rotate,opacity] duration-(--dur-open) group-hover:opacity-100 group-focus-within:opacity-100',
          open && 'rotate-90')} />
      </button>}
      {actions && !rename && (
        <span className={cn('-mr-1 ml-auto flex shrink-0 items-center gap-0.5',
          quiet && !keep && 'opacity-0 transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100 group-has-[[data-popup-open]]:opacity-100')}>
          {actions}
        </span>
      )}
    </div>
  );
}

// A section label's "…": rename, move up / down, delete (its projects go back under Projects). Opens to the right of the
// column, over the chat, like every menu of the desktop column
function SectionActions({ onRename, onUp, onDown, onDelete }: { onRename: () => void; onUp?: () => void; onDown?: () => void; onDelete: () => void }) {
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger render={<button type="button" title={t('session.section.more')} aria-label={t('session.section.more')} className={SECTION_ACT}><Ellipsis className="size-icon" strokeWidth={1.5} /></button>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="inline-end" align="start" width="sm"><DropdownMenu.Popup finalFocus={false}>
        <DropdownMenu.Item onClick={onRename}><OptionContent icon={<Pencil strokeWidth={1.5} />}>{t('common.rename')}</OptionContent></DropdownMenu.Item>
        {onUp && <DropdownMenu.Item onClick={onUp}><OptionContent icon={<ArrowUp strokeWidth={1.5} />}>{t('session.section.moveUp')}</OptionContent></DropdownMenu.Item>}
        {onDown && <DropdownMenu.Item onClick={onDown}><OptionContent icon={<ArrowDown strokeWidth={1.5} />}>{t('session.section.moveDown')}</OptionContent></DropdownMenu.Item>}
        {separator}
        <DropdownMenu.Item className="text-danger" onClick={onDelete}>
          <OptionContent icon={<Trash2 strokeWidth={1.5} />} description={t('session.section.deleteHint')}>{t('session.section.delete')}</OptionContent>
        </DropdownMenu.Item>
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

// A keyboard shortcut at the end of a nav row
function Hint({ children }: { children: ReactNode }) {
  return <span className="shrink-0 text-3 text-fg-3 tabular-nums">{children}</span>;
}

// Where a reordered category will land: a short accent line on the block's edge, positioned absolutely so nothing shifts
function InsertLine({ edge }: { edge: 'top' | 'bottom' }) {
  return <div role="presentation" className={cn('pointer-events-none absolute inset-x-2 z-10 h-0.5 rounded-full bg-accent', edge === 'top' ? '-top-px' : '-bottom-px')} />;
}

// Desktop: an empty project's only row. With a way to start a session it is that action, a muted plus and "new session in
// this project" that brighten under the pointer; without one it only says the project has none
function EmptyProjectRow({ onNew }: { onNew?: () => void }) {
  const box = 'group flex min-h-row w-full shrink-0 items-center gap-gap rounded-md pr-2 pl-6 text-left text-3 text-fg-3';
  if (!onNew) return <div className={box}><span className="-mr-gap-half w-lead shrink-0" />{t('session.none')}</div>;
  return (
    <button type="button" onClick={onNew}
      className={cn(box, 'transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1')}>
      <span className="-mr-gap-half flex size-lead shrink-0 items-center justify-center opacity-60 transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100">
        <Plus strokeWidth={1.5} className="size-icon" />
      </span>
      <span className="min-w-0 truncate">{t('session.project.newSession')}</span>
    </button>
  );
}

const pages = <T,>(list: T[]) => Array.from({ length: Math.ceil(list.length / MORE_PAGE) }, (_, i) => list.slice(i * MORE_PAGE, (i + 1) * MORE_PAGE));

// Desktop: the row under a project's latest sessions. Folded it tells what the rest holds: how many, the agents among them
// (their marks) and the most urgent state inside, on the session rows' state column; `rest` is what is still hidden, a
// page at a time coming out above it. With all of them out (`open`), it reads "show less" at the
// list's end: the chevron turns over, the labels cross-fade and the preview fades away, so the row changes in place
function MoreRow({ open, rest, nameOf, onToggle }: { open: boolean; rest: SessionSummary[]; nameOf: (id: string) => string; onToggle: () => void }) {
  const marks = [...new Set(rest.map(s => s.agent))].slice(0, MORE_MARKS);
  const state = aggregate(rest);
  return (
    <button type="button" data-more aria-expanded={open} onClick={onToggle}
      className="group flex min-h-row w-full shrink-0 items-center gap-gap rounded-md pr-2 pl-6 text-left text-3 text-fg-3 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1">
      <span className="-mr-gap-half flex size-lead shrink-0 items-center justify-center">
        <ChevronDown strokeWidth={1.5} className={cn('size-icon transition-transform duration-(--dur-open) ease-out', open && 'rotate-180')} />
      </span>
      <span className={cn(STACK, 'min-w-0 *:truncate *:transition-opacity *:duration-150')}>
        <span className={cn(open && 'opacity-0')} aria-hidden={open || undefined}>{t('session.showMore', { n: rest.length })}</span>
        <span className={cn(!open && 'opacity-0')} aria-hidden={!open || undefined}>{t('session.showLess')}</span>
      </span>
      <span aria-hidden className={cn('flex shrink-0 items-center gap-1 transition-[opacity,translate] duration-200', open ? '-translate-x-1 opacity-0' : 'opacity-70 group-hover:opacity-100')}>
        {marks.map(id => <AgentMark key={id} id={id} name={nameOf(id)} className="size-3" />)}
      </span>
      {/* On the session rows' state column: a spacer as wide as their time follows the mark */}
      <span className={cn('ml-auto flex shrink-0 items-center gap-2 transition-opacity duration-150', open && 'opacity-0')}>
        {state && <StateMark {...state} />}
        <span className="min-w-session-time" />
      </span>
    </button>
  );
}

// Header of a project or a category. A project: the folder glyph (open or shut with the fold; hovered, it cross-fades into
// the chevron), the heavier name, the "current" badge, then its actions. A category (`sub`): a quiet label with no glyph,
// its chevron after the name. Hovered, the count / state cross-fade into the actions — "new category", "new session here"
// and the "…" menu, which right-click opens too. Collapsed, both still show the most urgent state inside. A project on an
// SSH host (`folder` is `ssh://…`) has the globe-badged folder and the host's name after its own; its connection shows
// only when it is off the usual: the glyph breathes while connecting and turns amber when the connection failed
function GroupHeader({ project, sub, name, title, folder, badge, count, open, state, fresh, dragging, editing, onDragStart, onDragEnd, onToggle, onRename, onNewSession, newSessionLabel = t('session.category.newSession'), onNewCategory, menu }: {
  project?: boolean;
  sub?: boolean;
  name: string;
  title?: string;
  // A project's folder, for the SSH host it may be on
  folder?: string;
  badge?: string;
  // Absent: no count (the desktop's project rows)
  count?: number;
  open: boolean;
  state?: Mark;
  // A project just added: the row flashes once and its group scrolls into view, clear of the list's faded bottom edge
  fresh?: boolean;
  dragging?: boolean;
  editing?: boolean;
  onDragStart?: (e: DragEvent) => void;
  onDragEnd?: () => void;
  onToggle?: () => void;
  onRename?: (name: string) => void;
  onNewSession?: () => void;
  newSessionLabel?: string;
  onNewCategory?: () => void;
  menu?: ReactNode;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const self = useRef<HTMLDivElement>(null);
  // Desktop: the "…" menu floats to the right of the column, over the chat, so a narrow column never squeezes it
  const desktop = useDesktop();
  useEffect(() => {
    const group = self.current?.closest<HTMLElement>('[role="group"]');
    if (!fresh || !group) return;
    group.style.scrollMarginBottom = 'calc(var(--row) * 1.5)';
    group.scrollIntoView({ block: 'nearest', behavior: still(group) ? 'auto' : 'smooth' });
  }, [fresh]);
  const act = 'inline-flex size-lead items-center justify-center rounded-sm text-fg-3 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1';
  const actions = !editing && (onNewSession || onNewCategory || menu);
  const host = useFolderHost(project ? folder : undefined);
  const hostState = host?.view?.state;
  // The glyph's tooltip names the host, and the connection when it is under way or failed (with the engine's words)
  const hostTitle = host && [
    t('connections.folderOn', { host: host.alias }),
    (hostState === 'connecting' || hostState === 'error') && t(CONNECTION_STATE_NAME[hostState]),
    hostState === 'error' && host.view?.error?.message,
  ].filter(Boolean).join(t('common.metaSep'));
  // The glyph a hover or an open menu hides in favour of the chevron. A pixel low: centred on the line box, the folder sat
  // high beside the bold name's lowercase letters (the chevron stays centred)
  const glyph = cn('translate-y-px', open ? 'opacity-100' : 'opacity-0', onToggle && 'group-hover:opacity-0 group-focus-visible:opacity-0 group-data-[menu-open]:opacity-0');
  const header = (
    <div
      ref={self}
      role="button"
      aria-expanded={open}
      tabIndex={0}
      title={title}
      data-menu-open={menuOpen || contextOpen || undefined}
      data-project-row={project || undefined}
      draggable={!!onDragStart}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onClick={() => { if (!editing) onToggle?.(); }}
      onKeyDown={e => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); onToggle?.(); } }}
      className={cn(
        'group flex min-h-row items-center gap-gap rounded-md px-2 text-2 transition-colors hover:bg-hover focus-visible:bg-hover data-[menu-open]:bg-hover',
        onToggle && 'cursor-pointer',
        project ? 'font-semibold text-fg-strong' : 'text-3 text-fg-3 hover:text-fg-1',
        // Renaming keeps the hover fill, as on a session row
        editing && 'bg-hover',
        dragging && 'opacity-40',
        fresh && 'row-flash',
      )}
    >
      {project && (
        // The folder shuts and opens in place with the fold; hovered, it cross-fades into the chevron, which turns with it
        <span title={hostTitle || undefined} className={cn('relative -mr-gap-half flex size-lead shrink-0 items-center justify-center text-fg-3 *:absolute *:size-icon *:transition-[opacity,rotate] *:duration-200', hostState === 'error' && 'text-warn')}>
          {host ? <>
            <RemoteFolderIcon pulse={hostState === 'connecting'} className={cn(glyph, open ? 'opacity-0' : 'opacity-100')} />
            <RemoteFolderIcon open pulse={hostState === 'connecting'} className={glyph} />
          </> : <>
            <Folder strokeWidth={1.5} className={cn(glyph, open ? 'opacity-0' : 'opacity-100')} />
            <FolderOpen strokeWidth={1.5} className={glyph} />
          </>}
          {onToggle && <ChevronRight strokeWidth={1.5} className={cn('opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100 group-data-[menu-open]:opacity-100', open && 'rotate-90')} />}
        </span>
      )}
      {editing && onRename
        ? <RenameInput initial={name} label={t('session.category.rename')} onDone={onRename} />
        : <span className="min-w-0 truncate">{name}</span>}
      {/* The host gives way first: it shrinks a thousand times faster than the name */}
      {host && !editing && <span title={host.alias} className="min-w-0 shrink-1000 truncate text-3 font-normal text-fg-3/70">{host.alias}</span>}
      {/* Like a section label's: shown only on hover or focus in either state */}
      {sub && !editing && (
        <ChevronRight strokeWidth={1.5} className={cn('-ml-1 size-3 shrink-0 opacity-0 transition-[rotate,opacity] duration-(--dur-open) group-hover:opacity-100 group-focus-visible:opacity-100 group-data-[menu-open]:opacity-100',
          open && 'rotate-90')} />
      )}
      {badge && !editing && <span className="shrink-0 rounded-sm bg-active px-1 text-3 font-normal text-fg-3">{badge}</span>}
      {!editing && (
        <span className={cn(STACK, 'ml-auto shrink-0 items-center text-3 font-normal text-fg-3 tabular-nums')}>
          <span className={cn('flex items-center justify-end gap-2', actions && CONCEAL)}>
            {state && <StateMark {...state} />}
            {count !== undefined && <span className="text-right">{count}</span>}
          </span>
          {actions && (
            <span className={cn('flex items-center justify-end gap-0.5', REVEAL)} onClick={e => e.stopPropagation()} onKeyDown={e => e.stopPropagation()}>
              {onNewCategory && (
                <button type="button" title={t('session.category.new')} aria-label={t('session.category.new')} onClick={onNewCategory} className={act}>
                  <CategoryAddIcon />
                </button>
              )}
              {onNewSession && (
                <button type="button" title={newSessionLabel} aria-label={newSessionLabel} onClick={onNewSession} className={act}>
                  {/* A notch under the ellipsis: the pen's whole box reads heavier than three dots */}
                  <SquarePen className="size-2.75" strokeWidth={1.5} />
                </button>
              )}
              {menu && (
                // Narrow: three short entries, and at the column's width a wider menu could not end at the button (collision shifted it)
                <DropdownMenu.Root onOpenLifecycle={setMenuOpen}>
                  <DropdownMenu.Trigger render={<button type="button" title={t('session.more')} aria-label={t('session.more')} className={act}><Ellipsis className="size-3" strokeWidth={1.5} /></button>} />
                  <DropdownMenu.Portal><DropdownMenu.Positioner side={desktop ? 'inline-end' : 'bottom'} align={desktop ? 'start' : 'end'} width={desktop ? 'md' : 'sm'}><DropdownMenu.Popup finalFocus={false}>{menu}</DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
                </DropdownMenu.Root>
              )}
            </span>
          )}
        </span>
      )}
    </div>
  );
  return menu ? <RowContextMenu items={menu} disabled={editing} onOpenChange={setContextOpen}>{header}</RowContextMenu> : header;
}

// Only the channel filter uses a picker; the session rows retain their own search,
// rename, pin and selection behavior. Adding ACPs never adds rows to the toolbar.
// `compact`: the nav layout's section-label action, a bare filter glyph until a channel is picked, then that channel's mark and name
function ChannelFilter({ agents, value, onChange, compact }: { agents: AgentInfo[]; value?: string; onChange: (id: string | undefined) => void; compact?: boolean }) {
  const [open, setOpen] = useState(false);
  const options = [{ id: '', name: t('session.channels') }, ...agents];
  const current = options.find(a => a.id === (value ?? '')) ?? { id: value!, name: value! };
  const searchable = agents.length >= 12;
  const select = (id: string) => { onChange(id || undefined); setOpen(false); };
  const mark = (a: { id: string; name: string }) => a.id
    ? <AgentMark id={a.id} name={a.name} />
    : <ListFilter className="size-icon" strokeWidth={1.5} />;
  const trigger = compact
    ? <button type="button" aria-label={`${t('session.filterChannel')}: ${current.name}`} title={value ? current.name : t('session.filterChannel')}
      className={cn('inline-flex h-ctl-sm min-w-ctl-sm max-w-full items-center justify-center gap-1.5 rounded-md text-3 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 data-[popup-open]:bg-hover', value ? 'px-1.5 text-fg-1' : 'text-fg-3')}>
      <span className="flex size-icon shrink-0 items-center justify-center">{mark(current)}</span>
      {value && <span className="min-w-0 truncate">{current.name}</span>}
    </button>
    : <button type="button" aria-label={`${t('session.filterChannel')}: ${current.name}`} title={current.name}
      className={cn('group/chip inline-flex h-[calc(var(--ctl)-4px)] min-w-0 max-w-full items-center gap-1.5 rounded-md px-2 text-3 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 data-[open]:bg-active data-[popup-open]:bg-active', value ? 'text-fg-1' : 'text-fg-2')}>
      <span className="flex size-icon shrink-0 items-center justify-center">{mark(current)}</span>
      <span className="min-w-0 truncate">{current.name}</span>
      <ChevronDown className={cn('size-3 shrink-0 text-fg-3', CARET_TURN)} strokeWidth={1.5} />
    </button>;
  const content = (a: { id: string; name: string }) => <OptionContent icon={mark(a)} checked={a.id === current.id} checkSlot>{a.name}</OptionContent>;

  return <div className="min-w-0 max-w-full" onKeyDownCapture={e => {
    // The filter may sit in a history popup or drawer. Consume its first Escape
    // before either the enclosing overlay or the drawer handles the same key.
    if (open && e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setOpen(false); }
  }}>
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger render={trigger} />
      {/* Its trigger always ends a row, so the list ends where the trigger does instead of running past the row's edge */}
      <Popover.Portal><Popover.Positioner width="md" align="end"><Popover.Popup aria-label={t('session.filterChannel')}>
        <Command.Root items={options} value={current} itemToStringValue={a => a.id}
          itemToStringLabel={a => a.name} isItemEqualToValue={(a, b) => a.id === b.id}>
          <Command.Input visible={searchable} placeholder={t('session.searchChannels')} aria-label={t('session.searchChannels')} />
          <Command.Empty />
          <Command.List searchable={searchable} aria-label={t('session.filterChannel')}>
            {(a: { id: string; name: string }) => <Command.Item key={a.id} value={a} title={a.name} onClick={() => select(a.id)}>{content(a)}</Command.Item>}
          </Command.List>
        </Command.Root>
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  </div>;
}

// The filter row's import entry: lists the chosen agent's own sessions over ACP (the host spawns a throwaway process for
// session/list — nothing is created on the agent's side). A session already imported is dimmed and jumps to its record;
// the rest import on click. The agent switcher in the header re-requests the list for the agent it lands on.
// `row`: the nav layout's full-width action row instead of the filter row's icon
export function ImportSessions({ agents, agentFilter, activeAgent, native, onList, onImport, onSelect, row, action }: {
  agents: AgentInfo[];
  agentFilter?: AgentId;
  activeAgent?: AgentId;
  native?: NativeSessionsState;
  onList: (agent: AgentId) => void;
  onImport: (agent: AgentId, s: NativeSessionInfo) => void;
  onSelect: (id: string) => void;
  row?: boolean;
  // Desktop: an icon among the Projects label's actions (add a folder, the filter), the agent it imports from in its title
  action?: boolean;
}) {
  const locale = useLocale();
  const [open, setOpen] = useState(false);
  // An explicit pick outlives the popover only while it is open; a fresh open follows the filter / active session again
  const [picked, setPicked] = useState<AgentId>();
  // Importing starts a record, so a switched-off agent is not offered even when the filter or the active session names it
  const candidates = launchable(agents);
  const offered = (id?: AgentId) => candidates.some(a => a.id === id) ? id : undefined;
  const agent = picked ?? offered(agentFilter) ?? offered(activeAgent) ?? candidates.find(a => a.available !== false)?.id ?? candidates[0]?.id;
  const name = agent ? (agents.find(a => a.id === agent)?.name ?? agent) : '';

  // onList is a stable postMessage wrapper; the request fires when the popover opens and whenever the chosen agent changes
  useEffect(() => { if (open && agent) onList(agent); }, [open, agent]);

  if (!candidates.length) return null;
  // Answers for another agent are stale the moment the switcher moved on; keep the loading row until this agent's list lands
  const mine = native?.agent === agent ? native : undefined;
  const now = new Date();
  const dayOf = (iso: string) => Math.floor((startOfDay(now) - startOfDay(new Date(iso))) / 86_400_000);

  return (
    <span className={row ? 'flex flex-col' : action ? 'flex' : 'ml-auto shrink-0'} onKeyDownCapture={e => {
      // Nested inside the history popover / drawer: consume the first Escape like ChannelFilter does
      if (open && e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setOpen(false); }
    }}>
      <Popover.Root open={open} onOpenChange={o => { setOpen(o); if (!o) setPicked(undefined); }}>
        <Popover.Trigger render={row
          ? <button type="button" className={NAV_ROW}><span className={NAV_LEAD}><Import className="size-icon" strokeWidth={1.5} /></span><span className="min-w-0 truncate">{t('session.import.action')}</span></button>
          : action
            ? <button type="button" className={SECTION_ACT} title={t('start.import', { agent: name })} aria-label={t('start.import', { agent: name })}><Import className="size-icon" strokeWidth={1.5} /></button>
            : <IconButton title={t('session.import.action')} aria-label={t('session.import.action')}><Import strokeWidth={1.5} /></IconButton>} />
        <Popover.Portal><Popover.Positioner width="xl" side="bottom" align="start"><Popover.Popup initialFocus={interaction => interaction === 'keyboard'} aria-label={t('session.import.action')}>
          <PanelHeader tail={<ChannelFilter agents={candidates} value={agent} onChange={id => setPicked(id || undefined)} />}>
            {t('session.import.title', { agent: name })}
          </PanelHeader>
          <div className="scroll-thin flex max-h-pop flex-col overflow-y-auto" role="listbox" aria-label={t('session.import.title', { agent: name })}>
            {(!mine || mine.loading) && <div className="flex min-h-row items-center justify-center px-2 text-2 text-fg-3">{t('session.import.loading')}</div>}
            {mine && !mine.loading && mine.error && <div className="flex min-h-row items-center justify-center px-2 text-2 text-fg-3">{mine.error}</div>}
            {mine && !mine.loading && !mine.error && !mine.sessions.length && <div className="flex min-h-row items-center justify-center px-2 text-2 text-fg-3">{t('session.import.none')}</div>}
            {mine && !mine.loading && !mine.error && mine.sessions.map(s => (
              <Row
                key={s.sessionId}
                as="button"
                interactive
                role="option"
                className={s.localId ? 'text-fg-3' : undefined}
                lead={<AgentMark id={agent!} name={name} />}
                trailing={s.localId
                  ? <span className="flex items-center gap-1"><Check className="size-3" strokeWidth={2} />{t('session.import.imported')}</span>
                  : s.updatedAt ? fmtTime(s.updatedAt, dayOf(s.updatedAt), locale) : undefined}
                onClick={() => {
                  if (s.localId) onSelect(s.localId);
                  else onImport(agent!, s);
                  setOpen(false);
                }}
              >
                <span className={cn('min-w-0 truncate', !s.title && 'font-mono text-fg-3')}>{s.title || s.sessionId.slice(0, 8)}</span>
              </Row>
            ))}
          </div>
        </Popover.Popup></Popover.Positioner></Popover.Portal>
      </Popover.Root>
    </span>
  );
}

interface ItemProps {
  session: SessionSummary;
  agentName: string;
  // The current search: its terms are marked in the title and snippet, the snippet being the conversation hit if any
  terms: string[];
  snippet?: string;
  active: boolean;
  time: string;
  // Folder name of the session's project when it is not this window's; shown faint before the time
  project?: string;
  // The view menu's "model at the row end": the session's model name, faint before the time
  model?: string;
  // A pinned session listed inside its project: a faint pin before the time
  pinMark?: boolean;
  // Desktop: the row sits a step in from its project's folder glyph
  nudge?: boolean;
  editing: boolean;
  // What the row's controls do; the flags below say which of them it offers. A ref and plain flags, so the memo holds
  // across pushes
  actions: RefObject<RowActions>;
  // Only for a session from another project: re-home it into this window's workspace
  moveHere?: boolean;
  // The menus' "move to project" submenu over `projects`, the other projects
  moveTo?: boolean;
  projects?: MoveProject[];
  exportable?: boolean;
  copyable?: boolean;
  // Native drag: the row may be filed by dragging (not pinned, not a ChatGPT mirror)
  drag?: boolean;
  dragging?: boolean;
  // The "move to category" submenu over `categories` (the session's own project's) and "new category…"
  fileable?: boolean;
  categories?: SessionCategory[];
}

// What a row's controls do: SessionList's handlers of the moment, read through a ref when a control is used
interface RowActions {
  select: (s: SessionSummary) => void;
  edit: (s: SessionSummary) => void;
  rename: (s: SessionSummary, title: string) => void;
  delete: (s: SessionSummary) => void;
  pin: (s: SessionSummary) => void;
  // No target: into this window's workspace ("move here")
  move: (s: SessionSummary, to?: MoveTarget) => void;
  export: (s: SessionSummary, format: ExportFormat) => void;
  copy: (s: SessionSummary, what: SessionCopyKind) => void;
  file: (s: SessionSummary, category: string | null) => void;
  newCategory: (s: SessionSummary) => void;
  dragStart: (s: SessionSummary, e: DragEvent) => void;
  dragEnd: () => void;
}

// One item: the whole row is clickable to select; the tail shows a status dot + time by default, cross-fading to the actions on hover / keyboard focus. Action buttons can't nest inside a button, so the whole row is a div[role=option].
// The hover cluster keeps the two quick toggles (move / pin, pin rightmost); rename, export and delete live in the row's
// context menu (right click, not while renaming, where the input keeps the native one). It keeps the cluster alive while
// open — the row tracks menuOpen because a pointer inside the portaled popup is no longer a hover.
// The agent mark rests a notch back from the title and comes forward with the row; when the session's model comes from
// another vendor than its agent (Devin on Claude, OpenCode on GPT), it turns over to that vendor's mark after a short hold
// (`mark-flip`, motion.css), so a pointer sweeping down the list flips nothing.
// Memoized: a push re-renders only the rows whose summary (kept by reference while unchanged, App's reuseById) or
// place-dependent props changed
const Item = memo(function Item({ session: s, agentName, terms, snippet, active, time, project, model, pinMark, nudge, editing, actions, moveHere, moveTo, projects, exportable, copyable, drag, dragging, fileable, categories }: ItemProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const select = () => actions.current.select(s);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); select(); }
  };
  const act = 'inline-flex size-lead items-center justify-center rounded-sm text-fg-3 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1';
  const menu: SessionMenuItemsProps = {
    session: s,
    onRename: () => actions.current.edit(s),
    onPin: () => actions.current.pin(s),
    onMove: moveHere ? () => actions.current.move(s) : undefined,
    projects,
    onMoveTo: moveTo ? cwd => actions.current.move(s, { cwd }) : undefined,
    onExport: exportable ? format => actions.current.export(s, format) : undefined,
    onCopy: copyable ? what => actions.current.copy(s, what) : undefined,
    onDelete: () => actions.current.delete(s),
    categories,
    onFile: fileable ? category => actions.current.file(s, category) : undefined,
    onNewCategory: fileable ? () => actions.current.newCategory(s) : undefined,
  };
  const foreign = foreignBrand(s.agent, s.model);
  const row = (
    <div
      role="option"
      aria-selected={active}
      tabIndex={0}
      data-session={s.id}
      data-menu-open={menuOpen || undefined}
      draggable={!!drag}
      onDragStart={drag ? e => actions.current.dragStart(s, e) : undefined}
      onDragEnd={() => actions.current.dragEnd()}
      onClick={() => { if (!editing) select(); }}
      onKeyDown={onKey}
      className={cn(
        // The row whose context menu is open keeps the hover fill: the popup under the pointer ends the row's :hover
        'group flex min-h-row cursor-pointer items-center gap-gap rounded-md px-2 text-2 text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover data-[menu-open]:bg-hover data-[menu-open]:text-fg-1',
        nudge && 'pl-6',
        snippet && !editing && 'py-1',
        // Renaming keeps the fill too: the menu that started it has closed and the focused input is not the row's focus-visible
        editing && 'bg-hover text-fg-1',
        active && 'bg-active text-fg-strong hover:bg-active data-[menu-open]:bg-active data-[menu-open]:text-fg-strong',
        dragging && 'opacity-40',
      )}
    >
      {/* The mark at 90 %: a filled vendor logo carries more ink than the outlined folder above it (picked over 100 / 86 % in
          lab/desk-files.preview, 2026-10-24) */}
      <span className="mark-flip -mr-gap-half flex size-lead shrink-0 scale-90 items-center justify-center text-fg-3 opacity-75 transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100 group-aria-selected:opacity-100 group-data-[menu-open]:opacity-100"
        title={s.model ? `${agentName}${t('common.metaSep')}${s.model.name}` : agentName}>
        {foreign
          ? <span className="mark-flip-card size-icon">
            <AgentMark id={s.agent} name={agentName} />
            <ModelMark family={s.model!.name} brand={foreign} />
          </span>
          : <AgentMark id={s.agent} name={agentName} />}
      </span>
      {editing
        ? <RenameInput initial={s.title} onDone={title => actions.current.rename(s, title)} />
        : (
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate"><Marked text={s.title} terms={terms} /></span>
            {snippet && <span className="truncate text-3 text-fg-3"><Marked text={snippet} terms={terms} /></span>}
          </span>
        )}
      {!editing && (
        <span className={cn(STACK, 'ml-auto shrink-0 items-center text-3 text-fg-3 tabular-nums')}>
          <span className={cn('flex items-center justify-end gap-2', CONCEAL)}>
            {project && <span className="max-w-project truncate text-fg-3/70" title={s.cwd}>{project}</span>}
            {model && <span className="max-w-project truncate text-fg-3/70">{model}</span>}
            {pinMark && <Pin aria-label={t('session.group.pinned')} className="size-3 shrink-0 text-fg-3/70" strokeWidth={1.5} />}
            {s.state && <StateMark state={s.state} unread={s.unread} />}
            <span className="min-w-session-time text-right">{time}</span>
          </span>
          <span className={cn('flex items-center justify-end gap-0.5', REVEAL)}>
            {moveHere && (
              <button type="button" title={t('session.move')} aria-label={t('session.move')} onClick={e => { e.stopPropagation(); actions.current.move(s); }} className={act}>
                <FolderInput className="size-3" strokeWidth={1.5} />
              </button>
            )}
            <button type="button" title={s.pinned ? t('common.unpin') : t('common.pin')} aria-label={s.pinned ? t('common.unpin') : t('common.pin')} onClick={e => { e.stopPropagation(); actions.current.pin(s); }} className={act}>
              {s.pinned ? <PinOff className="size-3" strokeWidth={1.5} /> : <Pin className="size-3" strokeWidth={1.5} />}
            </button>
          </span>
        </span>
      )}
    </div>
  );
  return <SessionContextMenu {...menu} editing={editing} onOpenChange={setMenuOpen}>{row}</SessionContextMenu>;
});

// Marks every occurrence of the search terms in brighter, heavier text
function Marked({ text, terms }: { text: string; terms: string[] }) {
  if (!terms.length) return <>{text}</>;
  return <>{markParts(text, terms).map((part, i) => i % 2 ? <mark key={i} className="bg-transparent font-medium text-fg-strong">{part}</mark> : part)}</>;
}

// Inline rename: ⏎ commits, Esc cancels, blur commits; the callback fires only once (the blur right after Esc doesn't count)
function RenameInput({ initial, label = t('session.titleAria'), onDone }: { initial: string; label?: string; onDone: (title: string) => void }) {
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  const [value, setValue] = useState(initial);
  useEffect(() => { ref.current?.focus(); ref.current?.select(); }, []);
  const finish = (v: string) => { if (done.current) return; done.current = true; onDone(v); };
  return (
    <input
      ref={ref}
      value={value}
      onChange={e => setValue(e.target.value)}
      onClick={e => e.stopPropagation()}
      onKeyDown={e => {
        e.stopPropagation();
        if (imeKey(e.nativeEvent)) return;
        if (e.key === 'Enter') finish(value);
        if (e.key === 'Escape') finish(initial);
      }}
      onBlur={() => finish(value)}
      aria-label={label}
      className="min-w-0 flex-1 bg-transparent px-0 text-2 text-fg-1 outline-none"
    />
  );
}

function startOfDay(d: Date) { return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); }

// The last path segment of a session's cwd (posix or Windows); a bare root or home shows as the path itself
export function projectName(cwd: string): string {
  const parts = cwd.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? cwd;
}

// An upper bound of a formatted time's width in ch: tabular digits are 1ch, separators and spaces narrower, letters (AM / PM, CJK
// date suffixes) wider; rounded up to a tenth so the column never clips its longest entry
function timeWidth(text: string): number {
  let w = 0;
  for (const c of text) w += /\d/.test(c) ? 1 : /[\s:/.,-]/.test(c) ? 0.6 : /[\u3000-\u9fff]/.test(c) ? 2 : 1.6;
  return Math.ceil(w * 10) / 10;
}

// Today shows the time of day, earlier shows month/day — both follow the UI locale
function fmtTime(iso: string, dayAgo: number, locale: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  if (dayAgo <= 0) return dateFormat(locale, { hour: 'numeric', minute: '2-digit' }).format(d);
  return dateFormat(locale, { month: 'numeric', day: 'numeric' }).format(d);
}
