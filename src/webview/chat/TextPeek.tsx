import { X } from 'lucide-react';
import { t } from '../i18n';
import { Dialog } from '../ui/Dialog';
import { IconButton } from '../ui/Button';

// In-shell peek at a text attachment, the image Lightbox's twin: backdrop click / Esc / the corner button close it.
// The name rides a header bar, the content a scrollable mono body; `text` still absent means the blob read is in flight
export function TextPeek({ name, text, failed, onClose }: { name: string; text?: string; failed?: boolean; onClose: () => void }) {
  return <Dialog.Root open onOpenChange={open => { if (!open) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Popup aria-label={name} onClick={onClose} className="absolute inset-0 z-40 flex items-center justify-center bg-scrim p-pad">
        <div
          onClick={e => e.stopPropagation()}
          className="flex max-h-full w-full max-w-(--content-w) flex-col overflow-hidden rounded-md border border-line bg-bg-1 shadow-pop"
        >
          <div className="flex shrink-0 items-center gap-2 border-b border-line py-1 pr-1 pl-pad">
            <span className="min-w-0 flex-1 truncate text-3 font-medium text-fg-1">{name}</span>
            <IconButton size="sm" title={t('common.close')} aria-label={t('common.close')} onClick={onClose}><X strokeWidth={1.5} /></IconButton>
          </div>
          <pre className="scroll-thin m-0 min-h-0 flex-1 overflow-auto p-pad font-mono text-mono whitespace-pre-wrap [overflow-wrap:anywhere] text-fg-1">
            {text ?? (failed ? t('attach.loadFailed', { name }) : t('attach.loading'))}
          </pre>
        </div>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>;
}
