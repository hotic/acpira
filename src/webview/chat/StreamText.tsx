import { memo, useCallback, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { useAppearance } from '../appearance';
import { advanceGlyphs, EMPTY_GLYPHS } from './streamGlyphs';
import { STREAM_BACKLOG_MS, STREAM_DURATION_MS, STREAM_STAGGER_MS } from './streamMotion';

const whitespace = /^\s+$/u;

// Thought text is plain text on the wire. Preserve its literal Markdown, emoji
// and line breaks while sharing the prose animation instead of parsing it anew.
export function StreamText({ text, streaming }: { text: string; streaming?: boolean }) {
  const [animateRun, setAnimateRun] = useState(!!streaming);
  const settle = useCallback(() => setAnimateRun(false), []);
  return animateRun ? <LiveStreamText text={text} streaming={streaming} onSettled={settle} /> : text;
}

function LiveStreamText({ text, streaming, onSettled }: { text: string; streaming?: boolean; onSettled: () => void }) {
  const { motion } = useAppearance();
  const ref = useRef<HTMLSpanElement>(null);
  const state = useRef(EMPTY_GLYPHS);
  const next = advanceGlyphs(state.current, text, performance.now(), {
    animate: !!streaming,
    stagger: STREAM_STAGGER_MS,
    backlogMs: STREAM_BACKLOG_MS,
    durationMs: STREAM_DURATION_MS,
  });
  useLayoutEffect(() => { state.current = next; });
  useLayoutEffect(() => {
    if (streaming) return;
    // Release the glyph tree once the final fading glyphs finish, including cancellation.
    const pending = ref.current?.getAnimations({ subtree: true }).filter(animation => animation.playState !== 'finished') ?? [];
    if (!pending.length) { onSettled(); return; }
    let disposed = false;
    void Promise.allSettled(pending.map(animation => animation.finished)).then(() => {
      if (!disposed) onSettled();
    });
    return () => { disposed = true; };
  }, [streaming, text, motion, onSettled]);
  // Motion-off retains the live cursor state without creating per-glyph DOM.
  if (motion === 'none') return text;
  return <span ref={ref} className="stream-text">{text.slice(0, next.settled)}{next.glyphs.map(glyph => whitespace.test(glyph.value) ? glyph.value
    : <Glyph key={glyph.at} value={glyph.value} delay={glyph.delay} />)}</span>;
}

const Glyph = memo(function Glyph({ value, delay }: { value: string; delay: number }) {
  // A glyph owns its birth animation; later chunks cannot restart or cancel it.
  const [birth] = useState({ delay });
  return <span className="stream-glyph" style={{ '--glyph-delay': `${birth.delay}ms` } as CSSProperties}>{value}</span>;
}, (previous, next) => previous.value === next.value);
