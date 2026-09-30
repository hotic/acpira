import { useEffect, useLayoutEffect, useRef, type DependencyList, type RefObject } from 'react';
import { followsBottom, scrollerUsable } from './promptStuck';

// Bottom follow for a transcript scroller (the main thread and the subagent inspector's session tab).
// Transcript changes pin the view to the bottom while following; scrolling up releases the follow and returning
// to the bottom restores it (see `followsBottom`). The scroller's own box is observed so a shrinking viewport
// (composer grows, panel narrows) keeps the tail visible.
//
// Content can also grow with no transcript change: images, mermaid diagrams and other async renders gain height
// after the pin, which left a freshly (re)opened session parked above its tail. The content box is observed for that,
// but only until the user touches the scroller: a fold opened by hand must stay under the pointer while content grows
// below it. The next transcript change that pins re-arms it, since growth after a pin belongs to that change.
export function useBottomFollow(scroller: RefObject<HTMLElement | null>, content: RefObject<HTMLElement | null>, resetKey: unknown, changes: DependencyList) {
  const pinned = useRef(true);
  const untouched = useRef(true);
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    pinned.current = true;
    untouched.current = true;
    const pin = () => { if (scrollerUsable(el) && pinned.current) el.scrollTop = el.scrollHeight; };
    pin();
    // A hidden sidebar collapses the scroller to no box and fires a scroll that looks like "left the bottom".
    let lastTop = el.scrollTop;
    const onScroll = () => {
      if (!scrollerUsable(el)) return;
      pinned.current = followsBottom(el, pinned.current, lastTop);
      lastTop = el.scrollTop;
    };
    const touched = () => { untouched.current = false; };
    const grew = () => { if (untouched.current) pin(); };
    el.addEventListener('scroll', onScroll, { passive: true });
    el.addEventListener('pointerdown', touched, { passive: true });
    el.addEventListener('keydown', touched);
    const box = new ResizeObserver(pin);
    box.observe(el);
    const body = new ResizeObserver(grew);
    if (content.current) body.observe(content.current);
    return () => {
      box.disconnect();
      body.disconnect();
      el.removeEventListener('scroll', onScroll);
      el.removeEventListener('pointerdown', touched);
      el.removeEventListener('keydown', touched);
    };
  // The refs are stable; `resetKey` names the transcript whose content element is mounted
  }, [resetKey]);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || !scrollerUsable(el) || !pinned.current) return;
    el.scrollTop = el.scrollHeight;
    untouched.current = true;
  }, changes);
}
