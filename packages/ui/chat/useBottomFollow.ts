import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type DependencyList, type RefObject } from 'react';
import { anchorOffset, followsBottom, pressAnchorNode, readingAnchor, readingOffset, SCROLL_INTENT_MS, scrollerUsable, type ReadingAnchor } from './promptStuck';

// Whether the transcript is following its bottom, for rows that must not move under a reader who scrolled up
// (the live turn's automatic fold closes wait for them). Subscribed per reader, so a scroll re-renders nothing else
export class FollowState {
  value = true;
  // Stop following as a reader's own scroll would, before the page moves the view itself (a plugin's scrollTo)
  release: () => void = () => this.set(false);
  // The height change about to land is a re-wrap, not growth: what the reader has stays put through it as through a
  // width change (exchangeSkip.ts lays the exchanges out of sight out again once the width has settled)
  rewrap = () => { this.rewrapAt = performance.now(); };
  rewrapAt = -Infinity;
  private listeners = new Set<() => void>();
  set(value: boolean) {
    if (value === this.value) return;
    this.value = value;
    for (const listener of this.listeners) listener();
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
}

export const FollowContext = createContext<FollowState | undefined>(undefined);
const idle = () => () => {};

export function useFollowing(): boolean {
  const state = useContext(FollowContext);
  return useSyncExternalStore(state?.subscribe ?? idle, () => state?.value ?? true);
}

// Bottom follow for a transcript scroller (the main thread and the subagent inspector's session tab).
// Transcript changes pin the view to the bottom while following; scrolling up releases the follow and returning
// to the bottom restores it (see `followsBottom`). The scroller's own box is observed so a shrinking viewport
// (composer grows, panel narrows) keeps the tail visible.
//
// Content can also grow with no transcript change: images, mermaid diagrams and other async renders gain height
// after the pin, which left a freshly (re)opened session parked above its tail. The content box is observed for that,
// but only until the user touches the scroller: a fold opened by hand must stay under the pointer while content grows
// below it. The next transcript change that pins re-arms it, since growth after a pin belongs to that change.
// Covers the fold open / close transitions (--dur-open 220ms, --dur-close 320ms) after a press inside the scroller,
// so a stream chunk arriving mid-toggle cannot re-arm content-growth pinning while the fold is still opening
const FOLD_SETTLE_MS = 400;
const DRAG_SLOP_PX = 8;
// How long after a width change, or after `FollowState.rewrap`, a height change still belongs to the re-wrap
const REWRAP_SETTLE_MS = 100;

export function useBottomFollow(scroller: RefObject<HTMLDivElement | null>, content: RefObject<HTMLDivElement | null>, resetKey: unknown, changes: DependencyList, prompt?: unknown): FollowState {
  const pinned = useRef(true);
  const untouched = useRef(true);
  const holding = useRef(-Infinity);
  const holdingDown = useRef(false);
  const [follow] = useState(() => new FollowState());
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    pinned.current = true;
    follow.set(true);
    untouched.current = true;
    holding.current = -Infinity;
    holdingDown.current = false;
    // WebKit scrolls in whole CSS pixels while content heights are fractional. Held at the bottom, every fractional
    // change above the tail (a history slice prepending on a session switch, an image landing) moved the tail by that
    // fraction, which a 2x screen showed as a one-pixel jolt up and back. A sub-pixel pad above the content puts the
    // content's end on a whole pixel, so the pinned tail lands on the same pixel whatever the history adds up to.
    // Only a held tail is aligned: a new pad moves every row by the fraction, so a reader parked above the tail (a fold
    // opened by hand, a scroll up) saw the whole thread twitch a device pixel with every growth below them
    let lead = 0;
    // The content element carrying the pad (the thread remounts its own with the transcript)
    let padded: HTMLElement | null = null;
    const align = () => {
      const body = content.current;
      if (!body || !scrollerUsable(el)) return false;
      const end = body.getBoundingClientRect().bottom - el.getBoundingClientRect().top - el.clientTop + el.scrollTop - lead;
      const frac = end - Math.floor(end);
      // A thread that fits its viewport has no tail to hold: its top is what stays put, and a pad following the height
      // of a fold animating open or shut moved every row a device pixel back and forth per frame. The pad itself can
      // be the only overflow, hence the pixel of slack while one is set
      const fits = el.scrollHeight - el.clientHeight <= (lead ? 1 : 0);
      const next = fits || frac < 0.01 || frac > 0.99 ? 0 : Math.round((1 - frac) * 100) / 100;
      if (next === lead && padded === body) return false;
      lead = next;
      // The content adds it to its own top padding (`follow-pad`, chat.css). Not a padding on the scroller: sticky prompts
      // stick at the scroller's padding edge, and handing them the pad took a property every element of the transcript
      // inherited, each change restyling them all; a width change at the tail (a pane dragged or easing beside the
      // chat) makes one in every frame
      if (padded !== body) padded?.style.removeProperty('--follow-lead');
      padded = body;
      if (next) body.style.setProperty('--follow-lead', `${next}px`);
      else body.style.removeProperty('--follow-lead');
      return true;
    };
    // What a press landed on and where it shows in the scroller: content growing above it (a skin's borders, an image or
    // closing editor higher up) would move it from under the pointer, so the view moves with it while the press and its
    // fold transition settle (FOLD_SETTLE_MS). Growth below it (a fold it opened) moves nothing. Every scroll or pin
    // re-reads where it shows, and dragging, scrolling, typing or settling past FOLD_SETTLE_MS drops it
    let anchor: { node: Element; at: number } | undefined;
    let downX = 0;
    let downY = 0;
    const syncAnchor = () => {
      if (!anchor) return;
      const at = anchorOffset(anchor.node, el);
      if (at === undefined) anchor = undefined;
      else anchor.at = at;
    };
    // A re-wrap (a pane dragged or easing beside the chat, the window resized) changes every height above a reader parked
    // above the tail, which moved the text under them by whole paragraphs. `reading` is what they have at the top of the
    // view, and the view moves with it. It is read after the reader's own scrolls and plain growth, never after such a
    // move: WebKit's offsets are whole pixels, and re-reading would add the rounding up over a drag
    let reading: ReadingAnchor | undefined;
    // The offset the last such move left, to tell its scroll event from the reader's
    let heldTop: number | undefined;
    const read = () => {
      const body = content.current;
      reading = body && !pinned.current && scrollerUsable(el) ? readingAnchor(body, el) : undefined;
    };
    const hold = () => {
      const at = reading && readingOffset(reading, el);
      if (at === undefined) return read();
      const shift = at - reading!.at;
      if (Math.abs(shift) < 0.5) return;
      el.scrollTop += shift;
      heldTop = el.scrollTop;
    };
    const pin = () => {
      if (!scrollerUsable(el) || !pinned.current) return;
      const maxTop = el.scrollHeight - el.clientHeight;
      if (maxTop > 0 && Math.abs(el.scrollTop - maxTop) > 0.5) {
        el.scrollTop = el.scrollHeight;
        syncAnchor();
      }
    };
    let lastH = -1;
    // Only re-pin when the scroller's height changes; a width-only resize (or WebKit's scrollbar gutter settling)
    // must not snap the view while a fold opens under the pointer.
    const syncBox = () => {
      const h = el.clientHeight;
      if (h <= 0 || h === lastH) return;
      const had = lastH > 0;
      lastH = h;
      if (had) pin();
    };
    syncBox();
    align();
    pin();
    // A hidden sidebar collapses the scroller to no box and fires a scroll that looks like "left the bottom".
    let lastTop = el.scrollTop;
    // The reader's own scrolling: a pointer held on the scroller (its scrollbar, a selection drag), or a wheel, touch or
    // key within SCROLL_INTENT_MS; any other upward scroll is a clamp and is pinned back (see followsBottom)
    let pointerDown = false;
    let intentAt = -Infinity;
    const intent = () => pointerDown || performance.now() - intentAt < SCROLL_INTENT_MS;
    const onScroll = () => {
      if (!scrollerUsable(el)) return;
      const was = pinned.current;
      const user = intent();
      pinned.current = followsBottom(el, was, lastTop, user, untouched.current);
      follow.set(pinned.current);
      if (was && pinned.current && !user && el.scrollHeight - el.scrollTop - el.clientHeight > 1) el.scrollTop = el.scrollHeight;
      lastTop = el.scrollTop;
      syncAnchor();
      if (lastTop !== heldTop) {
        heldTop = undefined;
        read();
      }
    };
    // Whether the browser has clamped the offset since the reader's last input: content above a held tail shrank (a
    // fold closing over it), so the tail is what stays put and the pad follows every frame. A fold closing back to
    // where it opened from never clamps: there the top stays put, and re-padding in its last frames twitched it
    let clamping = false;
    let seenTop = el.scrollTop;
    const touched = () => {
      untouched.current = false;
      intentAt = holding.current = performance.now();
      clamping = false;
      seenTop = el.scrollTop;
    };
    follow.release = () => { anchor = undefined; touched(); pinned.current = false; follow.set(false); };
    const pressed = (event: PointerEvent) => {
      pointerDown = holdingDown.current = true;
      downX = event.clientX;
      downY = event.clientY;
      touched();
      const node = pressAnchorNode(event.target, content.current);
      const at = node !== undefined ? anchorOffset(node, el) : undefined;
      anchor = node !== undefined && at !== undefined ? { node, at } : undefined;
    };
    const dragged = (event: PointerEvent) => {
      if (pointerDown && anchor && Math.hypot(event.clientX - downX, event.clientY - downY) > DRAG_SLOP_PX) {
        anchor = undefined;
      }
    };
    const typed = () => { anchor = undefined; touched(); };
    const released = () => {
      if (pointerDown) {
        pointerDown = holdingDown.current = false;
        intentAt = holding.current = performance.now();
      }
    };
    const cancelled = () => {
      anchor = undefined;
      released();
    };
    // A wheel over a nested scrollport (thought, command output, code) scrolls that box first, not the thread.
    const takesWheel = (event: WheelEvent) => {
      for (let node = event.target as Element | null; node && node !== el; node = node.parentElement) {
        if (node.scrollTop <= 0 || node.scrollHeight <= node.clientHeight) continue;
        const { overflowY } = getComputedStyle(node);
        if (overflowY === 'auto' || overflowY === 'scroll') return false;
      }
      return el.scrollTop > 0;
    };
    // A history slice can prepend before the wheel's scroll event arrives, making the corrected offset look like a
    // downward scroll. Release on upward wheel intent itself, but only when the thread takes the vertical gesture.
    const moved = (event: WheelEvent | TouchEvent) => {
      intentAt = performance.now();
      anchor = undefined;
      if ('deltaY' in event && event.deltaY < 0 && !event.ctrlKey && Math.abs(event.deltaY) >= Math.abs(event.deltaX) && takesWheel(event)) {
        pinned.current = false;
        follow.set(false);
      }
    };
    // A fold opened or closed by hand changes scrollHeight without firing a scroll event: sync the bottom state so
    // an opened fold that now overflows below shows the bottom fade (and is not yanked to the bottom by the next
    // streamed chunk), and closing it back into the viewport restores the follow.
    let lastWidth = -1;
    const grew = (entries: ResizeObserverEntry[]) => {
      const width = entries[entries.length - 1]?.contentRect.width ?? lastWidth;
      if (lastWidth >= 0 && width !== lastWidth) follow.rewrap();
      lastWidth = width;
      const rewrap = performance.now() - follow.rewrapAt < REWRAP_SETTLE_MS;
      if (anchor && !pointerDown && performance.now() - holding.current >= FOLD_SETTLE_MS) {
        anchor = undefined;
      }
      // A re-wrap keeps what the reader has: the tail when held (growth alone lets go of it once they touched the
      // thread), else the top of the view. A press still settling keeps its own row under the pointer below
      if (rewrap && !anchor && scrollerUsable(el)) {
        if (!pinned.current) hold();
        else if (!untouched.current) {
          align();
          pin();
        }
        seenTop = el.scrollTop;
      }
      if (untouched.current) {
        if (pinned.current) align();
        pin();
        if (!rewrap) read();
        return;
      }
      if (!scrollerUsable(el)) return;
      if (anchor) {
        const nowAt = anchorOffset(anchor.node, el);
        if (nowAt === undefined) {
          anchor = undefined;
        } else {
          const shift = nowAt - anchor.at;
          if (Math.abs(shift) >= 1) el.scrollTop += shift;
          syncAnchor();
        }
      }
      const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= 1;
      if (pinned.current !== atBottom) {
        pinned.current = atBottom;
        follow.set(atBottom);
      }
      // Measured after the growth, so the frame a hand-opened fold first pushes the tail out already leaves the pad.
      // A new pad moves the end by under a pixel; re-pin so a held tail stays flush instead of waiting for the next chunk
      if (el.scrollTop < seenTop - 0.5) clamping = true;
      if (atBottom && clamping && align()) pin();
      seenTop = el.scrollTop;
      if (!rewrap || pinned.current) read();
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    el.addEventListener('pointerdown', pressed, { passive: true });
    el.addEventListener('keydown', typed);
    el.addEventListener('wheel', moved, { passive: true });
    el.addEventListener('touchmove', moved, { passive: true });
    // The release or a selection drag leaving the scroller can land outside it
    window.addEventListener('pointermove', dragged, { passive: true });
    window.addEventListener('pointerup', released, { passive: true });
    window.addEventListener('pointercancel', cancelled, { passive: true });
    const box = new ResizeObserver(syncBox);
    box.observe(el);
    const body = new ResizeObserver(grew);
    if (content.current) body.observe(content.current, { box: 'border-box' });
    return () => {
      padded?.style.removeProperty('--follow-lead');
      box.disconnect();
      body.disconnect();
      el.removeEventListener('scroll', onScroll);
      el.removeEventListener('pointerdown', pressed);
      el.removeEventListener('keydown', typed);
      el.removeEventListener('wheel', moved);
      el.removeEventListener('touchmove', moved);
      window.removeEventListener('pointermove', dragged);
      window.removeEventListener('pointerup', released);
      window.removeEventListener('pointercancel', cancelled);
    };
  // The refs are stable; `resetKey` names the transcript whose content element is mounted
  }, [resetKey]);
  // A new prompt (sent here or from another window) brings the reader back to the tail, even after a scroll away or a
  // click in the thread (a plugin card's Apply) released the follow; `prompt` is the last user turn, whose reference
  // stays the same across stream chunks and history slices (shared/reuse.ts)
  const lastPrompt = useRef(prompt);
  useLayoutEffect(() => {
    if (lastPrompt.current === prompt) return;
    const had = lastPrompt.current !== undefined;
    lastPrompt.current = prompt;
    const el = scroller.current;
    if (!had || !el || !scrollerUsable(el)) return;
    pinned.current = true;
    untouched.current = true;
    holding.current = -Infinity;
    holdingDown.current = false;
    follow.set(true);
    el.scrollTop = el.scrollHeight;
  }, [prompt]);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || !scrollerUsable(el) || !pinned.current) return;
    if (holdingDown.current || performance.now() - holding.current < FOLD_SETTLE_MS) return;
    el.scrollTop = el.scrollHeight;
    untouched.current = true;
  }, changes);
  return follow;
}
