import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { MessageSquarePlus } from 'lucide-react';
import type { Draft } from '@shared/transcript';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { Popover, ShellLayerContext } from '../ui/Popover';
import { useMorph } from '../ui/useMorph';
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
  // The "Add to chat" button grows into the comment field
  const morph = useMorph<HTMLDivElement>(!!added);

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

  // The toolbar belongs to the transcript, so it lives in a clip layer over the thread instead of the shell layer: the composer
  // dock below and the header above are outside the layer, and its top edge follows the bottom of the prompt card of the exchange
  // holding the quote (stuck or not), so neither the composer nor the prompt card is ever covered by it
  const layer = useRef<HTMLDivElement>(null);
  const range = picked?.range;
  useLayoutEffect(() => {
    const el = layer.current;
    const region = el?.offsetParent;
    const thread = root.current?.closest<HTMLElement>('[data-thread]');
    if (!el || !region || !thread || !range) return;
    const start = range.startContainer;
    const prompt = (start instanceof Element ? start : start.parentElement)?.closest('[data-exchange]')?.querySelector<HTMLElement>('[data-sticky-prompt]');
    // A quote taken from the prompt itself has nothing above it to give way to
    const cover = prompt && !prompt.contains(start) ? prompt : undefined;
    if (!cover) return;
    // Runs before the positioner's own scroll update reads the layer's offset
    const place = () => {
      const edge = cover.getBoundingClientRect().bottom;
      el.style.top = `${Math.max(0, edge - region.getBoundingClientRect().top)}px`;
      // A quote gone under the card hides the bare button like one scrolled out of the thread (data-anchor-hidden below)
      el.toggleAttribute('data-covered', !added && range.getBoundingClientRect().bottom <= edge);
    };
    place();
    thread.addEventListener('scroll', place, { passive: true });
    const observer = new ResizeObserver(place);
    observer.observe(thread);
    observer.observe(cover);
    return () => {
      thread.removeEventListener('scroll', place);
      observer.disconnect();
      el.style.top = '';
      el.removeAttribute('data-covered');
    };
  }, [range, root, added]);

  // A quote taken inside a nested scrollport (a thought body, command output, a long prompt) belongs to that layer: the bare
  // button rides on the selection's first line and leaves once that line scrolls out of the scrollport, instead of floating over
  // the rows around it (or flipping below the range) until the whole range is gone, which is all the positioner's own
  // anchor-hidden check sees. The thread itself is still handled by the clip layer and data-anchor-hidden
  useLayoutEffect(() => {
    const el = layer.current;
    const thread = root.current?.closest<HTMLElement>('[data-thread]');
    if (!el || !thread || !range || added) return;
    const ports: Element[] = [];
    const common = range.commonAncestorContainer;
    for (let node: Element | null = common instanceof Element ? common : common.parentElement; node && node !== thread; node = node.parentElement) {
      if (getComputedStyle(node).overflowY !== 'visible') ports.push(node);
    }
    if (!ports.length) return;
    const place = () => {
      let top = -Infinity;
      let bottom = Infinity;
      for (const port of ports) {
        const r = port.getBoundingClientRect();
        top = Math.max(top, r.top);
        bottom = Math.min(bottom, r.bottom);
      }
      // Half a line in still counts, so a line cut by the scrollport's edge (or its fade) keeps its button
      const inside = (line?: DOMRect) => !!line && (line.top + line.bottom) / 2 >= top && (line.top + line.bottom) / 2 <= bottom;
      const lines = [...range.getClientRects()].filter(r => r.width > 0 && r.height > 0);
      // Flipped below the range (no room above it in the shell), the button also stands on the last line
      const below = el.querySelector('[data-side]')?.getAttribute('data-side') === 'bottom';
      el.toggleAttribute('data-scrolled-out', !inside(lines[0]) || (below && !inside(lines.at(-1))));
    };
    place();
    const resize = new ResizeObserver(place);
    // The positioner settles its side after the scroll that moved the range, so a flip re-runs the check
    const flip = new MutationObserver(place);
    flip.observe(el, { subtree: true, attributes: true, attributeFilter: ['data-side'] });
    for (const port of ports) {
      port.addEventListener('scroll', place, { passive: true });
      resize.observe(port);
    }
    return () => {
      for (const port of ports) port.removeEventListener('scroll', place);
      resize.disconnect();
      flip.disconnect();
      el.removeAttribute('data-scrolled-out');
    };
  }, [range, root, added]);

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
  // Pasted screenshots join the prompt as ordinary image attachments, right after the quote they were pasted on
  const comment = (value: string, images: Draft[]) => {
    const quote = added;
    if (quote && (value || images.length)) {
      updateMainComposer(d => d.flatMap(x => (x === quote ? [value ? { ...quote, comment: value } : quote, ...images] : [x])));
    }
    document.getSelection()?.removeAllRanges();
    close();
  };

  return (
    <>
      {/* Sits under the plan dock and toasts (later siblings at the same z) and under every shell-layer overlay */}
      <div ref={layer} data-quote-layer className="pointer-events-none absolute inset-0 z-10 overflow-hidden data-[covered]:invisible data-[scrolled-out]:invisible" />
      <ShellLayerContext.Provider value={layer}>
        <Popover.Root open={!!picked} onOpenChange={next => { if (!next) close(); }}>
          <Popover.Portal>
            {/* The bare button leaves once the quote has scrolled out of the thread; the comment field keeps its focus and is only clipped */}
            <Popover.Positioner side="top" align="center" width={added ? 'xl' : 'anchor'} className={cn('pointer-events-auto', !added && 'w-auto data-[anchor-hidden]:invisible')}
              anchor={range ? { getBoundingClientRect: () => range.getBoundingClientRect(), contextElement: range.startContainer.parentElement ?? undefined } : null}>
              <Popover.Popup ref={morph} data-quote-toolbar>
                {added
                  ? <CommentEditor className="w-full bg-transparent" acceptImages submit={t('quote.saveComment')} onSave={comment} onCancel={() => { document.getSelection()?.removeAllRanges(); close(); }} />
                  : <button type="button"
                      // Pressing the button must not collapse the selection it acts on
                      onMouseDown={e => e.preventDefault()}
                      onClick={add}
                      className="inline-flex h-ctl-sm items-center gap-1 rounded-sm px-2 text-3 font-medium text-fg-1 outline-none transition-colors hover:bg-active focus-visible:bg-active [&_svg]:size-icon [&_svg]:text-fg-3">
                      <MessageSquarePlus strokeWidth={1.5} />
                      {t('quote.add')}
                    </button>}
              </Popover.Popup>
            </Popover.Positioner>
          </Popover.Portal>
        </Popover.Root>
      </ShellLayerContext.Provider>
    </>
  );
}
