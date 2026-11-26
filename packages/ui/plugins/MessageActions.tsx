import { memo, useRef, useState } from 'react';
import { Check, Copy } from 'lucide-react';
import type { PluginSlot } from '@acpira/shared/plugins';
import { pushShellToast } from '../desktop';
import { t } from '../i18n';
import { callPluginMethod, usePluginPieces } from '../pluginSlots';
import { Prose } from '../markdown/Prose';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { Shimmer } from '../ui/Shimmer';
import { cn } from '../ui/cn';
import { useCopied } from '../ui/useCopied';
import { useScrollFade } from '../ui/useScrollFade';

// The message.actions slot (docs/dev/plugins.md Interface): a plugin's buttons in the row under a finished reply, drawn
// natively (a frame per message would cost a document per turn), labelled with the piece's title. A click opens the
// button's popover, like the stats card beside it, and calls the plugin's runtime method with the session, the reply's
// transcript index and its Markdown. An answer `{ note: { markdown, title } }` fills the popover; any other answer
// closes it (the plugin shows its result its own way: a ui/post to its pieces, a tool card); a failure is a toast.
// Clicking again closes it, and a late answer to a closed popover is dropped (the plugin keeps what it would give again)

// One click of a plugin's native button (message.actions, selection.actions): its runtime method, a failure as a toast.
// Answers the method's result, or undefined when it failed
export async function runPluginAction(p: PluginSlot, params: { session: string; turn?: number; text: string }): Promise<unknown> {
  if (!p.method) return undefined;
  try {
    return await callPluginMethod(p.plugin, p.method, params);
  } catch (e) {
    pushShellToast({ type: 'toast', level: 'error', text: t('plugin.actionFailed', { name: p.title, error: e instanceof Error ? e.message : String(e) }) });
    return undefined;
  }
}

interface PluginNote { title?: string; markdown: string }

// A method's answer as a note: `{ note: { markdown, title? } }`, or `{ note: "<markdown>" }`
function noteOf(result: unknown): PluginNote | undefined {
  const raw = (result as { note?: unknown } | null | undefined)?.note;
  const body = typeof raw === 'string' ? { markdown: raw } : (raw as { markdown?: unknown; title?: unknown } | null | undefined);
  const markdown = typeof body?.markdown === 'string' ? body.markdown.trim() : '';
  if (!markdown) return undefined;
  return { markdown, ...(typeof body?.title === 'string' && body.title.trim() ? { title: body.title.trim() } : {}) };
}

// `onOpenChange`: a popover opened or closed, so the row stays shown while one is open (TurnActions `data-open`)
export const PluginActions = memo(function PluginActions({ session, turn, text, slot = 'message.actions', onOpenChange }: { session: string; turn: number; text: string; slot?: 'message.actions' | 'prompt.actions'; onOpenChange?: (open: boolean) => void }) {
  const pieces = usePluginPieces(slot);
  if (!pieces.length) return null;
  return <>{pieces.map(p => <PluginAction key={`${p.plugin}/${p.id}`} piece={p} session={session} turn={turn} text={text} onOpenChange={onOpenChange} />)}</>;
});

function PluginAction({ piece: p, session, turn, text, onOpenChange }: { piece: PluginSlot; session: string; turn: number; text: string; onOpenChange?: (open: boolean) => void }) {
  const key = `${p.plugin}/${p.id}`;
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState<PluginNote>();
  // Each opening is one call; an answer to an older one (closed since) is dropped
  const call = useRef(0);
  const toggle = (next: boolean) => {
    setOpen(next);
    onOpenChange?.(next);
    const mine = ++call.current;
    if (!next || !p.method) return;
    setNote(undefined);
    void runPluginAction(p, { session, turn, text }).then(result => {
      if (call.current !== mine) return;
      const answered = noteOf(result);
      if (answered) { setNote(answered); return; }
      setOpen(false);
      onOpenChange?.(false);
    });
  };
  return (
    <Popover.Root open={open} onOpenChange={toggle}>
      <Popover.Trigger render={
        <button type="button" data-plugin-action={key} title={p.title} data-open={open || undefined} aria-expanded={open}
          className={cn('inline-flex h-ctl-sm max-w-pop-sm items-center rounded-md px-1.5 text-3 text-fg-2 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 data-[open]:bg-active data-[open]:text-fg-1')}>
          <span className="truncate">{p.title}</span>
        </button>
      } />
      <Popover.Portal><Popover.Positioner side="bottom" align="end" width="xl"><Popover.Popup>
        {open && <NoteCard title={p.title} note={note} />}
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
}

// The stats card's grammar: the button's title on the header (shimmering while the plugin works), the note's own title
// (the model that wrote it, say) dim beside it with copy at the end, the Markdown under it, scrolling inside when long
function NoteCard({ title, note }: { title: string; note?: PluginNote }) {
  const fade = useScrollFade<HTMLDivElement>();
  const copied = useCopied(note?.markdown ?? '');
  const copyLabel = copied.state === 'copied' ? t('code.copied') : copied.state === 'failed' ? t('code.copyFailed') : t('common.copy');
  return (
    <div data-plugin-note className="flex flex-col gap-1 p-1">
      <div className="flex h-ctl items-center gap-3 px-2">
        <Shimmer active={!note} className="shrink-0 text-2 font-medium text-fg-1">{title}</Shimmer>
        {note?.title && <span className="ml-auto min-w-0 truncate text-3 text-fg-3" title={note.title}>{note.title}</span>}
        {note && <IconButton size="sm" className={cn('shrink-0', !note.title && 'ml-auto')} title={copyLabel} aria-label={copyLabel} onClick={copied.copy}>
          {copied.state === 'copied' ? <Check strokeWidth={1.5} /> : <Copy strokeWidth={1.5} />}
        </IconButton>}
      </div>
      {note
        ? <div ref={fade} className="scroll-fade scroll-thin max-h-plan-body overflow-y-auto px-2 pb-1">
            <Prose block={{ type: 'text', markdown: note.markdown }} />
          </div>
        // While the plugin works: placeholder lines in the body, the dock cards' pulse, so the card never reads as empty
        : <div aria-hidden className="flex flex-col gap-2 px-2 pb-2">
            {['w-full', 'w-3/4', 'w-1/2'].map(w => <div key={w} className={cn('h-2 animate-pulse rounded-xs bg-chip motion-reduce:animate-none', w)} />)}
          </div>}
    </div>
  );
}

// The prompt.actions slot: a plugin's buttons in a strip under a sent prompt, called with the prompt's transcript index
// and text. Acpira puts nothing of its own there, so without such a plugin no strip exists and a prompt sits one
// message gap above its reply. With one, the strip stands in for that gap (it cancels the gap on both sides and brings
// its own padding) and shows like a reply's action row: while the prompt above it (`peer/prompt` on HistoryMessage) or
// the strip itself is hovered / focused, and always on touch screens.
// As the exchange's last child there is no gap below to cancel, and a negative margin would push the strip past the
// exchange's paint containment, so `last:mb-0` keeps it inside
export function PromptActions({ session, turn, text }: { session?: string; turn: number; text: string }) {
  const pieces = usePluginPieces('prompt.actions');
  if (!session || !pieces.length) return null;
  return (
    <div data-plugin-slot="prompt.actions" className="-my-msg last:mb-0 box-content flex h-ctl-sm items-center justify-end gap-0.5 py-1 opacity-0 transition-opacity duration-(--code-copy-duration) ease-out peer-hover/prompt:opacity-100 peer-focus-within/prompt:opacity-100 hover:opacity-100 focus-within:opacity-100 [@media(hover:none)]:opacity-100 motion-reduce:transition-none">
      <PluginActions session={session} turn={turn} text={text} slot="prompt.actions" />
    </div>
  );
}
