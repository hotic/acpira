import { useEffect, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type WheelEvent as ReactWheelEvent } from 'react';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { Dialog } from '../ui/Dialog';
import { ImageMenu } from './ImageMenu';

const MIN_SCALE = 0.5;
const MAX_SCALE = 4;
const MAX_FIT_SCALE = 24;
const ZOOM_SPEED = 0.002;

// Shared wheel-zoom / drag-pan / double-click-reset state for the Lightbox family (`Lightbox` and `SharePreviewDialog`).
// Zooming in anchors under the pointer; zooming out glides the pan back toward center so returning to 1x never jumps,
// and tall images can zoom at least to 1:1 natural pixel width instead of stopping while fitted text is still tiny.
export function useImageZoom(resetKey?: string) {
  const [view, setView] = useState({ scale: 1, x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const imgRef = useRef<HTMLImageElement>(null);
  const committedRef = useRef(view);
  const cleanupDrag = useRef<(() => void) | null>(null);
  const suppressBackdropClick = useRef(false);
  const clearSuppressTimer = useRef<number | undefined>(undefined);

  useLayoutEffect(() => {
    committedRef.current = view;
  });

  useEffect(() => {
    cleanupDrag.current?.();
    cleanupDrag.current = null;
    setDragging(false);
    setView({ scale: 1, x: 0, y: 0 });
  }, [resetKey]);

  useEffect(() => () => {
    cleanupDrag.current?.();
    if (clearSuppressTimer.current !== undefined) window.clearTimeout(clearSuppressTimer.current);
  }, []);

  const onWheel = (e: ReactWheelEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const img = imgRef.current;
    if (!img) return;
    const factor = Math.exp(-e.deltaY * ZOOM_SPEED);
    const rect = img.getBoundingClientRect();
    const committed = committedRef.current;
    const fitCap = img.clientWidth > 0 && img.naturalWidth > 0
      ? Math.min(MAX_FIT_SCALE, img.naturalWidth / img.clientWidth)
      : MAX_SCALE;
    const maxScale = Math.max(MAX_SCALE, fitCap);
    const clientX = e.clientX;
    const clientY = e.clientY;

    setView(cur => {
      const nextScale = Math.min(maxScale, Math.max(MIN_SCALE, Number((cur.scale * factor).toFixed(3))));
      if (nextScale <= 1) return { scale: nextScale, x: 0, y: 0 };
      if (nextScale < cur.scale && cur.scale > 1) {
        const t = (nextScale - 1) / (cur.scale - 1);
        return { scale: nextScale, x: cur.x * t, y: cur.y * t };
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
      const dx = Math.max(-curW / 2, Math.min(curW / 2, clientX - curCx));
      const dy = Math.max(-curH / 2, Math.min(curH / 2, clientY - curCy));
      const ratio = nextScale / cur.scale;
      return {
        scale: nextScale,
        x: cur.x + dx * (1 - ratio),
        y: cur.y + dy * (1 - ratio),
      };
    });
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLImageElement>) => {
    if (e.button !== 0 || view.scale <= 1) return;
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
      if (moved) {
        suppressBackdropClick.current = true;
        if (clearSuppressTimer.current !== undefined) window.clearTimeout(clearSuppressTimer.current);
        clearSuppressTimer.current = window.setTimeout(() => {
          clearSuppressTimer.current = undefined;
          suppressBackdropClick.current = false;
        }, 0);
      }
      cleanup();
    };
    cleanupDrag.current = cleanup;
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
  };

  const reset = () => setView({ scale: 1, x: 0, y: 0 });

  const onBackdropClick = (onClose: () => void) => () => {
    if (suppressBackdropClick.current) {
      suppressBackdropClick.current = false;
      return;
    }
    onClose();
  };

  const transformed = view.scale !== 1 || view.x !== 0 || view.y !== 0;
  const transform = transformed ? `translate3d(${view.x}px, ${view.y}px, 0) scale(${view.scale})` : undefined;

  return {
    imgRef,
    scale: view.scale,
    dragging,
    transform,
    onWheel,
    onPointerDown,
    reset,
    onBackdropClick,
  };
}

// Full-shell image preview, portaled into the shell layer (same home as Popover, so theme tokens and the shell's
// positioning context apply). The image starts within `--lightbox-max-h` / `--lightbox-max-w` so a high-DPI screenshot
// does not fill a desktop window edge to edge; the wheel zooms it (drag to pan while zoomed, double-click to reset),
// and a backdrop click or Esc closes it without a corner button. Right-click on the image opens the shell's image menu
// (Copy image, plus Open in editor when the caller can open it) instead of the webview's native Cut / Copy / Paste
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
        aria-label={name ?? t('common.image')}
        onClick={zoom.onBackdropClick(onClose)}
        onWheel={zoom.onWheel}
        className="absolute inset-0 z-40 flex items-center justify-center overflow-hidden bg-scrim p-page"
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
