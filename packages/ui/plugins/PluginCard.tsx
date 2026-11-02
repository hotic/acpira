import { memo, useContext, useEffect, useRef, type CSSProperties, type MouseEvent, type ReactNode } from 'react';
import { Bot, Blocks, Clock, ExternalLink, LayoutPanelLeft, Palette, SquareTerminal, type LucideIcon } from 'lucide-react';
import type { PluginsView } from '@acpira/shared/plugins';
import type { PluginCard as Card_ } from '@acpira/shared/transcript';
import { Button, IconButton } from '../ui/Button';
import { Card } from '../ui/Card';
import { StableText } from '../ui/StableText';
import { cn } from '../ui/cn';
import { t, useLocale } from '../i18n';
import { vscodeApi } from '../vscodeApi';
import { TurnActionsContext } from '../chat/TurnActions';
import { SandboxFrame } from './SandboxFrame';
import { expectArrival } from './arrivals';
import { captureReturns, flyBack, flyHome, restylePart, restylePiece, type Return } from './landings';
import { chargeCard } from './charge';
import { pressIn } from '../effects/magic';
import { useInView, useReducedMotion, useWindowFocused } from './motion';
import { useAppearance } from '../appearance';
import { askPluginsView, backdropsShown, expectReveal, usePluginBackdrop, usePluginsView } from './skin';

// The in-chat plugin card (docs/dev/plugins.md Activation): what plugin_activate leaves after the reply instead of a
// question in the dock, clickable for good. Applying is the confirmation. Built as the plan card is
// (chat/PlanDocument.tsx), so the two read as one family: a Card as wide as the reply, the files glyph in the corner on
// hover, and an action bar under a hairline with small controls at its end. Never folds open.
// Its state is read from the plugins' state, so every window and the settings page agree: pending, applied, replaced
// (applied here, then another skin took over), newer (another version of the plugin is in use)

export type CardState = 'pending' | 'applied' | 'replaced' | 'newer';

export function cardState(card: Card_, view: PluginsView | undefined): { state: CardState; replacedBy?: string } {
  const info = view?.plugins.find(p => p.id === card.plugin);
  const enabledHere = !!info?.enabled && info.snapshot === card.snapshot;
  const skin = card.kind === 'skin';
  if (enabledHere && (!skin || view?.skin === card.plugin)) return { state: 'applied' };
  // Another version in effect is newer only when it was committed after this card was offered; a card offered later
  // is an update to the version in effect and is still to be applied
  if (info?.enabled && info.snapshot !== card.snapshot && !(info.committedAt !== undefined && card.at > info.committedAt)) return { state: 'newer' };
  if (card.undo && skin && view?.skin && view.skin !== card.plugin) {
    return { state: 'replaced', replacedBy: view.plugins.find(p => p.id === view.skin)?.name ?? view.skin };
  }
  return { state: 'pending' };
}

// How long after the click an apply still flies: the dynamic check of a program may take a while
const LAUNCH_MS = 30_000;
const KIND: Record<string, LucideIcon> = { skin: Palette, agent: Bot, ui: LayoutPanelLeft, exec: SquareTerminal };
const kindLabel = (kind: string) => t(kind === 'skin' || kind === 'ui' || kind === 'agent' || kind === 'exec' ? `plugin.card.kind.${kind}` : 'plugin.card.kind.other');

export const PluginCard = memo(function PluginCard({ card }: { card: Card_ }) {
  // memo keeps the card from rendering again; the locale still reaches it after a language change
  useLocale();
  // The session it sits in; absent where the shell does not wire turn actions (an external session)
  const sessionId = useContext(TurnActionsContext)?.sessionId;
  useEffect(() => askPluginsView(), []);
  const view = usePluginsView();
  const { state, replacedBy } = cardState(card, view);
  const thumb = useRef<HTMLDivElement>(null);
  const Kind = KIND[card.kind] ?? Blocks;
  // Nothing to picture (no skin backdrop, no map of interface pieces): the thumbnail is the kind's glyph drawn large
  // along its right side, cut by its edge, fading out to the left (skin.css); the mark lifts off from it (landings.ts)
  const glyph = card.kind !== 'skin' && !card.slots?.length && !restyledBlocks(card.restyles).length;
  const box = useRef<HTMLDivElement>(null);
  // An apply clicked here charges the card (charge.ts) and, once the card reads applied (the check passed), sends lit
  // frames from the thumbnail to where the plugin now lives (landings.ts). An undo clicked here sends them back once
  // the card reads undone, from where they were at the click. Another window, or an apply that failed, flies nothing
  const launched = useRef<number>(undefined);
  const charged = useRef<() => void>(undefined);
  const returning = useRef<{ at: number; returns: Return[] }>(undefined);
  useEffect(() => {
    if (state === 'applied' && launched.current !== undefined) {
      const fresh = performance.now() - launched.current < LAUNCH_MS;
      launched.current = undefined;
      if (fresh && thumb.current) flyHome(card, thumb.current);
      charged.current?.();
      charged.current = undefined;
    }
    if (state !== 'applied' && returning.current) {
      const { at, returns } = returning.current;
      returning.current = undefined;
      if (performance.now() - at < LAUNCH_MS && thumb.current) flyBack(thumb.current, returns);
    }
  }, [state, card]);
  useEffect(() => () => charged.current?.(), []);
  // One button for the card's whole life: Apply becomes Undo in place (Switch back / Use this version once superseded),
  // its box reserving the longest of those words, so the click lands where it is undone. No status beside it: the
  // change itself says it is applied
  const words = [
    { key: 'pending', text: t(card.kind === 'exec' ? 'plugin.card.enable' : 'plugin.card.apply') },
    { key: 'applied', text: t('plugin.card.undo') },
    { key: 'replaced', text: t('plugin.card.restore') },
    { key: 'newer', text: t('plugin.card.keep') },
  ];
  const note = view?.safeMode ? t('plugin.card.safeMode') : state === 'replaced' ? t('plugin.card.replaced', { name: replacedBy ?? '' }) : state === 'newer' ? t('plugin.card.newer') : undefined;
  const act = (e: MouseEvent<HTMLButtonElement>) => {
    const undo = state === 'applied';
    const button = e.currentTarget;
    // A skin is uncovered from its thumbnail, the live picture of it, and covered back up into it
    const r = (thumb.current ?? button).getBoundingClientRect();
    if (card.kind === 'skin') expectReveal({ x: r.left + r.width / 2, y: r.top + r.height / 2 }, undo, card.plugin);
    if (!undo) {
      launched.current = performance.now();
      charged.current?.();
      charged.current = box.current && thumb.current ? chargeCard(box.current, button, thumb.current, { x: e.clientX, y: e.clientY }) : undefined;
    } else {
      pressIn(button, 0.06);
      if (thumb.current && card.kind !== 'skin') returning.current = { at: performance.now(), returns: captureReturns(card, thumb.current) };
    }
    // Any card with pieces, one that also runs a program (exec) included. A reply action is a native button, which
    // the mark flies to instead (landings.ts)
    if (!undo && card.slots?.length) {
      for (const el of thumb.current?.querySelectorAll<HTMLElement>('[data-piece]') ?? []) {
        if (el.dataset.piece !== 'message.actions') expectArrival(card.plugin, el.dataset.piece!, el.getBoundingClientRect(), sessionId);
      }
    }
    if (!sessionId) return;
    vscodeApi().postMessage({ type: 'pluginCard', sessionId, card: card.id, op: undo ? 'undo' : 'apply' });
  };
  return (
    <Card ref={box} data-plugin-card={card.kind} data-state={state} className="group/plugin relative my-gap flex min-w-0 flex-col overflow-hidden">
      {/* The plugin's folder: the plan card's corner glyph, placed and revealed the same way */}
      <div className="absolute top-gap-half right-gap-half z-3 opacity-0 transition-opacity duration-(--code-copy-duration) ease-out group-hover/plugin:opacity-100 group-focus-within/plugin:opacity-100 motion-reduce:transition-none [@media(hover:none)]:opacity-100">
        <IconButton size="sm" className="bg-bg-2 ring ring-conversation-line" title={`${t('plugin.card.files')} ${card.folder}`} aria-label={t('plugin.card.files')}
          onClick={() => vscodeApi().postMessage({ type: 'pluginAction', action: { op: 'reveal', id: card.plugin } })}>
          <ExternalLink strokeWidth={1.5} />
        </IconButton>
      </div>
      {/* Body: the thumbnail, the name and one line beside it, clear of the corner */}
      <div className="flex min-w-0 items-center gap-pad p-pad pr-ctl">
        <div ref={thumb} data-plugin-thumb aria-hidden="true" className={cn('relative shrink-0 overflow-hidden rounded-md bg-bg-0 shadow-[inset_0_0_0_1px_var(--line)] transition-opacity duration-300', state === 'replaced' && 'opacity-50')}>
          {glyph ? <div data-plugin-glyph><Kind strokeWidth={1.75} /></div> : <Thumb card={card} />}
        </div>
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="truncate text-2 font-medium text-fg-strong">{card.name}</span>
          {card.description && <span className="truncate text-2 text-fg-2">{card.description}</span>}
        </div>
      </div>
      {/* The bar: the kind (and for a program, what it runs) at the start, the button at the end */}
      <div data-plugin-bar className="flex min-w-0 items-center gap-gap border-t border-line py-gap-half pr-gap-half pl-pad">
        <span className="flex min-w-0 flex-1 items-center gap-1.5 text-3 text-fg-3">
          <Kind className="size-icon shrink-0" strokeWidth={1.5} />
          <span className="shrink-0">{kindLabel(card.kind)}</span>
          {card.runs && <span className="ml-1.5 min-w-0 truncate">{t('plugin.card.runs')} <code className="rounded-xs bg-active px-1 font-mono text-mono text-fg-2">{card.runs}</code></span>}
          {/* A plugin with schedules acts while nobody watches: said on the card it is enabled from */}
          {!!card.schedules?.length && (
            <span className="ml-1.5 flex shrink-0 items-center gap-1" title={t('plugin.card.scheduledHint', { cron: card.schedules.join(', ') })}>
              <Clock className="size-icon" strokeWidth={1.5} />{t('plugin.card.scheduled')}
            </span>
          )}
        </span>
        {note && <span className="shrink-0 px-1.5 text-3 text-fg-3">{note}</span>}
        <Button variant="primary" className="h-ctl-sm shrink-0 justify-center" disabled={!view || view.safeMode || !sessionId} onClick={act}>
          <StableText items={words} current={state} itemClassName="text-center" />
        </Button>
      </div>
    </Card>
  );
});

// A skin's thumbnail is its backdrop, live: drawn at twice the box and scaled down, so the whole composition shows and
// its glyphs become texture; 2D, since a page holds few WebGL contexts and a transcript can hold many cards. An interface
// plugin's is a map of where its pieces go; other cards draw their kind's glyph instead (PluginCard)
function Thumb({ card }: { card: Card_ }) {
  if (card.kind === 'skin') return <SkinThumb plugin={card.plugin} snapshot={card.snapshot} name={card.name} />;
  // Otherwise interface pieces (a command's button among them) show where they go
  return <UiThumb slots={card.slots ?? []} restyles={card.restyles ?? []} />;
}

// Also the settings page's skin picker
export function SkinThumb({ plugin, snapshot, name }: { plugin: string; snapshot: string; name: string }) {
  const html = usePluginBackdrop(plugin, snapshot);
  // Safe mode runs no plugin document, a thumbnail's included
  const safe = usePluginsView()?.safeMode;
  // Animates only on screen, in a focused window, with motion on: a long transcript can hold many of these
  const box = useRef<HTMLDivElement>(null);
  const seen = useInView(box);
  const focused = useWindowFocused();
  const { motion } = useAppearance();
  const reduced = useReducedMotion();
  const still = motion === 'none' || reduced;
  if (!backdropsShown || !html || safe) {
    // No live picture (another host, a skin of colours only, safe mode): drawn like a program's card, its glyph large
    return <div data-plugin-glyph><Palette strokeWidth={1.75} /></div>;
  }
  return (
    <div ref={box} className="absolute inset-0">
      <SandboxFrame plugin={plugin} piece={`thumb:${snapshot}`} html={html} title={name} options={{ font: 8, fps: 20, gl: false, run: !still }}
        run={!still && seen && focused} className="pointer-events-none absolute top-0 left-0 h-[200%] w-[200%] origin-top-left scale-50" />
    </div>
  );
}

// Where each slot sits on a sketch of the window: the session column, the sheet with its header and lines, the composer.
// Each piece a lit block where it lands; applying flies that block to the same place in the real window
const BLOCKS: Record<string, { in: 'column' | 'sheet' | 'composer'; style: CSSProperties }> = {
  'composer.above': { in: 'sheet', style: { right: '12%', bottom: '29%', width: '9%', height: '14%' } },
  'composer.toolbar': { in: 'composer', style: { left: '8%', bottom: '18%', width: '22%', height: '28%' } },
  sidebar: { in: 'column', style: { left: '12%', right: '12%', bottom: '7%', height: '24%' } },
  'session.panel': { in: 'sheet', style: { right: '2.5%', top: '18%', width: '3.5%', bottom: '32%' } },
  'panel.tab': { in: 'sheet', style: { right: '4%', top: '6%', width: '7%', height: '10%' } },
  statusbar: { in: 'sheet', style: { left: '34%', right: '9%', bottom: '2%', height: '4%' } },
  start: { in: 'sheet', style: { left: '40%', right: '20%', top: '30%', height: '24%' } },
  'message.actions': { in: 'sheet', style: { left: '34%', top: '50%', width: '14%', height: '6%' } },
  page: { in: 'sheet', style: { left: '30%', right: '6%', top: '20%', bottom: '34%' } },
};
const lit: CSSProperties = { position: 'absolute', borderRadius: 'var(--r-xs, 2px)', background: 'color-mix(in srgb, var(--land-ink) 30%, transparent)', boxShadow: '0 0 0 1px var(--land-ink), 0 0 8px -1px var(--land-ink)' };
const line = (left: string, top: string, width: string): CSSProperties => ({ position: 'absolute', left, top, width, height: '3%', borderRadius: 1, background: 'color-mix(in srgb, var(--fg-3) 45%, transparent)' });

type Where = 'column' | 'sheet' | 'composer';

// The window in miniature, inside the box its caller frames: column, header and text lines, the composer; `at` puts
// pieces in each part, `send` draws the composer's send button
function WindowSketch({ at, send }: { at?: (where: Where) => ReactNode; send?: boolean }) {
  return (
    <>
      <div className="absolute inset-y-0 left-0 w-[27%] border-r border-line bg-bg-0">
        {[16, 26, 36, 46].map(top => <div key={top} style={line('14%', `${top}%`, top === 16 ? '60%' : '72%')} />)}
        {at?.('column')}
      </div>
      <div style={line('34%', '10%', '30%')} />
      {at?.('sheet')}
      {[28, 36, 44].map((top, i) => <div key={top} style={line('34%', `${top}%`, i === 2 ? '34%' : '56%')} />)}
      <div className="absolute rounded-xs border border-line-strong" style={{ left: '33%', right: '9%', bottom: '7%', height: '22%' }}>
        {at?.('composer')}
        {send && <div className="absolute rounded-xs bg-btn-1" style={{ right: '4%', bottom: '16%', width: '9%', aspectRatio: '1' }} />}
      </div>
    </>
  );
}

// Where each style hook's part sits on the sketch: a style sheet lights the parts it restyles, and its frame lifts off
// from there to the same part of the real window. The whole window, sheet or thread is no part to light: a sheet
// naming only those draws its kind's glyph
const PARTS: Record<string, { in: Where; style: CSSProperties }> = {
  '[data-part="header"]': { in: 'sheet', style: { left: '30%', right: '4%', top: '4%', height: '12%' } },
  '[data-part="user-message"]': { in: 'sheet', style: { left: '58%', right: '9%', top: '18%', height: '7%' } },
  '[data-part="agent-message"]': { in: 'sheet', style: { left: '32%', right: '9%', top: '26%', height: '22%' } },
  '[data-turn-actions]': { in: 'sheet', style: { left: '33%', top: '51%', width: '14%', height: '6%' } },
  '[data-plan-dock]': { in: 'sheet', style: { left: '36%', right: '12%', bottom: '31%', height: '7%' } },
  '[data-part="composer"]': { in: 'sheet', style: { left: '33%', right: '9%', bottom: '7%', height: '22%' } },
  '[data-exchange]': { in: 'sheet', style: { left: '31%', right: '7%', top: '16%', bottom: '41%' } },
  '[data-session-panel]': { in: 'sheet', style: { right: '2.5%', top: '18%', width: '3.5%', bottom: '32%' } },
  '[data-tool-panel-wrap]': { in: 'sheet', style: { right: '1.5%', top: '4%', width: '5%', bottom: '4%' } },
  '[data-part="tab"]': { in: 'sheet', style: { right: '1.5%', top: '4%', width: '5%', height: '6%' } },
  '[data-session]': { in: 'column', style: { left: '8%', right: '8%', top: '21%', height: '10%' } },
};
function restyledBlocks(restyles: readonly string[] | undefined) {
  return [...new Set((restyles ?? []).map(restylePart))].flatMap(h => (PARTS[h] ? [{ slot: restylePiece(h), ...PARTS[h] }] : []));
}

function UiThumb({ slots, restyles }: { slots: string[]; restyles: string[] }) {
  const blocks = [...[...new Set(slots)].flatMap(s => (BLOCKS[s] ? [{ slot: s, ...BLOCKS[s] }] : [])), ...restyledBlocks(restyles)];
  const at = (where: Where) => blocks.filter(b => b.in === where).map(b => <div key={b.slot} data-piece={b.slot} style={{ ...lit, ...b.style }} />);
  return (
    <div className="absolute inset-0 bg-bg-0">
      <div className="absolute inset-2 overflow-hidden rounded-sm border border-line-strong bg-bg-1"><WindowSketch at={at} /></div>
    </div>
  );
}

// The skin picker's "no skin" picture: the window in the default palette of the current scheme. skin.css pins those
// colours on `.skin-default-thumb`, so the skin in effect cannot repaint the picture of going without one
export function DefaultLookThumb() {
  return (
    <div className="skin-default-thumb absolute inset-0 bg-(--thumb-around)">
      <div className="absolute overflow-hidden rounded-sm border border-line-strong bg-bg-1" style={{ inset: '13% 11%' }}><WindowSketch send /></div>
    </div>
  );
}
