import { useCallback, useLayoutEffect, useRef, type KeyboardEvent } from 'react';

// What a hand-built composite holds, by its role
const ITEM: Record<string, string> = { tablist: 'tab', radiogroup: 'radio' };

const itemsOf = (group: HTMLElement) => {
  const role = ITEM[group.getAttribute('role') ?? ''];
  return role ? [...group.querySelectorAll<HTMLElement>(`[role="${role}"]:not(:disabled)`)] : [];
};
const isOn = (item: HTMLElement) => item.getAttribute('aria-selected') === 'true' || item.getAttribute('aria-checked') === 'true';

// Scrolls a strip sideways just far enough to show one of its items, without scrollIntoView (or a focus scroll) moving
// any of the shell's ancestor scrollports
export function revealInline(strip: HTMLElement, item: HTMLElement) {
  const box = strip.getBoundingClientRect();
  const at = item.getBoundingClientRect();
  if (at.left < box.left) strip.scrollLeft -= box.left - at.left;
  else if (at.right > box.right) strip.scrollLeft += at.right - box.right;
}

// The keyboard behaviour of a tablist or radiogroup built from plain buttons, as the ARIA authoring practices give it and
// as Base UI's own Tabs / RadioGroup behave: the group is one tab stop (the selected item, else the first), the arrows
// walk its items and wrap, Home / End jump to the ends. Spread the result on the element carrying the role; the items
// set no `tabIndex` of their own.
// `select` also presses the item the focus lands on, for groups where choosing costs nothing. Without it Enter / Space
// press it, as on any button: right where a choice starts work (opening a terminal tab, launching an agent).
export function useRoving<T extends HTMLElement>({ select = false }: { select?: boolean } = {}) {
  const ref = useRef<T>(null);
  // Written to the DOM after every render: an item cannot tell whether a sibling is selected, and a group with nothing
  // selected still needs its one stop
  useLayoutEffect(() => {
    if (!ref.current) return;
    const items = itemsOf(ref.current);
    const stop = items.find(isOn) ?? items[0];
    for (const item of items) item.tabIndex = item === stop ? 0 : -1;
  });
  const onKeyDown = useCallback((e: KeyboardEvent<T>) => {
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    const group = e.currentTarget;
    const items = itemsOf(group);
    // Keys typed inside an item (a rename field, its close button) are its own
    const at = items.indexOf(e.target as HTMLElement);
    if (at < 0) return;
    // A radio group answers both axes; a tab strip leaves up / down to the page
    const radio = group.getAttribute('role') === 'radiogroup';
    const next = e.key === 'ArrowRight' || (radio && e.key === 'ArrowDown') ? (at + 1) % items.length
      : e.key === 'ArrowLeft' || (radio && e.key === 'ArrowUp') ? (at - 1 + items.length) % items.length
      : e.key === 'Home' ? 0
      : e.key === 'End' ? items.length - 1
      : -1;
    if (next < 0) return;
    e.preventDefault();
    const item = items[next]!;
    item.focus({ preventScroll: true });
    revealInline(group, item);
    if (select && next !== at) item.click();
  }, [select]);
  return { ref, onKeyDown };
}
