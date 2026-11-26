import { useMemo, useState } from 'react';
import { CodeXml, Eye, X } from 'lucide-react';
import { t } from '../i18n';
import { Dialog } from '../ui/Dialog';
import { IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { MarkdownParts, StagedParts } from '../panel/FileViewer';
import { CODE_CARD, CODE_INSET } from './CodeBlock';
import { looksLikeMarkdown } from './markdownGuess';

// .md names render unconditionally; plain-text names (pasted text, a .txt, a label with no extension) earn it through
// content; any other extension (.json, .log, source) stays raw: a JSON request dump with Markdown in its strings passed
// the content guess and went through the Markdown pipeline whole, holding the window for good
const MD_NAME = /\.(md|markdown|mdx|mdown)$/i;
const EXT = /\.([a-z0-9]+)$/i;
const PLAIN_EXT = new Set(['txt', 'text']);
// The content guess reads the head only: its regexes cost tens of ms on a megabyte
const SNIFF_MAX = 64 * 1024;
// Text that is not Markdown by name or content may still be rendered on request up to this size; past it the toggle
// is gone, since text with no blank line to split at (pretty JSON) parses whole in one frame
const RENDER_ANY_MAX = 64 * 1024;
// Raw text mounts in parts of about this many characters, cut at line breaks. One line longer than that stays whole up
// to RAW_LINE_MAX (copying across a cut gains a line break), past it it is cut there
const RAW_PART = 32 * 1024;
const RAW_LINE_MAX = 4 * RAW_PART;

function isMarkdown(name: string, text: string): boolean {
  if (MD_NAME.test(name)) return true;
  const ext = EXT.exec(name)?.[1]?.toLowerCase();
  if (ext && !PLAIN_EXT.has(ext)) return false;
  return looksLikeMarkdown(text.length > SNIFF_MAX ? text.slice(0, SNIFF_MAX) : text);
}

// Raw text split for StagedParts. A part boundary stands for the line break it was cut at (each part is its own block),
// so the break itself is dropped and copying across parts still gives the text back
export function rawParts(text: string): string[] {
  const parts: string[] = [];
  let start = 0;
  while (text.length - start > RAW_PART) {
    let end = text.lastIndexOf('\n', start + RAW_PART);
    if (end <= start) {
      const next = text.indexOf('\n', start + 1);
      end = next !== -1 && next - start <= RAW_LINE_MAX ? next : start + RAW_LINE_MAX;
      if (end >= text.length) break;
      // A cut inside a line never splits a surrogate pair
      const lead = text.charCodeAt(end - 1);
      if (text[end] !== '\n' && lead >= 0xd800 && lead <= 0xdbff) end--;
    }
    const breaksHere = text[end] === '\n';
    parts.push(text.slice(start, breaksHere && text[end - 1] === '\r' ? end - 1 : end));
    start = breaksHere ? end + 1 : end;
  }
  parts.push(text.slice(start));
  return parts;
}

const RAW = 'm-0 font-mono text-mono whitespace-pre-wrap [overflow-wrap:anywhere] text-fg-1';
const rawOf = (text: string) => <pre className={RAW}>{text}</pre>;

// In-shell peek at a text attachment, the image Lightbox's twin: backdrop click / Esc / the corner button close it.
// Markdown-looking text renders through Prose (the same pipeline as agent messages); the header toggle falls back to raw text.
// The name rides a header bar, the content a scrollable body; `text` still absent means the blob read is in flight.
// Attachments run up to a megabyte, so both views mount in parts (StagedParts) rather than laying the whole out at once
// `comment`: the remark sent with shared context, above the text like a quote's
export function TextPeek({ name, text, failed, comment, onClose }: { name: string; text?: string; failed?: boolean; comment?: string; onClose: () => void }) {
  const [mode, setMode] = useState<'auto' | 'raw' | 'rendered'>('auto');
  const markdown = useMemo(() => text != null && isMarkdown(name, text), [name, text]);
  const canRender = text != null && (markdown || text.length <= RENDER_ANY_MAX);
  const rendered = canRender && (mode === 'rendered' || (mode === 'auto' && markdown));
  const parts = useMemo(() => (text != null && !rendered ? rawParts(text) : undefined), [text, rendered]);
  return <Dialog.Root open onOpenChange={open => { if (!open) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Popup aria-label={name} onScrimClick={onClose} className="absolute inset-0 z-40 flex items-center justify-center bg-scrim p-pad">
        <div
          className="flex max-h-full w-full max-w-(--content-w) flex-col overflow-hidden rounded-md border border-line bg-bg-1 shadow-pop"
        >
          <div className="flex shrink-0 items-center gap-2 border-b border-line py-1 pr-1 pl-pad">
            <span className="min-w-0 flex-1 truncate text-3 font-medium text-fg-1">{name}</span>
            {canRender && <IconButton size="sm" aria-pressed={rendered}
              title={rendered ? t('attach.viewSource') : t('attach.renderMarkdown')}
              aria-label={rendered ? t('attach.viewSource') : t('attach.renderMarkdown')}
              onClick={() => setMode(rendered ? 'raw' : 'rendered')}>
              {rendered ? <CodeXml strokeWidth={1.5} /> : <Eye strokeWidth={1.5} />}
            </IconButton>}
            <IconButton size="sm" title={t('common.close')} aria-label={t('common.close')} onClick={onClose}><X strokeWidth={1.5} /></IconButton>
          </div>
          {/* The comment and the text share one column; the comment is a card cut like the code card below it, so the
              label and remark start where the code does */}
          <div className="scroll-thin flex min-h-0 flex-1 flex-col gap-gap overflow-auto p-pad">
            {comment && (
              <div className={cn('flex shrink-0 flex-col gap-0.5', CODE_CARD, CODE_INSET)}>
                <span className="text-3 text-fg-3">{t('quote.comment')}</span>
                <p className="m-0 text-2 whitespace-pre-wrap break-words text-fg-1">{comment}</p>
              </div>
            )}
            {text == null
              ? <pre className={RAW}>{failed ? t('attach.loadFailed', { name }) : t('attach.loading')}</pre>
              : rendered ? <MarkdownParts text={text} />
              : <StagedParts parts={parts!} render={rawOf} className="min-w-0" />}
          </div>
        </div>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>;
}
