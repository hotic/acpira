import { afterEach, describe, expect, it, vi } from 'vitest';
import { captureSliceScroll, overriddenBy, readerScrolling, restoreSliceScroll } from '../chat/sliceScroll';

// Model the browser's rounded scrollHeight separately from its fractional layout and scroll offset.
function transcript(step: number, bottom: boolean, initialHeight = 1000.25) {
  let height = initialHeight;
  let row = initialHeight - 100;
  let top = 0;
  const clientHeight = 400;
  const style = { marginTop: '' };
  const margin = () => Number.parseFloat(style.marginTop) || 0;
  const content = { style, getBoundingClientRect: () => ({ height, bottom: height + margin() - top }) } as HTMLElement;
  const scroller = {
    firstElementChild: content,
    clientHeight,
    get scrollHeight() { return Math.max(clientHeight, Math.round(height + margin())); },
    // A zoomed WebKit getter rounds its CSS offset even when the physical scroll step is not one CSS pixel.
    get scrollTop() { return step ? Math.round(top) : top; },
    set scrollTop(value: number) {
      const snap = (n: number) => step ? Math.round(n / step) * step : n;
      const max = height + margin() - clientHeight;
      top = Math.max(0, Math.min(snap(max), Math.max(0, snap(value))));
    },
    getBoundingClientRect: () => ({ top: 0 }),
  } as unknown as HTMLElement;
  scroller.scrollTop = bottom ? scroller.scrollHeight : 300;
  return {
    scroller,
    content,
    rowTop: () => row + margin() - top,
    prepend: (added: number) => { height += added; row += added; },
  };
}

describe('history slice scroll anchoring', () => {
  for (const step of [1, 0, 1 / 1.1, 1 / 0.9]) {
    for (const bottom of [true, false]) {
      it(`keeps the visible row fixed through fractional prepends (scroll step ${step}, ${bottom ? 'following' : 'reading'})`, () => {
        const view = transcript(step, bottom);
        const rowTop = view.rowTop();
        // Mixed prose and tool rows produce alternating fractions, not just one final layout correction.
        for (const added of [120.5, 121.5, 88.25, 75.125, 100.75, 40, 96.875]) {
          const kept = captureSliceScroll(view.scroller)!;
          expect(kept.bottom).toBe(bottom);
          view.prepend(added);
          restoreSliceScroll(kept);
          expect(view.rowTop()).toBeCloseTo(rowTop, 8);
          const margin = Number.parseFloat(view.content.style.marginTop) || 0;
          expect(Math.abs(margin)).toBeLessThan(1 + step);
        }
      });
    }
  }

  it('pins a short transcript gaining overflow instead of preserving its old short edge', () => {
    const view = transcript(1, true, 250.25);
    const kept = captureSliceScroll(view.scroller)!;
    view.prepend(500.5);
    restoreSliceScroll(kept);
    expect(view.scroller.scrollHeight - view.scroller.scrollTop - view.scroller.clientHeight).toBe(0);
    expect(view.content.style.marginTop).toBe('');
  });

  it('pins a following view whose layout already left a gap', () => {
    const view = transcript(1, false);
    const kept = captureSliceScroll(view.scroller, true)!;
    view.prepend(120.5);
    restoreSliceScroll(kept);
    expect(view.scroller.scrollHeight - view.scroller.scrollTop - view.scroller.clientHeight).toBe(0);
    expect(view.content.style.marginTop).toBe('');
  });

  it('does not capture a scroller without transcript content', () => {
    expect(captureSliceScroll({ firstElementChild: null } as HTMLElement)).toBeUndefined();
  });
});

describe('history slices and the reader\'s own scroll', () => {
  afterEach(() => { vi.restoreAllMocks(); });

  function reader(scrollTop = 500) {
    const handlers = new Map<string, () => void>();
    const scroller = { scrollTop, addEventListener: (type: string, handler: () => void) => { handlers.set(type, handler); } } as unknown as HTMLElement;
    let now = 1000;
    vi.spyOn(performance, 'now').mockImplementation(() => now);
    return { scroller, fire: (type: string) => handlers.get(type)!(), wait: (ms: number) => { now += ms; } };
  }

  it('holds a slice from the wheel until its scroll events have stopped', () => {
    const view = reader();
    expect(readerScrolling(view.scroller)).toBe(false);
    view.fire('wheel');
    expect(readerScrolling(view.scroller)).toBe(true);
    // The animation the notch started keeps scrolling with no further input
    for (let i = 0; i < 20; i++) {
      view.wait(16);
      view.fire('scroll');
    }
    expect(readerScrolling(view.scroller)).toBe(true);
    view.wait(200);
    expect(readerScrolling(view.scroller)).toBe(false);
  });

  it('does not take the page\'s own scroll for the reader\'s', () => {
    const view = reader();
    readerScrolling(view.scroller);
    view.fire('scroll');
    expect(readerScrolling(view.scroller)).toBe(false);
  });

  it('lets a slice land for a reader stopped at the top of what is mounted', () => {
    const view = reader(0);
    readerScrolling(view.scroller);
    view.fire('wheel');
    expect(readerScrolling(view.scroller)).toBe(false);
  });

  it('tells a drop by a landed slice\'s height from the reader\'s scroll', () => {
    const heights = [3957, 4427, 6778];
    expect(overriddenBy(heights, 40)).toBe(0);
    expect(overriddenBy(heights, 3000)).toBe(0);
    // One slice, with a few pixels the reader scrolled in that frame; it is undone once
    expect(overriddenBy(heights, 4436)).toBe(4427);
    expect(heights).toEqual([3957, 6778]);
    expect(overriddenBy(heights, 4436)).toBe(0);
    // Two slices whose commits were overridden in the same frame
    expect(overriddenBy(heights, 3957 + 6778 + 3)).toBe(3957 + 6778);
    expect(heights).toEqual([]);
  });
});
