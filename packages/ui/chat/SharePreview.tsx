import { useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type Ref } from 'react';
import { ContextMenu } from '@base-ui/react/context-menu';
import { Copy, Download } from 'lucide-react';
import { brandColor } from '@acpira/shared/models';
import { t, useLocale } from '../i18n';
import { Dialog } from '../ui/Dialog';
import { optionClass } from '../ui/DropdownMenu';
import { ShellLayerContext, popupClass } from '../ui/Overlay';
import { hostImageSave } from '../vscodeApi';
import { AgentMark } from './AgentMark';
import { ModelMark } from './ModelMark';
import { MarkSvg } from './marks';
import { blobToDataUrl, renderShareCardToPng, shareFileName } from './shareCapture';
import type { ShareMeta } from './turnActionHelpers';

const contextMenuCollision = { side: 'flip', align: 'flip', fallbackAxisSide: 'none' } as const;
const positionerClass = 'z-50 w-max min-w-pop-sm';

export interface SharePayload {
  meta: ShareMeta;
  node: HTMLElement;
}

// Screenshot card in two layers: the reply is a raised sheet on a darker frame, and the frame carries a minimal one-line
// watermark footer naming the session on the left and the model on the right (plus a subtle vendor-tinted corner glow).
export function ShareCard({ ref, meta, node, children }: {
  ref?: Ref<HTMLDivElement>;
  meta: ShareMeta;
  node?: HTMLElement;
  children?: ReactNode;
}) {
  useLocale();
  const mountRef = useRef<HTMLDivElement>(null);
  const vendorColor = brandColor(meta.brand);

  useLayoutEffect(() => {
    const slot = mountRef.current;
    if (!slot || !node) return;
    slot.replaceChildren(node);
    return () => { slot.replaceChildren(); };
  }, [node]);

  return (
    <div
      ref={ref}
      data-share-card
      style={vendorColor ? ({ '--share-tint': vendorColor } as CSSProperties) : undefined}
      className="acp-share-card flex w-full min-w-0 flex-col overflow-hidden rounded-lg border border-conversation-line p-gap-half text-fg-1"
    >
      <div
        data-share-body
        className="acp-share-body flex min-w-0 flex-col rounded-md border p-pad"
      >
        {node ? <div ref={mountRef} className="flex min-w-0 flex-col" /> : children}
      </div>
      <div
        data-share-watermark
        className="flex min-w-0 items-center justify-between gap-gap px-pad pt-pad pb-gap text-4 text-fg-2 opacity-85 select-none"
      >
        <div className="flex min-w-0 items-center gap-chip-gap">
          <MarkSvg id="acpira" name="Acpira" className="size-icon shrink-0 translate-y-px text-fg-3" />
          <span className="truncate font-medium text-fg-2">{meta.sessionTitle}</span>
        </div>
        <div className="flex shrink-0 items-center gap-chip-gap">
          {meta.model ? (
            <span className="inline-flex shrink-0 translate-y-px items-center text-fg-2" style={vendorColor ? { color: vendorColor } : undefined}>
              <ModelMark family={meta.model} brand={meta.brand} colored className="size-icon shrink-0" />
            </span>
          ) : (
            <AgentMark id={meta.agentId} name={meta.agentName} className="size-icon shrink-0 translate-y-px text-fg-3" />
          )}
          <span className="font-medium text-fg-2">{meta.model ?? meta.agentName}</span>
        </div>
      </div>
    </div>
  );
}

// Frameless screenshot overlay: only the rendered PNG itself floats over the scrim, scaled to fit whole;
// right-clicking opens the shell's context menu with Copy image and Save image.
export function SharePreviewDialog({ payload, onClose }: {
  payload: SharePayload;
  onClose: () => void;
}) {
  useLocale();
  const layer = useContext(ShellLayerContext);
  const cardRef = useRef<HTMLDivElement>(null);
  const pngCache = useRef<Promise<Blob> | undefined>(undefined);
  const [renderedImg, setRenderedImg] = useState<{ src: string; width: number; height: number }>();

  const ensurePng = useCallback((): Promise<Blob> => {
    if (pngCache.current) return pngCache.current;
    const el = cardRef.current;
    if (!el) return Promise.reject(new Error('Share card is not mounted'));
    const job = renderShareCardToPng(el).catch(err => {
      pngCache.current = undefined;
      throw err;
    });
    pngCache.current = job;
    return job;
  }, []);

  // The card lays out off-stage; only the finished PNG is shown, decoded before it mounts so it lands in one frame
  // instead of swapping a live card for a blank-then-painted image
  useEffect(() => {
    let cancelled = false;
    pngCache.current = undefined;
    setRenderedImg(undefined);
    const raf = requestAnimationFrame(() => {
      void ensurePng()
        .then(blobToDataUrl)
        .then(async src => {
          const probe = new Image();
          probe.src = src;
          await probe.decode().catch(() => {});
          // Measured once the capture is done: video frames and widget snapshots land in the card while it runs
          const el = cardRef.current;
          if (cancelled || !el) return;
          const rect = el.getBoundingClientRect();
          setRenderedImg({ src, width: Math.max(1, el.offsetWidth || Math.ceil(rect.width)), height: Math.max(1, el.offsetHeight || Math.ceil(rect.height)) });
        })
        .catch(() => {});
    });
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
    };
  }, [payload, ensurePng]);

  const onCopy = useCallback(() => {
    if (!navigator.clipboard?.write || typeof ClipboardItem === 'undefined') return;
    const pending = ensurePng();
    void navigator.clipboard.write([new ClipboardItem({ 'image/png': pending })]).catch(() => {});
  }, [ensurePng]);

  const onSave = useCallback(async () => {
    const fileName = shareFileName(payload.meta.sessionTitle);
    try {
      const blob = await ensurePng();
      const saveHost = hostImageSave();
      if (saveHost) {
        const bytes = new Uint8Array(await blob.arrayBuffer());
        await saveHost(bytes, fileName);
        return;
      }
      const dataUrl = renderedImg?.src ?? await blobToDataUrl(blob);
      const link = document.createElement('a');
      link.href = dataUrl;
      link.download = fileName;
      link.click();
    } catch { /* Dismissed or save failed */ }
  }, [ensurePng, payload.meta.sessionTitle, renderedImg]);

  return (
    <Dialog.Root open onOpenChange={open => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Popup
          aria-label={t('share.title')}
          onClick={onClose}
          className="absolute inset-0 z-40 flex items-center justify-center overflow-hidden bg-scrim p-page backdrop-blur-xs"
        >
          {/* Capture source: laid out at the chat column width but never painted */}
          <div aria-hidden className="pointer-events-none invisible absolute inset-0 flex items-center justify-center p-page">
            <div className="w-chat-column max-w-full">
              <ShareCard ref={cardRef} meta={payload.meta} node={payload.node} />
            </div>
          </div>
          <ContextMenu.Root>
            {renderedImg && (
              <ContextMenu.Trigger
                render={
                  <img
                    data-share-dialog
                    data-share-image
                    src={renderedImg.src}
                    width={renderedImg.width}
                    height={renderedImg.height}
                    alt={payload.meta.sessionTitle}
                    draggable={false}
                    onClick={e => e.stopPropagation()}
                    // Shown whole: at most the card's own width, scaled down (ratio kept) when taller than the stage
                    style={{ maxWidth: `min(100%, ${renderedImg.width}px)` }}
                    className="acp-share-stage block h-auto max-h-full w-auto shrink-0 rounded-lg shadow-pop select-none"
                  />
                }
              />
            )}
            <ContextMenu.Portal container={layer?.current ?? undefined}>
              <ContextMenu.Positioner className={positionerClass} collisionAvoidance={contextMenuCollision} collisionBoundary={layer?.current ?? undefined}>
                <ContextMenu.Popup finalFocus={false} className={popupClass} onClick={e => e.stopPropagation()}>
                  <ContextMenu.Item data-part="share-copy" onClick={onCopy} className={optionClass}>
                    <Copy className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                    {t('image.copy')}
                  </ContextMenu.Item>
                  <ContextMenu.Item data-part="share-save" onClick={() => void onSave()} className={optionClass}>
                    <Download className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                    {t('share.saveImage')}
                  </ContextMenu.Item>
                </ContextMenu.Popup>
              </ContextMenu.Positioner>
            </ContextMenu.Portal>
          </ContextMenu.Root>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
