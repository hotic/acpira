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
