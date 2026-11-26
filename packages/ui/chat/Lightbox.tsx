import { useEffect, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from 'react';
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

export interface ZoomOptions {
  // Intrinsic content dimensions for non-<img> targets (e.g. inline SVG viewBox) so zoom can reach 1:1 readable text and beyond.
  naturalSize?: { width: number; height: number };
  // 'zoom' intercepts every wheel event (modal lightboxes); 'ctrl-only' leaves plain scrolling to the document and zooms only on pinch / Ctrl+wheel.
  wheelMode?: 'zoom' | 'ctrl-only';
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

const gesturePoint = (e: WebKitGestureEvent): { x: number; y: number } | null =>
  typeof e.clientX === 'number' && typeof e.clientY === 'number' && Number.isFinite(e.clientX) && Number.isFinite(e.clientY) && (e.clientX !== 0 || e.clientY !== 0)
    ? { x: e.clientX, y: e.clientY }
    : null;

// Shared wheel / pinch-zoom / drag-pan / double-click-reset state for the Lightbox family (`Lightbox`,
// `SharePreviewDialog` and `MermaidBlock`). Handles three pinch paths on the popup surface (`surfaceRef`): WebKit
// `gesture*` events (macOS touch displays and trackpads, prevented so the desktop shell does not step window zoom),
// multi-touch PointerEvents (Windows WebView2 / Chromium touchscreens, paired with `touch-none`), and non-passive
// `wheel` events (mouse wheel, trackpad scroll, and Chromium `ctrlKey` pinch). Zooming in anchors under the pointer or
// fingers; zooming out glides the pan back toward center so returning to 1x never jumps, and large images or diagrams
// can zoom past 1:1 natural size instead of stopping while fitted text is still tiny.
export function useImageZoom(resetKey?: string, options?: ZoomOptions) {
  const [view, setView] = useState({ scale: 1, x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const [surface, setSurface] = useState<HTMLElement | null>(null);
  const targetRef = useRef<HTMLElement | null>(null);
  const committedRef = useRef(view);
  const optionsRef = useRef(options);
  optionsRef.current = options;
  const cleanupDrag = useRef<(() => void) | null>(null);
  const gestureRef = useRef<{ scale: number; point: { x: number; y: number } | null } | null>(null);
  const suppressBackdropClick = useRef(false);
  const clearSuppressTimer = useRef<number | undefined>(undefined);

  useLayoutEffect(() => {
    committedRef.current = view;
  });

  const armSuppressBackdropClick = () => {
    suppressBackdropClick.current = true;
    if (clearSuppressTimer.current !== undefined) window.clearTimeout(clearSuppressTimer.current);
    clearSuppressTimer.current = window.setTimeout(() => {
      clearSuppressTimer.current = undefined;
      suppressBackdropClick.current = false;
    }, 0);
  };

  const getActualScale = () => {
    const el = targetRef.current;
    if (!el) return 1;
    const nat = optionsRef.current?.naturalSize;
    const natW = nat?.width ?? (el instanceof HTMLImageElement ? el.naturalWidth : 0);
    const natH = nat?.height ?? (el instanceof HTMLImageElement ? el.naturalHeight : 0);
    const sw = el.clientWidth > 0 && natW > 0 ? natW / el.clientWidth : 1;
    const sh = el.clientHeight > 0 && natH > 0 ? natH / el.clientHeight : 1;
    return Math.min(MAX_FIT_SCALE, Math.max(1, sw, sh));
  };

  const zoomBy = (factor: number, clientX?: number, clientY?: number, panX = 0, panY = 0) => {
    const el = targetRef.current;
    if (!el || !Number.isFinite(factor) || factor <= 0) return;
    const rect = el.getBoundingClientRect();
    const committed = committedRef.current;
    const nat = optionsRef.current?.naturalSize;
    const actual = getActualScale();
    const fitCap = nat
      ? Math.min(MAX_FIT_SCALE, actual * 2.5)
      : el.clientWidth > 0 && el instanceof HTMLImageElement && el.naturalWidth > 0
        ? Math.min(MAX_FIT_SCALE, el.naturalWidth / el.clientWidth)
        : MAX_SCALE;
    const maxScale = Math.max(MAX_SCALE, fitCap);

    setView(cur => {
      const nextScale = Math.min(maxScale, Math.max(MIN_SCALE, cur.scale * factor));
      if (nextScale <= 1) return { scale: nextScale, x: 0, y: 0 };
      if (nextScale < cur.scale && cur.scale > 1) {
        const t = (nextScale - 1) / (cur.scale - 1);
        return { scale: nextScale, x: (cur.x + panX) * t, y: (cur.y + panY) * t };
      }
      if (rect.width <= 0 || rect.height <= 0 || cur.scale <= 0 || committed.scale <= 0) {
        return { ...cur, scale: nextScale };
      }
      const baseCx = rect.left + rect.width / 2 - committed.x;
      const baseCy = rect.top + rect.height / 2 - committed.y;
      const curCx = baseCx + cur.x;
      const curCy = baseCy + cur.y;
      const curW = (rect.width / committed.scale) * cur.scale;
      const curH = (rect.height / committed.scale) * cur.scale;
      const anchorX = clientX ?? curCx;
      const anchorY = clientY ?? curCy;
      const dx = Math.max(-curW / 2, Math.min(curW / 2, anchorX - curCx));
      const dy = Math.max(-curH / 2, Math.min(curH / 2, anchorY - curCy));
      const ratio = nextScale / cur.scale;
      return {
        scale: nextScale,
        x: cur.x + dx * (1 - ratio) + panX,
        y: cur.y + dy * (1 - ratio) + panY,
      };
    });
  };

  useEffect(() => {
    cleanupDrag.current?.();
    cleanupDrag.current = null;
    gestureRef.current = null;
    setDragging(false);
    setView({ scale: 1, x: 0, y: 0 });
  }, [resetKey]);

  useEffect(() => () => {
    cleanupDrag.current?.();
    if (clearSuppressTimer.current !== undefined) window.clearTimeout(clearSuppressTimer.current);
  }, []);

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
      if (committedRef.current.scale <= 1) return;
      if (Math.abs(e.clientX - prev.startX) > 2 || Math.abs(e.clientY - prev.startY) > 2) touchMoved = true;
      setView(cur => (cur.scale > 1 ? { ...cur, x: cur.x + dx, y: cur.y + dy } : cur));
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
    };

    surface.addEventListener('wheel', onWheel, { passive: false });
    surface.addEventListener('gesturestart', onGestureStart, { passive: false });
    surface.addEventListener('gesturechange', onGestureChange, { passive: false });
    surface.addEventListener('gestureend', onGestureEnd, { passive: false });
    surface.addEventListener('pointerdown', onTouchDown);
    window.addEventListener('blur', onBlur);
    return () => {
      gestureRef.current = null;
      touches.clear();
      detachTouchWindow();
      surface.removeEventListener('wheel', onWheel);
      surface.removeEventListener('gesturestart', onGestureStart);
      surface.removeEventListener('gesturechange', onGestureChange);
      surface.removeEventListener('gestureend', onGestureEnd);
      surface.removeEventListener('pointerdown', onTouchDown);
      window.removeEventListener('blur', onBlur);
    };
  }, [surface]);

  const onPointerDown = (e: ReactPointerEvent<HTMLElement>) => {
    if (e.pointerType === 'touch' || e.button !== 0 || view.scale <= 1) return;
    e.preventDefault();
    const startX = e.clientX;
    const startY = e.clientY;
    const originX = view.x;
    const originY = view.y;
    let moved = false;
    setDragging(true);
    const onMove = (ev: PointerEvent) => {
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      if (Math.abs(dx) > 2 || Math.abs(dy) > 2) moved = true;
      setView(cur => ({ ...cur, x: originX + dx, y: originY + dy }));
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
  };

  const reset = () => setView({ scale: 1, x: 0, y: 0 });
  const zoomIn = () => zoomBy(ZOOM_STEP);
  const zoomOut = () => zoomBy(1 / ZOOM_STEP);
  const zoomTo = (nextScale: number, clientX?: number, clientY?: number) => {
    const cur = committedRef.current.scale;
    if (cur <= 0 || !Number.isFinite(nextScale) || nextScale <= 0) return;
    zoomBy(nextScale / cur, clientX, clientY);
  };
  const consumeDrag = () => {
    if (!suppressBackdropClick.current) return false;
    suppressBackdropClick.current = false;
    return true;
  };

  const onBackdropClick = (onClose: () => void) => () => {
    if (consumeDrag()) return;
    onClose();
  };

  const transformed = view.scale !== 1 || view.x !== 0 || view.y !== 0;
  const transform = transformed ? `translate3d(${view.x}px, ${view.y}px, 0) scale(${Number(view.scale.toFixed(4))})` : undefined;

  return {
    imgRef: targetRef as RefObject<HTMLImageElement | null>,
    targetRef,
    surfaceRef: setSurface,
    scale: view.scale,
    transformed,
    dragging,
    transform,
    onPointerDown,
    reset,
    zoomIn,
    zoomOut,
    zoomTo,
    getActualScale,
    consumeDrag,
    onBackdropClick,
  };
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
            style={zoom.transform ? { transform: zoom.transform } : undefined}
            className={cn(
              'max-h-(--lightbox-max-h) max-w-(--lightbox-max-w) rounded-md object-contain shadow-pop select-none',
              zoom.scale > 1 && (zoom.dragging ? 'cursor-grabbing' : 'cursor-grab'),
            )}
          />
        </ImageMenu>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>;
}
