import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Blocks, Bot, FolderOpen, LayoutPanelLeft, Palette, PanelTop, Undo2, Workflow, Wrench, type LucideIcon } from 'lucide-react';
import type { PluginAction, PluginContribution, PluginInfo, PluginsView } from '@acpira/shared/plugins';
import { Card } from '../ui/Card';
import { IconButton } from '../ui/Button';
import { Switch } from '../ui/Switch';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { vscodeApi } from '../vscodeApi';
import { usePluginPieces } from '../pluginSlots';
import { SkinThumb } from '../plugins/PluginCard';
import { SandboxFrame } from '../plugins/SandboxFrame';
import { backdropsShown, expectReveal, useSkin } from '../plugins/skin';
import { DialogBody, SettingsDialog } from './controls';

// Settings → Plugins, the user's plugins (docs/dev/plugins.md): the skin picker heading the page, and a tile per plugin in
// the layer it belongs to, beside the built-in modules. Read from the plugins view (plugins/skin.ts), so the page, every
// window and the in-chat cards agree; every action goes to the engine and comes back as a new view

const act = (action: PluginAction) => vscodeApi().postMessage({ type: 'pluginAction', action });
const centre = (el: Element) => { const r = el.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; };
const themed = (p: PluginInfo) => p.contributes.includes('theme');

// What a plugin adds, as the mark on its tile: the first kind it contributes that has a glyph
const MARK: [kind: PluginContribution, icon: LucideIcon][] = [['theme', Palette], ['ui', LayoutPanelLeft], ['agent', Bot], ['tools', Wrench], ['mcp', Wrench], ['hooks', Workflow], ['runtime', Workflow]];
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
    <div ref={group} role="radiogroup" aria-label={t('settings.plugins.appearance')} onKeyDown={onKey} className="grid grid-cols-3 gap-gap @max-[600px]/settings-shell:grid-cols-2">
      <SkinTile name={t('settings.plugins.noSkin')} sub={t('settings.plugins.noSkinDesc')} on={!chosen} onPick={el => pick(el, '')}>
        <div className="absolute inset-0 bg-bg-0" />
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

// One user plugin: what it adds as its mark, name and description (or why it is not in effect), the switch; the foot
// holds its version and whether the folder has changes not enabled, then its page, the earlier version and its folder
export function UserPluginTile({ plugin: p, safeMode }: { plugin: PluginInfo; safeMode: boolean }) {
  const Mark = markOf(p);
  const [page, setPage] = useState(false);
  const pages = usePluginPieces('page').filter(x => x.plugin === p.id);
  const toggle = (on: boolean) => act(on ? (p.proposal ? { op: 'confirm', id: p.id, snapshot: p.proposal.snapshot } : { op: 'enable', id: p.id }) : { op: 'disable', id: p.id });
  // Why it is not in effect, or what its last check found wrong
  const why = p.error ?? p.findings?.errors?.[0] ?? (p.enabled && !p.loaded && safeMode ? t('settings.plugins.safeMode') : undefined);
  const openPage = t('settings.plugins.openPage', { name: p.name });
  const rollback = t('settings.plugins.rollback');
  const reveal = t('settings.plugins.reveal');
  return (
    <Card data-plugin-tile={p.id} className="flex h-full flex-col shadow-none">
      <div className="flex items-center gap-gap p-pad">
        <span className={cn('flex size-(--tile-mark) shrink-0 items-center justify-center rounded-lg bg-active text-fg-1', !p.enabled && 'text-fg-3')}>
          <Mark className="size-lead" strokeWidth={1.5} />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-y-(--stack-gap)">
          <span className={cn('truncate text-2 font-medium', p.enabled ? 'text-fg-1' : 'text-fg-2')}>{p.name}</span>
          <span className={cn('truncate text-3', why ? 'text-danger' : 'text-fg-3')} title={why ?? p.description}>{why ?? p.description}</span>
        </span>
        <span className="flex shrink-0 items-center">
          <Switch checked={p.enabled} onCheckedChange={toggle} aria-label={t('settings.agent.enabled', { agent: p.name })} title={t('settings.agent.enabled', { agent: p.name })} />
        </span>
      </div>
      <div className="mt-auto flex h-(--tile-foot) items-center justify-between gap-gap border-t border-line px-pad">
        <span className="flex min-w-0 items-baseline gap-2 text-3 text-fg-3">
          <span className="truncate font-mono tabular-nums">{p.version}</span>
          {p.changed && <span className="shrink-0">{t('settings.plugins.changed')}</span>}
        </span>
        <span className="-mr-ctl-sm-bleed flex shrink-0 items-center">
          {backdropsShown && p.loaded && pages.length > 0 && (
            <IconButton size="sm" title={openPage} aria-label={openPage} aria-haspopup="dialog" onClick={() => setPage(true)} className="text-fg-2"><PanelTop strokeWidth={1.75} /></IconButton>
          )}
          {p.canRollback && <IconButton size="sm" title={rollback} aria-label={rollback} onClick={() => act({ op: 'rollback', id: p.id })} className="text-fg-2"><Undo2 strokeWidth={1.75} /></IconButton>}
          <IconButton size="sm" title={reveal} aria-label={reveal} onClick={() => act({ op: 'reveal', id: p.id })} className="text-fg-2"><FolderOpen strokeWidth={1.75} /></IconButton>
        </span>
      </div>
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
