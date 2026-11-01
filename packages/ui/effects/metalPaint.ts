// The send button's silver ring, drawn on the CPU into a 2D canvas: no WebGL, so no context to lose or to be refused.
// The ring is 1 px wide, so only a band of cells along the edge is evaluated each frame (a few hundred), then
// bilinearly stretched over the canvas and punched out to a ring.
//
// Credit: ported from metal-fx 1.0.4 by Jakub Antalik (https://metal.jakubantalik.com), MIT License, Copyright (c) 2026 Jakub Antalik.
// Full notice in THIRD_PARTY_NOTICES.md.
// Look reproduced from metal-fx: its "silver" preset, the Plasma field of its fragment
// shader (simplex FBM warp + four sine bands through a five-stop palette, 5-tap blur, gamma 1.3, edge vignette), the
// button variant's crop of a 140 x 40 baseline at shader scale 1.6, its 15 fps cadence, and the GL blend that leaves
// the ring at colour x opacity with alpha opacity squared. The simplex noise is the public-domain Ashima / McEwan one.

export type MetalTheme = 'dark' | 'light';

interface Mode {
  stops: readonly [number, number, number][];
  vignette: number;
  vigOpacity: number;
  opacity: number;
  // inset hairline over the ring (metal-fx draws it as a 1 px inset box-shadow)
  hairline: string;
}

const hex = (h: string): [number, number, number] => [0, 2, 4].map(i => parseInt(h.slice(1 + i, 3 + i), 16) / 255) as [number, number, number];

// Only the first five palette stops reach the Plasma field
const SILVER: Record<MetalTheme, Mode> = {
  dark: { stops: ['#000000', '#dedede', '#747270', '#e5e5e5', '#0d0d0d'].map(hex), vignette: 0.26, vigOpacity: 0.6, opacity: 0.88, hairline: 'rgba(255,255,255,0.1)' },
  light: { stops: ['#f6f6f6', '#ffffff', '#ffffff', '#f7f7f7', '#c9c9c9'].map(hex), vignette: 0.2, vigOpacity: 0.26, opacity: 1, hairline: 'rgba(0,0,0,0.06)' },
};

// Shared preset numbers (identical in both silver modes)
const SPEED = 1.2;
const DIR = (80 * Math.PI) / 180;
const DIR_X = Math.cos(DIR) * 0.15, DIR_Y = Math.sin(DIR) * 0.15;
const FIELD_SCALE = 2.5;
const FREQ = 3 + 0.68 * 8;
const WARP = 0.3 * 2; // distortion x 2
const DIST = 0.3;
const OCTAVES = 5; // int(3 + complexity 0.68 x 4)
const BLUR = 0.02;
// The GL canvas metal-fx renders into is 96 px a side at up to 2x; the vignette is sized in its pixels
const GL_SIDE = 96;
// Button-variant geometry: crop of a 140 x 40 baseline at shader scale 1.6, 1 px ring
export const BASE_W = 140, BASE_H = 40;
const SHADER_SCALE = 1.6;
const FRAME_MS = 66;
// Cells evaluated per CSS px, and how far past the ring the band reaches (for the bilinear stretch)
const DENSITY = 1;
const BAND_PAD = 1;

// Simplex noise permutation, (34x + 1)x mod 289, over every index the lookups can reach
const PERM = new Float64Array(580);
for (let k = 0; k < PERM.length; k++) PERM[k] = ((k * 34 + 1) * k) % 289;
const perm = (k: number) => PERM[k] ?? 0;
const mod289 = (x: number) => ((x % 289) + 289) % 289;

function corner(p: number, x: number, y: number, m: number): number {
  if (m <= 0) return 0;
  const v = 2 * ((p * 0.024390243902439) % 1) - 1;
  const h = Math.abs(v) - 0.5;
  const a = v - Math.floor(v + 0.5);
  m *= m; m *= m;
  return m * (1.79284291400159 - 0.85373472095314 * (a * a + h * h)) * (a * x + h * y);
}

function snoise(vx: number, vy: number): number {
  const s = (vx + vy) * 0.366025403784439;
  const ix = Math.floor(vx + s), iy = Math.floor(vy + s);
  const t = (ix + iy) * 0.211324865405187;
  const x0 = vx - ix + t, y0 = vy - iy + t;
  const i1x = x0 > y0 ? 1 : 0, i1y = 1 - i1x;
  const x1 = x0 + 0.211324865405187 - i1x, y1 = y0 + 0.211324865405187 - i1y;
  const x2 = x0 - 0.577350269189626, y2 = y0 - 0.577350269189626;
  const mx = mod289(ix), my = mod289(iy);
  return 130 * (
    corner(perm(perm(my) + mx), x0, y0, 0.5 - x0 * x0 - y0 * y0)
    + corner(perm(perm(my + i1y) + mx + i1x), x1, y1, 0.5 - x1 * x1 - y1 * y1)
    + corner(perm(perm(my + 1) + mx + 1), x2, y2, 0.5 - x2 * x2 - y2 * y2));
}

function fbm(x: number, y: number): number {
  let v = 0, a = 0.5;
  for (let i = 0; i < OCTAVES; i++) { v += a * snoise(x, y); x *= 2; y *= 2; a *= 0.5; }
  return v;
}

const rgb: [number, number, number] = [0, 0, 0];

// Plasma at one uv (y up), accumulated into `rgb` with weight `k`
function plasma(ux: number, uy: number, t: number, stops: Mode['stops'], k: number) {
  const px = (ux - 0.5) * FIELD_SCALE + DIR_X * t, py = (uy - 0.5) * FIELD_SCALE + DIR_Y * t;
  let v = Math.sin(px * FREQ + t) + Math.sin(py * FREQ + t * 1.3) + Math.sin((px + py) * FREQ * 0.7 + t * 0.7)
    + Math.sin(Math.sqrt(px * px + py * py) * FREQ * 0.8 - t * 1.5);
  v += (fbm(px + t * 0.1, py) + fbm(px + 5, py + t * 0.12 + 5)) * WARP * DIST;
  v = Math.min(1, Math.max(0, v * 0.4 + 0.5));
  v = v * v * (3 - 2 * v);
  let r = 0, g = 0, b = 0, sum = 0.0001;
  for (let i = 0; i < 5; i++) {
    const d = v - i * 0.25, w = Math.exp(-64 * d * d), c = stops[i]!;
    r += c[0] * w; g += c[1] * w; b += c[2] * w; sum += w;
  }
  rgb[0] += (r / sum) * k; rgb[1] += (g / sum) * k; rgb[2] += (b / sum) * k;
}

// Shader output at one uv as stored in the GL canvas: colour premultiplied by the preset opacity once (the blend)
function shade(ux: number, uy: number, t: number, mode: Mode, glRes: number, out: Uint8ClampedArray | number[], at: number) {
  rgb[0] = rgb[1] = rgb[2] = 0;
  plasma(ux, uy, t, mode.stops, 0.4);
  plasma(ux + BLUR, uy, t, mode.stops, 0.15);
  plasma(ux - BLUR, uy, t, mode.stops, 0.15);
  plasma(ux, uy + BLUR, t, mode.stops, 0.15);
  plasma(ux, uy - BLUR, t, mode.stops, 0.15);
  const edge = Math.min(ux, 1 - ux, uy, 1 - uy);
  const range = (40 / glRes) * (1 + mode.vignette * 3);
  let vig = Math.min(1, (edge * edge) / (range * range));
  vig = vig * vig * (3 - 2 * vig);
  const f = (1 + (vig - 1) * mode.vignette * mode.vigOpacity) * mode.opacity * 255;
  out[at] = Math.pow(rgb[0], 1.3) * f;
  out[at + 1] = Math.pow(rgb[1], 1.3) * f;
  out[at + 2] = Math.pow(rgb[2], 1.3) * f;
  out[at + 3] = mode.opacity * mode.opacity * 255;
}

export interface RingGeometry { w: number; h: number; r: number }

export interface Ring {
  canvas: HTMLCanvasElement;
  geo: RingGeometry;
  theme: MetalTheme;
  scale: number;
  paused: boolean;
  visible: boolean;
  painted: boolean;
  // Glow hook, run on each frame the ring animates
  onFrame?: (now: number) => void;
  // internal
  dpr: number;
  field: HTMLCanvasElement;
  fieldCtx: CanvasRenderingContext2D | null;
  img: ImageData | null;
  cells: Int32Array;
  uv: Float64Array;
}

const rings = new Set<Ring>();
let start = 0, raf = 0, last = -Infinity;

export const clock = (now = performance.now()) => ((now - start) / 1000) * SPEED;
const glRes = () => GL_SIDE * Math.min(2, window.devicePixelRatio || 1);

// Crop the button samples from the field: centred, its size over the scaled baseline, y up
export function uvAt(ring: Ring, x: number, y: number, out: [number, number]) {
  const ss = SHADER_SCALE * ring.scale;
  const cw = Math.min(1, ring.geo.w / (BASE_W * ss)), ch = Math.min(1, ring.geo.h / (BASE_H * ss));
  out[0] = 0.5 + (x / ring.geo.w - 0.5) * cw;
  out[1] = 0.5 + (0.5 - y / ring.geo.h) * ch;
  return out;
}

const px4 = [0, 0, 0, 0];
const uv2: [number, number] = [0, 0];
// Shader output (0..255, as metal-fx reads it back) at a CSS point of the ring, at time t
export function sample(ring: Ring, x: number, y: number, t: number) {
  uvAt(ring, x, y, uv2);
  shade(uv2[0], uv2[1], t, SILVER[ring.theme], glRes(), px4, 0);
  return px4;
}

// Signed distance to a rounded rect's edge, negative inside
function sdf(x: number, y: number, g: RingGeometry): number {
  const qx = Math.abs(x - g.w / 2) - (g.w / 2 - g.r), qy = Math.abs(y - g.h / 2) - (g.h / 2 - g.r);
  return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - g.r;
}

const ringPx = (ring: Ring) => ring.scale;

// Size the canvas and pick the band cells after a geometry or scale change
function layout(ring: Ring) {
  const { geo } = ring;
  ring.dpr = window.devicePixelRatio || 1;
  const cw = Math.max(1, Math.round(geo.w * ring.dpr)), ch = Math.max(1, Math.round(geo.h * ring.dpr));
  if (ring.canvas.width !== cw) ring.canvas.width = cw;
  if (ring.canvas.height !== ch) ring.canvas.height = ch;
  const gw = Math.max(1, Math.ceil(geo.w * DENSITY)), gh = Math.max(1, Math.ceil(geo.h * DENSITY));
  ring.field.width = gw; ring.field.height = gh;
  ring.fieldCtx = ring.field.getContext('2d');
  ring.img = ring.fieldCtx?.createImageData(gw, gh) ?? null;
  const band = ringPx(ring) + BAND_PAD, cells: number[] = [], uv: number[] = [];
  for (let j = 0; j < gh; j++) {
    for (let i = 0; i < gw; i++) {
      const x = ((i + 0.5) / gw) * geo.w, y = ((j + 0.5) / gh) * geo.h;
      if (sdf(x, y, geo) < -band) continue;
      uvAt(ring, x, y, uv2);
      cells.push((j * gw + i) * 4);
      uv.push(uv2[0], uv2[1]);
    }
  }
  ring.cells = Int32Array.from(cells);
  ring.uv = Float64Array.from(uv);
}

function paint(ring: Ring, t: number) {
  const ctx = ring.canvas.getContext('2d');
  const { img, fieldCtx, cells, uv, dpr, geo } = ring;
  if (!ctx || !img || !fieldCtx) return;
  const mode = SILVER[ring.theme], res = glRes();
  for (let c = 0; c < cells.length; c++) shade(uv[c * 2]!, uv[c * 2 + 1]!, t, mode, res, img.data, cells[c]!);
  fieldCtx.putImageData(img, 0, 0);
  const { width: W, height: H } = ring.canvas, rp = ringPx(ring) * dpr, r = geo.r * dpr;
  ctx.save();
  ctx.clearRect(0, 0, W, H);
  ctx.imageSmoothingEnabled = true;
  ctx.drawImage(ring.field, 0, 0, W, H);
  // Keep only the ring: the outer rounded rect minus the inner one
  ctx.globalCompositeOperation = 'destination-in';
  ctx.beginPath(); ctx.roundRect(0, 0, W, H, r); ctx.fill();
  ctx.globalCompositeOperation = 'destination-out';
  ctx.beginPath(); ctx.roundRect(rp, rp, W - 2 * rp, H - 2 * rp, Math.max(0, r - rp)); ctx.fill();
  // 1 px hairline on the ring (a CSS inset shadow in metal-fx, so it does not follow `scale`)
  ctx.globalCompositeOperation = 'source-over';
  ctx.strokeStyle = mode.hairline;
  ctx.lineWidth = dpr;
  const hl = ctx.lineWidth / 2;
  ctx.beginPath(); ctx.roundRect(hl, hl, W - 2 * hl, H - 2 * hl, Math.max(0, r - hl)); ctx.stroke();
  ctx.restore();
  ring.painted = true;
}

function loop(now: number) {
  raf = 0;
  let live = false;
  for (const ring of rings) if (ring.visible && !ring.paused) { live = true; break; }
  if (!live || document.hidden) return;
  raf = requestAnimationFrame(loop);
  if (now - last < FRAME_MS) return;
  last = now;
  const t = clock(now);
  for (const ring of rings) {
    if (!ring.visible || ring.paused) continue;
    paint(ring, t);
    ring.onFrame?.(now);
  }
}

function wake() {
  if (!raf && rings.size && !document.hidden) raf = requestAnimationFrame(loop);
}

if (typeof document !== 'undefined') document.addEventListener('visibilitychange', wake);

export function createRing(canvas: HTMLCanvasElement, geo: RingGeometry, theme: MetalTheme, scale: number, paused: boolean): Ring {
  if (!rings.size) start = performance.now();
  const ring: Ring = {
    canvas, geo, theme, scale, paused, visible: true, painted: false,
    dpr: 1, field: document.createElement('canvas'), fieldCtx: null, img: null, cells: new Int32Array(0), uv: new Float64Array(0),
  };
  layout(ring);
  // Paint now so the ring is there on the first frame, paused or not
  paint(ring, clock());
  rings.add(ring);
  wake();
  return ring;
}

export function updateRing(ring: Ring, patch: Partial<Pick<Ring, 'geo' | 'theme' | 'scale' | 'paused' | 'visible'>>) {
  const relayout = (patch.geo && (patch.geo.w !== ring.geo.w || patch.geo.h !== ring.geo.h || patch.geo.r !== ring.geo.r))
    || (patch.scale !== undefined && patch.scale !== ring.scale) || ring.dpr !== (window.devicePixelRatio || 1);
  const repaint = relayout || (patch.theme !== undefined && patch.theme !== ring.theme);
  Object.assign(ring, patch);
  if (relayout) layout(ring);
  if (repaint) paint(ring, clock());
  wake();
}

export function destroyRing(ring: Ring) {
  rings.delete(ring);
  ring.field.width = ring.field.height = 0;
  if (!rings.size && raf) { cancelAnimationFrame(raf); raf = 0; }
}
