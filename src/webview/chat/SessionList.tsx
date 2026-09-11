import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { FolderInput, LoaderCircle, Pencil, Pin, PinOff, Search, Trash2 } from 'lucide-react';
import type { AgentInfo, SessionSummary } from '@shared/transcript';
import { inWorkspace, type SessionScope } from '@shared/settings';
import { cn } from '../ui/cn';
import { t, useLocale } from '../i18n';
import { AgentMark } from './AgentMark';

// A running session shows a spinning ring (the one place a spinner is allowed: a list has no verb to shimmer); the other states are plain dots
const STATE_DOT: Record<Exclude<NonNullable<SessionSummary['state']>, 'working'>, string> = {
  waiting: 'bg-warn',
  unread: 'bg-fg-2',
  error: 'bg-danger',
};

function StateMark({ state }: { state: NonNullable<SessionSummary['state']> }) {
  if (state === 'working') return <LoaderCircle className="size-3 animate-spin live-spin text-fg-2" strokeWidth={2} aria-label={t('session.state.working')} />;
  return <span className={cn('size-1.5 rounded-full', STATE_DOT[state])} />;
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
  onSelect: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
  onPin: (id: string, pinned: boolean) => void;
  onMove?: (id: string) => void;
}

// Session list: search and agent filters stay visible even without history; pinned sessions get their own section, the rest is one flat list.
// Each item: vendor mark · title · time; on hover those swap for the actions — pin / rename / delete, plus "move here" for a session from another project.
// Deletion applies immediately, undo lives on the Toast at the shell's bottom. Scope comes from acpira.sessionScope (settings);
// under "all" each row from another project carries that project's folder name before the time
export function SessionList({ sessions, agents, activeId, workspace, scope = 'all', autoFocus, onSelect, onRename, onDelete, onPin, onMove }: SessionListProps) {
  const locale = useLocale();
  const [query, setQuery] = useState('');
  const [agentFilter, setAgentFilter] = useState<string>();
  const [editing, setEditing] = useState<string>();
  const nameOf = (id: string) => agents.find(a => a.id === id)?.name ?? id;
  const here = (s: SessionSummary) => !workspace || inWorkspace(s, workspace);

  const q = query.trim().toLowerCase();
  // The active session always stays listed (its row is the highlight), even when it belongs to another project
  const inScope = (s: SessionSummary) => scope === 'all' || here(s) || s.id === activeId;
  const shown = sessions.filter(s => inScope(s) && (!agentFilter || s.agent === agentFilter) && (!q || s.title.toLowerCase().includes(q) || nameOf(s.agent).toLowerCase().includes(q)));

  const now = new Date();
  const dayOf = (iso: string) => Math.floor((startOfDay(now) - startOfDay(new Date(iso))) / 86_400_000);
  const pinned = shown.filter(s => s.pinned);
  const rest = shown.filter(s => !s.pinned);

  const renderItem = (s: SessionSummary) => (
    <Item
      key={s.id}
      session={s}
      agentName={nameOf(s.agent)}
      active={s.id === activeId}
      time={fmtTime(s.updatedAt, dayOf(s.updatedAt), locale)}
      project={here(s) ? undefined : projectName(s.cwd)}
      editing={editing === s.id}
      onSelect={() => onSelect(s.id)}
      onEdit={() => setEditing(s.id)}
      onRename={t => { setEditing(undefined); if (t.trim() && t.trim() !== s.title) onRename(s.id, t); }}
      onDelete={() => { setEditing(undefined); onDelete(s.id); }}
      onPin={() => onPin(s.id, !s.pinned)}
      onMove={onMove && workspace && !here(s) ? () => onMove(s.id) : undefined}
    />
  );
  const empty = q ? t('session.noMatch') : agentFilter ? t('session.noneAgent', { name: nameOf(agentFilter) }) : scope === 'workspace' && workspace ? t('session.noneWorkspace') : t('session.none');

  return (
    <div className="flex max-h-[60vh] flex-col" onKeyDown={e => { if (e.key === 'Escape' && editing) { e.stopPropagation(); setEditing(undefined); } }}>
      <div className="flex items-center gap-1 px-1 pt-1">
        <label className="flex h-ctl min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-fg-3 focus-within:bg-hover">
          <Search className="size-icon shrink-0" strokeWidth={1.5} />
          <input
            autoFocus={autoFocus}
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder={t('session.search')}
            aria-label={t('session.search')}
            className="min-w-0 flex-1 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3"
          />
        </label>
      </div>
      <div className="flex flex-wrap items-center gap-1 px-1 pt-1">
        <FilterChip active={!agentFilter} onClick={() => setAgentFilter(undefined)}>{t('common.all')}</FilterChip>
        {agents.map(a => (
          <FilterChip key={a.id} active={agentFilter === a.id} onClick={() => setAgentFilter(a.id)}>
            <AgentMark id={a.id} name={a.name} />{a.name}
          </FilterChip>
        ))}
      </div>
      <div className="mt-1 flex min-h-0 flex-col overflow-y-auto border-t border-line pb-1" role="listbox" aria-label={t('session.listAria')}>
        {/* Empty state takes exactly one item row (pt-1 + min-h-row) so the popover keeps its height whether the filter matches 0 or 1 session */}
        {!shown.length && <div className="mt-1 flex min-h-row items-center justify-center px-2 text-2 text-fg-3">{empty}</div>}
        {pinned.length > 0 && (
          <div className="flex flex-col pt-1" role="group" aria-label={t('session.group.pinned')}>
            {pinned.map(renderItem)}
          </div>
        )}
        {/* Keep the divider mounted so both pinning and unpinning can transition. */}
        <div
          role="presentation"
          className={cn(
            'shrink-0 bg-linear-to-r from-transparent via-line to-transparent transition-[height,margin,opacity,scale] duration-(--dur-open) ease-out',
            pinned.length > 0 && rest.length > 0 ? 'my-1 h-px scale-x-100 opacity-100' : 'my-0 h-0 scale-x-0 opacity-0',
          )}
        />
        {rest.length > 0 && <div className="flex flex-col pt-1">{rest.map(renderItem)}</div>}
      </div>
    </div>
  );
}

function FilterChip({ active, onClick, children }: { active: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn('inline-flex h-[calc(var(--ctl)-4px)] items-center gap-1 rounded-md px-2 text-3 transition-colors', active ? 'bg-active text-fg-1' : 'text-fg-2 hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1')}
    >
      {children}
    </button>
  );
}

interface ItemProps {
  session: SessionSummary;
  agentName: string;
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
}

// One item: the whole row is clickable to select; the tail shows a status dot + time by default, swapping to actions on hover / keyboard focus. Action buttons can't nest inside a button, so the whole row is a div[role=option]
function Item({ session: s, agentName, active, time, project, editing, onSelect, onEdit, onRename, onDelete, onPin, onMove }: ItemProps) {
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onSelect(); }
  };
  const act = 'inline-flex size-lead items-center justify-center rounded-sm text-fg-3 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1';
  return (
    <div
      role="option"
      aria-selected={active}
      tabIndex={0}
      onClick={() => { if (!editing) onSelect(); }}
      onKeyDown={onKey}
      className={cn(
        'group flex min-h-row cursor-pointer items-center gap-gap rounded-md px-2 text-2 text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover',
        active && 'bg-active text-fg-strong hover:bg-active',
      )}
    >
      <span className="flex size-lead shrink-0 items-center justify-center text-fg-3" title={agentName}><AgentMark id={s.agent} name={agentName} /></span>
      {editing
        ? <RenameInput initial={s.title} onDone={onRename} />
        : <span className="min-w-0 flex-1 truncate">{s.title}</span>}
      {!editing && (
        <span className="ml-auto flex shrink-0 items-center text-3 text-fg-3 tabular-nums">
          <span className="flex items-center gap-2 group-hover:hidden group-focus-within:hidden">
            {project && <span className="max-w-project truncate text-fg-3/70" title={s.cwd}>{project}</span>}
            {s.state && <StateMark state={s.state} />}
            <span>{time}</span>
          </span>
          <span className="hidden items-center gap-0.5 group-hover:flex group-focus-within:flex">
            {onMove && (
              <button type="button" title={t('session.move')} aria-label={t('session.move')} onClick={e => { e.stopPropagation(); onMove(); }} className={act}>
                <FolderInput className="size-3" strokeWidth={1.5} />
              </button>
            )}
            <button type="button" title={s.pinned ? t('common.unpin') : t('common.pin')} aria-label={s.pinned ? t('common.unpin') : t('common.pin')} onClick={e => { e.stopPropagation(); onPin(); }} className={act}>
              {s.pinned ? <PinOff className="size-3" strokeWidth={1.5} /> : <Pin className="size-3" strokeWidth={1.5} />}
            </button>
            <button type="button" title={t('common.rename')} aria-label={t('common.rename')} onClick={e => { e.stopPropagation(); onEdit(); }} className={act}>
              <Pencil className="size-3" strokeWidth={1.5} />
            </button>
            <button type="button" title={t('common.delete')} aria-label={t('common.delete')} onClick={e => { e.stopPropagation(); onDelete(); }} className={act}>
              <Trash2 className="size-3" strokeWidth={1.5} />
            </button>
          </span>
        </span>
      )}
    </div>
  );
}

// Inline rename: ⏎ commits, Esc cancels, blur commits; the callback fires only once (the blur right after Esc doesn't count)
function RenameInput({ initial, onDone }: { initial: string; onDone: (title: string) => void }) {
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
      aria-label={t('session.titleAria')}
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

// Today shows the time of day, earlier shows month/day — both follow the UI locale
function fmtTime(iso: string, dayAgo: number, locale: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  if (dayAgo <= 0) return new Intl.DateTimeFormat(locale, { hour: 'numeric', minute: '2-digit' }).format(d);
  return new Intl.DateTimeFormat(locale, { month: 'numeric', day: 'numeric' }).format(d);
}
