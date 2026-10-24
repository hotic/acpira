import { useCallback, useLayoutEffect, useRef } from 'react';
import { cssTimeVar } from './cssTime';

// Room past the box for the popup shadow (--shadow: 28 px blur, 8 px drop), so the clip ends outside it instead of
// cutting the shadow off until the animation is removed
const SHADOW_ROOM = 40;

interface Size { width: number; height: number }

// A popup whose content changes size on a mode switch (a button bar turning into a comment field) grows out of its old box
// instead of jumping: on a `mode` change the new box starts clipped to the previous size, pinned to the edges it grows away
// from (the anchored side from Base UI's `data-side`, the aligned edge from `data-align`), and opens to full while the new
// content fades in. Only growth is animated; shrinking back is instant. Skipped under motion: none and reduced motion.
// Clip-path and opacity only, run with the Web Animations API so nothing is left on the element afterwards.
// The size is tracked by a ResizeObserver from the moment the element attaches: Base UI mounts the popup on its own render,
// so the owner may not commit again before the switch, and the observer reports after layout, so it still holds the old size
// when the switch's layout effect runs
export function useMorph<T extends HTMLElement>(mode: unknown) {
  const node = useRef<T | null>(null);
  const size = useRef<Size>(undefined);
  // The running animations, ended by a newer morph or by the element detaching
  const runs = useRef<Animation[]>([]);
  const ref = useCallback((el: T | null) => {
    node.current = el;
    if (!el) return;
    size.current = { width: el.offsetWidth, height: el.offsetHeight };
    const observer = new ResizeObserver(() => { size.current = { width: el.offsetWidth, height: el.offsetHeight }; });
    observer.observe(el);
    return () => {
      observer.disconnect();
      for (const run of runs.current) run.cancel();
      runs.current = [];
      node.current = null;
      size.current = undefined;
    };
  }, []);
  const seen = useRef(mode);
  useLayoutEffect(() => {
    if (Object.is(seen.current, mode)) return;
    seen.current = mode;
    const el = node.current;
    const from = size.current;
    if (!el || !from) return;
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    size.current = { width, height };
    const dw = Math.max(0, width - from.width);
    const dh = Math.max(0, height - from.height);
    if (!dw && !dh) return;
    if (el.closest('[data-motion="none"]') || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return;
    // Growing away from the anchor: a popup above it (side top) grows upward, an end-aligned one leftward
    const side = el.dataset.side ?? 'bottom';
    const align = el.dataset.align ?? 'center';
    const top = side === 'top' ? dh : side === 'bottom' ? 0 : dh / 2;
    const left = align === 'end' ? dw : align === 'center' ? dw / 2 : 0;
    const radius = getComputedStyle(el).borderTopLeftRadius;
    const start = `inset(${top}px ${dw - left}px ${dh - top}px ${left}px round ${radius})`;
    const end = `inset(-${SHADOW_ROOM}px round ${radius})`;
    const duration = cssTimeVar(el, '--dur-open', 220);
    const easing = 'cubic-bezier(0.2, 0.7, 0.2, 1)';
    for (const run of runs.current) run.cancel();
    runs.current = [el.animate([{ clipPath: start }, { clipPath: end }], { duration, easing })];
    for (const child of el.children) runs.current.push(child.animate([{ opacity: 0 }, { opacity: 1 }], { duration, easing: 'ease' }));
  }, [mode]);
  return ref;
}
