import { describe, expect, it } from 'vitest';
import { followsBottom, scrollerUsable } from '../src/webview/chat/promptStuck';

describe('followsBottom', () => {
  it('keeps following when a late scroll event reads a gap left by a second viewport shrink', () => {
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1100, clientHeight: 840 }, true, 1040)).toBe(true);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1100, clientHeight: 840 }, true, 1100)).toBe(true);
  });

  it('releases only on an upward scroll away from the bottom, and re-arms at the bottom', () => {
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 900, clientHeight: 900 }, true, 1100)).toBe(false);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1080, clientHeight: 900 }, true, 1100)).toBe(false);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1000, clientHeight: 900 }, false, 900)).toBe(false);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1100, clientHeight: 900 }, false, 900)).toBe(true);
  });

  it('lets small trackpad steps escape streaming and does not re-arm on their trailing scroll events', () => {
    const el = { scrollHeight: 2000, scrollTop: 1080, clientHeight: 900 };
    const pinned = followsBottom(el, true, 1100);
    expect(pinned).toBe(false);
    expect(followsBottom(el, pinned, el.scrollTop)).toBe(false);
    // New content must not take back a viewport the user just moved.
    expect(followsBottom({ ...el, scrollHeight: 2034 }, pinned, el.scrollTop)).toBe(false);
    expect(followsBottom({ ...el, scrollTop: 1090 }, false, 1080)).toBe(false);
  });

  it('tolerates fractional bottom offsets and a layout shrink that clamps to the bottom', () => {
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1099.5, clientHeight: 900 }, false, 1090)).toBe(true);
    expect(followsBottom({ scrollHeight: 1800, scrollTop: 900, clientHeight: 900 }, true, 1100)).toBe(true);
  });
});

describe('scrollerUsable', () => {
  it('rejects a collapsed box and accepts a real thread', () => {
    expect(scrollerUsable({ clientHeight: 0, clientWidth: 320 })).toBe(false);
    expect(scrollerUsable({ clientHeight: 400, clientWidth: 0 })).toBe(false);
    expect(scrollerUsable({ clientHeight: 400, clientWidth: 320 })).toBe(true);
  });
});
