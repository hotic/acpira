import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type WheelEvent as ReactWheelEvent } from 'react';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { Dialog } from '../ui/Dialog';
import { ImageMenu } from './ImageMenu';

const MIN_SCALE = 0.5;
const MAX_SCALE = 4;
const ZOOM_SPEED = 0.002;

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
  const [scale, setScale] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const cleanupDrag = useRef<(() => void) | null>(null);
  const suppressBackdropClick = useRef(false);

  useEffect(() => () => cleanupDrag.current?.(), []);

  const onWheel = (e: ReactWheelEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const factor = Math.exp(-e.deltaY * ZOOM_SPEED);
    setScale(cur => {
      const next = Math.min(MAX_SCALE, Math.max(MIN_SCALE, Number((cur * factor).toFixed(3))));
      if (next <= 1) setPan({ x: 0, y: 0 });
      return next;
    });
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLImageElement>) => {
    if (e.button !== 0 || scale <= 1) return;
    e.preventDefault();
    const startX = e.clientX;
    const startY = e.clientY;
    const originX = pan.x;
    const originY = pan.y;
    let moved = false;
    setDragging(true);
    const onMove = (ev: PointerEvent) => {
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      if (Math.abs(dx) > 2 || Math.abs(dy) > 2) moved = true;
      setPan({ x: originX + dx, y: originY + dy });
    };
    const onUp = () => {
      cleanupDrag.current = null;
      setDragging(false);
      if (moved) suppressBackdropClick.current = true;
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
    };
    cleanupDrag.current = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
  };

  const onBackdropClick = () => {
    if (suppressBackdropClick.current) {
      suppressBackdropClick.current = false;
      return;
    }
    onClose();
  };

  const transformed = scale !== 1 || pan.x !== 0 || pan.y !== 0;

  return <Dialog.Root open onOpenChange={open => { if (!open) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Popup
        aria-label={name ?? t('common.image')}
        onClick={onBackdropClick}
        onWheel={onWheel}
        className="absolute inset-0 z-40 flex items-center justify-center overflow-hidden bg-scrim p-page"
      >
        <ImageMenu src={src} mimeType={mimeType} path={path} line={line} openInEditor={openInEditor}>
          <img
            src={src}
            alt={name ?? t('common.image')}
            title={name}
            draggable={false}
            onClick={e => e.stopPropagation()}
            onDoubleClick={e => { e.stopPropagation(); setScale(1); setPan({ x: 0, y: 0 }); }}
            onPointerDown={onPointerDown}
            style={transformed ? { transform: `translate3d(${pan.x}px, ${pan.y}px, 0) scale(${scale})` } : undefined}
            className={cn(
              'max-h-(--lightbox-max-h) max-w-(--lightbox-max-w) rounded-md object-contain shadow-pop select-none',
              scale > 1 && (dragging ? 'cursor-grabbing' : 'cursor-grab'),
            )}
          />
        </ImageMenu>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>;
}
