import type { PluginCard } from '@acpira/shared/transcript';
import { motionOff, pressIn } from '../effects/magic';
import { flyStandIn } from './Pieces';
import { playWhoosh } from '../effects/whoosh';

// Where an applied plugin goes to live, for the parts that have no slot of their own (docs/dev/plugins.md Activation):
// interface pieces fly from the thumbnail's map (arrivals.ts) and a skin is uncovered (skin.ts); everything else sends
// the same lit frame from the card to the control it now stands behind, the frame growing into that control's outline,
// so every kind of plugin lands somewhere visible.
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
// Between the flights of one card: each frame is seen leaving on its own, still one gesture
const STAGGER_MS = 320;
// A frame rises off the card for this long before it flies, then takes this long to land: slower than a piece, since
// it crosses the window to a control the eye has to follow
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

// How far a landing presses its control in: a lone icon, a row or chip, the composer
const PRESS_ICON = 0.08;
const PRESS_ROW = 0.025;
const PRESS_WIDE = 0.006;
// The frames take the landing colour (--land-ink, skin.css, read in Pieces.tsx flyStandIn): the skin's own, else the
// foreground. Acpira's brand yellow read as a yellow ball

interface Target {
  // The box the frame lands on
  rect: () => DOMRect | undefined;
  // The control whose shape it takes: its corner radius, and the ring that answers the landing
  outline?: () => HTMLElement | undefined;
  land?: () => void;
}

// The newest shown button this plugin put under a reply, its row held open while the frame lands
function actionsTarget(plugin: string): Target | undefined {
  const all = [...document.querySelectorAll<HTMLElement>(`[data-plugin-action^="${CSS.escape(plugin)}/"]`)];
  for (let i = all.length - 1; i >= 0; i--) {
    const el = all[i]!;
    const row = el.closest<HTMLElement>('[data-turn-actions]');
    if (row) row.dataset.landing = '';
    if (shown(el)) {
      setTimeout(() => { if (row) delete row.dataset.landing; }, ACTIONS_SHOWN_MS);
      return { rect: () => el.getBoundingClientRect(), outline: () => el, land: () => pressIn(el, PRESS_ROW) };
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
      // The frame grows into the whole field, which gives a little under it
      return { rect: () => field.getBoundingClientRect(), outline: () => field, land: () => pressIn(field, PRESS_WIDE) };
    }
    case 'settings':
    case 'agents':
    case 'models': {
      const el = nearest(`[data-land="${at}"]`, near);
      if (!el) return undefined;
      // The frame takes the control's shape; the control gives under it, a row or chip less than a lone icon, and the
      // gear also turns a little as something settles in behind it
      const r = el.getBoundingClientRect();
      const land = () => {
        pressIn(el, r.width > r.height * 1.4 ? PRESS_ROW : PRESS_ICON);
        if (at === 'settings') el.querySelector('svg')?.animate([{ transform: 'rotate(0)' }, { transform: 'rotate(90deg)' }], { duration: 900, easing: 'cubic-bezier(.4,0,.2,1)' });
      };
      return { rect: () => el.getBoundingClientRect(), outline: () => el, land };
    }
    case 'actions':
      return actionsTarget(plugin);
  }
}


// Sends a lit frame from the card's thumbnail to every place the plugin now lives. Called once the card reads applied,
// so a failed check flies nothing
export function flyHome(card: PluginCard, thumb: HTMLElement, onLeave?: () => void): void {
  const to = landingsFor(card);
  if (motionOff() || !to.length) return;
  const from = thumb.getBoundingClientRect();
  onLeave?.();
  // The whoosh goes with the first frame leaving, not the lift
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
      // The lit frame leaves as the whole thumbnail (its glyph inside) and grows into the outline of the control it lands
      // on, as an interface piece's does into its box
      const shape = t.outline?.();
      flyStandIn(thumb.getBoundingClientRect(), shape ?? thumb, t.rect, () => t.land?.(), undefined, { shape, lift: LIFT_MS, duration: MARK_FLY_MS });
    }, wait);
    go(delay);
    delay += STAGGER_MS;
  }
}

// An undo sends everything back the way it came: a frame leaves each place the plugin lived, shrinking from that
// control's or piece's outline into the card's thumbnail (an interface piece into its block on the thumbnail's map).
// Taken on the click, since the pieces are gone by the time the card reads undone
export interface Return {
  from: DOMRect;
  // The thumbnail map's block it goes back into; none: the thumbnail itself
  slot?: string;
}

const RETURN_LIFT_MS = 220;
const RETURN_FLY_MS = 1100;

export function captureReturns(card: PluginCard, thumb: HTMLElement): Return[] {
  const near = thumb.getBoundingClientRect();
  const out: Return[] = [];
  const seen = new Set<Landing>();
  for (const at of landingsFor(card)) {
    let land: Landing | undefined = at;
    let t: Target | undefined;
    while (land && !(t = target(land, card.plugin, near))) land = FALLBACK[land];
    const r = t?.rect();
    if (!land || !r || seen.has(land)) continue;
    seen.add(land);
    out.push({ from: r, slot: land === 'actions' ? 'message.actions' : undefined });
  }
  const id = CSS.escape(card.plugin);
  for (const el of document.querySelectorAll<HTMLElement>(`[data-plugin-piece^="${id}/"]`)) {
    if (!shown(el)) continue;
    const slot = el.closest<HTMLElement>('[data-plugin-slot]')?.dataset.pluginSlot ?? (el.closest('[data-plugin-above]') ? 'composer.above' : undefined);
    out.push({ from: el.getBoundingClientRect(), slot });
  }
  for (const el of document.querySelectorAll<HTMLElement>(`[data-plugin-buttons="${id}"]`)) {
    if (shown(el)) out.push({ from: el.getBoundingClientRect(), slot: 'composer.toolbar' });
  }
  return out;
}

export function flyBack(thumb: HTMLElement, returns: Return[]): void {
  if (motionOff() || !returns.length) return;
  playWhoosh('piece');
  returns.forEach((r, i) => setTimeout(() => {
    const block = r.slot ? thumb.querySelector<HTMLElement>(`[data-piece="${CSS.escape(r.slot)}"]`) : null;
    const into = block ?? thumb;
    flyStandIn(r.from, into, () => into.getBoundingClientRect(), () => {}, undefined, { shape: into, lift: RETURN_LIFT_MS, duration: RETURN_FLY_MS });
  }, i * STAGGER_MS));
}
