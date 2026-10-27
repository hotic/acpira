import { useSyncExternalStore } from 'react';
import type { PluginSlot, PluginUiSlot } from '@acpira/shared/plugins';
import type { HostMsg } from '@acpira/shared/protocol';
import { vscodeApi } from './vscodeApi';

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

// The frames on screen, by their window: the only sources whose messages are read
interface Frame { plugin: string; piece: string; css: () => string }
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
  const m = e.data as { acpira?: unknown; id?: unknown; method?: unknown; params?: unknown } | null;
  if (!m || typeof m !== 'object') return;
  if (m.acpira === 'ready') {
    win.postMessage({ acpira: 'load', html: pluginPiece(f.plugin, f.piece)?.html ?? '', css: f.css() }, '*');
  } else if (m.acpira === 'call' && typeof m.method === 'string') {
    const id = `pf-${++seq}`;
    calls.set(id, { win, id: m.id });
    vscodeApi().postMessage({ type: 'pluginCall', id, plugin: f.plugin, method: m.method, ...(m.params !== undefined ? { params: m.params } : {}) });
  }
}

// A frame of `plugin`'s piece; `css` gives the page's theme tokens as declarations for the frame's root
export function registerPluginFrame(win: Window, plugin: string, piece: string, css: () => string): () => void {
  if (!listening) { listening = true; window.addEventListener('message', onMessage); }
  frames.set(win, { plugin, piece, css });
  return () => {
    frames.delete(win);
    for (const [id, c] of calls) if (c.win === win) calls.delete(id);
  };
}

export function deliverPluginCalled(m: Extract<HostMsg, { type: 'pluginCalled' }>) {
  const c = calls.get(m.id);
  if (!c) return;
  calls.delete(m.id);
  c.win.postMessage({ acpira: 'result', id: c.id, ...(m.error !== undefined ? { error: m.error } : { result: m.result }) }, '*');
}

export function deliverPluginPost(m: Extract<HostMsg, { type: 'pluginPost' }>) {
  for (const [win, f] of frames) if (f.plugin === m.plugin) win.postMessage({ acpira: 'post', message: m.message }, '*');
}
