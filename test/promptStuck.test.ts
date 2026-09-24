import { describe, expect, it } from 'vitest';
import { followsBottom, promptIsStuck, promptIsStuckAt, scrollerUsable } from '../src/webview/chat/promptStuck';

const entry = (p: { intersecting: boolean; top: number; rootTop: number; rootHeight: number; rootWidth?: number; root?: null }) => ({
  isIntersecting: p.intersecting,
  boundingClientRect: { top: p.top },
  rootBounds: p.root === null ? null : { top: p.rootTop, height: p.rootHeight, width: p.rootWidth ?? 320 },
});

describe('promptIsStuck', () => {
  it('ignores a collapsed thread so a hidden sidebar does not fold visible prompts', () => {
    expect(promptIsStuck(entry({ intersecting: false, top: -80, rootTop: 0, rootHeight: 0 }))).toBeUndefined();
    expect(promptIsStuck(entry({ intersecting: false, top: 0, rootTop: 0, rootHeight: 400, rootWidth: 0 }))).toBeUndefined();
    expect(promptIsStuck(entry({ intersecting: false, top: -80, rootTop: 0, rootHeight: 0, root: null }))).toBeUndefined();
  });

  it('is stuck when the exchange top has scrolled above the thread', () => {
    expect(promptIsStuck(entry({ intersecting: false, top: -8, rootTop: 0, rootHeight: 400 }))).toBe(true);
  });

  it('is not stuck while the exchange top is in view, or still below it', () => {
    expect(promptIsStuck(entry({ intersecting: true, top: 12, rootTop: 0, rootHeight: 400 }))).toBe(false);
    expect(promptIsStuck(entry({ intersecting: false, top: 480, rootTop: 0, rootHeight: 400 }))).toBe(false);
  });
});

describe('promptIsStuckAt', () => {
  it('reads the same verdict from geometry, for the synchronous check before the first paint', () => {
    const root = { top: 100, height: 400, width: 320 };
    expect(promptIsStuckAt({ bottom: 92 }, root)).toBe(true);
    expect(promptIsStuckAt({ bottom: 100 }, root)).toBe(true);
    expect(promptIsStuckAt({ bottom: 101 }, root)).toBe(false);
    expect(promptIsStuckAt({ bottom: 520 }, root)).toBe(false);
    expect(promptIsStuckAt({ bottom: 0 }, { top: 0, height: 0, width: 320 })).toBeUndefined();
  });
});

describe('followsBottom', () => {
  it('keeps following when a late scroll event reads a gap left by a second viewport shrink', () => {
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1100, clientHeight: 840 }, true, 1040)).toBe(true);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1100, clientHeight: 840 }, true, 1100)).toBe(true);
  });

  it('releases only on an upward scroll away from the bottom, and re-arms at the bottom', () => {
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 900, clientHeight: 900 }, true, 1100)).toBe(false);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1080, clientHeight: 900 }, true, 1100)).toBe(true);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1000, clientHeight: 900 }, false, 900)).toBe(false);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1100, clientHeight: 900 }, false, 900)).toBe(true);
  });
});

describe('scrollerUsable', () => {
  it('rejects a collapsed box and accepts a real thread', () => {
    expect(scrollerUsable({ clientHeight: 0, clientWidth: 320 })).toBe(false);
    expect(scrollerUsable({ clientHeight: 400, clientWidth: 0 })).toBe(false);
    expect(scrollerUsable({ clientHeight: 400, clientWidth: 320 })).toBe(true);
  });
});
