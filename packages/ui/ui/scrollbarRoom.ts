// WebKit lays out an auto-height box inside a flex or grid container before it adds the box's horizontal scrollbar, and
// does not lay the ancestors out again: the box then overflows its parent by the bar's height until something re-lays
// that subtree out (a diff's colours landing, a hover). On a session switch every wide code card grew by the bar a beat
// after it showed, and a thread held to its bottom jumped by the sum. A box that overflows sideways gets `data-hbar`
// (`overflow-x: scroll` in base.css) before it paints: a bar present from the start is part of the first layout.
// Chromium sizes these boxes right, so it skips all of this.
const webkit = typeof CSS !== 'undefined' && CSS.supports('-webkit-nbsp-mode', 'space');

let sharedResize: ResizeObserver | undefined;

// Resize callbacks run after layout and before paint, so a flip here is laid out in the same frame. All reads, then
// all writes: the batch costs one layout. The border box is observed: an auto-height box gaining its bar grows by it
// while its content box keeps its size
function observer(): ResizeObserver {
  return sharedResize ??= new ResizeObserver(entries => {
    const flips = entries.map(({ target }) => {
      const el = target as HTMLElement;
      const wide = el.scrollWidth > el.clientWidth;
      return el.hasAttribute('data-hbar') === wide ? undefined : { el, wide };
    });
    for (const flip of flips) flip?.el.toggleAttribute('data-hbar', flip.wide);
  });
}

// Ref callback for a box that may scroll sideways (code, diffs, command output, tables): stable, so it can sit in JSX
export function scrollbarRoom(element: HTMLElement | null): (() => void) | undefined {
  if (!element || !webkit || typeof ResizeObserver === 'undefined') return undefined;
  observer().observe(element, { box: 'border-box' });
  return () => sharedResize?.unobserve(element);
}
