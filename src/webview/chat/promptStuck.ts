// A hidden sidebar webview collapses the thread to no box. IntersectionObserver then
// reports every exchange sentinel as off-screen, and scroll listeners treat the empty
// viewport as "left the bottom". Callers must ignore those frames and re-check once
// the thread has a box again.

export function scrollerUsable(el: { clientHeight: number; clientWidth: number }): boolean {
  return el.clientHeight >= 1 && el.clientWidth >= 1;
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
