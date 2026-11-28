import { createContext, useEffect, useRef, useState } from 'react';

// Exchanges out of sight skip style, layout and paint (`content-visibility: auto` on `[data-exchange][data-seen]`, chat.css),
// so a long transcript costs a stream chunk, a fold or a relayout about what its visible part costs. The thread has
// `overflow-anchor: none`, so an exchange coming back into view must keep the height it was skipped at, or the text under
// the reader moves: `data-seen` is set only a frame after the exchange was laid out at its real size, when the browser
// remembers that size (`contain-intrinsic-block-size: auto`). A remembered size goes stale when the exchange's width changes
// (window or panel resize) or the text metrics do (appearance, locale, font sizes): such an exchange drops the mark, is laid out in full
// once, which is what every exchange costs without skipping, and is marked again.
// A width that keeps changing (a pane dragged or easing beside the chat, the window resized) would do that in every frame,
// costing more than no skipping at all: the exchanges are laid out again only once the width has held still, and the
// ones out of sight keep their old heights until then. `rewrap` is told as that layout starts, since it moves what is
// under a reader as the width change itself does (useBottomFollow.ts).

interface Skipper {
  ref: (el: HTMLElement | null) => (() => void) | void;
  remeasure: () => void;
}

const NONE: Skipper = { ref: () => {}, remeasure: () => {} };

// How long a width has to hold still before the exchanges it changed under are measured again
const SETTLE_MS = 120;
// How many are measured again per frame: all of a long transcript at once is one long frame
const REMEASURE_BATCH = 4;

function createSkipper(rewrap: () => void, setStale: (stale: boolean) => void): Skipper {
  if (typeof ResizeObserver !== 'function' || typeof requestAnimationFrame !== 'function') return NONE;
  // The width each marked exchange was measured at
  const widths = new WeakMap<Element, number>();
  const observed = new Set<HTMLElement>();
  // Exchanges to mark / unmark in the next frame, with the width they were laid out at. `fresh`: reported in the last
  // frame, with that height
  let mark = new Map<HTMLElement, { width: number; fresh?: number }>();
  const unmark = new Map<HTMLElement, number>();
  // Marked exchanges whose width is still changing
  const stale = new Map<HTMLElement, number>();
  let settle: ReturnType<typeof setTimeout> | undefined;
  let frame = 0;
  // Whether anything still holds the height of an earlier width, told as it changes
  let wasStale = false;
  const told = () => {
    const now = stale.size + unmark.size > 0;
    if (now !== wasStale) setStale(wasStale = now);
  };
  const schedule = () => {
    if (!frame && (mark.size || unmark.size)) frame = requestAnimationFrame(flush);
  };
  const flush = () => {
    frame = 0;
    const marking = mark;
    mark = new Map();
    // A reported size waits one more frame: WebKit remembers a size that changed twice in one frame (a block fitting
    // itself from its own observer) only in the next, and a mark set before that skips the exchange at the first.
    // Content can also change between the report and this frame (a commit in between): read first, and leave such an
    // exchange to its next report
    const ready: [HTMLElement, number][] = [];
    // A chunk inside an exchange still to be measured (SkipContext): skipped over it, the exchange would keep its old height
    const pending = (el: HTMLElement) => {
      for (const queue of [marking, mark, unmark, stale]) for (const inner of queue.keys()) if (inner !== el && el.contains(inner)) return true;
      return false;
    };
    for (const [el, size] of marking) {
      if (!el.isConnected || el.hasAttribute('data-seen')) continue;
      if (size.fresh === undefined && pending(el)) mark.set(el, size);
      else if (size.fresh === undefined) ready.push([el, size.width]);
      else if (Math.abs(el.getBoundingClientRect().height - size.fresh) < 0.01) mark.set(el, { width: size.width });
    }
    for (const [el, width] of ready) {
      el.setAttribute('data-seen', '');
      widths.set(el, width);
    }
    // Shown in full for this frame's layout, marked again in the next: the observer reports nothing when the height came
    // out the same, so the mark is not left to it
    let shown = 0;
    for (const [el, width] of unmark) {
      if (shown >= REMEASURE_BATCH) break;
      unmark.delete(el);
      if (!el.isConnected) continue;
      el.removeAttribute('data-seen');
      widths.delete(el);
      mark.set(el, { width });
      // A process chunk inside it (SkipContext) skipped along with it kept its old height, which the exchange would be
      // measured and marked at: laid out again with it, it reports its new size and is marked again on its own. One that
      // reported the width itself (the exchange was in sight) waits for its own turn: all of a working turn's chunks at
      // once were one long frame
      for (const inner of el.querySelectorAll<HTMLElement>('[data-seen]')) {
        if (!observed.has(inner) || unmark.has(inner) || stale.has(inner)) continue;
        inner.removeAttribute('data-seen');
        mark.set(inner, { width: widths.get(inner) ?? 0 });
        widths.delete(inner);
      }
      shown++;
    }
    if (shown) rewrap();
    told();
    schedule();
  };
  // Mark and unmark in a frame of their own, never in the callback: a size change caused there is a ResizeObserver loop
  const observer = new ResizeObserver(entries => {
    for (const entry of entries) {
      const el = entry.target as HTMLElement;
      const box = entry.borderBoxSize?.[0];
      const width = box?.inlineSize ?? entry.contentRect.width;
      if (!el.hasAttribute('data-seen')) mark.set(el, { width, fresh: box?.blockSize ?? el.getBoundingClientRect().height });
      else if (widths.get(el) !== width) stale.set(el, width);
    }
    if (stale.size) {
      clearTimeout(settle);
      settle = setTimeout(function settled() {
        // A splitter still held (InspectorSplitter's `data-dragging`): the width rests at a bound or under a resting hand
        // and is about to change again, so measuring now is work thrown away, a long frame each batch
        if (document.querySelector('[role="separator"][data-dragging]')) { settle = setTimeout(settled, SETTLE_MS); return; }
        for (const [el, width] of stale) unmark.set(el, width);
        stale.clear();
        schedule();
      }, SETTLE_MS);
    }
    told();
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
        // Also when it stays mounted: a chunk the turn grows again (ProcessChunks `growing`) must not be skipped at a stale size
        el.removeAttribute('data-seen');
        widths.delete(el);
        mark.delete(el);
        unmark.delete(el);
        stale.delete(el);
        told();
      };
    },
    remeasure: () => {
      for (const el of observed) {
        const width = widths.get(el);
        if (width !== undefined) unmark.set(el, width);
      }
      told();
      schedule();
    },
  };
}

// The same skip for the chunks of a long process inside an exchange (chat/ProcessChunks.tsx): a working turn is one exchange
// that stays in sight, and hundreds of its steps re-wrapped on every width change. Absent outside the main thread
export const SkipContext = createContext<Skipper['ref'] | undefined>(undefined);

// The ref for each exchange `section`. `appearance`, `locale` and `fontSizes` change text metrics without changing the width;
// `rewrap` and `setStale` (FollowState) must be stable
export function useExchangeSkip(appearance: unknown, locale: unknown, fontSizes: unknown, rewrap: () => void, setStale: (stale: boolean) => void): Skipper['ref'] {
  const [skipper] = useState(() => createSkipper(rewrap, setStale));
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
