const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
const whitespace = /^\s+$/u;

export interface LiveGlyph {
  at: number;
  value: string;
  delay: number;
  end: number;
}

export interface GlyphState {
  text: string;
  settled: number;
  glyphs: LiveGlyph[];
  lastStart: number;
}

export const EMPTY_GLYPHS: GlyphState = { text: '', settled: 0, glyphs: [], lastStart: 0 };

export function advanceGlyphs(prev: GlyphState, text: string, now: number, opts: {
  animate: boolean;
  stagger: number;
  backlogMs: number;
  durationMs: number;
}): GlyphState {
  const extendsPrevious = text.startsWith(prev.text);
  if (!extendsPrevious || (!opts.animate && text !== prev.text)) {
    return { text, settled: text.length, glyphs: [], lastStart: prev.lastStart };
  }

  let expired = 0;
  while (expired < prev.glyphs.length && prev.glyphs[expired]!.end <= now) expired++;
  const glyphs = expired ? prev.glyphs.slice(expired) : prev.glyphs;
  const settled = glyphs[0]?.at ?? prev.text.length;
  const appended = text.slice(prev.text.length);
  if (!appended) {
    if (!expired) return prev;
    return { text, settled, glyphs, lastStart: prev.lastStart };
  }

  const additions = [...segmenter.segment(appended)];
  const count = additions.length;
  const base = Math.max(now, prev.lastStart);
  const stagger = Math.min(opts.stagger, Math.max(0, now + opts.backlogMs - base) / Math.max(1, count));
  const next = additions.map((part, index): LiveGlyph => {
    const delay = Math.max(0, base - now + index * stagger);
    const value = part.segment;
    return {
      at: prev.text.length + part.index,
      value,
      delay,
      end: whitespace.test(value) ? now : now + delay + opts.durationMs,
    };
  });
  return {
    text,
    settled,
    glyphs: glyphs.concat(next),
    lastStart: count ? base + (count - 1) * stagger : prev.lastStart,
  };
}
