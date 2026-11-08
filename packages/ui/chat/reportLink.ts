import { useEffect, useRef, type RefObject } from 'react';
import type { Turn } from '@acpira/shared/transcript';
import { getReportLink, subscribeReportLink } from '../context/link';

// The transcript's side of the context report's link (context/link.ts): the rows of the calls the report points at
// light up and everything else in the thread dims (chat.css `data-ctx-lit` / `data-ctx-dim`). Done on the DOM, not in
// render: a hover must not re-render the turns, and the memo chain stays as it is.
// A call's row carries its id on a `display: contents` wrapper (`data-calls`, ToolCall / ToolGroup). A call folded away
// (its row inert under a closed fold, or not mounted at all, as a finished turn's closed fold mounts nothing) lights the
// head row of the outermost closed fold instead, never the fold's whole box: a closed process fold still shows its
// steered prompts, and lighting it lit half the thread. A picked call in an exchange not mounted yet mounts it and is
// brought into view once its row is there. A hover whose rows are all out of view changes nothing: dimming the whole
// visible thread for a row nobody can see reads as a glitch; a pick scrolls there instead

// --dur-open: an element let go keeps its transition until the fade back is over
const FADE_MS = 220;

interface Options {
  session?: string;
  enabled: boolean;
  turns: Turn[];
  // The first turn of each exchange, in order, and the exchange window's mount
  starts: number[];
  showFrom: (exchange: number) => void;
  // Lets go of the bottom follow before a scroll
  release: () => void;
}

export function useReportLink(thread: RefObject<HTMLElement | null>, body: RefObject<HTMLElement | null>, options: Options): void {
  const latest = useRef(options);
  latest.current = options;
  const kick = useRef<() => void>(undefined);

  useEffect(() => {
    let lit = new Set<Element>(), dim = new Set<Element>();
    const fading = new Map<Element, ReturnType<typeof setTimeout>>();
    let revealed: number | undefined;
    let frame = 0;
    let watching: MutationObserver | undefined;
    let scroller: HTMLElement | undefined;

    const fadeBack = (el: Element) => {
      el.setAttribute('data-ctx-fade', '');
      clearTimeout(fading.get(el));
      fading.set(el, setTimeout(() => { el.removeAttribute('data-ctx-fade'); fading.delete(el); }, FADE_MS));
    };
    const mark = (next: Set<Element>, prev: Set<Element>, attr: string) => {
      for (const el of prev) if (!next.has(el)) { el.removeAttribute(attr); fadeBack(el); }
      for (const el of next) if (!prev.has(el)) el.setAttribute(attr, '');
    };

    const apply = () => {
      frame = 0;
      const root = body.current;
      const { session, enabled, turns, starts, showFrom, release } = latest.current;
      const link = getReportLink();
      const want = enabled && root && link && link.session === session ? link : undefined;
      const nextLit = new Set<Element>();
      let missing: number | undefined;
      if (want && root) {
        for (const id of want.calls) {
          const hit = rowOf(root, id, turns);
          if (hit instanceof Element) nextLit.add(hit);
          else if (hit !== undefined) missing ??= hit;
        }
      }
      // A row inside another lit one is already lit
      for (const el of nextLit) for (const other of nextLit) if (other !== el && other.contains(el)) nextLit.delete(el);
      // A hover lights nothing unless one of its rows is in view (a pick keeps them: it scrolls there below)
      const view = thread.current;
      if (want?.reveal === undefined && view && nextLit.size && ![...nextLit].some(el => inView(el, view))) nextLit.clear();
      const nextDim = new Set<Element>();
      if (root && nextLit.size) dimAround(root, nextLit, nextDim);
      mark(nextDim, dim, 'data-ctx-dim');
      mark(nextLit, lit, 'data-ctx-lit');
      lit = nextLit;
      dim = nextDim;

      // The picked call comes into view once; one in an exchange not mounted yet mounts it, and the next pass scrolls
      if (want?.reveal !== undefined && want.reveal !== revealed) {
        const first = [...nextLit].sort((a, b) => (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1))[0];
        if (first && view) {
          revealed = want.reveal;
          const r = first.getBoundingClientRect(), v = view.getBoundingClientRect();
          if (r.top < v.top || r.bottom > v.bottom || !inView(first, view)) {
            release();
            const smooth = !matchMedia('(prefers-reduced-motion: reduce)').matches;
            view.scrollBy({ top: r.top - v.top - v.height / 3, behavior: smooth ? 'smooth' : 'auto' });
          }
        } else if (missing !== undefined) {
          let k = -1;
          for (let i = 0; i < starts.length; i++) if (starts[i]! <= missing) k = i;
          showFrom(k);
        }
      }

      // While the report points somewhere, rows that mount or change shape are marked again, and a scroll that brings a
      // hovered row into view (or takes it out) lights or lets go of it
      if (want && root && !watching) {
        watching = new MutationObserver(schedule);
        watching.observe(root, { childList: true, subtree: true });
        scroller = view ?? undefined;
        scroller?.addEventListener('scroll', schedule, { passive: true });
      } else if (!want && watching) {
        watching.disconnect();
        watching = undefined;
        scroller?.removeEventListener('scroll', schedule);
        scroller = undefined;
      }
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(apply); };
    kick.current = schedule;
    const off = subscribeReportLink(schedule);
    schedule();
    return () => {
      off();
      cancelAnimationFrame(frame);
      watching?.disconnect();
      scroller?.removeEventListener('scroll', schedule);
      for (const el of dim) el.removeAttribute('data-ctx-dim');
      for (const el of lit) el.removeAttribute('data-ctx-lit');
      for (const [el, timer] of fading) { clearTimeout(timer); el.removeAttribute('data-ctx-fade'); }
    };
  }, [thread, body]);

  // The setting switched, or the session changed under a kept link
  useEffect(() => { kick.current?.(); }, [options.enabled, options.session]);
}

// The element to light for a call: its row, the nearest one holding it that is not inside a closed fold, else the head
// of the outermost closed fold around it, else its turn's process fold head (or its reply when the fold has none).
// A turn not mounted gives its index; a call the transcript does not hold gives nothing
function rowOf(root: Element, id: string, turns: Turn[]): Element | number | undefined {
  const holders = root.querySelectorAll(`[data-calls~="${CSS.escape(id)}"]`);
  // Ancestors come first in document order: the deepest open one wins
  for (let i = holders.length - 1; i >= 0; i--) {
    const el = holders[i]!;
    if (!el.closest('[inert]')) return el.firstElementChild ?? el;
  }
  const folded = holders.length ? closedHead(root, holders[holders.length - 1]!) : undefined;
  if (folded) return folded;
  let turn = -1;
  for (let i = turns.length - 1; i >= 0 && turn < 0; i--) {
    const t = turns[i]!;
    if (t.role === 'agent' && t.blocks.some(b => b.type === 'tool_call' && b.id === id)) turn = i;
  }
  if (turn < 0) return undefined;
  const reply = root.querySelector(`[data-part="agent-message"][data-turn="${turn}"]`);
  if (!reply) return turn;
  const fold = reply.querySelector('[data-process-fold]');
  return (fold && openTrigger(fold)) ?? reply;
}

// The head row of the outermost closed fold holding `el`: the fold's trigger (Base UI's Collapsible.Trigger carries
// `aria-expanded`), found in the nearest box above the closed panel that holds one outside any closed fold
function closedHead(root: Element, el: Element): Element | undefined {
  let panel = el.closest('[inert]');
  if (!panel || !root.contains(panel)) return undefined;
  for (let up = panel.parentElement?.closest('[inert]'); up && root.contains(up); up = up.parentElement?.closest('[inert]')) panel = up;
  for (let box = panel.parentElement; box && box !== root; box = box.parentElement) {
    const head = openTrigger(box);
    if (head) return head;
  }
  return undefined;
}

function openTrigger(box: Element): Element | undefined {
  for (const t of box.querySelectorAll('[aria-expanded]')) if (!t.closest('[inert]')) return t;
  return undefined;
}

// Whether `el` shows inside the thread's viewport: on screen, and not under a pinned prompt (sticky prompts cover the
// rows that scroll beneath them), judged at the middle of its visible part
function inView(el: Element, view: Element): boolean {
  const r = el.getBoundingClientRect(), v = view.getBoundingClientRect();
  const top = Math.max(r.top, v.top), bottom = Math.min(r.bottom, v.bottom);
  if (bottom <= top || r.width <= 0) return false;
  const hit = document.elementFromPoint(r.left + r.width / 2, (top + bottom) / 2);
  return !hit || el.contains(hit) || hit.contains(el);
}

// Everything beside the path from each lit element up to the thread's body; a `display: contents` box has no opacity
// of its own, so its children take the mark. A pinned prompt keeps its opaque backing (HistoryMessage's `bg-bg-0`) and
// only its card fades: dimmed whole, it turned see-through and the rows scrolling beneath showed through it
function dimAround(root: Element, lit: Set<Element>, out: Set<Element>) {
  const path = new Set<Element>();
  for (const el of lit) for (let e: Element | null = el; e && e !== root; e = e.parentElement) path.add(e);
  const take = (el: Element) => {
    const backing = el.hasAttribute('data-sticky-prompt') ? el.querySelector(':scope > [data-part="user-message"]') : null;
    if (backing) for (const c of backing.children) out.add(c);
    else if (getComputedStyle(el).display === 'contents') for (const c of el.children) take(c);
    else out.add(el);
  };
  for (const el of path) {
    for (const sibling of el.parentElement?.children ?? []) if (!path.has(sibling)) take(sibling);
  }
}
