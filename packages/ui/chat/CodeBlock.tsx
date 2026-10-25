import { memo, useCallback, useContext, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import type { DiffLine, DiffSource } from '@acpira/shared/transcript';
import { cn } from '../ui/cn';
import { useTheme } from '../look';
import { diffCopyText, plainDiffRows, type CodeDiffRow } from './codeDiff';
import { requestCodeHighlight, requestDiffHighlight } from './diffHighlight';
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
// such as commit messages stay plain. Tokenizing runs in the shared highlight worker, debounced while a block streams
const STREAM_SETTLE_MS = 120;

interface Colored { code: string; language: Language; lines: CodeToken[][] }

export function CodeBlock({ code, lang }: { code: string; lang?: string }) {
  const language = fenceLanguage(lang);
  const light = useTheme() === 'light';
  const [colored, setColored] = useState<Colored>();
  const tokenized = useRef(false);

  useEffect(() => {
    if (!language) return;
    let current = true;
    // The first pass runs at once; later passes only follow a streaming fence, so wait for its chunks to pause
    const timer = setTimeout(() => {
      requestCodeHighlight(code, language).then(lines => {
        if (!current) return;
        tokenized.current = true;
        setColored({ code, language, lines });
      }).catch(() => { /* Plain source remains when a grammar cannot load. */ });
    }, tokenized.current ? STREAM_SETTLE_MS : 0);
    return () => { current = false; clearTimeout(timer); };
  }, [code, language]);

  const usable = language && colored?.language === language ? colored : undefined;
  // The wrapper carries the hover group and anchors the copy button outside the scrolling <pre>,
  // so the button stays pinned to the top-right corner while long lines scroll sideways
  return <div className="group/code-output relative min-w-0">
    <OutputCopy text={code} label={t('code.copyCode')} />
    <CodeSurface><code className={usable ? 'code-syntax' : undefined}>{usable ? syntaxChildren(code, usable, light) : code}</code></CodeSurface>
  </div>;
}

// Coloured lines for the part of `code` that the last tokenized text covers. While a fence streams, the new text
// extends the old one: its complete lines keep their colours and only the growing tail renders plain until the next pass
function syntaxChildren(code: string, colored: Colored, light: boolean): ReactNode[] {
  const exact = colored.code === code;
  if (!exact && !code.startsWith(colored.code)) return [code];
  const tail = exact ? code.length : colored.code.lastIndexOf('\n') + 1;
  const keep = exact ? colored.lines.length : colored.code.slice(0, tail).split('\n').length - 1;
  const out: ReactNode[] = [];
  colored.lines.slice(0, keep).forEach((tokens, row) => {
    if (row > 0) out.push('\n');
    tokens.forEach((token, index) => {
      const color = light ? token.light : token.dark;
      out.push(color ? <span key={`${row}:${index}`} style={{ color }}>{token.text}</span> : token.text);
    });
  });
  if (!exact) out.push((keep > 0 ? '\n' : '') + code.slice(tail));
  return out;
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
  const light = useTheme() === 'light';
  const plain = useMemo(() => plainDiffRows(lines), [lines]);
  const [colored, setColored] = useState<{ lines: DiffLine[]; source?: DiffSource; path: string; rows: CodeDiffRow[] }>();
  const isColored = colored?.lines === lines && colored.source === source && colored.path === path;
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
      requestDiffHighlight(lines, source, path, myCtrl.signal).then(nextRows => {
        if (!current || myCtrl.signal.aborted) return;
        const applyWhenIdle = () => {
          if (!current || !visible || myCtrl.signal.aborted) return;
          const idleFor = performance.now() - lastScrollTime;
          if (idleFor < SCROLL_IDLE_MS) {
            timer = setTimeout(() => { timer = undefined; applyWhenIdle(); }, SCROLL_IDLE_MS - idleFor + 16);
            return;
          }
          setColored({ lines, source, path, rows: nextRows });
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
  }, [lines, source, path, language, isColored]);

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
