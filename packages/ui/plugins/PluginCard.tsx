import { memo, useContext, useEffect, useRef, type CSSProperties, type MouseEvent } from 'react';
import { Bot, Blocks, Clock, ExternalLink, LayoutPanelLeft, Palette, SquareTerminal, type LucideIcon } from 'lucide-react';
import type { PluginsView } from '@acpira/shared/plugins';
import type { PluginCard as Card_ } from '@acpira/shared/transcript';
import { Button, IconButton } from '../ui/Button';
import { Card } from '../ui/Card';
import { StableText } from '../ui/StableText';
import { cn } from '../ui/cn';
import { t, useLocale } from '../i18n';
import { vscodeApi } from '../vscodeApi';
import { AgentMark } from '../chat/AgentMark';
import { TurnActionsContext } from '../chat/TurnActions';
import { SandboxFrame } from './SandboxFrame';
import { expectArrival } from './arrivals';
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
  if (info?.enabled && info.snapshot !== card.snapshot) return { state: 'newer' };
  if (card.undo && skin && view?.skin && view.skin !== card.plugin) {
    return { state: 'replaced', replacedBy: view.plugins.find(p => p.id === view.skin)?.name ?? view.skin };
  }
  return { state: 'pending' };
}

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
    const r = e.currentTarget.getBoundingClientRect();
    if (card.kind === 'skin') expectReveal({ x: r.left + r.width / 2, y: r.top + r.height / 2 }, undo, card.plugin);
    // Any card with pieces, one that also runs a program (exec) included
    if (!undo && card.slots?.length) {
      for (const el of thumb.current?.querySelectorAll<HTMLElement>('[data-piece]') ?? []) expectArrival(card.plugin, el.dataset.piece!, el.getBoundingClientRect(), sessionId);
    }
    if (!sessionId) return;
    vscodeApi().postMessage({ type: 'pluginCard', sessionId, card: card.id, op: undo ? 'undo' : 'apply' });
  };
  return (
    <Card data-plugin-card={card.kind} data-state={state} className="group/plugin relative my-gap flex min-w-0 flex-col overflow-hidden">
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
          <Thumb card={card} />
        </div>
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="truncate text-2 font-medium text-fg-strong">{card.name}</span>
          {card.description && <span className="truncate text-2 text-fg-2">{card.description}</span>}
        </div>
      </div>
      {/* The bar: the kind (and for a program, what it runs) at the start, the button at the end */}
      <div className="flex min-w-0 items-center gap-gap border-t border-line py-gap-half pr-gap-half pl-pad">
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
// its glyphs become texture; 2D, since a page holds few WebGL contexts and a transcript can hold many cards. An agent's
// is its mark on the brand tint, an interface plugin's a map of where its pieces go, anything else its kind
function Thumb({ card }: { card: Card_ }) {
  if (card.kind === 'skin') return <SkinThumb plugin={card.plugin} snapshot={card.snapshot} name={card.name} />;
  // Interface pieces (a command's button among them) show where they go; a program's foot still names what it runs
  if (card.kind === 'ui' || card.slots?.length) return <UiThumb slots={card.slots ?? []} />;
  const Kind = KIND[card.kind] ?? Blocks;
  return (
    <div className="absolute inset-0 flex items-center justify-center bg-[color-mix(in_srgb,var(--brand)_14%,var(--bg-1))] text-brand">
      {card.kind === 'agent' ? <AgentMark id={card.plugin} name={card.name} className="size-ctl" /> : <Kind className="size-lead" strokeWidth={1.5} />}
    </div>
  );
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
    return <div className="absolute inset-0 flex items-center justify-center bg-[color-mix(in_srgb,var(--brand)_14%,var(--bg-1))] text-brand"><Palette className="size-lead" strokeWidth={1.5} /></div>;
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
const lit: CSSProperties = { position: 'absolute', borderRadius: 'var(--r-xs, 2px)', background: 'color-mix(in srgb, var(--brand) 30%, transparent)', boxShadow: '0 0 0 1px var(--brand), 0 0 8px -1px var(--brand)' };
const line = (left: string, top: string, width: string): CSSProperties => ({ position: 'absolute', left, top, width, height: '3%', borderRadius: 1, background: 'color-mix(in srgb, var(--fg-3) 45%, transparent)' });

function UiThumb({ slots }: { slots: string[] }) {
  const blocks = [...new Set(slots)].flatMap(s => (BLOCKS[s] ? [{ slot: s, ...BLOCKS[s] }] : []));
  const at = (where: 'column' | 'sheet' | 'composer') => blocks.filter(b => b.in === where).map(b => <div key={b.slot} data-piece={b.slot} style={{ ...lit, ...b.style }} />);
  return (
    <div className="absolute inset-0 bg-[color-mix(in_srgb,var(--brand)_6%,var(--bg-0))]">
      <div className="absolute inset-2 overflow-hidden rounded-sm border border-line-strong bg-bg-1">
        <div className="absolute inset-y-0 left-0 w-[27%] border-r border-line bg-bg-0">
          {[16, 26, 36, 46].map(top => <div key={top} style={line('14%', `${top}%`, top === 16 ? '60%' : '72%')} />)}
          {at('column')}
        </div>
        <div style={line('34%', '10%', '30%')} />
        {at('sheet')}
        {[28, 36, 44].map((top, i) => <div key={top} style={line('34%', `${top}%`, i === 2 ? '34%' : '56%')} />)}
        <div className="absolute rounded-xs border border-line-strong" style={{ left: '33%', right: '9%', bottom: '7%', height: '22%' }}>{at('composer')}</div>
      </div>
    </div>
  );
}
