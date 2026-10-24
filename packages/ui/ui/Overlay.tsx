import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';

// Shell portals inherit the host theme and escape the composer's clipping beam.
export const ShellLayerContext = createContext<RefObject<HTMLDivElement | null> | null>(null);
export type OverlayWidth = 'sm' | 'md' | 'lg' | 'xl';
export const overlayWidth = { sm: 'w-pop-sm', md: 'w-pop-md', lg: 'w-pop-lg', xl: 'w-pop-xl' };
// Popups never outgrow the space their anchor side has: --available-height is written by the positioner's
// size middleware (seeded to 100vh before the first pass, so the var is always resolvable). The flex column
// lets the inner scroll region shrink and scroll instead of being clipped by the shell's overflow:hidden.
export const popupClass = 'popup-surface group/popup flex max-h-(--available-height) flex-col overflow-hidden rounded-lg border p-1 shadow-pop outline-none';

// Count actual open lifetimes, including a controlled root or an unmounted trigger.
// Request callbacks may be cancelled and must never change the composer's count.
export function useOpenLifecycle(open: boolean, notify?: (open: boolean) => void) {
  const latest = useRef(notify);
  latest.current = notify;
  useEffect(() => {
    if (!open) return;
    latest.current?.(true);
    return () => latest.current?.(false);
  }, [open]);
}

type ShellMetrics = { gap: number; pad: number; width: number };
const DEFAULT_METRICS: ShellMetrics = { gap: 4, pad: 0, width: 0 };

interface ShellStore {
  metrics: ShellMetrics;
  listeners: Set<(m: ShellMetrics) => void>;
  disconnect: () => void;
}

const shellStores = new WeakMap<HTMLElement, ShellStore>();

function readShellMetrics(shell: HTMLElement): ShellMetrics {
  const css = getComputedStyle(shell);
  return {
    gap: parseFloat(css.getPropertyValue('--pop-gap')) || 0,
    pad: parseFloat(css.getPropertyValue('--pad')) || 0,
    width: shell.clientWidth,
  };
}

function ensureShellStore(shell: HTMLElement): ShellStore {
  let store = shellStores.get(shell);
  if (!store) {
    const listeners = new Set<(m: ShellMetrics) => void>();
    const measure = () => {
      if (!store) return;
      const next = readShellMetrics(shell);
      const prev = store.metrics;
      if (prev.gap === next.gap && prev.pad === next.pad && prev.width === next.width) return;
      store.metrics = next;
      for (const fn of listeners) fn(next);
    };
    const resize = new ResizeObserver(measure);
    resize.observe(shell);
    const attributes = new MutationObserver(measure);
    attributes.observe(shell, { attributes: true });
    store = {
      metrics: readShellMetrics(shell),
      listeners,
      disconnect: () => { resize.disconnect(); attributes.disconnect(); shellStores.delete(shell); },
    };
    shellStores.set(shell, store);
  }
  return store;
}

function subscribeShell(shell: HTMLElement, listener: (m: ShellMetrics) => void) {
  const store = ensureShellStore(shell);
  store.listeners.add(listener);
  listener(store.metrics);
  return () => {
    store.listeners.delete(listener);
  };
}

// Prime the shell's metrics store when the shell mounts so opening any popup later reads cached metrics synchronously
// on its first render without forcing a synchronous layout or a second Positioner render pass.
export function useShellLayer(layer: RefObject<HTMLElement | null>) {
  useLayoutEffect(() => {
    const shell = layer.current;
    if (!shell) return;
    const store = ensureShellStore(shell);
    return () => store.disconnect();
  }, [layer]);
}

export function useShellPosition() {
  const layer = useContext(ShellLayerContext);
  const [metrics, setMetrics] = useState<ShellMetrics>(() => (layer?.current ? ensureShellStore(layer.current).metrics : undefined) ?? DEFAULT_METRICS);
  useLayoutEffect(() => {
    const shell = layer?.current;
    if (!shell) return;
    return subscribeShell(shell, setMetrics);
  }, [layer]);
  return { shell: layer?.current ?? undefined, ...metrics };
}
