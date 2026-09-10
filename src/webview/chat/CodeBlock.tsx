import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import type { DiffLine, DiffSource } from '@shared/transcript';
import { cn } from '../ui/cn';
import { diffCopyText, plainDiffRows, type CodeDiffRow } from './codeDiff';
import { requestDiffHighlight } from './diffHighlight';
import { codeLanguage } from './codeSyntax';
import { OutputCopy } from './OutputCopy';
import { t } from '../i18n';

// Code surface: all monospace content (code blocks / tool output / diffs) shares this one surface,
// outlined with the conversation ring so it reads as a card next to the composer and bubbles
export function CodeSurface({ children, className, padded = true }: { children: ReactNode; className?: string; padded?: boolean }) {
  return (
    <pre className={cn('m-0 overflow-auto rounded-lg border border-conversation-line bg-code font-mono text-mono text-fg-1', padded && 'px-pad py-[calc(var(--pad)-2px)]', className)}>
      {children}
    </pre>
  );
}

// Lightweight token coloring for tool output that is not Markdown; streamdown handles fenced blocks in Prose
const TOKEN = /(\/\/.*$)|('[^']*')|\b(const|let|export|async|function|return|for|of|if|await|import|from|new|type|interface)\b|\b([A-Z][A-Za-z0-9]*)\b|\b([a-z_][A-Za-z0-9_]*)(?=\()/gm;
const CLASS = ['text-[var(--tk-c)]', 'text-[var(--tk-s)]', 'text-[var(--tk-k)]', 'text-[var(--tk-t)]', 'text-[var(--tk-f)]'];

export function highlight(code: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0, i = 0;
  for (const m of code.matchAll(TOKEN)) {
    const idx = m.index ?? 0;
    if (idx > last) out.push(code.slice(last, idx));
    const g = m.slice(1).findIndex(Boolean);
    out.push(<span key={i++} className={CLASS[g]}>{m[0]}</span>);
    last = idx + m[0].length;
  }
  if (last < code.length) out.push(code.slice(last));
  return out;
}

export function CodeBlock({ code }: { code: string }) {
  return <CodeSurface><code>{highlight(code)}</code></CodeSurface>;
}

// Shared production version of the selected code-detail LAB design.
export function DiffBlock({ lines, source, path = '' }: { lines: DiffLine[]; source?: DiffSource; path?: string }) {
  const root = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);
  const plain = useMemo(() => plainDiffRows(lines), [lines]);
  const [colored, setColored] = useState<{ lines: DiffLine[]; source?: DiffSource; path: string; rows: CodeDiffRow[] }>();
  const rows = colored?.lines === lines && colored.source === source && colored.path === path ? colored.rows : plain;
  const copyText = useMemo(() => diffCopyText(lines, source), [lines, source]);
  const language = codeLanguage(source?.path ?? path);

  useEffect(() => {
    const element = root.current;
    if (!element) return;
    // Kept-mounted, collapsed outputs should not initialize grammars or tokenize.
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting) && !element.closest('[inert]')) {
        setVisible(true);
        observer.disconnect();
      }
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (!visible || !language) return;
    let current = true;
    requestDiffHighlight(lines, source, path).then(rows => { if (current) setColored({ lines, source, path, rows }); })
      .catch(() => { /* Plain source remains available if a grammar cannot load. */ });
    return () => { current = false; };
  }, [visible, lines, source, path, language]);

  return <div ref={root} className="group/code-output code-output diff-surface" data-language={language ?? 'plain'}>
    <OutputCopy text={copyText} label={source !== undefined ? t('code.copySource') : t('code.copyVisible')} />
    <div className="diff-scroll scroll-thin max-h-code-output overflow-auto py-gap-half" tabIndex={0} role="region" aria-label={t('code.diff')}>
      <div className="diff-table min-w-full w-max font-mono text-mono leading-code-output">
        {rows.map((row, index) => row.kind === 'hunk' && (index === 0 || index === rows.length - 1) ? null : row.kind === 'hunk'
          ? <div className="diff-hunk relative h-diff-hunk border-y border-line my-gap-half text-fg-3 select-none" key={index} title={row.text} aria-label={row.text}>
              <span className="block sticky left-0 w-diff-gutter text-center text-3 leading-gap" aria-hidden="true">···</span></div>
          : <div className="diff-row relative flex min-h-code-line bg-(--diff-row-bg)" data-kind={row.kind} key={index}>
            <span className="diff-gutter sticky left-0 z-1 flex shrink-0 self-stretch text-fg-3 bg-(--diff-row-bg) select-none tabular-nums" aria-hidden="true">
              <span className="diff-number min-w-diff-number text-right">{row.kind === 'del' ? row.oldLine : row.newLine}</span>
              <span className={cn('diff-sign w-diff-sign text-center', row.kind === 'add' && 'text-ok', row.kind === 'del' && 'text-danger')}>{row.kind === 'add' ? '+' : row.kind === 'del' ? '−' : ''}</span>
            </span>
            <code className="diff-source block flex-1 whitespace-pre">{row.tokens.length ? row.tokens.map((token, tokenIndex) =>
              <span key={tokenIndex} style={{ '--syntax-light': token.light, '--syntax-dark': token.dark } as CSSProperties}>{token.text}</span>) : '\u200b'}</code>
          </div>)}
      </div>
    </div>
  </div>;
}
