import { useSyncExternalStore } from 'react';
import { flushSync } from 'react-dom';
import type { PluginSkin, PluginsView } from '@acpira/shared/plugins';
import type { HostMsg } from '@acpira/shared/protocol';
import { desktopShell } from '../desktop';
import { motionOff, revealKeepsOld, revealSkin, type Point } from '../effects/magic';
import { playWhoosh } from '../effects/whoosh';
import { applyPluginTheme } from '../pluginTheme';
import { vscodeApi } from '../vscodeApi';

// The page's side of plugin skins and cards (docs/dev/plugins.md Interface, Activation):
// - the plugins view, which a card reads its state from (asked for once a card is on screen, then pushed on every change);
// - the skin on screen and the one being brought in. A theme push the user caused with a card's button (`expectReveal`)
//   waits for the new backdrop's first frame, then a circle from that button uncovers it (effects/magic.ts); any other
//   push (another window, the settings page, start-up) applies at once.

type Listener = () => void;
const listeners = new Set<Listener>();
const emit = () => { for (const l of listeners) l(); };
const subscribe = (l: Listener) => { listeners.add(l); return () => { listeners.delete(l); }; };

let view: PluginsView | undefined;
let asked = false;

export function setPluginsView(next: PluginsView) {
  view = next;
  emit();
}

// The first card on screen asks; the engine keeps sending the view on every change after that
export function askPluginsView() {
  if (asked) return;
  asked = true;
  vscodeApi().postMessage({ type: 'plugins' });
}

export function usePluginsView(): PluginsView | undefined {
  return useSyncExternalStore(subscribe, () => view);
}

// Only the desktop serves the frame document a backdrop runs in (`/plugin-frame.html`); elsewhere a skin is its colours
export const backdropsShown = desktopShell();

export interface SkinState {
  // On screen: its tokens are in the theme layer and its backdrop is the visible one
  shown?: PluginSkin;
  // Being brought in: its backdrop is laid out unseen until its first frame
  incoming?: PluginSkin;
  // Going, during a reveal without View Transitions: kept on screen for the circle to uncover (under the new one) or
  // close on (over it, an undo)
  leaving?: { skin: PluginSkin; over: boolean };
}
let state: SkinState = {};
const set = (next: SkinState) => { state = next; emit(); };

export function useSkin(): SkinState {
  return useSyncExternalStore(subscribe, () => state);
}

export const skinKey = (s: PluginSkin) => `${s.plugin}@${s.snapshot}`;
const same = (a?: PluginSkin, b?: PluginSkin) => (a ? b !== undefined && skinKey(a) === skinKey(b) && a.backdrop === b.backdrop && a.glass === b.glass && a.base === b.base : !b);

// A card's button: the theme push that brings what it asked for is revealed from `at`; applying asks for `plugin` as the
// skin, undoing for any other. A push that does not match (another window, the settings page) applies at once and leaves
// the origin waiting; stale after a while, since an apply that failed sends no push
let origin: { at: Point; back: boolean; plugin: string; until: number } | undefined;
export function expectReveal(at: Point, back: boolean, plugin: string) {
  origin = { at, back, plugin, until: performance.now() + 15_000 };
}
const answers = (o: NonNullable<typeof origin>, skin: PluginSkin | undefined) => (o.back ? skin?.plugin !== o.plugin : skin?.plugin === o.plugin);

// The new backdrop's first frame, by its key
const drawnWaits = new Map<string, () => void>();
export function backdropDrawn(key: string) {
  drawnWaits.get(key)?.();
}
const DRAWN_WAIT_MS = 1500;
function waitDrawn(key: string): Promise<void> {
  return new Promise(done => {
    const timer = setTimeout(finish, DRAWN_WAIT_MS);
    function finish() { clearTimeout(timer); drawnWaits.delete(key); done(); }
    drawnWaits.set(key, finish);
  });
}

// Pushes apply one after another, so a reveal in flight finishes before the next starts; the latest wins, and one built
// from an older plugins state than the page shows is dropped (two engines' pushes can cross)
let chain: Promise<void> = Promise.resolve();
let latest = 0;
let generation = -1;

export function receivePluginTheme(m: Extract<HostMsg, { type: 'pluginTheme' }>) {
  if (m.generation !== undefined) {
    if (m.generation < generation) return;
    generation = m.generation;
  }
  const n = ++latest;
  chain = chain.then(() => (n === latest ? bringIn(m) : undefined)).catch(() => {});
}

async function bringIn(m: Extract<HostMsg, { type: 'pluginTheme' }>) {
  const skin = m.skin ?? undefined;
  const o = origin && origin.until > performance.now() && answers(origin, skin) ? origin : undefined;
  if (o || (origin && origin.until <= performance.now())) origin = undefined;
  const commit = () => { applyPluginTheme(m.css); set({ shown: skin }); };
  if (!o || motionOff() || same(state.shown, skin)) { commit(); return; }
  // The backdrop is laid out unseen first, so the circle uncovers a scene already drawn rather than an empty frame
  if (skin?.backdrop && backdropsShown) {
    set({ ...state, incoming: skin });
    await waitDrawn(skinKey(skin));
  }
  // The edge glows in the colour of the skin it uncovers, or of the one it takes away
  const ring = (o.back ? state.shown?.brand : skin?.brand) || getComputedStyle(document.querySelector('.acp-shell') ?? document.documentElement).getPropertyValue('--brand').trim();
  playWhoosh('skin');
  if (revealKeepsOld()) {
    await revealSkin(o.at, () => flushSync(commit), { back: o.back, ring });
    return;
  }
  // Without a snapshot of the old page, the old backdrop stays while the circle moves: the new one opens over it, or
  // (undo) the old one closes over the new
  const before = state.shown;
  const keep = before?.backdrop && backdropsShown ? { skin: before, over: o.back } : undefined;
  const frame = (s?: PluginSkin) => (s ? document.querySelector<HTMLElement>(`[data-skin-frame="${CSS.escape(skinKey(s))}"]`) : null);
  await revealSkin(o.at, () => flushSync(() => { applyPluginTheme(m.css); set({ shown: skin, ...(keep ? { leaving: keep } : {}) }); }), {
    back: o.back, ring, backdrop: () => (o.back ? frame(before) : frame(skin)),
  });
  if (state.leaving) set({ ...state, leaving: undefined });
}

// Card thumbnails: the backdrop of the version a card offers, read from its frozen copy once per page. null: it has none
// (gone, too large)
const backdrops = new Map<string, string | null>();
const backdropAsked = new Set<string>();

export function receivePluginBackdrop(m: Extract<HostMsg, { type: 'pluginBackdrop' }>) {
  backdrops.set(`${m.plugin}@${m.snapshot}`, m.html ?? null);
  emit();
}

export function usePluginBackdrop(plugin: string, snapshot: string): string | null | undefined {
  const key = `${plugin}@${snapshot}`;
  const html = useSyncExternalStore(subscribe, () => backdrops.get(key));
  if (html === undefined && backdropsShown && !backdropAsked.has(key)) {
    backdropAsked.add(key);
    // Posted outside render's commit is harmless: the reply only fills the cache
    queueMicrotask(() => vscodeApi().postMessage({ type: 'pluginBackdrop', plugin, snapshot }));
  }
  return html;
}
