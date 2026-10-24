import { Fragment, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import type { FileHit, ProjectChange } from '@acpira/shared/protocol';
import { AtSign, ChevronRight, ChevronsDownUp, Crosshair, Folder, FolderOpen, Layers, Search, X } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n';
import { t } from '../i18n';
import { IconButton } from '../ui/Button';
import { Row } from '../ui/Row';
import { cn } from '../ui/cn';
import { DiffStat } from '../chat/ToolCall';
import { nameOf as uriName } from '../chat/drafts';
import { lastRead, project, sameRead, type DirListing } from './api';
import { FileGlyph, MARK_DOT, MARK_TEXT, buildTree, dirOf, nameOf, type MarkLookup, type TreeNode } from './files';

// The Files tab's tree, Codex-like (picked in the LAB, 2026-10-23): a filter, the session's changed files pinned over the
// project's folders, open folders sticking under their ancestors, git marks at rest and an @ on hover. Folders are listed
// one at a time as they open (projectDir); the filter searches the whole project with the composer's @ index

// The fill is one tint over a base (panel.css .tree-row): hover and selection only change the tint. The lead glyph sits a
// pixel low: centred on the line box it rode high beside the names' lowercase letters (as the session list's project folder)
const TREE_ROW = 'tree-row group/row relative cursor-pointer gap-1 rounded-md pr-2 [&>.row-lead]:translate-y-px';
// The leading space for a depth: the chevron's slot, then one step per level
const indent = (depth: number): CSSProperties => ({ paddingLeft: `calc(var(--gap) / 2 + var(--tree-chevron) + ${depth} * var(--tree-indent))` });
const chevronAt = (depth: number): CSSProperties => ({ left: `calc(var(--gap) / 2 + ${depth} * var(--tree-indent))` });

function Highlighted({ text, query }: { text: string; query: string }) {
  const at = query ? text.toLowerCase().indexOf(query) : -1;
  if (at < 0) return <>{text}</>;
  return <>{text.slice(0, at)}<span className="rounded-xs bg-focus/25 text-fg-strong">{text.slice(at, at + query.length)}</span>{text.slice(at + query.length)}</>;
}

// One column that may shrink (a track sized to its content let long names push past the pane), clipped rather than hidden
// (a hidden box would become the scroll container of the sticky rows inside)
function Fold({ open, children }: { open: boolean; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] transition-[grid-template-rows,opacity] duration-(--dur-open) ease-out" style={{ gridTemplateRows: open ? '1fr' : '0fr', opacity: open ? 1 : 0 }}>
      <div className="min-h-0 overflow-clip">{children}</div>
    </div>
  );
}

interface BranchCtx {
  dirs: Record<string, DirListing | undefined>;
  expanded: Set<string>;
  // Folders opened at least once keep their rows mounted, so closing them can animate
  seen: Set<string>;
  mark: MarkLookup;
  active?: string;
  query: string;
  toggle: (path: string) => void;
  open: (path: string) => void;
  mention: (path: string) => void;
}

// Stuck folders stack with the outer ones on top: a deeper folder whose rows have scrolled by is pushed up out of its
// slot and has to slide under its ancestors (an equal z-index painted it over them, later in the DOM, in reverse order)
const STUCK_Z = 50;

function TreeRow({ node, depth, open, ctx }: { node: { name: string; path: string; dir: boolean }; depth: number; open: boolean; ctx: BranchCtx }) {
  const mark = ctx.mark(node.path, node.dir);
  const ignored = mark === 'I';
  const selected = !node.dir && ctx.active === node.path;
  return (
    <Row role="treeitem" aria-expanded={node.dir ? open : undefined} aria-selected={selected} data-path={node.path} data-sticky={(node.dir && open) || undefined}
      style={{ ...indent(depth), ...(node.dir && open ? { top: `calc(${depth} * var(--row))`, zIndex: STUCK_Z - depth } : {}) }}
      onClick={() => (node.dir ? ctx.toggle(node.path) : ctx.open(node.path))}
      // `sticky` as a class: the rows' `relative` utility outranks a position set in panel.css's components layer, which
      // left open folders relative and pushed each one down by its own sticky offset over its first child. An ignored row
      // dims its content, not itself: a stuck one has to stay opaque over the rows scrolling under it
      className={cn(TREE_ROW, node.dir && open && 'sticky', selected ? 'text-fg-1' : 'hover:text-fg-1', ignored && '*:opacity-50')}
      lead={node.dir
        ? <span className="relative flex size-icon items-center justify-center *:transition-opacity *:duration-(--dur-open)">
            <Folder strokeWidth={1.5} className={cn('absolute size-icon', open ? 'opacity-0' : 'opacity-100')} />
            <FolderOpen strokeWidth={1.5} className={cn('absolute size-icon', open ? 'opacity-100' : 'opacity-0')} />
          </span>
        : <FileGlyph name={node.name} />}
      trailing={
        // At rest the git mark; on hover the row's one action, @ into the composer
        <span className="relative flex w-ctl-sm items-center justify-center">
          {mark && !ignored && (node.dir
            ? <span title={t(`panel.mark.${mark}` as MsgKey)} className={cn('size-1.5 rounded-full opacity-80 transition-opacity group-hover/row:opacity-0', MARK_DOT[mark])} />
            : <span title={t(`panel.mark.${mark}` as MsgKey)} className={cn('transition-opacity group-hover/row:opacity-0', MARK_TEXT[mark])}>{mark}</span>)}
          {!node.dir && (
            <span role="button" tabIndex={-1} title={t('panel.mention')} aria-label={t('panel.mention')}
              onClick={e => { e.stopPropagation(); ctx.mention(node.path); }}
              className="absolute inset-0 flex items-center justify-center rounded-sm opacity-0 transition-opacity group-hover/row:opacity-100 hover:bg-active hover:text-fg-1">
              <AtSign strokeWidth={1.5} className="size-3.5" />
            </span>
          )}
        </span>
      }>
      {node.dir && (
        <span aria-hidden className="absolute inset-y-0 flex w-tree-chevron items-center justify-center text-fg-3" style={chevronAt(depth)}>
          <ChevronRight strokeWidth={1.5} className={cn('size-3 transition-transform duration-(--dur-open) ease-out', open && 'rotate-90')} />
        </span>
      )}
      {node.dir && open && <span aria-hidden data-stuck-edge className="pointer-events-none absolute inset-x-0 -bottom-px h-px bg-line opacity-0" />}
      <span className={cn('min-w-0 truncate', mark && !node.dir && !ignored && MARK_TEXT[mark], mark === 'D' && 'line-through')}><Highlighted text={node.name} query={ctx.query} /></span>
    </Row>
  );
}

// The project as listed so far: a folder's rows once it was opened, loaded on demand
function Branch({ dir, depth, ctx }: { dir: string; depth: number; ctx: BranchCtx }) {
  const listing = ctx.dirs[dir];
  if (!listing) return null;
  if (listing.error || !listing.entries.length) {
    return <div className="flex min-h-row items-center text-3 text-fg-3" style={indent(depth)}>{listing.error ? t('panel.error.other', { error: listing.error }) : t('panel.emptyFolder')}</div>;
  }
  return <>{listing.entries.map(e => {
    const path = dir ? `${dir}/${e.name}` : e.name;
    const open = !!e.dir && ctx.expanded.has(path);
    const listed = !!e.dir && !!ctx.dirs[path];
    const Group = e.dir ? 'div' : Fragment;
    return (
      <Group key={path}>
        <TreeRow node={{ name: e.name, path, dir: !!e.dir }} depth={depth} open={open} ctx={ctx} />
        {e.dir && <Fold open={open && listed}>{(open || ctx.seen.has(path)) && <Branch dir={path} depth={depth + 1} ctx={ctx} />}</Fold>}
      </Group>
    );
  })}</>;
}

// The filter's matches with the folders above them, all open
function Matches({ nodes, depth, ctx }: { nodes: TreeNode[]; depth: number; ctx: BranchCtx }) {
  return <>{nodes.map(n => (
    <Fragment key={n.path}>
      <TreeRow node={n} depth={depth} open={n.dir} ctx={{ ...ctx, toggle: () => {} }} />
      {n.dir && <Matches nodes={n.children} depth={depth + 1} ctx={ctx} />}
    </Fragment>
  ))}</>;
}

// Revealing a row follows the folds above it until the list's height holds for this many frames, for at most this long
const REVEAL_CALM_FRAMES = 3;
const REVEAL_MAX_MS = 1000;

const SECTION = 'group/section flex min-h-row shrink-0 items-center gap-gap px-2 text-3 text-fg-3';

export interface FileTreeProps {
  sessionId: string;
  root: string;
  // Bumped when the project may have changed on disk (a turn ended): listed folders are read again
  refresh: number;
  mark: MarkLookup;
  changes: ProjectChange[];
  active?: string;
  review: boolean;
  expanded: string[];
  setExpanded: (fn: (open: string[]) => string[]) => void;
  onOpen: (path: string) => void;
  onReview: () => void;
  onMention: (path: string) => void;
  search?: (query: string) => Promise<FileHit[]>;
}

export function FileTree(p: FileTreeProps) {
  // Seeded with what this project's tree showed last; every folder is still read again below
  const [dirs, setDirs] = useState<Record<string, DirListing | undefined>>(() => lastRead.dirs.get(p.root) ?? {});
  const expanded = useMemo(() => new Set(p.expanded), [p.expanded]);
  const [seen, setSeen] = useState(() => new Set(p.expanded));
  const [changesOpen, setChangesOpen] = useState(true);
  const [query, setQuery] = useState('');
  const q = query.trim().toLowerCase();
  const [hits, setHits] = useState<string[]>();
  const list = useRef<HTMLDivElement>(null);
  const asked = useRef(new Set<string>());

  // The project folder itself, and every open folder not read yet in this mount. A listing equal to the shown one (the
  // cached one after a session switch, or an unchanged folder after a turn) leaves the tree alone
  const load = (dir: string) => {
    asked.current.add(dir);
    const root = p.root;
    void project.dir(p.sessionId, dir).then(l => {
      // A read nobody answered is shown but not kept: the next mount shows the last real listing
      if (l.error !== 'timeout') lastRead.dirs.set(root, { ...lastRead.dirs.get(root), [dir]: l });
      setDirs(d => (sameRead(d[dir], l) ? d : { ...d, [dir]: l }));
    });
  };
  useEffect(() => {
    for (const dir of ['', ...p.expanded]) if (!asked.current.has(dir)) load(dir);
  }, [p.expanded]);
  // A turn ended: what is listed is read again (a listing stays shown until its fresh one arrives)
  useEffect(() => {
    if (!p.refresh) return;
    for (const dir of asked.current) load(dir);
  }, [p.refresh]);
  useEffect(() => { setSeen(s => (p.expanded.some(x => !s.has(x)) ? new Set([...s, ...p.expanded]) : s)); }, [p.expanded]);

  // The filter asks the project's file index (the composer's @ list), a beat after the last key
  useEffect(() => {
    if (!q || !p.search) { setHits(undefined); return; }
    let live = true;
    const timer = setTimeout(() => {
      void p.search!(query.trim()).then(found => { if (live) setHits(found.map(h => uriName(h.uri, p.root))); });
    }, 120);
    return () => { live = false; clearTimeout(timer); };
  }, [q]);

  // The row to bring to the middle of the list: the open file when it changes, or again on the locate button (`n` asks
  // anew for the same file). Cleared once it got there, so a later relisting (a turn ended) leaves the scroll alone
  const [target, setTarget] = useState<{ path: string; n: number }>();
  useEffect(() => { if (p.active) setTarget(x => ({ path: p.active!, n: (x?.n ?? 0) + 1 })); }, [p.active]);
  useEffect(() => {
    const el = list.current;
    if (!target || !el) return;
    // The folders above it are still unfolding (Fold eases their height), so at first the list is too short to scroll the
    // row to the middle and the scroll is cut short: it is put there frame after frame until the height holds still
    let frame = 0, calm = 0, lastH = -1;
    const end = performance.now() + REVEAL_MAX_MS;
    const step = () => {
      const row = el.querySelector<HTMLElement>(`[data-path="${CSS.escape(target.path)}"]`);
      // Not listed yet: the folders above it are still being read, `dirs` runs this again as they arrive
      if (!row) return;
      el.scrollTop = row.offsetTop - el.clientHeight / 2 + row.offsetHeight / 2;
      calm = el.scrollHeight === lastH ? calm + 1 : 0;
      lastH = el.scrollHeight;
      if (calm < REVEAL_CALM_FRAMES && performance.now() < end) frame = requestAnimationFrame(step);
      else setTarget(x => (x === target ? undefined : x));
    };
    step();
    return () => cancelAnimationFrame(frame);
  }, [target, dirs]);
  // VS Code's "reveal active file": the folders above the open file open, the filter clears, its row comes to the middle
  const locate = () => {
    const at = p.active;
    if (!at) return;
    setQuery('');
    p.setExpanded(open => [...new Set([...open, ...ancestors(at)])]);
    setTarget(x => ({ path: at, n: (x?.n ?? 0) + 1 }));
  };

  const toggle = (path: string) => {
    if (!asked.current.has(path)) load(path);
    setSeen(s => new Set(s).add(path));
    p.setExpanded(open => (open.includes(path) ? open.filter(x => x !== path) : [...open, path]));
  };
  const ctx: BranchCtx = { dirs, expanded, seen, mark: p.mark, active: p.active, query: q, toggle, open: p.onOpen, mention: p.onMention };
  const add = p.changes.reduce((n, c) => n + c.add, 0), del = p.changes.reduce((n, c) => n + c.del, 0);
  const shown = p.changes.filter(c => !q || c.path.toLowerCase().includes(q));
  const matches = useMemo(() => (hits ? buildTree(hits) : undefined), [hits]);

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-tree>
      {/* The trailing inset is the tool panel's when the tree sits under its pinned toggle (panel.css --tree-filter-end) */}
      <div className="flex min-h-row shrink-0 items-center border-b border-line py-gap-half pl-gap pr-[var(--tree-filter-end,var(--spacing-gap))]">
        <label className="flex h-ctl-sm min-w-0 flex-1 items-center gap-1.5 rounded-md border border-line bg-chip px-2 text-3 focus-within:border-line-strong">
          <Search strokeWidth={1.5} className="size-3 shrink-0 text-fg-3" />
          <input value={query} onChange={e => setQuery(e.target.value)} placeholder={t('panel.filter')} spellCheck={false}
            onKeyDown={e => { if (e.key === 'Escape' && query) { e.preventDefault(); e.stopPropagation(); setQuery(''); } }}
            className="min-w-0 flex-1 border-0 bg-transparent text-3 text-fg-1 outline-none placeholder:text-fg-3" />
          {query && <button type="button" onClick={() => setQuery('')} aria-label={t('panel.clear')} title={t('panel.clear')} className="text-fg-3 hover:text-fg-1"><X strokeWidth={1.5} className="size-3" /></button>}
        </label>
      </div>
      {p.changes.length > 0 && (
        <div className="shrink-0 px-gap-half pt-gap-half pb-gap" data-changes>
          <div className={SECTION}>
            <button type="button" onClick={() => setChangesOpen(o => !o)} aria-expanded={changesOpen} className="flex items-center gap-1 hover:text-fg-1">
              {t('panel.changes')}<ChevronRight strokeWidth={1.5} className={cn('size-3 opacity-0 transition-[rotate,opacity] duration-(--dur-open) group-hover/section:opacity-100', changesOpen && 'rotate-90')} />
            </button>
            <span className="flex-1" />
            <DiffStat add={add} del={del} />
          </div>
          <Fold open={changesOpen}>
            <div role="tree" aria-label={t('panel.changes')}>
              <Row role="treeitem" aria-selected={p.review} onClick={p.onReview} className={cn(TREE_ROW, 'px-2', p.review ? 'text-fg-1' : 'hover:text-fg-1')}
                lead={<Layers strokeWidth={1.5} className="size-icon" />} trailing={t('panel.fileCount', { n: p.changes.length })}>
                <span className="truncate">{t('panel.allChanges')}</span>
              </Row>
              {shown.map(c => {
                const on = p.active === c.path && !p.review;
                return (
                  <Row key={c.path} role="treeitem" aria-selected={on} onClick={() => p.onOpen(c.path)} title={c.path}
                    className={cn(TREE_ROW, 'px-2', on ? 'text-fg-1' : 'hover:text-fg-1', c.gone && 'line-through decoration-fg-3')}
                    lead={<FileGlyph name={c.path} />} trailing={c.gone && !c.del ? t('panel.mark.D') : <DiffStat add={c.add} del={c.del} />}>
                    {/* The folder gives way first (no basis), then the name truncates instead of running under the counts */}
                    <span className="min-w-0 truncate"><Highlighted text={nameOf(c.path)} query={q} /></span>
                    <span className="min-w-0 flex-1 truncate text-3 text-fg-3" dir="rtl"><bdi>{dirOf(c.path)}</bdi></span>
                  </Row>
                );
              })}
            </div>
          </Fold>
        </div>
      )}
      <div className={cn(SECTION, 'mx-gap-half')}>
        <span className="truncate">{nameOf(p.root.replace(/\\/g, '/').replace(/\/+$/, ''))}</span>
        <span className="flex-1" />
        <span className="flex items-center gap-0.5 opacity-0 transition-opacity group-hover/section:opacity-100 focus-within:opacity-100">
          {p.active && <IconButton size="sm" title={t('panel.locate')} aria-label={t('panel.locate')}
            onClick={locate}><Crosshair strokeWidth={1.5} /></IconButton>}
          <IconButton size="sm" title={t('panel.collapseAll')} aria-label={t('panel.collapseAll')} onClick={() => p.setExpanded(() => [])}><ChevronsDownUp strokeWidth={1.5} /></IconButton>
        </span>
      </div>
      <div ref={list} role="tree" aria-label={t('panel.tree')} className="scroll-thin relative min-h-0 flex-1 overflow-auto px-gap-half pb-gap">
        {matches
          ? matches.length ? <Matches nodes={matches} depth={0} ctx={ctx} /> : <div className="flex min-h-row items-center px-2 text-3 text-fg-3">{t('panel.noMatch', { query: query.trim() })}</div>
          : <Branch dir="" depth={0} ctx={ctx} />}
      </div>
    </div>
  );
}

// The folders above a relative path, outermost first
export function ancestors(path: string): string[] {
  const parts = path.split('/');
  return parts.slice(0, -1).map((_, i) => parts.slice(0, i + 1).join('/'));
}
