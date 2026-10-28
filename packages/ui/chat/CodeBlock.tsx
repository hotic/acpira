import { memo, useCallback, useContext, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import type { DiffLine, DiffSource } from '@acpira/shared/transcript';
import { cn } from '../ui/cn';
import { useTheme, type Theme } from '../look';
import { diffCopyText, plainDiffRows, type CodeDiffRow } from './codeDiff';
import { promoteHighlight, requestCodeHighlight, requestDiffHighlight } from './diffHighlight';
import { codeLanguage, fenceLanguage, type CodeToken, type Language } from './codeLanguage';
import { OutputCopy } from './OutputCopy';
import { DiffSourceContext } from './fileLinks';
import { t } from '../i18n';

// The code surface's card and inset, for a card that sits beside one and must line its text up with the code's
export const CODE_CARD = 'rounded-lg border border-conversation-line';
export const CODE_INSET = 'px-pad py-[calc(var(--pad)-2px)]';

// Code surface: all monospace content (code blocks / tool output / diffs) shares this one surface,
// outlined with the conversation ring so it reads as a card next to the composer and bubbles
export function CodeSurface({ children, className, padded = true }: { children: ReactNode; className?: string; padded?: boolean }) {
  return (
    <pre className={cn('m-0 overflow-auto bg-code font-mono text-mono text-fg-1', CODE_CARD, padded && CODE_INSET, className)}>
      {children}
    </pre>
  );
}

// Fenced Markdown code: coloured only when the fence names a known grammar (see fenceLanguage); unlabelled blocks
// such as commit messages stay plain. Tokenizing runs in the shared highlight worker, a pass at a time while a block streams
const STREAM_SETTLE_MS = 120;

// A fence asks for its colours once its place on the page is known, and one in sight goes ahead of the rest: in page
// order, the fences in sight came last (a long answer is read from its end, and a switch of scheme colours every fence
// again). One observer serves every fence; a hidden page runs no observer, so a fence asks anyway after PLACE_WAIT_MS
const PLACE_WAIT_MS = 250;
const placing = new Map<Element, (inSight: boolean) => void>();
// Fences whose pass was asked out of sight and still waits: one the reader scrolls to goes ahead of the rest
const watching = new Map<Element, () => void>();
let sight: IntersectionObserver | undefined;
function observer(): IntersectionObserver {
  return sight ??= new IntersectionObserver(entries => {
    for (const { target, isIntersecting } of entries) {
      const done = placing.get(target);
      const go = isIntersecting ? watching.get(target) : undefined;
      placing.delete(target);
      if (go) watching.delete(target);
      if (!watching.has(target)) sight!.unobserve(target);
      done?.(isIntersecting);
      go?.();
    }
  });
}
function whenPlaced(el: Element, then: (inSight: boolean) => void): () => void {
  placing.set(el, then);
  observer().observe(el);
  return () => { if (placing.delete(el) && !watching.has(el)) sight?.unobserve(el); };
}
function whenInSight(el: Element, then: () => void): () => void {
  watching.set(el, then);
  observer().observe(el);
  return () => { if (watching.delete(el) && !placing.has(el)) sight?.unobserve(el); };
}

interface Colored { code: string; language: Language; theme: Theme; lines: CodeToken[][]; rows: ColoredRows }

export function CodeBlock({ code, lang }: { code: string; lang?: string }) {
  const language = fenceLanguage(lang);
  const theme = useTheme();
  const [colored, setColored] = useState<Colored>();
  const root = useRef<HTMLDivElement>(null);
  // A fence has one pass with the worker at a time. A pass for text the fence has since extended (it streams) goes on,
  // and its colours cover the text it asked for; the next one follows STREAM_SETTLE_MS after it lands and asks for the
  // text then. Withdrawn by every chunk, a pass never landed (the text grows every 32 ms reveal tick), and a fence that
  // streamed for seconds stayed plain until it stopped
  const pass = useRef<{ ctrl: AbortController; code: string; language: Language; theme: Theme } | undefined>(undefined);
  const want = useRef<{ code: string; language: Language; theme: Theme } | undefined>(undefined);
  const next = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const landed = useRef<{ code: string; theme: Theme } | undefined>(undefined);
  // Its place, asked one at a time: passes that come while it is pending share it, so a fence whose text changes
  // faster than the observer answers still gets its answer within a frame
  const place = useRef<Promise<boolean> | undefined>(undefined);
  const placed = () => place.current ??= new Promise<boolean>(resolve => {
    let settled = false;
    const done = (inSight: boolean) => {
      if (settled) return;
      settled = true;
      forget();
      clearTimeout(timer);
      place.current = undefined;
      resolve(inSight);
    };
    const forget = root.current ? whenPlaced(root.current, done) : () => {};
    const timer = setTimeout(() => done(false), root.current ? PLACE_WAIT_MS : 0);
  });

  const ask = () => {
    next.current = undefined;
    const asked = want.current;
    if (!asked) return;
    const ctrl = new AbortController();
    pass.current = { ctrl, ...asked };
    const end = () => { if (pass.current?.ctrl === ctrl) pass.current = undefined; };
    void placed().then(inSight => {
      if (ctrl.signal.aborted) return;
      // Asked out of sight, the pass waits behind the fences asked before it, until the reader scrolls to this one
      const unwatch = !inSight && root.current ? whenInSight(root.current, () => promoteHighlight(ctrl.signal)) : () => {};
      ctrl.signal.addEventListener('abort', unwatch, { once: true });
      return requestCodeHighlight(asked.code, asked.language, asked.theme, { signal: ctrl.signal, first: inSight }).finally(unwatch).then(lines => {
        if (ctrl.signal.aborted) return;
        end();
        landed.current = { code: asked.code, theme: asked.theme };
        setColored(had => {
          // A pass over a longer text of the fence colours the complete lines of the one before as it did: they keep
          // their elements, which React passes over
          const same = had && had.language === asked.language && had.theme === asked.theme && asked.code.startsWith(had.code);
          return { ...asked, lines, rows: coloredRows(lines, asked.theme === 'light', same ? had.rows.rows.slice(0, had.lines.length - 1) : []) };
        });
        if (want.current && want.current.code !== asked.code) next.current = setTimeout(ask, STREAM_SETTLE_MS);
      });
    }).catch(end /* Plain source remains when a grammar cannot load. */);
  };
  useEffect(() => {
    const running = pass.current;
    // A pass for another text than this one's start, scheme or grammar is of no use
    if (running && (running.language !== language || running.theme !== theme || !code.startsWith(running.code))) {
      running.ctrl.abort();
      pass.current = undefined;
    }
    want.current = language ? { code, language, theme } : undefined;
    if (!language || pass.current) return;
    // A first pass, a switch of scheme or a replaced text asks at once, even past a wait already set; a streaming fence
    // a while after its last pass landed
    const last = landed.current;
    const settle = !!last && last.theme === theme && last.code !== code && code.startsWith(last.code);
    if (next.current !== undefined) {
      if (settle) return;
      clearTimeout(next.current);
    }
    next.current = setTimeout(ask, settle ? STREAM_SETTLE_MS : 0);
  }, [code, language, theme]);
  // A fence that left the page withdraws its pass, so the worker does not tokenize text no one sees
  useEffect(() => () => {
    pass.current?.ctrl.abort();
    pass.current = undefined;
    clearTimeout(next.current);
    next.current = undefined;
  }, []);

  // Tokens of the other scheme have no colours for this one: the source shows plain until it is coloured again
  const usable = language && colored?.language === language && colored.theme === theme ? colored : undefined;
  // The wrapper carries the hover group and anchors the copy button outside the scrolling <pre>,
  // so the button stays pinned to the top-right corner while long lines scroll sideways
  return <div ref={root} className="group/code-output relative min-w-0">
    <OutputCopy text={code} label={t('code.copyCode')} />
    <CodeSurface><code className={usable ? 'code-syntax' : undefined}>{usable ? syntaxChildren(code, usable) : plainLines(code)}</code></CodeSurface>
  </div>;
}

// Every source line is a block of its own (`.code-line`). WebKit lays a block's inline content out again from scratch
// when text changes in a block that also holds an inline box (a token span) or when a node leaves it, and parks the old
// layout until an idle frame of a focused window (docs/dev/desktop.md, page memory). As one block, a fence that streams
// was laid out again and parked whole on every reveal pass; as lines, only the line that grows is.
// Each line but the last keeps its newline: a blank line then has a line's height, and a hand-made selection copies blank
// lines (WebKit and Blink emit nothing for an empty block). The text is the source exactly, as when it was one text run
const codeLine = (row: number, children: ReactNode, newline: boolean) =>
  <span key={row} className="code-line">{children}{newline && '\n'}</span>;

// `text` split into plain line blocks, numbered from `row`
function plainLines(text: string, row = 0): ReactNode[] {
  const lines = text.split('\n');
  return lines.map((line, i) => codeLine(row + i, line, i < lines.length - 1));
}

// A pass's coloured lines, made once when it lands: a fence that streams on renders them as the same elements chunk
// after chunk, and only its growing tail anew. Every line keeps its newline; `last` is the final one without. `kept`:
// elements of the lines before, coloured the same
interface ColoredRows { rows: ReactNode[]; last: ReactNode }
function coloredRows(lines: CodeToken[][], light: boolean, kept: ReactNode[]): ColoredRows {
  const tokens = (row: number) => lines[row]!.map((token, index) => {
    const color = light ? token.light : token.dark;
    return color ? <span key={index} style={{ color }}>{token.text}</span> : token.text;
  });
  return {
    rows: lines.map((_, row) => (row < kept.length ? kept[row] : codeLine(row, tokens(row), true))),
    last: lines.length ? codeLine(lines.length - 1, tokens(lines.length - 1), false) : null,
  };
}

// Coloured lines for the part of `code` that the last tokenized text covers. While a fence streams, the new text
// extends the old one: its complete lines keep their colours and only the growing tail renders plain until the next pass
function syntaxChildren(code: string, colored: Colored): ReactNode[] {
  const { rows, last } = colored.rows;
  if (colored.code === code) return rows.length ? [...rows.slice(0, -1), last] : [];
  if (!code.startsWith(colored.code)) return plainLines(code);
  // Kept lines are all followed by more text while the fence streams
  const tail = colored.code.lastIndexOf('\n') + 1;
  const keep = Math.min(rows.length, colored.code.slice(0, tail).split('\n').length - 1);
  return [...rows.slice(0, keep), ...plainLines(code.slice(tail), keep)];
}

// Defer syntax-highlight worker jobs and React state commits while any scroll container is actively moving,
// so cards flying through the viewport never contend with scroll frames
const SCROLL_IDLE_MS = 150;
let lastScrollTime = -SCROLL_IDLE_MS;
let scrollListenerAttached = false;

function ensureScrollTracker() {
  if (scrollListenerAttached || typeof window === 'undefined') return;
  scrollListenerAttached = true;
  window.addEventListener('scroll', () => { lastScrollTime = performance.now(); }, { capture: true, passive: true });
}

const DIFF_BLOCK = 40;
function chunkDiffRows(rows: CodeDiffRow[]): CodeDiffRow[][] {
  const out: CodeDiffRow[][] = [];
  for (let i = 0; i < rows.length; i += DIFF_BLOCK) out.push(rows.slice(i, i + DIFF_BLOCK));
  return out;
}

// Where a diff sits in the session: the tool call and which of its diffs (counted in `contents`, or `content` alone)
export interface DiffLocation { toolCallId: string; nth: number }

// Shared production version of the selected code-detail LAB design. A page view's diff carries its source without the
// file texts (`omitted`): rows highlight from the visible segments, and copying asks the host for the new file
// `whole`: the diff at its full height, its container scrolling it (the review), instead of a capped box of its own
export const DiffBlock = memo(function DiffBlock({ lines, source, path = '', locate, whole }: { lines: DiffLine[]; source?: DiffSource; path?: string; locate?: DiffLocation; whole?: boolean }) {
  const root = useRef<HTMLDivElement>(null);
  const theme = useTheme();
  const light = theme === 'light';
  const plain = useMemo(() => plainDiffRows(lines), [lines]);
  const [colored, setColored] = useState<{ lines: DiffLine[]; source?: DiffSource; path: string; theme: Theme; rows: CodeDiffRow[] }>();
  const isColored = colored?.lines === lines && colored.source === source && colored.path === path && colored.theme === theme;
  const rows = isColored ? colored.rows : plain;
  const blocks = useMemo(() => (whole ? chunkDiffRows(rows) : undefined), [whole, rows]);
  const copyText = useMemo(() => diffCopyText(lines, source), [lines, source]);
  const fetchSource = useContext(DiffSourceContext);
  // Falls back to the visible lines when the host no longer has the call (the session closed, the turn was replaced)
  const readSource = useCallback(async () => (await fetchSource!(locate!.toolCallId, locate!.nth))?.newText ?? copyText,
    [fetchSource, locate?.toolCallId, locate?.nth, copyText]);
  const fetchable = source?.omitted && locate && fetchSource;
  const language = codeLanguage(source?.path ?? path);

  useEffect(() => {
    const element = root.current;
    if (!element || !language || isColored) return;
    ensureScrollTracker();
    let current = true;
    let visible = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let ctrl: AbortController | undefined;
    const stop = () => {
      if (timer !== undefined) { clearTimeout(timer); timer = undefined; }
      if (ctrl) { ctrl.abort(); ctrl = undefined; }
    };
    const schedule = () => {
      if (!current || !visible || ctrl || timer !== undefined) return;
      const sinceScroll = performance.now() - lastScrollTime;
      if (sinceScroll < SCROLL_IDLE_MS) {
        timer = setTimeout(() => { timer = undefined; schedule(); }, SCROLL_IDLE_MS - sinceScroll + 16);
        return;
      }
      const myCtrl = new AbortController();
      ctrl = myCtrl;
      requestDiffHighlight(lines, source, path, theme, myCtrl.signal).then(nextRows => {
        if (!current || myCtrl.signal.aborted) return;
        const applyWhenIdle = () => {
          if (!current || !visible || myCtrl.signal.aborted) return;
          const idleFor = performance.now() - lastScrollTime;
          if (idleFor < SCROLL_IDLE_MS) {
            timer = setTimeout(() => { timer = undefined; applyWhenIdle(); }, SCROLL_IDLE_MS - idleFor + 16);
            return;
          }
          setColored({ lines, source, path, theme, rows: nextRows });
        };
        applyWhenIdle();
      }).catch(() => { /* Plain source remains available if aborted or a grammar cannot load. */ });
    };
    const observer = new IntersectionObserver(entries => {
      visible = entries.some(entry => entry.isIntersecting) && !element.closest('[inert]');
      if (visible) schedule();
      else stop();
    });
    observer.observe(element);
    return () => {
      current = false;
      stop();
      observer.disconnect();
    };
  }, [lines, source, path, language, isColored, theme]);

  const renderRow = (row: CodeDiffRow, index: number) => row.kind === 'hunk' && (index === 0 || index === rows.length - 1) ? null : row.kind === 'hunk'
    ? <div className="diff-hunk relative h-diff-hunk border-y border-line my-gap-half text-fg-3 select-none" key={index} title={row.text} aria-label={row.text}>
        <span className="block w-diff-gutter text-center text-3 leading-gap" aria-hidden="true">···</span></div>
    : <div className="diff-row relative flex min-h-code-line bg-(--diff-row-bg)" data-kind={row.kind} key={index}>
      <span className={cn('diff-gutter w-diff-gutter shrink-0 text-right text-fg-3 select-none tabular-nums', row.kind === 'ctx' && 'pr-diff-sign')} aria-hidden="true">
        {row.kind === 'ctx' ? row.newLine : <>
          {row.kind === 'del' ? row.oldLine : row.newLine}
          <span className={cn('diff-sign inline-block w-diff-sign text-center', row.kind === 'add' ? 'text-ok' : 'text-danger')}>{row.kind === 'add' ? '+' : '−'}</span>
        </>}
      </span>
      <span className="diff-source block flex-1 pr-pad whitespace-pre">{row.tokens.length ? row.tokens.map((token, tokenIndex) => {
        const color = light ? token.light : token.dark;
        return color ? <span key={tokenIndex} style={{ color }}>{token.text}</span> : token.text;
      }) : '\u200b'}</span>
    </div>;

  return <div ref={root} className="group/code-output code-output diff-surface" data-language={language ?? 'plain'}>
    <OutputCopy text={copyText} label={source !== undefined && (!source.omitted || fetchable) ? t('code.copySource') : t('code.copyVisible')}
      read={fetchable ? readSource : undefined} />
    <div data-whole={whole || undefined} className={cn('diff-scroll scroll-thin py-gap-half', whole ? 'overflow-clip hover:overflow-x-auto focus-within:overflow-x-auto data-scrolled-x:overflow-x-auto' : 'max-h-code-output overflow-auto')} tabIndex={0} role="region" aria-label={t('code.diff')}
      onWheel={e => { if (e.deltaX > 0) e.currentTarget.setAttribute('data-scrolled-x', ''); }}
      onScroll={e => e.currentTarget.toggleAttribute('data-scrolled-x', e.currentTarget.scrollLeft > 0)}>
      <div className="diff-table min-w-full w-max font-mono text-mono leading-code-output">
        {blocks ? blocks.map((block, b) => (
          <div key={b} className="diff-block" style={{ '--diff-block-rows': block.length } as CSSProperties}>
            {block.map((row, k) => renderRow(row, b * DIFF_BLOCK + k))}
          </div>
        )) : rows.map(renderRow)}
      </div>
    </div>
  </div>;
});
