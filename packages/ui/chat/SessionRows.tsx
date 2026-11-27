import { memo, useEffect, useRef, useState, type DragEvent, type KeyboardEvent, type RefObject } from 'react';
import { Circle, FolderInput, LoaderCircle, Pin, PinOff } from 'lucide-react';
import type { SessionCategory, SessionSummary } from '@acpira/shared/transcript';
import type { ExportFormat } from '@acpira/shared/protocol';
import { foreignBrand } from '@acpira/shared/models';
import { cn } from '../ui/cn';
import { useTapActivate } from '../ui/useTapActivate';
import { imeKey } from '../ui/ime';
import { dateFormat, getLocale, t, useLocale } from '../i18n';
import { AgentMark } from './AgentMark';
import { ModelMark } from './ModelMark';
import { SessionContextMenu, type MoveProject, type SessionCopyKind, type SessionMenuItemsProps } from './SessionMenu';
import { markParts } from './sessionSearch';
import { draggable, type MoveTarget } from './sessionTree';
import { rowDone, rowState } from '../chrome/styleClock';

// A running session shows a spinning ring (the one place a spinner is allowed: a list has no verb to shimmer); the other states are plain dots.
// Unread (a turn that finished unwatched) is a slightly larger dot in the strongest foreground with a halo rippling out of it;
// motion=none / reduced motion drop the halo and leave the dot lit. Not the accent: its default brand amber is indistinguishable from waiting's warn
const STATE_DOT: Record<Exclude<NonNullable<SessionSummary['state']>, 'working' | 'background'>, string> = {
  waiting: 'size-1.5 bg-warn waiting-pulse',
  unread: 'size-unread-dot bg-fg-strong unread-halo',
  error: 'size-1.5 bg-danger error-blink',
};

// Ring and dots share one fixed slot so every mark sits on the same centre line, and each names its meaning on hover.
// The ring is drawn smaller than the slot: at full size its arc outweighed even an 8 px dot
// Background (the turn ended, a task it started still runs) is the working ring at rest: a closed circle in the faint
// foreground with a small dot orbiting it slowly, so it reads as "alive, not done, nothing to do". A reply that came in
// meanwhile lights it in the strongest foreground with a dot inside, never the unread dot itself, which means finished
// A background ring waiting on Claude's scheduled wakeup names the time it resumes; the mark itself stays the same, since
// either way the session is alive and asks nothing of the user
export interface Mark { state: NonNullable<SessionSummary['state']>; unread?: boolean; wakeAt?: number }
export function StateMark({ state, unread, wakeAt }: Mark) {
  const label = state === 'background' && wakeAt
    ? t(unread ? 'session.state.wakeupUnread' : 'session.state.wakeup', { time: dateFormat(getLocale(), { hour: 'numeric', minute: '2-digit' }).format(wakeAt) })
    : t(state === 'background' && unread ? 'session.state.backgroundUnread' : `session.state.${state}`);
  return <span className="flex size-3 shrink-0 items-center justify-center" title={label} aria-label={label} role="img">
    {state === 'working'
      // Spin a wrapper span rather than the <svg> root: Blink cannot composite SVG-root transforms and falls back to
      // main-thread style recalc + Layerize on every display vsync
      ? <span className="inline-flex size-2.5 animate-spin live-spin text-fg-2"><LoaderCircle className="size-full" strokeWidth={2.5} /></span>
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

// Row leading inset across tree levels: loose rows under a project align with the project name (`project`),
// category sessions step 12 px further in so their agent mark aligns with the category name (`category` in single-project
// workspace scope, `nested` under a project group)
export type RowIndent = 'project' | 'category' | 'nested';
export const ROW_INDENT_CLASS: Record<RowIndent, string> = {
  category: 'pl-6',
  project: 'pl-6',
  nested: 'pl-10',
};

// What a collapsed group still tells about its sessions: the most urgent state inside. A finished reply outranks
// background work, which asks nothing of the user
const STATE_RANK: NonNullable<SessionSummary['state']>[] = ['working', 'waiting', 'error', 'unread', 'background'];
export function aggregate(sessions: SessionSummary[]): Mark | undefined {
  const state = STATE_RANK.find(state => sessions.some(s => s.state === state));
  return state && { state, unread: state === 'background' && sessions.some(s => s.state === 'background' && s.unread) };
}

// A header's hover actions: they cross-fade in over its resting count / state, in the same cell, and take clicks only then.
// Keyboard focus (focus-visible) reveals them too, not focus-within: a click focuses the row, which kept them showing after
// the pointer left until a click elsewhere. A focused action is seen through the span's own `:has(>…)` (CONCEAL: its next
// sibling's), never a group-level `:has()`, which restyled the whole shell on every keystroke (docs/dev/webview.md)
export const REVEAL = 'pointer-events-none opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-visible:pointer-events-auto group-focus-visible:opacity-100 has-[>:focus-visible]:pointer-events-auto has-[>:focus-visible]:opacity-100 group-data-[menu-open]:pointer-events-auto group-data-[menu-open]:opacity-100';
// The concealed tail also stops taking the pointer: at opacity 0 it paints (and hit-tests) above the fully opaque actions,
// so once their fade ended the pointer fell through to it, the button lost its hover fill and a click selected the row
export const CONCEAL = 'transition-opacity duration-150 group-hover:pointer-events-none group-hover:opacity-0 group-focus-visible:pointer-events-none group-focus-visible:opacity-0 has-[+*>:focus-visible]:pointer-events-none has-[+*>:focus-visible]:opacity-0 group-data-[menu-open]:pointer-events-none group-data-[menu-open]:opacity-0';
// The resting tail and the hover actions stacked in one grid cell, so the title's room never changes under the pointer
export const STACK = "grid [grid-template-areas:'x'] *:[grid-area:x]";

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
  // Leading inset in the session tree: under a project (`project`), under a category (`category`), or both (`nested`)
  indent?: RowIndent;
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
  // The menu's "archive" and "mark as unread / read"
  archivable?: boolean;
  markable?: boolean;
  // Native drag: the row may be filed by dragging (not pinned)
  drag?: boolean;
  dragging?: boolean;
  // The "move to category" submenu over `categories` (the session's own project's) and "new category…"
  fileable?: boolean;
  categories?: SessionCategory[];
}

// What a row's controls do: SessionList's handlers of the moment, read through a ref when a control is used
export interface RowActions {
  select: (s: SessionSummary) => void;
  edit: (s: SessionSummary) => void;
  rename: (s: SessionSummary, title: string) => void;
  delete: (s: SessionSummary) => void;
  pin: (s: SessionSummary) => void;
  archive: (s: SessionSummary, archived: boolean) => void;
  unread: (s: SessionSummary, unread: boolean) => void;
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
export const Item = memo(function Item({ session, agentName, terms, snippet, active, time, project, model, indent, editing, actions, moveHere, moveTo, projects, exportable, copyable, archivable, markable, drag, dragging, fileable, categories }: ItemProps) {
  // Memo / a cached parent can skip this on a language switch: subscribe so the strings follow the new locale
  useLocale();
  // Unrelated pushes and replies to earlier renames must not replace the latest submitted title.
  const [renamed, setRenamed] = useState<{ title: string; waiting: string[] }>();
  const pending = renamed?.waiting.includes(session.title) ? renamed : undefined;
  if (renamed && !pending) setRenamed(undefined);
  const s = pending ? { ...session, title: pending.title } : session;
  const [menuOpen, setMenuOpen] = useState(false);
  const select = () => actions.current.select(s);
  // A finger tap selects too, though the pin it reveals makes WebKit withhold the click
  const tap = useTapActivate(select, !editing);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); select(); }
  };
  const act = 'inline-flex size-lead items-center justify-center rounded-sm text-fg-3 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1';
  const menu: SessionMenuItemsProps = {
    session: s,
    onRename: () => actions.current.edit(s),
    onPin: () => actions.current.pin(s),
    onUnread: markable ? unread => actions.current.unread(s, unread) : undefined,
    onArchive: archivable ? archived => actions.current.archive(s, archived) : undefined,
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
      data-state={rowState(s)}
      data-done={s.completedAt !== undefined || undefined}
      style={rowDone(s)}
      data-menu-open={menuOpen || undefined}
      draggable={!!drag}
      onDragStart={drag ? e => actions.current.dragStart(s, e) : undefined}
      onDragEnd={() => actions.current.dragEnd()}
      {...tap}
      onKeyDown={onKey}
      className={cn(
        // The row whose context menu is open keeps the hover fill: the popup under the pointer ends the row's :hover
        'group flex min-h-row cursor-pointer items-center gap-gap rounded-md px-2 text-2 text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover data-[menu-open]:bg-hover data-[menu-open]:text-fg-1',
        indent && ROW_INDENT_CLASS[indent],
        snippet && !editing && 'py-1',
        // Renaming keeps the fill too: the menu that started it has closed and the focused input is not the row's focus-visible
        // select-none as on ui/Row: a drag or double click selected titles, and WebKit keeps a selection when the next click
        // lands on unselectable chrome. Off while renaming: an ancestor's -webkit-user-select: none blocks selecting in the input
        editing ? 'bg-hover text-fg-1' : 'select-none',
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
        ? <RenameInput initial={s.title} onDone={value => {
          const title = value.trim();
          if (title && title !== s.title) {
            const waiting = new Set([session.title, ...(pending?.waiting ?? []), s.title]);
            waiting.delete(title);
            setRenamed({ title, waiting: [...waiting] });
          }
          actions.current.rename(s, title);
        }} />
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
            {s.state && <StateMark state={s.state} unread={s.unread} wakeAt={s.wakeAt} />}
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
export function RenameInput({ initial, label = t('session.titleAria'), onDone }: { initial: string; label?: string; onDone: (title: string) => void }) {
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
