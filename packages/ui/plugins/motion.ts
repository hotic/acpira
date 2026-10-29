import { useEffect, useState, type RefObject } from 'react';

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

export function useWindowFocused(): boolean {
  const [focused, setFocused] = useState(() => document.hasFocus());
  useEffect(() => {
    // Focus moving into one of the page's own frames blurs the window too, while the document keeps it
    const on = () => setFocused(true), off = () => setTimeout(() => setFocused(document.hasFocus()));
    window.addEventListener('focus', on);
    window.addEventListener('blur', off);
    return () => { window.removeEventListener('focus', on); window.removeEventListener('blur', off); };
  }, []);
  return focused;
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
