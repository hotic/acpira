import { useEffect, useRef, useState, type RefObject } from 'react';
import { MessageSquarePlus } from 'lucide-react';
import type { Draft } from '@shared/transcript';
import { t } from '../i18n';
import { Popover } from '../ui/Popover';
import { CommentEditor } from './Quotes';
import { updateMainComposer } from './useComposerDraft';

type QuoteDraft = Extract<Draft, { kind: 'quote' }>;

interface Picked {
  range: Range;
  text: string;
}

// The quote stays painted while focus sits in the comment field (the document selection moves there): a CSS custom highlight,
// styled like ::selection in base.css. Engines without the Highlight API simply lose the paint
const HIGHLIGHT = 'acpira-quote';
function paint(range?: Range) {
  const registry = (globalThis.CSS as { highlights?: Map<string, unknown> } | undefined)?.highlights;
  const Highlight = (globalThis as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight;
  if (!registry || !Highlight) return;
  if (range) registry.set(HIGHLIGHT, new Highlight(range)); else registry.delete(HIGHLIGHT);
}

// Text selected inside the transcript
function pickIn(root: HTMLElement | null): Picked | undefined {
  const sel = document.getSelection();
  if (!root || !sel || sel.isCollapsed || sel.rangeCount === 0) return undefined;
  const range = sel.getRangeAt(0);
  if (!root.contains(range.commonAncestorContainer)) return undefined;
  const text = sel.toString().trim();
  return text ? { range: range.cloneRange(), text } : undefined;
}

// Selecting text in the transcript raises a small toolbar over it (Codex's "Add to chat"): the quote goes into the main composer
// as an annotation at once, and the toolbar turns into an optional comment field for it. Enter keeps the comment, Escape / clicking
// away leaves the quote without one. `root` is the transcript content; selections elsewhere (composer, cards) are ignored
export function QuoteToolbar({ root }: { root: RefObject<HTMLElement | null> }) {
  const [picked, setPicked] = useState<Picked>();
  const [added, setAdded] = useState<QuoteDraft>();
  const addedRef = useRef(added);
  addedRef.current = added;

  useEffect(() => {
    // Shown only once the pointer / keyboard gesture ends, so the toolbar never chases a drag in progress
    let timer = 0;
    const settle = () => {
      window.clearTimeout(timer);
      // After the click that ends a double-click word selection has landed
      timer = window.setTimeout(() => { if (!addedRef.current) setPicked(pickIn(root.current)); });
    };
    const onPointerDown = (e: PointerEvent) => {
      if ((e.target as Element | null)?.closest?.('[data-quote-toolbar]')) return;
      if (!addedRef.current) setPicked(undefined);
    };
    const onKeyUp = (e: KeyboardEvent) => { if (e.shiftKey || e.key === 'Shift' || ((e.metaKey || e.ctrlKey) && e.key === 'a')) settle(); };
    const onSelectionChange = () => { if (!addedRef.current && document.getSelection()?.isCollapsed) setPicked(undefined); };
    document.addEventListener('pointerdown', onPointerDown, true);
    document.addEventListener('pointerup', settle);
    document.addEventListener('keyup', onKeyUp);
    document.addEventListener('selectionchange', onSelectionChange);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener('pointerdown', onPointerDown, true);
      document.removeEventListener('pointerup', settle);
      document.removeEventListener('keyup', onKeyUp);
      document.removeEventListener('selectionchange', onSelectionChange);
    };
  }, [root]);

  useEffect(() => () => paint(), []);

  const close = () => {
    paint();
    setAdded(undefined);
    setPicked(undefined);
  };
  const add = () => {
    if (!picked) return;
    const quote: QuoteDraft = { kind: 'quote', text: picked.text };
    updateMainComposer(d => [...d, quote]);
    paint(picked.range);
    setAdded(quote);
  };
  const comment = (value: string) => {
    const quote = added;
    if (quote && value) updateMainComposer(d => d.map(x => (x === quote ? { ...quote, comment: value } : x)));
    document.getSelection()?.removeAllRanges();
    close();
  };

  const range = picked?.range;
  return (
    <Popover.Root open={!!picked} onOpenChange={next => { if (!next) close(); }}>
      <Popover.Portal>
        <Popover.Positioner side="top" align="center" width={added ? 'xl' : 'anchor'} className={added ? undefined : 'w-auto'}
          anchor={range ? { getBoundingClientRect: () => range.getBoundingClientRect(), contextElement: range.startContainer.parentElement ?? undefined } : null}>
          <Popover.Popup data-quote-toolbar>
            {added
              ? <CommentEditor className="w-full" onSave={comment} onCancel={() => { document.getSelection()?.removeAllRanges(); close(); }} />
              : <button type="button"
                  // Pressing the button must not collapse the selection it acts on
                  onMouseDown={e => e.preventDefault()}
                  onClick={add}
                  className="inline-flex h-ctl-sm items-center gap-1 rounded-sm px-2 text-3 font-medium text-fg-1 outline-none hover:bg-hover focus-visible:bg-hover [&_svg]:size-icon [&_svg]:text-fg-3">
                  <MessageSquarePlus strokeWidth={1.5} />
                  {t('quote.add')}
                </button>}
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}
