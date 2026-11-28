import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { ArrowDown, ArrowUp, Search, X } from 'lucide-react';
import { hitShortcut, keyHint } from '../keybindings';
import { t, useLocale } from '../i18n';
import { IconButton } from '../ui/Button';
import { imeKey } from '../ui/ime';
import { StableText } from '../ui/StableText';
import { focusMainComposer } from './useComposerDraft';

const FIND_ALL = 'acpira-find';
const FIND_ACTIVE = 'acpira-find-active';

function paintMatches(ranges: Range[], active?: Range) {
  const registry = (globalThis.CSS as { highlights?: Map<string, unknown> } | undefined)?.highlights;
  const Highlight = (globalThis as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight;
  if (!registry || !Highlight) return;
  if (ranges.length) registry.set(FIND_ALL, new Highlight(...ranges));
  else registry.delete(FIND_ALL);
  if (active) registry.set(FIND_ACTIVE, new Highlight(active));
  else registry.delete(FIND_ACTIVE);
}

// Visible text ranges inside `root` matching `needle` case-insensitively, skipping inert folds and hidden mirrors
export function collectTextMatches(root: HTMLElement, rawQuery: string): Range[] {
  const needle = rawQuery.toLowerCase();
  if (!needle) return [];
  const ranges: Range[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const parent = node.parentElement;
      if (!parent) return NodeFilter.FILTER_REJECT;
      if (parent.closest('[inert], [aria-hidden="true"], .sr-only, script, style, textarea, input')) {
        return NodeFilter.FILTER_REJECT;
      }
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  let current = walker.nextNode();
  while (current) {
    const text = (current.nodeValue ?? '').toLowerCase();
    let from = 0;
    while (from <= text.length - needle.length) {
      const hit = text.indexOf(needle, from);
      if (hit < 0) break;
      const range = document.createRange();
      range.setStart(current, hit);
      range.setEnd(current, hit + needle.length);
      ranges.push(range);
      from = hit + needle.length;
    }
    current = walker.nextNode();
  }
  return ranges;
}

function revealRange(root: HTMLElement, range: Range, releaseFollow?: () => void) {
  const thread = root.closest<HTMLElement>('[data-thread]');
  if (!thread) return;
  const startEl = range.startContainer instanceof Element ? range.startContainer : range.startContainer.parentElement;
  // Ensure a content-visibility-skipped exchange (and a process chunk inside it) lays out at its real height before
  // measuring the range
  for (let skipped = startEl?.closest<HTMLElement>('[data-seen]'); skipped && thread.contains(skipped); skipped = skipped.parentElement?.closest<HTMLElement>('[data-seen]'))
    skipped.removeAttribute('data-seen');
  const rect = range.getBoundingClientRect();
  const view = thread.getBoundingClientRect();
  if (rect.height === 0 && rect.width === 0) return;
  const sticky = startEl?.closest('[data-exchange]')?.querySelector<HTMLElement>('[data-sticky-prompt]');
  const topClear = sticky && !sticky.contains(startEl) ? Math.max(view.top, sticky.getBoundingClientRect().bottom) : view.top;
  if (rect.top >= topClear && rect.bottom <= view.bottom) return;
  releaseFollow?.();
  thread.scrollBy({ top: rect.top - topClear - (view.bottom - topClear) / 3, behavior: 'auto' });
}

export interface ThreadSearchBridge {
  mountAll: () => void;
  releaseFollow: () => void;
}

export function ChatFind({
  root,
  sessionId,
  initialQuery,
  focusSeq,
  bridge,
  onClose,
}: {
  root: RefObject<HTMLElement | null>;
  sessionId?: string;
  initialQuery?: string;
  focusSeq: number;
  bridge: RefObject<ThreadSearchBridge | undefined>;
  onClose: () => void;
}) {
  useLocale();
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState(initialQuery ?? '');
  const [state, setState] = useState<{ index: number; count: number }>({ index: 0, count: 0 });
  const rangesRef = useRef<Range[]>([]);
  const indexRef = useRef(0);
  const queryRef = useRef(query);
  queryRef.current = query;

  // Opening or pressing ⌘F again adopts any fresh selection text and selects the field
  useLayoutEffect(() => {
    if (initialQuery !== undefined && initialQuery !== queryRef.current) {
      setQuery(initialQuery);
      queryRef.current = initialQuery;
    }
    input.current?.focus();
    input.current?.select();
  }, [focusSeq, initialQuery]);

  // Mount windowed history exchanges so every turn in the conversation is searchable
  useEffect(() => {
    if (query) bridge.current?.mountAll();
  }, [query, sessionId, bridge]);

  const runSearch = useCallback((q: string, scroll: boolean, preferredIndex?: number) => {
    const body = root.current;
    if (!body || !q) {
      rangesRef.current = [];
      indexRef.current = 0;
      paintMatches([]);
      setState({ index: 0, count: 0 });
      return;
    }
    const ranges = collectTextMatches(body, q);
    rangesRef.current = ranges;
    const nextIndex = ranges.length
      ? ((preferredIndex ?? indexRef.current) % ranges.length + ranges.length) % ranges.length
      : 0;
    indexRef.current = nextIndex;
    const active = ranges[nextIndex];
    paintMatches(ranges, active);
    setState({ index: nextIndex, count: ranges.length });
    if (scroll && active) revealRange(body, active, () => bridge.current?.releaseFollow());
  }, [root, bridge]);

  useEffect(() => {
    indexRef.current = 0;
    runSearch(query, true, 0);
  }, [query, sessionId, runSearch]);

  // Keep highlights in step as streamed chunks or mounted history slices land, without moving scroll
  useEffect(() => {
    const body = root.current;
    if (!body || !query) return;
    let frame = 0;
    const observer = new MutationObserver(() => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        runSearch(queryRef.current, false);
      });
    });
    observer.observe(body, { childList: true, subtree: true, characterData: true });
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [root, query, sessionId, runSearch]);

  useEffect(() => () => paintMatches([]), []);

  const step = useCallback((back: boolean) => {
    const ranges = rangesRef.current;
    const body = root.current;
    if (!ranges.length || !body) return;
    const next = (indexRef.current + (back ? -1 : 1) + ranges.length) % ranges.length;
    indexRef.current = next;
    const active = ranges[next]!;
    paintMatches(ranges, active);
    setState({ index: next, count: ranges.length });
    revealRange(body, active, () => bridge.current?.releaseFollow());
  }, [root, bridge]);

  const close = useCallback(() => {
    paintMatches([]);
    onClose();
    focusMainComposer();
  }, [onClose]);

  // ⌘G / ⇧⌘G (Ctrl+G / Ctrl+Shift+G or F3 / Shift+F3 elsewhere, or custom findNext / findPrev) steps matches anywhere in the chat
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || e.defaultPrevented) return;
      if (hitShortcut(e, 'findPrev')) { e.preventDefault(); step(true); }
      else if (hitShortcut(e, 'findNext')) { e.preventDefault(); step(false); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [step]);

  const count = !query ? '' : state.count ? `${state.index + 1}/${state.count}` : t('terminal.noResults');

  return (
    <div data-find data-chat-find role="search" className="absolute top-gap right-pad z-10 flex h-ctl items-center gap-1 rounded-md border border-line-strong bg-bg-1 pr-1 pl-2 shadow-pop">
      <Search strokeWidth={1.5} className="size-icon shrink-0 text-fg-3" />
      <input
        ref={input}
        value={query}
        spellCheck={false}
        placeholder={t('terminal.find')}
        aria-label={t('terminal.find')}
        onChange={e => setQuery(e.target.value)}
        onKeyDown={e => {
          if (imeKey(e.nativeEvent)) return;
          if (e.key === 'Enter') { e.preventDefault(); step(e.shiftKey); }
          else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
        }}
        className="w-term-find min-w-0 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3"
      />
      <StableText
        className="shrink-0 text-3 text-fg-3 tabular-nums"
        current="now"
        items={[{ key: 'now', text: count }, { key: 'wide', text: '999/999' }, { key: 'none', text: t('terminal.noResults') }]}
      />
      <IconButton size="sm" title={`${t('terminal.findPrevious')} ${keyHint('Shift+Enter')}`} aria-label={t('terminal.findPrevious')} disabled={!state.count} onClick={() => step(true)}>
        <ArrowUp strokeWidth={1.5} />
      </IconButton>
      <IconButton size="sm" title={`${t('terminal.findNext')} ${keyHint('Enter')}`} aria-label={t('terminal.findNext')} disabled={!state.count} onClick={() => step(false)}>
        <ArrowDown strokeWidth={1.5} />
      </IconButton>
      <IconButton size="sm" title={`${t('common.close')} Esc`} aria-label={t('common.close')} onClick={close}>
        <X strokeWidth={1.5} />
      </IconButton>
    </div>
  );
}
