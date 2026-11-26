import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Blocks, Bot, Clock, FolderOpen, Layers, LayoutPanelLeft, MousePointerClick, Paintbrush, Palette, PanelTop, Trash2, Undo2, Workflow, Wrench, type LucideIcon } from 'lucide-react';
import type { PluginAction, PluginContribution, PluginInfo, PluginOverlap, PluginOverlapKind, PluginsView } from '@acpira/shared/plugins';
import { Card } from '../ui/Card';
import { Button, IconButton } from '../ui/Button';
import { Switch } from '../ui/Switch';
import { Tooltip } from '../ui/Tooltip';
import { cn } from '../ui/cn';
import { dateFormat, getLocale, t } from '../i18n';
import { vscodeApi } from '../vscodeApi';
import { usePluginPieces } from '../pluginSlots';
import { DefaultLookThumb, SkinThumb } from '../plugins/PluginCard';
import { SandboxFrame } from '../plugins/SandboxFrame';
import { backdropsShown, expectReveal, useSkin } from '../plugins/skin';
import { DialogBody, Note, SettingsDialog } from '../chrome/controls';

// Settings → Plugins, the user's plugins (docs/dev/plugins.md): the skin picker heading the Appearance filter, and a tile
// per plugin. Read from the plugins view (plugins/skin.ts), so the page, every
// window and the in-chat cards agree; every action goes to the engine and comes back as a new view

const act = (action: PluginAction) => vscodeApi().postMessage({ type: 'pluginAction', action });
const centre = (el: Element) => { const r = el.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; };
const themed = (p: PluginInfo) => p.contributes.includes('theme');

// What a plugin adds, as the mark on its tile: the first kind it contributes that has a glyph
const MARK: [kind: PluginContribution, icon: LucideIcon][] = [['theme', Palette], ['style', Paintbrush], ['ui', LayoutPanelLeft], ['agent', Bot], ['tools', Wrench], ['mcp', Wrench], ['hooks', Workflow], ['schedules', Clock], ['runtime', Workflow]];
const markOf = (p: PluginInfo) => MARK.find(([k]) => p.contributes.includes(k))?.[1] ?? Blocks;

// The skins: one choice among the themes in effect, or none. Picking one uncovers it from the tile (plugins/skin.ts
// expectReveal); a theme that is off is switched on from its tile below first. The last pick wins: until the engine's
// view says otherwise the picker shows (and compares against) what was asked last, so A, B, A quickly ends on A. Arrow
// keys move the choice like any radio group; only the chosen tile is in the Tab order
export function SkinPicker({ view, filter }: { view: PluginsView; filter?: (p: PluginInfo) => boolean }) {
  const { shown } = useSkin();
  const skins = view.plugins.filter(p => themed(p) && p.loaded && p.snapshot && (!filter || filter(p)));
  // '' is none; undefined: nothing asked that the view does not show yet
  const [asked, setAsked] = useState<string>();
  const current = view.skin ?? '';
  useEffect(() => setAsked(undefined), [view.skin, view.generation]);
  const chosen = asked ?? current;
  const group = useRef<HTMLDivElement>(null);
  const pick = (el: Element, id: string) => {
    if (id === chosen) return;
    // Choosing none closes the circle on the skin going; another skin opens from the tile
    expectReveal(centre(el), !id, id || chosen || shown?.plugin || '');
    setAsked(id);
    act(id ? { op: 'skin', id } : { op: 'skin' });
  };
  const ids = ['', ...skins.map(p => p.id)];
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const next = (Math.max(0, ids.indexOf(chosen)) + step + ids.length) % ids.length;
    const tile = group.current?.querySelectorAll<HTMLElement>('[data-skin-tile]')[next];
    if (!tile) return;
    tile.focus();
    pick(tile, ids[next]!);
  };
  return (
    <div ref={group} role="radiogroup" aria-label={t('settings.plugins.skins')} onKeyDown={onKey} className="grid grid-cols-3 gap-gap settings-narrow:grid-cols-2">
      <SkinTile name={t('settings.plugins.noSkin')} sub={t('settings.plugins.noSkinDesc')} on={!chosen} onPick={el => pick(el, '')}>
        <DefaultLookThumb />
      </SkinTile>
      {skins.map(p => (
        <SkinTile key={p.id} name={p.name} sub={p.description} on={chosen === p.id} onPick={el => pick(el, p.id)}>
          <SkinThumb plugin={p.id} snapshot={p.snapshot!} name={p.name} />
        </SkinTile>
      ))}
    </div>
  );
}

function SkinTile({ name, sub, on, onPick, children }: { name: string; sub: string; on: boolean; onPick: (el: Element) => void; children: ReactNode }) {
  return (
    <button type="button" role="radio" aria-checked={on} tabIndex={on ? 0 : -1} onClick={e => onPick(e.currentTarget)} data-skin-tile
      className={cn('group flex min-w-0 flex-col overflow-hidden rounded-lg border bg-bg-1 text-left transition-[border-color,box-shadow]',
        on ? 'border-fg-2 ring-1 ring-fg-2' : 'border-line hover:border-line-strong')}>
      <span className="relative aspect-video w-full overflow-hidden border-b border-line">{children}</span>
      <span className="flex min-w-0 flex-col gap-y-(--stack-gap) p-pad">
        <span className={cn('truncate text-2 font-medium', on ? 'text-fg-strong' : 'text-fg-1')}>{name}</span>
        <span className="truncate text-3 text-fg-3">{sub}</span>
      </span>
    </button>
  );
}

// A schedule's line in the tile's tooltip: its id, and when it runs next (or why it does not)
const when = (ms: number) => dateFormat(getLocale(), { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(ms);
const scheduleLine = (s: NonNullable<PluginInfo['schedules']>[number]) =>
  `${s.id}: ${s.paused ? t('settings.plugins.schedulePaused') : s.running ? t('settings.plugins.scheduleRunning') : s.next ? t('settings.plugins.scheduleNext', { when: when(s.next) }) : s.cron}`;

const OVERLAP_KIND_KEY: Record<PluginOverlapKind, `settings.plugins.overlap.${PluginOverlapKind}`> = {
  command: 'settings.plugins.overlap.command',
  provider: 'settings.plugins.overlap.provider',
  mcp: 'settings.plugins.overlap.mcp',
  hook: 'settings.plugins.overlap.hook',
  style: 'settings.plugins.overlap.style',
  prompt: 'settings.plugins.overlap.prompt',
  skill: 'settings.plugins.overlap.skill',
};

export interface ResolvedOverlapPeer {
  id: string;
  name: string;
  summary: string;
}

// Format one overlap kind concisely (keep raw CSS hooks out of the label; cap item previews at two names)
function formatOverlap(o: PluginOverlap): string {
  const label = t(OVERLAP_KIND_KEY[o.kind]);
  if (o.kind === 'style' || !o.items?.length) return label;
  const shown = o.items.slice(0, 2).join(', ');
  const extra = o.items.length > 2 ? ` +${o.items.length - 2}` : '';
  return `${label} ${shown}${extra}`;
}

// Group a loaded plugin's overlaps by peer and split them into peers it loads before vs. after in `loadOrder`
export function resolvePluginOverlaps(
  plugin: PluginInfo,
  loadOrder: readonly string[],
  pluginNames: ReadonlyMap<string, string>,
): { before: ResolvedOverlapPeer[]; after: ResolvedOverlapPeer[] } {
  if (!plugin.loaded || !plugin.overlaps?.length) return { before: [], after: [] };
  const grouped = new Map<string, string[]>();
  for (const o of plugin.overlaps) {
    const list = grouped.get(o.plugin) ?? [];
    list.push(formatOverlap(o));
    grouped.set(o.plugin, list);
  }
  const rank = (id: string) => {
    const i = loadOrder.indexOf(id);
    return i >= 0 ? i : Number.MAX_SAFE_INTEGER;
  };
  const selfRank = rank(plugin.id);
  const before: ResolvedOverlapPeer[] = [];
  const after: ResolvedOverlapPeer[] = [];
  const sep = t('common.listSep');
  const peers = [...grouped.entries()].sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b));
  for (const [id, parts] of peers) {
    const peer: ResolvedOverlapPeer = { id, name: pluginNames.get(id) ?? id, summary: parts.join(sep) };
    const peerRank = rank(id);
    if (selfRank < peerRank || (selfRank === peerRank && plugin.id.localeCompare(id) < 0)) before.push(peer);
    else after.push(peer);
  }
  return { before, after };
}

function OverlapSection({ title, peers, warn = false }: { title: string; peers: ResolvedOverlapPeer[]; warn?: boolean }) {
  return (
    <div className="flex flex-col gap-1">
      <span className={cn('text-3 font-medium', warn ? 'text-warn' : 'text-fg-2')}>{title}</span>
      <ul className="m-0 flex list-none flex-col gap-1 p-0">
        {peers.map(peer => (
          <li key={peer.id} className="flex items-baseline justify-between gap-gap text-3">
            <span className="min-w-0 truncate text-fg-1">{peer.name}</span>
            <span className="max-w-1/2 shrink-0 truncate text-fg-3">{peer.summary}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

// Inline footer badge + hover list showing which overlapping plugins this tile loads before or after
function PluginOverlapBadge({ before, after, canDrag, dragging }: {
  before: ResolvedOverlapPeer[];
  after: ResolvedOverlapPeer[];
  canDrag: boolean;
  dragging: boolean;
}) {
  if (!before.length && !after.length) return null;
  const shadowed = after.length > 0;
  return (
    <Tooltip.Root disabled={dragging}>
      <Tooltip.Trigger
        type="button"
        data-no-drag
        data-plugin-overlap-trigger
        onClick={e => e.stopPropagation()}
        className={cn(
          'inline-flex h-ctl-sm min-w-0 shrink-0 items-center gap-1 rounded-sm px-1.5 text-3 transition-colors outline-none hover:bg-hover focus-visible:bg-hover',
          shadowed ? 'text-warn' : 'text-fg-2',
        )}
      >
        <Layers className="size-icon shrink-0" strokeWidth={1.75} aria-hidden />
        <span className="truncate">{t('settings.plugins.overlapTrigger')}</span>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Positioner side="top" align="start" width="sm">
          <Tooltip.Popup data-plugin-overlap-popup className="gap-2 p-pad">
            {before.length > 0 && <OverlapSection title={t('settings.plugins.loadsBefore')} peers={before} />}
            {after.length > 0 && <OverlapSection title={t('settings.plugins.loadsAfter')} peers={after} warn />}
            {canDrag && <span className="border-t border-line pt-1.5 text-3 text-fg-3">{t('settings.plugins.overlapHint')}</span>}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

// One user plugin: what it adds as its mark, name and description (or why it is not in effect), the switch; the foot
// holds its version and whether the folder has changes not enabled, then its page, the earlier version and its folder
export function UserPluginTile({ plugin: p, safeMode, dragging = false, canDrag = false, loadOrder = [], pluginNames }: {
  plugin: PluginInfo;
  safeMode: boolean;
  dragging?: boolean;
  canDrag?: boolean;
  loadOrder?: readonly string[];
  pluginNames?: ReadonlyMap<string, string>;
}) {
  const Mark = markOf(p);
  const [page, setPage] = useState(false);
  const [removing, setRemoving] = useState(false);
  const pages = usePluginPieces('page').filter(x => x.plugin === p.id);
  const toggle = (on: boolean) => act(on ? (p.proposal ? { op: 'confirm', id: p.id, snapshot: p.proposal.snapshot } : { op: 'enable', id: p.id }) : { op: 'disable', id: p.id });
  // Why it is not in effect, or what its last check found wrong
  const missing = p.dependencies?.find(d => !d.optional && !d.met);
  const stopped = p.schedules?.find(s => s.paused);
  const why = p.error ?? p.findings?.errors?.[0] ?? (p.enabled && !p.loaded && safeMode ? t('settings.plugins.safeMode') : undefined)
    ?? (missing && t('settings.plugins.missingDependency', { id: missing.id, range: missing.range }))
    ?? (stopped && t('settings.plugins.scheduleStopped', { id: stopped.id, error: stopped.error ?? '' }));
  const schedules = p.enabled && p.loaded ? p.schedules ?? [] : [];
  const { before, after } = resolvePluginOverlaps(p, loadOrder, pluginNames ?? new Map());
  const openPage = t('settings.plugins.openPage', { name: p.name });
  const rollback = t('settings.plugins.rollback');
  const reveal = t('settings.plugins.reveal');
  const uninstall = t('settings.plugins.uninstall');
  return (
    <Card data-plugin-tile={p.id}
      className={cn(
        'flex h-full flex-col shadow-none transition-[background-color,box-shadow,scale]',
        canDrag && !dragging && 'cursor-grab hover:bg-hover',
        dragging && 'scale-102 cursor-grabbing bg-bg-2 shadow-pop',
      )}>
      <div className="flex items-center gap-gap p-pad">
        <span className={cn('flex size-(--tile-mark) shrink-0 items-center justify-center rounded-lg bg-active text-fg-1', !p.enabled && 'text-fg-3')}>
          <Mark className="size-lead" strokeWidth={1.5} />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-y-(--stack-gap)">
          <span className={cn('truncate text-2 font-medium', p.enabled ? 'text-fg-1' : 'text-fg-2')}>{p.name}</span>
          <span className={cn('truncate text-3', why ? 'text-danger' : 'text-fg-3')} title={why ?? p.description}>{why ?? p.description}</span>
        </span>
        <span data-no-drag className="flex shrink-0 items-center">
          <Switch checked={p.enabled} onCheckedChange={toggle} aria-label={t('settings.agent.enabled', { agent: p.name })} title={t('settings.agent.enabled', { agent: p.name })} />
        </span>
      </div>
      <div className="mt-auto flex h-(--tile-foot) items-center justify-between gap-gap border-t border-line px-pad">
        <span className="flex min-w-0 items-center gap-2 text-3 text-fg-3">
          <span className="truncate font-mono tabular-nums">{p.version}</span>
          {p.changed && <span className="shrink-0">{t('settings.plugins.changed')}</span>}
          {schedules.length > 0 && (
            <span className="flex shrink-0 items-center self-center" title={schedules.map(scheduleLine).join('\n')} aria-label={schedules.map(scheduleLine).join(', ')}>
              <Clock className="size-icon" strokeWidth={1.75} />
            </span>
          )}
          <PluginOverlapBadge before={before} after={after} canDrag={canDrag} dragging={dragging} />
        </span>
        <span data-no-drag className="-mr-ctl-sm-bleed flex shrink-0 items-center">
          {backdropsShown && p.loaded && pages.length > 0 && (
            <IconButton size="sm" title={openPage} aria-label={openPage} aria-haspopup="dialog" onClick={() => setPage(true)} className="text-fg-2"><PanelTop strokeWidth={1.75} /></IconButton>
          )}
          {p.canRollback && <IconButton size="sm" title={rollback} aria-label={rollback} onClick={() => act({ op: 'rollback', id: p.id })} className="text-fg-2"><Undo2 strokeWidth={1.75} /></IconButton>}
          <IconButton size="sm" title={reveal} aria-label={reveal} onClick={() => act({ op: 'reveal', id: p.id })} className="text-fg-2"><FolderOpen strokeWidth={1.75} /></IconButton>
          <IconButton size="sm" title={uninstall} aria-label={uninstall} aria-haspopup="dialog" onClick={() => setRemoving(true)} className="text-fg-2"><Trash2 strokeWidth={1.75} /></IconButton>
        </span>
      </div>
      {removing && (
        // Uninstalling moves the folder aside and keeps the data; the storage page clears both
        <SettingsDialog icon={<Mark />} title={t('settings.plugins.uninstallTitle', { name: p.name })} onClose={() => setRemoving(false)}
          footer={<>
            {/* The confirm's closing action: the primary after Cancel, on the right */}
            <span className="flex-1" />
            <Button onClick={() => setRemoving(false)}>{t('common.cancel')}</Button>
            <Button variant="primary" onClick={() => { act({ op: 'remove', id: p.id }); setRemoving(false); }}>{uninstall}</Button>
          </>}>
          <DialogBody><Note>{t('settings.plugins.uninstallNote')}</Note></DialogBody>
        </SettingsDialog>
      )}
      {page && pages[0] && (
        // The plugin's own page (the `page` slot): its first one, filling a tall dialog
        <SettingsDialog wide tall icon={<Mark />} title={pages[0].title || p.name} sub={p.version} onClose={() => setPage(false)}>
          <DialogBody className="overflow-hidden p-0">
            <SandboxFrame plugin={p.id} piece={pages[0].id} title={pages[0].title || p.name} focusable className="size-full" />
          </DialogBody>
        </SettingsDialog>
      )}
    </Card>
  );
}

// Computer use: the built-in tools that read and operate other apps. The one tool group with a tile, since it stays
// off until the user turns it on here; the engine lists the group only where it can drive apps
export function ComputerUseTile({ on, dragging = false, canDrag = false }: { on: boolean; dragging?: boolean; canDrag?: boolean }) {
  const name = t('settings.plugins.computer');
  const desc = t('settings.plugins.computerDesc');
  const label = t('settings.agent.enabled', { agent: name });
  return (
    <Card data-plugin-tile="computer"
      className={cn(
        'flex h-full flex-col shadow-none transition-[background-color,box-shadow,scale]',
        canDrag && !dragging && 'cursor-grab hover:bg-hover',
        dragging && 'scale-102 cursor-grabbing bg-bg-2 shadow-pop',
      )}>
      <div className="flex items-center gap-gap p-pad">
        <span className={cn('flex size-(--tile-mark) shrink-0 items-center justify-center rounded-lg bg-active text-fg-1', !on && 'text-fg-3')}>
          <MousePointerClick className="size-lead" strokeWidth={1.5} />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-y-(--stack-gap)">
          <span className={cn('truncate text-2 font-medium', on ? 'text-fg-1' : 'text-fg-2')}>{name}</span>
          <span className="truncate text-3 text-fg-3" title={desc}>{desc}</span>
        </span>
        <span data-no-drag className="flex shrink-0 items-center">
          <Switch checked={on} onCheckedChange={next => act({ op: 'toolGroup', id: 'computer', on: next })} aria-label={label} title={label} />
        </span>
      </div>
      <div className="mt-auto flex h-(--tile-foot) items-center border-t border-line px-pad">
        <span className="truncate text-3 text-fg-3">{t('settings.plugins.bundled')}</span>
      </div>
    </Card>
  );
}
