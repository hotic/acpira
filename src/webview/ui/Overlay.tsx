import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';

// Shell portals inherit the host theme and escape the composer's clipping beam.
export const ShellLayerContext = createContext<RefObject<HTMLDivElement | null> | null>(null);
export type OverlayWidth = 'sm' | 'md' | 'lg' | 'xl';
export const overlayWidth = { sm: 'w-pop-sm', md: 'w-pop-md', lg: 'w-pop-lg', xl: 'w-pop-xl' };
// Popups never outgrow the space their anchor side has: --available-height is written by the positioner's
// size middleware (seeded to 100vh before the first pass, so the var is always resolvable). The flex column
// lets the inner scroll region shrink and scroll instead of being clipped by the shell's overflow:hidden.
export const popupClass = 'popup-surface flex max-h-(--available-height) flex-col overflow-hidden rounded-lg border p-1 shadow-pop outline-none';

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

export function useShellPosition() {
  const layer = useContext(ShellLayerContext);
  const [metrics, setMetrics] = useState({ gap: 4, pad: 0, width: 0 });
  useLayoutEffect(() => {
    const shell = layer?.current;
    if (!shell) return;
    const measure = () => {
      const css = getComputedStyle(shell);
      const next = { gap: parseFloat(css.getPropertyValue('--pop-gap')) || 0, pad: parseFloat(css.getPropertyValue('--pad')) || 0, width: shell.clientWidth };
      setMetrics(prev => prev.gap === next.gap && prev.pad === next.pad && prev.width === next.width ? prev : next);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(shell);
    const attributes = new MutationObserver(measure);
    attributes.observe(shell, { attributes: true });
    return () => { observer.disconnect(); attributes.disconnect(); };
  }, [layer]);
  return { shell: layer?.current ?? undefined, ...metrics };
}
