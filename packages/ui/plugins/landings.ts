import type { PluginCard } from '@acpira/shared/transcript';
import { flyInto, motionOff, pressIn } from '../effects/magic';
import { playWhoosh } from '../effects/whoosh';

// Where an applied plugin goes to live, for the parts that have no slot of their own (docs/dev/plugins.md Activation):
// interface pieces fly from the thumbnail's map (arrivals.ts) and a skin is uncovered (skin.ts); everything else sends
// the card's mark to the control it now stands behind, so every kind of plugin lands somewhere visible.
// - composer: tools, MCP servers, prompts, skills and commands are the agent's to use from the next message
// - settings: hooks, a runtime and schedules work in the background, behind the gear; a style sheet alone goes there
//   too, where the looks are chosen
// - agents: a new agent is picked from the new-session entry
// - models: a model source shows in the model chip
// - actions: a reply action is a native button, not a framed piece, so it lands here too
export type Landing = 'composer' | 'settings' | 'agents' | 'models' | 'actions';

const BY_ADD: Record<string, Landing> = {
  tools: 'composer', mcp: 'composer', prompts: 'composer', skills: 'composer', commands: 'composer',
  hooks: 'settings', runtime: 'settings', schedules: 'settings',
  agent: 'agents', providers: 'models',
};
// A landing whose control is absent here (a narrow header folds the gear into its menu, a session without a model
// chip) falls back along this chain; the composer is always there under a card
const FALLBACK: Record<Landing, Landing | undefined> = { settings: 'composer', agents: 'models', models: 'composer', composer: undefined, actions: undefined };
// Between the flights of one card: each mark is seen leaving on its own, still one gesture
const STAGGER_MS = 320;
// A mark rises off the card for this long before it flies, then takes this long to land: slower than a piece, since
// it crosses the window to a small control the eye has to follow
const LIFT_MS = 420;
const MARK_FLY_MS = 1500;
// A reply's action row shows on hover; it stays shown this long for a button landing in it
const ACTIONS_SHOWN_MS = 2400;
// Targets that come with the plugins' push (a reply action) are looked for this long
const FIND_MS = 1500;

export function landingsFor(card: Pick<PluginCard, 'kind' | 'adds' | 'slots' | 'schedules'>): Landing[] {
  // Records from before `adds` was sent: the card's kind is all there is
  const adds = card.adds ?? (card.kind === 'agent' ? ['agent'] : card.kind === 'exec' ? ['tools'] : []);
  const slots = card.slots ?? [];
  const to = new Set<Landing>();
  for (const a of adds) {
    const at = BY_ADD[a];
    // A command with a button flies in as that button (a composer.toolbar piece)
    if (at && !(a === 'commands' && slots.includes('composer.toolbar'))) to.add(at);
  }
  if (card.schedules?.length) to.add('settings');
  if (slots.includes('message.actions')) to.add('actions');
  // A style sheet alone goes behind the gear, where the looks are chosen; with anything else it rides along. Never a
  // flash over the whole window: that read as the window being wiped
  if (adds.includes('style') && !adds.includes('theme') && !to.size && !slots.length) to.add('settings');
  return [...to];
}

function shown(el: Element): boolean {
  const r = el.getBoundingClientRect();
  if (r.width < 1 || r.height < 1 || r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) return false;
  if (el.closest('[inert], [aria-hidden="true"]')) return false;
  const v = (el as Element & { checkVisibility?: (o: object) => boolean }).checkVisibility;
  return v ? v.call(el, { visibilityProperty: true, opacityProperty: true }) : true;
}

const centre = (r: DOMRect) => ({ x: r.left + r.width / 2, y: r.top + r.height / 2 });
const distance = (a: DOMRect, b: DOMRect) => { const p = centre(a), q = centre(b); return Math.hypot(p.x - q.x, p.y - q.y); };

// The shown candidate nearest the card: with two sessions side by side, the one in the card's own column
function nearest(selector: string, near: DOMRect): HTMLElement | undefined {
  let best: HTMLElement | undefined, d = Infinity;
  for (const el of document.querySelectorAll<HTMLElement>(selector)) {
    if (!shown(el)) continue;
    const dd = distance(el.getBoundingClientRect(), near);
    if (dd < d) { d = dd; best = el; }
  }
  return best;
}

// The size a mark lands at inside a field with no glyph of its own
const MARK_PX = 28;
// The lifting mark against the thumbnail's height
const LIFT = 0.62;
// How far a landing presses its control in: a lone icon, a row or chip, the composer
const PRESS_ICON = 0.08;
const PRESS_ROW = 0.025;
const PRESS_WIDE = 0.006;
// The mark and its impact are neutral, the foreground's own colour (white on dark, ink on light): the brand tint read
// as a yellow ball
const INK = 'var(--fg-strong)';

// A square on `el`'s centre: as tall as it, or `size` when given (a mark scales evenly, so its box must be square)
function square(el: Element, size?: number): DOMRect {
  const r = el.getBoundingClientRect();
  const s = size ?? Math.max(r.width, r.height);
  return new DOMRect(r.left + r.width / 2 - s / 2, r.top + r.height / 2 - s / 2, s, s);
}

interface Target {
  // The box the mark shrinks onto
  rect: () => DOMRect | undefined;
  // The box that lights up as it lands, when not that one
  glow?: () => DOMRect | undefined;
  land?: () => void;
}

// The newest shown button this plugin put under a reply, its row held open while the mark lands
function actionsTarget(plugin: string): Target | undefined {
  const all = [...document.querySelectorAll<HTMLElement>(`[data-plugin-action^="${CSS.escape(plugin)}/"]`)];
  for (let i = all.length - 1; i >= 0; i--) {
    const el = all[i]!;
    const row = el.closest<HTMLElement>('[data-turn-actions]');
    if (row) row.dataset.landing = '';
    if (shown(el)) {
      setTimeout(() => { if (row) delete row.dataset.landing; }, ACTIONS_SHOWN_MS);
      return { rect: () => square(el, el.getBoundingClientRect().height), land: () => pressIn(el, PRESS_ROW) };
    }
    if (row) delete row.dataset.landing;
  }
  return undefined;
}

function target(at: Landing, plugin: string, near: DOMRect): Target | undefined {
  switch (at) {
    case 'composer': {
      const field = nearest('[data-land="composer"]', near);
      if (!field) return undefined;
      // The mark sinks into the middle of the field at a control's size; the field's edge answers and the field gives a
      // little under the hit
      return { rect: () => square(field, MARK_PX), glow: () => field.getBoundingClientRect(), land: () => pressIn(field, PRESS_WIDE) };
    }
    case 'settings':
    case 'agents':
    case 'models': {
      const el = nearest(`[data-land="${at}"]`, near);
      if (!el) return undefined;
      // The mark lands on the control's own glyph and bursts there; the control takes the hit, a row or chip less than
      // a lone icon, and the gear also turns a little as something settles in behind it
      const r = el.getBoundingClientRect();
      const land = () => {
        pressIn(el, r.width > r.height * 1.4 ? PRESS_ROW : PRESS_ICON);
        if (at === 'settings') el.querySelector('svg')?.animate([{ transform: 'rotate(0)' }, { transform: 'rotate(90deg)' }], { duration: 900, easing: 'cubic-bezier(.4,0,.2,1)' });
      };
      return { rect: () => square(el.querySelector('svg') ?? el), land };
    }
    case 'actions':
      return actionsTarget(plugin);
  }
}

// The flying mark: the card's kind glyph on a neutral rounded square with a hairline edge and a soft shadow, never a
// ring. The box is laid out at its starting size (magic.ts flyInto), so the corners keep their proportion as it shrinks
function markFlyer(icon: SVGElement | null): HTMLElement {
  const flyer = document.createElement('div');
  Object.assign(flyer.style, {
    display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: '26%', color: INK,
    background: `color-mix(in srgb, ${INK} 10%, var(--bg-1))`,
    boxShadow: `inset 0 0 0 1px color-mix(in srgb, ${INK} 32%, transparent), 0 10px 24px -10px rgba(0, 0, 0, .65)`,
  });
  if (icon) {
    const glyph = icon.cloneNode(true) as SVGElement;
    Object.assign(glyph.style, { width: '50%', height: '50%' });
    flyer.append(glyph);
  }
  return flyer;
}

// Where the mark lifts off: the thumbnail's own mark tile when it has one (a program's or an agent's card), so the tile
// itself seems to leave; else a square on the thumbnail's middle (an interface plugin's map)
const liftOff = (thumb: HTMLElement) => {
  const tile = thumb.querySelector('[data-thumb-mark]');
  return tile ? tile.getBoundingClientRect() : square(thumb, Math.round(thumb.getBoundingClientRect().height * LIFT));
};

// Sends the card's mark from its thumbnail to every place the plugin now lives. Called once the card reads applied,
// so a failed check flies nothing
export function flyHome(card: PluginCard, thumb: HTMLElement, icon: SVGElement | null): void {
  const to = landingsFor(card);
  if (motionOff() || !to.length) return;
  const from = thumb.getBoundingClientRect();
  const shell = thumb.closest<HTMLElement>('.acp-shell') ?? undefined;
  // The whoosh goes with the first mark leaving, not the lift
  setTimeout(() => playWhoosh('piece'), LIFT_MS);
  // One flight per control: two landings that fall back onto the same one fly once
  const flown = new Set<Landing>();
  let delay = 0;
  for (const at of to) {
    const started = performance.now();
    const go = (wait: number) => setTimeout(() => {
      let land: Landing | undefined = at;
      let t: Target | undefined;
      while (land && !(t = target(land, card.plugin, from))) land = FALLBACK[land];
      // A reply action comes with the plugins' push, which may trail the card's state a little
      if (!t && at === 'actions' && performance.now() - started < FIND_MS) { go(100); return; }
      if (!t || !land || flown.has(land)) return;
      flown.add(land);
      void flyInto(liftOff(thumb), t.rect, { flyer: markFlyer(icon), glow: INK, fit: 'mark', merge: true, host: shell, glowBox: t.glow, land: t.land, lift: LIFT_MS, duration: MARK_FLY_MS });
    }, wait);
    go(delay);
    delay += STAGGER_MS;
  }
}
