import type { PluginCard } from '@acpira/shared/transcript';
import { motionOff, pressIn } from '../effects/magic';
import { flyStandIn } from './Pieces';
import { playWhoosh } from '../effects/whoosh';

// Where an applied plugin goes to live, for the parts that have no slot of their own (docs/dev/plugins.md Activation):
// interface pieces fly from the thumbnail's map (arrivals.ts) and a skin is uncovered (skin.ts); everything else sends
// the same lit frame from the card to the control it now stands behind, the frame growing into that control's outline,
// so every kind of plugin lands somewhere visible.
// - composer: tools, MCP servers, prompts, skills and commands are the agent's to use from the next message
// - settings: hooks, a runtime and schedules work in the background, behind the gear; a style sheet whose parts are
//   not on screen goes there too, where the looks are chosen
// - restyled parts: a style sheet lands on the parts it names through the style hooks (`restyles`, found by the engine)
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

// `restyled`: the style sheet already lands on parts of the page it restyles
export function landingsFor(card: Pick<PluginCard, 'kind' | 'adds' | 'slots' | 'schedules'>, restyled = false): Landing[] {
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
  // A style sheet whose parts are not on screen goes behind the gear, where the looks are chosen; with anything else it
  // rides along. Never a flash over the whole window: that read as the window being wiped
  if (adds.includes('style') && !adds.includes('theme') && !restyled && !to.size && !slots.length) to.add('settings');
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
function nearest(selector: string, near: DOMRect, ok: (el: HTMLElement) => boolean = () => true): HTMLElement | undefined {
  let best: HTMLElement | undefined, d = Infinity;
  for (const el of document.querySelectorAll<HTMLElement>(selector)) {
    if (!shown(el) || !ok(el)) continue;
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

// The part of the page a style hook names: the richer of two hooks on one part names it once ([data-session][data-state]
// is a session row too), and a control names the area it sits in (a tab's buttons the tab, the send button the
// composer). The thumbnail lights it as the block `restylePiece` (PluginCard), where its frame lifts off
const CONTROL_AREA: [prefix: string, area: string][] = [
  ['[data-part="tab-', '[data-part="tab"]'],
  ['[data-part="composer-', '[data-part="composer"]'],
  ['[data-part="header-', '[data-part="header"]'],
  ['[data-part="turn-', '[data-turn-actions]'],
  ['[data-part="session-', '[data-session]'],
];
export const restylePart = (hook: string) => (hook.startsWith('[data-session][')
  ? '[data-session]'
  : CONTROL_AREA.find(([prefix]) => hook.startsWith(prefix))?.[1] ?? hook);
export const restylePiece = (part: string) => `style:${part}`;

// A restyled part's flights: at most this many, one per part
const RESTYLE_MAX = 3;
// A part covering more than this share of the window is the page itself (the shell, the thread), not a place to land
const RESTYLE_AREA = 0.5;
// How deep a hook's single-child wrappers are followed to the box that is drawn
const RESTYLE_DEPTH = 8;

function painted(el: Element): boolean {
  const s = getComputedStyle(el);
  const fill = s.backgroundColor !== 'rgba(0, 0, 0, 0)' && s.backgroundColor !== 'transparent';
  return fill || s.backgroundImage !== 'none' || s.boxShadow !== 'none' || parseFloat(s.borderTopWidth) > 0;
}

// A hook often marks a bare wrapper around what the eye sees (the user message's band around its bubble): follow the
// only child down and take the deepest box that draws itself, else the hook's own element
function drawnBox(el: HTMLElement): HTMLElement {
  let box = el, at: Element = el;
  for (let i = 0; i < RESTYLE_DEPTH && at.childElementCount === 1; i++) {
    at = at.firstElementChild!;
    if (!(at instanceof HTMLElement) || !shown(at)) break;
    if (painted(at)) box = at;
  }
  return box;
}

const pressFor = (el: HTMLElement) => {
  const r = el.getBoundingClientRect();
  return r.width > innerWidth * 0.4 ? PRESS_WIDE : r.width > r.height * 1.4 ? PRESS_ROW : PRESS_ICON;
};

// The parts a style sheet restyles, each the shown one nearest the card (a user message: the one above it), in the
// order the sheet names them; none when none of them is on screen, and the sheet goes behind the gear instead
function restyleTargets(card: Pick<PluginCard, 'restyles'>, near: DOMRect): (Target & { piece: string })[] {
  const out: (Target & { piece: string })[] = [];
  const taken = new Set<HTMLElement>();
  const small = (el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    return r.width * r.height <= innerWidth * innerHeight * RESTYLE_AREA;
  };
  const parts = new Set<string>();
  for (const hook of card.restyles ?? []) {
    if (out.length >= RESTYLE_MAX) break;
    const part = restylePart(hook);
    if (parts.has(part)) continue;
    let el: HTMLElement | undefined;
    // A hook from a newer engine this page cannot parse is skipped, not thrown
    try { el = nearest(hook, near, e => small(e) && !taken.has(e)); } catch { continue; }
    if (!el) continue;
    taken.add(el);
    parts.add(part);
    const box = drawnBox(el);
    out.push({ piece: restylePiece(part), rect: () => box.getBoundingClientRect(), outline: () => box, land: () => pressIn(box, pressFor(box)) });
  }
  return out;
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
      // gear also turns as something settles in behind it: a whole turn, so it comes to rest as the icon it was, with no
      // jump back when the animation lets go
      const r = el.getBoundingClientRect();
      const land = () => {
        pressIn(el, r.width > r.height * 1.4 ? PRESS_ROW : PRESS_ICON);
        if (at === 'settings') el.querySelector('svg')?.animate([{ transform: 'rotate(0)' }, { transform: 'rotate(360deg)' }], { duration: 1100, easing: 'cubic-bezier(.4,0,.2,1)' });
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
  if (motionOff()) return;
  const from = thumb.getBoundingClientRect();
  const restyled = restyleTargets(card, from);
  const to = landingsFor(card, restyled.length > 0);
  if (!to.length && !restyled.length) return;
  onLeave?.();
  // The whoosh goes with the first frame leaving, not the lift
  setTimeout(() => playWhoosh('piece'), LIFT_MS);
  const fly = (t: Target, piece?: string) => {
    // The lit frame leaves as the whole thumbnail (its glyph inside), or a restyled part's lit block on its map, and
    // grows into the outline of the control it lands on, as an interface piece's does into its box
    const block = piece ? thumb.querySelector<HTMLElement>(`[data-piece="${CSS.escape(piece)}"]`) : null;
    const start = block ?? thumb;
    const shape = t.outline?.();
    flyStandIn(start.getBoundingClientRect(), shape ?? start, t.rect, () => t.land?.(), undefined, { shape, lift: LIFT_MS, duration: MARK_FLY_MS });
  };
  let delay = 0;
  // Restyled parts first: they are what the user asked to change
  for (const t of restyled) {
    setTimeout(() => fly(t, t.piece), delay);
    delay += STAGGER_MS;
  }
  // One flight per control: two landings that fall back onto the same one fly once
  const flown = new Set<Landing>();
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
      fly(t);
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
  const restyled = restyleTargets(card, near);
  for (const t of restyled) {
    const r = t.rect();
    if (r) out.push({ from: r, slot: t.piece });
  }
  const seen = new Set<Landing>();
  for (const at of landingsFor(card, restyled.length > 0)) {
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
