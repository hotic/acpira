import { useEffect, useState, type ReactNode } from 'react';
import { Braces, Check, Copy, ExternalLink, TextAlignStart, type LucideIcon } from 'lucide-react';
import type { ContextContent } from '@acpira/shared/context';
import { t } from '../i18n';
import { IconButton } from '../ui/Button';
import { Row } from '../ui/Row';
import { cn } from '../ui/cn';
import { contextApi } from './api';
import type { TNode } from './tree';

// What a leaf of the report sent (ContextReport.tsx), hung on its branch: a call's input and output, a definition's
// description and schema, each its own row that folds its text; plain text hangs straight off the branch. Asked of the
// agent when the leaf opens, by the report's rev

// Per rev and leaf: what an open leaf showed, so reopening it (or a refresh of the same conversation) shows it at once
const shown = new Map<string, ContextContent | null>();
const keyOf = (rev: string, id: string) => `${rev}\u0000${id}`;

function useContent(sessionId: string, rev: string, id: string): ContextContent | null | undefined {
  const [c, setC] = useState(() => shown.get(keyOf(rev, id)));
  useEffect(() => {
    const key = keyOf(rev, id);
    if (shown.has(key)) { setC(shown.get(key)); return; }
    setC(undefined);
    let live = true;
    void contextApi.item(sessionId, rev, id).then(r => {
      shown.set(key, r ?? null);
      if (live) setC(r ?? null);
    });
    return () => { live = false; };
  }, [sessionId, rev, id]);
  return c;
}

export function CopyButton({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  useEffect(() => {
    if (!done) return;
    const timer = setTimeout(() => setDone(false), 1200);
    return () => clearTimeout(timer);
  }, [done]);
  const copy = (e: React.MouseEvent) => {
    e.stopPropagation();
    void navigator.clipboard?.writeText(text).then(() => setDone(true), () => {});
  };
  const label = done ? t('code.copied') : t('common.copy');
  return <IconButton size="sm" title={label} aria-label={label} onClick={copy}>{done ? <Check strokeWidth={1.75} /> : <Copy strokeWidth={1.5} />}</IconButton>;
}

const BOX = 'scroll-thin mt-0.5 mb-1.5 max-h-(--report-part-h) overflow-auto rounded-sm bg-bg-1 px-2 py-1 font-mono text-mono text-fg-2';
const pre = (text: string, wrap: boolean) => <pre className={cn(BOX, 'm-0', wrap ? 'whitespace-pre-wrap break-words' : 'whitespace-pre')}>{text}</pre>;
const json = (v: unknown) => (typeof v === 'string' ? v : JSON.stringify(v, null, 2));
const lines = (s: string) => s.split('\n').length;

// A call's input as the model wrote it: key / value lines when it is a JSON object, else the text itself
function Input({ text }: { text: string }) {
  let value: unknown;
  try { value = JSON.parse(text); } catch { return pre(text, true); }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return pre(json(value), true);
  return (
    <div className={cn(BOX, 'grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3')}>
      {Object.entries(value).map(([k, v]) => {
        const s = typeof v === 'string' ? v : JSON.stringify(v);
        return [
          <span key={`${k}-k`} className="text-fg-3">{k}</span>,
          // Values with line breaks keep them and scroll sideways
          <span key={`${k}-v`} className={cn('min-w-0 text-fg-1', s.includes('\n') ? 'scroll-thin overflow-x-auto whitespace-pre' : 'whitespace-pre-wrap break-words')}>{s}</span>,
        ];
      })}
    </div>
  );
}

const noArgs = (input: string) => /^(\{\s*\}|null)?$/.test(input.trim());

const quiet = (text: string) => <div className="flex min-h-row-dense items-center text-3 text-fg-3">{text}</div>;

export function Parts({ node, sessionId, rev, file, onOpenFile }: {
  node: TNode; sessionId: string; rev: string;
  file?: string; onOpenFile?: (path: string) => void;
}) {
  const content = useContent(sessionId, rev, node.id);
  const [shut, setShut] = useState<Set<string>>(new Set());
  const openFile = file && onOpenFile
    ? <IconButton size="sm" title={t('context.openFile')} aria-label={t('context.openFile')} onClick={e => { e.stopPropagation(); onOpenFile(file); }}><ExternalLink strokeWidth={1.5} /></IconButton>
    : undefined;
  // Plain text's actions on a line of their own, kept out of the leaf's row so the row never shifts when it opens
  const head = (actions?: ReactNode) => actions && (
    <li data-bare data-head>
      <div className="flex min-h-row-dense items-center justify-end gap-gap text-3 text-fg-3">{actions}</div>
    </li>
  );
  if (content === undefined) return null;
  if (content === null) return <li data-bare>{quiet(t('context.empty'))}</li>;
  const cut = content.truncated && <li data-bare>{quiet(t('context.truncated'))}</li>;
  if (content.kind === 'text') {
    // The provider counted reasoning the reply does not show (context.rs gives it the last turn's thinking leaf)
    if (!content.text && node.node?.kind === 'thinking') return <li data-bare>{quiet(t('context.hiddenThinking'))}</li>;
    return <>{head(<>{openFile}<CopyButton text={content.text} /></>)}<li data-bare>{pre(content.text, true)}</li>{cut}</>;
  }
  // Input and Output hang a dot off the elbow instead of an icon (contextReport.css): hollow for what went in, filled for
  // what came back
  const parts: { key: string; label: string; icon?: LucideIcon; dot?: 'in' | 'out'; meta?: string; copy?: string; body: ReactNode; error?: boolean }[] = content.kind === 'def'
    ? [
      { key: 'desc', label: t('context.part.description'), icon: TextAlignStart, copy: content.description, body: pre(content.description, true) },
      { key: 'schema', label: t('context.part.schema'), icon: Braces, copy: json(content.schema), body: pre(json(content.schema), false) },
    ]
    : [
      // A call without arguments (`{}` or nothing) says so instead of opening an empty box
      noArgs(content.input)
        ? { key: 'in', label: t('context.part.input'), dot: 'in', meta: t('context.noInput'), body: null }
        : { key: 'in', label: t('context.part.input'), dot: 'in', copy: content.input, body: <Input text={content.input} /> },
      content.output === undefined
        ? { key: 'out', label: t('context.part.output'), dot: 'out', meta: t('context.noOutput'), body: null }
        : {
          key: 'out', label: t('context.part.output'), dot: 'out', copy: content.output, error: content.error,
          meta: [t('context.part.lines', { n: lines(content.output) }), content.images ? t('context.part.images', { n: content.images }) : ''].filter(Boolean).join(t('common.metaSep')),
          body: pre(content.output, false),
        },
    ];
  return <>{parts.map((p, i) => {
    const isOpen = !!p.body && !shut.has(p.key);
    const flip = () => setShut(s => { const n = new Set(s); if (n.has(p.key)) n.delete(p.key); else n.add(p.key); return n; });
    return (
      <li key={p.key} data-part={p.key}>
        <Row as="div" dense role="button" tabIndex={0} aria-expanded={p.body ? isOpen : undefined} onClick={p.body ? flip : undefined}
          onKeyDown={e => { if (p.body && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); flip(); } }}
          className={cn('ctx-band w-full', p.body && 'cursor-pointer')}
          lead={p.dot ? <span aria-hidden className="ctx-part-dot" data-dir={p.dot} /> : p.icon && <p.icon strokeWidth={1.5} className="size-3.5 text-fg-3" />}
          trailing={<>
            {i === 0 && openFile}
            {p.copy !== undefined && <CopyButton text={p.copy} />}
          </>}>
          <span className={cn('shrink-0', p.error ? 'text-danger' : 'text-fg-1')}>{p.label}</span>
          {p.meta && <span className="min-w-0 truncate text-3 text-fg-3">{p.meta}</span>}
        </Row>
        {isOpen && p.body}
      </li>
    );
  })}{cut}</>;
}
