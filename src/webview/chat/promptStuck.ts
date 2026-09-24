// A hidden sidebar webview collapses the thread to no box. IntersectionObserver then
// reports every exchange sentinel as off-screen, and scroll listeners treat the empty
// viewport as "left the bottom". Callers must ignore those frames and re-check once
// the thread has a box again.

export function scrollerUsable(el: { clientHeight: number; clientWidth: number }): boolean {
  return el.clientHeight >= 1 && el.clientWidth >= 1;
}

// Bottom-follow verdict for a scroll event. Only an upward scroll can release the follow: scroll events are dispatched a frame
// late, so the one caused by a pin can land after the viewport shrank again (connecting notice, then composer chips) and read a
// gap the user never made — treating that as "left the bottom" skipped every later pin and left the tail under the composer
export function followsBottom(el: { scrollHeight: number; scrollTop: number; clientHeight: number }, pinned: boolean, lastTop: number): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight < 48 || (pinned && el.scrollTop >= lastTop);
}

// The same verdict from plain geometry, for a synchronous check before the first paint: the sentinel has left through the top edge
export function promptIsStuckAt(sentinel: { bottom: number }, root: { top: number; height: number; width: number }): boolean | undefined {
  if (root.height < 1 || root.width < 1) return undefined;
  return sentinel.bottom <= root.top;
}

export function promptIsStuck(entry: {
  isIntersecting: boolean;
  boundingClientRect: { top: number };
  rootBounds: { top: number; height: number; width: number } | null;
}): boolean | undefined {
  const root = entry.rootBounds;
  if (!root || root.height < 1 || root.width < 1) return undefined;
  return !entry.isIntersecting && entry.boundingClientRect.top < root.top;
}
