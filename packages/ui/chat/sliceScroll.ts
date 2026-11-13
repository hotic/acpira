// Both transcript windows prepend history in slices. Keep the rows already on screen at the same physical position,
// including in WebKit, whose scroll offsets are whole pixels even when the added layout height is fractional.
export interface SliceScroll {
  scroller: HTMLElement;
  content: HTMLElement;
  contentHeight: number;
  contentBottom: number;
  top: number;
  bottom: boolean;
  atBottom: boolean;
}

export function captureSliceScroll(scroller: HTMLElement, following = false): SliceScroll | undefined {
  const content = scroller.firstElementChild as HTMLElement | null;
  if (!content) return;
  const atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 1;
  const rect = content.getBoundingClientRect();
  return {
    scroller,
    content,
    contentHeight: rect.height,
    contentBottom: rect.bottom - scroller.getBoundingClientRect().top,
    top: scroller.scrollTop,
    bottom: following || atBottom,
    atBottom: atBottom && rect.height >= scroller.clientHeight,
  };
}

export function restoreSliceScroll({ scroller, content, contentHeight, contentBottom, top, bottom, atBottom }: SliceScroll) {
  const added = content.getBoundingClientRect().height - contentHeight;
  const margin = Number.parseFloat(content.style.marginTop) || 0;
  // Normalize the old adjustment first so rounding errors cannot accumulate across slices or inspector transcripts.
  if (margin) content.style.marginTop = '';
  scroller.scrollTop = bottom ? scroller.scrollHeight : top + added - margin;
  // A short view gaining overflow, or a following view with a gap, deliberately moves to the new bottom.
  if (bottom && !atBottom) return;
  // Absorb the difference between fractional layout growth and the scroll the browser actually applied. Its step
  // changes with page zoom, so assuming one CSS pixel still made a zoomed WebKit transcript shake on every slice.
  const next = contentBottom - (content.getBoundingClientRect().bottom - scroller.getBoundingClientRect().top);
  content.style.marginTop = next ? `${next}px` : '';
}

// A slice moves the scroll offset by what it mounted above the reader, and a scroll the reader has in flight does not
// survive that write: WKWebView drops most of the wheel's distance, and Chromium stalls the wheel's animation for as
// long as slices keep landing (docs/dev/ui-motion-layout.md). So a slice waits for the reader: from a wheel, touch,
// key or press on the scroller until the scroll events that follow have stopped for this long
const SCROLL_QUIET_MS = 150;
// Chromium at a device scale above 1 can also run a wheel animation on from the offset it had before a slice landed,
// some frames after the write: the view drops a slice's height towards the start. A wheel turned before the page hears
// of it cannot be waited for, so the slices landed within this long are watched and such a drop is undone
const OVERRIDE_WATCH_MS = 1000;
const OVERRIDE_SLICES = 8;
// A drop is a slice's (or a run of slices') height, plus up to this much of the reader's own scroll in that frame
const OVERRIDE_SLACK_PX = 160;
const OVERRIDE_ROUNDING_PX = 4;

interface Reader {
  // Until when the reader counts as scrolling
  until: number;
  // Slices landed lately, oldest first, and where the content's end showed in the last frame
  landed?: { content: HTMLElement; heights: number[]; until: number; end: number };
}

// Per scroller, kept past the effect that asked: every slice asks again
const readers = new WeakMap<HTMLElement, Reader>();

function readerOf(scroller: HTMLElement): Reader {
  let reader = readers.get(scroller);
  if (reader) return reader;
  const state: Reader = reader = { until: -Infinity };
  const input = () => { state.until = performance.now() + SCROLL_QUIET_MS; };
  // A press or a key moves the view on purpose (a jump to a prompt, Home): nothing to undo after it
  const pressed = () => { state.landed = undefined; input(); };
  for (const type of ['wheel', 'touchmove']) scroller.addEventListener(type, input, { passive: true });
  for (const type of ['keydown', 'pointerdown']) scroller.addEventListener(type, pressed, { passive: true });
  // Momentum, a wheel animation or a dragged scrollbar keep scrolling with no further input. A scroll with no input
  // before it is the page's own (a pin, a slice's correction) and holds nothing
  scroller.addEventListener('scroll', () => { if (performance.now() < state.until) input(); }, { passive: true });
  readers.set(scroller, reader);
  return reader;
}

// Whether the reader is scrolling `scroller` right now, so a slice has to wait. Starts listening on its first call,
// which must come before any slice can land (the hooks call it as they mount). At the very top nothing is in flight,
// the offset being clamped there: a reader who outran what is mounted gets the next slice at once
export function readerScrolling(scroller: HTMLElement): boolean {
  return performance.now() < readerOf(scroller).until && scroller.scrollTop > 0;
}

// The height of the run of landed slices that `dropped` matches, forgetting that run; 0 when it is the reader's scroll
export function overriddenBy(heights: number[], dropped: number): number {
  for (let from = heights.length - 1; from >= 0; from--) {
    let sum = 0;
    for (let to = from; to < heights.length; to++) {
      sum += heights[to]!;
      if (dropped - sum > -OVERRIDE_ROUNDING_PX && dropped - sum < OVERRIDE_SLACK_PX) {
        heights.splice(from, to - from + 1);
        return sum;
      }
    }
  }
  return 0;
}

// `restoreSliceScroll`, then the watch for the engine undoing it (OVERRIDE_WATCH_MS)
export function landSlice(kept: SliceScroll) {
  const { scroller, content } = kept;
  const height = content.getBoundingClientRect().height;
  restoreSliceScroll(kept);
  const added = height - kept.contentHeight;
  if (added <= 0 || typeof requestAnimationFrame !== 'function') return;
  const reader = readerOf(scroller);
  const end = () => content.getBoundingClientRect().bottom - scroller.getBoundingClientRect().top;
  const until = performance.now() + OVERRIDE_WATCH_MS;
  const watched = reader.landed;
  if (watched?.content === content) {
    watched.heights.push(added);
    if (watched.heights.length > OVERRIDE_SLICES) watched.heights.shift();
    watched.until = until;
    watched.end = end();
    return;
  }
  const landed = reader.landed = { content, heights: [added], until, end: end() };
  const check = () => {
    if (reader.landed !== landed) return;
    if (!content.isConnected || performance.now() > landed.until) {
      reader.landed = undefined;
      return;
    }
    const undone = overriddenBy(landed.heights, end() - landed.end);
    if (undone) scroller.scrollTop += undone;
    landed.end = end();
    requestAnimationFrame(check);
  };
  requestAnimationFrame(check);
}
