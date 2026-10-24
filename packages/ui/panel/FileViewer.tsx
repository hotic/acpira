import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type RefObject } from 'react';
import type { DiffLine } from '@shared/transcript';
import type { ProjectBase, ProjectChange, ProjectHunk } from '@shared/protocol';
import { BookOpen, ChevronDown, ChevronUp, ExternalLink, Layers, MessageSquarePlus, WrapText, X } from 'lucide-react';
import type { TextBlock } from '@shared/transcript';
import { t } from '../i18n';
import { IconButton } from '../ui/Button';
import { popupClass } from '../ui/Overlay';
import { cn } from '../ui/cn';
import { DiffBlock } from '../chat/CodeBlock';
import { DiffStat } from '../chat/ToolCall';
import { Prose } from '../chat/Prose';
import { OpenToolFileContext } from '../chat/fileLinks';
import { requestCodeHighlight } from '../chat/diffHighlight';
import { codeLanguage, type CodeToken } from '../chat/codeLanguage';
import { project, type FileRead } from './api';
import type { LineRange } from './store';
import { FileGlyph, dirOf, nameOf } from './files';

// The Files tab's viewer: one file as it is now with the changes against a base marked (added lines tinted, removed ones
// put back where they stood), a change navigator, soft wrap with a hanging indent, and line numbers that pick a range for
// the chat. The review lays every changed file out in the transcript's own diff blocks

// A line of the viewer: the file as it is now, with the removed lines put back in place
interface ViewRow { kind: 'ctx' | 'add' | 'del'; line?: number; text: string; tokens?: CodeToken[]; hunk?: number }

const linesOf = (text: string) => text.replace(/\r?\n$/, '').split(/\r?\n/);

function viewRows(text: string, hunks: ProjectHunk[], after?: CodeToken[][], removed?: CodeToken[][][]): ViewRow[] {
  const lines = text ? linesOf(text) : [];
  const starts = new Map(hunks.map((h, i) => [h.at, i]));
  const rows: ViewRow[] = [];
  const putBack = (i: number) => hunks[i]!.del?.forEach((d, j) => rows.push({ kind: 'del', text: d, tokens: removed?.[i]?.[j], hunk: i }));
  let inside = 0;
  lines.forEach((line, idx) => {
    const n = idx + 1;
    const start = starts.get(n);
    if (start !== undefined) putBack(start);
    while (inside < hunks.length && n >= hunks[inside]!.at + hunks[inside]!.add) inside++;
    const h = hunks[inside];
    const added = !!h && n >= h.at && n < h.at + h.add;
    rows.push({ kind: added ? 'add' : 'ctx', line: n, text: line, tokens: after?.[idx], ...(added ? { hunk: inside } : start !== undefined ? { hunk: start } : {}) });
  });
  // Lines removed at the very end stand after the last line
  const tail = starts.get(lines.length + 1);
  if (tail !== undefined) putBack(tail);
  return rows;
}

// The same changes as unified diff lines (three lines of context, near hunks merged), for the transcript's DiffBlock
export function diffLinesOf(text: string, hunks: ProjectHunk[]): DiffLine[] {
  const rows: { kind: 'ctx' | 'add' | 'del'; text: string; old?: number; now?: number }[] = [];
  let old = 1;
  for (const r of viewRows(text, hunks)) {
    if (r.kind === 'del') rows.push({ kind: 'del', text: r.text, old: old++ });
    else if (r.kind === 'add') rows.push({ kind: 'add', text: r.text, now: r.line });
    else rows.push({ kind: 'ctx', text: r.text, old: old++, now: r.line });
  }
  const near = rows.map((_, i) => rows.slice(Math.max(0, i - 3), i + 4).some(x => x.kind !== 'ctx'));
  const out: DiffLine[] = [];
  for (let i = 0; i < rows.length; i++) {
    if (!near[i]) continue;
    let j = i;
    while (j < rows.length && near[j]) j++;
    const part = rows.slice(i, j);
    const oldStart = part.find(x => x.old !== undefined)?.old ?? 1, newStart = part.find(x => x.now !== undefined)?.now ?? 1;
    out.push({ kind: 'hunk', text: `@@ -${oldStart},${part.filter(x => x.kind !== 'add').length} +${newStart},${part.filter(x => x.kind !== 'del').length} @@` });
    for (const x of part) out.push({ kind: x.kind, text: `${x.kind === 'add' ? '+' : x.kind === 'del' ? '-' : ' '}${x.text}`, ...(x.old === undefined ? {} : { oldLine: x.old }), ...(x.now === undefined ? {} : { newLine: x.now }) });
    i = j - 1;
  }
  return out;
}

// Highlighting a file past this size would hold the worker for a while; it is shown plain
const HIGHLIGHT_MAX = 256 * 1024;

// Syntax colours for the file and each hunk's removed lines, from the transcript's highlight worker
function useHighlight(path: string, read: FileRead | undefined) {
  const [colored, setColored] = useState<{ read: FileRead; after: CodeToken[][]; removed: CodeToken[][][] }>();
  useEffect(() => {
    const text = read?.text;
    const language = codeLanguage(path);
    if (text === undefined || !language || text.length > HIGHLIGHT_MAX) return;
    let live = true;
    void Promise.all([
      requestCodeHighlight(text.replace(/\r?\n$/, ''), language),
      Promise.all(read!.hunks.map(h => (h.del?.length ? requestCodeHighlight(h.del.join('\n'), language) : Promise.resolve([])))),
    ]).then(([after, removed]) => { if (live) setColored({ read: read!, after, removed }); }).catch(() => { /* plain text stays */ });
    return () => { live = false; };
  }, [read]);
  return colored?.read === read ? colored : undefined;
}

function Code({ tokens, text }: { tokens?: CodeToken[]; text: string }) {
  if (!tokens) return <>{text || '​'}</>;
  return <>{tokens.length ? tokens.map((tok, i) => <span key={i} style={{ '--syntax-light': tok.light, '--syntax-dark': tok.dark } as CSSProperties}>{tok.text}</span>) : '​'}</>;
}

// A read error as the viewer says it
export function readError(error: string): string {
  if (error === 'missing') return t('panel.error.missing');
  if (error === 'binary') return t('panel.error.binary');
  if (error === 'tooLarge') return t('panel.error.tooLarge');
  if (error === 'outside') return t('panel.error.outside');
  return t('panel.error.other', { error });
}

export const VIEW_BAR = 'flex min-h-row shrink-0 items-center gap-1 border-b border-line py-gap-half pr-gap pl-pad';
// Files the viewer can show rendered as well as as source
const isMarkdown = (path: string) => /\.(md|markdown|mdx)$/i.test(path);

// The last preview choice, carried to the next Markdown file this page opens
let previewMarkdown = false;

// YAML front matter (a skill's `name` / `description`) would render as a rule and a setext heading; it shows as a YAML block
function previewText(text: string): string {
  const front = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/.exec(text);
  return front ? `\`\`\`yaml\n${front[1]}\n\`\`\`\n\n${text.slice(front[0].length)}` : text;
}

// The file rendered the way a reply is; links go through `onOpenLink`, relative ones from the file's folder
function MarkdownPreview({ text, onOpenLink }: { text: string; onOpenLink?: (path: string, line?: number) => void }) {
  const block = useMemo<TextBlock>(() => ({ type: 'text', markdown: previewText(text) }), [text]);
  return (
    <OpenToolFileContext.Provider value={onOpenLink}>
      <div className="scroll-thin absolute inset-0 overflow-auto px-pad py-pad" data-markdown-preview>
        <Prose block={block} />
      </div>
    </OpenToolFileContext.Provider>
  );
}

// Rows laid out a block at a time (panel.css .code-block): a long file skips layout and paint for what is out of sight
const BLOCK = 200;

export interface FileViewerProps {
  sessionId: string;
  path: string;
  // What the changes are measured against: before this session first edited the file when it did, else the last commit.
  // Not offered as a pick since 2026-10-24 (a dropdown on the bar was judged unnecessary)
  base: ProjectBase;
  // Bumped when the file may have changed on disk (a turn ended)
  refresh: number;
  line?: number;
  selection?: LineRange;
  onSelect: (r: LineRange | undefined) => void;
  // The picked range's text, for the chat
  onSelectionText: (r: LineRange | undefined, text: string) => void;
  onAdd: (r: LineRange, text: string) => void;
  onReveal: () => void;
  onOpenEditor?: (line?: number) => void;
  // A link in the Markdown preview: a path as written in the file (relative to its folder, or absolute)
  onOpenLink?: (path: string, line?: number) => void;
  // Closes the viewer, leaving the tree alone (the tool panel's Files tab; the files layout closes its file tabs instead)
  onClose?: () => void;
}

// The viewer bar's last button: never shed by a narrow bar, so a file can always be put away
function CloseView({ onClose }: { onClose: () => void }) {
  return <IconButton size="sm" className="shrink-0" title={t('common.close')} aria-label={t('common.close')} onClick={onClose}><X strokeWidth={1.5} /></IconButton>;
}

export function FileViewer(p: FileViewerProps) {
  const [read, setRead] = useState<FileRead & { key: string }>();
  const key = `${p.path}\n${p.base}`;
  useEffect(() => {
    let live = true;
    void project.file(p.sessionId, p.path, p.base).then(r => { if (live) setRead({ ...r, key }); });
    return () => { live = false; };
  }, [p.sessionId, key, p.refresh]);
  const current = read?.key === key ? read : undefined;
  const colored = useHighlight(p.path, current);
  const hunks = current?.hunks ?? [];
  const rows = useMemo(() => (current?.text !== undefined ? viewRows(current.text, current.hunks, colored?.after, colored?.removed) : []), [current, colored]);
  const total = current?.text !== undefined ? linesOf(current.text).length : 0;
  const [hunk, setHunk] = useState(0);
  // Soft wrap is the default in a side panel; a continuation line hangs two columns past the line's own indentation
  const [wrap, setWrap] = useState(true);
  const markdown = isMarkdown(p.path);
  const [preview, setPreview] = useState(previewMarkdown);
  const previewing = markdown && preview && current?.text !== undefined;
  const togglePreview = () => setPreview(v => (previewMarkdown = !v));
  const scroller = useRef<HTMLDivElement>(null);
  const anchor = useRef<number | undefined>(undefined);
  const barRow = useRef<HTMLDivElement>(null);
  const [barTop, setBarTop] = useState<number>();
  const go = (i: number, smooth = true) => {
    setHunk(i);
    scroller.current?.querySelector(`[data-hunk-start="${i}"]`)?.scrollIntoView({ block: 'center', behavior: smooth ? 'smooth' : 'auto' });
  };
  // A file opened at a line goes there, else to the change nearest its top
  const placed = useRef<string | undefined>(undefined);
  useLayoutEffect(() => {
    if (!current || placed.current === `${key}\n${p.line ?? ''}`) return;
    placed.current = `${key}\n${p.line ?? ''}`;
    const target = p.selection?.start ?? p.line;
    if (target !== undefined) {
      scroller.current?.querySelector(`[data-line="${target}"]`)?.scrollIntoView({ block: 'center' });
      const near = hunks.findIndex(h => h.at + Math.max(h.add, 1) > target);
      setHunk(near < 0 ? 0 : near);
    } else if (hunks.length) go(0, false);
    else scroller.current?.scrollTo({ top: 0 });
  }, [current, p.line]);
  useLayoutEffect(() => { setBarTop(barRow.current ? barRow.current.offsetTop + barRow.current.offsetHeight : undefined); }, [p.selection, rows]);
  const textOf = (r: LineRange) => (current?.text !== undefined ? linesOf(current.text).slice(r.start - 1, r.end).join('\n') : '');
  // The picked range goes out as the live selection while it stays picked
  useEffect(() => { p.onSelectionText(p.selection, p.selection ? textOf(p.selection) : ''); }, [p.selection?.start, p.selection?.end, current]);
  useEffect(() => () => p.onSelectionText(undefined, ''), []);
  const pick = (line: number, extend: boolean) => {
    const s = p.selection;
    if (extend && anchor.current !== undefined) p.onSelect({ start: Math.min(anchor.current, line), end: Math.max(anchor.current, line) });
    else { anchor.current = line; p.onSelect(s && s.start === line && s.end === line ? undefined : { start: line, end: line }); }
  };
  const dir = dirOf(p.path);
  const blocks = useMemo(() => {
    const out: ViewRow[][] = [];
    for (let i = 0; i < rows.length; i += BLOCK) out.push(rows.slice(i, i + BLOCK));
    return out;
  }, [rows]);
  const seen = new Set<number>();
  return (
    <div className="@container/viewer flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden" data-viewer>
      {/* A narrow viewer sheds the folder, then open-in-editor and the count; the name truncates last */}
      <div className={VIEW_BAR}>
        <FileGlyph name={p.path} className="text-fg-3" />
        <button type="button" onClick={p.onReveal} title={t('panel.reveal', { path: p.path })} className="flex min-w-0 shrink items-baseline gap-1.5 text-left">
          <span className="min-w-0 truncate text-2 font-medium text-fg-1">{nameOf(p.path)}</span>
          {dir && <span className="min-w-0 truncate text-3 text-fg-3 @max-bar-path/viewer:hidden" dir="rtl"><bdi>{dir}</bdi></span>}
        </button>
        <span className="min-w-0 flex-1" />
        {!previewing && hunks.length > 0 && <>
          <IconButton size="sm" className="shrink-0" title={t('panel.prevChange')} aria-label={t('panel.prevChange')} onClick={() => go((hunk + hunks.length - 1) % hunks.length)}><ChevronUp strokeWidth={1.5} /></IconButton>
          <span className="min-w-[3ch] shrink-0 text-center text-3 text-fg-2 tabular-nums @max-bar-count/viewer:hidden">{Math.min(hunk, hunks.length - 1) + 1}/{hunks.length}</span>
          <IconButton size="sm" className="shrink-0" title={t('panel.nextChange')} aria-label={t('panel.nextChange')} onClick={() => go((hunk + 1) % hunks.length)}><ChevronDown strokeWidth={1.5} /></IconButton>
        </>}
        <IconButton size="sm" className="shrink-0 disabled:pointer-events-none disabled:opacity-40" title={t('panel.wrap')} aria-label={t('panel.wrap')} aria-pressed={wrap} disabled={previewing} onClick={() => setWrap(w => !w)}><WrapText strokeWidth={1.5} /></IconButton>
        {/* The one toggle here with a pressed fill: the view changes wholesale, so its state has to read at a glance */}
        {markdown && <IconButton size="sm" className="shrink-0 aria-pressed:bg-active aria-pressed:text-fg-1" title={t('panel.markdownPreview')} aria-label={t('panel.markdownPreview')} aria-pressed={preview} onClick={togglePreview}><BookOpen strokeWidth={1.5} /></IconButton>}
        {/* Open-in-editor stands after the view toggles, only the close button past it */}
        {p.onOpenEditor && <IconButton size="sm" className="shrink-0 @max-bar-open/viewer:hidden" title={t('panel.openFile')} aria-label={t('panel.openFile')} onClick={() => p.onOpenEditor!(p.selection?.start ?? p.line)}><ExternalLink strokeWidth={1.5} /></IconButton>}
        {p.onClose && <CloseView onClose={p.onClose} />}
      </div>
      <div className="relative min-h-0 flex-1">
        {current?.error !== undefined
          ? <div className="flex h-full items-center justify-center px-page text-center text-2 text-fg-3">{readError(current.error)}</div>
          : previewing ? <MarkdownPreview text={current.text!} onOpenLink={p.onOpenLink} />
          : (
            <div ref={scroller} className="scroll-thin absolute inset-0 overflow-auto py-gap font-mono text-mono" data-code>
              <div className={cn('relative', wrap ? 'w-full' : 'w-max min-w-full')}>
                {blocks.map((block, b) => (
                  <div key={b} className="code-block" style={{ '--code-block-rows': block.length } as CSSProperties}>
                    {block.map((r, k) => {
                      const i = b * BLOCK + k;
                      const selected = !!p.selection && r.line !== undefined && r.line >= p.selection.start && r.line <= p.selection.end;
                      const first = r.hunk !== undefined && !seen.has(r.hunk) && r.kind !== 'ctx';
                      const hang = (r.text.match(/^[ \t]*/)?.[0].replace(/\t/g, '  ').length ?? 0) + 2;
                      if (first) seen.add(r.hunk!);
                      return (
                        <div key={i} ref={selected && r.line === p.selection!.end ? barRow : undefined}
                          data-kind={r.kind} data-line={r.line} data-hunk-start={first ? r.hunk : undefined}
                          className={cn('code-row group/line relative flex min-h-code-line leading-code-output', selected && 'is-selected')}>
                          <span className="code-bar absolute inset-y-0 left-0 w-code-bar" aria-hidden />
                          {/* A span, not a button: a button centres its text, which put the number in the middle of a wrapped line */}
                          <span aria-hidden onClick={e => r.line && pick(r.line, e.shiftKey)}
                            className="code-num sticky left-0 z-1 w-code-gutter shrink-0 cursor-pointer self-stretch pr-[1ch] text-right tabular-nums select-none">{r.line ?? ''}</span>
                          {/* Not a <code>: the shell's inline-code chip (base.css) would fill and round every line */}
                          <span className={cn('code-text code-syntax block min-w-0 flex-1 pr-pad', wrap ? 'whitespace-pre-wrap [overflow-wrap:anywhere]' : 'whitespace-pre pl-[1ch]')}
                            style={wrap ? { paddingLeft: `${hang + 1}ch`, textIndent: `-${hang}ch` } : undefined}><Code tokens={r.tokens} text={r.text} /></span>
                        </div>
                      );
                    })}
                  </div>
                ))}
                {p.selection && barTop !== undefined && (
                  <div className={cn(popupClass, 'absolute left-code-gutter z-10 mt-gap-half flex-row items-center p-0.5')} style={{ top: barTop }} data-pick-bar>
                    <button type="button" onClick={() => p.onAdd(p.selection!, textOf(p.selection!))}
                      className="inline-flex h-ctl-sm items-center gap-1 whitespace-nowrap rounded-sm px-2 text-3 font-medium text-fg-1 outline-none transition-colors hover:bg-active focus-visible:bg-active [&_svg]:size-icon [&_svg]:text-fg-3">
                      <MessageSquarePlus strokeWidth={1.5} />{t('panel.addToChat')}
                    </button>
                  </div>
                )}
              </div>
            </div>
          )}
        {/* Overview ruler: where the changes sit in the whole file */}
        {!previewing && total > 0 && (
          <div aria-hidden className="pointer-events-none absolute inset-y-gap right-0.5 w-code-ruler">
            {hunks.map((h, i) => (
              <Fragment key={i}>
                {!!h.del?.length && <span className="absolute left-0 h-1 w-code-ruler-mark rounded-full bg-danger/80" style={{ top: `${(Math.min(h.at - 1, total) / total) * 100}%` }} />}
                {h.add > 0 && <span className="absolute right-0 min-h-1 w-code-ruler-mark rounded-full bg-ok/80" style={{ top: `${((h.at - 1) / total) * 100}%`, height: `${(h.add / total) * 100}%` }} />}
              </Fragment>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// Everything the session changed, file after file, in the transcript's own diff blocks; a file name opens it
export function ReviewView({ sessionId, changes, refresh, onOpen, onClose }: { sessionId: string; changes: ProjectChange[]; refresh: number; onOpen: (path: string) => void; onClose?: () => void }) {
  const add = changes.reduce((n, c) => n + c.add, 0), del = changes.reduce((n, c) => n + c.del, 0);
  const scroller = useRef<HTMLDivElement>(null);
  return (
    <div className="@container/viewer flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden" data-viewer data-review>
      <div className={VIEW_BAR}>
        <Layers strokeWidth={1.5} className="size-icon shrink-0 text-fg-3" />
        <span className="min-w-0 truncate text-2 font-medium text-fg-1">{t('panel.changes')}</span>
        <span className="shrink-0 text-3 text-fg-3 @max-bar-files/viewer:hidden">{t('panel.fileCount', { n: changes.length })}</span>
        <span className="shrink-0 text-3 @max-bar-stat/viewer:hidden"><DiffStat add={add} del={del} /></span>
        <span className="min-w-0 flex-1" />
        {onClose && <CloseView onClose={onClose} />}
      </div>
      <div ref={scroller} className="scroll-thin flex min-h-0 flex-1 flex-col gap-pad overflow-auto p-pad">
        {changes.map(c => <ReviewFile key={c.path} sessionId={sessionId} change={c} refresh={refresh} onOpen={onOpen} scroller={scroller} />)}
      </div>
    </div>
  );
}

// Diff lines one card shows; a longer diff ends in a row that opens the file in the viewer, so a generated or vendored
// file of thousands of lines never lands in the page at once
const REVIEW_LINES = 400;
// How far beyond the visible part of the review a card starts reading its file and lays out its diff
const REVIEW_AHEAD = '600px 0px';

// One changed file of the review. It reads and renders only once it comes near the visible part of the list (36 files
// read and laid out together froze the panel on opening); until then it holds the height its diff is expected to take,
// so the scrollbar and the cards below stay put. `shrink-0`: the cards sit in a scrolling flex column, which squeezed
// them to their headers
function ReviewFile({ sessionId, change, refresh, onOpen, scroller }: { sessionId: string; change: ProjectChange; refresh: number; onOpen: (path: string) => void; scroller: RefObject<HTMLDivElement | null> }) {
  const [read, setRead] = useState<FileRead>();
  const [near, setNear] = useState(false);
  const card = useRef<HTMLElement>(null);
  // A file outside the project or gone has nothing to read; its card names it only
  const readable = !change.gone && !/^([a-z]:)?[\\/]/i.test(change.path);
  useEffect(() => {
    const el = card.current;
    if (!el || near) return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(e => e.isIntersecting)) { setNear(true); observer.disconnect(); }
    }, { root: scroller.current, rootMargin: REVIEW_AHEAD });
    observer.observe(el);
    return () => observer.disconnect();
  }, [near]);
  useEffect(() => {
    if (!readable || !near) return;
    let live = true;
    void project.file(sessionId, change.path, 'session').then(r => { if (live) setRead(r); });
    return () => { live = false; };
  }, [sessionId, change.path, change.add, change.del, refresh, near]);
  const all = useMemo(() => (read?.text !== undefined && read.hunks.length ? diffLinesOf(read.text, read.hunks) : undefined), [read]);
  const lines = all && all.length > REVIEW_LINES ? all.slice(0, REVIEW_LINES) : all;
  const rest = all ? all.length - (lines?.length ?? 0) : 0;
  // Until the diff is there: its changed lines plus a hunk header, capped like a shown diff (at least one line)
  const expected = readable && !read ? Math.min(change.add + change.del + 1, REVIEW_LINES) : 0;
  return (
    <section ref={card} className="shrink-0 overflow-hidden rounded-lg border border-conversation-line">
      <button type="button" onClick={() => onOpen(change.path)} disabled={!readable}
        className="flex h-row w-full items-center gap-1.5 bg-bg-1 px-gap text-left transition-colors enabled:hover:bg-hover">
        <FileGlyph name={change.path} className="text-fg-3" />
        <span className={cn('shrink-0 text-2 text-fg-1', change.gone && 'line-through')}>{nameOf(change.path)}</span>
        <span className="min-w-0 flex-1 truncate text-3 text-fg-3">{dirOf(change.path)}</span>
        <span className="shrink-0 text-3"><DiffStat add={change.add} del={change.del} /></span>
      </button>
      {expected > 0 && <div aria-hidden className="review-pending" style={{ '--review-rows': expected } as CSSProperties} />}
      {lines && <DiffBlock lines={lines} path={change.path} />}
      {rest > 0 && (
        <button type="button" onClick={() => onOpen(change.path)}
          className="flex h-row w-full items-center border-t border-conversation-line px-gap text-left text-3 text-fg-3 transition-colors hover:bg-hover hover:text-fg-1">
          {t('panel.reviewMore', { n: rest })}
        </button>
      )}
      {read?.error !== undefined && <div className="px-gap py-gap-half text-3 text-fg-3">{readError(read.error)}</div>}
    </section>
  );
}
