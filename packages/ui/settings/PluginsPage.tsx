import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { Settings2 } from 'lucide-react';
import { pluginKinds, PLUGIN_KINDS, type PluginInfo, type PluginKind } from '@acpira/shared/plugins';
import type { AgentId, AgentInfo } from '@acpira/shared/transcript';
import type { AgentInventory } from '@acpira/shared/inventory';
import type { AgentInstallProgress } from '@acpira/shared/protocol';
import { moveAgent } from '@acpira/shared/agentOrder';
import { BUILTIN_AGENT_ID } from '@acpira/shared/providers';
import { Card } from '../ui/Card';
import { IconButton } from '../ui/Button';
import { Switch } from '../ui/Switch';
import { Shimmer } from '../ui/Shimmer';
import { UpdateDot } from '../ui/UpdateDot';
import { cn } from '../ui/cn';
import { AgentMark } from '../chat/AgentMark';
import { t } from '../i18n';
import { ListHead, Note, SearchBox, Section, Tabs } from './controls';
import { askPluginsView, usePluginsView } from '../plugins/skin';
import { SkinPicker, UserPluginTile } from './UserPlugins';
import { PLUGIN_KIND_LABEL } from './search';

// Settings → Models: a tile per agent (the built-in one and any a plugin adds among them) with its switch (an off agent
// leaves the new-session entry points, acpira.disabledAgents) and its gear, which opens the agent's dialog with its
// models; dragging a tile (or Alt+arrow on it) reorders the agents everywhere they are listed (acpira.agentOrder).
// Settings → Plugins: every plugin the user has in one list, filtered by what it adds (shared/plugins.ts PLUGIN_KINDS);
// the skin in effect is picked at the head of the Appearance filter. Drafted in lab/plugin-home.preview.tsx

// The vendors of the agents Acpira knows by id; any other agent is described as an ACP agent
const VENDOR: Partial<Record<AgentId, string>> = {
  claude: 'Anthropic', codex: 'OpenAI', grok: 'xAI', devin: 'Cognition', kimi: 'Moonshot AI', antigravity: 'Google', dsh: 'DeepSeek',
};

export interface ModelsPageProps {
  // The host's list as received (its identity changes only when the host pushes a new one)
  agents: AgentInfo[];
  inventories: Partial<Record<AgentId, AgentInventory>>;
  // The agent tiles' own order, as the full list of visible ids (stored as acpira.agentOrder)
  onReorder: (ids: AgentId[]) => void;
  // Every visible agent that is now off
  onDisabled: (ids: AgentId[]) => void;
  // A tile's gear: the agent's dialog
  onOpen: (id: AgentId) => void;
  // Ask for an agent's inventory (its version for the tile's foot); a file scan, no process is started
  onInventory: (id: AgentId) => void;
  // Each agent's latest in-app install: an update running on a tile shows there
  installs?: AgentInstallProgress[];
  // A tile's update action: the agent's install line, which pins the newer version (AgentInfo.update)
  onUpdate: (id: AgentId) => void;
}

export function ModelsPage({ agents: all, inventories, onReorder, onDisabled, onOpen, onInventory, installs, onUpdate }: ModelsPageProps) {
  const agents = all.filter(a => !a.external);
  const [query, setQuery] = useState('');
  // The versions on the tiles' feet: every agent not scanned yet, once per visit
  const asked = useRef(new Set<AgentId>());
  useEffect(() => {
    for (const a of agents) if (!inventories[a.id] && !asked.current.has(a.id)) { asked.current.add(a.id); onInventory(a.id); }
  });
  const q = query.trim().toLowerCase();
  const match = (a: AgentInfo) => !q || a.name.toLowerCase().includes(q) || a.id.toLowerCase().includes(q);
  return <>
    <SearchBox value={query} onChange={setQuery} label={t('settings.models.search')} />
    <AgentTiles agents={all} inventories={inventories} filter={q ? match : undefined} onReorder={onReorder} onDisabled={onDisabled} onOpen={onOpen}
      installs={installs} onUpdate={onUpdate} />
  </>;
}

type Filter = 'all' | PluginKind;
const themed = (p: PluginInfo) => p.contributes.includes('theme');

// `tab`: the open filter, part of the page so the rail's search can land on one
export function PluginsPage({ tab, onTab }: { tab?: string; onTab: (tab: Filter) => void }) {
  const [query, setQuery] = useState('');
  // The user's plugins come from the engine's plugins view, asked for once
  useEffect(() => askPluginsView(), []);
  const view = usePluginsView();
  const q = query.trim().toLowerCase();
  const matchPlugin = (p: PluginInfo) => !q || p.name.toLowerCase().includes(q) || p.id.toLowerCase().includes(q) || p.description.toLowerCase().includes(q);
  const all = (view?.plugins ?? []).filter(matchPlugin);
  // A filter shows only while some plugin carries it
  const tabs: { value: Filter; label: string; count: number }[] = [{ value: 'all', label: t('common.all'), count: all.length },
    ...PLUGIN_KINDS.map(k => ({ value: k.kind, label: t(PLUGIN_KIND_LABEL[k.kind]), count: all.filter(p => pluginKinds(p.contributes).includes(k.kind)).length })).filter(k => k.count > 0)];
  const current = tabs.find(x => x.value === tab)?.value ?? 'all';
  const shown = current === 'all' ? all : all.filter(p => pluginKinds(p.contributes).includes(current));
  // Under Appearance a skin shows once, as a choice in the picker; its tile (version, switch, folder) stays under All
  const skins = view ? view.plugins.filter(p => themed(p) && p.loaded && matchPlugin(p)).length : 0;
  const picking = current === 'look' && !!view && (skins > 0 || !!view.skin);
  const tiles = picking ? shown.filter(p => !themed(p)) : shown;
  return <>
    <SearchBox value={query} onChange={setQuery} label={t('settings.plugins.search')} />
    {tabs.length > 1 && <Tabs items={tabs} value={current} onChange={onTab} label={t('settings.plugins.kinds')} />}
    {picking && view && <>
      <ListHead title={t('settings.plugins.skins')} count={skins} outside />
      <SkinPicker view={view} filter={matchPlugin} />
      {tiles.length > 0 && <ListHead title={t('settings.plugins.styles')} count={tiles.length} outside />}
    </>}
    {view && !picking && tiles.length === 0 && <Section><Note>{t(q ? 'settings.plugins.noMatch' : 'settings.plugins.none')}</Note></Section>}
    {tiles.length > 0 && (
      <div className="grid grid-cols-2 gap-gap settings-narrow:grid-cols-1">
        {tiles.map(p => <UserPluginTile key={p.id} plugin={p} safeMode={!!view?.safeMode} />)}
      </div>
    )}
  </>;
}

// Past this many pixels a press on a tile becomes a drag; below it the press stays a click
const DRAG_SLOP = 4;
// A tile sliding to a new slot: the session list's reorder curve (chat/SessionList.tsx), a touch longer for tiles
const SLIDE_EASE = 'cubic-bezier(.2,.7,.2,1)';
const SLIDE_MS = 280;

// The px offsets of a computed `translate` ("none", "Xpx" or "Xpx Ypx")
function translateOf(el: HTMLElement): { x: number; y: number } {
  const [x = 0, y = 0] = getComputedStyle(el).translate.split(' ').map(v => parseFloat(v) || 0);
  return { x, y };
}

// The agent tiles: drag a tile (or Alt+← / Alt+→ on its gear) to reorder, the switch turns the agent on or off. Both
// write through the host and come back as a rearranged list; until then the tiles show the local result so nothing
// snaps back. The last agent on cannot be turned off.
// A dragged tile follows the pointer (an inline translate over its slot in the previewed order); the others slide into
// their new slots, and on release the dragged one slides from under the pointer into its own
function AgentTiles({ agents: all, inventories, filter, onReorder, onDisabled, onOpen, installs, onUpdate }: {
  agents: AgentInfo[];
  inventories: Partial<Record<AgentId, AgentInventory>>;
  installs?: AgentInstallProgress[];
  onUpdate: (id: AgentId) => void;
  // A search narrows the tiles; reordering waits until it is cleared
  filter?: (a: AgentInfo) => boolean;
  onReorder: (ids: AgentId[]) => void;
  onDisabled: (ids: AgentId[]) => void;
  onOpen: (id: AgentId) => void;
}) {
  const agents = all.filter(a => !a.external);
  const ids = agents.map(a => a.id);
  const [pending, setPending] = useState<{ order?: AgentId[]; enabled?: Record<AgentId, boolean> }>({});
  // A fresh list from the host supersedes whatever was shown optimistically
  useEffect(() => setPending({}), [all]);
  const [drag, setDrag] = useState<{ id: AgentId; order: AgentId[] }>();
  // The tile that looks picked up: from the drag's start until it has slid into its slot after the release
  const [lifted, setLifted] = useState<AgentId>();
  // The press: where it started and the pointer is now, and the pressed tile's slot then (layout offsets, which a
  // translate does not move)
  const press = useRef<{
    id: AgentId; x: number; y: number; at: { x: number; y: number }; from: { x: number; y: number };
    others: { id: AgentId; rect: DOMRect }[];
  } | undefined>(undefined);
  const swallowClick = useRef(false);
  const grid = useRef<HTMLDivElement>(null);
  const tileEls = useRef(new Map<AgentId, HTMLDivElement>());
  // Each tile's slot as of the last commit, and what decided the slots then; slides run only when that changed, so a
  // resize or an inventory update never replays one from stale slots
  const slots = useRef(new Map<AgentId, { x: number; y: number }>());
  const lastLayout = useRef('');

  // The committed order (host list, or the local result waiting for it); a drag only previews over it
  const settled = pending.order ?? ids;
  const order = drag?.order ?? settled;
  const byId = new Map(agents.map(a => [a.id, a]));
  const shown = order.map(id => byId.get(id)).filter((a): a is AgentInfo => !!a);
  const enabled = (a: AgentInfo) => pending.enabled?.[a.id] ?? !a.disabled;
  const enabledCount = shown.filter(enabled).length;
  const visible = filter ? shown.filter(filter) : shown;

  // Keep the dragged tile under the pointer wherever its slot has moved to
  const follow = () => {
    const p = press.current;
    const el = p && tileEls.current.get(p.id);
    if (!p || !el) return;
    el.style.translate = `${p.at.x - p.x - (el.offsetLeft - p.from.x)}px ${p.at.y - p.y - (el.offsetTop - p.from.y)}px`;
  };

  useLayoutEffect(() => {
    const layout = `${drag?.id ?? ''}|${visible.map(a => a.id).join()}`;
    const changed = layout !== lastLayout.current;
    lastLayout.current = layout;
    const box = grid.current;
    // A tile filtered out loses its slot, so it comes back without a slide
    for (const id of slots.current.keys()) if (!tileEls.current.has(id)) slots.current.delete(id);
    const still = !box || !!box.closest('[data-motion="none"]') || !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    for (const [id, el] of tileEls.current) {
      const was = slots.current.get(id);
      const now = { x: el.offsetLeft, y: el.offsetTop };
      slots.current.set(id, now);
      if (!changed || id === drag?.id || !was) continue;
      // Only the tile just let go carries an inline translate
      const released = !!el.style.translate;
      if (!released && was.x === now.x && was.y === now.y) continue;
      // Where the tile shows now, measured from its new slot: its old slot plus any slide or drag offset still on it
      const cur = translateOf(el);
      for (const a of el.getAnimations()) if (a.id === 'tile-slide') a.cancel();
      el.style.translate = '';
      const dx = was.x + cur.x - now.x;
      const dy = was.y + cur.y - now.y;
      const land = () => setLifted(l => l === id ? undefined : l);
      if (still || Math.hypot(dx, dy) < 0.5) {
        if (released) land();
        continue;
      }
      const slide = el.animate([{ translate: `${dx}px ${dy}px` }, { translate: '0 0' }], { id: 'tile-slide', duration: SLIDE_MS, easing: SLIDE_EASE });
      // The tile let go keeps its lifted look, above the rest, until it lands
      if (released) slide.onfinish = slide.oncancel = land;
    }
    if (drag) follow();
  });

  const commit = (next: AgentId[]) => {
    if (next.join('\n') === settled.join('\n')) return;
    setPending(p => ({ ...p, order: next }));
    onReorder(next);
  };
  // Built from what the tiles show, so a second switch flipped before the host answered the first keeps both
  const toggle = (id: AgentId, on: boolean) => {
    setPending(p => ({ ...p, enabled: { ...p.enabled, [id]: on } }));
    onDisabled(agents.filter(a => a.id === id ? !on : !enabled(a)).map(a => a.id));
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>, id: AgentId) => {
    if (filter || e.button !== 0 || (e.target as Element).closest('[data-no-drag]')) return;
    const el = e.currentTarget;
    // A tile caught mid-slide is picked up from its slot
    for (const a of el.getAnimations()) if (a.id === 'tile-slide') a.cancel();
    // The other tiles where they stand; the drop slot is how many of them come before the pointer in reading order
    const tiles = [...(grid.current?.querySelectorAll<HTMLElement>('[data-agent-tile]') ?? [])];
    press.current = {
      id, x: e.clientX, y: e.clientY, at: { x: e.clientX, y: e.clientY }, from: { x: el.offsetLeft, y: el.offsetTop },
      others: tiles.filter(t => t.dataset.agentTile !== id).map(t => ({ id: t.dataset.agentTile!, rect: t.getBoundingClientRect() })),
    };
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const p = press.current;
    if (!p || (!drag && Math.hypot(e.clientX - p.x, e.clientY - p.y) < DRAG_SLOP)) return;
    if (!drag) {
      e.currentTarget.setPointerCapture(e.pointerId);
      setLifted(p.id);
    }
    p.at = { x: e.clientX, y: e.clientY };
    // A tile comes before the pointer when its row is above the pointer, or it is on the pointer's row left of it
    const before = (r: DOMRect) => e.clientY >= r.bottom || (e.clientY >= r.top && e.clientX > r.left + r.width / 2);
    const slot = p.others.filter(o => before(o.rect)).length;
    const next = p.others.map(o => o.id);
    next.splice(slot, 0, p.id);
    if (next.join('\n') !== drag?.order.join('\n')) setDrag({ id: p.id, order: next });
    follow();
  };
  const onPointerEnd = (e: PointerEvent<HTMLDivElement>, drop: boolean) => {
    press.current = undefined;
    if (!drag) return;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    // The click that follows a drag's pointerup must not open the dialog
    swallowClick.current = true;
    setTimeout(() => { swallowClick.current = false; });
    setDrag(undefined);
    if (drop) commit(drag.order);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLElement>, id: AgentId) => {
    const step = e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : 0;
    if (!e.altKey || !step || filter) return;
    e.preventDefault();
    commit(moveAgent(order, id, order.indexOf(id) + step));
  };

  return (
    // Move and release are the grid's: React moves tile nodes while the order previews, which drops a tile's pointer
    // capture, so the capture sits on the grid, which stays put
    <div ref={grid} className="grid grid-cols-2 gap-gap settings-narrow:grid-cols-1"
      onPointerMove={onPointerMove}
      onPointerUp={e => onPointerEnd(e, true)}
      onPointerCancel={e => onPointerEnd(e, false)}
      onClickCapture={e => { if (swallowClick.current) { e.preventDefault(); e.stopPropagation(); } }}>
      {visible.map(a => {
        const on = enabled(a);
        const last = on && enabledCount === 1;
        return (
          <div
            key={a.id}
            ref={el => { if (el) tileEls.current.set(a.id, el); else tileEls.current.delete(a.id); }}
            data-agent-tile={a.id}
            onPointerDown={e => onPointerDown(e, a.id)}
            className={cn('relative min-w-0 touch-none select-none', lifted === a.id && 'z-10')}
          >
            <AgentTile agent={a} on={on} last={last} dragging={lifted === a.id} inventory={inventories[a.id]}
              updating={installs?.some(i => i.agent === a.id && i.status === 'running')} onUpdate={() => onUpdate(a.id)}
              onToggle={v => toggle(a.id, v)} onOpen={() => onOpen(a.id)} onKeyDown={e => onKeyDown(e, a.id)} />
          </div>
        );
      })}
    </div>
  );
}

// One agent: its mark in a square, name and what it is, the switch; the foot names where the module comes from, the
// gear and the version (or that it is not installed). A click anywhere on the tile but its switch opens the dialog too
function AgentTile({ agent: a, on, last, dragging, inventory, updating, onUpdate, onToggle, onOpen, onKeyDown }: {
  agent: AgentInfo;
  updating?: boolean;
  onUpdate: () => void;
  on: boolean;
  last: boolean;
  dragging: boolean;
  inventory?: AgentInventory;
  onToggle: (on: boolean) => void;
  onOpen: () => void;
  onKeyDown: (e: KeyboardEvent<HTMLElement>) => void;
}) {
  const vendor = VENDOR[a.id] ?? a.vendor;
  const desc = a.id === BUILTIN_AGENT_ID ? t('settings.plugins.builtinDesc') : vendor ? t('settings.plugins.officialCli', { vendor }) : t('settings.plugins.acpAgent');
  const version = a.available === false ? t('settings.plugins.notInstalled') : inventory?.runtime?.version ?? inventory?.adapter?.adapter?.version;
  const settings = t('settings.plugins.settings', { name: a.name });
  return (
    <Card onClick={e => { if (!(e.target as Element).closest('[data-no-drag]')) onOpen(); }}
      className={cn('flex h-full flex-col shadow-none transition-[background-color,box-shadow,scale] hover:bg-hover', dragging ? 'scale-102 cursor-grabbing bg-bg-2 shadow-pop' : 'cursor-pointer')}>
      <div className="flex items-center gap-gap p-pad">
        <span className={cn('flex size-(--tile-mark) shrink-0 items-center justify-center rounded-lg bg-active text-fg-1', !on && 'text-fg-3')}>
          <AgentMark id={a.id} name={a.name} className="size-lead" />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-y-(--stack-gap)">
          <span className={cn('truncate text-2 font-medium', on ? 'text-fg-1' : 'text-fg-2')}>{a.name}</span>
          <span className="truncate text-3 text-fg-3">{desc}</span>
        </span>
        <span data-no-drag className="flex shrink-0 items-center">
          <Switch checked={on} disabled={last} onCheckedChange={onToggle}
            aria-label={t('settings.agent.enabled', { agent: a.name })}
            title={last ? t('settings.agent.lastEnabled') : t('settings.agent.enabled', { agent: a.name })} />
        </span>
      </div>
      {/* The version on the left, after Built in for the agent that ships inside Acpira; the gear alone on the right, so it
          lines up across tiles whatever the version's length, its glyph on the switch's right edge */}
      <div className="mt-auto flex h-(--tile-foot) items-center justify-between gap-gap border-t border-line px-pad">
        <span className="flex min-w-0 items-baseline gap-2 text-3 text-fg-3">
          {a.id === BUILTIN_AGENT_ID && <span className="shrink-0">{t('settings.plugins.bundled')}</span>}
          {version && <span className="truncate font-mono tabular-nums">{version}</span>}
        </span>
        <span data-no-drag className="-mr-ctl-sm-bleed flex shrink-0 items-center">
          {/* A newer pinned adapter: the update action beside the gear, its dot breathing like an unread session; while the
              install runs the same slot says so */}
          {updating
            ? <Shimmer className="max-w-full truncate px-2 text-3 whitespace-nowrap text-fg-2">{t('settings.plugins.updating')}</Shimmer>
            : a.update && (
              <button type="button" onClick={onUpdate}
                className="flex h-ctl-sm items-center gap-2 rounded-md px-2 text-3 text-fg-1 transition-colors hover:bg-hover focus-visible:bg-hover">
                <UpdateDot />
                <UpdateLabel version={a.update} />
              </button>
            )}
          <IconButton size="sm" title={settings} aria-label={settings} aria-haspopup="dialog" aria-keyshortcuts="Alt+ArrowLeft Alt+ArrowRight"
            onClick={onOpen} onKeyDown={onKeyDown} className="text-fg-2">
            <Settings2 strokeWidth={1.75} />
          </IconButton>
        </span>
      </div>
    </Card>
  );
}

// "Update to 0.87.0" with the version in the foot's monospace figures, wherever the language puts it
function UpdateLabel({ version }: { version: string }) {
  const [before, after = ''] = t('settings.plugins.update', { version: '\u0000' }).split('\u0000');
  return <span>{before}<span className="font-mono tabular-nums">{version}</span>{after}</span>;
}
