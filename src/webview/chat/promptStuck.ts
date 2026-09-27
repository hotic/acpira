// A hidden sidebar webview collapses the thread to no box. Scroll listeners must
// ignore the empty viewport and re-check once the thread has a box again.

export function scrollerUsable(el: { clientHeight: number; clientWidth: number }): boolean {
  return el.clientHeight >= 1 && el.clientWidth >= 1;
}

// Bottom-follow verdict for a scroll event. Only an upward scroll can release the follow: scroll events are dispatched a frame
// late, so the one caused by a pin can land after the viewport shrank again (connecting notice, then composer chips) and read a
// gap the user never made — treating that as "left the bottom" skipped every later pin and left the tail under the composer.
// Only re-arm at the actual bottom (allowing subpixel rounding). A near-bottom zone swallows small wheel/trackpad steps:
// every streamed update pins again before those steps can accumulate enough distance to release the follow.
export function followsBottom(el: { scrollHeight: number; scrollTop: number; clientHeight: number }, pinned: boolean, lastTop: number): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= 1 || (pinned && el.scrollTop >= lastTop);
}
