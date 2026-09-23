import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { ArrowLeft, Palette, Settings2 } from 'lucide-react';
import type { AgentId, AgentInfo } from '@shared/transcript';
import { moveAgent } from '@shared/agentOrder';
import { cn } from '../ui/cn';
import { Switch } from '../ui/Switch';
import { AgentMark } from '../chat/AgentMark';
import { t } from '../i18n';

export type SettingsPage = { kind: 'chatgpt' } | { kind: 'general' } | { kind: 'appearance' } | { kind: 'agent'; id: AgentInfo['id'] };

// Fixed pages first, then one page per agent; agent ids never collide with the fixed names
const FIXED = ['general', 'appearance', 'chatgpt'] as const;
type FixedId = (typeof FIXED)[number];
type PageId = FixedId | AgentInfo['id'];
const isFixed = (id: PageId): id is FixedId => (FIXED as readonly string[]).includes(id);
const pageId = (p: SettingsPage): PageId => (p.kind === 'agent' ? p.id : p.kind);
const toPage = (id: PageId): SettingsPage => (isFixed(id) ? { kind: id } : { kind: 'agent', id });

export interface PageRailProps {
  agents: AgentInfo[];
  page: SettingsPage;
  onPage: (p: SettingsPage) => void;
  onBack: () => void;
  // The agent rows' own order, as the full list of visible ids (stored as acpira.agentOrder)
  onReorder: (ids: AgentId[]) => void;
  // The row switches: every visible agent that is now off; an off agent leaves the new-session entry points (acpira.disabledAgents)
  onDisabled: (ids: AgentId[]) => void;
}

// Full-width navigation on wide surfaces collapses to an icon rail in narrow webviews (the shell is the container).
// Back navigation stays in this column and never consumes space beside the page heading.
const navItem = 'flex h-ctl shrink-0 items-center gap-gap rounded-md px-2 text-2 transition-colors @max-[600px]/settings-shell:justify-center @max-[600px]/settings-shell:px-0';
const tone = (active: boolean, dim?: boolean) => active ? 'text-fg-1' : dim ? 'text-fg-3' : 'text-fg-2 hover:text-fg-1 focus-visible:text-fg-1';

function RailIcon({ children }: { children: ReactNode }) {
  return <span className="flex size-icon-ctl shrink-0 items-center justify-center [&_svg]:size-icon-ctl">{children}</span>;
}

export function PageRail({ agents, page, onPage, onBack, onReorder, onDisabled }: PageRailProps) {
  const cur = pageId(page);
  const item = (id: PageId, name: string, icon: ReactNode) => (
    <button
      key={id}
      type="button"
      title={name}
      aria-label={name}
      aria-current={cur === id ? 'page' : undefined}
      onClick={() => onPage(toPage(id))}
      className={cn(navItem, tone(cur === id), cur === id ? 'bg-active' : 'hover:bg-hover focus-visible:bg-hover')}
    >
      <RailIcon>{icon}</RailIcon>
      <span className="min-w-0 truncate @max-[600px]/settings-shell:hidden">{name}</span>
    </button>
  );
  return (
    <aside className="flex w-(--settings-nav-w) shrink-0 flex-col gap-(--section-gap) px-pad py-pad-y @max-[600px]/settings-shell:w-(--settings-rail-w) @max-[600px]/settings-shell:px-2">
      <button type="button" onClick={onBack} title={t('settings.back')} aria-label={t('settings.back')}
        className={cn(navItem, 'text-fg-2 hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 active:text-fg-1')}>
        <ArrowLeft className="size-icon-ctl shrink-0" strokeWidth={1.5} aria-hidden />
        <span className="truncate @max-[600px]/settings-shell:hidden">{t('settings.back')}</span>
      </button>
      <nav aria-label={t('settings.title')} className="flex min-h-0 flex-col gap-1 overflow-y-auto">
        {item('general', t('settings.nav.general'), <Settings2 strokeWidth={1.5} />)}
        {item('appearance', t('settings.nav.appearance'), <Palette strokeWidth={1.5} />)}
        <AgentRows agents={agents} cur={cur} onOpen={id => onPage(toPage(id))} onReorder={onReorder} onDisabled={onDisabled} />
        {agents.some(a => a.id === 'chatgpt' && a.external) && item('chatgpt', 'ChatGPT', <AgentMark id="chatgpt" name="ChatGPT" />)}
      </nav>
    </aside>
  );
}

// Past this many pixels a press on a row becomes a drag; below it the press stays a click
const DRAG_SLOP = 4;

// The agent rows: drag a row (or Alt+↑ / Alt+↓ on it) to reorder, the switch at the end turns the agent on or off.
// Both write through the host and come back as a rearranged list; until then the rows show the local result so nothing snaps back.
// The icon rail keeps reordering but has no room for the switch
function AgentRows({ agents: all, cur, onOpen, onReorder, onDisabled }: {
  // The host's list as received (its identity changes only when the host pushes a new one)
  agents: AgentInfo[];
  cur: PageId;
  onOpen: (id: AgentId) => void;
  onReorder: (ids: AgentId[]) => void;
  onDisabled: (ids: AgentId[]) => void;
}) {
  const agents = all.filter(a => !a.external);
  const ids = agents.map(a => a.id);
  const [pending, setPending] = useState<{ order?: AgentId[]; enabled?: Record<AgentId, boolean> }>({});
  // A fresh list from the host supersedes whatever was shown optimistically
  useEffect(() => setPending({}), [all]);
  const [drag, setDrag] = useState<{ id: AgentId; order: AgentId[] }>();
  const press = useRef<{ id: AgentId; y: number; mids: number[]; others: AgentId[] } | undefined>(undefined);
  const swallowClick = useRef(false);
  const list = useRef<HTMLDivElement>(null);

  // The committed order (host list, or the local result waiting for it); a drag only previews over it
  const settled = pending.order ?? ids;
  const order = drag?.order ?? settled;
  const byId = new Map(agents.map(a => [a.id, a]));
  const shown = order.map(id => byId.get(id)).filter((a): a is AgentInfo => !!a);
  const enabled = (a: AgentInfo) => pending.enabled?.[a.id] ?? !a.disabled;
  const enabledCount = shown.filter(enabled).length;

  const commit = (next: AgentId[]) => {
    if (next.join('\n') === settled.join('\n')) return;
    setPending(p => ({ ...p, order: next }));
    onReorder(next);
  };
  // Built from what the rows show, so a second switch flipped before the host answered the first keeps both
  const toggle = (id: AgentId, on: boolean) => {
    setPending(p => ({ ...p, enabled: { ...p.enabled, [id]: on } }));
    onDisabled(agents.filter(a => a.id === id ? !on : !enabled(a)).map(a => a.id));
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>, id: AgentId) => {
    if (e.button !== 0 || (e.target as Element).closest('[data-no-drag]')) return;
    // Midlines of the other rows in the order on screen; the drop slot is how many of them sit above the pointer
    const rows = [...(list.current?.querySelectorAll<HTMLElement>('[data-agent-row]') ?? [])];
    const others = rows.filter(r => r.dataset.agentRow !== id);
    press.current = { id, y: e.clientY, others: others.map(r => r.dataset.agentRow!), mids: others.map(r => { const b = r.getBoundingClientRect(); return b.top + b.height / 2; }) };
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const p = press.current;
    if (!p || (!drag && Math.abs(e.clientY - p.y) < DRAG_SLOP)) return;
    if (!drag) e.currentTarget.setPointerCapture(e.pointerId);
    const slot = p.mids.filter(mid => mid < e.clientY).length;
    const next = [...p.others];
    next.splice(slot, 0, p.id);
    if (next.join('\n') !== drag?.order.join('\n')) setDrag({ id: p.id, order: next });
  };
  const onPointerEnd = (e: PointerEvent<HTMLDivElement>, drop: boolean) => {
    press.current = undefined;
    if (!drag) return;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    // The click that follows a drag's pointerup must not open the page
    swallowClick.current = true;
    setTimeout(() => { swallowClick.current = false; });
    setDrag(undefined);
    if (drop) commit(drag.order);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>, id: AgentId) => {
    if (!e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
    e.preventDefault();
    commit(moveAgent(order, id, order.indexOf(id) + (e.key === 'ArrowUp' ? -1 : 1)));
  };

  return (
    <div ref={list} className="flex flex-col gap-1">
      {shown.map(a => {
        const active = cur === a.id;
        const on = enabled(a);
        const last = on && enabledCount === 1;
        return (
          <div
            key={a.id}
            data-agent-row={a.id}
            onPointerDown={e => onPointerDown(e, a.id)}
            onPointerMove={onPointerMove}
            onPointerUp={e => onPointerEnd(e, true)}
            onPointerCancel={e => onPointerEnd(e, false)}
            onClickCapture={e => { if (swallowClick.current) { e.preventDefault(); e.stopPropagation(); } }}
            className={cn(
              'flex shrink-0 items-center rounded-md transition-colors',
              drag?.id === a.id ? 'relative z-10 cursor-grabbing bg-active shadow-[0_1px_4px_var(--line-strong)]' : active ? 'bg-active' : 'hover:bg-hover',
            )}
          >
            <button
              type="button"
              title={a.name}
              aria-label={a.name}
              aria-current={active ? 'page' : undefined}
              aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
              onClick={() => onOpen(a.id)}
              onKeyDown={e => onKeyDown(e, a.id)}
              className={cn(navItem, 'min-w-0 flex-1 outline-none focus-visible:bg-hover', tone(active, a.available === false || !on))}
            >
              <RailIcon><AgentMark id={a.id} name={a.name} /></RailIcon>
              <span className="min-w-0 truncate @max-[600px]/settings-shell:hidden">{a.name}</span>
            </button>
            <span data-no-drag className="flex shrink-0 items-center pr-2 @max-[600px]/settings-shell:hidden">
              <Switch
                skin="compact"
                checked={on}
                disabled={last}
                onCheckedChange={v => toggle(a.id, v)}
                aria-label={t('settings.agent.enabled', { agent: a.name })}
                title={last ? t('settings.agent.lastEnabled') : t('settings.agent.enabled', { agent: a.name })}
              />
            </span>
          </div>
        );
      })}
    </div>
  );
}
