import { useContext, useState, type ReactNode } from 'react';
import { FileText, Image as ImageIcon, X } from 'lucide-react';
import type { Attachment, Draft } from '@shared/transcript';
import { imageMimeOf } from '@shared/attachments';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { Lightbox } from './Lightbox';
import { TextPeek } from './TextPeek';
import { OpenToolFileContext, parseFileLink } from './fileLinks';

// An image open in the Lightbox: the source to show and the name for labels
interface Preview {
  src: string;
  name?: string;
}

// A text chip opened for reading; `url` doubles as the race token — a blob answer lands only while its peek is still the one on screen
interface Peek {
  name: string;
  url?: string;
  text?: string;
  failed?: boolean;
}

// Blob names are content hashes, so a fetched text never changes: reads are cached, failures are not
const blobTexts = new Map<string, Promise<string>>();
function readBlobText(url: string): Promise<string> {
  let pending = blobTexts.get(url);
  if (!pending) {
    pending = fetch(url).then(r => { if (!r.ok) throw new Error(String(r.status)); return r.text(); });
    pending.catch(() => blobTexts.delete(url));
    blobTexts.set(url, pending);
  }
  return pending;
}

// Shared open-a-text-attachment state for the chip rows below: `open` takes a draft's in-memory text, `openBlob` fetches a staged one
function useTextPeek() {
  const [peek, setPeek] = useState<Peek | null>(null);
  const open = (name: string, text: string) => setPeek({ name, text });
  const openBlob = (name: string, url: string) => {
    setPeek({ name, url });
    readBlobText(url).then(
      text => setPeek(p => (p?.url === url ? { name, url, text } : p)),
      () => setPeek(p => (p?.url === url ? { name, url, failed: true } : p)),
    );
  };
  const card = peek && <TextPeek name={peek.name} text={peek.text} failed={peek.failed} onClose={() => setPeek(null)} />;
  return { open, openBlob, card };
}

// What a chip click does, by kind: a text draft shows its in-memory text, a staged text blob is fetched into the peek,
// a file opens in the editor (`openFile` only exists inside the thread context, so composer's file chips stay inert on their own).
// Images never come through here — they keep the Lightbox on `onPreview`
function openFor(a: Draft | Attachment, ctx: {
  blobUrl?: (blob: string) => string;
  openFile?: (path: string, line?: number) => void;
  peek: { open: (name: string, text: string) => void; openBlob: (name: string, url: string) => void };
}): (() => void) | undefined {
  if (a.kind === 'text') {
    if ('text' in a) return () => ctx.peek.open(a.name, a.text);
    const url = a.blob && ctx.blobUrl ? ctx.blobUrl(a.blob) : undefined;
    return url ? () => ctx.peek.openBlob(a.name, url) : undefined;
  }
  if (a.kind === 'file' && ctx.openFile) {
    const file = parseFileLink(a.uri);
    const open = ctx.openFile;
    if (file) return () => open(file.path, file.line);
  }
}

// Composer images use individual thumbnails; other attachments keep compact file labels.
// An inline editor's retained attachments (`before`) share this one wrapping row, so a newly pasted image lands beside them instead of on a second row.
export function DraftChips({ drafts, before, onRemove }: { drafts: Draft[]; before?: ReactNode; onRemove?: (index: number) => void }) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const peek = useTextPeek();
  const openFile = useContext(OpenToolFileContext);
  if (!drafts.length && !before) return null;
  return (
    <div className="flex flex-wrap items-start gap-gap px-pad pt-gap">
      {before}
      {drafts.map((d, i) => (
        <Removable key={d.kind === 'file' ? d.uri : `${d.name ?? d.kind}-${i}`} label={t('common.removeNamed', { name: d.name ?? t('common.image') })} onRemove={onRemove ? () => onRemove(i) : undefined}>
          <AttachmentTag
            thumbnail
            name={d.name}
            image={d.kind === 'image' || (d.kind === 'file' && !!imageMimeOf(d.name))}
            src={d.kind === 'image' ? `data:${d.mimeType};base64,${d.data}` : undefined}
            onPreview={src => setPreview({ src, name: d.name })}
            onOpen={openFor(d, { openFile, peek })}
          />
        </Removable>
      ))}
      {preview && <Lightbox src={preview.src} name={preview.name} onClose={() => setPreview(null)} />}
      {peek.card}
    </div>
  );
}

// Sent images reuse the editor thumbnails and wrap to the available width.
// Names remain in tooltips; non-image attachments keep their file labels.
export function TurnAttachments({ attachments, blobUrl }: { attachments: Attachment[]; blobUrl?: (blob: string) => string }) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const peek = useTextPeek();
  const openFile = useContext(OpenToolFileContext);
  return (
    <div className="flex shrink-0 flex-wrap items-start gap-gap">
      {attachments.map((a, i) => (
        <AttachmentTag
          key={a.kind === 'file' ? a.uri : a.blob ?? `${a.kind}-${i}`}
          thumbnail
          name={a.name}
          image={a.kind === 'image' || (a.kind === 'file' && !!imageMimeOf(a.name))}
          src={a.kind === 'image' && blobUrl && a.blob ? blobUrl(a.blob) : undefined}
          title={a.kind === 'file' ? a.uri : undefined}
          onPreview={src => setPreview({ src, name: a.name })}
          onOpen={openFor(a, { blobUrl, openFile, peek })}
        />
      ))}
      {preview && <Lightbox src={preview.src} name={preview.name} onClose={() => setPreview(null)} />}
      {peek.card}
    </div>
  );
}

// Attachments of a queued prompt, inline before its text: images as square tiles (the "little grid" Cursor shows), the rest as the usual pill
export function AttachmentTiles({ attachments, blobUrl }: { attachments: Attachment[]; blobUrl?: (blob: string) => string }) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const peek = useTextPeek();
  return (
    <span className="flex shrink-0 self-center items-center gap-1">
      {attachments.map((a, i) => {
        const key = a.kind === 'file' ? a.uri : a.blob ?? `${a.kind}-${i}`;
        const src = a.kind === 'image' && blobUrl && a.blob ? blobUrl(a.blob) : undefined;
        return src
          ? <button key={key} type="button" title={a.name} aria-label={t('common.previewImage', { name: a.name ?? t('common.image') })} onClick={() => setPreview({ src, name: a.name })}
              className="flex size-lead shrink-0 cursor-zoom-in overflow-hidden rounded-xs outline-none hover:ring-1 hover:ring-fg-3 focus-visible:ring-1 focus-visible:ring-focus">
              <img src={src} alt="" className="size-full object-cover" />
            </button>
          : <AttachmentTag key={key} name={a.name} image={a.kind === 'image' || (a.kind === 'file' && !!imageMimeOf(a.name))} title={a.kind === 'file' ? a.uri : undefined}
              onOpen={openFor(a, { blobUrl, peek })} />;
      })}
      {preview && <Lightbox src={preview.src} name={preview.name} onClose={() => setPreview(null)} />}
      {peek.card}
    </span>
  );
}

// Retained attachments share the draft thumbnails, including the corner removal button.
// Renders bare items: the Composer places them at the head of the `DraftChips` row, so kept and new attachments wrap together.
export function EditAttachments({ attachments, retained, blobUrl, disabled, onRemove }: {
  attachments: Attachment[]; retained: number[]; blobUrl?: (blob: string) => string; disabled?: boolean; onRemove: (index: number) => void;
}) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const peek = useTextPeek();
  const openFile = useContext(OpenToolFileContext);
  if (!retained.length) return null;
  return (
    <>
      {retained.map(i => {
        const attachment = attachments[i]!;
        const src = attachment.kind === 'image' && blobUrl && attachment.blob ? blobUrl(attachment.blob) : undefined;
        return <Removable key={i} disabled={disabled} label={t('common.removeNamed', { name: attachment.name ?? t('common.image') })} onRemove={() => onRemove(i)}>
          <AttachmentTag thumbnail name={attachment.name} src={src}
            image={attachment.kind === 'image' || (attachment.kind === 'file' && !!imageMimeOf(attachment.name))}
            title={attachment.kind === 'file' ? attachment.uri : undefined}
            onPreview={src => setPreview({ src, name: attachment.name })}
            onOpen={openFor(attachment, { blobUrl, openFile, peek })} />
        </Removable>;
      })}
      {preview && <Lightbox src={preview.src} name={preview.name} onClose={() => setPreview(null)} />}
      {peek.card}
    </>
  );
}

function AttachmentTag({ name = 'image.png', src, image, title, thumbnail, onPreview, onOpen }: {
  name?: string;
  src?: string;
  image: boolean;
  title?: string;
  thumbnail?: boolean;
  onPreview?: (src: string) => void;
  onOpen?: () => void;
}) {
  const click = src && onPreview ? () => onPreview(src) : onOpen;
  const Tag = click ? 'button' : 'span';
  return (
    <Tag
      type={click ? 'button' : undefined}
      title={title ?? name}
      aria-label={src ? t('common.previewImage', { name }) : click ? t('attach.view', { name }) : undefined}
      onClick={click}
      className={cn(
        'inline-flex max-w-full min-w-0 shrink-0 items-center rounded-sm bg-chip text-fg-2 [&_svg]:size-icon [&_svg]:shrink-0 [&_svg]:text-fg-3',
        thumbnail && image ? 'size-thumb justify-center overflow-hidden' : 'h-ctl-sm gap-1 px-2 text-3 font-medium',
        click && 'outline-none hover:bg-chip-hover focus-visible:ring-1 focus-visible:ring-focus active:bg-active',
        src ? 'cursor-zoom-in' : click && 'cursor-pointer',
      )}
    >
      {src
        ? <img src={src} alt="" className={thumbnail && image ? 'size-full object-cover' : 'size-icon-ctl shrink-0 rounded-xs object-cover'} />
        : image ? <ImageIcon strokeWidth={1.5} /> : <FileText strokeWidth={1.5} />}
      {!(thumbnail && image) && <span className="truncate">{name}</span>}
    </Tag>
  );
}

// Wraps a chip with a remove button riding its top-right corner — a badge over the edge, so it never lands on the
// label (and keeps the thumbnail's corner visible too). Shown on hover / focus, inert while hidden.
function Removable({ label, onRemove, disabled, children }: { label: string; onRemove?: () => void; disabled?: boolean; children: ReactNode }) {
  return (
    <span className="group/chip relative inline-flex max-w-full min-w-0">
      {children}
      {onRemove && <button
        type="button"
        disabled={disabled}
        aria-label={label}
        title={t('common.remove')}
        onClick={onRemove}
        className={cn(
          'pointer-events-none absolute -top-2 -right-2 flex size-icon-ctl items-center justify-center rounded-full border border-line-strong bg-bg-2 text-fg-2 opacity-0 transition-opacity',
          'group-hover/chip:pointer-events-auto group-hover/chip:opacity-100 group-focus-within/chip:pointer-events-auto group-focus-within/chip:opacity-100 focus-visible:pointer-events-auto focus-visible:opacity-100',
          'hover:bg-bg-1 hover:text-fg-1',
        )}
      >
        <X className="size-3" strokeWidth={2} />
      </button>}
    </span>
  );
}
