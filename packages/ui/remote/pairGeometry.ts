// The pairing card's drawing: the QR code from the gateway's rows (pairing/PairingCore.tsx) and the countdown ring around
// it. Pure geometry in the drawing's own units, kept apart from the components so their markup carries no numbers.
// Decoding was checked with Apple Vision on real version 10 codes (error correction M, the gateway's `qrcode` default):
// dots, an 11-module hole with the mark, both themes. Finder patterns are kept whole; the timing rows are drawn as modules

/** The code's box in CSS pixels, quiet zone included; the ring runs on its edge */
export const QR_BOX = 228;
/** The quiet zone, in modules */
export const QUIET = 4;
/** The rings the success moment dissolves outward, counted from the centre */
export const BAND_COUNT = 11;
/** A dot's radius and the finder eye's three radii (outer ring, its hole, the inner square), in modules */
const DOT_R = 0.42;
const EYE = [2.3, 1.5, 1.05] as const;

/** The centre knock-out for the mark, in modules: 11 on a 57-module code, smaller on smaller codes (odd, so it is
 * centred on whole modules) */
export function holeOf(n: number): number {
  const h = Math.max(7, Math.min(11, Math.round(n * 11 / 57)));
  return h % 2 === n % 2 ? h : h - 1;
}

/** The knock-out's share of the box before a code arrives: a version 10 code's (57 modules), the size a pairing link
 *  takes, so the mark keeps its place when the code fills in */
export const WAIT_HOLE = holeOf(57) / (57 + QUIET * 2);

// Clockwise rounded rectangle (counter-clockwise for a hole under the nonzero rule)
function rr(x: number, y: number, w: number, h: number, r: number, ccw = false) {
  const iw = w - 2 * r, ih = h - 2 * r;
  return ccw
    ? `M${x + r} ${y}a${r} ${r} 0 0 0 ${-r} ${r}v${ih}a${r} ${r} 0 0 0 ${r} ${r}h${iw}a${r} ${r} 0 0 0 ${r} ${-r}v${-ih}a${r} ${r} 0 0 0 ${-r} ${-r}z`
    : `M${x + r} ${y}h${iw}a${r} ${r} 0 0 1 ${r} ${r}v${ih}a${r} ${r} 0 0 1 ${-r} ${r}h${-iw}a${r} ${r} 0 0 1 ${-r} ${-r}v${-ih}a${r} ${r} 0 0 1 ${r} ${-r}z`;
}
const dot = (cx: number, cy: number, r: number) => `M${cx - r} ${cy}a${r} ${r} 0 1 1 ${2 * r} 0a${r} ${r} 0 1 1 ${-2 * r} 0z`;

export interface QrShape {
  n: number;
  hole: number;
  /** Module paths, one per ring from the centre out */
  bands: string[];
  /** The three finder eyes and the ring each belongs to */
  eyes: { d: string; band: number }[];
}

const cache = new Map<string, QrShape>();

/** The code's rows (`1` dark, no quiet zone) as round dots in rings, the finders as rounded eyes, the centre left open */
export function qrShape(rows: string[]): QrShape {
  const key = rows.join('');
  const hit = cache.get(key);
  if (hit) return hit;
  const n = rows.length;
  const hole = holeOf(n);
  const c = (n - 1) / 2, max = Math.hypot(c, c) || 1;
  const band = (x: number, y: number) => Math.min(BAND_COUNT - 1, Math.floor(Math.hypot(x - c, y - c) / max * BAND_COUNT));
  const finder = (x: number, y: number) => (x < 7 && y < 7) || (x >= n - 7 && y < 7) || (x < 7 && y >= n - 7);
  const h0 = (n - hole) / 2;
  const open = (x: number, y: number) => x >= h0 && x < h0 + hole && y >= h0 && y < h0 + hole;
  const bands = Array.from({ length: BAND_COUNT }, () => '');
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
    if (rows[y]?.[x] !== '1' || finder(x, y) || open(x, y)) continue;
    bands[band(x, y)] += dot(x + 0.5, y + 0.5, DOT_R);
  }
  const [ro, rh, ri] = EYE;
  const eyes = n < 7 ? [] : [[0, 0], [n - 7, 0], [0, n - 7]].map(([fx, fy]) => ({
    d: rr(fx!, fy!, 7, 7, ro) + rr(fx! + 1, fy! + 1, 5, 5, rh, true) + rr(fx! + 2, fy! + 2, 3, 3, ri),
    band: band(fx! + 3, fy! + 3),
  }));
  const shape = { n, hole, bands, eyes };
  if (cache.size > 8) cache.clear();
  cache.set(key, shape);
  return shape;
}

/** The ring's stroke, corner radius and the height of a straight run's strip, in CSS pixels */
export const RING_STROKE = 1.5;
const RING_R = 16;
export const STRIP_H = 8;

export type RingPiece =
  | { kind: 'side'; s0: number; len: number; x: number; y: number; deg: number }
  | { kind: 'corner'; s0: number; len: number; cx: number; cy: number; a0: number };

// The ring's centre line on the box edge, cut into its five straight runs and four corners, clockwise from 12 o'clock
export const RING = (() => {
  const S = QR_BOX, a = RING_STROKE / 2, R = RING_R - a, cx = S / 2, ls = S - 2 * a - 2 * R, qa = Math.PI * R / 2;
  const pieces: RingPiece[] = [];
  let s = 0;
  const side = (x: number, y: number, deg: number, len: number) => { pieces.push({ kind: 'side', s0: s, len, x, y, deg }); s += len; };
  const corner = (ccx: number, ccy: number, a0: number) => { pieces.push({ kind: 'corner', s0: s, len: qa, cx: ccx, cy: ccy, a0 }); s += qa; };
  side(cx, a, 0, ls / 2);
  corner(S - a - R, a + R, -90);
  side(S - a, a + R, 90, ls);
  corner(S - a - R, S - a - R, 0);
  side(S - a - R, S - a, 180, ls);
  corner(a + R, S - a - R, 90);
  side(a, S - a - R, 270, ls);
  corner(a + R, a + R, 180);
  side(a + R, a, 0, ls / 2);
  const L = s;
  /** The point at distance d along the ring */
  const at = (d: number) => {
    const p = pieces.find(q => d <= q.s0 + q.len) ?? pieces[pieces.length - 1]!;
    const u = Math.max(0, Math.min(p.len, d - p.s0));
    if (p.kind === 'side') { const r = p.deg * Math.PI / 180; return { x: p.x + Math.cos(r) * u, y: p.y + Math.sin(r) * u }; }
    const r = (p.a0 + u / R * 180 / Math.PI) * Math.PI / 180;
    return { x: p.cx + Math.cos(r) * R, y: p.cy + Math.sin(r) * R };
  };
  const track = `M${cx} ${a}H${S - a - R}A${R} ${R} 0 0 1 ${S - a} ${a + R}V${S - a - R}A${R} ${R} 0 0 1 ${S - a - R} ${S - a}H${a + R}A${R} ${R} 0 0 1 ${a} ${S - a - R}V${a + R}A${R} ${R} 0 0 1 ${a + R} ${a}Z`;
  // A corner's box: whole pixels, since a fractional one is snapped and its arc no longer meets the straight runs
  const cornerBox = (p: Extract<RingPiece, { kind: 'corner' }>) => {
    const half = Math.ceil(R) + 2;
    const ox = Math.floor(p.cx - half), oy = Math.floor(p.cy - half);
    const from = at(p.s0), to = at(p.s0 + p.len);
    return { ox, oy, size: half * 2, d: `M${from.x - ox} ${from.y - oy}A${R} ${R} 0 0 1 ${to.x - ox} ${to.y - oy}` };
  };
  return { S, L, pieces, at, track, cornerBox };
})();

/** How far a straight run's window has slid at lit fraction f: two pixels past the run once burnt, so no sliver stays */
export const slide = (len: number, f: number) => (len + 2) * (1 - f);

/** The spark's walk along the ring from d0 to d1, as transform keyframes about every three pixels */
export function walk(d0: number, d1: number): Keyframe[] {
  const n = Math.max(2, Math.ceil(Math.abs(d1 - d0) / 3));
  return Array.from({ length: n + 1 }, (_, j) => { const p = RING.at(d0 + (d1 - d0) * j / n); return { transform: `translate(${p.x}px, ${p.y}px)` }; });
}
