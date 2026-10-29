import { useSyncExternalStore } from 'react';
import type { PluginSlot, PluginUiSlot } from '@acpira/shared/plugins';
import type { HostMsg } from '@acpira/shared/protocol';
import { vscodeApi } from './vscodeApi';
import { FRAME_KIT } from './plugins/kit';
import type { SessionOutline } from './plugins/outline';

// The plugins' interface pieces in effect (HostMsg pluginUi, docs/dev/plugins.md Interface) and the bridge between their
// sandboxed frames and the engine. A frame is a document of the app's own `/plugin-frame.html` loaded in an
// `<iframe sandbox="allow-scripts">`, so it has an opaque origin: it reaches nothing of the page, only postMessage. It may
// call only its own plugin's methods (`pluginCall`) and hears only what its runtime posts (`ui/post`)

let slots: PluginSlot[] = [];
// Per slot, so a hook's snapshot keeps its reference until a push changes the list
const bySlot = new Map<string, PluginSlot[]>();
const listeners = new Set<() => void>();

export const pieceKey = (plugin: string, id: string) => `${plugin}/${id}`;

export function setPluginSlots(next: PluginSlot[]) {
  slots = next;
  bySlot.clear();
  // A frame whose piece changed (a new version) is made anew, one whose piece went (switched off, safe mode) goes
  for (const l of listeners) l();
}

export function pluginPieces(slot: PluginUiSlot): PluginSlot[] {
  let list = bySlot.get(slot);
  if (!list) { list = slots.filter(s => s.slot === slot); bySlot.set(slot, list); }
  return list;
}

export function pluginPiece(plugin: string, id: string): PluginSlot | undefined {
  return slots.find(s => s.plugin === plugin && s.id === id);
}

const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };

export function usePluginPieces(slot: PluginUiSlot): PluginSlot[] {
  return useSyncExternalStore(subscribe, () => pluginPieces(slot));
}

export function usePluginPiece(plugin: string, id: string): PluginSlot | undefined {
  return useSyncExternalStore(subscribe, () => pluginPiece(plugin, id));
}

// The frames on screen, by their window: the only sources whose messages are read. A frame of a document the page holds
// itself (a skin's backdrop, a card's thumbnail) brings `html` and the `options` it starts with, and hears its first
// frame. Such a document may be a version nobody confirmed (a card's thumbnail), so it neither calls the plugin's
// methods nor hears its runtime: those belong to the version in effect
interface Frame { plugin: string; piece: string; css: () => string; html?: () => string; options?: unknown; state?: () => unknown; drawn?: () => void }
const frames = new Map<Window, Frame>();
// Calls in flight: the engine's answer goes back to the frame that asked, under the frame's own id
const calls = new Map<string, { win: Window; id: unknown }>();
let seq = 0;
let listening = false;

function onMessage(e: MessageEvent) {
  const win = e.source as Window | null;
  const f = win && frames.get(win);
  // A sandboxed frame without allow-same-origin always says "null"; anything else is not one of ours
  if (!f || e.origin !== 'null') return;
  const m = e.data as { acpira?: unknown; id?: unknown; method?: unknown; params?: unknown; turn?: unknown } | null;
  if (!m || typeof m !== 'object') return;
  if (m.acpira === 'ready') {
    const html = f.html ? f.html() : pluginPiece(f.plugin, f.piece)?.html ?? '';
    const state = f.state?.();
    // A document the page holds hears nothing of the session either
    const session = !f.html && outline ? { session: outline } : {};
    win.postMessage({ acpira: 'load', html, css: f.css(), kit: FRAME_KIT, ...(f.options !== undefined ? { options: f.options } : {}), ...(state !== undefined ? { state } : {}), ...session }, '*');
  } else if (m.acpira === 'scrollTo' && typeof m.turn === 'number' && !f.html) {
    scrollToExchange?.(m.turn);
  } else if (m.acpira === 'drawn') {
    f.drawn?.();
  } else if (m.acpira === 'call' && typeof m.method === 'string' && !f.html) {
    const id = `pf-${++seq}`;
    calls.set(id, { win, id: m.id });
    vscodeApi().postMessage({ type: 'pluginCall', id, plugin: f.plugin, method: m.method, ...(m.params !== undefined ? { params: m.params } : {}) });
  }
}

// A frame of `plugin`'s piece; `css` gives the page's theme tokens as declarations for the frame's root; `own` for a
// document the page holds rather than a piece in effect
export function registerPluginFrame(win: Window, plugin: string, piece: string, css: () => string, own?: Pick<Frame, 'html' | 'options' | 'state' | 'drawn'>): () => void {
  if (!listening) { listening = true; window.addEventListener('message', onMessage); }
  frames.set(win, { plugin, piece, css, ...own });
  return () => {
    frames.delete(win);
    for (const [id, c] of calls) if (c.win === win) calls.delete(id);
  };
}

// A call the page makes itself (a message.actions button), answered through the same pluginCalled
const ownCalls = new Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
export function callPluginMethod(plugin: string, method: string, params?: unknown): Promise<unknown> {
  const id = `pc-${++seq}`;
  return new Promise((resolve, reject) => {
    ownCalls.set(id, { resolve, reject });
    vscodeApi().postMessage({ type: 'pluginCall', id, plugin, method, ...(params !== undefined ? { params } : {}) });
  });
}

export function deliverPluginCalled(m: Extract<HostMsg, { type: 'pluginCalled' }>) {
  const own = ownCalls.get(m.id);
  if (own) {
    ownCalls.delete(m.id);
    if (m.error !== undefined) own.reject(new Error(m.error)); else own.resolve(m.result);
    return;
  }
  const c = calls.get(m.id);
  if (!c) return;
  calls.delete(m.id);
  c.win.postMessage({ acpira: 'result', id: c.id, ...(m.error !== undefined ? { error: m.error } : { result: m.result }) }, '*');
}

// The conversation on screen as frames hear it (plugins/outline.ts); posted only when it changed, so a streamed chunk
// that adds no tool call costs a string compare
let outline: SessionOutline | undefined;
let outlineText = '';
export function publishSessionOutline(next: SessionOutline | undefined) {
  const text = next ? JSON.stringify(next) : '';
  if (text === outlineText) return;
  outline = next;
  outlineText = text;
  for (const [win, f] of frames) if (!f.html) win.postMessage({ acpira: 'session', session: next ?? null }, '*');
}

// acpira.scrollTo(turn): the thread on screen brings that exchange into view
let scrollToExchange: ((turn: number) => void) | undefined;
// Cleared by the same thread that set it (`was`): a thread mounting for another session may have taken over already
export function setScrollToExchange(fn: ((turn: number) => void) | undefined, was?: (turn: number) => void) {
  if (fn || scrollToExchange === was) scrollToExchange = fn;
}

export function deliverPluginPost(m: Extract<HostMsg, { type: 'pluginPost' }>) {
  for (const [win, f] of frames) if (f.plugin === m.plugin && !f.html) win.postMessage({ acpira: 'post', message: m.message }, '*');
}
