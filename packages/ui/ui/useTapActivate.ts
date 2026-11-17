import { useRef, type MouseEvent, type PointerEvent } from 'react';

// Pointer handlers for a clickable row whose hover reveals controls (a cross-fade from opacity 0). On a macOS touch
// display WKWebView treats a finger iOS-style: when the tap's hover makes clickable content visible, the tap stays a
// hover and no click follows (docs/dev/desktop.md), so a row with hover actions took two taps. The row activates on
// the press's release instead, and the click that follows it (a mouse, or a tap WebKit let through) is dropped, so a
// toggle never runs twice. A press that starts on a control inside the row, moves, or is held is left to the click.
const SLOP = 6;
const HOLD_MS = 600;

export function useTapActivate(activate: () => void, enabled = true) {
  const down = useRef<{ x: number; y: number; at: number }>(undefined);
  // Set when the release activated; the next click (or press) clears it
  const done = useRef(false);
  return {
    onPointerDown: (e: PointerEvent<HTMLElement>) => {
      done.current = false;
      const control = (e.target as Element).closest('button, a, input, textarea, select, [role="menuitem"]');
      const own = control && control !== e.currentTarget && e.currentTarget.contains(control);
      down.current = enabled && e.button === 0 && !e.ctrlKey && !e.metaKey && !e.shiftKey && !e.altKey && !own
        ? { x: e.clientX, y: e.clientY, at: e.timeStamp }
        : undefined;
    },
    onPointerUp: (e: PointerEvent<HTMLElement>) => {
      const d = down.current;
      down.current = undefined;
      if (!d || !enabled || e.timeStamp - d.at > HOLD_MS || Math.hypot(e.clientX - d.x, e.clientY - d.y) > SLOP) return;
      done.current = true;
      activate();
    },
    onPointerCancel: () => { down.current = undefined; },
    onClick: (_e: MouseEvent<HTMLElement>) => {
      if (done.current) { done.current = false; return; }
      if (enabled) activate();
    },
  };
}
