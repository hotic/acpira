import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from 'react';
import {
  BookOpen, Brain, FilePen, FileText, Globe, History, MessageSquare, MessageSquareText, Pencil, Plug, Search, Tag, Terminal, Trash2, Wrench, type LucideIcon,
} from 'lucide-react';
import type { ContextNode, ContextReport } from '@acpira/shared/context';
import { useAppearance } from '../appearance';
import { t, useLocale } from '../i18n';
import { ChipTag } from '../ui/Button';
import { Row } from '../ui/Row';
import { cn } from '../ui/cn';
import { SHADES, shadeLabel, shadeOf } from './categories';
import { setReportLink } from './link';
import { Parts } from './Parts';
import { buildTree, leafCount, ROOT_ID, share, type TNode, type TreePaths } from './tree';
import { fitLabel, lay, MORE_SUFFIX, type Box, type Item, type Measure } from './treemap';

// The context report (the tool panel's context tab, ContextTab.tsx): the figure line and the map of the used tokens
// stay on top while the full tree scrolls under them. The tree drives the map's zoom, there is no breadcrumb: any number
// of rows stay open, and the map follows the row last picked (a leaf opens what was sent under its row); the picked row
// again or Esc folds it and goes up, the tree's title or the figure shows the whole map.
// Hover lights a node in both. Tones from tokens.css --context-*, the map and tree rules from contextReport.css

const ZOOM_MS = 340;
const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

// A leaf's icon by what it is; a call's by its ACP tool kind
const KIND_ICON: Partial<Record<ContextNode['kind'], LucideIcon>> = {
  base: FileText, environment: Terminal, prompt: FileText, doc: FileText, tool: Wrench, skill: BookOpen, skillTool: BookOpen,
  server: Plug, mcpTool: Wrench, summary: History, turn: MessageSquare, message: MessageSquare, note: Tag, thinking: Brain, reply: MessageSquareText,
};
const CALL_ICON: Record<string, LucideIcon> = {
  read: FileText, edit: Pencil, delete: Trash2, move: FilePen, search: Search, execute: Terminal, think: Brain, fetch: Globe,
};
const iconOf = (n: ContextNode): LucideIcon => (n.kind === 'call' ? CALL_ICON[n.toolKind ?? ''] ?? Wrench : KIND_ICON[n.kind] ?? Wrench);

// Text widths in the tiles' own fonts, read off hidden probes once the web fonts are in
function useMeasure(probe: RefObject<HTMLSpanElement | null>, small: RefObject<HTMLSpanElement | null>, look: string): Measure | undefined {
  const [m, setM] = useState<Measure>();
  useLayoutEffect(() => {
    if (!probe.current || !small.current) return;
    const p3 = probe.current, p4 = small.current;
    let live = true;
    void document.fonts.ready.then(() => {
      if (!live) return;
      const ctx = document.createElement('canvas').getContext('2d');
      if (!ctx) return;
      // Built by hand: the computed `font` shorthand is empty under tabular-nums
      const font = (el: Element) => { const c = getComputedStyle(el); return `${c.fontWeight} ${c.fontSize} ${c.fontFamily}`; };
      const f3 = font(p3), f4 = font(p4);
      const cache = new Map<string, number>();
      setM(() => (s: string, sm?: boolean) => {
        const key = `${sm ? 4 : 3}\u0000${s}`;
        let w = cache.get(key);
        if (w === undefined) { ctx.font = sm ? f4 : f3; w = ctx.measureText(s).width + 2; cache.set(key, w); }
        return w;
      });
    });
    return () => { live = false; };
  }, [look]);
  return m;
}

// A quiet segmented control (the tree's order)
function Segmented<T extends string>({ value, options, onChange, name }: { value: T; options: { id: T; label: string }[]; onChange: (v: T) => void; name: string }) {
  return (
    <span role="radiogroup" aria-label={name} className="flex items-center gap-px rounded-md bg-active/50 p-px">
      {options.map(o => (
        <button key={o.id} type="button" role="radio" aria-checked={value === o.id} onClick={() => onChange(o.id)}
          className={cn('h-ctl-sm cursor-pointer rounded-sm px-2 text-3 whitespace-nowrap transition-colors', value === o.id ? 'bg-bg-1 text-fg-1 shadow-xs' : 'text-fg-3 hover:text-fg-1')}>
          {o.label}
        </button>
      ))}
    </span>
  );
}

export interface ContextReportProps {
  report: ContextReport;
  sessionId: string;
  // Where auto-compaction starts (the settings' threshold), for the bar's tick
  compactAt?: number;
  paths?: TreePaths;
  // Opens a file the leaf names (absent where the session's files are not reachable)
  onOpenFile?: (path: string) => void;
  // At the end of the figure line: the tab's own controls
  actions?: ReactNode;
  // The tab is on screen: only then does a hover or pick light rows in the transcript
  shown?: boolean;
}

export function ContextReportView({ report: r, sessionId, compactAt, paths, onOpenFile, actions, shown = true }: ContextReportProps) {
  const locale = useLocale();
  const fmt = useMemo(() => new Intl.NumberFormat(locale).format, [locale]);
  const { root, index, pathOf } = useMemo(() => buildTree(r, paths), [r, paths, locale]);
  // The shades the conversation's tiles show, in the legend's fixed order; one shade alone needs no key
  const shades = useMemo(() => {
    const seen = new Set<string | undefined>();
    const walk = (n: TNode) => { if (n.children) n.children.forEach(walk); else seen.add(shadeOf(n.node)); };
    root.children!.filter(c => c.cat === 'chat').forEach(walk);
    const on = SHADES.filter(s => seen.has(s));
    return on.length > 1 ? on : [];
  }, [root]);
  const sticky = useRef<HTMLDivElement>(null);
  const treeBox = useRef<HTMLElement>(null);
  const mapBox = useRef<HTMLDivElement>(null);
  const p3 = useRef<HTMLSpanElement>(null), p4 = useRef<HTMLSpanElement>(null);
  const measure = useMeasure(p3, p4, locale);
  const [size, setSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    const el = mapBox.current;
    if (!el) return;
    const read = () => setSize({ w: el.clientWidth, h: el.clientHeight });
    const ro = new ResizeObserver(read);
    ro.observe(el);
    read();
    return () => ro.disconnect();
  }, []);

  // The map's zoom, the layer it leaves while a zoom runs, the tree's open rows (groups and leaves alike), the leaf the
  // map marks, the hovered node and the tree's order
  const [focus, setFocus] = useState(ROOT_ID);
  const [leaving, setLeaving] = useState<{ from: string; n: number }>();
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [leaf, setLeaf] = useState<string>();
  const [over, setOver] = useState<string>();
  const [order, setOrder] = useState<'order' | 'size'>('order');
  const seq = useRef(0);
  const scrollTo = useRef<string | undefined>(undefined);

  // A refresh can drop what was zoomed or open (a compaction folds the turns away): back to the top
  const focusNow = index.has(focus) ? focus : ROOT_ID;
  const leafNow = leaf && index.has(leaf) ? leaf : undefined;

  // The nearest group the map can zoom into
  const zoomable = (id: string) => [...pathOf(id)].reverse().find(x => index.get(x)?.children) ?? ROOT_ID;
  const parentOf = (id: string) => pathOf(id).at(-2) ?? ROOT_ID;
  const openAll = (ids: string[]) => setOpen(o => new Set([...o, ...ids]));
  // Zoom the map into a group: the tree opens down to it and marks it, whatever else is open stays open
  const zoom = (to: string) => {
    if (!index.get(to)?.children) return;
    setLeaf(undefined);
    openAll(pathOf(to).filter(x => x !== ROOT_ID));
    scrollTo.current = to;
    if (to === focusNow) return;
    setLeaving({ from: focusNow, n: ++seq.current });
    setFocus(to);
    // The tile under the pointer is replaced without a leave event
    setOver(undefined);
  };
  // A leaf zooms the map to the group holding it, marks its tile and opens its content
  const openLeaf = (id: string) => {
    zoom(zoomable(parentOf(id)));
    setLeaf(id);
    openAll([id]);
    scrollTo.current = id;
  };
  // Fold a row; when the map is on it (or inside it), the map goes up to the group holding it
  const fold = (id: string) => {
    setOpen(o => { const next = new Set(o); next.delete(id); return next; });
    if (pathOf(focusNow).includes(id)) { zoom(zoomable(parentOf(id))); return; }
    if (leafNow && pathOf(leafNow).includes(id)) setLeaf(undefined);
    scrollTo.current = focusNow;
  };
  // Esc goes up one level: the marked leaf first, then the zoomed group
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      if (leafNow) fold(leafNow);
      else if (focusNow !== ROOT_ID) fold(focusNow);
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });

  // The transcript lights the calls under the hovered node, else under the picked leaf (context/link.ts, chat/reportLink.ts).
  // Not for the whole report or a category: they hold most of the conversation, and lighting all of it says nothing
  const linked = useAppearance().reportLink !== 'off' && shown;
  const callsUnder = (id: string | undefined): string[] => {
    if (!id || pathOf(id).length < 3) return [];
    const out: string[] = [];
    const walk = (n: TNode) => { if (n.children) n.children.forEach(walk); else if (n.node?.call) out.push(n.node.call); };
    const n = index.get(id);
    if (n) walk(n);
    return out;
  };
  const hoverCalls = useMemo(() => callsUnder(over), [over, index]);
  const pickCalls = useMemo(() => callsUnder(leafNow), [leafNow, index]);
  // Each pick brings its row into view once, right away when the pointer is still on the picked row
  const picks = useRef(0);
  const pick = useMemo(() => ++picks.current, [leafNow]);
  useEffect(() => {
    const calls = hoverCalls.length ? hoverCalls : pickCalls;
    const reveal = hoverCalls.length && over !== leafNow ? undefined : pick;
    setReportLink(linked && calls.length ? { session: sessionId, calls, reveal } : null);
  }, [linked, sessionId, hoverCalls, pickCalls, pick, over, leafNow]);
  useEffect(() => () => setReportLink(null), []);

  // Bring the zoomed row (or an open leaf's row) into view under the sticky map
  useEffect(() => {
    const id = scrollTo.current;
    scrollTo.current = undefined;
    if (!id || id === ROOT_ID) return;
    const row = treeBox.current?.querySelector<HTMLElement>(`[data-row="${CSS.escape(id)}"]`);
    const scroller = row?.closest<HTMLElement>('[data-report-scroll]');
    if (!row || !scroller) return;
    const top = (sticky.current?.getBoundingClientRect().bottom ?? scroller.getBoundingClientRect().top) + 8;
    const bottom = scroller.getBoundingClientRect().bottom - 8;
    const rr = row.getBoundingClientRect();
    // Room for a few of a group's rows, or a leaf's content
    const need = index.get(id)?.children ? 160 : 360;
    if (rr.top < top || rr.top + need > bottom) scroller.scrollBy({ top: rr.top - top - 4, behavior: reduced() ? 'auto' : 'smooth' });
  }, [focusNow, leafNow, open]);

  const layoutOf = (id: string) => (size.w > 0 && size.h > 0 ? lay(index.get(id) ?? root, { x: 0, y: 0, w: size.w, h: size.h }) : []);
  const cur = useMemo(() => layoutOf(focusNow), [focusNow, size, root, index]);
  const curLayer = useRef<HTMLDivElement>(null), oldLayer = useRef<HTMLDivElement>(null);

  // Between the two layers with transforms only: the zoomed node's box grows to the whole map, or shrinks back into it
  useLayoutEffect(() => {
    if (!leaving || !curLayer.current) return;
    const done = () => setLeaving(l => (l?.n === leaving.n ? undefined : l));
    if (reduced() || !oldLayer.current) { done(); return; }
    const inward = pathOf(focusNow).includes(leaving.from);
    const outer = inward ? leaving.from : focusNow, inner = inward ? focusNow : leaving.from;
    const path = pathOf(inner);
    const hit = layoutOf(outer).filter(i => path.includes(i.n.id)).sort((a, b) => path.indexOf(b.n.id) - path.indexOf(a.n.id))[0];
    if (!hit) { done(); return; }
    const R: Box = hit.b, W = size.w, H = size.h;
    const into = `translate(${R.x}px, ${R.y}px) scale(${R.w / W}, ${R.h / H})`;
    const sx = W / R.w, sy = H / R.h;
    const outOf = `translate(${-R.x * sx}px, ${-R.y * sy}px) scale(${sx}, ${sy})`;
    const opt: KeyframeAnimationOptions = { duration: ZOOM_MS, easing: 'cubic-bezier(0.2, 0.7, 0.2, 1)', fill: 'both' };
    const a = curLayer.current.animate([{ transform: inward ? into : outOf, opacity: 0 }, { transform: 'none', opacity: 1 }], opt);
    const b = oldLayer.current.animate([{ transform: 'none', opacity: 1 }, { transform: inward ? outOf : into, opacity: 0 }], { ...opt, duration: ZOOM_MS - 60 });
    a.onfinish = () => { a.cancel(); done(); };
    return () => { a.cancel(); b.cancel(); };
  }, [leaving?.n]);

  // A stub of thin tiles stands for its group
  const ownerOf = (n: TNode) => (n.more ? n.id.slice(0, -MORE_SUFFIX.length) : n.id);
  // A tile click: a leaf opens (the map zooms to its group), anything else zooms into the focus's child holding it
  const clickTile = (id: string) => {
    if (index.get(id) && !index.get(id)!.children) { openLeaf(id); return; }
    const p = pathOf(id), i = p.indexOf(focusNow);
    const next = i >= 0 ? p.slice(i + 1).find(x => index.get(x)?.children) : undefined;
    if (next) zoom(next);
  };
  const isLit = (it: Item) => {
    if (!over) return false;
    if (it.n.more) return over === ownerOf(it.n) || it.n.more.includes(over);
    if (pathOf(it.n.id).includes(over)) return it.k !== 'group';
    return it.k === 'block' && pathOf(over).includes(it.n.id);
  };
  // The group a hover belongs to gets a soft outline
  const outlined = (items: Item[]) => {
    if (!over) return undefined;
    const p = pathOf(over);
    return items.filter(i => i.k === 'group' && p.includes(i.n.id) && (i.n.id !== over || !!index.get(over)?.children))
      .sort((a, b) => p.indexOf(b.n.id) - p.indexOf(a.n.id))[0]?.n.id;
  };
  // An open leaf hidden in a block or a stub marks that block or stub instead
  const marked = (it: Item) => !!leafNow && (it.n.id === leafNow || (it.k === 'block' && pathOf(leafNow).includes(it.n.id)) || !!it.n.more?.includes(leafNow));
  const nameOf = (n: TNode) => (n.target && !n.children ? `${n.label} ${n.target}` : n.label);

  const layer = (id: string, ref: RefObject<HTMLDivElement | null>, ghost?: boolean) => {
    const items = id === focusNow ? cur : layoutOf(id);
    const ring = ghost ? undefined : outlined(items);
    return (
      <div ref={ref} key={ghost ? `old-${leaving?.n}` : 'cur'} className={cn('absolute inset-0 origin-top-left', ghost && 'pointer-events-none')} aria-hidden={ghost}>
        {items.map(it => {
          if (it.k === 'group') {
            return <div key={`${it.n.id}-g`} aria-hidden className="ctx-plate" data-ring={ring === it.n.id || undefined} onClick={ghost ? undefined : () => zoom(it.n.id)}
              style={{ left: it.b.x, top: it.b.y, width: it.b.w, height: it.b.h }} />;
          }
          const owner = ownerOf(it.n);
          return <Tile key={it.n.id + it.k} it={it} title={`${it.n.more ? it.n.label : nameOf(it.n)} ${fmt(it.n.tokens)}`} figure={fmt(it.n.tokens)} measure={measure}
            lit={!ghost && isLit(it)} marked={marked(it)} onEnter={() => setOver(owner)} onLeave={() => setOver(undefined)} onClick={() => clickTile(owner)} />;
        })}
      </div>
    );
  };

  // One dense row of the tree: a category's tint or the node's icon, the label and target, the figures in fixed columns
  const litUnder = (id: string) => !!over && pathOf(over).includes(id);
  const branch = (n: TNode): ReactNode => {
    const isOpen = open.has(n.id);
    // A sum the agent did not split has nothing to show under its row; a click only marks it on the map
    const opens = !!n.children || n.node?.kind !== 'rest';
    const Icon = n.node && n.node.kind !== 'rest' && iconOf(n.node);
    // A row opens and the map moves to it; the row the map is on folds again
    const toggle = () => {
      if (!n.children) { if (leafNow === n.id) fold(n.id); else openLeaf(n.id); return; }
      if (n.id === focusNow && !leafNow) { fold(n.id); return; }
      zoom(n.id);
    };
    const kids = n.children ? [...n.children] : [];
    if (order === 'size') kids.sort((a, b) => b.tokens - a.tokens);
    return (
      <li key={n.id}>
        <Row as="button" dense data-row={n.id} onClick={toggle} aria-expanded={opens ? isOpen : undefined}
          className="ctx-band w-full cursor-pointer"
          data-current={n.id === focusNow || n.id === leafNow || undefined} data-lit={over === n.id || (!isOpen && litUnder(n.id)) || undefined}
          onMouseEnter={() => setOver(n.id)} onMouseLeave={() => setOver(undefined)}
          lead={Icon ? <Icon strokeWidth={1.5} className="size-3.5 text-fg-3" /> : <span aria-hidden className="ctx-mark" data-cat={n.cat} />}
          trailing={<>
            <span className="w-(--report-figure-w) text-right text-fg-2">{fmt(n.tokens)}</span>
            <span className="w-(--report-share-w) text-right">{share(n.tokens, r.used)}</span>
          </>}>
          <span className={cn('shrink-0 text-fg-1', n.mono && 'font-mono text-mono')}>{n.label}</span>
          {n.target && <span className={cn('min-w-0 truncate text-fg-3', n.monoTarget && 'font-mono text-mono')}>{n.target}</span>}
          {n.children && <span className="shrink-0 text-3 text-fg-3">{leafCount(n)}</span>}
        </Row>
        {isOpen && opens && (
          <ul data-focus={n.id === focusNow || undefined}>
            {n.children
              ? kids.map(branch)
              : <Parts node={n} sessionId={sessionId} rev={r.rev} file={n.file} onOpenFile={onOpenFile} />}
          </ul>
        )}
      </li>
    );
  };

  const pct = Math.round((r.used / r.size) * 100);
  const showAll = t('context.showAll');
  return (
    <div className="flex flex-col gap-(--report-gap)">
      {/* The figure and the map stay in view while the tree scrolls under them */}
      <div ref={sticky} data-report-sticky className="sticky top-0 z-10 -mx-2 flex flex-col gap-2 bg-(--report-surface) px-2 pt-gap pb-1">
        <span ref={p3} aria-hidden className="pointer-events-none invisible absolute text-3">x</span>
        <span ref={p4} aria-hidden className="pointer-events-none invisible absolute text-4">x</span>
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-2">
            <button type="button" data-all-figure onClick={() => zoom(ROOT_ID)} title={showAll}
              className="-mx-1 flex cursor-pointer items-baseline gap-2 rounded-sm px-1 transition-colors hover:bg-hover">
              <span className="text-2 font-medium text-fg-1">{r.estimated ? t('usage.about', { n: fmt(r.used) }) : fmt(r.used)}</span>
              <span className="text-3 text-fg-3">/ {fmt(r.size)}</span>
            </button>
            <ChipTag>{t('usage.usedPctShort', { pct })}</ChipTag>
            {r.estimated && <span title={t('context.estimatedHint')}><ChipTag>{t('context.estimated')}</ChipTag></span>}
            {actions && <span className="ml-auto flex shrink-0 items-center gap-gap">{actions}</span>}
          </div>
          {/* The tick marks where compaction starts; the title names it */}
          <div className="relative h-1 overflow-hidden rounded-full bg-active" title={compactAt ? t('usage.budget', { n: fmt(compactAt) }) : undefined}>
            <span className="absolute inset-y-0 left-0 bg-fg-2" style={{ width: `${Math.min(1, r.used / r.size) * 100}%` }} />
            {compactAt !== undefined && compactAt < r.size && <span className="absolute inset-y-0 w-0.5 bg-fg-1" style={{ left: `${(compactAt / r.size) * 100}%` }} />}
          </div>
          {shades.length > 0 && (
            <div data-shade-key className="flex flex-wrap gap-x-3 gap-y-1 text-3 text-fg-3">
              {shades.map(s => (
                <span key={s} className="flex items-center gap-1.5"><span aria-hidden className="ctx-mark" data-cat="chat" data-shade={s} />{shadeLabel(s)}</span>
              ))}
            </div>
          )}
        </div>
        <div ref={mapBox} data-map className="ctx-map relative h-(--report-map-h) overflow-hidden" data-zoom={leaving ? '' : undefined} onMouseLeave={() => setOver(undefined)}>
          {leaving && layer(leaving.from, oldLayer, true)}
          {layer(focusNow, curLayer)}
        </div>
      </div>
      <section ref={treeBox} data-tree className="flex flex-col gap-1">
        <div className="flex h-ctl shrink-0 items-center gap-2 text-3">
          {/* The tree's title is its root: a click shows everything again */}
          <button type="button" data-all onClick={() => zoom(ROOT_ID)} title={showAll}
            className={cn('-mx-1 cursor-pointer rounded-sm px-1 text-2 font-medium transition-colors hover:bg-hover', focusNow === ROOT_ID ? 'text-fg-1' : 'text-fg-2 hover:text-fg-1')}>
            {t('context.breakdown')}
          </button>
          <span className="text-fg-3">{t('context.tokens', { n: fmt(r.used) })}</span>
          <span className="flex-1" />
          <Segmented name={t('context.order')} value={order} onChange={setOrder}
            options={[{ id: 'order', label: t('context.byOrder') }, { id: 'size', label: t('context.bySize') }]} />
        </div>
        <ul className="branch-tree ctx-tree">{root.children!.map(branch)}</ul>
      </section>
    </div>
  );
}

// A label shows only when it fits whole: flat with room for its line, else upright in a tall narrow tile, else none
const LINE = 24;
const LINE_NUM = 40;
const UPRIGHT_W = 20;
// Label padding inside a tile (contextReport.css .ctx-tile > [data-label])
const ROOM = 12;

function Tile({ it, title, figure, measure, lit, marked, onEnter, onLeave, onClick }: {
  it: Item; title: string; figure: string; measure?: Measure; lit: boolean; marked: boolean;
  onEnter: () => void; onLeave: () => void; onClick: () => void;
}) {
  const { n, b, k } = it;
  const { w, h } = b;
  // Anything thinner than a few pixels is left as gap; the tree still names it
  if (Math.min(w, h) < 6) return null;
  const attrs = {
    onMouseEnter: onEnter, onMouseLeave: onLeave, onClick, title, 'data-node': n.id, 'data-cat': n.cat, 'data-shade': n.cat === 'chat' ? shadeOf(n.node) : undefined,
    'data-more': n.more ? '' : undefined, 'data-lit': lit || undefined, 'data-marked': marked || undefined,
    className: 'ctx-tile', style: { left: b.x, top: b.y, width: w, height: h } as CSSProperties,
  };
  // Upright: a tall narrow tile (or block) whose name will not lie flat
  const upright = (label: string) => {
    if (!measure || w < UPRIGHT_W) return null;
    const lw = measure(label);
    if (lw + ROOM > h) return null;
    return (
      <div {...attrs} data-upright className="ctx-tile text-3">
        <span className="ctx-text text-fg-1">{label}</span>
        {lw + measure(figure) + 16 <= h && <span className="ctx-text text-fg-3">{figure}</span>}
      </div>
    );
  };
  if (k === 'block' || n.more) {
    const lw = measure?.(n.label) ?? Infinity;
    const flat = !!measure && lw <= w - ROOM && h >= LINE;
    if (!flat && (w < 48 || lw > w - ROOM)) { const u = upright(n.label); if (u) return u; }
    return (
      <div {...attrs}>
        {flat && <div data-label className="ctx-text gap-1 text-3">
          <span className={n.more ? 'text-fg-3' : 'text-fg-1'}>{n.label}</span>
          {h >= LINE_NUM && measure!(figure, true) <= w - ROOM && <span className="text-fg-3">{figure}</span>}
        </div>}
      </div>
    );
  }
  // A leaf: the most telling label that fits whole, the figure under it when the tile is tall enough
  const room = w - ROOM;
  const fit = measure && h >= LINE ? fitLabel(n, room, measure) : undefined;
  if (!fit) { const u = upright(n.label); if (u) return u; }
  return (
    <div {...attrs}>
      {/* Padding lives on the label, so a bare tile keeps the exact width it was given */}
      {fit && <div data-label className="ctx-text">
        <span className="text-3 text-fg-1">{fit.a}{fit.b && <span className="text-fg-2"> {fit.b}</span>}</span>
        {h >= LINE_NUM && measure!(figure, true) <= room && <span className="text-4 text-fg-3">{figure}</span>}
      </div>}
    </div>
  );
}
