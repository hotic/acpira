import { useEffect, useRef, useState, type DragEvent, type ReactNode } from 'react';
import { ChevronDown, ChevronRight, Ellipsis, Folder, FolderOpen, ListFilter, Plus, SquarePen } from 'lucide-react';
import type { AgentInfo, SessionSummary } from '@acpira/shared/transcript';
import { cn } from '../ui/cn';
import { Command } from '../ui/Command';
import { DropdownMenu } from '../ui/DropdownMenu';
import { CARET_TURN } from '../ui/Button';
import { OptionContent } from '../ui/Panel';
import { useTapActivate } from '../ui/useTapActivate';
import { Popover } from '../ui/Popover';
import { t } from '../i18n';
import { useDesktop } from '../desktop';
import { AgentMark } from './AgentMark';
import { CategoryAddIcon } from './CategoryIcon';
import { RowContextMenu } from './SessionMenu';
import { webviewState } from '../vscodeApi';
import { draggable } from './sessionTree';
import { CONNECTION_STATE_NAME, useFolderHost } from '../remote/api';
import { RemoteFolderIcon } from '../remote/RemoteFolderIcon';
import { still } from './SessionListMotion';
import { Mark, StateMark, aggregate, RowIndent, ROW_INDENT_CLASS, REVEAL, CONCEAL, STACK, RenameInput } from './SessionRows';

export const MORE_MARKS = 3;
// Each click on the "more" row shows this many more, as Codex does; once all are out it reads "show less"
export const MORE_PAGE = 10;

// The rule between the pinned rows and the rest. Kept mounted so both pinning and unpinning can fade it. Its room changes
// at once: the rows around it slide through FlipRows, and a height transition under those slides made them overshoot
export function PinDivider({ on }: { on: boolean }) {
  return (
    <div
      role="presentation"
      className={cn(
        'shrink-0 bg-linear-to-r from-transparent via-line to-transparent transition-[opacity,scale] duration-(--dur-open) ease-out',
        on ? 'my-1 h-px scale-x-100 opacity-100' : 'my-0 h-0 scale-x-0 opacity-0',
      )}
    />
  );
}

export const SECTION_ACT = 'inline-flex size-ctl-sm items-center justify-center rounded-md text-fg-3 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 data-[popup-open]:bg-hover data-[popup-open]:text-fg-1';
// The filter row's view button, an IconButton's box (ui/Button)
export const ROW_ACT = 'inline-flex size-ctl shrink-0 items-center justify-center rounded-md text-fg-2 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 data-[popup-open]:bg-active data-[popup-open]:text-fg-1';

export type Section = 'pinned' | 'rest' | 'chats';
export const FOLDED_KEY = 'sessionSectionsFolded';
export function storedFolds(): Section[] {
  const v = webviewState(FOLDED_KEY);
  return Array.isArray(v) ? v.filter((x): x is Section => x === 'pinned' || x === 'rest' || x === 'chats') : [];
}

// A section's quiet label in the nav layout (Pinned, Projects / Sessions), its actions at the end. The label folds the
// section and reads the same either way: no count, and the chevron (turned to the state) only on hover or focus, so
// folding never makes anything appear or vanish. The text starts where the rows' glyphs do, not at the rows' padding.
// `quiet` (desktop): the actions come out on hover or focus too, and stay while they hold a choice (`keep`: a channel
// filter is on) or one of them has its popup open
// `rename`: a section being renamed shows this input in place of its label.
// Draggable sections use a div[role="button"] trigger because WebKit swallows drags started on a native <button>
export function SectionLabel({ children, actions, open, quiet, keep, first, rename, onToggle, section, onDragStart, onDragEnd, menu }: {
  children: ReactNode;
  actions?: ReactNode;
  open: boolean;
  quiet?: boolean;
  keep?: boolean;
  first?: boolean;
  rename?: ReactNode;
  onToggle: () => void;
  section?: string;
  onDragStart?: (e: DragEvent) => void;
  onDragEnd?: () => void;
  menu?: ReactNode;
}) {
  const [contextOpen, setContextOpen] = useState(false);
  const triggerClass = 'inline-flex min-w-0 cursor-pointer select-none items-center gap-1 rounded-sm px-[calc((var(--spacing-lead)-var(--spacing-icon))/2)] transition-colors hover:text-fg-1 focus-visible:text-fg-1';
  const triggerContent = <>
    <span className="min-w-0 truncate">{children}</span>
    <ChevronRight strokeWidth={1.5} className={cn('size-3 shrink-0 opacity-0 transition-[rotate,opacity] duration-(--dur-open) group-hover:opacity-100 [:focus-visible>&]:opacity-100 group-data-[menu-open]:opacity-100',
      open && 'rotate-90')} />
  </>;
  const row = (
    <div
      data-section-row={section}
      data-menu-open={contextOpen || undefined}
      draggable={!!onDragStart}
      onDragStart={onDragStart ? e => {
        if ((e.target as Element).closest('[data-section-actions]')) { e.preventDefault(); return; }
        onDragStart(e);
      } : undefined}
      onDragEnd={onDragEnd}
      className={cn('group flex min-h-row items-center gap-gap px-2 text-3 text-fg-3', !first && 'mt-gap', !rename && 'select-none')}
    >
      {rename ?? (onDragStart ? (
        <div
          role="button"
          tabIndex={0}
          aria-expanded={open}
          onClick={onToggle}
          onKeyDown={e => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); onToggle(); } }}
          className={triggerClass}>
          {triggerContent}
        </div>
      ) : (
        <button type="button" aria-expanded={open} onClick={onToggle} className={triggerClass}>
          {triggerContent}
        </button>
      ))}
      {actions && !rename && (
        <span data-section-actions className={cn('-mr-1 ml-auto flex shrink-0 items-center gap-0.5',
          quiet && !keep && 'opacity-0 transition-opacity duration-150 group-hover:opacity-100 has-[:focus-visible]:opacity-100 [:focus-visible~&]:opacity-100 has-[[data-popup-open]]:opacity-100 group-data-[menu-open]:opacity-100')}>
          {actions}
        </span>
      )}
    </div>
  );
  return menu ? <RowContextMenu items={menu} disabled={!!rename} onOpenChange={setContextOpen}>{row}</RowContextMenu> : row;
}

// A section label's "…": rename, move up / down, delete (its projects go back under Projects). Opens to the right of the
// column, over the chat, like every menu of the desktop column
export function SectionActions({ menu }: { menu: ReactNode }) {
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger render={<button type="button" title={t('session.section.more')} aria-label={t('session.section.more')} className={SECTION_ACT}><Ellipsis className="size-icon" strokeWidth={1.5} /></button>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="inline-end" align="start" width="sm"><DropdownMenu.Popup finalFocus={false}>
        {menu}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

// Desktop: an empty project's only row. With a way to start a session it is that action, a muted plus and "new session in
// this project" that brighten under the pointer; without one it only says the project has none
// `flush`: at a section label's inset (the empty chats section), `label` its action's words
export function EmptyProjectRow({ onNew, flush, label = t('session.project.newSession') }: { onNew?: () => void; flush?: boolean; label?: string }) {
  const box = cn('group flex min-h-row w-full shrink-0 items-center gap-gap rounded-md pr-2 text-left text-3 text-fg-3', flush ? 'pl-2' : 'pl-6');
  if (!onNew) return <div className={box}><span className="-mr-gap-half w-lead shrink-0" />{t('session.none')}</div>;
  return (
    <button type="button" onClick={onNew}
      className={cn(box, 'transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1')}>
      <span className="-mr-gap-half flex size-lead shrink-0 items-center justify-center opacity-60 transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100">
        <Plus strokeWidth={1.5} className="size-icon" />
      </span>
      <span className="min-w-0 truncate">{label}</span>
    </button>
  );
}

export const pages = <T,>(list: T[]) => Array.from({ length: Math.ceil(list.length / MORE_PAGE) }, (_, i) => list.slice(i * MORE_PAGE, (i + 1) * MORE_PAGE));

// Desktop: the row under a project's latest sessions. Folded it tells what the rest holds: how many, the agents among them
// (their marks) and the most urgent state inside, on the session rows' state column; `rest` is what is still hidden, a
// page at a time coming out above it. With all of them out (`open`), it reads "show less" at the
// list's end: the chevron turns over, the labels cross-fade and the preview fades away, so the row changes in place
// `indent`: aligns with the rows it expands (chats sit flush, project loose rows at `project`, category rows at `category` / `nested`)
// `group`: the project's full path (or the chats' / category's key), the row's identity for FlipRows
export function MoreRow({ group, open, rest, nameOf, onToggle, indent }: { group: string; open: boolean; rest: SessionSummary[]; nameOf: (id: string) => string; onToggle: () => void; indent?: RowIndent }) {
  const marks = [...new Set(rest.map(s => s.agent))].slice(0, MORE_MARKS);
  const state = aggregate(rest);
  return (
    <button type="button" data-more={group} aria-expanded={open} onClick={onToggle}
      className={cn('group flex min-h-row w-full shrink-0 items-center gap-gap rounded-md pr-2 text-left text-3 text-fg-3 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1', indent ? ROW_INDENT_CLASS[indent] : 'pl-2')}>
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

// Header of a project or a category. A project: the folder glyph (open or shut with the fold), the heavier name, the
// "current" badge, then its tail. A category (`sub`): a collapsible tree row with an always-visible leading chevron
// that turns 90 deg when open, and a medium-muted name aligned under its project when grouped. Hovered,
// the resting tail (host / state / count) cross-fades into the actions — "new category", "new session here" and the "…"
// menu, which right-click opens too. Collapsed, both still show the most urgent state inside. A project on an SSH host
// (`folder` is `ssh://…`) has the monochrome globe-badged folder and, right-aligned in the tail, the host's name and its
// connection dot (green connected, breathing while connecting, dimmed when off, amber when failed)
export function GroupHeader({ project, sub, indent, name, title, folder, badge, count, open, state, preview, fresh, dragging, editing, onDragStart, onDragEnd, onToggle, onRename, onNewSession, newSessionLabel = t('session.category.newSession'), onNewCategory, menu }: {
  project?: boolean;
  sub?: boolean;
  // Category under a project group: sits on the project's session line (pl-6)
  indent?: boolean;
  name: string;
  title?: string;
  // A project's folder, for the SSH host it may be on
  folder?: string;
  badge?: string;
  // Absent: no count (the desktop's project rows)
  count?: number;
  open: boolean;
  state?: Mark;
  preview?: ReactNode;
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
  // A finger tap toggles too, though the actions it reveals make WebKit withhold the click
  const tap = useTapActivate(() => onToggle?.(), !editing && !!onToggle);
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
  const header = (
    <div
      ref={self}
      role="button"
      aria-expanded={open}
      tabIndex={0}
      title={title}
      data-menu-open={menuOpen || contextOpen || undefined}
      data-project-row={project ? folder ?? '' : undefined}
      draggable={!!onDragStart}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      {...tap}
      onKeyDown={e => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); onToggle?.(); } }}
      className={cn(
        'group flex min-h-row items-center gap-gap rounded-md pr-2 text-2 transition-colors hover:bg-hover focus-visible:bg-hover data-[menu-open]:bg-hover',
        indent ? 'pl-6' : 'pl-2',
        onToggle && 'cursor-pointer',
        project ? 'font-semibold text-fg-strong' : 'text-fg-2 hover:text-fg-1',
        // Renaming keeps the hover fill, as on a session row. select-none as on ui/Row (see the session row), off while renaming
        editing ? 'bg-hover' : 'select-none',
        dragging && 'opacity-40',
        fresh && 'row-flash',
      )}
    >
      {project && (
        // The folder shuts and opens in place with the fold. A pixel low: centred on the line box, the folder sat high
        // beside the bold name's lowercase letters
        <span title={hostTitle || undefined} className="relative -mr-gap-half flex size-lead shrink-0 items-center justify-center text-fg-3 *:absolute *:size-icon *:translate-y-px *:transition-opacity *:duration-200">
          {host ? <>
            <RemoteFolderIcon pulse={hostState === 'connecting'} className={open ? 'opacity-0' : 'opacity-100'} />
            <RemoteFolderIcon open pulse={hostState === 'connecting'} className={open ? 'opacity-100' : 'opacity-0'} />
          </> : <>
            <Folder strokeWidth={1.5} className={open ? 'opacity-0' : 'opacity-100'} />
            <FolderOpen strokeWidth={1.5} className={open ? 'opacity-100' : 'opacity-0'} />
          </>}
        </span>
      )}
      {sub && (
        // Softer folder at the same icon box as project folders, shutting and opening in place with the fold
        <span className="relative -mr-gap-half flex size-lead shrink-0 items-center justify-center text-fg-3/75 transition-colors group-hover:text-fg-1 group-focus-visible:text-fg-1 group-data-[menu-open]:text-fg-1 *:absolute *:size-icon *:translate-y-px *:transition-opacity *:duration-200">
          <Folder strokeWidth={1.5} className={open ? 'opacity-0' : 'opacity-100'} />
          <FolderOpen strokeWidth={1.5} className={open ? 'opacity-100' : 'opacity-0'} />
        </span>
      )}
      {editing && onRename
        ? <RenameInput initial={name} label={t('session.category.rename')} onDone={onRename} />
        : <span className={cn('min-w-0 truncate', sub && 'font-medium text-fg-2 group-hover:text-fg-1 group-focus-visible:text-fg-1 group-data-[menu-open]:text-fg-1')}>{name}</span>}
      {badge && !editing && <span className="shrink-0 rounded-sm bg-active px-1 text-3 font-normal text-fg-3">{badge}</span>}
      {!editing && (
        <span className={cn(STACK, 'ml-auto shrink-0 items-center text-3 font-normal text-fg-3 tabular-nums')}>
          <span className={cn('flex items-center justify-end gap-1.5', actions && CONCEAL)}>
            {preview && <span className="flex items-center gap-1 opacity-70 empty:hidden">{preview}</span>}
            {state && <StateMark {...state} />}
            {host && <>
              <span title={hostTitle || host.alias} className="max-w-host truncate text-fg-3/70">{host.alias}</span>
              <span title={hostTitle || host.alias} className={cn('size-1.5 shrink-0 rounded-full',
                hostState === 'connected' ? 'bg-ok'
                  : hostState === 'error' ? 'bg-warn'
                  : hostState === 'connecting' ? 'bg-fg-3 remote-breathe'
                  : 'bg-fg-3/40')} />
            </>}
            {count !== undefined && <span className="text-right">{count}</span>}
          </span>
          {actions && (
            // justify-self-end: only as wide as the buttons. Stretched to the cell, which a wide resting tail (an SSH
            // host's name) sets, the span's stopPropagation swallowed clicks on the blank beside them and the row never toggled
            <span className={cn('flex items-center justify-end justify-self-end gap-0.5', REVEAL)} onClick={e => e.stopPropagation()} onKeyDown={e => e.stopPropagation()}>
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
export function ChannelFilter({ agents, value, onChange, compact }: { agents: AgentInfo[]; value?: string; onChange: (id: string | undefined) => void; compact?: boolean }) {
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
