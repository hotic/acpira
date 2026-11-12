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
// `intent`: the reader moved the view (wheel, touch, keys or the pointer on the scroller just now). An upward scroll without
// it is the browser clamping the offset to content that shrank for a layout (a block remounting, a fold measuring): WebKit
// lays out and clamps between React's commits where Chromium did not, and treating that as "left the bottom" stopped the
// follow two seconds into a streamed reply in WKWebView (2026-10-25). Only the reader lets go of the bottom.
// `untouched`: no press or key has landed in the scroller since the last pin. Once the reader opened a fold by hand, a
// downward scroll short of the new bottom is their own reading, not a late scroll event from a pin.
export function followsBottom(el: { scrollHeight: number; scrollTop: number; clientHeight: number }, pinned: boolean, lastTop: number, intent = true, untouched = true): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= 1 || (pinned && (!intent || (untouched && el.scrollTop >= lastTop)));
}

// How long after the reader's last wheel, touch, key or pointer release a scroll still counts as theirs (trackpad
// momentum keeps sending wheel events, so this only has to bridge the gap to the scroll event)
export const SCROLL_INTENT_MS = 800;

function isElementNode(value: unknown): value is Element {
  return typeof Element !== 'undefined'
    ? value instanceof Element
    : !!value && typeof value === 'object' && (value as { nodeType?: number }).nodeType === 1;
}

// Pick a stable box for press anchoring instead of the raw event target:
// - sticky prompt cards stay pinned to the scroller's top edge on their own and must not drive scrollTop
// - hits inside a nested scrollport (thought body, command output, diff) anchor on the outer scrollport box so scrolling
//   inside it never looks like the conversation above shrank
// - hits inside an inline command toggle, interactive row/button (rotating carets, unmounting shimmers) or inline text
//   span climb to the owning header/row/block whose top edge stays still while a panel opens below it
export function pressAnchorNode(
  target: unknown,
  content: Element | null,
  overflowYOf: (el: Element) => string = el => getComputedStyle(el).overflowY,
): Element | undefined {
  if (!content || !isElementNode(target) || !content.contains(target)) return undefined;
  if (target.closest('[data-sticky-prompt]')) return undefined;
  let node: Element = target;
  for (let cur: Element | null = target; cur && cur !== content; cur = cur.parentElement) {
    const overflowY = overflowYOf(cur);
    if (overflowY === 'auto' || overflowY === 'scroll') node = cur;
  }
  const command = node.closest('.command-prompt');
  if (command && content.contains(command)) return command;
  let row: Element | undefined;
  for (let cur: Element | null = node; cur && cur !== content; cur = cur.parentElement) {
    if (cur.matches('button, a, [role="button"], .action-row')) row = cur;
  }
  if (row) return row;
  const block = node.closest('p, li, pre, blockquote');
  return block && content.contains(block) ? block : node;
}

// Offset of an anchor node from the scroller's top edge. Returns undefined once detached or hidden (`display: none` or
// `contents` keeps `isConnected` true while `getBoundingClientRect().top` collapses to 0, which would yank scrollTop up).
export function anchorOffset(
  node: { isConnected: boolean; getClientRects(): { length: number }; getBoundingClientRect(): { top: number } },
  scroller: { getBoundingClientRect(): { top: number } },
): number | undefined {
  if (!node.isConnected || node.getClientRects().length === 0) return undefined;
  return node.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
}
