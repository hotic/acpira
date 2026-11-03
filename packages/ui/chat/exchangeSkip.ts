import { useEffect, useRef, useState } from 'react';

// Exchanges out of sight skip style, layout and paint (`content-visibility: auto` on `[data-exchange][data-seen]`, chat.css),
// so a long transcript costs a stream chunk, a fold or a relayout about what its visible part costs. The thread has
// `overflow-anchor: none`, so an exchange coming back into view must keep the height it was skipped at, or the text under
// the reader moves: `data-seen` is set only a frame after the exchange was laid out at its real size, when the browser
// remembers that size (`contain-intrinsic-block-size: auto`). A remembered size goes stale when the exchange's width changes
// (window or panel resize) or the text metrics do (appearance, locale, font sizes): such an exchange drops the mark, is laid out in full
// once, which is what every exchange costs without skipping, and is marked again.
// WebKit is left out: skipped session rows painted blank there (`[data-session]`, chat.css, docs/dev/webview.md).

interface Skipper {
  ref: (el: HTMLElement | null) => (() => void) | void;
  remeasure: () => void;
}

const NONE: Skipper = { ref: () => {}, remeasure: () => {} };

function createSkipper(): Skipper {
  if (typeof ResizeObserver !== 'function' || typeof requestAnimationFrame !== 'function') return NONE;
  if (typeof CSS !== 'undefined' && CSS.supports('-webkit-nbsp-mode', 'space')) return NONE;
  // The width each marked exchange was measured at
  const widths = new WeakMap<Element, number>();
  const observed = new Set<HTMLElement>();
  // Exchanges to mark / unmark in the next frame, with the width they were laid out at
  let mark = new Map<HTMLElement, number>();
  let unmark = new Map<HTMLElement, number>();
  let frame = 0;
  const schedule = () => {
    if (!frame && (mark.size || unmark.size)) frame = requestAnimationFrame(flush);
  };
  const flush = () => {
    frame = 0;
    const marking = mark;
    const unmarking = unmark;
    mark = new Map();
    unmark = new Map();
    for (const [el, width] of marking) {
      if (!el.isConnected || el.hasAttribute('data-seen')) continue;
      el.setAttribute('data-seen', '');
      widths.set(el, width);
    }
    // Shown in full for this frame's layout, marked again in the next: the observer reports nothing when the height came
    // out the same, so the mark is not left to it
    for (const [el, width] of unmarking) {
      if (!el.isConnected) continue;
      el.removeAttribute('data-seen');
      widths.delete(el);
      mark.set(el, width);
    }
    schedule();
  };
  // Mark and unmark in a frame of their own, never in the callback: a size change caused there is a ResizeObserver loop
  const observer = new ResizeObserver(entries => {
    for (const entry of entries) {
      const el = entry.target as HTMLElement;
      const width = entry.borderBoxSize?.[0]?.inlineSize ?? entry.contentRect.width;
      if (!el.hasAttribute('data-seen')) mark.set(el, width);
      else if (widths.get(el) !== width) unmark.set(el, width);
    }
    schedule();
  });
  return {
    ref: el => {
      if (!el) return;
      observed.add(el);
      observer.observe(el);
      return () => {
        observed.delete(el);
        observer.unobserve(el);
        mark.delete(el);
        unmark.delete(el);
      };
    },
    remeasure: () => {
      for (const el of observed) {
        const width = widths.get(el);
        if (width !== undefined) unmark.set(el, width);
      }
      schedule();
    },
  };
}

// The ref for each exchange `section`. `appearance`, `locale` and `fontSizes` change text metrics without changing the width
export function useExchangeSkip(appearance: unknown, locale: unknown, fontSizes: unknown): Skipper['ref'] {
  const [skipper] = useState(createSkipper);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    skipper.remeasure();
  }, [appearance, locale, fontSizes, skipper]);
  return skipper.ref;
}
