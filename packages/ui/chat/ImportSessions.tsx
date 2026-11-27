import { useEffect, useState } from 'react';
import { Check, Import } from 'lucide-react';
import type { AgentId, AgentInfo, NativeSessionInfo } from '@acpira/shared/transcript';
import type { NativeSessionsState } from '@acpira/shared/protocol';
import { launchable } from '@acpira/shared/agentOrder';
import { cn } from '../ui/cn';
import { IconButton } from '../ui/Button';
import { PanelHeader } from '../ui/Panel';
import { Popover } from '../ui/Popover';
import { Row } from '../ui/Row';
import { t, useLocale } from '../i18n';
import { AgentMark } from './AgentMark';
import { NAV_LEAD, NAV_ROW } from './NavRows';
import { startOfDay, fmtTime } from './sessionTime';
import { SECTION_ACT, ChannelFilter } from './SessionListSections';

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
          <div className="scroll-none flex max-h-pop flex-col overflow-y-auto" role="listbox" aria-label={t('session.import.title', { agent: name })}>
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
