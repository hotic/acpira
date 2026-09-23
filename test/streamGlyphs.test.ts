import { describe, expect, it } from 'vitest';
import { advanceGlyphs, EMPTY_GLYPHS, type GlyphState } from '../src/webview/chat/streamGlyphs';

const opts = { animate: true, stagger: 0, backlogMs: 48, durationMs: 100 };
const contentOf = (state: GlyphState) => state.text.slice(0, state.settled) + state.glyphs.map(glyph => glyph.value).join('');

describe('stream glyphs', () => {
  it('keeps appended chunks at contiguous UTF-16 offsets', () => {
    const first = advanceGlyphs(EMPTY_GLYPHS, 'ab', 0, opts);
    const second = advanceGlyphs(first, 'abcd', 20, opts);
    expect(second.settled).toBe(0);
    expect(second.glyphs.map(glyph => glyph.at)).toEqual([0, 1, 2, 3]);
  });

  it('moves finished chunks into the settled prefix', () => {
    const first = advanceGlyphs(EMPTY_GLYPHS, 'ab', 0, opts);
    const second = advanceGlyphs(first, 'abcd', 20, opts);
    const third = advanceGlyphs(second, 'abcdef', 101, opts);
    expect(third.settled).toBe(2);
    expect(third.glyphs[0]?.at).toBe(2);
    expect(contentOf(third)).toBe(third.text);
  });

  it('settles non-appended text without entrance glyphs', () => {
    const first = advanceGlyphs(EMPTY_GLYPHS, 'original', 0, opts);
    const edited = advanceGlyphs(first, 'edited', 10, opts);
    expect(edited.settled).toBe(edited.text.length);
    expect(edited.glyphs).toEqual([]);
  });

  it('keeps the fading tail when streaming ends', () => {
    const live = advanceGlyphs(EMPTY_GLYPHS, 'fade', 0, opts);
    const fading = advanceGlyphs(live, 'fade', 50, { ...opts, animate: false });
    expect(fading.glyphs).toHaveLength(4);
    expect(contentOf(fading)).toBe(fading.text);
    const settled = advanceGlyphs(fading, 'fade', 101, { ...opts, animate: false });
    expect(settled.settled).toBe(settled.text.length);
    expect(settled.glyphs).toEqual([]);
  });

  it('settles appended text when animation is disabled', () => {
    const live = advanceGlyphs(EMPTY_GLYPHS, 'plain', 0, opts);
    const state = advanceGlyphs(live, 'plain text', 50, { ...opts, animate: false });
    expect(state.settled).toBe(state.text.length);
    expect(state.glyphs).toEqual([]);
  });

  it('segments CJK and emoji ZWJ sequences as graphemes', () => {
    const state = advanceGlyphs(EMPTY_GLYPHS, '思考👩‍💻ok', 0, opts);
    expect(state.glyphs.map(glyph => glyph.value)).toEqual(['思', '考', '👩‍💻', 'o', 'k']);
    expect(state.glyphs.map(glyph => glyph.at)).toEqual([0, 1, 2, 7, 8]);
  });

  it('returns the same state for an unchanged unexpired stream', () => {
    const state = advanceGlyphs(EMPTY_GLYPHS, 'live', 0, opts);
    expect(advanceGlyphs(state, 'live', 50, opts)).toBe(state);
  });

  it('preserves the settled-prefix and live-tail invariant over time', () => {
    const chunks = ['思', '考', '👩‍💻', ' ', 'moves', '\n', '快'];
    let state = EMPTY_GLYPHS;
    let text = '';
    let now = 0;
    for (let index = 0; index < 28; index++) {
      now += (index % 4) * 17 + 1;
      if (index < chunks.length || index % 3 === 0) text += chunks[index % chunks.length];
      state = advanceGlyphs(state, text, now, { ...opts, stagger: 6, durationMs: 45 });
      expect(contentOf(state)).toBe(text);
    }
  });
});
