import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from 'react';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { Dialog } from '../ui/Dialog';
import { ImageMenu } from './ImageMenu';

const MIN_SCALE = 0.5;
const MAX_SCALE = 4;
const MAX_FIT_SCALE = 24;
const ZOOM_STEP = 1.35;
const ZOOM_SPEED = 0.002;
// Chromium / WebView2 encodes trackpad pinch as ctrlKey wheel deltas ~5x finer than scroll deltas.
const CTRL_WHEEL_ZOOM_SPEED = ZOOM_SPEED * 5;
const GEOM_CACHE_MS = 200;

export interface ZoomOptions {
  // Intrinsic content dimensions for non-<img> targets (e.g. inline SVG viewBox) so zoom can reach 1:1 readable text and beyond.
  naturalSize?: { width: number; height: number };
  // 'zoom' intercepts every wheel event (modal lightboxes); 'ctrl-only' leaves plain scrolling to the document and zooms only on pinch / Ctrl+wheel.
  wheelMode?: 'zoom' | 'ctrl-only';
  // Called after each transform write, so a caller can re-rasterize its content once the view settles.
  onViewChange?: () => void;
}

// WebKit's non-standard pinch gesture event (macOS trackpads and touch displays).
interface WebKitGestureEvent extends UIEvent {
  scale: number;
  clientX?: number;
  clientY?: number;
}

interface TouchPoint {
  x: number;
  y: number;
  startX: number;
  startY: number;
}

interface ViewState {
  scale: number;
  x: number;
  y: number;
}

interface BaseGeom {
  cx: number;
  cy: number;
  w: number;
  h: number;
  clientW: number;
  clientH: number;
}

interface ZoomFlags {
  transformed: boolean;
  canPan: boolean;
  zoomed: boolean;
}

const IDENTITY_VIEW: ViewState = { scale: 1, x: 0, y: 0 };
const IDENTITY_FLAGS: ZoomFlags = { transformed: false, canPan: false, zoomed: false };

const flagsOf = (v: ViewState): ZoomFlags => ({
  transformed: v.scale !== 1 || v.x !== 0 || v.y !== 0,
  canPan: v.scale > 1,
  zoomed: v.scale > 1.01,
});

const formatTransform = (v: ViewState): string =>
  v.scale !== 1 || v.x !== 0 || v.y !== 0
    ? `translate3d(${v.x}px, ${v.y}px, 0) scale(${Number(v.scale.toFixed(4))})`
    : '';

const gesturePoint = (e: WebKitGestureEvent): { x: number; y: number } | null =>
  typeof e.clientX === 'number' && typeof e.clientY === 'number' && Number.isFinite(e.clientX) && Number.isFinite(e.clientY) && (e.clientX !== 0 || e.clientY !== 0)
    ? { x: e.clientX, y: e.clientY }
    : null;

// Shared wheel / pinch-zoom / drag-pan / double-click-reset state for the Lightbox family (`Lightbox`,
// `SharePreviewDialog` and `MermaidBlock`). Handles three pinch paths on the popup surface (`surfaceRef`): WebKit
// `gesture*` events (macOS touch displays and trackpads, prevented so the desktop shell does not step window zoom),
// multi-touch PointerEvents (Windows WebView2 / Chromium touchscreens, paired with `touch-none`), and non-passive
// `wheel` events (mouse wheel, trackpad scroll, and Chromium `ctrlKey` pinch). Writes transforms directly to the target
// element on requestAnimationFrame and caches the untransformed geometry during a gesture so high-rate wheel / pinch
// streams trigger zero React re-renders and zero forced synchronous layouts per tick.
export function useImageZoom(resetKey?: string, options?: ZoomOptions) {
  const [flags, setFlags] = useState<ZoomFlags>(IDENTITY_FLAGS);
  const [dragging, setDragging] = useState(false);
  const [surface, setSurface] = useState<HTMLElement | null>(null);
  const targetRef = useRef<HTMLElement | null>(null);
  const scaleLabelRef = useRef<HTMLElement | null>(null);
  const viewRef = useRef<ViewState>({ ...IDENTITY_VIEW });
  const appliedRef = useRef<ViewState>({ ...IDENTITY_VIEW });
  const flagsRef = useRef<ZoomFlags>(IDENTITY_FLAGS);
  const baseGeomRef = useRef<BaseGeom | null>(null);
  const geomTimerRef = useRef<number | undefined>(undefined);
  const rafRef = useRef<number | null>(null);
  const optionsRef = useRef(options);
  optionsRef.current = options;
  const cleanupDrag = useRef<(() => void) | null>(null);
  const gestureRef = useRef<{ scale: number; point: { x: number; y: number } | null } | null>(null);
  const suppressBackdropClick = useRef(false);
  const clearSuppressTimer = useRef<number | undefined>(undefined);

  const clearBaseGeom = useCallback(() => {
    baseGeomRef.current = null;
    if (geomTimerRef.current !== undefined) {
      window.clearTimeout(geomTimerRef.current);
      geomTimerRef.current = undefined;
    }
  }, []);

  const touchBaseGeom = useCallback(() => {
    if (geomTimerRef.current !== undefined) window.clearTimeout(geomTimerRef.current);
    geomTimerRef.current = window.setTimeout(() => {
      geomTimerRef.current = undefined;
      baseGeomRef.current = null;
    }, GEOM_CACHE_MS);
  }, []);

  const syncFlags = useCallback((v: ViewState) => {
    const next = flagsOf(v);
    const prev = flagsRef.current;
    if (next.transformed !== prev.transformed || next.canPan !== prev.canPan || next.zoomed !== prev.zoomed) {
      flagsRef.current = next;
      setFlags(next);
    }
  }, []);

  const applyTransformNow = useCallback(() => {
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    const v = viewRef.current;
    const el = targetRef.current;
    if (el) {
      el.style.transform = formatTransform(v);
      appliedRef.current = { ...v };
    }
    if (scaleLabelRef.current) {
      scaleLabelRef.current.textContent = `${Math.round(v.scale * 100)}%`;
    }
    syncFlags(v);
    optionsRef.current?.onViewChange?.();
  }, [syncFlags]);

  const scheduleApply = useCallback(() => {
    if (rafRef.current !== null) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      const v = viewRef.current;
      const el = targetRef.current;
      if (el) {
        el.style.transform = formatTransform(v);
        appliedRef.current = { ...v };
      }
      if (scaleLabelRef.current) {
        scaleLabelRef.current.textContent = `${Math.round(v.scale * 100)}%`;
      }
      syncFlags(v);
      optionsRef.current?.onViewChange?.();
    });
  }, [syncFlags]);

  useLayoutEffect(() => {
    const el = targetRef.current;
    if (el) {
      const v = viewRef.current;
      el.style.transform = formatTransform(v);
      appliedRef.current = { ...v };
    }
    if (scaleLabelRef.current) {
      scaleLabelRef.current.textContent = `${Math.round(viewRef.current.scale * 100)}%`;
    }
  });

  const armSuppressBackdropClick = useCallback(() => {
    suppressBackdropClick.current = true;
    if (clearSuppressTimer.current !== undefined) window.clearTimeout(clearSuppressTimer.current);
    clearSuppressTimer.current = window.setTimeout(() => {
      clearSuppressTimer.current = undefined;
      suppressBackdropClick.current = false;
    }, 0);
  }, []);

  const readBaseGeom = useCallback((el: HTMLElement): BaseGeom | null => {
    if (baseGeomRef.current) {
      touchBaseGeom();
      return baseGeomRef.current;
    }
    const rect = el.getBoundingClientRect();
    const applied = appliedRef.current;
    if (rect.width <= 0 || rect.height <= 0 || applied.scale <= 0) return null;
    const geom: BaseGeom = {
      cx: rect.left + rect.width / 2 - applied.x,
      cy: rect.top + rect.height / 2 - applied.y,
      w: rect.width / applied.scale,
      h: rect.height / applied.scale,
      clientW: el.clientWidth,
      clientH: el.clientHeight,
    };
    baseGeomRef.current = geom;
    touchBaseGeom();
    return geom;
  }, [touchBaseGeom]);

  const getActualScale = useCallback((geom?: BaseGeom | null) => {
    const el = targetRef.current;
    if (!el) return 1;
    const nat = optionsRef.current?.naturalSize;
    const natW = nat?.width ?? (el instanceof HTMLImageElement ? el.naturalWidth : 0);
    const natH = nat?.height ?? (el instanceof HTMLImageElement ? el.naturalHeight : 0);
    const cw = geom?.clientW ?? el.clientWidth;
    const ch = geom?.clientH ?? el.clientHeight;
    const sw = cw > 0 && natW > 0 ? natW / cw : 1;
    const sh = ch > 0 && natH > 0 ? natH / ch : 1;
    return Math.min(MAX_FIT_SCALE, Math.max(1, sw, sh));
  }, []);

  const zoomBy = useCallback((factor: number, clientX?: number, clientY?: number, panX = 0, panY = 0) => {
    const el = targetRef.current;
    if (!el || !Number.isFinite(factor) || factor <= 0) return;
    const geom = readBaseGeom(el);
    const nat = optionsRef.current?.naturalSize;
    const actual = getActualScale(geom);
    const cw = geom?.clientW ?? el.clientWidth;
    const fitCap = nat
      ? Math.min(MAX_FIT_SCALE, actual * 2.5)
      : cw > 0 && el instanceof HTMLImageElement && el.naturalWidth > 0
        ? Math.min(MAX_FIT_SCALE, el.naturalWidth / cw)
        : MAX_SCALE;
    const maxScale = Math.max(MAX_SCALE, fitCap);

    const cur = viewRef.current;
    const nextScale = Math.min(maxScale, Math.max(MIN_SCALE, cur.scale * factor));
    let next: ViewState;
    if (nextScale <= 1) {
      next = { scale: nextScale, x: 0, y: 0 };
    } else if (nextScale < cur.scale && cur.scale > 1) {
      const ratio = (nextScale - 1) / (cur.scale - 1);
      next = { scale: nextScale, x: (cur.x + panX) * ratio, y: (cur.y + panY) * ratio };
    } else if (!geom || cur.scale <= 0) {
      next = { ...cur, scale: nextScale };
    } else {
      const curCx = geom.cx + cur.x;
      const curCy = geom.cy + cur.y;
      const curW = geom.w * cur.scale;
      const curH = geom.h * cur.scale;
      const anchorX = clientX ?? curCx;
      const anchorY = clientY ?? curCy;
      const dx = Math.max(-curW / 2, Math.min(curW / 2, anchorX - curCx));
      const dy = Math.max(-curH / 2, Math.min(curH / 2, anchorY - curCy));
      const ratio = nextScale / cur.scale;
      next = {
        scale: nextScale,
        x: cur.x + dx * (1 - ratio) + panX,
        y: cur.y + dy * (1 - ratio) + panY,
      };
    }

    viewRef.current = next;
    scheduleApply();
  }, [getActualScale, readBaseGeom, scheduleApply]);

  useEffect(() => {
    cleanupDrag.current?.();
    cleanupDrag.current = null;
    gestureRef.current = null;
    clearBaseGeom();
    setDragging(false);
    viewRef.current = { ...IDENTITY_VIEW };
    applyTransformNow();
  }, [resetKey, applyTransformNow, clearBaseGeom]);

  useEffect(() => () => {
    cleanupDrag.current?.();
    clearBaseGeom();
    if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    if (clearSuppressTimer.current !== undefined) window.clearTimeout(clearSuppressTimer.current);
  }, [clearBaseGeom]);

  useEffect(() => {
    if (!surface) return;
    const touches = new Map<number, TouchPoint>();
    let touchMoved = false;
    let touchTracking = false;

    const detachTouchWindow = () => {
      if (!touchTracking) return;
      touchTracking = false;
      window.removeEventListener('pointermove', onTouchMove);
      window.removeEventListener('pointerup', onTouchEnd);
      window.removeEventListener('pointercancel', onTouchEnd);
    };

    const onWheel = (e: WheelEvent) => {
      if (optionsRef.current?.wheelMode === 'ctrl-only' && !e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      e.stopPropagation();
      // WebKit emits wheel events alongside a trackpad or touch pinch; the gesture handler already owns it.
      if (gestureRef.current) return;
      const speed = e.ctrlKey ? CTRL_WHEEL_ZOOM_SPEED : ZOOM_SPEED;
      zoomBy(Math.exp(-e.deltaY * speed), e.clientX, e.clientY);
    };

    const onGestureStart = (ev: Event) => {
      const e = ev as WebKitGestureEvent;
      e.preventDefault();
      e.stopPropagation();
      const scale = typeof e.scale === 'number' && Number.isFinite(e.scale) && e.scale > 0 ? e.scale : 1;
      gestureRef.current = { scale, point: gesturePoint(e) };
    };

    const onGestureChange = (ev: Event) => {
      const e = ev as WebKitGestureEvent;
      e.preventDefault();
      e.stopPropagation();
      const g = gestureRef.current;
      if (!g || typeof e.scale !== 'number' || !Number.isFinite(e.scale) || e.scale <= 0) return;
      const factor = e.scale / g.scale;
      const nextPoint = gesturePoint(e);
      const prevPoint = g.point;
      const anchor = prevPoint ?? nextPoint;
      const panX = prevPoint && nextPoint ? nextPoint.x - prevPoint.x : 0;
      const panY = prevPoint && nextPoint ? nextPoint.y - prevPoint.y : 0;
      g.scale = e.scale;
      if (nextPoint) g.point = nextPoint;
      zoomBy(factor, anchor?.x, anchor?.y, panX, panY);
    };

    const onGestureEnd = (ev: Event) => {
      ev.preventDefault();
      ev.stopPropagation();
      gestureRef.current = null;
    };

    const onTouchDown = (e: PointerEvent) => {
      if (e.pointerType !== 'touch') return;
      touches.set(e.pointerId, { x: e.clientX, y: e.clientY, startX: e.clientX, startY: e.clientY });
      if (touches.size >= 2) touchMoved = true;
      if (!touchTracking) {
        touchTracking = true;
        window.addEventListener('pointermove', onTouchMove);
        window.addEventListener('pointerup', onTouchEnd);
        window.addEventListener('pointercancel', onTouchEnd);
      }
    };

    const onTouchMove = (e: PointerEvent) => {
      if (e.pointerType !== 'touch') return;
      const prev = touches.get(e.pointerId);
      if (!prev) return;
      if (touches.size >= 2) {
        const entries = [...touches.entries()];
        const [idA, a] = entries[0]!;
        const [idB, b] = entries[1]!;
        if (e.pointerId !== idA && e.pointerId !== idB) {
          touches.set(e.pointerId, { ...prev, x: e.clientX, y: e.clientY });
          return;
        }
        const prevDist = Math.hypot(b.x - a.x, b.y - a.y);
        const prevMidX = (a.x + b.x) / 2;
        const prevMidY = (a.y + b.y) / 2;
        touches.set(e.pointerId, { ...prev, x: e.clientX, y: e.clientY });
        const na = touches.get(idA)!;
        const nb = touches.get(idB)!;
        const nextDist = Math.hypot(nb.x - na.x, nb.y - na.y);
        const nextMidX = (na.x + nb.x) / 2;
        const nextMidY = (na.y + nb.y) / 2;
        const factor = prevDist > 0 && nextDist > 0 ? nextDist / prevDist : 1;
        zoomBy(factor, prevMidX, prevMidY, nextMidX - prevMidX, nextMidY - prevMidY);
        return;
      }
      const dx = e.clientX - prev.x;
      const dy = e.clientY - prev.y;
      touches.set(e.pointerId, { ...prev, x: e.clientX, y: e.clientY });
      if (viewRef.current.scale <= 1) return;
      if (Math.abs(e.clientX - prev.startX) > 2 || Math.abs(e.clientY - prev.startY) > 2) touchMoved = true;
      const cur = viewRef.current;
      viewRef.current = { ...cur, x: cur.x + dx, y: cur.y + dy };
      scheduleApply();
    };

    const onTouchEnd = (e: PointerEvent) => {
      if (e.pointerType !== 'touch' || !touches.delete(e.pointerId)) return;
      if (touches.size === 0) {
        detachTouchWindow();
        if (touchMoved) armSuppressBackdropClick();
        touchMoved = false;
      }
    };

    const onBlur = () => {
      gestureRef.current = null;
      touches.clear();
      detachTouchWindow();
      touchMoved = false;
      clearBaseGeom();
    };

    surface.addEventListener('wheel', onWheel, { passive: false });
    surface.addEventListener('gesturestart', onGestureStart, { passive: false });
    surface.addEventListener('gesturechange', onGestureChange, { passive: false });
    surface.addEventListener('gestureend', onGestureEnd, { passive: false });
    surface.addEventListener('pointerdown', onTouchDown);
    window.addEventListener('blur', onBlur);
    window.addEventListener('resize', clearBaseGeom);
    return () => {
      gestureRef.current = null;
      touches.clear();
      detachTouchWindow();
      clearBaseGeom();
      surface.removeEventListener('wheel', onWheel);
      surface.removeEventListener('gesturestart', onGestureStart);
      surface.removeEventListener('gesturechange', onGestureChange);
      surface.removeEventListener('gestureend', onGestureEnd);
      surface.removeEventListener('pointerdown', onTouchDown);
      window.removeEventListener('blur', onBlur);
      window.removeEventListener('resize', clearBaseGeom);
    };
  }, [surface, zoomBy, scheduleApply, armSuppressBackdropClick, clearBaseGeom]);

  const onPointerDown = useCallback((e: ReactPointerEvent<HTMLElement>) => {
    if (e.pointerType === 'touch' || e.button !== 0 || viewRef.current.scale <= 1) return;
    e.preventDefault();
    const startX = e.clientX;
    const startY = e.clientY;
    const originX = viewRef.current.x;
    const originY = viewRef.current.y;
    let moved = false;
    setDragging(true);
    const onMove = (ev: PointerEvent) => {
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      if (Math.abs(dx) > 2 || Math.abs(dy) > 2) moved = true;
      const cur = viewRef.current;
      viewRef.current = { ...cur, x: originX + dx, y: originY + dy };
      scheduleApply();
    };
    const cleanup = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
    };
    const onUp = () => {
      cleanupDrag.current = null;
      setDragging(false);
      if (moved) armSuppressBackdropClick();
      cleanup();
    };
    cleanupDrag.current = cleanup;
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
  }, [scheduleApply, armSuppressBackdropClick]);

  const reset = useCallback(() => {
    clearBaseGeom();
    viewRef.current = { ...IDENTITY_VIEW };
    applyTransformNow();
  }, [applyTransformNow, clearBaseGeom]);

  const zoomIn = useCallback(() => zoomBy(ZOOM_STEP), [zoomBy]);
  const zoomOut = useCallback(() => zoomBy(1 / ZOOM_STEP), [zoomBy]);
  const zoomTo = useCallback((nextScale: number, clientX?: number, clientY?: number) => {
    const cur = viewRef.current.scale;
    if (cur <= 0 || !Number.isFinite(nextScale) || nextScale <= 0) return;
    zoomBy(nextScale / cur, clientX, clientY);
  }, [zoomBy]);

  const getScale = useCallback(() => viewRef.current.scale, []);
  const isTransformed = useCallback(() => flagsOf(viewRef.current).transformed, []);

  const consumeDrag = useCallback(() => {
    if (!suppressBackdropClick.current) return false;
    suppressBackdropClick.current = false;
    return true;
  }, []);

  const onBackdropClick = useCallback((onClose: () => void) => () => {
    if (consumeDrag()) return;
    onClose();
  }, [consumeDrag]);

  return useMemo(() => ({
    imgRef: targetRef as RefObject<HTMLImageElement | null>,
    targetRef,
    scaleLabelRef,
    surfaceRef: setSurface,
    transformed: flags.transformed,
    canPan: flags.canPan,
    zoomed: flags.zoomed,
    dragging,
    onPointerDown,
    reset,
    zoomIn,
    zoomOut,
    zoomTo,
    getScale,
    isTransformed,
    getActualScale,
    consumeDrag,
    onBackdropClick,
  }), [flags, dragging, onPointerDown, reset, zoomIn, zoomOut, zoomTo, getScale, isTransformed, getActualScale, consumeDrag, onBackdropClick]);
}

// Full-shell image preview, portaled into the shell layer (same home as Popover, so theme tokens and the shell's
// positioning context apply). The image starts within `--lightbox-max-h` / `--lightbox-max-w` so a high-DPI screenshot
// does not fill a desktop window edge to edge; the wheel or pinch zooms it (drag to pan while zoomed, double-click to
// reset), and a backdrop click or Esc closes it without a corner button. Right-click on the image opens the shell's
// image menu (Copy image, plus Open in editor when the caller can open it) instead of the webview's native Cut / Copy /
// Paste
export function Lightbox({ src, name, mimeType, path, line, openInEditor, onClose }: {
  src: string;
  name?: string;
  mimeType?: string;
  path?: string;
  line?: number;
  openInEditor?: () => void;
  onClose: () => void;
}) {
  const zoom = useImageZoom(src);

  return <Dialog.Root open onOpenChange={open => { if (!open) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Popup
        ref={zoom.surfaceRef}
        aria-label={name ?? t('common.image')}
        onScrimClick={zoom.onBackdropClick(onClose)}
        className="absolute inset-0 z-40 flex touch-none items-center justify-center overflow-hidden bg-scrim p-page"
      >
        <ImageMenu src={src} mimeType={mimeType} path={path} line={line} openInEditor={openInEditor}>
          <img
            ref={zoom.imgRef}
            src={src}
            alt={name ?? t('common.image')}
            title={name}
            draggable={false}
            onClick={e => e.stopPropagation()}
            onDoubleClick={e => { e.stopPropagation(); zoom.reset(); }}
            onPointerDown={zoom.onPointerDown}
            className={cn(
              'max-h-(--lightbox-max-h) max-w-(--lightbox-max-w) rounded-md object-contain shadow-pop select-none will-change-transform',
              zoom.canPan && (zoom.dragging ? 'cursor-grabbing' : 'cursor-grab'),
            )}
          />
        </ImageMenu>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>;
}
