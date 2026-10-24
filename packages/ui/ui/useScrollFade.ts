import { useCallback } from 'react';

// Batch scroll-fade measurements into one read-then-write pass per animation frame so mounting many
// scrollable cards (user messages, code blocks) never triggers forced synchronous layout thrashing.
const pending = new Set<HTMLElement>();
let frameId: number | undefined;
let sharedResize: ResizeObserver | undefined;

function updateOne(el: HTMLElement) {
  if (el.clientHeight < 1) return;
  const overflow = el.scrollHeight - el.clientHeight;
  const above = overflow > 1 && el.scrollTop > 1;
  const below = overflow > 1 && overflow - el.scrollTop > 1;
  if (el.hasAttribute('data-more-above') !== above) el.toggleAttribute('data-more-above', above);
  if (el.hasAttribute('data-more-below') !== below) el.toggleAttribute('data-more-below', below);
}

function flush() {
  frameId = undefined;
  if (!pending.size) return;
  const batch = [...pending];
  pending.clear();
  // Read pass: read all geometry before mutating any attribute so the browser layouts at most once.
  const next = batch.map(el => {
    const ch = el.clientHeight;
    if (ch < 1) return undefined;
    const overflow = el.scrollHeight - ch;
    const top = el.scrollTop;
    return { el, above: overflow > 1 && top > 1, below: overflow > 1 && overflow - top > 1 };
  });
  // Write pass: only touch attributes that actually flipped.
  for (const item of next) {
    if (!item) continue;
    const { el, above, below } = item;
    if (el.hasAttribute('data-more-above') !== above) el.toggleAttribute('data-more-above', above);
    if (el.hasAttribute('data-more-below') !== below) el.toggleAttribute('data-more-below', below);
  }
}

function scheduleElement(el: HTMLElement) {
  if (typeof requestAnimationFrame === 'undefined') { updateOne(el); return; }
  pending.add(el);
  if (frameId === undefined) frameId = requestAnimationFrame(flush);
}

function resizeObserver(): ResizeObserver | undefined {
  if (typeof ResizeObserver === 'undefined') return undefined;
  return (sharedResize ??= new ResizeObserver(entries => {
    for (const entry of entries) scheduleElement(entry.target as HTMLElement);
  }));
}

// Fade only edges with hidden content. Observe text updates and disclosure sizing,
// so streaming, resizing and mounting an initially empty output update immediately.
export function useScrollFade<T extends HTMLElement>() {
  return useCallback((element: T | null) => {
    if (!element) return;
    const schedule = () => scheduleElement(element);
    schedule();
    element.addEventListener('scroll', schedule, { passive: true });
    const size = resizeObserver();
    size?.observe(element);
    const content = new MutationObserver(schedule);
    content.observe(element, { childList: true, characterData: true, subtree: true });
    return () => {
      element.removeEventListener('scroll', schedule);
      size?.unobserve(element);
      content.disconnect();
      pending.delete(element);
    };
  }, []);
}
