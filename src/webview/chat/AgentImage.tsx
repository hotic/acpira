import { useCallback, useContext, useState } from 'react';
import { Copy, Image as ImageIcon } from 'lucide-react';
import { ContextMenu } from '@base-ui/react/context-menu';
import type { ImageRef } from '@shared/transcript';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { ShellLayerContext, overlayWidth, popupClass } from '../ui/Overlay';
import { optionClass } from '../ui/DropdownMenu';
import { useCopyAction } from '../ui/useCopied';
import { copyImage } from './copyImage';
import { Lightbox } from './Lightbox';
import { BlobUrlContext, OpenBlobContext, OpenToolFileContext, parseFileLink } from './fileLinks';

// An agent-emitted image (message chunk or tool content): pixels live in the session's blob store, base64 never
// enters the transcript. A click opens the file in the editor; without a host opener the Lightbox is the fallback.
// The agent's `uri` (where it saved its copy, e.g. codex-acp) rides along as a small openable caption.
export function AgentImage({ image }: { image: ImageRef }) {
  const blobUrl = useContext(BlobUrlContext);
  const openBlob = useContext(OpenBlobContext);
  const openFile = useContext(OpenToolFileContext);
  const layer = useContext(ShellLayerContext);
  const [preview, setPreview] = useState(false);
  const src = image.blob && blobUrl ? blobUrl(image.blob) : undefined;
  const file = image.uri ? parseFileLink(image.uri) : undefined;
  const name = file?.path.split(/[\\/]/).pop() ?? t('common.image');
  const open = image.blob && openBlob ? () => openBlob(image.blob!) : src ? () => setPreview(true) : undefined;
  const writeImage = useCallback(() => (src ? copyImage(src, image.mimeType) : Promise.reject(new Error('No image pixels'))), [src, image.mimeType]);
  const { state: copyState, copy } = useCopyAction(src, writeImage);
  return (
    <div className="flex min-w-0 flex-col items-start gap-1">
      {src ? (
        <ContextMenu.Root>
          <ContextMenu.Trigger className="max-w-full">
            <button
              type="button"
              title={name}
              aria-label={t('attach.view', { name })}
              onClick={open}
              disabled={!open}
              className={cn('block max-w-full overflow-hidden rounded-md border border-line outline-none focus-visible:ring-1 focus-visible:ring-focus', open && 'cursor-pointer')}
            >
              <img src={src} alt={name} className="max-h-agent-image w-auto max-w-full object-contain" />
            </button>
          </ContextMenu.Trigger>
          <ContextMenu.Portal container={layer?.current ?? undefined}>
            <ContextMenu.Positioner className={cn('z-40', overlayWidth.sm)} collisionBoundary={layer?.current ?? undefined}>
              <ContextMenu.Popup className={popupClass}>
                <ContextMenu.Item onClick={() => void copy()} className={optionClass}>
                  <Copy className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                  {t('image.copy')}
                </ContextMenu.Item>
              </ContextMenu.Popup>
            </ContextMenu.Positioner>
          </ContextMenu.Portal>
        </ContextMenu.Root>
      ) : (
        // No saved pixels (a refused payload or a URI-only reference): the card still names the source
        <button
          type="button"
          title={image.uri ?? name}
          aria-label={t('attach.view', { name })}
          onClick={file && openFile ? () => openFile(file.path, file.line) : undefined}
          disabled={!(file && openFile)}
          className="inline-flex h-ctl-sm max-w-full min-w-0 items-center gap-1 rounded-sm bg-chip px-2 text-3 font-medium text-fg-2 disabled:cursor-default enabled:cursor-pointer enabled:hover:bg-chip-hover [&_svg]:size-icon [&_svg]:shrink-0 [&_svg]:text-fg-3"
        >
          <ImageIcon strokeWidth={1.5} />
          <span className="truncate">{name}</span>
        </button>
      )}
      {copyState !== 'idle' && <span role="status" className="text-3 text-fg-3">{t(copyState === 'copied' ? 'code.copied' : 'code.copyFailed')}</span>}
      {file && openFile && (
        <button type="button" title={file.path} onClick={() => openFile(file.path, file.line)}
          className="max-w-full truncate text-3 text-fg-3 underline-offset-2 outline-none hover:text-fg-2 hover:underline focus-visible:text-fg-1 focus-visible:underline">
          {image.uri}
        </button>
      )}
      {preview && src && <Lightbox src={src} name={name} onClose={() => setPreview(false)} />}
    </div>
  );
}
