import { useState } from 'react';
import { CodeXml, Eye, X } from 'lucide-react';
import { t } from '../i18n';
import { Dialog } from '../ui/Dialog';
import { IconButton } from '../ui/Button';
import { Prose } from './Prose';
import { looksLikeMarkdown } from './markdownGuess';

// .md names render unconditionally; .txt & friends earn it through content
const MD_NAME = /\.(md|markdown|mdx|mdown)$/i;

// In-shell peek at a text attachment, the image Lightbox's twin: backdrop click / Esc / the corner button close it.
// Markdown-looking text renders through Prose (the same pipeline as agent messages); the header toggle falls back to raw text.
// The name rides a header bar, the content a scrollable body; `text` still absent means the blob read is in flight
export function TextPeek({ name, text, failed, onClose }: { name: string; text?: string; failed?: boolean; onClose: () => void }) {
  const [mode, setMode] = useState<'auto' | 'raw' | 'rendered'>('auto');
  const markdown = text != null && (MD_NAME.test(name) || looksLikeMarkdown(text));
  const rendered = mode === 'rendered' || (mode === 'auto' && markdown);
  return <Dialog.Root open onOpenChange={open => { if (!open) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Popup aria-label={name} onClick={onClose} className="absolute inset-0 z-40 flex items-center justify-center bg-scrim p-pad">
        <div
          onClick={e => e.stopPropagation()}
          className="flex max-h-full w-full max-w-(--content-w) flex-col overflow-hidden rounded-md border border-line bg-bg-1 shadow-pop"
        >
          <div className="flex shrink-0 items-center gap-2 border-b border-line py-1 pr-1 pl-pad">
            <span className="min-w-0 flex-1 truncate text-3 font-medium text-fg-1">{name}</span>
            {text != null && <IconButton size="sm" aria-pressed={rendered}
              title={rendered ? t('attach.viewSource') : t('attach.renderMarkdown')}
              aria-label={rendered ? t('attach.viewSource') : t('attach.renderMarkdown')}
              onClick={() => setMode(rendered ? 'raw' : 'rendered')}>
              {rendered ? <CodeXml strokeWidth={1.5} /> : <Eye strokeWidth={1.5} />}
            </IconButton>}
            <IconButton size="sm" title={t('common.close')} aria-label={t('common.close')} onClick={onClose}><X strokeWidth={1.5} /></IconButton>
          </div>
          {rendered
            ? <div className="scroll-thin min-h-0 flex-1 overflow-auto p-pad"><Prose block={{ type: 'text', markdown: text! }} /></div>
            : <pre className="scroll-thin m-0 min-h-0 flex-1 overflow-auto p-pad font-mono text-mono whitespace-pre-wrap [overflow-wrap:anywhere] text-fg-1">
              {text ?? (failed ? t('attach.loadFailed', { name }) : t('attach.loading'))}
            </pre>}
        </div>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>;
}
