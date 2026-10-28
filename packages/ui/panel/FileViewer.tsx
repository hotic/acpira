import { Fragment, memo, useCallback, useEffect, useEffectEvent, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type RefObject } from 'react';
import { flushSync } from 'react-dom';
import type { DiffLine } from '@acpira/shared/transcript';
import type { ProjectBase, ProjectChange, ProjectHunk } from '@acpira/shared/protocol';
import { BookOpen, ChevronDown, ChevronUp, ExternalLink, FolderTree, Layers, MessageSquarePlus, WrapText, X } from 'lucide-react';
import type { TextBlock } from '@acpira/shared/transcript';
import { t } from '../i18n';
import { useTheme, type Theme } from '../look';
import { IconButton } from '../ui/Button';
import { popupClass } from '../ui/Overlay';
import { cn } from '../ui/cn';
import { DiffBlock } from '../chat/CodeBlock';
import { DiffStat } from '../chat/ToolCall';
import { Prose } from '../chat/Prose';
import { OpenToolFileContext } from '../chat/fileLinks';
import { requestCodeHighlight, requestCodesHighlight } from '../chat/diffHighlight';
import { codeLanguage, type CodeToken } from '../chat/codeLanguage';
import { project, sameRead, shellResizeIdleIn, type FileRead } from './api';
import type { LineRange } from './store';
import { useNearBlocks } from './nearBlocks';
import { previewParts } from './previewParts';
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
  const near = new Uint8Array(rows.length);
  for (let i = 0; i < rows.length; i++) {
    if (rows[i]!.kind === 'ctx') continue;
    const end = Math.min(rows.length, i + 4);
    for (let k = Math.max(0, i - 3); k < end; k++) near[k] = 1;
  }
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

// Capped diff slice for ReviewFile: avoids allocating thousands of DiffLine objects past REVIEW_LINES when a new/large file has thousands of lines
function reviewDiffSlice(text: string, hunks: ProjectHunk[], cap: number): { lines: DiffLine[]; rest: number } | undefined {
  if (!hunks.length) return undefined;
  // Fast path for a whole-file addition (e.g. a newly created file of thousands of lines)
  if (hunks.length === 1 && hunks[0]!.at === 1 && !hunks[0]!.del?.length) {
    const raw = text ? linesOf(text) : [];
    const total = hunks[0]!.add;
    if (total === raw.length) {
      const take = Math.min(total, Math.max(0, cap - 1));
      const lines: DiffLine[] = [{ kind: 'hunk', text: `@@ -1,0 +1,${total} @@` }];
      for (let i = 0; i < take; i++) lines.push({ kind: 'add', text: `+${raw[i]!}`, newLine: i + 1 });
      return { lines, rest: total + 1 - lines.length };
    }
  }
  const all = diffLinesOf(text, hunks);
  if (!all.length) return undefined;
  const lines = all.length > cap ? all.slice(0, cap) : all;
  return { lines, rest: all.length - lines.length };
}
const HIGHLIGHT_MAX = 256 * 1024;

// Syntax colours for the file and each hunk's removed lines, from the transcript's highlight worker, in the scheme on
// screen (`theme`)
function useHighlight(path: string, read: FileRead | undefined, theme: Theme) {
  // `done`: the whole file is coloured, not only its first parts
  const [colored, setColored] = useState<{ path: string; read: FileRead; theme: Theme; after: CodeToken[][]; removed: CodeToken[][][]; done: boolean }>();
  const last = useRef(colored);
  last.current = colored;
  useEffect(() => {
    const text = read?.text;
    const language = codeLanguage(path);
    if (text === undefined || !language || text.length > HIGHLIGHT_MAX) return;
    // The file the user opened goes ahead of the transcript's highlighting, and a file left (or read again) stops its own
    const ctrl = new AbortController();
    const opts = { signal: ctrl.signal, first: true };
    let timer: ReturnType<typeof setTimeout> | undefined;
    // The same text read again (the diff base switched, a turn ended) keeps the colours it has: all of them once done
    // (only its removed lines are asked), else the lines coloured so far until the new run passes them
    const had = last.current;
    const same = had && had.path === path && had.read.text === text && had.theme === theme ? had : undefined;
    const kept = same?.done ? same.after : undefined;
    const before = same?.after ?? [];
    // A large file takes its colours from the top as they come (a part skipped during a resize is in the next one)
    let shown: CodeToken[][] = [];
    const onPart = (part: CodeToken[][]) => {
      // Lines the run before coloured keep their token arrays (the same text in the same scheme), so their blocks of
      // rows do not render again
      shown = shown.concat(before.length > shown.length ? part.map((line, i) => before[shown.length + i] ?? line) : part);
      const after = shown.length < before.length ? shown.concat(before.slice(shown.length)) : shown;
      if (!ctrl.signal.aborted && shellResizeIdleIn() <= 0) setColored({ path, read: read!, theme, after, removed: [], done: false });
    };
    // Every hunk's removed lines in one request, so a file with many hunks costs one run beside its text
    const hunks = read!.hunks;
    const removedTexts = hunks.filter(h => h.del?.length).map(h => h.del!.join('\n'));
    const removed = removedTexts.length
      ? requestCodesHighlight(removedTexts, language, theme, opts).then(all => {
        let k = 0;
        return hunks.map(h => (h.del?.length ? all[k++]! : []));
      })
      : Promise.resolve(hunks.map(() => []));
    void Promise.all([
      kept ? Promise.resolve(kept) : requestCodeHighlight(text.replace(/\r?\n$/, ''), language, theme, { ...opts, onPart }),
      removed,
    ]).then(([all, removed]) => {
      // Lines already shown, or coloured by the run before over the same text, keep their token arrays, so their blocks
      // of rows do not render again
      const after = shown.length || before.length ? all.map((line, i) => shown[i] ?? before[i] ?? line) : all;
      const apply = () => {
        if (ctrl.signal.aborted) return;
        const wait = shellResizeIdleIn();
        if (wait > 0) { timer = setTimeout(apply, wait); return; }
        setColored({ path, read: read!, theme, after, removed, done: true });
      };
      apply();
    }).catch(() => { /* plain text stays */ });
    return () => { ctrl.abort(); if (timer !== undefined) clearTimeout(timer); };
  }, [path, read, theme]);
  return useMemo(() => {
    // Colours of the other scheme are none in this one: plain text until it is coloured again
    if (!colored || colored.path !== path || colored.theme !== theme) return undefined;
    if (colored.read === read) return colored;
    // Keep the file's syntax colours while updated diff hunks are being highlighted, so the code never flashes plain text
    return colored.read.text === read?.text ? { ...colored, removed: [] } : undefined;
  }, [colored, path, read, theme]);
}

function Code({ tokens, text, light }: { tokens?: CodeToken[]; text: string; light: boolean }) {
  if (!tokens) return <>{text || '​'}</>;
  return <>{tokens.length ? tokens.map((tok, i) => {
    const color = light ? tok.light : tok.dark;
    return color ? <span key={i} style={{ color }}>{tok.text}</span> : tok.text;
  }) : '​'}</>;
}

// A read error as the viewer says it
export function readError(error: string): string {
  if (error === 'missing') return t('panel.error.missing');
  if (error === 'binary') return t('panel.error.binary');
  if (error === 'tooLarge') return t('panel.error.tooLarge');
  if (error === 'outside') return t('panel.error.outside');
  return t('panel.error.other', { error });
}

export const VIEW_BAR = 'flex h-[calc(var(--ctl)+var(--spacing-gap))] shrink-0 items-center gap-1 border-b border-line pr-gap pl-pad';
// Files the viewer can show rendered as well as as source
const isMarkdown = (path: string) => /\.(md|markdown|mdx)$/i.test(path);

// The last preview choice, carried to the next Markdown file this page opens
let previewMarkdown = false;
let previewAsk = 0;

// Opening a plan document switches the viewer into Markdown preview
export function preferMarkdownPreview() {
  previewMarkdown = true;
  previewAsk++;
}

// YAML front matter (a skill's `name` / `description`) would render as a rule and a setext heading; it shows as a YAML block
function previewText(text: string): string {
  const front = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/.exec(text);
  return front ? `\`\`\`yaml\n${front[1]}\n\`\`\`\n\n${text.slice(front[0].length)}` : text;
}

const PreviewPart = memo(function PreviewPart({ markdown, part }: { markdown: string; part?: boolean }) {
  const block = useMemo<TextBlock>(() => ({ type: 'text', markdown }), [markdown]);
  const ref = useRef<HTMLDivElement>(null);
  // Once laid out at its real height, a part out of sight skips style, layout and paint (panel.css) and keeps that
  // height, so the document's stays exact. Skipped from the start, it would stand at an estimate until scrolled to
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!part || seen || !el) return;
    // The first observation comes after the part's first layout, when its size is also remembered
    const observer = new ResizeObserver(() => { observer.disconnect(); setSeen(true); });
    observer.observe(el);
    return () => observer.disconnect();
  }, [part, seen]);
  // prose.css gives a heading that opens a later part the gap it has in the whole
  return part ? <div ref={ref} data-preview-part data-seen={seen || undefined}><Prose block={block} /></div> : <Prose block={block} />;
});

// The file rendered the way a reply is; links go through `onOpenLink`, relative ones from the file's folder
function MarkdownPreview({ text, onOpenLink }: { text: string; onOpenLink?: (path: string, line?: number) => void }) {
  const parts = useMemo(() => previewParts(previewText(text)), [text]);
  // A new text of the same file keeps the parts already shown, so a live edit neither shrinks the preview under its
  // scroll nor renders it all in one frame; an unchanged part does not render again
  const [shown, setShown] = useState(1);
  useEffect(() => {
    if (shown >= parts.length) return;
    const frame = requestAnimationFrame(() => setShown(shown + 1));
    return () => cancelAnimationFrame(frame);
  }, [shown, parts.length]);
  return (
    <OpenToolFileContext.Provider value={onOpenLink}>
      <div className="scroll-thin absolute inset-0 overflow-auto px-pad py-pad" data-markdown-preview>
        {parts.length === 1 ? <PreviewPart markdown={parts[0]!} /> : (
          <div className="flex min-w-0 flex-col gap-gap">
            {parts.slice(0, shown).map((markdown, i) => <PreviewPart key={i} markdown={markdown} part />)}
          </div>
        )}
      </div>
    </OpenToolFileContext.Provider>
  );
}

// Rows laid out a block at a time (panel.css .code-block): a long file skips layout and paint for what is out of sight.
// Kept small (40 rows) so each block entering the viewport lays out within the 6.25 ms 160 Hz frame budget in WebKit.
const BLOCK = 40;
// How long a jump keeps re-centring its target while the blocks around it take their real heights (centerRow)
const SETTLE_MS = 400;
// How long the viewer's width stays put before its wrapped rows are measured again (a resize in progress keeps the last)
const FIT_SETTLE_MS = 150;

export interface FileViewerProps {
  sessionId: string;
  path: string;
  // Inline text for a plan document whose file is not inside the project folder
  text?: string;
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
  onReveal?: () => void;
  onOpenEditor?: (line?: number) => void;
  // A link in the Markdown preview: a path as written in the file (relative to its folder, or absolute)
  onOpenLink?: (path: string, line?: number) => void;
  // Toggles the project's file tree beside the viewer (the tool panel's Files tab)
  treeOpen?: boolean;
  onToggleTree?: () => void;
  // Closes the viewer, leaving the tree alone (the tool panel's Files tab; the files layout closes its file tabs instead)
  onClose?: () => void;
}

interface CodeRowsProps {
  rows: ViewRow[];
  // The index of the block's first row among the file's rows
  start: number;
  // The first changed row of each change: the navigator's targets
  hunkStart: ReadonlyMap<number, number>;
  wrap: boolean;
  light: boolean;
  // The picked range, when it reaches into this block
  selStart?: number;
  selEnd?: number;
  barRow: RefObject<HTMLDivElement | null>;
  onPick: (line: number, extend: boolean) => void;
}

// One block's rows: a pick, or the colours arriving, re-renders only the blocks that hold rows and that it touches
// Rows are built again whenever colours arrive; a block whose rows show the same text and tokens stays as it is
const sameRow = (a: ViewRow, b: ViewRow) => a === b
  || (a.kind === b.kind && a.line === b.line && a.text === b.text && a.tokens === b.tokens && a.hunk === b.hunk);
const sameRowsProps = (a: CodeRowsProps, b: CodeRowsProps) => a.start === b.start && a.hunkStart === b.hunkStart
  && a.wrap === b.wrap && a.light === b.light && a.selStart === b.selStart && a.selEnd === b.selEnd && a.barRow === b.barRow
  && a.onPick === b.onPick && a.rows.length === b.rows.length && a.rows.every((r, i) => sameRow(r, b.rows[i]!));

const CodeRows = memo(function CodeRows({ rows, start, hunkStart, wrap, light, selStart, selEnd, barRow, onPick }: CodeRowsProps) {
  return <>{rows.map((r, k) => {
    const i = start + k;
    const selected = selStart !== undefined && selEnd !== undefined && r.line !== undefined && r.line >= selStart && r.line <= selEnd;
    const first = r.hunk !== undefined && hunkStart.get(r.hunk) === i;
    const hang = wrap ? hangOf(r.text) : 0;
    return (
      <div key={i} ref={selected && r.line === selEnd ? barRow : undefined}
        data-kind={r.kind} data-line={r.line} data-hunk-start={first ? r.hunk : undefined}
        className={cn('code-row group/line relative flex min-h-code-line leading-code-output', selected && 'is-selected')}>
        {(r.kind !== 'ctx' || selected) && <span className="code-bar absolute inset-y-0 left-0 w-code-bar" aria-hidden />}
        {/* A span, not a button: a button centres its text, which put the number in the middle of a wrapped line */}
        <span aria-hidden onClick={e => r.line && onPick(r.line, e.shiftKey)}
          className="code-num w-code-gutter shrink-0 cursor-pointer self-stretch pr-[1ch] text-right tabular-nums select-none">{r.line ?? ''}</span>
        {/* Not a <code>: the shell's inline-code chip (base.css) would fill and round every line */}
        <span className={cn('code-text code-syntax block min-w-0 flex-1 pr-pad', wrap ? 'whitespace-pre-wrap [overflow-wrap:anywhere]' : 'whitespace-pre pl-[1ch]')}
          style={wrap ? { paddingLeft: `${hang + 1}ch`, textIndent: `-${hang}ch` } : undefined}><Code tokens={r.tokens} text={r.text} light={light} /></span>
      </div>
    );
  })}</>;
}, sameRowsProps);

// A wrapped row's continuation lines hang two columns past its own indentation (a tab counts two)
const hangOf = (text: string) => (text.match(/^[ \t]*/)?.[0].replace(/\t/g, '  ').length ?? 0) + 2;
// Characters two columns wide: CJK, Hangul, fullwidth forms (an astral one, an emoji, is two code units already)
const WIDE = /[\u1100-\u115f\u2e80-\u303e\u3041-\u33ff\u3400-\u4dbf\u4e00-\u9fff\ua000-\ua4cf\uac00-\ud7a3\uf900-\ufaff\ufe30-\ufe4f\uff00-\uff60\uffe0-\uffe6]/g;
// The lines a row takes wrapped at `cols` columns: a tab is taken as 8 (the CSS default), wrapping at spaces left aside
function rowLines(text: string, cols: number): number {
  let width = text.length;
  if (text.includes('\t')) width += 7 * (text.split('\t').length - 1);
  if (/[^\x00-\x7f]/.test(text)) width += text.match(WIDE)?.length ?? 0;
  if (width <= cols) return 1;
  return 1 + Math.ceil((width - cols) / Math.max(1, cols - hangOf(text)));
}

// The viewer bar's last button: never shed by a narrow bar, so a file can always be put away
function CloseView({ onClose }: { onClose: () => void }) {
  return <IconButton size="sm" className="shrink-0" title={t('common.close')} aria-label={t('common.close')} onClick={onClose}><X strokeWidth={1.5} /></IconButton>;
}

export function FileViewer(p: FileViewerProps) {
  const theme = useTheme();
  const light = theme === 'light';
  const [read, setRead] = useState<FileRead & { path: string; base: ProjectBase }>();
  const inline = useMemo<FileRead | undefined>(() => (p.text !== undefined ? { text: p.text, hunks: [] } : undefined), [p.text]);
  const sessionRef = useRef(p.sessionId);
  sessionRef.current = p.sessionId;
  // Against `head` the file and its diff belong to the project, not to a conversation, so switching sessions does not
  // re-read the file; against `session` the hunks are that conversation's own
  const readTarget = p.base === 'session' ? `${p.path}\n${p.base}\n${p.sessionId}` : `${p.path}\n${p.base}`;
  useEffect(() => {
    if (p.text !== undefined) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ask = (sid: string) => {
      void project.file(sid, p.path, p.base).then(r => {
        const apply = () => {
          if (!live) return;
          if ((r.error === 'noProject' || r.error === 'timeout') && sessionRef.current !== sid) {
            ask(sessionRef.current);
            return;
          }
          const wait = shellResizeIdleIn();
          if (wait > 0) { timer = setTimeout(apply, wait); return; }
          setRead(prev => {
            if ((r.error === 'noProject' || r.error === 'timeout') && prev?.path === p.path && prev.text !== undefined) return prev;
            if (prev && prev.path === p.path && prev.base === p.base && prev.text === r.text && prev.error === r.error && sameRead(prev.hunks, r.hunks)) return prev;
            return { ...r, path: p.path, base: p.base };
          });
        };
        apply();
      });
    };
    ask(sessionRef.current);
    return () => { live = false; if (timer !== undefined) clearTimeout(timer); };
  }, [readTarget, p.refresh, p.text !== undefined]);
  // Keep showing the file's text while its diff base switches between `session` and `head`, so the viewer never blanks
  const current = inline ?? (read?.path === p.path ? read : undefined);
  const colored = useHighlight(p.path, current, theme);
  const hunks = current?.hunks ?? [];
  // The hunk starts come from the rows without colours, so colours arriving (in parts for a large file) keep them, and
  // with them every block of rows whose tokens did not change (CodeRows)
  const plainRows = useMemo(() => (current?.text !== undefined ? viewRows(current.text, current.hunks) : []), [current]);
  const rows = useMemo(() => (current?.text !== undefined && colored ? viewRows(current.text, current.hunks, colored.after, colored.removed) : plainRows),
    [current, colored, plainRows]);
  const hunkStart = useMemo(() => {
    const at = new Map<number, number>();
    plainRows.forEach((r, i) => { if (r.hunk !== undefined && r.kind !== 'ctx' && !at.has(r.hunk)) at.set(r.hunk, i); });
    return at;
  }, [plainRows]);
  // Blocks get their rows near the visible part (nearBlocks.ts); the top ones from the start
  const { near, watch, add: addBlocks } = useNearBlocks('[data-code]', 3);
  const blockOf = (row: number) => Math.floor(row / BLOCK);
  const around = (row: number) => [blockOf(row) - 1, blockOf(row), blockOf(row) + 1];
  // Counted once per text: the viewer renders again for every block that comes near, and a 1 MB file split each time
  const total = useMemo(() => (current?.text !== undefined ? linesOf(current.text).length : 0), [current?.text]);
  const initialTarget = p.selection?.start ?? p.line;
  const [hunk, setHunk] = useState(() => {
    if (initialTarget === undefined || !hunks.length) return 0;
    const near = hunks.findIndex(h => h.at + Math.max(h.add, 1) > initialTarget);
    return near < 0 ? 0 : near;
  });
  // Soft wrap is the default in a side panel; a continuation line hangs two columns past the line's own indentation
  const [wrap, setWrap] = useState(true);
  const markdown = isMarkdown(p.path) || p.text !== undefined;
  const [preview, setPreview] = useState(previewMarkdown);
  const lastAsk = useRef(previewAsk);
  if (lastAsk.current !== previewAsk) {
    lastAsk.current = previewAsk;
    if (markdown && !preview) setPreview(true);
  }
  const previewing = markdown && preview && current?.text !== undefined;
  const togglePreview = () => setPreview(v => (previewMarkdown = !v));
  const scroller = useRef<HTMLDivElement>(null);
  const anchor = useRef<number | undefined>(undefined);
  const barRow = useRef<HTMLDivElement>(null);
  const [barTop, setBarTop] = useState<number>();
  // Blocks of rows out of sight hold an estimated height, which wrapped lines can still miss: blocks near a jump's
  // target take their real height once in sight and push the target away (at one line a row, a jump to a change far
  // down landed thousands of pixels off it). A jump beyond a screen is made at once, then the target is centred again
  // each frame while it still moves, until the reader scrolls or SETTLE_MS pass. The reader is told by their input, not
  // by scrollTop: scroll anchoring moves it as the blocks above settle
  const settling = useRef(0);
  const centerRow = (sel: string, smooth = false) => {
    const sc = scroller.current;
    if (!sc) return;
    const wanted = () => {
      const row = sc.querySelector<HTMLElement>(sel);
      if (!row) return undefined;
      const block = row.parentElement;
      const top = (block?.classList.contains('code-block') ? block.offsetTop : 0) + row.offsetTop;
      return Math.max(0, Math.min(sc.scrollHeight - sc.clientHeight, top - sc.clientHeight / 2 + row.offsetHeight / 2));
    };
    const top = wanted();
    if (top === undefined) return;
    const run = ++settling.current;
    if (smooth && Math.abs(top - sc.scrollTop) <= sc.clientHeight) {
      sc.scrollTo({ top, behavior: 'smooth' });
      return;
    }
    sc.scrollTo({ top });
    let reader = false;
    const input = () => { reader = true; };
    const inputs = ['wheel', 'pointerdown', 'touchstart', 'keydown'] as const;
    for (const type of inputs) sc.addEventListener(type, input, { passive: true });
    const end = () => { for (const type of inputs) sc.removeEventListener(type, input); };
    let still = 0;
    const until = performance.now() + SETTLE_MS;
    const step = () => {
      const next = reader || settling.current !== run || !sc.isConnected || performance.now() > until ? undefined : wanted();
      if (next === undefined) return end();
      if (Math.abs(next - sc.scrollTop) > 1) {
        sc.scrollTo({ top: next });
        still = 0;
      } else if (++still >= 3) return end();
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  };
  const go = (i: number, smooth = true) => {
    setHunk(i);
    // A change in a block without its rows yet: they are rendered first, so the row is there to scroll to
    const row = hunkStart.get(i);
    if (row !== undefined && !near.has(blockOf(row))) flushSync(() => addBlocks(around(row)));
    centerRow(`[data-hunk-start="${i}"]`, smooth);
  };
  // A file opened at a line goes there, else to the change nearest its top
  const placed = useRef<string | undefined>(undefined);
  useLayoutEffect(() => {
    // A Markdown file previewed has no rows to place: it is placed once its source shows
    if (!current || previewing || placed.current === `${p.path}\n${p.line ?? ''}`) return;
    const target = p.selection?.start ?? p.line;
    // The row it goes to needs its block's rows: they are added and this runs again before the frame is painted
    const row = target !== undefined ? rows.findIndex(r => r.line === target) : (hunkStart.get(0) ?? -1);
    if (row >= 0 && !near.has(blockOf(row))) { addBlocks(around(row)); return; }
    placed.current = `${p.path}\n${p.line ?? ''}`;
    if (target !== undefined) {
      centerRow(`[data-line="${target}"]`, false);
      const nearHunk = hunks.findIndex(h => h.at + Math.max(h.add, 1) > target);
      const nextHunk = nearHunk < 0 ? 0 : nearHunk;
      setHunk(cur => (cur === nextHunk ? cur : nextHunk));
    } else if (hunks.length) go(0, false);
    else scroller.current?.scrollTo({ top: 0 });
  }, [current, p.line, near, previewing]);
  const content = useRef<HTMLDivElement>(null);
  const placeBar = useCallback(() => {
    const r = barRow.current;
    const block = r?.parentElement;
    setBarTop(r ? (block?.classList.contains('code-block') ? block.offsetTop : 0) + r.offsetTop + r.offsetHeight : undefined);
  }, []);
  // Blocks getting their rows (the picked range's end among them) or a change of wrapping move the row
  useLayoutEffect(placeBar, [p.selection, rows, near, wrap]);
  // So do blocks above it taking their real height when first laid out (content-visibility), which no render marks
  const picked = p.selection !== undefined;
  useEffect(() => {
    const el = content.current;
    if (!picked || !el) return;
    const ro = new ResizeObserver(placeBar);
    ro.observe(el);
    return () => ro.disconnect();
  }, [picked, previewing, current?.error, placeBar]);
  const textOf = (r: LineRange) => (current?.text !== undefined ? linesOf(current.text).slice(r.start - 1, r.end).join('\n') : '');
  // The picked range goes out as the live selection while it stays picked
  useEffect(() => { p.onSelectionText(p.selection, p.selection ? textOf(p.selection) : ''); }, [p.selection?.start, p.selection?.end, current]);
  useEffect(() => () => p.onSelectionText(undefined, ''), []);
  const pick = (line: number, extend: boolean) => {
    const s = p.selection;
    if (extend && anchor.current !== undefined) p.onSelect({ start: Math.min(anchor.current, line), end: Math.max(anchor.current, line) });
    else { anchor.current = line; p.onSelect(s && s.start === line && s.end === line ? undefined : { start: line, end: line }); }
  };
  // Steady for the blocks' memo, current when called
  const pickRef = useRef(pick);
  pickRef.current = pick;
  const onPick = useCallback((line: number, extend: boolean) => pickRef.current(line, extend), []);
  const dir = dirOf(p.path);
  const blocks = useMemo(() => {
    const out: ViewRow[][] = [];
    for (let i = 0; i < rows.length; i += BLOCK) out.push(rows.slice(i, i + BLOCK));
    return out;
  }, [rows]);
  // Wrapped, a row takes as many lines as its text needs at the viewer's width, and a block out of sight holds that
  // estimate (`--code-block-lines`): at one line a row, blocks came out a few times too short, so the scrollbar misled
  // and a drag to the end stopped short as the blocks above took their height. The columns come from a laid-out row (the
  // line numbers' column is --code-gutter wide in the code's own `ch`), again once a resize settles
  const [cols, setCols] = useState<number>();
  const measured = useRef<number | undefined>(undefined);
  const hasRows = rows.length > 0;
  useLayoutEffect(() => {
    const sc = scroller.current;
    if (!wrap || !sc) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // The first row's number is as wide as the gutter's columns: watched too, it tells a change of code font or size,
    // which leaves the scroller as it was
    let watched: HTMLElement | null = null;
    const ro = new ResizeObserver(() => { clearTimeout(timer); timer = setTimeout(() => measure(true), FIT_SETTLE_MS); });
    const measure = (settled = false) => {
      const num = sc.querySelector<HTMLElement>('.code-row .code-num');
      if (num !== watched) {
        if (watched) ro.unobserve(watched);
        if (num) ro.observe(num);
        watched = num;
      }
      const text = num?.nextElementSibling as HTMLElement | null | undefined;
      // A viewer out of the layout (a narrow window's panel, closed) has no width: the columns it had hold
      if (!num || !text || !text.clientWidth) return;
      const ch = num.getBoundingClientRect().width / (parseFloat(getComputedStyle(num).getPropertyValue('--code-gutter')) || 6);
      if (!(ch > 0)) return;
      // The first line starts a column in: the hanging indent's padding less its negative text indent
      const next = Math.max(8, Math.floor((text.clientWidth - parseFloat(getComputedStyle(text).paddingRight)) / ch) - 1);
      if (next === measured.current) return;
      measured.current = next;
      if (!settled) { setCols(next); return; }
      // The blocks out of sight take new heights, which the browser's scroll anchoring holds the view through only at
      // times: a jump made while the width still moved (the viewer growing in beside the tree) was carried thousands of
      // pixels off its target. The row in the middle of the view is put back where it was; one anchoring held stays put
      const box = sc.getBoundingClientRect();
      const mid = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2)?.closest('.code-row, .code-block');
      const top = mid && sc.contains(mid) ? mid.getBoundingClientRect().top : undefined;
      flushSync(() => setCols(next));
      if (top !== undefined && mid!.isConnected) sc.scrollTop += mid!.getBoundingClientRect().top - top;
    };
    ro.observe(sc);
    measure();
    return () => { ro.disconnect(); clearTimeout(timer); };
  }, [wrap, previewing, current?.error, hasRows]);
  // From the rows without colours, the same text: colours arriving in parts leave the estimates as they are
  const blockLines = useMemo(() => {
    const out: number[] = [];
    for (let at = 0; at < plainRows.length; at += BLOCK) {
      const end = Math.min(at + BLOCK, plainRows.length);
      let n = end - at;
      if (wrap && cols !== undefined) {
        n = 0;
        for (let i = at; i < end; i++) n += rowLines(plainRows[i]!.text, cols);
      }
      out.push(n);
    }
    return out;
  }, [plainRows, wrap, cols]);
  // A selection reaching across blocks that never came near holds none of their text (they have no rows yet): its copy is
  // made from the file's rows instead, from where it starts to where it ends. One dragged on past the viewer (over the
  // tree) copies its part of the file, to the file's end: a native copy would drop every unrendered line between. A copy
  // fires where the selection starts, so one dragged in from before the viewer never reaches the scroller: the document
  // hears every copy
  const copy = useEffectEvent((e: globalThis.ClipboardEvent) => {
    const s = getSelection();
    const range = s?.rangeCount ? s.getRangeAt(0) : undefined;
    const sc = scroller.current;
    if (!sc || !e.clipboardData || !range || range.collapsed || !range.intersectsNode(sc)) return;
    if (![...sc.querySelectorAll('.code-block[data-pending]')].some(b => range.intersectsNode(b))) return;
    // A boundary as a place in the file's rows: the row's index, and how much of its text comes before the boundary.
    // A boundary on an element sits before its child `offset`
    const place = (node: Node, offset: number): { row: number; at: number } => {
      const el = node instanceof Element ? node : node.parentElement;
      const row = el?.closest<HTMLElement>('.code-row');
      const block = (row?.parentElement ?? el?.closest('.code-block')) as HTMLElement | null;
      if (!block) {
        // Between blocks on the rows' wrapper, or before or after them all on the scroller
        const b = node === content.current ? Math.min(offset, blocks.length) : offset ? blocks.length : 0;
        return { row: b * BLOCK, at: 0 };
      }
      const b = Number(block.dataset.block);
      if (!row) return { row: b * BLOCK + (node === block ? offset : 0), at: 0 };
      const index = b * BLOCK + Array.prototype.indexOf.call(block.children, row);
      const text = row.querySelector('.code-text');
      if (text?.contains(node)) {
        const before = document.createRange();
        before.setStart(text, 0);
        before.setEnd(node, offset);
        return { row: index, at: before.toString().length };
      }
      // On the row itself or its number: past the text is the row's end
      const past = node === row && !!text && offset > Array.prototype.indexOf.call(row.childNodes, text);
      return { row: index, at: past ? Infinity : 0 };
    };
    const from = sc.contains(range.startContainer) ? place(range.startContainer, range.startOffset) : { row: 0, at: 0 };
    let to = sc.contains(range.endContainer) ? place(range.endContainer, range.endOffset) : { row: rows.length - 1, at: Infinity };
    if (to.row >= rows.length) to = { row: rows.length - 1, at: Infinity };
    if (from.row >= rows.length || to.row < from.row) return;
    const lines = rows.slice(from.row, to.row + 1).map(r => r.text);
    lines[lines.length - 1] = lines.at(-1)!.slice(0, to.at);
    lines[0] = lines[0]!.slice(from.at);
    e.clipboardData.setData('text/plain', lines.join('\n'));
    e.preventDefault();
  });
  useEffect(() => {
    const onCopy = (e: globalThis.ClipboardEvent) => copy(e);
    document.addEventListener('copy', onCopy);
    return () => document.removeEventListener('copy', onCopy);
  }, []);
  const sel = p.selection;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden" data-viewer>
      {/* A narrow viewer sheds the folder, then open-in-editor and the count; the name truncates last */}
      <div className={cn(VIEW_BAR, '@container/viewer')}>
        <FileGlyph name={p.path} className="text-fg-3" />
        <button type="button" onClick={p.onReveal} disabled={!p.onReveal} title={p.onReveal ? t('panel.reveal', { path: p.path }) : p.path} className="flex min-w-0 shrink items-baseline gap-1.5 text-left">
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
        {p.onToggleTree && <IconButton size="sm" className="shrink-0 aria-pressed:bg-active aria-pressed:text-fg-1" title={t('settings.plugins.files')} aria-label={t('settings.plugins.files')} aria-pressed={p.treeOpen} onClick={p.onToggleTree}><FolderTree strokeWidth={1.5} /></IconButton>}
        {/* Open-in-editor stands after the view toggles, only the close button past it */}
        {p.onOpenEditor && <IconButton size="sm" className="shrink-0 @max-bar-open/viewer:hidden" title={t('panel.openFile')} aria-label={t('panel.openFile')} onClick={() => p.onOpenEditor!(p.selection?.start ?? p.line)}><ExternalLink strokeWidth={1.5} /></IconButton>}
        {p.onClose && <CloseView onClose={p.onClose} />}
      </div>
      <div className="relative min-h-0 flex-1">
        {current?.error !== undefined
          ? <div className="flex h-full items-center justify-center px-page text-center text-2 text-fg-3">{readError(current.error)}</div>
          : previewing ? <MarkdownPreview text={current.text!} onOpenLink={p.onOpenLink} />
          : (
            <div ref={scroller} className="scroll-thin absolute inset-0 overflow-auto py-gap font-mono text-mono" data-code
              onWheel={e => { if (!wrap && e.deltaX > 0) e.currentTarget.setAttribute('data-scrolled-x', ''); }}
              onScroll={e => e.currentTarget.toggleAttribute('data-scrolled-x', !wrap && e.currentTarget.scrollLeft > 0)}>
              <div ref={content} className={cn('relative', wrap ? 'w-full' : 'w-max min-w-full')}>
                {blocks.map((block, b) => {
                  const style = { '--code-block-lines': blockLines[b] } as CSSProperties;
                  if (!near.has(b)) return <div key={b} ref={watch} data-block={b} data-pending className="code-block" style={style} />;
                  const touched = !!sel && block.some(r => r.line !== undefined && r.line >= sel.start && r.line <= sel.end);
                  return (
                    <div key={b} data-block={b} className="code-block" style={style}>
                      <CodeRows rows={block} start={b * BLOCK} hunkStart={hunkStart} wrap={wrap} light={light}
                        selStart={touched ? sel.start : undefined} selEnd={touched ? sel.end : undefined} barRow={barRow} onPick={onPick} />
                    </div>
                  );
                })}
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
export function ReviewView({ sessionId, changes, refresh, onOpen, treeOpen, onToggleTree, onClose }: { sessionId: string; changes: ProjectChange[]; refresh: number; onOpen: (path: string) => void; treeOpen?: boolean; onToggleTree?: () => void; onClose?: () => void }) {
  const add = changes.reduce((n, c) => n + c.add, 0), del = changes.reduce((n, c) => n + c.del, 0);
  const scroller = useRef<HTMLDivElement>(null);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden" data-viewer data-review>
      <div className={cn(VIEW_BAR, '@container/viewer')}>
        <Layers strokeWidth={1.5} className="size-icon shrink-0 text-fg-3" />
        <span className="min-w-0 truncate text-2 font-medium text-fg-1">{t('panel.changes')}</span>
        <span className="shrink-0 text-3 text-fg-3 @max-bar-files/viewer:hidden">{t('panel.fileCount', { n: changes.length })}</span>
        <span className="shrink-0 text-3 @max-bar-stat/viewer:hidden"><DiffStat add={add} del={del} /></span>
        <span className="min-w-0 flex-1" />
        {onToggleTree && <IconButton size="sm" className="shrink-0 aria-pressed:bg-active aria-pressed:text-fg-1" title={t('settings.plugins.files')} aria-label={t('settings.plugins.files')} aria-pressed={treeOpen} onClick={onToggleTree}><FolderTree strokeWidth={1.5} /></IconButton>}
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
  const near = useRef(false);
  const card = useRef<HTMLElement>(null);
  // A file that is gone has nothing to read; its card names it only
  const readable = !change.gone;
  useEffect(() => {
    if (!readable) return;
    let live = true;
    const fetchFile = () => {
      void project.file(sessionId, change.path, 'session').then(r => { if (live) setRead(r); });
    };
    if (near.current) {
      fetchFile();
      return () => { live = false; };
    }
    const el = card.current;
    if (!el) return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(e => e.isIntersecting)) {
        near.current = true;
        observer.disconnect();
        fetchFile();
      }
    }, { root: scroller.current, rootMargin: REVIEW_AHEAD });
    observer.observe(el);
    return () => { live = false; observer.disconnect(); };
  }, [readable, sessionId, change.path, change.add, change.del, refresh]);
  const sliced = useMemo(() => (read?.text !== undefined && read.hunks.length ? reviewDiffSlice(read.text, read.hunks, REVIEW_LINES) : undefined), [read]);
  const lines = sliced?.lines;
  const rest = sliced?.rest ?? 0;
  // Until the diff is there: its changed lines plus a hunk header, capped like a shown diff (at least one line)
  const estimate = Math.min(change.add + change.del + 1, REVIEW_LINES);
  const expected = readable && !read ? estimate : 0;
  // review-card: out of sight the card skips layout and paint (panel.css), sized by the same estimate until it has been
  // laid out once; 40 laid-out cards made every scroll frame cost a quarter of a second in WebKit
  return (
    <section ref={card} className="review-card shrink-0 overflow-hidden rounded-lg border border-conversation-line"
      style={{ '--review-rows': readable ? estimate : 0 } as CSSProperties}>
      <button type="button" onClick={() => onOpen(change.path)} disabled={!readable}
        className="flex h-row w-full items-center gap-1.5 bg-bg-1 px-gap text-left transition-colors enabled:hover:bg-hover">
        <FileGlyph name={change.path} className="text-fg-3" />
        <span className={cn('shrink-0 text-2 text-fg-1', change.gone && 'line-through')}>{nameOf(change.path)}</span>
        <span className="min-w-0 flex-1 truncate text-3 text-fg-3">{dirOf(change.path)}</span>
        <span className="shrink-0 text-3"><DiffStat add={change.add} del={change.del} /></span>
      </button>
      {expected > 0 && <div aria-hidden className="review-pending" />}
      {lines && <DiffBlock lines={lines} path={change.path} whole />}
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
