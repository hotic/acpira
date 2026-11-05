import { t } from '../i18n';
import { leafCount, type TNode } from './tree';

// The report's map (ContextReport.tsx): the used tokens as nested tiles, each as large as what it holds. Pure layout,
// rendered and animated by the page

export interface Box { x: number; y: number; w: number; h: number }

// Squarified treemap: rows of nodes along the shorter side, each row closed when the next node would worsen its aspect
export function squarify<T>(items: { value: number; data: T }[], rect: Box): (Box & { data: T })[] {
  const sum = items.reduce((s, i) => s + Math.max(0, i.value), 0);
  if (sum <= 0 || rect.w <= 0 || rect.h <= 0) return [];
  const scale = (rect.w * rect.h) / sum;
  let nodes = items.filter(i => i.value > 0).sort((a, b) => b.value - a.value).map(i => ({ ...i, area: i.value * scale }));
  const out: (Box & { data: T })[] = [];
  let r = { ...rect };
  const worst = (row: typeof nodes, side: number) => {
    const s = row.reduce((acc, n) => acc + n.area, 0);
    const max = Math.max(...row.map(n => n.area)), min = Math.min(...row.map(n => n.area));
    return Math.max((side * side * max) / (s * s), (s * s) / (side * side * min));
  };
  while (nodes.length) {
    const side = Math.min(r.w, r.h);
    let n = 1;
    while (n < nodes.length && worst(nodes.slice(0, n + 1), side) <= worst(nodes.slice(0, n), side)) n++;
    const row = nodes.slice(0, n);
    const s = row.reduce((acc, x) => acc + x.area, 0);
    if (r.w >= r.h) {
      const w = s / r.h;
      let y = r.y;
      for (const x of row) { const h = x.area / w; out.push({ x: r.x, y, w, h, data: x.data }); y += h; }
      r = { x: r.x + w, y: r.y, w: r.w - w, h: r.h };
    } else {
      const h = s / r.w;
      let x0 = r.x;
      for (const x of row) { const w = x.area / h; out.push({ x: x0, y: r.y, w, h, data: x.data }); x0 += w; }
      r = { x: r.x, y: r.y + h, w: r.w, h: r.h - h };
    }
    nodes = nodes.slice(n);
  }
  return out;
}

// tile: a leaf (or a stub of thin leaves); group: a plate its children lie on; block: a group too small for its
// children, drawn as one named tile
export interface Item { k: 'tile' | 'group' | 'block'; n: TNode; b: Box }

// Gutters around groups (between categories, then inside them), the plate's padding, and the gap every tile keeps
const GUTTER = [3, 2];
const PAD = [3, 2];
const TILE_GAP = 1;
// A leaf box thinner than this would read as a strip; two or more of them fold into one quiet stub
const MIN_SIDE = 10;
// A group gets a plate with its tiles only with this much room, else it is one block
const MIN_GROUP_W = 40;
const MIN_GROUP_H = 28;
const MIN_LEAF_AREA = 700;

export const MORE_SUFFIX = '~more';

// A group laid out in its box with no captions: its tiles on a faint plate when they have room, else one named block
export function lay(n: TNode, b: Box, out: Item[] = [], depth = 0): Item[] {
  if (!n.children) { out.push({ k: 'tile', n, b }); return out; }
  let inner = b;
  if (depth > 0) {
    if (b.w < MIN_GROUP_W || b.h < MIN_GROUP_H || (b.w * b.h) / leafCount(n) < MIN_LEAF_AREA) { out.push({ k: 'block', n, b }); return out; }
    out.push({ k: 'group', n, b });
    const p = PAD[Math.min(depth - 1, 1)]!;
    inner = { x: b.x + p, y: b.y + p, w: b.w - 2 * p, h: b.h - 2 * p };
  }
  const place = (kids: TNode[]) => squarify(kids.map(c => ({ value: c.tokens, data: c })), inner);
  let boxes = place(n.children);
  const thin = boxes.filter(c => !c.data.children && Math.min(c.w, c.h) - 2 * TILE_GAP < MIN_SIDE).map(c => c.data);
  if (thin.length > 1) {
    const more: TNode = { id: `${n.id}${MORE_SUFFIX}`, label: t('context.more'), tokens: 0, cached: 0, cat: n.cat, more: thin.map(x => x.id) };
    for (const x of thin) { more.tokens += x.tokens; more.cached += x.cached; }
    boxes = place([...n.children.filter(c => !thin.includes(c)), more]);
  }
  for (const c of boxes) {
    const g = c.data.children ? GUTTER[Math.min(depth, 1)]! : TILE_GAP;
    // The map's own edges keep no gutter, so the outermost tiles line up with the bar and the tree above and below
    const edge = (at: number, bound: number) => (depth === 0 && Math.abs(at - bound) < 0.5 ? 0 : g);
    const l = edge(c.x, inner.x), top = edge(c.y, inner.y), r = edge(c.x + c.w, inner.x + inner.w), btm = edge(c.y + c.h, inner.y + inner.h);
    lay(c.data, { x: c.x + l, y: c.y + top, w: c.w - l - r, h: c.h - top - btm }, out, depth + 1);
  }
  return out;
}

// A text's width in the tiles' fonts (small: the figure under a label)
export type Measure = (s: string, small?: boolean) => number;

// The most informative label that fits whole: label and target, then a command without flags or a file without its
// extension or folders, then the bare label; nothing when none fits
export function fitLabel(n: TNode, room: number, m: Measure): { a: string; b?: string } | undefined {
  const base = (s: string) => s.split(/[\\/]/).pop()!;
  const short = (s: string) => s.split(' ').slice(0, 2).join(' ');
  const c: { a: string; b?: string }[] = [];
  if (n.target) {
    c.push({ a: n.label, b: n.target });
    if (n.node?.kind === 'call') {
      const bare = n.target.replace(/\s--?[\w-]+(\s(?!-)\S+)?/g, '');
      const file = base(n.target);
      const stem = n.target.includes(' ') ? n.target : file.replace(/\.\w+$/, '');
      c.push({ a: n.label, b: file }, { a: n.target }, { a: n.label, b: bare }, { a: bare }, { a: file }, { a: stem }, { a: short(n.target) }, { a: n.label });
    } else c.push({ a: n.label });
  } else c.push({ a: n.label }, { a: base(n.label) }, { a: base(n.label).replace(/\.\w+$/, '') });
  return c.find(x => x.a && m(x.b ? `${x.a} ${x.b}` : x.a) <= room);
}
