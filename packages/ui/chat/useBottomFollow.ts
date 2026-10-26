import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type DependencyList, type RefObject } from 'react';
import { followsBottom, SCROLL_INTENT_MS, scrollerUsable } from './promptStuck';

// Whether the transcript is following its bottom, for rows that must not move under a reader who scrolled up
// (the live turn's automatic fold closes wait for them). Subscribed per reader, so a scroll re-renders nothing else
export class FollowState {
  value = true;
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

export function useBottomFollow(scroller: RefObject<HTMLDivElement | null>, content: RefObject<HTMLDivElement | null>, resetKey: unknown, changes: DependencyList): FollowState {
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
      el.style.setProperty('--thread-h', `${h}px`);
      if (had) pin();
    };
    syncBox();
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
      pinned.current = followsBottom(el, was, lastTop, intent());
      follow.set(pinned.current);
      if (was && pinned.current && el.scrollHeight - el.scrollTop - el.clientHeight > 1) el.scrollTop = el.scrollHeight;
      lastTop = el.scrollTop;
    };
    const touched = () => { untouched.current = false; intentAt = holding.current = performance.now(); };
    const pressed = () => { pointerDown = holdingDown.current = true; touched(); };
    const released = () => {
      if (pointerDown) {
        pointerDown = holdingDown.current = false;
        intentAt = holding.current = performance.now();
      }
    };
    const moved = () => { intentAt = performance.now(); };
    // A fold opened or closed by hand changes scrollHeight without firing a scroll event: sync the bottom state so
    // an opened fold that now overflows below shows the bottom fade (and is not yanked to the bottom by the next
    // streamed chunk), and closing it back into the viewport restores the follow.
    const grew = () => {
      if (untouched.current) {
        pin();
        return;
      }
      if (!scrollerUsable(el)) return;
      const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= 1;
      if (pinned.current !== atBottom) {
        pinned.current = atBottom;
        follow.set(atBottom);
      }
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    el.addEventListener('pointerdown', pressed, { passive: true });
    el.addEventListener('keydown', touched);
    el.addEventListener('wheel', moved, { passive: true });
    el.addEventListener('touchmove', moved, { passive: true });
    // The release can land outside the scroller (a scrollbar drag that left it)
    window.addEventListener('pointerup', released, { passive: true });
    window.addEventListener('pointercancel', released, { passive: true });
    const box = new ResizeObserver(syncBox);
    box.observe(el);
    const body = new ResizeObserver(grew);
    if (content.current) body.observe(content.current);
    return () => {
      box.disconnect();
      body.disconnect();
      el.removeEventListener('scroll', onScroll);
      el.removeEventListener('pointerdown', pressed);
      el.removeEventListener('keydown', touched);
      el.removeEventListener('wheel', moved);
      el.removeEventListener('touchmove', moved);
      window.removeEventListener('pointerup', released);
      window.removeEventListener('pointercancel', released);
    };
  // The refs are stable; `resetKey` names the transcript whose content element is mounted
  }, [resetKey]);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || !scrollerUsable(el) || !pinned.current) return;
    if (holdingDown.current || performance.now() - holding.current < FOLD_SETTLE_MS) return;
    el.scrollTop = el.scrollHeight;
    untouched.current = true;
  }, changes);
  return follow;
}
