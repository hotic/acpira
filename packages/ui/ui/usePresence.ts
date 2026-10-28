import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { cssTimeVar } from './cssTime';

export interface Presence<T> {
  key: string;
  item: T;
  open: boolean;
}

interface Entry<T> {
  key: string;
  item: T;
  open: boolean;
  leaving?: boolean;
}

// Fallback for --dur-close, plus a frame or two for the fold to finish (as in ui/Collapsible)
const DEFAULT_CLOSE_MS = 320;
const RELEASE_SLACK_MS = 80;

/**
 * A keyed list whose items come and go through a fold (`Collapsible.Section open={entry.open}`), so the rows around
 * them slide instead of jumping: an item added later mounts closed and opens on the next frame; a removed one stays
 * where it was, closed, until the close transition has run. Items present on the first render show at once.
 */
export function usePresence<T>(items: readonly T[], keyOf: (item: T) => string): Presence<T>[] {
  const [entries, setEntries] = useState<Entry<T>[]>(() => items.map(item => ({ key: keyOf(item), item, open: true })));
  const latest = useRef(items);
  latest.current = items;
  const signature = items.map(keyOf).join('\0');
  // Merged before paint, so a new item's first frame is already closed and a removed one never blinks out
  useLayoutEffect(() => {
    setEntries(prev => merge(prev, latest.current, keyOf));
    // keyOf is read with the items; the signature is what changes the list
  }, [signature]);
  const entering = entries.some(e => !e.open && !e.leaving);
  useEffect(() => {
    if (!entering) return;
    // The closed state was painted in the last frame, so opening now runs the transition
    const frame = requestAnimationFrame(() => setEntries(prev => prev.map(e => (e.open || e.leaving ? e : { ...e, open: true }))));
    return () => cancelAnimationFrame(frame);
  }, [entering]);
  const leaving = entries.some(e => e.leaving);
  useEffect(() => {
    if (!leaving) return;
    const close = cssTimeVar(document.documentElement, '--dur-close', DEFAULT_CLOSE_MS);
    const timer = setTimeout(() => setEntries(prev => prev.filter(e => !e.leaving)), close + RELEASE_SLACK_MS);
    return () => clearTimeout(timer);
  }, [leaving, entries]);
  // A present item always renders its current value; only a leaving one keeps the value it left with
  const current = new Map(items.map(item => [keyOf(item), item]));
  return entries.map(e => ({ key: e.key, item: current.get(e.key) ?? e.item, open: e.open }));
}

// The new order, with each removed item kept right after the entry it followed
function merge<T>(prev: Entry<T>[], items: readonly T[], keyOf: (item: T) => string): Entry<T>[] {
  const was = new Map(prev.map(e => [e.key, e]));
  const next: Entry<T>[] = items.map(item => {
    const key = keyOf(item);
    const old = was.get(key);
    // An item coming back while it was leaving opens again from where its fold is
    return old && !old.leaving ? { ...old, item } : { key, item, open: false };
  });
  const present = new Set(next.map(e => e.key));
  let after = -1;
  for (const e of prev) {
    const at = next.findIndex(n => n.key === e.key);
    if (present.has(e.key)) { after = at; continue; }
    next.splice(after + 1, 0, { ...e, open: false, leaving: true });
    after += 1;
  }
  return next;
}
