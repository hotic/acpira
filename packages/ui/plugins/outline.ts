import { useEffect, useRef, type RefObject } from 'react';
import type { Turn } from '@acpira/shared/transcript';
import { publishSessionOutline, setScrollToExchange } from '../pluginSlots';

// What a plugin frame hears of the conversation on screen (acpira.onSession in /plugin-frame.html, docs/dev/plugins.md
// Interface): its shape per exchange, never its text, file names or commands, so a minimap or a progress strip can be
// drawn without the plugin reading the transcript. An exchange is a typed prompt and everything up to the next one
export interface ExchangeOutline {
  // The transcript index of the exchange's first turn: what acpira.scrollTo takes
  turn: number;
  edits: { files: number; add: number; del: number };
  runs: number;
  reads: number;
  tools: number;
  failed: number;
}

export interface SessionOutline {
  session: string;
  running: boolean;
  exchanges: ExchangeOutline[];
  // The exchanges at least partly in view, as transcript indices of their first turns
  seen?: [number, number];
}

// One pass over the turns; `starts` are the exchanges' first transcript indices, in order
export function outlineOf(turns: readonly Turn[], starts: readonly number[]): ExchangeOutline[] {
  const out: ExchangeOutline[] = [];
  for (let e = 0; e < starts.length; e++) {
    const from = starts[e]!;
    const to = starts[e + 1] ?? turns.length;
    const files = new Set<string>();
    const x: ExchangeOutline = { turn: from, edits: { files: 0, add: 0, del: 0 }, runs: 0, reads: 0, tools: 0, failed: 0 };
    for (let ti = from; ti < to; ti++) {
      const turn = turns[ti];
      if (turn?.role !== 'agent') continue;
      for (const b of turn.blocks) {
        if (b.type !== 'tool_call') continue;
        x.tools++;
        if (b.status === 'failed') x.failed++;
        if (b.kind === 'execute') x.runs++;
        else if (b.kind === 'read' || b.kind === 'search') x.reads++;
        if (b.diffStat) {
          x.edits.add += b.diffStat.add;
          x.edits.del += b.diffStat.del;
          // Only counted, never sent: the path names a file of the user's
          files.add(b.locations?.[0]?.path ?? b.id);
        }
      }
    }
    x.edits.files = files.size;
    out.push(x);
  }
  return out;
}

// How often the outline is rebuilt while a reply streams; the frame posts only what changed
const OUTLINE_EVERY_MS = 250;
// Frames to wait for an exchange the thread was just asked to mount (`mount`): a render and a layout, with room to spare
const SCROLL_WAIT_FRAMES = 30;

// The thread's side: builds the outline (at most every OUTLINE_EVERY_MS; a piece may arrive any time), tracks which exchanges are in view,
// and serves acpira.scrollTo. Exchanges carry `data-exchange-start` with their first transcript index. `mount` makes the
// thread mount an exchange it has not reached yet (a long session mounts earlier ones progressively); `release` stops the
// bottom follow first, which would otherwise take the view straight back
export function useSessionOutline(scroller: RefObject<HTMLElement | null>, session: string | undefined, turns: readonly Turn[], starts: readonly number[], running: boolean,
  nav: { mount: (turn: number) => void; release: () => void }) {
  const live = useRef({ turns, starts, running, session, nav });
  live.current = { turns, starts, running, session, nav };
  const seen = useRef<[number, number] | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const publish = useRef(() => {
    timer.current = undefined;
    const { turns, starts, running, session } = live.current;
    if (!session) return;
    publishSessionOutline({ session, running, exchanges: outlineOf(turns, starts), ...(seen.current ? { seen: seen.current } : {}) });
  });
  const schedule = () => { if (timer.current === undefined) timer.current = setTimeout(publish.current, OUTLINE_EVERY_MS); };
  const startsKey = starts.join(',');
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(schedule, [turns, running, startsKey, session]);
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    let frame = 0;
    const measure = () => {
      frame = 0;
      const box = el.getBoundingClientRect();
      let first: number | undefined;
      let last: number | undefined;
      for (const s of el.querySelectorAll<HTMLElement>('[data-exchange-start]')) {
        const r = s.getBoundingClientRect();
        if (r.bottom <= box.top || r.top >= box.bottom) { if (first !== undefined) break; continue; }
        const n = Number(s.dataset.exchangeStart);
        first ??= n;
        last = n;
      }
      const next: [number, number] | undefined = first !== undefined && last !== undefined ? [first, last] : undefined;
      if (next?.[0] === seen.current?.[0] && next?.[1] === seen.current?.[1]) return;
      seen.current = next;
      schedule();
    };
    const onScroll = () => { if (!frame) frame = requestAnimationFrame(measure); };
    el.addEventListener('scroll', onScroll, { passive: true });
    const sized = new ResizeObserver(onScroll);
    sized.observe(el);
    const go = (turn: number) => {
      if (!live.current.starts.includes(turn)) return;
      live.current.nav.release();
      live.current.nav.mount(turn);
      let tries = 0;
      const find = () => {
        const target = el.querySelector<HTMLElement>(`[data-exchange-start="${turn}"]`);
        if (target) target.scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
        else if (++tries < SCROLL_WAIT_FRAMES) requestAnimationFrame(find);
      };
      find();
    };
    setScrollToExchange(go);
    return () => {
      el.removeEventListener('scroll', onScroll);
      sized.disconnect();
      cancelAnimationFrame(frame);
      clearTimeout(timer.current);
      timer.current = undefined;
      setScrollToExchange(undefined, go);
      publishSessionOutline(undefined);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scroller]);
}
