import { useCallback, useEffect, useRef, useState } from 'react';

// Long lists in the panel (a file's rows, a folder's files) are cut into blocks that get their rows only once they come
// within NEAR of the visible part of their scroller; until then a block is an empty box of its estimated height
// (panel.css). content-visibility spares the blocks out of sight their layout and paint, not their DOM and style: a 1 MB
// JSON is ~66k rows, and creating and styling every one of them froze the page for over a second in WebKit. A block that
// came near keeps its rows. WebKit and Chromium both take the percentage of the scroller's height (the spec's text says
// its width), so a narrow, tall panel reaches as far
const NEAR = '200% 0px';

// `root`: a selector for the scroller around the blocks (the observer's root, so NEAR reaches past its edges); the first
// `initial` blocks have their rows from the start
export function useNearBlocks(root: string, initial: number) {
  const [near, setNear] = useState<ReadonlySet<number>>(() => new Set(Array.from({ length: initial }, (_, i) => i)));
  const io = useRef<IntersectionObserver | null>(null);
  useEffect(() => () => io.current?.disconnect(), []);
  const add = useCallback((blocks: Iterable<number>) => setNear(cur => {
    let next: Set<number> | undefined;
    for (const b of blocks) if (b >= 0 && !cur.has(b)) (next ??= new Set(cur)).add(b);
    return next ?? cur;
  }), []);
  // The ref of a block still empty (it carries `data-block`): watched until it comes near
  const watch = useCallback((el: HTMLElement | null) => {
    if (!el) return;
    const scroller = el.closest<HTMLElement>(root);
    if (io.current && io.current.root !== scroller) {
      io.current.disconnect();
      io.current = null;
    }
    const observer = io.current ??= new IntersectionObserver(entries => {
      const came = entries.filter(e => e.isIntersecting);
      for (const e of came) observer.unobserve(e.target);
      if (came.length) add(came.map(e => Number((e.target as HTMLElement).dataset.block)));
    }, { root: scroller, rootMargin: NEAR });
    observer.observe(el);
    return () => observer.unobserve(el);
  }, [root, add]);
  return { near, watch, add };
}
