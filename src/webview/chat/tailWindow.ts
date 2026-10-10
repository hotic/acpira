import { useContext, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { flushSync } from 'react-dom';
import { QuietEntranceContext } from '../ui/Row';

// A live turn opened with a long history renders its tail first and mounts the earlier items in idle slices after
// the first paint. Mounting every row of a 700-block live turn in one commit took ~140 ms in a production build
// (LAB `performance.preview.html`, `scripts/probe-fold-perf.ts --switch`); the last 60 items alone paint like a
// settled turn (~30 ms). Settled turns need no window: their process folds away and closed bodies mount lazily.
export const TAIL_THRESHOLD = 150;
export const TAIL_SIZE = 60;
const SLICE = 120;
// Idle callbacks wait for a quiet main thread; a streaming turn rarely has one, so a slice runs at the latest after this
const SLICE_TIMEOUT_MS = 250;

export interface TailWindow {
  // Index of the first item to render; earlier ones are not mounted yet
  start: number;
  // Provide to the windowed rows: rows mounted by a slice are restored history and skip their entrance
  quiet: { readonly current: boolean };
}

// `count` items, of which only the tail mounts when the turn first renders live with more than TAIL_THRESHOLD.
// `anchor` is an element of the turn: a slice mounts above the reader's view, so the scroller is moved by what was
// added unless the view sits at the bottom (then it stays there) or the turn starts below the view.
export function useTailWindow(count: number, live: boolean, anchor: RefObject<HTMLElement | null>): TailWindow {
  const [start, setStart] = useState(() => live && count > TAIL_THRESHOLD ? count - TAIL_SIZE : 0);
  const outer = useContext(QuietEntranceContext);
  const slicing = useRef(false);
  const [quiet] = useState(() => ({ get current() { return slicing.current || outer.current; } }));
  const before = useRef<{ scroller: HTMLElement; top: number; height: number; bottom: boolean } | undefined>(undefined);

  useEffect(() => {
    if (start === 0) return;
    const grow = () => {
      const scroller = anchor.current ? scrollerOf(anchor.current) : undefined;
      // Read from the geometry, not the follow state: a scroll the reader just made may not have dispatched its event yet
      before.current = scroller ? { scroller, top: scroller.scrollTop, height: scroller.scrollHeight,
        bottom: scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 1 } : undefined;
      slicing.current = true;
      // Committed right here, so no streamed push lands between the measure above and the correction below
      flushSync(() => setStart(current => Math.max(0, current - SLICE)));
    };
    if (typeof requestIdleCallback === 'function') {
      const id = requestIdleCallback(grow, { timeout: SLICE_TIMEOUT_MS });
      return () => cancelIdleCallback(id);
    }
    const timer = setTimeout(grow, 16);
    return () => clearTimeout(timer);
  }, [start, anchor]);

  useLayoutEffect(() => {
    slicing.current = false;
    const kept = before.current;
    before.current = undefined;
    if (!kept) return;
    const { scroller, top, height, bottom } = kept;
    if (bottom) { scroller.scrollTop = scroller.scrollHeight; return; }
    // A reader above the whole turn sees nothing move
    const turnTop = anchor.current?.getBoundingClientRect().top;
    if (turnTop !== undefined && turnTop >= scroller.getBoundingClientRect().bottom) return;
    scroller.scrollTop = top + (scroller.scrollHeight - height);
  }, [start]);

  return { start, quiet };
}

// The nearest scrolling ancestor: the thread, or the subagent inspector's transcript
function scrollerOf(element: HTMLElement): HTMLElement | undefined {
  for (let el = element.parentElement; el; el = el.parentElement) {
    const { overflowY } = getComputedStyle(el);
    if (overflowY === 'auto' || overflowY === 'scroll') return el;
  }
  return undefined;
}
