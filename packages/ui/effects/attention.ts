import { useSyncExternalStore } from 'react';
import { useWindowFocused } from '../plugins/motion';

// Whether anyone is plausibly looking at the page: its window has the focus and there was input within REST_MS. The
// composer beam turns for as long as it is lit (around a tenth of a core in WebKit's GPU process and as much again in
// the page), so it fades out while nobody is. The metal ring and the turn Orb are not gated by this: a ring held on one
// frame reads as stuck, and the Orb is the "still working" signal.
// WebKit sends no `focusout` when its window loses the focus, so the composer still counts as focused there; the window's
// own focus is the only sign.
export const REST_MS = 30_000;
// `input` covers text that arrives without a key (dictation, paste from a menu); `focus` is seen in the capture phase, so
// the window's own and any element's both count
const INPUT = ['keydown', 'input', 'pointerdown', 'pointermove', 'wheel', 'focus'] as const;

let recent = true;
let last = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
const subs = new Set<() => void>();

function set(next: boolean) {
  if (next === recent) return;
  recent = next;
  for (const fn of subs) fn();
}

function check() {
  const left = last + REST_MS - performance.now();
  if (left > 0) { timer = setTimeout(check, left); return; }
  timer = undefined;
  set(false);
}

// Runs for every pointer move: one clock read, and a timer only when coming back from rest
function onInput() {
  last = performance.now();
  if (timer !== undefined) return;
  timer = setTimeout(check, REST_MS);
  set(true);
}

function subscribe(fn: () => void) {
  if (!subs.size) {
    for (const type of INPUT) window.addEventListener(type, onInput, { capture: true, passive: true });
    onInput();
  }
  subs.add(fn);
  return () => {
    subs.delete(fn);
    if (subs.size) return;
    for (const type of INPUT) window.removeEventListener(type, onInput, { capture: true });
    clearTimeout(timer);
    timer = undefined;
    recent = true;
  };
}
const never = () => () => {};

// `on`: whether the caller has a loop to run at all; the input listeners stay off the window while no caller does
export function useAttended(on: boolean): boolean {
  const focused = useWindowFocused();
  const input = useSyncExternalStore(on ? subscribe : never, () => recent, () => true);
  return !on || (focused && input);
}
