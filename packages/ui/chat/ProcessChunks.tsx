import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { QuietEntranceContext } from '../ui/Row';
import { AutoFoldContext } from './autoFold';
import { chunkItems, itemCalls, itemExact, itemRows, type ItemChunk, type ProcessItem } from './processGroups';
import { readerScrolling } from './sliceScroll';
import { scrollerOf } from './tailWindow';
import { FollowContext } from './useBottomFollow';

// A turn can run for hours and leave hundreds of steps in one process (700 steps were 21k nodes: over two seconds of
// mounting when its fold opened, and a restyle of them all whenever the fold's `inert` flipped). Its items are cut into
// chunks, and a chunk gets its rows only once it comes within NEAR of the visible part of its scroller; until then it
// is an empty box of the height its rows will take (`itemRows`). A chunk that came near keeps its rows: what the reader
// opened or selected in them stays.
// A chunk whose height cannot be known (narrative text, a card) gets its rows ahead of the reader instead, one chunk at a
// time while the page is idle and the reader is not scrolling: rows that land taller than their box above the reader
// move the scroll offset, and no engine keeps a scroll in flight through that write (sliceScroll.ts)
const CHUNK_ROWS = 24;
const NEAR = '100% 0px';
// An idle callback waits for a quiet main thread; a streaming turn rarely has one
const AHEAD_TIMEOUT_MS = 250;
// The shortest a process row gets, to fill a tall window from the first commit
const MIN_ROW_PX = 20;

interface Near { near: ReadonlySet<string>; known: ReadonlySet<string> }

const windowed = () => typeof IntersectionObserver !== 'undefined' && typeof window !== 'undefined';

// The chunks with rows from the first commit: a window's worth from the start (a fold opens downward from its head)
// and, with `tail`, from the end as well (a working turn, or a fold that mounts open, is read from its end)
function firstNear(chunks: ItemChunk[], tail: boolean): Near {
  const known = new Set(chunks.map(chunk => chunk.id));
  if (!windowed()) return { near: known, known };
  const want = Math.ceil(window.innerHeight / MIN_ROW_PX);
  const near = new Set<string>();
  const take = (from: ItemChunk[]) => {
    let rows = 0;
    for (const chunk of from) {
      near.add(chunk.id);
      rows += chunk.items.reduce((n, item) => n + itemRows(item, false), 0);
      if (rows >= want) break;
    }
  };
  take(chunks);
  if (tail) take([...chunks].reverse());
  return { near, known };
}

// The far chunk of unknown height nearest the chunks that have their rows
function nextAhead(chunks: ItemChunk[], near: ReadonlySet<string>): string | undefined {
  const reach = chunks.map(() => Infinity);
  for (let i = 0, d = Infinity; i < chunks.length; i++) { d = near.has(chunks[i]!.id) ? 0 : d + 1; reach[i] = d; }
  for (let i = chunks.length - 1, d = Infinity; i >= 0; i--) { d = near.has(chunks[i]!.id) ? 0 : d + 1; reach[i] = Math.min(reach[i]!, d); }
  let pick: number | undefined;
  chunks.forEach((chunk, i) => {
    if (reach[i]! > 0 && (pick === undefined || reach[i]! < reach[pick]!) && !chunk.items.every(itemExact)) pick = i;
  });
  return pick === undefined ? undefined : chunks[pick]!.id;
}

// `children` renders a run of items (their rows as direct children, as `.process-content` expects). `open`: the fold
// around them shows them; a closed one that stays mounted (a working turn's) gains no rows ahead
export function ProcessChunks({ items, open, tail, children }: { items: ProcessItem[]; open: boolean; tail: boolean; children: (items: ProcessItem[]) => ReactNode }) {
  const chunks = useMemo(() => chunkItems(items, CHUNK_ROWS), [items]);
  const folds = useContext(AutoFoldContext);
  const follow = useContext(FollowContext);
  const [state, setState] = useState(() => firstNear(chunks, tail));
  // A chunk the process grew by has its rows when the one before it does: the reader is at that end
  if (chunks.some(chunk => !state.known.has(chunk.id))) {
    const near = new Set(state.near);
    const known = new Set(state.known);
    chunks.forEach((chunk, at) => {
      if (known.has(chunk.id)) return;
      known.add(chunk.id);
      if (!windowed() || at === 0 || near.has(chunks[at - 1]!.id)) near.add(chunk.id);
    });
    setState({ near, known });
  }
  // Chunks drawn empty so far: their rows were part of the process before the reader saw them
  const empty = useRef(new Set<string>());
  const observer = useRef<IntersectionObserver | null>(null);
  const scroller = useRef<HTMLElement | undefined>(undefined);
  useEffect(() => () => observer.current?.disconnect(), []);
  // The ref of an empty chunk: watched until it comes near
  const watch = useCallback((el: HTMLElement | null) => {
    if (!el) return;
    scroller.current ??= scrollerOf(el);
    // Listening from here on, so a wheel turned before the first chunk mounts ahead holds it too
    if (scroller.current) readerScrolling(scroller.current);
    const io = observer.current ??= new IntersectionObserver(entries => {
      const came = entries.filter(entry => entry.isIntersecting).map(entry => (entry.target as HTMLElement).dataset.chunk!);
      if (!came.length) return;
      // Rows that turn out taller or shorter than their box above a reader must not move what is being read
      follow?.rewrap();
      setState(current => ({ ...current, near: new Set([...current.near, ...came]) }));
    }, { root: scroller.current ?? null, rootMargin: NEAR });
    io.observe(el);
    return () => io.unobserve(el);
  }, [follow]);
  const ahead = open && windowed() ? nextAhead(chunks, state.near) : undefined;
  useEffect(() => {
    if (ahead === undefined) return;
    const idle = typeof requestIdleCallback === 'function';
    let pending = 0;
    const wait = () => { pending = idle ? requestIdleCallback(mount, { timeout: AHEAD_TIMEOUT_MS }) : window.setTimeout(mount, AHEAD_TIMEOUT_MS); };
    const mount = () => {
      if (scroller.current && readerScrolling(scroller.current)) return wait();
      follow?.rewrap();
      setState(current => ({ ...current, near: new Set(current.near).add(ahead) }));
    };
    wait();
    return () => { if (idle) cancelIdleCallback(pending); else clearTimeout(pending); };
  }, [ahead, follow]);
  return chunks.map(chunk => {
    if (state.near.has(chunk.id)) return <ChunkRows key={chunk.id} chunk={chunk} late={empty.current.has(chunk.id)}>{children(chunk.items)}</ChunkRows>;
    empty.current.add(chunk.id);
    const rows = chunk.items.reduce((n, item) => n + itemRows(item, folds?.get(item.id) ?? false), 0);
    // Its calls ride on the box, so the context report finds a row that is not mounted (chat/reportLink.ts)
    return <div key={chunk.id} ref={watch} data-chunk={chunk.id} data-calls={chunk.items.flatMap(itemCalls).join(' ') || undefined} className="shrink-0"
      style={{ height: `calc(${rows} * max(var(--row), var(--text-2-lh)) + ${chunk.items.length - 1} * var(--process-row-gap)${chunk.afterText ? ' + var(--process-prose-gap) - var(--process-row-gap)' : ''})` }} />;
  });
}

// The rows of a chunk, spaced like the list around them. `late`: mounted after their box was drawn empty, so they are
// restored history and skip their entrance (QuietEntranceContext)
function ChunkRows({ chunk, late, children }: { chunk: ItemChunk; late: boolean; children: ReactNode }) {
  const outer = useContext(QuietEntranceContext);
  const mounting = useRef(late);
  const [quiet] = useState(() => ({ get current() { return mounting.current || outer.current; } }));
  useLayoutEffect(() => { mounting.current = false; }, []);
  return (
    <div data-chunk={chunk.id} data-after-text={chunk.afterText || undefined} className="process-content flex min-w-0 flex-col gap-(--process-row-gap)">
      <QuietEntranceContext.Provider value={quiet}>{children}</QuietEntranceContext.Provider>
    </div>
  );
}
