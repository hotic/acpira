import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { AnimateOptions } from 'streamdown';
import { useAppearance } from '../appearance';

// Reveal pacing: the visible text advances at the agent's measured throughput (characters per ms, a time-weighted
// EMA that counts stalls too), so a burst after a pause plays at the speed the stream has been running at instead
// of all at once. Lag beyond `lagMs` raises the target speed proportionally (never above `maxLagMs` of backlog),
// the actual speed eases toward the target over `easeMs`, and `finishMs` flushes the rest once the stream ends.
export interface StreamPace { tickMs: number; lagMs: number; maxLagMs: number; easeMs: number; windowMs: number; finishMs: number }
export interface StreamMotion { pace: StreamPace | false; animation: AnimateOptions }

// Per-glyph entrance on top of the pacing: the pacing already spreads the text over time, so the graphemes of
// one tick start together (no stagger). A stagger schedules glyphs ahead of the clock, and streamdown clamps
// each new render pass to `maxBacklogMs` ahead: a burst then let a later block start fading in while an earlier
// block was still waiting, i.e. text appearing out of order. The entrance is short to keep the leading edge crisp.
// The CSS side (`--stream-char-duration` in tokens.css, `sd-acpGlyph` in motion.css) owns the actual duration.
export const STREAM_STAGGER_MS = 0;
export const STREAM_BACKLOG_MS = 48;
export const STREAM_DURATION_MS = 180;
// Chosen in lab/stream-pace.preview.html: a 500 ms lag budget covers agents that push 30–60 characters about
// twice a second (Devin, Grok) without pauses; draining each chunk within the next arrival gap instead burst
// 8× the median speed right after a stall
export const STREAM_PACE: StreamPace = { tickMs: 32, lagMs: 500, maxLagMs: 1500, easeMs: 180, windowMs: 800, finishMs: 220 };
export const STREAM_MOTION: StreamMotion = {
  pace: STREAM_PACE,
  animation: { animation: 'acpGlyph', sep: 'char', duration: STREAM_DURATION_MS, stagger: STREAM_STAGGER_MS, maxBacklogMs: STREAM_BACKLOG_MS },
};

const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

// First grapheme boundary at or after `n`, so a cut never splits a surrogate pair, a ZWJ emoji or a combining mark.
function graphemeEnd(text: string, n: number) {
  if (n <= 0) return 0;
  if (n >= text.length) return text.length;
  const lo = Math.max(0, n - 16);
  for (const { index } of segmenter.segment(text.slice(lo, n + 16))) if (lo + index >= n) return lo + index;
  return Math.min(text.length, n + 16);
}

// Markdown punctuation and whitespace a visible prefix must not end on. streamdown's animate plugin remembers how
// many characters of each block it has already animated and renders that many as settled on the next pass; a
// transient literal ("- *", "**", "1") counts toward that, so the real text replacing it ("- **节奏**") showed
// up without an entrance while the text before it was still fading in
const SYNTAX_TAIL = /[\s*_`~#>|+=\-[\]()!\\]$/u;
const LIST_MARKER = /(^|\n)[ \t]*\d+[.)]?$/u;
const SYNTAX_REACH = 64;

// Tested on a short window so a long reply does not rescan from its start every tick; a window that does not
// reach a line start cannot prove an ordered-list marker
function endsOnSyntax(text: string, cut: number) {
  const lo = Math.max(0, cut - 24);
  const head = text.slice(lo, cut);
  if (SYNTAX_TAIL.test(head)) return true;
  const marker = LIST_MARKER.exec(head);
  return !!marker && (marker[1] === '\n' || lo === 0);
}

// Cut point at or after `n` that lands on a grapheme boundary and after a content character. While streaming,
// received text that itself ends on syntax is held back to its last content character until more arrives
function cutAt(text: string, n: number, streaming: boolean) {
  let cut = graphemeEnd(text, n);
  const limit = Math.min(text.length, cut + SYNTAX_REACH);
  while (cut > 0 && cut < limit && endsOnSyntax(text, cut)) cut = graphemeEnd(text, cut + 1);
  // Syntax characters are ASCII, so stepping back over them one code unit at a time stays on grapheme boundaries
  if (streaming && cut === text.length) while (cut > 0 && endsOnSyntax(text, cut)) cut--;
  return cut;
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

// The visible prefix of a streamed text. Text that is not an extension of the previous value (an edit, a
// replay) and motion-off show at once; `draining` stays true until the visible text catches up.
export function useSmoothText(text: string, streaming: boolean, pace: StreamPace | false = STREAM_PACE) {
  const { motion } = useAppearance();
  const off = !pace || motion === 'none';
  const [count, setCount] = useState(() => (streaming && !off ? 0 : text.length));
  // throughput: chars / ms of the incoming stream; speed: chars / ms actually revealed
  const state = useRef({ target: '', pos: count, at: 0, throughput: 0, speed: 0, streaming });
  useLayoutEffect(() => {
    const s = state.current;
    const appended = text.startsWith(s.target);
    const added = text.length - s.target.length;
    s.target = text;
    s.streaming = streaming;
    if (off || !appended) { s.pos = text.length; s.speed = 0; setCount(text.length); return; }
    if (added <= 0 || !pace) return;
    const now = performance.now();
    if (!s.at) {
      // First chunk: no rate yet, assume it covers one lag budget
      s.throughput = s.speed = added / pace.lagMs;
    } else {
      // Time-weighted EMA: a stall lowers the measured rate instead of making the next burst look fast
      const gap = clamp(now - s.at, 16, pace.maxLagMs);
      s.throughput += (added / gap - s.throughput) * (1 - Math.exp(-gap / pace.windowMs));
    }
    s.at = now;
  }, [text, streaming, off, pace]);
  const pending = !off && count < text.length;
  useEffect(() => {
    if (!pending || !pace) return;
    let last = performance.now();
    const timer = setInterval(() => {
      const s = state.current;
      const now = performance.now();
      const dt = now - last;
      last = now;
      const backlog = s.target.length - s.pos;
      // Proportional catch-up past the lag budget, a hard floor at maxLagMs, and a quick flush once the stream ends
      const lag = s.throughput > 0 ? backlog / s.throughput : pace.lagMs;
      let want = Math.max(s.throughput * (1 + Math.max(0, lag - pace.lagMs) / pace.lagMs), backlog / pace.maxLagMs);
      if (!s.streaming) want = Math.max(want, backlog / pace.finishMs);
      s.speed += (want - s.speed) * (1 - Math.exp(-dt / pace.easeMs));
      s.pos = Math.min(s.target.length, s.pos + s.speed * dt);
      const next = cutAt(s.target, Math.floor(s.pos), s.streaming);
      setCount(value => Math.max(value, next));
    }, pace.tickMs);
    return () => clearInterval(timer);
  }, [pending, pace]);
  return { text: off ? text : text.slice(0, Math.min(count, text.length)), draining: pending };
}

export function useStreamMotion(streaming: boolean, config: StreamMotion = STREAM_MOTION) {
  const { motion } = useAppearance();
  const [settling, setSettling] = useState(streaming);
  const { duration = STREAM_DURATION_MS, maxBacklogMs = STREAM_BACKLOG_MS } = config.animation;
  useEffect(() => {
    if (streaming || motion === 'none') { setSettling(streaming); return; }
    // Keep the renderer mounted until the final delayed character has settled.
    const timer = setTimeout(() => setSettling(false), maxBacklogMs + duration);
    return () => clearTimeout(timer);
  }, [streaming, motion, maxBacklogMs, duration]);
  return { animated: motion === 'none' ? false as const : config.animation, animating: streaming || settling };
}
