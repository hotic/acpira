import { useEffect, type RefObject } from 'react';

const SETTLE_MS = 800;
// Covers the fade-out in base.css (--dur-close, 320ms) with room to spare; the attribute only carries the transition
const FADE_MS = 500;

type Reveal = { settle?: ReturnType<typeof setTimeout>; release?: ReturnType<typeof setTimeout> };

// True when the point sits in the element's scrollbar gutter (vertical on the right, horizontal at the bottom) and that bar
// actually scrolls. clientLeft + clientWidth is where the vertical bar starts; a width-0 bar (scroll-fade) has no gutter.
function inGutter(el: Element, x: number, y: number) {
  if (!(el instanceof HTMLElement)) return false;
  const r = el.getBoundingClientRect();
  if (x < r.left || x >= r.right || y < r.top || y >= r.bottom) return false;
  const vertical = el.offsetWidth - el.clientWidth - 2 * el.clientLeft > 0 && el.scrollHeight > el.clientHeight;
  if (vertical && x >= r.left + el.clientLeft + el.clientWidth) return true;
  const horizontal = el.offsetHeight - el.clientHeight - 2 * el.clientTop > 0 && el.scrollWidth > el.clientWidth;
  return horizontal && y >= r.top + el.clientTop + el.clientHeight;
}

// Overlay scrollbars (base.css): a scroller is stamped data-scrolling while its content moves (cleared SETTLE_MS after the last
// scroll) and data-scrollbar-hover while the pointer is in its gutter; either lights the thumb, hover also widens it.
// data-scrollbar-anim carries the transition from the first of them until the fade-out after the last has run, so the thumb
// eases in, out and between widths. One capturing listener per event on the shell root covers every scroller inside it,
// portaled menus included; the pointer is resolved once per frame, where the layout it reads is due anyway.
export function useScrollReveal(root: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const states = new Map<Element, Reveal>();
    const state = (target: Element) => {
      let s = states.get(target);
      if (!s) states.set(target, s = {});
      return s;
    };
    const light = (target: Element) => {
      const s = state(target);
      clearTimeout(s.release);
      s.release = undefined;
      if (!target.hasAttribute('data-scrollbar-anim')) target.setAttribute('data-scrollbar-anim', '');
    };
    // Once neither scrolling nor hovered, let the fade-out run, then drop the transition carrier
    const release = (target: Element) => {
      if (target.hasAttribute('data-scrolling') || target.hasAttribute('data-scrollbar-hover')) return;
      const s = state(target);
      clearTimeout(s.release);
      s.release = setTimeout(() => {
        target.removeAttribute('data-scrollbar-anim');
        states.delete(target);
      }, FADE_MS);
    };

    const onScroll = (event: Event) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      light(target);
      if (!target.hasAttribute('data-scrolling')) target.setAttribute('data-scrolling', '');
      const s = state(target);
      clearTimeout(s.settle);
      s.settle = setTimeout(() => {
        s.settle = undefined;
        target.removeAttribute('data-scrolling');
        release(target);
      }, SETTLE_MS);
    };

    let hovered: Element | undefined;
    let frame = 0;
    let pointer: { target: EventTarget | null; x: number; y: number } | undefined;
    const setHover = (next: Element | undefined) => {
      if (next === hovered) return;
      if (hovered) {
        hovered.removeAttribute('data-scrollbar-hover');
        release(hovered);
      }
      hovered = next;
      if (next) {
        light(next);
        next.setAttribute('data-scrollbar-hover', '');
      }
    };
    const resolve = () => {
      frame = 0;
      const p = pointer;
      // Over a scrollbar the event targets the scroller itself, so only that element needs the gutter test
      setHover(p && p.target instanceof Element && inGutter(p.target, p.x, p.y) ? p.target : undefined);
    };
    const onPointerMove = (event: PointerEvent) => {
      pointer = { target: event.target, x: event.clientX, y: event.clientY };
      if (!frame) frame = requestAnimationFrame(resolve);
    };
    const onPointerLeave = () => {
      pointer = undefined;
      cancelAnimationFrame(frame);
      frame = 0;
      setHover(undefined);
    };

    el.addEventListener('scroll', onScroll, { capture: true, passive: true });
    el.addEventListener('pointermove', onPointerMove, { capture: true, passive: true });
    el.addEventListener('pointerleave', onPointerLeave);
    return () => {
      el.removeEventListener('scroll', onScroll, { capture: true });
      el.removeEventListener('pointermove', onPointerMove, { capture: true });
      el.removeEventListener('pointerleave', onPointerLeave);
      cancelAnimationFrame(frame);
      for (const [target, s] of states) {
        clearTimeout(s.settle);
        clearTimeout(s.release);
        target.removeAttribute('data-scrolling');
        target.removeAttribute('data-scrollbar-hover');
        target.removeAttribute('data-scrollbar-anim');
      }
      hovered?.removeAttribute('data-scrollbar-hover');
    };
  }, [root]);
}
