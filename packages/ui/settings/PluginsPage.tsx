import { Fragment, useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { BrainCircuit, FolderTree, Globe, Palette, PanelsTopLeft, Settings2, SquareTerminal, Workflow, Wrench } from 'lucide-react';
import type { PluginInfo, PluginLayer } from '@acpira/shared/plugins';
import type { AgentId, AgentInfo } from '@acpira/shared/transcript';
import type { AgentInventory } from '@acpira/shared/inventory';
import { moveAgent } from '@acpira/shared/agentOrder';
import { BUILTIN_AGENT_ID } from '@acpira/shared/providers';
import { Card } from '../ui/Card';
import { IconButton } from '../ui/Button';
import { Switch } from '../ui/Switch';
import { cn } from '../ui/cn';
import { AgentMark } from '../chat/AgentMark';
import { t } from '../i18n';
import { Count, SearchBox } from './controls';
import { askPluginsView, usePluginsView } from '../plugins/skin';
import { SkinPicker, UserPluginTile } from './UserPlugins';

// Settings → Plugins: what Acpira is built from, one tile per module in layers. The skins head the page (one at a time,
// settings/UserPlugins.tsx); then the agents: a tile per agent with its switch (an off agent leaves the new-session entry
// points, acpira.disabledAgents) and its gear, which opens the agent's dialog; dragging a tile (or Alt+arrow on it)
// reorders the agents everywhere they are listed (acpira.agentOrder). The user's plugins join the layer they declare,
// after the built-in modules. Laid out in lab/settings-all.preview.tsx

// The vendors of the agents Acpira knows by id; any other agent is described as an ACP agent
const VENDOR: Partial<Record<AgentId, string>> = {
  claude: 'Anthropic', codex: 'OpenAI', grok: 'xAI', devin: 'Cognition', kimi: 'Moonshot AI', antigravity: 'Google', dsh: 'DeepSeek',
};

export interface PluginsPageProps {
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
  // Built-in interface modules in the tool panel (`acpira.panelFiles`, `acpira.panelTerminal`, `acpira.panelBrowser`)
  panelFiles?: boolean;
  onPanelFiles?: (on: boolean) => void;
  panelTerminal?: boolean;
  onPanelTerminal?: (on: boolean) => void;
  panelBrowser?: boolean;
  onPanelBrowser?: (on: boolean) => void;
}

export function PluginsPage({ agents: all, inventories, onReorder, onDisabled, onOpen, onInventory, panelFiles, onPanelFiles, panelTerminal, onPanelTerminal, panelBrowser, onPanelBrowser }: PluginsPageProps) {
  const agents = all.filter(a => !a.external);
  const [query, setQuery] = useState('');
  // The user's plugins come from the engine's plugins view, asked for once
  useEffect(() => askPluginsView(), []);
  const view = usePluginsView();
  // The versions on the tiles' feet: every agent not scanned yet, once per visit
  const asked = useRef(new Set<AgentId>());
  useEffect(() => {
    for (const a of agents) if (!inventories[a.id] && !asked.current.has(a.id)) { asked.current.add(a.id); onInventory(a.id); }
  });
  const q = query.trim().toLowerCase();
  const match = (a: AgentInfo) => !q || a.name.toLowerCase().includes(q) || a.id.toLowerCase().includes(q);
  const matchedAgents = agents.filter(match);
  const builtinUi: { id: string; icon: ReactNode; name: string; desc: string; on: boolean; onToggle: (on: boolean) => void }[] = [
    ...(onPanelFiles ? [{ id: 'files', icon: <FolderTree className="size-lead" strokeWidth={1.5} />, name: t('settings.plugins.files'), desc: t('settings.plugins.filesDesc'), on: panelFiles !== false, onToggle: onPanelFiles }] : []),
    ...(onPanelTerminal ? [{ id: 'terminal', icon: <SquareTerminal className="size-lead" strokeWidth={1.5} />, name: t('settings.plugins.terminal'), desc: t('settings.plugins.terminalDesc'), on: panelTerminal !== false, onToggle: onPanelTerminal }] : []),
    ...(onPanelBrowser ? [{ id: 'browser', icon: <Globe className="size-lead" strokeWidth={1.5} />, name: t('settings.plugins.browser'), desc: t('settings.plugins.browserDesc'), on: panelBrowser !== false, onToggle: onPanelBrowser }] : []),
  ].filter(m => !q || m.name.toLowerCase().includes(q) || m.desc.toLowerCase().includes(q) || m.id.includes(q));
  const matchPlugin = (p: PluginInfo) => !q || p.name.toLowerCase().includes(q) || p.id.toLowerCase().includes(q) || p.description.toLowerCase().includes(q);
  const mine = (layer: PluginLayer) => (view?.plugins ?? []).filter(p => p.layer === layer && matchPlugin(p));
  const skinCount = view ? view.plugins.filter(p => p.contributes.includes('theme') && p.loaded && matchPlugin(p)).length : 0;
  const userTiles = (list: PluginInfo[]) => list.map(p => <UserPluginTile key={p.id} plugin={p} safeMode={!!view?.safeMode} />);
  // Each layer shown, in page order, so only the ones after the first draw the rule above them
  const layers: { key: string; node: (rule: boolean) => ReactNode }[] = [];
  if (view && (skinCount > 0 || view.skin)) {
    layers.push({ key: 'appearance', node: rule => (
      <Layer rule={rule} icon={<Palette />} title={t('settings.plugins.appearance')} count={skinCount}>
        <SkinPicker view={view} filter={matchPlugin} />
      </Layer>
    ) });
  }
  const agentPlugins = mine('agents');
  if (!q || matchedAgents.length > 0 || agentPlugins.length > 0) {
    layers.push({ key: 'agents', node: rule => (
      <Layer rule={rule} icon={<BrainCircuit />} title={t('settings.plugins.agents')} count={matchedAgents.length + agentPlugins.length}>
        <AgentTiles agents={all} inventories={inventories} filter={q ? match : undefined} onReorder={onReorder} onDisabled={onDisabled} onOpen={onOpen} />
        {agentPlugins.length > 0 && <div className="grid grid-cols-2 gap-gap @max-[600px]/settings-shell:grid-cols-1">{userTiles(agentPlugins)}</div>}
      </Layer>
    ) });
  }
  const tools = mine('tools');
  if (tools.length) {
    layers.push({ key: 'tools', node: rule => (
      <Layer rule={rule} icon={<Wrench />} title={t('settings.plugins.tools')} count={tools.length}>
        <div className="grid grid-cols-2 gap-gap @max-[600px]/settings-shell:grid-cols-1">{userTiles(tools)}</div>
      </Layer>
    ) });
  }
  const ui = mine('interface');
  if (builtinUi.length + ui.length > 0) {
    layers.push({ key: 'interface', node: rule => (
      <Layer rule={rule} icon={<PanelsTopLeft />} title={t('settings.plugins.interface')} count={builtinUi.length + ui.length}>
        <div className="grid grid-cols-2 gap-gap @max-[600px]/settings-shell:grid-cols-1">
          {builtinUi.map(m => <BuiltinTile key={m.id} {...m} />)}
          {userTiles(ui)}
        </div>
      </Layer>
    ) });
  }
  const automation = mine('automation');
  if (automation.length) {
    layers.push({ key: 'automation', node: rule => (
      <Layer rule={rule} icon={<Workflow />} title={t('settings.plugins.automation')} count={automation.length}>
        <div className="grid grid-cols-2 gap-gap @max-[600px]/settings-shell:grid-cols-1">{userTiles(automation)}</div>
      </Layer>
    ) });
  }
  return <>
    <SearchBox value={query} onChange={setQuery} label={t('settings.plugins.search')} />
    {layers.map((l, i) => <Fragment key={l.key}>{l.node(i > 0)}</Fragment>)}
  </>;
}

// One layer: its head (icon, title, count on the title's baseline), then the tiles. Layers after the first are parted
// from the one above by a rule fading out at both ends
function Layer({ icon, title, count, rule, children }: { icon: ReactNode; title: string; count: number; rule?: boolean; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      {rule && <span aria-hidden className="h-px w-full bg-linear-to-r from-transparent via-line to-transparent" />}
      <h2 className="m-0 flex min-h-ctl items-center gap-2 text-fg-2 [&>svg]:size-icon-ctl [&>svg]:stroke-[1.75]">
        {icon}
        {/* The count sits on the title's baseline: an icon inside the baseline box would pull it down to the icon's bottom */}
        <span className="flex items-baseline gap-2 text-(length:--text-h) leading-(--text-h-lh) font-medium text-fg-1">{title}<Count n={count} /></span>
      </h2>
      {children}
    </section>
  );
}

// Past this many pixels a press on a tile becomes a drag; below it the press stays a click
const DRAG_SLOP = 4;

// The agent tiles: drag a tile (or Alt+← / Alt+→ on its gear) to reorder, the switch turns the agent on or off. Both
// write through the host and come back as a rearranged list; until then the tiles show the local result so nothing
// snaps back. The last agent on cannot be turned off
function AgentTiles({ agents: all, inventories, filter, onReorder, onDisabled, onOpen }: {
  agents: AgentInfo[];
  inventories: Partial<Record<AgentId, AgentInventory>>;
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
  const press = useRef<{ id: AgentId; x: number; y: number; others: { id: AgentId; rect: DOMRect }[] } | undefined>(undefined);
  const swallowClick = useRef(false);
  const grid = useRef<HTMLDivElement>(null);

  // The committed order (host list, or the local result waiting for it); a drag only previews over it
  const settled = pending.order ?? ids;
  const order = drag?.order ?? settled;
  const byId = new Map(agents.map(a => [a.id, a]));
  const shown = order.map(id => byId.get(id)).filter((a): a is AgentInfo => !!a);
  const enabled = (a: AgentInfo) => pending.enabled?.[a.id] ?? !a.disabled;
  const enabledCount = shown.filter(enabled).length;
  const visible = filter ? shown.filter(filter) : shown;

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
    // The other tiles where they stand; the drop slot is how many of them come before the pointer in reading order
    const tiles = [...(grid.current?.querySelectorAll<HTMLElement>('[data-agent-tile]') ?? [])];
    press.current = { id, x: e.clientX, y: e.clientY, others: tiles.filter(el => el.dataset.agentTile !== id).map(el => ({ id: el.dataset.agentTile!, rect: el.getBoundingClientRect() })) };
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const p = press.current;
    if (!p || (!drag && Math.hypot(e.clientX - p.x, e.clientY - p.y) < DRAG_SLOP)) return;
    if (!drag) e.currentTarget.setPointerCapture(e.pointerId);
    // A tile comes before the pointer when its row is above the pointer, or it is on the pointer's row left of it
    const before = (r: DOMRect) => e.clientY >= r.bottom || (e.clientY >= r.top && e.clientX > r.left + r.width / 2);
    const slot = p.others.filter(o => before(o.rect)).length;
    const next = p.others.map(o => o.id);
    next.splice(slot, 0, p.id);
    if (next.join('\n') !== drag?.order.join('\n')) setDrag({ id: p.id, order: next });
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
    <div ref={grid} className="grid grid-cols-2 gap-gap @max-[600px]/settings-shell:grid-cols-1">
      {visible.map(a => {
        const on = enabled(a);
        const last = on && enabledCount === 1;
        return (
          <div
            key={a.id}
            data-agent-tile={a.id}
            onPointerDown={e => onPointerDown(e, a.id)}
            onPointerMove={onPointerMove}
            onPointerUp={e => onPointerEnd(e, true)}
            onPointerCancel={e => onPointerEnd(e, false)}
            onClickCapture={e => { if (swallowClick.current) { e.preventDefault(); e.stopPropagation(); } }}
            className={cn('min-w-0 touch-none', drag?.id === a.id && 'relative z-10 cursor-grabbing')}
          >
            <AgentTile agent={a} on={on} last={last} dragging={drag?.id === a.id} inventory={inventories[a.id]}
              onToggle={v => toggle(a.id, v)} onOpen={() => onOpen(a.id)} onKeyDown={e => onKeyDown(e, a.id)} />
          </div>
        );
      })}
    </div>
  );
}

// One agent: its mark in a square, name and what it is, the switch; the foot names where the module comes from, the
// gear and the version (or that it is not installed). A click anywhere on the tile but its switch opens the dialog too
function AgentTile({ agent: a, on, last, dragging, inventory, onToggle, onOpen, onKeyDown }: {
  agent: AgentInfo;
  on: boolean;
  last: boolean;
  dragging: boolean;
  inventory?: AgentInventory;
  onToggle: (on: boolean) => void;
  onOpen: () => void;
  onKeyDown: (e: KeyboardEvent<HTMLElement>) => void;
}) {
  const vendor = VENDOR[a.id];
  const desc = a.id === BUILTIN_AGENT_ID ? t('settings.plugins.builtinDesc') : vendor ? t('settings.plugins.officialCli', { vendor }) : t('settings.plugins.acpAgent');
  const version = a.available === false ? t('settings.plugins.notInstalled') : inventory?.runtime?.version ?? inventory?.adapter?.adapter?.version;
  const settings = t('settings.plugins.settings', { name: a.name });
  return (
    <Card onClick={e => { if (!(e.target as Element).closest('[data-no-drag]')) onOpen(); }}
      className={cn('flex h-full cursor-pointer flex-col shadow-none transition-[background-color,box-shadow] hover:bg-hover', dragging && 'bg-active shadow-pop')}>
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
        <span data-no-drag className="-mr-ctl-sm-bleed flex shrink-0">
          <IconButton size="sm" title={settings} aria-label={settings} aria-haspopup="dialog" aria-keyshortcuts="Alt+ArrowLeft Alt+ArrowRight"
            onClick={onOpen} onKeyDown={onKeyDown} className="text-fg-2">
            <Settings2 strokeWidth={1.75} />
          </IconButton>
        </span>
      </div>
    </Card>
  );
}

// One built-in interface module tile: same silhouette as an agent tile, switched in place without a restart
function BuiltinTile({ id, icon, name, desc, on, onToggle }: {
  id: string;
  icon: ReactNode;
  name: string;
  desc: string;
  on: boolean;
  onToggle: (on: boolean) => void;
}) {
  return (
    <Card onClick={e => { if (!(e.target as Element).closest('[data-no-drag]')) onToggle(!on); }}
      data-plugin-tile={id}
      className="flex h-full cursor-pointer flex-col shadow-none transition-[background-color,box-shadow] hover:bg-hover">
      <div className="flex items-center gap-gap p-pad">
        <span className={cn('flex size-(--tile-mark) shrink-0 items-center justify-center rounded-lg bg-active text-fg-1', !on && 'text-fg-3')}>
          {icon}
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-y-(--stack-gap)">
          <span className={cn('truncate text-2 font-medium', on ? 'text-fg-1' : 'text-fg-2')}>{name}</span>
          <span className="truncate text-3 text-fg-3">{desc}</span>
        </span>
        <span data-no-drag className="flex shrink-0 items-center">
          <Switch checked={on} onCheckedChange={onToggle}
            aria-label={t('settings.agent.enabled', { agent: name })}
            title={t('settings.agent.enabled', { agent: name })} />
        </span>
      </div>
      <div className="mt-auto flex h-(--tile-foot) items-center justify-between gap-gap border-t border-line px-pad">
        <span className="flex min-w-0 items-baseline gap-2 text-3 text-fg-3">
          <span className="shrink-0">{t('settings.plugins.bundled')}</span>
        </span>
      </div>
    </Card>
  );
}
