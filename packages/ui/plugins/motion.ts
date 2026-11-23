import { useEffect, useState, useSyncExternalStore, type RefObject } from 'react';

// When a plugin document may animate: the window has focus, the system does not ask for less motion, and the frame is on
// screen. A document that may not just holds its last frame

export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() => !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
  useEffect(() => {
    const q = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    if (!q) return;
    const on = () => setReduced(q.matches);
    q.addEventListener('change', on);
    return () => q.removeEventListener('change', on);
  }, []);
  return reduced;
}

// Shared window-focus store: drives both React hooks (WorkingBeam / plugins) and non-React canvas engines (Orb, MetalRing),
// and mirrors `!focused` onto `html[data-window-blurred]` so CSS pauses ambient loops while unfocused on Windows WebView2
let focusedState = typeof document !== 'undefined' ? document.hasFocus() : true;
let watching = false;
const focusSubs = new Set<() => void>();

function applyFocusAttr(focused: boolean) {
  if (typeof document !== 'undefined') {
    document.documentElement?.toggleAttribute('data-window-blurred', !focused);
  }
}

function setFocusedState(next: boolean) {
  if (next === focusedState) return;
  focusedState = next;
  applyFocusAttr(next);
  for (const fn of focusSubs) fn();
}

export function watchWindowFocus(): void {
  if (watching || typeof window === 'undefined' || typeof document === 'undefined') return;
  watching = true;
  focusedState = document.hasFocus();
  applyFocusAttr(focusedState);
  // Focus moving into one of the page's own frames blurs the window too, while the document keeps it
  const on = () => setFocusedState(true);
  const off = () => setTimeout(() => setFocusedState(document.hasFocus()));
  const onVis = () => setFocusedState(!document.hidden && document.hasFocus());
  window.addEventListener('focus', on);
  window.addEventListener('blur', off);
  document.addEventListener('visibilitychange', onVis);
}

export function isWindowFocused(): boolean {
  if (!watching && typeof document !== 'undefined') return document.hasFocus();
  return focusedState;
}

export function subscribeWindowFocused(fn: () => void): () => void {
  watchWindowFocus();
  focusSubs.add(fn);
  return () => { focusSubs.delete(fn); };
}

export function useWindowFocused(): boolean {
  return useSyncExternalStore(subscribeWindowFocused, isWindowFocused, () => true);
}

export function useInView(ref: RefObject<Element | null>): boolean {
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el || !('IntersectionObserver' in window)) { setSeen(true); return; }
    const io = new IntersectionObserver(([e]) => setSeen(!!e?.isIntersecting));
    io.observe(el);
    return () => io.disconnect();
  }, [ref]);
  return seen;
}
