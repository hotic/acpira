// Streamed-text entrance for a live Markdown block, as a rehype plugin in place of streamdown's animate plugin.
//
// streamdown's plugin wraps every character of an animated block in a span of its own, on every pass, for as long as the
// block is the streaming tail: a long paragraph or table in the tail was hundreds of spans re-created and re-matched
// (an attribute substring selector) on each 32 ms reveal tick, which WKWebView paid for in frame time (lab/tauri-shell
// bench, 2026-10-25). The pacing (streamMotion.ts) has no stagger, so every glyph of one pass starts its fade together:
// one span per pass and text node draws the same thing. Runs whose fade has finished turn back into plain text nodes in
// place, each at the index it always had, so React never moves a fading span onto another run's text (which would show
// as a jump in that run's opacity). Only the runs still fading are elements.
//
// Characters are counted as streamdown counts them (text, plus one for an image or rule), skipping code, SVG and math.

interface HastText { type: 'text'; value: string }
interface HastElement { type: 'element'; tagName: string; properties?: Record<string, unknown>; children: HastNode[] }
interface HastOther { type: 'root' | 'comment' | 'doctype' | 'raw'; children?: HastNode[] }
type HastNode = HastText | HastElement | HastOther;
interface HastRoot { type: 'root'; children: HastNode[] }

// One pass's characters [start, next run's start), first shown at `at`
interface Run { start: number; at: number }

export interface GlyphRunState {
  runs: Run[];
  // The block's character count after the last pass
  count: number;
  // The next pass shows at once (see seedGlyphRuns)
  resume: boolean;
}

const SKIP = new Set(['pre', 'svg', 'math', 'annotation']);
const COUNTED = new Set(['img', 'hr']);
const WHITESPACE = /^\s+$/u;

const isElement = (node: HastNode): node is HastElement => node.type === 'element';

export const GLYPH_CLASS = 'stream-glyph';

export function createGlyphRuns(): GlyphRunState {
  return { runs: [], count: 0, resume: false };
}

// What the block holds already shows (a session reopened mid-turn): the next pass is one finished run, and only text
// after it fades in
export function seedGlyphRuns(state: GlyphRunState) {
  state.resume = true;
}

function countOf(node: HastNode): number {
  if (node.type === 'text') return (node as HastText).value.length;
  if (!isElement(node)) return ((node as HastOther).children ?? []).reduce((n: number, c: HastNode) => n + countOf(c), 0);
  if (SKIP.has(node.tagName)) return 0;
  if (COUNTED.has(node.tagName)) return 1;
  return node.children.reduce((n, c) => n + countOf(c), 0);
}

// Never cut between the halves of a surrogate pair
function safeCut(text: string, at: number) {
  const code = text.charCodeAt(at - 1);
  return code >= 0xd800 && code <= 0xdbff ? at + 1 : at;
}

const fadeProps = (properties: Record<string, unknown> | undefined) => ({ ...properties, className: [...([] as unknown[]).concat(properties?.className ?? []), GLYPH_CLASS] });

// Rewrites `tree` for the pass at `now`: text split at run boundaries, the runs still fading (younger than `fadeMs`)
// wrapped in a glyph span, list items whose first character is fading get the marker fade
export function applyGlyphRuns(state: GlyphRunState, tree: HastRoot, now: number, fadeMs: number) {
  const total = countOf(tree);
  if (state.resume) {
    state.resume = false;
    state.runs = total > 0 ? [{ start: 0, at: -Infinity }] : [];
    state.count = total;
  }
  // Text that grew is this pass's run; text that shrank (held-back syntax, a re-parse) keeps the runs it had
  if (total > state.count) state.runs.push({ start: state.count, at: now });
  state.count = Math.max(total, state.count);
  const runs = state.runs;
  if (!runs.length) return;
  const fading = (i: number) => now - runs[i]!.at < fadeMs;
  // Index of the run holding character `offset`
  const runAt = (offset: number) => {
    let lo = 0, hi = runs.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (runs[mid]!.start <= offset) lo = mid; else hi = mid - 1;
    }
    return lo;
  };
  let offset = 0;
  const walk = (parent: { children: HastNode[] }) => {
    const out: HastNode[] = [];
    for (const child of parent.children) {
      if (child.type === 'text') {
        const value = (child as HastText).value;
        const start = offset;
        offset += value.length;
        if (WHITESPACE.test(value) || !value) { out.push(child); continue; }
        let i = runAt(start);
        let from = 0;
        while (from < value.length) {
          const next = runs[i + 1]?.start;
          const to = next === undefined || next - start >= value.length ? value.length : safeCut(value, Math.max(from + 1, next - start));
          const piece = value.slice(from, to);
          out.push(fading(i) && !WHITESPACE.test(piece)
            ? { type: 'element', tagName: 'span', properties: { className: [GLYPH_CLASS] }, children: [{ type: 'text', value: piece }] } as HastElement
            : { type: 'text', value: piece } as HastText);
          from = to;
          i++;
        }
        continue;
      }
      if (isElement(child)) {
        if (SKIP.has(child.tagName)) { out.push(child); continue; }
        if (COUNTED.has(child.tagName)) {
          const i = runAt(offset);
          offset += 1;
          out.push(fading(i) ? { ...child, properties: fadeProps(child.properties) } : child);
          continue;
        }
        if (child.tagName === 'li') {
          const i = runAt(offset);
          const marked = fading(i) ? { ...child, properties: { ...child.properties, 'data-glyph-marker': true } } : child;
          marked.children = walk(child);
          out.push(marked);
          continue;
        }
        child.children = walk(child);
        out.push(child);
        continue;
      }
      const other = child as HastOther;
      if (other.children) other.children = walk(other as { children: HastNode[] });
      out.push(child);
    }
    return out;
  };
  tree.children = walk(tree);
}
