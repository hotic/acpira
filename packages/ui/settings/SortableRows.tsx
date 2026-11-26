import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { cn } from '../ui/cn';
import { rowRules } from './controls';

// Past this many pixels a press on a row becomes a drag; below it the press stays a click
const DRAG_SLOP = 4;
// Row slide curve, shared with the agent tiles (PluginsPage.tsx) and the session list (SessionList.tsx)
const SLIDE_EASE = 'cubic-bezier(.2,.7,.2,1)';
const SLIDE_MS = 240;
// Auto-scroll zone at the top / bottom of the scroller while holding a row, and its peak speed (px/s)
const SCROLL_EDGE = 44;
const SCROLL_SPEED = 520;
// Pressing on a row's own controls stays theirs and never starts a drag
const NO_DRAG = '[data-no-drag], button:not([data-row-key]), a, input, textarea, select, label, [role="switch"], [role="checkbox"], [role="menu"]';
const SLIDE_ANIM_ID = 'row-slide';

// Move one item id to another index of the list (no-op when absent or already there)
export function moveById<T extends { id: string }>(items: readonly T[], id: string, to: number): T[] {
  const from = items.findIndex(x => x.id === id);
  if (from < 0) return items as T[];
  const clamped = Math.max(0, Math.min(items.length - 1, to));
  if (from === clamped) return items as T[];
  const next = [...items];
  const [picked] = next.splice(from, 1);
  next.splice(clamped, 0, picked!);
  return next;
}

// Arrange items in the given id order; any items not listed follow in their existing order
export function orderById<T extends { id: string }>(items: readonly T[], ids: readonly string[]): T[] {
  const byId = new Map(items.map(x => [x.id, x]));
  const seen = new Set<string>();
  const ordered: T[] = [];
  for (const id of ids) {
    const item = byId.get(id);
    if (item && !seen.has(id)) { seen.add(id); ordered.push(item); }
  }
  for (const item of items) if (!seen.has(item.id)) ordered.push(item);
  return ordered;
}

// The Y offset of a computed `translate` ("none", "Xpx" or "Xpx Ypx")
function translateYOf(el: HTMLElement): number {
  const parts = getComputedStyle(el).translate.split(' ');
  return parts.length >= 2 ? parseFloat(parts[1]!) || 0 : 0;
}

// Nearest vertically scrollable ancestor that currently overflows (the dialog's DialogBody, or the settings page)
function scrollParentOf(el: HTMLElement | null): HTMLElement | null {
  for (let cur = el?.parentElement; cur && cur !== document.body; cur = cur.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(cur).overflowY) && cur.scrollHeight > cur.clientHeight + 1) return cur;
  }
  return null;
}

// Given the non-dragged rows' layout heights in order, the dragged row's height, and the dragged row's centre
// relative to the list top, return the insertion slot [0..otherHeights.length].
// Slot k puts the dragged row at y_k = sum_{j < k} otherHeights[j], centred at y_k + dragHeight / 2; the midpoint
// to slot k + 1 is y_k + (otherHeights[k] + dragHeight) / 2, independent of the current slot so swaps never oscillate
export function dropSlot(otherHeights: readonly number[], dragHeight: number, dragCenter: number): number {
  let slot = 0;
  let y = 0;
  for (const h of otherHeights) {
    if (dragCenter > y + (h + dragHeight) / 2) {
      y += h;
      slot++;
    } else break;
  }
  return slot;
}

export interface SortableRowState {
  dragging: boolean;
  keyShortcuts?: string;
}

// Vertical settings rows with pointer drag-to-reorder and Alt+↑ / Alt+↓ keyboard reordering: the held row stays glued
// under the pointer (clamped to the list, with edge auto-scroll in long lists) while siblings slide into their new slots;
// releasing slides the held row into place and commits the new order
export function SortableRows<T extends { id: string }>({ items, canDrag, resetKey = '', dataAttr, onReorder, renderRow }: {
  items: readonly T[];
  canDrag: boolean;
  // When this changes (e.g. a search filter), skip the FLIP animation so rows appear directly in place
  resetKey?: string;
  // Optional `data-*` attribute set to each item's id on its row wrapper
  dataAttr?: `data-${string}`;
  onReorder: (ids: string[]) => void;
  renderRow: (item: T, state: SortableRowState) => ReactNode;
}) {
  const [drag, setDrag] = useState<{ id: string; order: string[] }>();
  // Mirrored in a ref so scroll, rAF auto-scroll and Escape listeners read live state without stale closures
  const dragRef = useRef<{ id: string; order: string[] }>(undefined);
  // The row that keeps its lifted surface: from drag start until its landing slide finishes after release
  const [lifted, setLifted] = useState<string>();
  const press = useRef<{
    id: string; x: number; y: number; atY: number; grabOffset: number; dragHeight: number;
    others: { id: string; height: number }[]; scroller: HTMLElement | null; pointerId: number;
  }>(undefined);
  const listRef = useRef<HTMLDivElement>(null);
  const rowEls = useRef(new Map<string, HTMLDivElement>());
  const slots = useRef(new Map<string, number>());
  const lastLayout = useRef('');
  const lastReset = useRef(resetKey);
  const swallowClick = useRef(false);
  const refocus = useRef<string>(undefined);
  const scrollRaf = useRef<number>(undefined);
  const scrollLast = useRef(0);

  const shown = useMemo(() => (drag ? orderById(items, drag.order) : items), [items, drag]);

  const stopScroll = () => {
    if (scrollRaf.current !== undefined) {
      cancelAnimationFrame(scrollRaf.current);
      scrollRaf.current = undefined;
    }
  };

  // Position the dragged row under the pointer within the list's vertical bounds
  const follow = () => {
    const p = press.current;
    const list = listRef.current;
    const el = p && rowEls.current.get(p.id);
    if (!p || !list || !el) return;
    const maxTop = Math.max(0, list.offsetHeight - p.dragHeight);
    const top = Math.max(0, Math.min(maxTop, p.atY - list.getBoundingClientRect().top - p.grabOffset));
    el.style.translate = `0 ${top - el.offsetTop}px`;
  };

  const syncDrag = () => {
    const p = press.current;
    const list = listRef.current;
    if (!p || !list) return;
    const center = p.atY - list.getBoundingClientRect().top - p.grabOffset + p.dragHeight / 2;
    const slot = dropSlot(p.others.map(o => o.height), p.dragHeight, center);
    const next = p.others.map(o => o.id);
    next.splice(slot, 0, p.id);
    if (next.join('\0') !== dragRef.current?.order.join('\0')) {
      const nextDrag = { id: p.id, order: next };
      dragRef.current = nextDrag;
      setDrag(nextDrag);
    }
    follow();
  };

  const scrollVelocity = (): number => {
    const p = press.current;
    const scroller = p?.scroller;
    if (!p || !dragRef.current || !scroller) return 0;
    const r = scroller.getBoundingClientRect();
    const topDist = r.top + SCROLL_EDGE - p.atY;
    if (topDist > 0 && scroller.scrollTop > 0) return -SCROLL_SPEED * Math.min(1, topDist / SCROLL_EDGE);
    const botDist = p.atY - (r.bottom - SCROLL_EDGE);
    const maxScroll = scroller.scrollHeight - scroller.clientHeight;
    if (botDist > 0 && scroller.scrollTop < maxScroll) return SCROLL_SPEED * Math.min(1, botDist / SCROLL_EDGE);
    return 0;
  };

  const pumpScroll = () => {
    if (scrollRaf.current !== undefined || scrollVelocity() === 0) return;
    scrollLast.current = performance.now();
    const step = (now: number) => {
      const dt = Math.min(0.05, Math.max(0, (now - scrollLast.current) / 1000));
      scrollLast.current = now;
      const v = scrollVelocity();
      const scroller = press.current?.scroller;
      if (!v || !scroller) { scrollRaf.current = undefined; return; }
      const prev = scroller.scrollTop;
      scroller.scrollTop += v * dt;
      if (scroller.scrollTop !== prev) syncDrag();
      scrollRaf.current = requestAnimationFrame(step);
    };
    scrollRaf.current = requestAnimationFrame(step);
  };

  const finish = (drop: boolean) => {
    const p = press.current;
    const active = dragRef.current;
    press.current = undefined;
    dragRef.current = undefined;
    stopScroll();
    if (!active) return;
    const list = listRef.current;
    if (p && list?.hasPointerCapture(p.pointerId)) {
      try { list.releasePointerCapture(p.pointerId); } catch { /* already released */ }
    }
    // The click that follows a drag's pointerup must not open the row's dialog
    swallowClick.current = true;
    setTimeout(() => { swallowClick.current = false; });
    setDrag(undefined);
    if (drop && active.order.join('\0') !== items.map(x => x.id).join('\0')) onReorder(active.order);
  };

  const finishRef = useRef(finish);
  finishRef.current = finish;
  const syncRef = useRef(syncDrag);
  syncRef.current = syncDrag;

  useEffect(() => stopScroll, []);
  useEffect(() => { if (!canDrag && dragRef.current) finishRef.current(false); }, [canDrag]);
  // While holding a row: follow manual wheel scrolling on the scroller, and let Escape cancel the drag without closing the dialog
  useEffect(() => {
    if (!drag) return;
    const scroller = press.current?.scroller;
    const onScroll = () => syncRef.current();
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      finishRef.current(false);
    };
    scroller?.addEventListener('scroll', onScroll, { passive: true });
    window.addEventListener('keydown', onKey, true);
    return () => {
      scroller?.removeEventListener('scroll', onScroll);
      window.removeEventListener('keydown', onKey, true);
    };
  }, [!!drag]);

  // Three-pass FLIP (read → clear → animate) so reordering a long list never interleaves reads and writes
  useLayoutEffect(() => {
    const focusId = refocus.current;
    if (focusId) {
      refocus.current = undefined;
      const row = rowEls.current.get(focusId);
      row?.querySelector<HTMLElement>('[data-row-key]')?.focus({ preventScroll: true });
      row?.scrollIntoView({ block: 'nearest' });
    }
    const resetChanged = resetKey !== lastReset.current;
    lastReset.current = resetKey;
    const layout = `${drag?.id ?? ''}|${shown.map(x => x.id).join('\0')}`;
    const changed = layout !== lastLayout.current;
    lastLayout.current = layout;
    const list = listRef.current;
    for (const id of slots.current.keys()) if (!rowEls.current.has(id)) slots.current.delete(id);
    const still = !list || resetChanged || !!list.closest('[data-motion="none"]') || !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

    // Pass 1: read current slots and any active drag / slide offset
    const moves: { id: string; el: HTMLDivElement; dy: number; released: boolean }[] = [];
    for (const [id, el] of rowEls.current) {
      const was = slots.current.get(id);
      const now = el.offsetTop;
      slots.current.set(id, now);
      if (!changed || id === drag?.id || was === undefined) continue;
      const released = !!el.style.translate;
      if (!released && was === now) continue;
      moves.push({ id, el, dy: was + translateYOf(el) - now, released });
    }
    // Pass 2: cancel existing slides and clear inline drag offsets
    for (const { el } of moves) {
      for (const a of el.getAnimations()) if (a.id === SLIDE_ANIM_ID) a.cancel();
      el.style.translate = '';
    }
    // Pass 3: animate each shifted row (and the released row into its slot)
    for (const { id, el, dy, released } of moves) {
      const land = () => setLifted(l => l === id ? undefined : l);
      if (still || Math.abs(dy) < 0.5) {
        if (released) land();
        continue;
      }
      const slide = el.animate([{ translate: `0 ${dy}px` }, { translate: '0 0' }], { id: SLIDE_ANIM_ID, duration: SLIDE_MS, easing: SLIDE_EASE });
      if (released) slide.onfinish = slide.oncancel = land;
    }
    if (drag) follow();
    else if (lifted) {
      const el = rowEls.current.get(lifted);
      if (!el || (!el.style.translate && !el.getAnimations().some(a => a.id === SLIDE_ANIM_ID))) {
        setLifted(l => l === lifted ? undefined : l);
      }
    }
  });

  const onPointerDown = (e: PointerEvent<HTMLDivElement>, id: string) => {
    if (!canDrag || e.button !== 0 || !e.currentTarget.contains(e.target as Node)) return;
    if ((e.target as Element).closest(NO_DRAG)) return;
    const el = e.currentTarget;
    const list = listRef.current;
    if (!list) return;
    // A row caught mid-slide is picked up from its resting slot
    for (const a of el.getAnimations()) if (a.id === SLIDE_ANIM_ID) a.cancel();
    el.style.translate = '';
    const listTop = list.getBoundingClientRect().top;
    const others: { id: string; height: number }[] = [];
    for (const item of items) {
      if (item.id === id) continue;
      const row = rowEls.current.get(item.id);
      if (row) others.push({ id: item.id, height: row.offsetHeight });
    }
    press.current = {
      id, x: e.clientX, y: e.clientY, atY: e.clientY,
      grabOffset: e.clientY - (listTop + el.offsetTop),
      dragHeight: el.offsetHeight,
      others,
      scroller: scrollParentOf(list),
      pointerId: e.pointerId,
    };
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const p = press.current;
    if (!p) return;
    // Button released outside the list before capture started: drop the stale press
    if (e.pointerType === 'mouse' && (e.buttons & 1) === 0) { finish(false); return; }
    if (!dragRef.current) {
      if (Math.hypot(e.clientX - p.x, e.clientY - p.y) < DRAG_SLOP) return;
      try { e.currentTarget.setPointerCapture(e.pointerId); } catch { /* synthetic events in tests */ }
      setLifted(p.id);
      const initial = { id: p.id, order: items.map(x => x.id) };
      dragRef.current = initial;
      setDrag(initial);
    }
    p.atY = e.clientY;
    syncDrag();
    pumpScroll();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>, id: string) => {
    if (!canDrag || !e.altKey || !e.currentTarget.contains(e.target as Node)) return;
    const step = e.key === 'ArrowUp' ? -1 : e.key === 'ArrowDown' ? 1 : 0;
    if (!step) return;
    e.preventDefault();
    const ids = items.map(x => x.id);
    const next = moveById(items, id, ids.indexOf(id) + step).map(x => x.id);
    if (next.join('\0') === ids.join('\0')) return;
    refocus.current = id;
    onReorder(next);
  };

  const keyShortcuts = canDrag ? 'Alt+ArrowUp Alt+ArrowDown' : undefined;

  return (
    // Move and release sit on the list: React reorders row nodes while previewing, which drops a child's pointer capture
    <div
      ref={listRef}
      className={cn('relative flex flex-col', rowRules)}
      onPointerMove={onPointerMove}
      onPointerUp={() => finish(true)}
      onPointerCancel={() => finish(false)}
      onLostPointerCapture={() => { if (dragRef.current) finish(false); }}
      onClickCapture={e => {
        if (swallowClick.current && e.currentTarget.contains(e.target as Node)) {
          swallowClick.current = false;
          e.preventDefault();
          e.stopPropagation();
        }
      }}
    >
      {shown.map(item => {
        const isLifted = lifted === item.id;
        return (
          <div
            key={item.id}
            ref={el => { if (el) rowEls.current.set(item.id, el); else rowEls.current.delete(item.id); }}
            {...(dataAttr ? { [dataAttr]: item.id } : {})}
            data-dragging={isLifted ? '' : undefined}
            onPointerDown={e => onPointerDown(e, item.id)}
            onKeyDown={e => onKeyDown(e, item.id)}
            className={cn(
              'relative min-w-0',
              canDrag && 'touch-none select-none',
              isLifted && 'z-10 after:opacity-0',
              drag && !isLifted && 'pointer-events-none',
            )}
          >
            {renderRow(item, { dragging: isLifted, keyShortcuts })}
          </div>
        );
      })}
    </div>
  );
}
