import type { CSSProperties } from 'react';
import type { SessionSummary } from '@acpira/shared/transcript';

// The session list's side of plugin styles (docs/dev/plugins.md Interface, `style`): what a row says about itself as
// attributes and custom properties a stylesheet can select and compute with, without a frame per row. A finished
// session is aged with calc() over two properties, `--done-at` on the row (fixed) and `--now` on the list (ticking),
// both seconds since this page loaded: small numbers calc keeps exact, and one write a tick for the whole list

const EPOCH = Date.now();
const TICK_MS = 5_000;
const seconds = (ms: number) => Math.round((ms - EPOCH) / 1000);

const roots = new Set<HTMLElement>();
let needed = false;
let timer: ReturnType<typeof setInterval> | undefined;
const tick = () => {
  if (document.hidden) return;
  const now = String(seconds(Date.now()));
  for (const r of roots) r.style.setProperty('--now', now);
};
const onVisible = () => { if (!document.hidden) tick(); };

function syncTimer() {
  if (needed && roots.size > 0) {
    tick();
    if (!timer) {
      timer = setInterval(tick, TICK_MS);
      document.addEventListener('visibilitychange', onVisible);
    }
    return;
  }
  if (!timer) return;
  clearInterval(timer);
  timer = undefined;
  document.removeEventListener('visibilitychange', onVisible);
}

// Only tick `--now` while a loaded plugin stylesheet actually references it: writing an unregistered inherited custom
// property on the list root every 5 s invalidates styles across the entire session list subtree for nothing otherwise
export function setStyleClockNeeded(on: boolean) {
  if (needed === on) return;
  needed = on;
  syncTimer();
}

// A ref callback for a list root: it carries `--now` while mounted and a plugin stylesheet asks for it
export function clockRoot(el: HTMLElement | null) {
  if (!el) return;
  roots.add(el);
  if (needed) el.style.setProperty('--now', String(seconds(Date.now())));
  syncTimer();
  return () => {
    roots.delete(el);
    syncTimer();
  };
}

// `data-state`: the list's state, with waiting split into what waits
export const rowState = (s: SessionSummary) => (s.state === 'waiting' ? (s.question ? 'question' : 'permission') : s.state);

export const rowDone = (s: SessionSummary): CSSProperties | undefined =>
  s.completedAt === undefined ? undefined : ({ '--done-at': seconds(s.completedAt) } as CSSProperties);
