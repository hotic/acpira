import { Fragment, useEffect, useMemo, useRef, useState, type CSSProperties, type DragEvent, type KeyboardEvent, type ReactNode } from 'react';
import { Check, ChevronDown, ChevronRight, Ellipsis, Folder, FolderInput, FolderOpen, Import, Inbox, ListFilter, LoaderCircle, Pencil, Pin, PinOff, Search, SquarePen, Trash2 } from 'lucide-react';
import type { AgentId, AgentInfo, CategoryOp, NativeSessionInfo, SessionCategories, SessionCategory, SessionSummary } from '@shared/transcript';
import type { NativeSessionsState, SessionHit } from '@shared/protocol';
import { inWorkspace, type SessionScope } from '@shared/settings';
import { launchable } from '@shared/agentOrder';
import { cn } from '../ui/cn';
import { Command } from '../ui/Command';
import { DropdownMenu } from '../ui/DropdownMenu';
import { IconButton } from '../ui/Button';
import { OptionContent, PanelHeader } from '../ui/Panel';
import { Popover } from '../ui/Popover';
import { Row } from '../ui/Row';
import { t, useLocale } from '../i18n';
import { AgentMark } from './AgentMark';
import { CategoryAddIcon } from './CategoryIcon';
import { SessionContextMenu, SessionMenu, type MoveProject, type SessionMenuItemsProps } from './SessionMenu';
import { markParts, matchesTitle, searchTerms } from './sessionSearch';
import { buildSessionTree, categoryOf, draggable, dropAction, type CategoryNode, type MoveTarget, type ProjectNode } from './sessionTree';

// A running session shows a spinning ring (the one place a spinner is allowed: a list has no verb to shimmer); the other states are plain dots.
// Unread (a turn that finished unwatched) is a slightly larger dot in the strongest foreground with a halo rippling out of it;
// motion=none / reduced motion drop the halo and leave the dot lit. Not the accent: its default brand amber is indistinguishable from waiting's warn
const STATE_DOT: Record<Exclude<NonNullable<SessionSummary['state']>, 'working'>, string> = {
  waiting: 'size-1.5 bg-warn',
  unread: 'size-unread-dot bg-fg-strong unread-halo',
  error: 'size-1.5 bg-danger',
};

// Titles and agent names match locally on every key; the conversation search runs host-side once typing pauses
const SEARCH_DEBOUNCE = 180;

// Ring and dots share one fixed slot so every mark sits on the same centre line, and each names its meaning on hover.
// The ring is drawn smaller than the slot: at full size its arc outweighed even an 8 px dot
function StateMark({ state }: { state: NonNullable<SessionSummary['state']> }) {
  const label = t(`session.state.${state}`);
  return <span className="flex size-3 shrink-0 items-center justify-center" title={label} aria-label={label} role="img">
    {state === 'working'
      ? <LoaderCircle className="size-2.5 animate-spin live-spin text-fg-2" strokeWidth={2.5} />
      : <span className={cn('rounded-full', STATE_DOT[state])} />}
  </span>;
}

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
  onExport?: (id: string, format: 'markdown' | 'json') => void;
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

// What a collapsed group still tells about its sessions: the most urgent state inside
const STATE_RANK: NonNullable<SessionSummary['state']>[] = ['working', 'waiting', 'error', 'unread'];
function aggregate(sessions: SessionSummary[]): SessionSummary['state'] {
  return STATE_RANK.find(state => sessions.some(s => s.state === state));
}

// Session list: search and agent filters stay visible even without history; pinned sessions get their own section on top.
// Below them the sessions are a tree (sessionTree.ts): under the workspace scope the window's project, its user categories
// then its loose sessions; under "all" one collapsible group per project (folder glyph), each holding its categories (inbox
// glyph) and loose sessions. Each item: vendor mark · title · time; on hover those swap for the actions — pin, a "…" menu
// (rename / move to category / export / delete), plus "move here" for a session from another project.
// Sessions file by native HTML5 drag (the browser's own translucent drag image) into a category of their own project or back
// to its loose area; under "all", dropped on another project's group (or one of its categories) they move to that project.
// Pinned rows are locked and do not drag; category headers drag to reorder within their project.
// Deletion applies immediately, undo lives on the Toast at the shell's bottom
export function SessionList({ sessions, agents, activeId, workspace, scope = 'all', autoFocus, fill, activeAgent, nativeSessions, onListNative, onImportNative, onSelect, onRename, onDelete, onPin, onMove, onExport, onSearch, categories: categoryState, onFile, onCategoryOp, onNewInCategory }: SessionListProps) {
  const locale = useLocale();
  const [query, setQuery] = useState('');
  const [agentFilter, setAgentFilter] = useState<string>();
  const [editing, setEditing] = useState<string>();
  // A category being renamed; set right after creating one, before the host has pushed it back
  const [editingCategory, setEditingCategory] = useState<string>();
  const [held, setHeld] = useState<Held>();
  const [over, setOver] = useState<Over>();
  const nameOf = (id: string) => agents.find(a => a.id === id)?.name ?? id;
  const here = (s: SessionSummary) => !workspace || inWorkspace(s, workspace);
  const filing = !!onFile && !!onCategoryOp;
  const categories = filing ? categoryState?.categories ?? [] : [];

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
  const shown = sessions.filter(s => inScope(s) && (!agentFilter || s.agent === agentFilter)
    && (!q || matchesTitle(s.title, nameOf(s.agent), terms) || !!snippets?.has(s.id)));

  const filtering = !!q || !!agentFilter;
  const grouped = scope === 'all' && !!workspace;
  const tree = buildSessionTree({
    shown, categories, collapsedProjects: filing ? categoryState?.collapsedProjects ?? [] : [],
    workspace, grouped, filtering,
  });
  // Every project the list knows (the sessions' folders and the categories' own), the window's first, then the most recently
  // used: the "move to project" destinations. Taken from all sessions, not the filtered view, so a search does not hide one
  const projects = useMemo<MoveProject[]>(() => {
    const latest = new Map<string, string>();
    for (const s of sessions) if (s.cwd && (latest.get(s.cwd) ?? '') < s.updatedAt) latest.set(s.cwd, s.updatedAt);
    for (const c of categoryState?.categories ?? []) if (c.cwd && !latest.has(c.cwd)) latest.set(c.cwd, '');
    const cwds = [...latest.keys()].sort((a, b) => Number(b === workspace) - Number(a === workspace) || latest.get(b)!.localeCompare(latest.get(a)!));
    const names = cwds.map(projectName);
    return cwds.map((cwd, i) => {
      const name = names[i]!;
      const shared = names.indexOf(name) !== names.lastIndexOf(name);
      return { cwd, name, current: cwd === workspace, ...(shared ? { hint: cwd.slice(0, cwd.lastIndexOf(name)).replace(/[\\/]+$/, '') || cwd } : {}) };
    });
  }, [sessions, categoryState, workspace]);
  // Under the workspace scope only the window's own categories show (a foreign active session stays loose)
  const visibleCategories = tree.projects.some(p => p.categories.length > 0);

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

  const renderItem = (s: SessionSummary) => (
    <Item
      key={s.id}
      session={s}
      agentName={nameOf(s.agent)}
      terms={terms}
      snippet={snippets?.get(s.id)}
      active={s.id === activeId}
      time={times.get(s.id) ?? ''}
      // Grouped, a row's project is its group, so only pinned rows (which sit above every group) name theirs
      project={here(s) || (grouped && !s.pinned) ? undefined : projectName(s.cwd)}
      editing={editing === s.id}
      dragging={held?.kind === 'session' && held.id === s.id}
      onDragStart={filing && draggable(s) && (grouped || here(s)) && editing !== s.id ? begin({ kind: 'session', id: s.id }) : undefined}
      onDragEnd={end}
      onSelect={() => onSelect(s.id)}
      onEdit={() => setEditing(s.id)}
      onRename={t => { setEditing(undefined); if (t.trim() && t.trim() !== s.title) onRename(s.id, t); }}
      onDelete={() => { setEditing(undefined); onDelete(s.id); }}
      onPin={() => onPin(s.id, !s.pinned)}
      onMove={onMove && workspace && !s.external && !here(s) ? () => onMove(s.id) : undefined}
      projects={projects.filter(p => p.cwd !== s.cwd)}
      onMoveTo={onMove && !s.external ? cwd => onMove(s.id, { cwd }) : undefined}
      onExport={onExport ? format => onExport(s.id, format) : undefined}
      categories={categories.filter(c => c.cwd === s.cwd)}
      onFile={filing && !s.external ? category => onFile!(s.id, category) : undefined}
      onNewCategory={filing && !s.external ? () => createCategory(s.cwd, s.id) : undefined}
    />
  );

  // A category: its header and, open, its sessions indented under it. The whole block is one drop zone and lights up as one;
  // a collapsed category stays collapsed under a drag (no spring-open), so its frame is just the header
  const renderCategory = (node: CategoryNode, siblings: CategoryNode[]) => {
    const c = node.category;
    const hot = over?.kind === 'category' && over.id === c.id;
    const gapBefore = over?.kind === 'gap' && over.before === c.id;
    const gapAfter = over?.kind === 'gap' && !over.before && over.cwd === c.cwd && siblings.at(-1) === node;
    return (
      <div key={c.id} role="group" aria-label={c.name} data-category={c.id} data-drop={`category:${c.id}`}
        className={cn('relative flex flex-col rounded-lg transition-shadow', hot && FRAME)}>
        {gapBefore && <InsertLine edge="top" />}
        <GroupHeader
          icon={<Inbox className="size-icon" strokeWidth={1.5} />}
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
          onNewSession={onNewInCategory && c.cwd === workspace ? () => onNewInCategory(c.id) : undefined}
          menu={<>
            <DropdownMenu.Item onClick={() => setEditingCategory(c.id)}><OptionContent icon={<Pencil strokeWidth={1.5} />}>{t('session.category.rename')}</OptionContent></DropdownMenu.Item>
            {onNewInCategory && c.cwd === workspace && <DropdownMenu.Item onClick={() => onNewInCategory(c.id)}><OptionContent icon={<SquarePen strokeWidth={1.5} />}>{t('session.category.newSession')}</OptionContent></DropdownMenu.Item>}
            <DropdownMenu.Separator className="my-1 h-px bg-line" />
            <DropdownMenu.Item className="text-danger" onClick={() => onCategoryOp?.({ op: 'delete', id: c.id })}>
              <OptionContent icon={<Trash2 strokeWidth={1.5} />}>{t('session.category.delete')}</OptionContent>
            </DropdownMenu.Item>
          </>}
        />
        {node.open && (
          <div className="ml-indent flex flex-col">
            {node.sessions.length
              ? node.sessions.map(renderItem)
              : <div className="flex min-h-row items-center px-2 text-3 text-fg-3">{t('session.category.empty')}</div>}
          </div>
        )}
        {gapAfter && <InsertLine edge="bottom" />}
      </div>
    );
  };

  // A project's body: its categories, then its loose sessions (the zone that takes a session out of its category)
  const renderBody = (p: ProjectNode) => {
    // Only a session of this project lights the loose area; one from elsewhere frames the whole project group
    const looseHot = over?.kind === 'loose' && over.cwd === p.cwd && heldSession?.cwd === p.cwd;
    // A filed session of this project being dragged needs somewhere to land even when nothing is loose
    const strip = !p.loose.length && !!heldSession && heldSession.cwd === p.cwd && !!categoryOf(heldSession, categories);
    return <>
      {p.categories.map(n => renderCategory(n, p.categories))}
      {(p.loose.length > 0 || strip) && (
        <div data-drop={`loose:${p.cwd}`} className={cn('flex flex-col rounded-lg transition-shadow', looseHot && FRAME)}>
          {p.loose.map(renderItem)}
          {strip && <div className="flex min-h-row items-center px-2 text-3 text-fg-3">{t('session.category.remove')}</div>}
        </div>
      )}
    </>;
  };

  const renderProject = (p: ProjectNode, i: number) => {
    const all = [...p.categories.flatMap(n => n.sessions), ...p.loose];
    // A session of another project dropped anywhere on the group (its categories keep their own zones) moves here
    const incoming = !!onMove && !!heldSession && heldSession.cwd !== p.cwd && over?.kind === 'loose' && over.cwd === p.cwd;
    return (
      <div key={p.cwd} role="group" aria-label={projectName(p.cwd)} data-drop={`loose:${p.cwd}`}
        className={cn('flex flex-col rounded-lg transition-shadow', i > 0 && 'mt-2', incoming && FRAME)}>
        <GroupHeader
          project
          icon={p.open ? <FolderOpen className="size-icon" strokeWidth={1.5} /> : <Folder className="size-icon" strokeWidth={1.5} />}
          name={projectName(p.cwd)}
          title={p.cwd}
          badge={p.current ? t('session.project.current') : undefined}
          count={p.count}
          open={p.open}
          state={p.open ? undefined : aggregate(all)}
          onToggle={filing ? () => onCategoryOp?.({ op: 'collapseProject', cwd: p.cwd, collapsed: p.open }) : undefined}
          onNewCategory={filing ? () => createCategory(p.cwd) : undefined}
        />
        {p.open && <div className="ml-indent flex flex-col">{renderBody(p)}</div>}
      </div>
    );
  };

  const rest = tree.projects.some(p => p.loose.length || p.categories.length);
  const empty = searching ? t('session.searching') : q ? t('session.noMatch') : agentFilter ? t('session.noneAgent', { name: nameOf(agentFilter) }) : scope === 'workspace' && workspace ? t('session.noneWorkspace') : t('session.none');

  return (
    <div className={cn('flex flex-col', fill ? 'min-h-0 flex-1' : 'max-h-[60vh]')} onKeyDown={e => { if (e.key === 'Escape' && editing) { e.stopPropagation(); setEditing(undefined); } }}>
      <div className="flex items-center gap-1 px-1 pt-1">
        <label className="flex h-ctl min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-fg-3 focus-within:bg-hover">
          <Search className="size-icon shrink-0" strokeWidth={1.5} />
          <input
            autoFocus={autoFocus}
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder={onSearch ? t('session.searchContent') : t('session.search')}
            aria-label={onSearch ? t('session.searchContent') : t('session.search')}
            className="min-w-0 flex-1 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3"
          />
        </label>
      </div>
      <div className="flex min-w-0 shrink-0 items-center gap-1 px-1 pt-1">
        <ChannelFilter agents={agents} value={agentFilter} onChange={setAgentFilter} />
        <span className="ml-auto flex shrink-0 items-center">
          {filing && workspace && (
            <IconButton title={t('session.category.new')} aria-label={t('session.category.new')} onClick={() => createCategory(workspace)}>
              <CategoryAddIcon />
            </IconButton>
          )}
          {onListNative && onImportNative && (
            <ImportSessions
              agents={agents} agentFilter={agentFilter} activeAgent={activeAgent}
              native={nativeSessions}
              onList={onListNative}
              onImport={onImportNative}
              onSelect={onSelect}
            />
          )}
        </span>
      </div>
      <div className="scroll-thin mt-1 flex min-h-0 flex-col overflow-y-auto border-t border-line pb-1" role="listbox" aria-label={t('session.listAria')}
        style={timeCh ? { '--session-time': `${timeCh}ch` } as CSSProperties : undefined}
        onDragOver={onDragOver} onDrop={onDrop}>
        {/* Empty state takes exactly one item row (pt-1 + min-h-row) so the popover keeps its height whether the filter matches 0 or 1 session */}
        {!shown.length && !visibleCategories && <div className="mt-1 flex min-h-row items-center justify-center px-2 text-2 text-fg-3">{empty}</div>}
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
        {rest && (
          <div className="flex flex-col pt-1">
            {tree.grouped ? tree.projects.map(renderProject) : tree.projects.map(p => <Fragment key={p.cwd}>{renderBody(p)}</Fragment>)}
          </div>
        )}
      </div>
    </div>
  );
}

// Where a reordered category will land: a short accent line on the block's edge, positioned absolutely so nothing shifts
function InsertLine({ edge }: { edge: 'top' | 'bottom' }) {
  return <div role="presentation" className={cn('pointer-events-none absolute inset-x-2 z-10 h-0.5 rounded-full bg-accent', edge === 'top' ? '-top-px' : '-bottom-px')} />;
}

// Header of a project or a category. A project: folder glyph, heavier name, the "current" badge, a "new category" action.
// A category: the inbox glyph, a "new session here" action and a "…" menu. The row toggles; on hover the glyph turns into
// the chevron, and collapsed it still shows the most urgent state inside
function GroupHeader({ project, icon, name, title, badge, count, open, state, dragging, editing, onDragStart, onDragEnd, onToggle, onRename, onNewSession, onNewCategory, menu }: {
  project?: boolean;
  icon: ReactNode;
  name: string;
  title?: string;
  badge?: string;
  count: number;
  open: boolean;
  state?: SessionSummary['state'];
  dragging?: boolean;
  editing?: boolean;
  onDragStart?: (e: DragEvent) => void;
  onDragEnd?: () => void;
  onToggle?: () => void;
  onRename?: (name: string) => void;
  onNewSession?: () => void;
  onNewCategory?: () => void;
  menu?: ReactNode;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const act = 'inline-flex size-lead items-center justify-center rounded-sm text-fg-3 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1';
  const actions = !editing && (onNewSession || onNewCategory || menu);
  return (
    <div
      role="button"
      aria-expanded={open}
      tabIndex={0}
      title={title}
      data-menu-open={menuOpen || undefined}
      draggable={!!onDragStart}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onClick={() => { if (!editing) onToggle?.(); }}
      onKeyDown={e => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); onToggle?.(); } }}
      className={cn(
        'group flex min-h-row items-center gap-gap rounded-md px-2 text-2 transition-colors hover:bg-hover focus-visible:bg-hover',
        onToggle && 'cursor-pointer',
        project ? 'font-semibold text-fg-strong' : 'text-fg-1',
        dragging && 'opacity-40',
      )}
    >
      <span className="flex size-lead shrink-0 items-center justify-center text-fg-3">
        <span className={cn('flex', onToggle && 'group-hover:hidden group-focus-visible:hidden')}>{icon}</span>
        {onToggle && (
          <span className="hidden group-hover:flex group-focus-visible:flex">
            {open ? <ChevronDown className="size-icon" strokeWidth={1.5} /> : <ChevronRight className="size-icon" strokeWidth={1.5} />}
          </span>
        )}
      </span>
      {editing && onRename
        ? <RenameInput initial={name} label={t('session.category.rename')} onDone={onRename} />
        : <span className="min-w-0 truncate">{name}</span>}
      {badge && !editing && <span className="shrink-0 rounded-sm bg-active px-1 text-3 font-normal text-fg-3">{badge}</span>}
      {!editing && (
        <span className="ml-auto flex shrink-0 items-center text-3 font-normal text-fg-3 tabular-nums">
          <span className={cn('flex items-center gap-2', actions && 'group-hover:hidden group-focus-within:hidden group-data-[menu-open]:hidden')}>
            {state && <StateMark state={state} />}
            <span className="text-right">{count}</span>
          </span>
          {actions && (
            <span className="hidden items-center gap-0.5 group-hover:flex group-focus-within:flex group-data-[menu-open]:flex"
              onClick={e => e.stopPropagation()} onKeyDown={e => e.stopPropagation()}>
              {onNewCategory && (
                <button type="button" title={t('session.category.new')} aria-label={t('session.category.new')} onClick={onNewCategory} className={act}>
                  <CategoryAddIcon />
                </button>
              )}
              {onNewSession && (
                <button type="button" title={t('session.category.newSession')} aria-label={t('session.category.newSession')} onClick={onNewSession} className={act}>
                  <SquarePen className="size-3" strokeWidth={1.5} />
                </button>
              )}
              {menu && (
                <DropdownMenu.Root onOpenChange={setMenuOpen}>
                  <DropdownMenu.Trigger render={<button type="button" title={t('session.more')} aria-label={t('session.more')} className={act}><Ellipsis className="size-3" strokeWidth={1.5} /></button>} />
                  <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="end" width="md"><DropdownMenu.Popup>{menu}</DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
                </DropdownMenu.Root>
              )}
            </span>
          )}
        </span>
      )}
    </div>
  );
}

// Only the channel filter uses a picker; the session rows retain their own search,
// rename, pin and selection behavior. Adding ACPs never adds rows to the toolbar.
function ChannelFilter({ agents, value, onChange }: { agents: AgentInfo[]; value?: string; onChange: (id: string | undefined) => void }) {
  const [open, setOpen] = useState(false);
  const options = [{ id: '', name: t('session.channels') }, ...agents];
  const current = options.find(a => a.id === (value ?? '')) ?? { id: value!, name: value! };
  const searchable = agents.length >= 12;
  const select = (id: string) => { onChange(id || undefined); setOpen(false); };
  const mark = (a: { id: string; name: string }) => a.id
    ? <AgentMark id={a.id} name={a.name} />
    : <ListFilter className="size-icon" strokeWidth={1.5} />;
  const trigger = <button type="button" aria-label={`${t('session.filterChannel')}: ${current.name}`} title={current.name}
    className={cn('inline-flex h-[calc(var(--ctl)-4px)] min-w-0 max-w-full items-center gap-1.5 rounded-md px-2 text-3 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 data-[open]:bg-hover data-[popup-open]:bg-hover', value ? 'text-fg-1' : 'text-fg-2')}>
    <span className="flex size-icon shrink-0 items-center justify-center">{mark(current)}</span>
    <span className="min-w-0 truncate">{current.name}</span>
    <ChevronDown className="size-3 shrink-0 text-fg-3" strokeWidth={1.5} />
  </button>;
  const content = (a: { id: string; name: string }) => <OptionContent icon={mark(a)} checked={a.id === current.id} checkSlot>{a.name}</OptionContent>;

  return <div className="min-w-0 max-w-full" onKeyDownCapture={e => {
    // The filter may sit in a history popup or drawer. Consume its first Escape
    // before either the enclosing overlay or the drawer handles the same key.
    if (open && e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setOpen(false); }
  }}>
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger render={trigger} />
      <Popover.Portal><Popover.Positioner width="md"><Popover.Popup aria-label={t('session.filterChannel')}>
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
// the rest import on click. The agent switcher in the header re-requests the list for the agent it lands on
function ImportSessions({ agents, agentFilter, activeAgent, native, onList, onImport, onSelect }: {
  agents: AgentInfo[];
  agentFilter?: AgentId;
  activeAgent?: AgentId;
  native?: NativeSessionsState;
  onList: (agent: AgentId) => void;
  onImport: (agent: AgentId, s: NativeSessionInfo) => void;
  onSelect: (id: string) => void;
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
    <span className="ml-auto shrink-0" onKeyDownCapture={e => {
      // Nested inside the history popover / drawer: consume the first Escape like ChannelFilter does
      if (open && e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setOpen(false); }
    }}>
      <Popover.Root open={open} onOpenChange={o => { setOpen(o); if (!o) setPicked(undefined); }}>
        <Popover.Trigger render={<IconButton title={t('session.import.action')} aria-label={t('session.import.action')}><Import strokeWidth={1.5} /></IconButton>} />
        <Popover.Portal><Popover.Positioner width="xl"><Popover.Popup initialFocus={interaction => interaction === 'keyboard'} aria-label={t('session.import.action')}>
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
  editing: boolean;
  onSelect: () => void;
  onEdit: () => void;
  onRename: (title: string) => void;
  onDelete: () => void;
  onPin: () => void;
  // Present only for a session from another project: re-home it into this window's workspace
  onMove?: () => void;
  // The menus' "move to project" submenu: the other projects and the pick
  projects?: MoveProject[];
  onMoveTo?: (cwd: string) => void;
  onExport?: (format: 'markdown' | 'json') => void;
  // Native drag: present when the row may be filed by dragging (not pinned, not a ChatGPT mirror)
  dragging?: boolean;
  onDragStart?: (e: DragEvent) => void;
  onDragEnd?: () => void;
  // The "move to category" submenu: the session's own project's categories
  categories?: SessionCategory[];
  onFile?: (category: string | null) => void;
  onNewCategory?: () => void;
}

// One item: the whole row is clickable to select; the tail shows a status dot + time by default, swapping to actions on hover / keyboard focus. Action buttons can't nest inside a button, so the whole row is a div[role=option].
// The hover cluster keeps the two quick toggles (move / pin); rename, export and delete live in the "…" menu, which keeps
// the cluster alive while open — the row tracks menuOpen because a pointer inside the portaled popup is no longer a hover.
// Right-clicking the row opens the same menu at the pointer (not while renaming, where the input keeps the native one)
function Item({ session: s, agentName, terms, snippet, active, time, project, editing, onSelect, onEdit, onRename, onDelete, onPin, onMove, projects, onMoveTo, onExport, dragging, onDragStart, onDragEnd, categories, onFile, onNewCategory }: ItemProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onSelect(); }
  };
  const act = 'inline-flex size-lead items-center justify-center rounded-sm text-fg-3 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1';
  const menu: SessionMenuItemsProps = { session: s, onRename: onEdit, onPin, onMove, projects, onMoveTo, onExport, onDelete, categories, onFile, onNewCategory };
  const row = (
    <div
      role="option"
      aria-selected={active}
      tabIndex={0}
      data-session={s.id}
      data-menu-open={menuOpen || undefined}
      draggable={!!onDragStart}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onClick={() => { if (!editing) onSelect(); }}
      onKeyDown={onKey}
      className={cn(
        'group flex min-h-row cursor-pointer items-center gap-gap rounded-md px-2 text-2 text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover',
        snippet && !editing && 'py-1',
        active && 'bg-active text-fg-strong hover:bg-active',
        dragging && 'opacity-40',
      )}
    >
      <span className="flex size-lead shrink-0 items-center justify-center text-fg-3" title={agentName}><AgentMark id={s.agent} name={agentName} /></span>
      {editing
        ? <RenameInput initial={s.title} onDone={onRename} />
        : (
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate"><Marked text={s.title} terms={terms} /></span>
            {snippet && <span className="truncate text-3 text-fg-3"><Marked text={snippet} terms={terms} /></span>}
          </span>
        )}
      {!editing && (
        <span className="ml-auto flex shrink-0 items-center text-3 text-fg-3 tabular-nums">
          <span className="flex items-center gap-2 group-hover:hidden group-focus-within:hidden group-data-[menu-open]:hidden">
            {project && <span className="max-w-project truncate text-fg-3/70" title={s.cwd}>{project}</span>}
            {s.state && <StateMark state={s.state} />}
            <span className="min-w-session-time text-right">{time}</span>
          </span>
          <span className="hidden items-center gap-0.5 group-hover:flex group-focus-within:flex group-data-[menu-open]:flex">
            {onMove && (
              <button type="button" title={t('session.move')} aria-label={t('session.move')} onClick={e => { e.stopPropagation(); onMove(); }} className={act}>
                <FolderInput className="size-3" strokeWidth={1.5} />
              </button>
            )}
            <button type="button" title={s.pinned ? t('common.unpin') : t('common.pin')} aria-label={s.pinned ? t('common.unpin') : t('common.pin')} onClick={e => { e.stopPropagation(); onPin(); }} className={act}>
              {s.pinned ? <PinOff className="size-3" strokeWidth={1.5} /> : <Pin className="size-3" strokeWidth={1.5} />}
            </button>
            {/* The row selects on click and synthetic events bubble through the menu's portal; keep both from reaching the option */}
            <span onClick={e => e.stopPropagation()} onKeyDown={e => e.stopPropagation()}>
              <SessionMenu
                {...menu}
                align="end"
                onOpenChange={setMenuOpen}
                trigger={<button type="button" title={t('session.more')} aria-label={t('session.more')} className={act}>
                  <Ellipsis className="size-3" strokeWidth={1.5} />
                </button>}
              />
            </span>
          </span>
        </span>
      )}
    </div>
  );
  return editing ? row : <SessionContextMenu {...menu} onOpenChange={setMenuOpen}>{row}</SessionContextMenu>;
}

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
        if (e.key === 'Enter') finish(value);
        if (e.key === 'Escape') finish(initial);
      }}
      onBlur={() => finish(value)}
      aria-label={label}
      className="min-w-0 flex-1 rounded-sm bg-active px-1 text-2 text-fg-1 outline-none"
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
  if (dayAgo <= 0) return new Intl.DateTimeFormat(locale, { hour: 'numeric', minute: '2-digit' }).format(d);
  return new Intl.DateTimeFormat(locale, { month: 'numeric', day: 'numeric' }).format(d);
}
