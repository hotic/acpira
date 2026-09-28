import { useState, type KeyboardEvent } from 'react';
import { MessageSquareText, Pencil, Trash2 } from 'lucide-react';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';

export interface QuoteItem {
  text: string;
  comment?: string;
}

// Quotes picked in the transcript collapse into one "N annotations" chip (as Codex shows them); hovering lists each quote with its remark.
// With handlers the list is editable (composer), without it is a read-only receipt (a sent turn)
export function QuoteChip({ quotes, onEdit, onRemove }: {
  quotes: QuoteItem[];
  onEdit?: (index: number, comment: string) => void;
  onRemove?: (index: number) => void;
}) {
  const [open, setOpen] = useState(false);
  // An edit in progress keeps the card open even when the pointer wanders off it
  const [editing, setEditing] = useState<number>();
  const label = quotes.length === 1 ? t('quote.chipOne') : t('quote.chip', { n: quotes.length });
  return (
    <Popover.Root open={open || editing !== undefined} onOpenChange={next => { setOpen(next); if (!next) setEditing(undefined); }}>
      <Popover.Trigger openOnHover delay={120} closeDelay={250}
        render={<button type="button" data-open={open || undefined}
          className={cn(
            'inline-flex h-ctl-sm max-w-full min-w-0 shrink-0 items-center gap-1 rounded-sm bg-chip px-2 text-3 font-medium text-fg-2 outline-none',
            'hover:bg-chip-hover focus-visible:ring-1 focus-visible:ring-focus data-[open]:bg-chip-hover [&_svg]:size-icon [&_svg]:shrink-0 [&_svg]:text-fg-3',
          )}>
          <MessageSquareText strokeWidth={1.5} />
          <span className="truncate">{label}</span>
        </button>} />
      <Popover.Portal><Popover.Positioner side="top" align="start" width="xl"><Popover.Popup>
        <ol className="scroll-thin flex max-h-pop flex-col gap-3 overflow-y-auto py-1">
          {quotes.map((q, i) => (
            <QuoteEntry key={i} index={i} quote={q} editing={editing === i}
              onStartEdit={onEdit && (() => setEditing(i))}
              onSave={onEdit && (comment => { onEdit(i, comment); setEditing(undefined); })}
              onCancel={() => setEditing(undefined)}
              onRemove={onRemove && (() => { onRemove(i); setEditing(undefined); if (quotes.length === 1) setOpen(false); })} />
          ))}
        </ol>
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
}

function QuoteEntry({ index, quote, editing, onStartEdit, onSave, onCancel, onRemove }: {
  index: number;
  quote: QuoteItem;
  editing: boolean;
  onStartEdit?: () => void;
  onSave?: (comment: string) => void;
  onCancel: () => void;
  onRemove?: () => void;
}) {
  return (
    // Codex-style entry: muted index and captions, softer body text, no per-row hover fill so the card reads as one list
    <li className="group/quote flex gap-gap px-2">
      <span className="w-4 shrink-0 text-3 text-fg-3 tabular-nums">{index + 1}.</span>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-3 text-fg-3">{t('quote.selected')}</span>
        <p className="line-clamp-5 text-2 whitespace-pre-wrap break-words text-fg-1">{quote.text}</p>
        {editing && onSave
          ? <CommentEditor className="mt-1.5" initial={quote.comment ?? ''} onSave={onSave} onCancel={onCancel} />
          : quote.comment && <>
              <span className="mt-2 text-3 text-fg-3">{t('quote.comment')}</span>
              <p className="text-2 whitespace-pre-wrap break-words text-fg-1">{quote.comment}</p>
            </>}
      </div>
      {(onStartEdit || onRemove) && !editing && (
        <div className="flex shrink-0 items-start gap-0.5 opacity-0 transition-opacity group-hover/quote:opacity-100 group-focus-within/quote:opacity-100">
          {onStartEdit && <IconButton size="sm" title={t('quote.edit')} aria-label={t('quote.edit')} onClick={onStartEdit}><Pencil /></IconButton>}
          {onRemove && <IconButton size="sm" title={t('quote.remove')} aria-label={t('quote.remove')} onClick={onRemove}><Trash2 /></IconButton>}
        </div>
      )}
    </li>
  );
}

// One-line remark field shared by the card and the transcript toolbar: Enter saves, Shift+Enter breaks the line, Escape leaves it as it was.
// No focus ring: the field only exists while it is being typed in, so the caret is focus enough
export function CommentEditor({ initial = '', autoFocus = true, className, onSave, onCancel }: {
  initial?: string;
  autoFocus?: boolean;
  className?: string;
  onSave: (comment: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); onSave(value.trim()); }
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onCancel(); }
  };
  return (
    <textarea
      rows={1}
      autoFocus={autoFocus}
      value={value}
      placeholder={t('quote.commentPlaceholder')}
      aria-label={t('quote.comment')}
      onChange={e => setValue(e.target.value)}
      onKeyDown={onKeyDown}
      className={cn('field-sizing-content max-h-[calc(6*var(--text-2-lh))] min-w-0 resize-none rounded-sm bg-bg-0 px-2 py-1 text-2 text-fg-strong outline-none placeholder:text-fg-3', className)}
    />
  );
}
