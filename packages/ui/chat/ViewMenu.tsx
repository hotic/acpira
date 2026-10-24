import type { ReactNode } from 'react';
import { ArrowDownWideNarrow, Calendar, ChevronRight, Circle, CircleDot, Clock, Eye, Folder, Layers, List, ListFilter, LoaderCircle, MailCheck } from 'lucide-react';
import type { AgentInfo } from '@acpira/shared/transcript';
import { cn } from '../ui/cn';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { t } from '../i18n';
import { AgentMark } from './AgentMark';
import { clearFilters, filterCount, VIEW_GROUPS, VIEW_ORDERS, VIEW_PER, VIEW_SINCE, VIEW_STATUSES, type ListView, type ViewGroup, type ViewStatus } from './sessionView';

const separator = <DropdownMenu.Separator className="my-1 h-px bg-line" />;
const GROUP_ICON: Record<ViewGroup, ReactNode> = {
  project: <Folder strokeWidth={1.5} />, time: <Clock strokeWidth={1.5} />, agent: <Layers strokeWidth={1.5} />, none: <List strokeWidth={1.5} />,
};
// Each status wears the mark its rows carry in the list (SessionList's StateMark), at rest
const STATUS_MARK: Record<ViewStatus, ReactNode> = {
  working: <LoaderCircle className="text-fg-2" strokeWidth={2.5} />,
  waiting: <span className="size-1.5 rounded-full bg-warn" />,
  unread: <span className="size-unread-dot rounded-full bg-fg-strong" />,
  error: <span className="size-1.5 rounded-full bg-danger" />,
  background: <Circle className="text-fg-3" strokeWidth={2.5} />,
};

// A row opening a nested menu: its label, then the current value (or a dot while a filter inside is on), then the chevron.
// `side`: where the nested menu opens, the same side as the menu itself
function Sub({ icon, label, value, dot, side, children }: { icon: ReactNode; label: string; value?: string; dot?: boolean; side: 'inline-end' | 'inline-start'; children: ReactNode }) {
  return (
    <DropdownMenu.SubmenuRoot>
      <DropdownMenu.SubmenuTrigger>
        <OptionContent icon={icon}>{label}</OptionContent>
        {value && <span className="min-w-0 max-w-1/2 shrink truncate text-3 text-fg-3">{value}</span>}
        {dot && <span aria-hidden className="size-1.5 shrink-0 rounded-full bg-accent" />}
        <ChevronRight className="size-3 shrink-0 text-fg-3" strokeWidth={1.5} />
      </DropdownMenu.SubmenuTrigger>
      <DropdownMenu.Portal><DropdownMenu.Positioner side={side} align="start" width="sm"><DropdownMenu.Popup>
        <div className="scroll-thin flex max-h-pop flex-col overflow-y-auto">{children}</div>
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.SubmenuRoot>
  );
}

// One choice; picks leave the menus open (the list changes behind them), so several filters go on in one visit
function Pick({ icon, checked, onPick, children }: { icon?: ReactNode; checked: boolean; onPick: () => void; children: ReactNode }) {
  return <DropdownMenu.Item closeOnClick={false} onClick={onPick}><OptionContent icon={icon} checked={checked} checkSlot>{children}</OptionContent></DropdownMenu.Item>;
}

const toggle = <T,>(list: T[], v: T) => (list.includes(v) ? list.filter(x => x !== v) : [...list, v]);

export interface ViewMenuProps {
  view: ListView;
  onChange: (view: ListView) => void;
  // The channels to offer (the agents the list's sessions ran)
  agents: AgentInfo[];
  // Desktop: the menu floats to the right of the column, over the chat, so a narrow column never squeezes it (Cursor's
  // placement); the per-project count applies there only. Elsewhere it hangs under its button, ending where it does
  desktop?: boolean;
  onMarkAllRead?: () => void;
  // The trigger's own classes (a section label's action, or the filter row's)
  className?: string;
}

// The session list's view menu: grouping, ordering and what rows show, then the filters (status, channel,
// date) with a reset, then "mark all as read". The trigger turns to the accent with a dot while any filter is on
export function ViewMenu({ view, onChange, agents, desktop, onMarkAllRead, className }: ViewMenuProps) {
  const n = filterCount(view);
  const set = (patch: Partial<ListView>) => onChange({ ...view, ...patch });
  const side = desktop ? 'inline-end' : 'inline-start';
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger render={
        <button type="button" data-view-menu title={t('session.view')} aria-label={t('session.view')}
          className={cn('relative', className, n > 0 && 'text-accent hover:text-accent data-[popup-open]:text-accent')}>
          <ListFilter className="size-icon" strokeWidth={1.5} />
          {n > 0 && <span aria-hidden className="absolute top-0.5 right-0.5 size-1.5 rounded-full bg-accent" />}
        </button>
      } />
      <DropdownMenu.Portal>
        <DropdownMenu.Positioner side={desktop ? 'inline-end' : 'bottom'} align={desktop ? 'start' : 'end'} width="md">
          <DropdownMenu.Popup aria-label={t('session.view')}>
            <Sub side={side} icon={<Layers strokeWidth={1.5} />} label={t('session.view.group')} value={t(`session.view.group.${view.group}`)}>
              {VIEW_GROUPS.map(g => <Pick key={g} icon={GROUP_ICON[g]} checked={view.group === g} onPick={() => set({ group: g })}>{t(`session.view.group.${g}`)}</Pick>)}
            </Sub>
            <Sub side={side} icon={<ArrowDownWideNarrow strokeWidth={1.5} />} label={t('session.view.order')} value={t(`session.view.order.${view.order}`)}>
              {VIEW_ORDERS.map(o => <Pick key={o} checked={view.order === o} onPick={() => set({ order: o })}>{t(`session.view.order.${o}`)}</Pick>)}
            </Sub>
            <Sub side={side} icon={<Eye strokeWidth={1.5} />} label={t('session.view.show')}>
              {desktop && <>
                <DropdownMenu.Group>
                  <DropdownMenu.GroupLabel className="flex min-h-row items-center px-2 text-3 text-fg-3">{t('session.view.show.perProject')}</DropdownMenu.GroupLabel>
                  {VIEW_PER.map(k => <Pick key={k} checked={view.per === k} onPick={() => set({ per: k })}>{k ? t('session.view.show.latest', { n: k }) : t('session.view.show.all')}</Pick>)}
                </DropdownMenu.Group>
                {separator}
              </>}
              <Pick checked={view.model} onPick={() => set({ model: !view.model })}>{t('session.view.show.model')}</Pick>
            </Sub>
            {separator}
            <div className="flex min-h-row items-center px-2 text-3 text-fg-3">
              <span className="min-w-0 flex-1 truncate">{t('session.view.filters')}</span>
              <button type="button" disabled={!n} onClick={() => onChange(clearFilters(view))}
                className="rounded-sm px-1 transition-colors hover:text-fg-1 focus-visible:text-fg-1 disabled:pointer-events-none disabled:opacity-40">
                {t('session.view.reset')}
              </button>
            </div>
            <Sub side={side} icon={<CircleDot strokeWidth={1.5} />} label={t('session.view.status')} dot={view.status.length > 0}>
              {VIEW_STATUSES.map(s => <Pick key={s} icon={STATUS_MARK[s]} checked={view.status.includes(s)} onPick={() => set({ status: toggle(view.status, s) })}>{t(`session.view.status.${s}`)}</Pick>)}
            </Sub>
            <Sub side={side} icon={<Layers strokeWidth={1.5} />} label={t('session.view.channel')} dot={view.agents.length > 0}>
              {agents.map(a => <Pick key={a.id} icon={<AgentMark id={a.id} name={a.name} />} checked={view.agents.includes(a.id)} onPick={() => set({ agents: toggle(view.agents, a.id) })}>{a.name}</Pick>)}
            </Sub>
            <Sub side={side} icon={<Calendar strokeWidth={1.5} />} label={t('session.view.since')} value={view.since === 'all' ? undefined : t(`session.view.since.${view.since}`)}>
              {VIEW_SINCE.map(s => <Pick key={s} checked={view.since === s} onPick={() => set({ since: s })}>{t(`session.view.since.${s}`)}</Pick>)}
            </Sub>
            {onMarkAllRead && <>
              {separator}
              <DropdownMenu.Item onClick={onMarkAllRead}><OptionContent icon={<MailCheck strokeWidth={1.5} />}>{t('session.view.markAllRead')}</OptionContent></DropdownMenu.Item>
            </>}
          </DropdownMenu.Popup>
        </DropdownMenu.Positioner>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
