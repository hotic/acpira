import { Component, createRef, useEffect, useRef, useState, type ReactNode } from 'react';
import { cn } from '../ui/cn';
import { Collapsible } from '../ui/Collapsible';

// Rows move and step in through WAAPI, which the stylesheet's kill switches (motion=none, reduced motion) do not reach
export const still = (el: Element) => !!el.closest('[data-motion="none"]') || matchMedia('(prefers-reduced-motion: reduce)').matches;
const MOVE_EASE = 'cubic-bezier(.2,.7,.2,1)';
// The rows that move: sessions, project headers, section labels, the desktop's "more" row
const MOVING_ROWS = '[data-session], [data-project-row], [data-section-row], [data-more]';

// A fold whose rows step in one after another as it opens, while the panel grows on Collapsible's curve. It wraps the
// panel rather than sitting in it, so it stays mounted while a lazy panel releases its rows
function FoldRows({ open, children }: { open: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const was = useRef(open);
  useEffect(() => {
    const el = ref.current;
    if (open && !was.current && el && !still(el)) {
      el.querySelectorAll<HTMLElement>(MOVING_ROWS).forEach((row, i) => {
        if (row.getAnimations().some(a => a.id === 'flip')) return;
        row.animate([{ opacity: 0, translate: '0 -4px' }, { opacity: 1, translate: '0 0' }],
          { duration: 220, delay: Math.min(i, 10) * 18, easing: MOVE_EASE, fill: 'backwards' });
      });
    }
    was.current = open;
  }, [open]);
  return <div ref={ref} className="flex flex-col" data-fold data-open={open || undefined}>{children}</div>;
}

// A fold of the list: the panel on Collapsible's curve (released once closed, LazyPanelContext) with its rows stepping in
export function Fold({ open, className, children }: { open: boolean; className?: string; children: ReactNode }) {
  return <FoldRows open={open}><Collapsible.Section open={open}><div className={cn('flex flex-col', className)}>{children}</div></Collapsible.Section></FoldRows>;
}

// A long run of rows (a project listing all of its sessions, a flat view) mounts by chunk: the first LEAD_ROWS are plain
// siblings as before, the rest sit in chunks of CHUNK_ROWS that hold their rows only near the visible part of the list
// and are an empty box of the same height otherwise (a row is as tall as --row or, with a large font, its title's line;
// search rows with a snippet are taller, so a search lists plainly). With every row mounted, a 1,100-session project put ~24k nodes in the column: folding it
// took 300-450 ms in the desktop app's WKWebView and every fold or drop elsewhere re-laid them all on each frame
export const LEAD_ROWS = 40;
export const CHUNK_ROWS = 24;
// How far past the list's edges a chunk counts as near, in list heights
const CHUNK_NEAR = '150% 0px';

// `first`: the chunk right under the lead rows has its rows from the first paint (a fold opening shows them at once);
// `keep`: it holds a row that must not unmount (being dragged: its dragend would be lost; being renamed)
export function RowChunk({ count, first, keep, children }: { count: number; first: boolean; keep: boolean; children: () => ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(first || typeof IntersectionObserver === 'undefined');
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === 'undefined') return;
    const io = new IntersectionObserver(entries => {
      const seen = entries.at(-1)!.isIntersecting;
      // A row holding the focus stays: unmounting it would drop the focus to the page
      setNear(was => seen || (was && el.contains(document.activeElement)));
    }, { root: el.closest('[role="listbox"]'), rootMargin: CHUNK_NEAR });
    io.observe(el);
    return () => io.disconnect();
  }, []);
  const shown = near || keep;
  return <div ref={ref} data-chunk className="flex shrink-0 flex-col" style={shown ? undefined : { height: `calc(max(var(--row), var(--text-2-lh)) * ${count})` }}>{shown && children()}</div>;
}

// Rows that change place in a commit (a new session at the top of its project, one moved, pinned or unpinned, a project
// added) slide from where they were instead of jumping. Only a genuinely new row fades in: an existing row that changes
// group stays opaque, since fading what was already visible reads as the whole list blinking on an update. Positions
// are read right before React writes the DOM, so a slide still in flight restarts from where the row is on screen. Only a
// commit that changed `layout` measures: the shell re-renders the list on every stream chunk. Off while searching, where
// rows come and go with every key.
// After the commit the rows go through three passes that never interleave: end the slides in flight, read every row, start
// the new animations. Reading a row after an animation started (or a slide was cancelled) makes the browser restyle the
// document first, so cancelling, measuring and animating row by row cost a restyle per row: with ~500 rows listed that
// froze the column for most of a second on every new or deleted session (lab/session-list-perf). Only rows on screen
// before or after the commit animate; the others land where they belong out of sight
type FlipFirst = { tops: Map<string, number>; open: Set<Element> };
// Each row carries its own unique key: a project's full path, never its display name, which a local folder and one on an
// SSH host (or two local folders) can share; with a shared key one row slid from the other's place
const flipKey = (el: HTMLElement) => el.dataset.session
  ?? (el.dataset.projectRow !== undefined ? `p:${el.dataset.projectRow}`
    : el.dataset.sectionRow !== undefined ? `s:${el.dataset.sectionRow}`
    : `m:${el.dataset.more}`);
export class FlipRows extends Component<{ on: boolean; layout: string; children: ReactNode }> {
  box = createRef<HTMLDivElement>();
  getSnapshotBeforeUpdate(prev: Readonly<{ on: boolean; layout: string }>): FlipFirst | null {
    const box = this.box.current;
    if (!prev.on || !this.props.on || prev.layout === this.props.layout || !box || still(box)) return null;
    const tops: FlipFirst['tops'] = new Map();
    for (const el of box.querySelectorAll<HTMLElement>(MOVING_ROWS)) tops.set(flipKey(el), el.getBoundingClientRect().top);
    return { tops, open: new Set(box.querySelectorAll('[data-fold][data-open]')) };
  }
  componentDidUpdate(_prev: unknown, _state: unknown, first: FlipFirst | null) {
    const box = this.box.current;
    if (!first || !box) return;
    // One lookup for the whole list: el.getAnimations() per row restyles again after every cancel
    for (const a of box.getAnimations({ subtree: true })) if (a.id === 'flip') a.cancel();
    // The visible band: the list's scroller (the listbox around this box) within the window
    const clip = box.closest('[role="listbox"]')?.getBoundingClientRect();
    const [above, below] = [Math.max(0, clip?.top ?? 0), Math.min(innerHeight, clip?.bottom ?? innerHeight)];
    const seen = (top: number, height: number) => top + height > above && top < below;
    const slides: [HTMLElement, number][] = [];
    const entries: HTMLElement[] = [];
    for (const el of box.querySelectorAll<HTMLElement>(MOVING_ROWS)) {
      const { top, height } = el.getBoundingClientRect();
      const was = first.tops.get(flipKey(el));
      if (was !== undefined) {
        if (Math.abs(was - top) > 0.5 && (seen(top, height) || seen(was, height))) slides.push([el, was - top]);
        continue;
      }
      // The rows of a fold opening in this commit are FoldRows'
      const fold = el.closest('[data-fold]');
      if (fold && !first.open.has(fold)) continue;
      if (seen(top, height)) entries.push(el);
    }
    for (const [el, dy] of slides) el.animate([{ translate: `0 ${dy}px` }, { translate: '0 0' }], { id: 'flip', duration: 260, easing: MOVE_EASE });
    for (const el of entries) el.animate([{ opacity: 0, translate: '0 -4px' }, { opacity: 1, translate: '0 0' }], { id: 'flip', duration: 220, easing: MOVE_EASE });
  }
  render() { return <div ref={this.box} className="contents">{this.props.children}</div>; }
}
