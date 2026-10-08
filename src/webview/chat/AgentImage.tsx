import { useContext, useState } from 'react';
import { Image as ImageIcon } from 'lucide-react';
import type { ImageRef } from '@shared/transcript';
import { t } from '../i18n';
import { ImageMenu } from './ImageMenu';
import { Lightbox } from './Lightbox';
import { BlobUrlContext, OpenBlobContext, OpenToolFileContext, parseFileLink } from './fileLinks';

// An agent-emitted image (message chunk or tool content): pixels live in the session's blob store, base64 never
// enters the transcript. A click previews it in the shell's Lightbox, like a dropped attachment; opening the file in an
// editor tab is a right-click action next to Copy image, in the transcript and in the Lightbox alike. The agent's `uri`
// (where it saved its copy, e.g. codex-acp) rides along as a small openable caption unless the surrounding row already
// names the file (`caption={false}`).
export function AgentImage({ image, caption = true }: { image: ImageRef; caption?: boolean }) {
  const blobUrl = useContext(BlobUrlContext);
  const openBlob = useContext(OpenBlobContext);
  const openFile = useContext(OpenToolFileContext);
  const [preview, setPreview] = useState(false);
  const src = image.blob && blobUrl ? blobUrl(image.blob) : undefined;
  const file = image.uri ? parseFileLink(image.uri) : undefined;
  const name = file?.path.split(/[\\/]/).pop() ?? t('common.image');
  // Editor-tab opener for the context menu: the blob itself when the host can open it, else the agent's own file
  const openInEditor = image.blob && openBlob ? () => openBlob(image.blob!) : file && openFile ? () => openFile(file.path, file.line) : undefined;
  return (
    <div className="flex min-w-0 flex-col items-start gap-1">
      {src ? (
        <ImageMenu src={src} mimeType={image.mimeType} openInEditor={openInEditor}>
          <button
            type="button"
            title={name}
            aria-label={t('common.previewImage', { name })}
            onClick={() => setPreview(true)}
            className="block max-w-full cursor-pointer overflow-hidden rounded-md border border-line outline-none focus-visible:ring-1 focus-visible:ring-focus"
          >
            <img src={src} alt={name} className="max-h-agent-image w-auto max-w-full object-contain" />
          </button>
        </ImageMenu>
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
      {caption && file && openFile && (
        <button type="button" title={file.path} onClick={() => openFile(file.path, file.line)}
          className="max-w-full truncate text-3 text-fg-3 underline-offset-2 outline-none hover:text-fg-2 hover:underline focus-visible:text-fg-1 focus-visible:underline">
          {image.uri}
        </button>
      )}
      {preview && src && <Lightbox src={src} name={name} mimeType={image.mimeType} openInEditor={openInEditor} onClose={() => setPreview(false)} />}
    </div>
  );
}
