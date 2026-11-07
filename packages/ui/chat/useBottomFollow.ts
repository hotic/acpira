import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type DependencyList, type RefObject } from 'react';
import { followsBottom, SCROLL_INTENT_MS, scrollerUsable } from './promptStuck';

// Whether the transcript is following its bottom, for rows that must not move under a reader who scrolled up
// (the live turn's automatic fold closes wait for them). Subscribed per reader, so a scroll re-renders nothing else
export class FollowState {
  value = true;
  // Stop following as a reader's own scroll would, before the page moves the view itself (a plugin's scrollTo)
  release: () => void = () => this.set(false);
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
    // content's end on a whole pixel, so the pinned tail lands on the same pixel whatever the history adds up to
    let lead = 0;
    const align = () => {
      const body = content.current;
      if (!body || !scrollerUsable(el)) return;
      const end = body.getBoundingClientRect().bottom - el.getBoundingClientRect().top - el.clientTop + el.scrollTop - lead;
      const frac = end - Math.floor(end);
      const next = frac < 0.01 || frac > 0.99 ? 0 : Math.round((1 - frac) * 100) / 100;
      if (next === lead) return;
      lead = next;
      el.style.paddingTop = next ? `${next}px` : '';
    };
    const pin = () => {
      if (!scrollerUsable(el) || !pinned.current) return;
      const maxTop = el.scrollHeight - el.clientHeight;
      if (maxTop > 0 && Math.abs(el.scrollTop - maxTop) > 0.5) el.scrollTop = el.scrollHeight;
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
      if (anchor) anchor.at = offset(anchor.node);
    };
    // What a press landed on and where it shows: content growing above it (a skin's borders, an image higher up) would
    // move it from under the pointer, so the view moves with it, as Chrome's scroll anchoring does (WebKit has none).
    // Growth below it (a fold it opened) moves nothing. Every scroll (a clamp, a pin) re-reads where it shows, so only
    // a layout change counts. A wheel, touch or key drops it
    let anchor: { node: Element; at: number } | undefined;
    const offset = (node: Element) => node.getBoundingClientRect().top;
    const touched = () => { untouched.current = false; intentAt = holding.current = performance.now(); };
    follow.release = () => { touched(); pinned.current = false; follow.set(false); };
    const pressed = (event: PointerEvent) => {
      pointerDown = holdingDown.current = true;
      touched();
      const node = event.target instanceof Element && content.current?.contains(event.target) ? event.target : undefined;
      anchor = node && { node, at: offset(node) };
    };
    const typed = () => { anchor = undefined; touched(); };
    const released = () => {
      if (pointerDown) {
        pointerDown = holdingDown.current = false;
        intentAt = holding.current = performance.now();
      }
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
    const grew = () => {
      align();
      if (untouched.current) {
        pin();
        return;
      }
      if (!scrollerUsable(el)) return;
      if (anchor?.node.isConnected) {
        const shift = offset(anchor.node) - anchor.at;
        if (Math.abs(shift) >= 1) el.scrollTop += shift;
        anchor.at = offset(anchor.node);
      }
      const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= 1;
      if (pinned.current !== atBottom) {
        pinned.current = atBottom;
        follow.set(atBottom);
      }
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    el.addEventListener('pointerdown', pressed, { passive: true });
    el.addEventListener('keydown', typed);
    el.addEventListener('wheel', moved, { passive: true });
    el.addEventListener('touchmove', moved, { passive: true });
    // The release can land outside the scroller (a scrollbar drag that left it)
    window.addEventListener('pointerup', released, { passive: true });
    window.addEventListener('pointercancel', released, { passive: true });
    const box = new ResizeObserver(syncBox);
    box.observe(el);
    const body = new ResizeObserver(grew);
    if (content.current) body.observe(content.current, { box: 'border-box' });
    return () => {
      el.style.paddingTop = '';
      box.disconnect();
      body.disconnect();
      el.removeEventListener('scroll', onScroll);
      el.removeEventListener('pointerdown', pressed);
      el.removeEventListener('keydown', typed);
      el.removeEventListener('wheel', moved);
      el.removeEventListener('touchmove', moved);
      window.removeEventListener('pointerup', released);
      window.removeEventListener('pointercancel', released);
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
