import { useCallback, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { t } from '../../i18n';
import { vscodeApi } from '../../vscodeApi';

// Width of the docked subagent inspector. The user drags the pane's left edge; the choice survives a webview reload
// through the host's webview state (next to the fold memory), and is clamped to the room the shell has right now.
const KEY = 'subagentPaneW';
const STEP = 16;

function host(): { getState?(): unknown; setState?(state: unknown): void } | undefined {
  try { return vscodeApi(); } catch { return undefined; }
}

function stored(): number | undefined {
  const state = host()?.getState?.();
  const value = state && typeof state === 'object' ? (state as Record<string, unknown>)[KEY] : undefined;
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

function persist(width: number | undefined) {
  const api = host();
  if (!api?.setState) return;
  const previous = api.getState?.();
  const rest = { ...(previous && typeof previous === 'object' ? previous as Record<string, unknown> : {}) };
  if (width === undefined) delete rest[KEY];
  else rest[KEY] = width;
  api.setState(rest);
}

export interface PaneBounds { min: number; max: number; initial: number }

// The user's chosen width (undefined = the token default); the shell clamps it to the room it has right now,
// so a narrow window squeezes the pane without forgetting the choice.
export function usePreferredPaneWidth() {
  const [preferred, setPreferred] = useState<number | undefined>(stored);
  const commit = useCallback((next: number | undefined) => { setPreferred(next); persist(next); }, []);
  return [preferred, commit] as const;
}

export function clamp(value: number, min: number, max: number) {
  return Math.round(Math.min(Math.max(value, min), Math.max(min, max)));
}

interface SplitterProps {
  width: number;
  bounds: () => PaneBounds;
  // Live width while dragging (not persisted) and the final width on release
  onDrag: (width: number) => void;
  onCommit: (width: number) => void;
  onReset: () => void;
}

// The pane sits on the right, so dragging left widens it. Arrow keys nudge, double-click restores the default width.
export function InspectorSplitter({ width, bounds, onDrag, onCommit, onReset }: SplitterProps) {
  const [dragging, setDragging] = useState(false);
  const { min, max } = bounds();
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const target = e.currentTarget;
    // Capture keeps the drag alive over iframes / outside the pane; a synthetic pointer has nothing to capture
    try { target.setPointerCapture(e.pointerId); } catch { /* keep the listeners */ }
    const startX = e.clientX;
    const startW = width;
    let last = startW;
    setDragging(true);
    const move = (ev: globalThis.PointerEvent) => {
      const b = bounds();
      last = clamp(startW + startX - ev.clientX, b.min, b.max);
      onDrag(last);
    };
    const up = () => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
      target.removeEventListener('pointercancel', up);
      setDragging(false);
      if (last !== startW) onCommit(last);
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
    target.addEventListener('pointercancel', up);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const delta = e.key === 'ArrowLeft' ? STEP : e.key === 'ArrowRight' ? -STEP : 0;
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
      className="subagent-splitter"
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      onDoubleClick={onReset}
    />
  );
}
