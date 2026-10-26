import { memo, useCallback, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { t, useLocale } from '../../i18n';
import { setWebviewState, webviewState } from '../../vscodeApi';

// Width of a docked pane: the subagent inspector (`subagentPaneW`) or the session column (`sessionPaneW`). The user drags
// the pane's inner edge; the choice survives a webview reload through the host's webview state (next to the fold memory),
// and is clamped to the room the shell has right now.
const STEP = 16;

function stored(key: string): number | undefined {
  const value = webviewState(key);
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

export interface PaneBounds { min: number; max: number; initial: number }

// The user's chosen width (undefined = the token default); the shell clamps it to the room it has right now,
// so a narrow window squeezes the pane without forgetting the choice.
export function usePreferredPaneWidth(key = 'subagentPaneW') {
  const [preferred, setPreferred] = useState<number | undefined>(() => stored(key));
  const commit = useCallback((next: number | undefined) => { setPreferred(next); setWebviewState(key, next); }, [key]);
  return [preferred, commit] as const;
}

export function clamp(value: number, min: number, max: number) {
  return Math.round(Math.min(Math.max(value, min), Math.max(min, max)));
}

interface SplitterProps {
  // The pane's width as chosen (undefined: the default, `bounds().initial`)
  width: number | undefined;
  bounds: () => PaneBounds;
  // Live width while dragging (not persisted, at most once per frame) and the final width on release
  onDrag: (width: number) => void;
  onCommit: (width: number) => void;
  onReset: () => void;
  // The pane's side of the shell: on the right its left edge drags (leftwards widens), on the left its right edge
  side?: 'left' | 'right';
}

// Arrow keys nudge, double-click restores the default width. Memoized: the session column's edge is mounted for good
// beside a shell that renders on every stream chunk, and its bounds read computed styles
export const InspectorSplitter = memo(function InspectorSplitter({ width: chosen, bounds, onDrag, onCommit, onReset, side = 'right' }: SplitterProps) {
  // memo keeps a row from rendering again; the locale context still reaches it after a language change
  useLocale();
  const dir = side === 'right' ? 1 : -1;
  const [dragging, setDragging] = useState(false);
  const { min, max, initial } = bounds();
  const width = chosen ?? initial;
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const target = e.currentTarget;
    // Capture keeps the drag alive over iframes / outside the pane; a synthetic pointer has nothing to capture
    try { target.setPointerCapture(e.pointerId); } catch { /* keep the listeners */ }
    const startX = e.clientX;
    // The pane as laid out: CSS may have clamped the chosen width to a narrow shell
    const startW = target.parentElement?.offsetWidth ?? width;
    // Read once per drag: bounds() reads computed styles and widths, which per move forced a style recalc and a layout
    // on top of the one the new width causes (a resize mid-drag is rare, and the release commits a clamped width anyway)
    const b = bounds();
    let last = startW;
    let shown = startW;
    // One width per frame: pointer moves can outpace frames, and every applied width relays the shell out
    let frame: number | undefined;
    const apply = () => {
      frame = undefined;
      if (last === shown) return;
      shown = last;
      onDrag(last);
    };
    setDragging(true);
    const move = (ev: globalThis.PointerEvent) => {
      last = clamp(startW + (startX - ev.clientX) * dir, b.min, b.max);
      frame ??= requestAnimationFrame(apply);
    };
    const up = () => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
      target.removeEventListener('pointercancel', up);
      // The last move lands before the commit, also when it returns to the starting width (nothing to commit)
      if (frame !== undefined) { cancelAnimationFrame(frame); apply(); }
      setDragging(false);
      if (last !== startW) onCommit(last);
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
    target.addEventListener('pointercancel', up);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const delta = (e.key === 'ArrowLeft' ? STEP : e.key === 'ArrowRight' ? -STEP : 0) * dir;
    if (!delta) return;
    e.preventDefault();
    const b = bounds();
    onCommit(clamp(width + delta, b.min, b.max));
  };
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t('subagents.resize')}
      title={t('subagents.resize')}
      aria-valuenow={width}
      aria-valuemin={min}
      aria-valuemax={Math.max(min, max)}
      tabIndex={0}
      data-dragging={dragging || undefined}
      data-side={side}
      className="subagent-splitter"
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      onDoubleClick={onReset}
    />
  );
});
