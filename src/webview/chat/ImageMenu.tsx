import { useCallback, useContext, type ReactElement } from 'react';
import { Copy, ExternalLink } from 'lucide-react';
import { ContextMenu } from '@base-ui/react/context-menu';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { ShellLayerContext, overlayWidth, popupClass } from '../ui/Overlay';
import { optionClass } from '../ui/DropdownMenu';
import { useCopyAction } from '../ui/useCopied';
import { copyImage } from './copyImage';

// The shell's own right-click menu for an image (Copy image, Open in editor), replacing the webview's native
// Cut / Copy / Paste. Shared by transcript images and the Lightbox, so a previewed image keeps the same actions.
// The child element itself becomes the trigger (no wrapper), so it keeps its own sizing inside the Lightbox.
// `mimeType` may be omitted when unknown: copyImage then trusts the fetched bytes' own type.
export function ImageMenu({ src, mimeType = '', openInEditor, children }: {
  src: string;
  mimeType?: string;
  openInEditor?: () => void;
  children: ReactElement<Record<string, unknown>>;
}) {
  const layer = useContext(ShellLayerContext);
  const writeImage = useCallback(() => copyImage(src, mimeType), [src, mimeType]);
  const { state: copyState, copy } = useCopyAction(src, writeImage);
  return (
    <>
      <ContextMenu.Root>
        <ContextMenu.Trigger render={children} />
        <ContextMenu.Portal container={layer?.current ?? undefined}>
          <ContextMenu.Positioner className={cn('z-40', overlayWidth.sm)} collisionBoundary={layer?.current ?? undefined}>
            {/* Portaled, but React events still bubble up the component tree: a menu click must not reach the
                Lightbox's backdrop handler and close the preview */}
            <ContextMenu.Popup className={popupClass} onClick={e => e.stopPropagation()}>
              <ContextMenu.Item onClick={() => void copy()} className={optionClass}>
                <Copy className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                {t('image.copy')}
              </ContextMenu.Item>
              {openInEditor && (
                <ContextMenu.Item onClick={openInEditor} className={optionClass}>
                  <ExternalLink className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                  {t('image.openInEditor')}
                </ContextMenu.Item>
              )}
            </ContextMenu.Popup>
          </ContextMenu.Positioner>
        </ContextMenu.Portal>
      </ContextMenu.Root>
      {/* Copy feedback is for screen readers only: a visible line under the image would push the transcript down */}
      <span className="sr-only" role="status">{copyState === 'idle' ? '' : t(copyState === 'copied' ? 'code.copied' : 'code.copyFailed')}</span>
    </>
  );
}
