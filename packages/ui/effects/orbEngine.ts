import { ORB_SIZE, type OrbKind } from './presets';

// Own canvas 2D port of the two thinking-orbs states the UI uses, at the 20px preset only:
// 'think' = a face-on ring of dot lanes whose radius undulates, 'fetch' = a slowly turning constellation
// of 8 nodes wired by proximity, with one packet running an edge.
// Geometry, tuning constants and the hash / value noise reproduce thinking-orbs 0.3.1 so frames match it.
// Credit: ported from thinking-orbs 0.3.1 by Jakub Antalik (https://orbs.jakubantalik.com), MIT License, Copyright (c) 2026 Jakub Antalik.
// Full notice in THIRD_PARTY_NOTICES.md.
// Deliberately outside the app's motion switch (see Orb.tsx); only OS reduced motion freezes it.

interface Dot { x: number; y: number; z: number; r: number; ink: number; a: number }
interface Line { x1: number; y1: number; x2: number; y2: number; ink: number; a: number; w: number }

const TAU = Math.PI * 2;
const C = ORB_SIZE / 2;
// Library's dot-radius scale for the canvas size: (size / 300) ^ 0.6
const RS = (ORB_SIZE / 300) ** 0.6;
const R_MIN = 0.3;
// Static frame time (already in animation units) under reduced motion
const STILL_T = 0.6;

// The 20px preset after the library's count / size scaling (base value x factor, rounded where it counts)
const RING = {
  speed: 3.78,
  lanes: 8, // round(max(2, round(5 * sqrt(0.028))) * 3.968)
  segs: 15, // round(88 * sqrt(0.028))
  rBase: 1.1 * 1.622,
  rDepth: 1.7 * 1.622,
  wob: 0.565,
  radius: C * 0.78,
};
// Face-on: the wobble lives on the radius, so the rest radius shrinks to keep the peaks inside
const RING_REST = RING.radius / (1 + 0.85 * 0.23 * RING.wob);

const WEB = {
  speed: 6.63,
  nodes: 8, // round(30 * 0.25)
  signals: 1, // round(5 * 0.25)
  thr: 0.72,
  nodeR: 1.4 * 1.52,
  nodeRDepth: 1.8 * 1.52,
  lineW: Math.max(0.6, 0.8 * RS),
  pitch: 0.32,
  radius: C * 0.8,
};

export const ORB_SPEED: Record<OrbKind, number> = { think: RING.speed, fetch: WEB.speed };

// Ring trig per segment, fixed
const SEG_COS: number[] = [];
const SEG_SIN: number[] = [];
for (let s = 0; s < RING.segs; s++) {
  const a = (s / RING.segs) * TAU;
  SEG_COS.push(Math.cos(a));
  SEG_SIN.push(Math.sin(a));
}

// Fibonacci sphere point i of n
function fib(i: number, n: number): [number, number, number] {
  const y = 1 - (2 * (i + 0.5)) / n;
  const r = Math.sqrt(1 - y * y);
  const a = i * Math.PI * (3 - Math.sqrt(5));
  return [r * Math.cos(a), y, r * Math.sin(a)];
}
const WEB_BASE = Array.from({ length: WEB.nodes }, (_, i) => fib(i, WEB.nodes));

// Classic sine hash in [0, 1)
function hash(x: number, y: number): number {
  const v = Math.sin(x * 12.9898 + y * 78.233) * 43758.5453;
  return v - Math.floor(v);
}

// Smoothstep value noise in [0, 1)
function noise(x: number, y: number): number {
  const ix = Math.floor(x);
  const iy = Math.floor(y);
  let fx = x - ix;
  let fy = y - iy;
  fx = fx * fx * (3 - 2 * fx);
  fy = fy * fy * (3 - 2 * fy);
  const a = hash(ix, iy);
  const b = hash(ix + 1, iy);
  const c = hash(ix, iy + 1);
  const d = hash(ix + 1, iy + 1);
  return a + (b - a) * fx + (c - a) * fy + (a - b - c + d) * fx * fy;
}

// Scratch buffers, reused by every instance (drawing is synchronous)
const dots: Dot[] = [];
const lines: Line[] = [];
let nDots = 0;
let nLines = 0;

function dot(x: number, y: number, z: number, r: number, ink: number, a: number) {
  if (a < 0.02) return;
  const d = dots[nDots] ?? (dots[nDots] = { x: 0, y: 0, z: 0, r: 0, ink: 0, a: 0 });
  d.x = x; d.y = y; d.z = z; d.r = Math.max(R_MIN, r); d.ink = ink; d.a = a;
  nDots++;
}

function line(x1: number, y1: number, x2: number, y2: number, ink: number, a: number, w: number) {
  if (a < 0.02) return;
  const l = lines[nLines] ?? (lines[nLines] = { x1: 0, y1: 0, x2: 0, y2: 0, ink: 0, a: 0, w: 0 });
  l.x1 = x1; l.y1 = y1; l.x2 = x2; l.y2 = y2; l.ink = ink; l.a = a; l.w = w;
  nLines++;
}

// Face-on ring: lanes stacked along the view axis (depth only shades them), radius undulating per segment.
// The camera's 0.3 rad pitch is cancelled by the ring's own tilt, so the projection reduces to a plain circle.
function ringFrame(t: number) {
  const mid = (RING.lanes - 1) / 2;
  for (let lane = 0; lane < RING.lanes; lane++) {
    const off = (lane - mid) * 0.075;
    const edge = Math.abs(lane - mid) / Math.max(1, mid);
    const norm = 1 / Math.sqrt(1 + off * off);
    const shrink = 1 - 0.25 * edge;
    for (let s = 0; s < RING.segs; s++) {
      const a = (s / RING.segs) * TAU;
      const wob = (0.16 * Math.sin(a * 3 - t * 1.7 + lane * 0.22) + 0.07 * Math.sin(a * 5 + t * 1.1)) * RING.wob;
      const k = RING_REST * (1 + wob) * norm;
      const z = off * k;
      const depth = (z / RING.radius + 1) / 2;
      dot(
        C + SEG_COS[s]! * k,
        C - SEG_SIN[s]! * k,
        z,
        (RING.rBase + RING.rDepth * depth) * shrink * RS,
        0.52 - 0.44 * depth + 0.18 * edge,
        0.4 + 0.6 * depth,
      );
    }
  }
}

// Constellation: jittered sphere nodes under a slow yaw, edges below a chord threshold, node pulses, one packet
function webFrame(t: number) {
  const yaw = t * 0.12;
  const sy = Math.sin(yaw), cy = Math.cos(yaw), sp = Math.sin(WEB.pitch), cp = Math.cos(WEB.pitch);
  const sc = WEB.radius;
  const nodes: [number, number, number][] = [];
  const proj: [number, number, number][] = [];
  // Yaw about y, then pitch about x; screen y points down
  const project = (x: number, y: number, z: number): [number, number, number] => {
    const rx = x * cy + z * sy;
    const rz = -x * sy + z * cy;
    return [C + rx * sc, C - (y * cp - rz * sp) * sc, y * sp + rz * cp];
  };
  for (let i = 0; i < WEB.nodes; i++) {
    const b = WEB_BASE[i]!;
    const x = b[0] + 0.6 * (noise(i * 0.31 + 9, t * 0.24) - 0.5);
    const y = b[1] + 0.6 * (noise(i * 0.53 + 27, t * 0.21) - 0.5);
    const z = b[2] + 0.6 * (noise(i * 0.77 + 55, t * 0.27) - 0.5);
    const l = Math.sqrt(x * x + y * y + z * z);
    const n: [number, number, number] = [x / l, y / l, z / l];
    nodes.push(n);
    proj.push(project(n[0], n[1], n[2]));
  }
  for (let i = 0; i < WEB.nodes; i++) {
    for (let j = i + 1; j < WEB.nodes; j++) {
      const p = nodes[i]!, q = nodes[j]!;
      const dist = Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
      if (dist >= WEB.thr) continue;
      const a = proj[i]!, b = proj[j]!;
      const depth = ((a[2] + b[2]) / 2 + 1) / 2;
      line(a[0], a[1], b[0], b[1], 0.42, (1 - dist / WEB.thr) * (0.3 + 0.55 * depth), WEB.lineW);
    }
  }
  for (let i = 0; i < WEB.nodes; i++) {
    const [x, y, z] = proj[i]!;
    const depth = (z + 1) / 2;
    const pulse = 1 + 0.25 * Math.sin(t * 1.4 + i * 2.7);
    dot(x, y, z, (WEB.nodeR + WEB.nodeRDepth * depth) * pulse * RS, 0.55 - 0.45 * depth, 1);
  }
  // Packets: each picks a node pair per period and slides along the chord, pushed back onto the sphere
  for (let i = 0; i < WEB.signals; i++) {
    const phase = t * 0.55 + i * 7.31;
    const period = Math.floor(phase);
    const from = Math.floor(hash(period, i * 3.1 + 1.7) * WEB.nodes);
    const to = Math.floor(hash(period, i * 5.7 + 4.2) * WEB.nodes);
    if (from === to) continue;
    const f = phase - period;
    const p = nodes[from]!, q = nodes[to]!;
    const x = p[0] + (q[0] - p[0]) * f, y = p[1] + (q[1] - p[1]) * f, z = p[2] + (q[2] - p[2]) * f;
    const l = Math.max(1e-6, Math.sqrt(x * x + y * y + z * z));
    const [px, py, pz] = project(x / l, y / l, z / l);
    const depth = (pz + 1) / 2;
    dot(px, py, pz, (WEB.nodeR * 1.5 + WEB.nodeRDepth * depth) * RS, 0.05, 0.5 + 0.5 * depth);
  }
}

const byDepth = (a: Dot, b: Dot) => a.z - b.z;

// Ink 0 = darkest on light paper; dark paper flips it so the same frame reads as light ink
function grey(ink: number, dark: boolean) {
  const v = Math.min(1, Math.max(0, ink));
  return Math.round((dark ? 1 - v : v) * 255);
}

// Draw one frame at animation time t (seconds x the state's speed) into a context already scaled to CSS px
export function paintOrb(ctx: CanvasRenderingContext2D, kind: OrbKind, t: number, dark: boolean) {
  nDots = 0;
  nLines = 0;
  if (kind === 'think') ringFrame(t);
  else webFrame(t);
  for (let i = 0; i < nLines; i++) {
    const l = lines[i]!;
    const g = grey(l.ink, dark);
    ctx.strokeStyle = `rgba(${g},${g},${g},${l.a})`;
    ctx.lineWidth = l.w;
    ctx.beginPath();
    ctx.moveTo(l.x1, l.y1);
    ctx.lineTo(l.x2, l.y2);
    ctx.stroke();
  }
  // Back to front; stable sort keeps insertion order on ties like the library
  const list = dots.slice(0, nDots).sort(byDepth);
  for (const d of list) {
    const g = grey(d.ink, dark);
    ctx.fillStyle = `rgba(${g},${g},${g},${d.a})`;
    ctx.beginPath();
    ctx.arc(d.x, d.y, d.r, 0, TAU);
    ctx.fill();
  }
}

// Mounted canvases share one rAF loop, one IntersectionObserver, one theme observer and one media listener

interface Inst {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  kind: OrbKind;
  paused: boolean;
  dpr: number;
  dark: boolean;
  visible: boolean;
}

const insts = new Set<Inst>();
const byCanvas = new WeakMap<Element, Inst>();
let raf = 0;
let reduced = false;
let io: IntersectionObserver | null = null;
let mo: MutationObserver | null = null;
let mqDark: MediaQueryList | null = null;
let mqReduced: MediaQueryList | null = null;

// Nearest ancestor data-theme or dark / light class, else the OS scheme
function resolveDark(el: Element): boolean {
  for (let e: Element | null = el; e; e = e.parentElement) {
    const t = e.getAttribute('data-theme');
    if (t === 'dark') return true;
    if (t === 'light') return false;
    if (e.classList.contains('dark')) return true;
    if (e.classList.contains('light')) return false;
  }
  return mqDark?.matches ?? true;
}

function draw(i: Inst, now: number) {
  const t = reduced ? STILL_T : (now / 1000) * ORB_SPEED[i.kind];
  i.ctx.setTransform(i.dpr, 0, 0, i.dpr, 0, 0);
  i.ctx.clearRect(0, 0, ORB_SIZE, ORB_SIZE);
  paintOrb(i.ctx, i.kind, t, i.dark);
}

const running = (i: Inst) => i.visible && !i.paused && !reduced;

function tick() {
  raf = 0;
  const now = performance.now();
  let any = false;
  for (const i of insts) {
    if (!running(i)) continue;
    draw(i, now);
    any = true;
  }
  if (any) raf = requestAnimationFrame(tick);
}

function wake() {
  if (raf) return;
  for (const i of insts) {
    if (running(i)) {
      raf = requestAnimationFrame(tick);
      return;
    }
  }
}

// Re-resolve the theme of every orb (or only those under a mutated element) and repaint the ones that flipped
function retheme(targets?: Node[]) {
  for (const i of insts) {
    if (targets && !targets.some(n => n.contains(i.canvas))) continue;
    const dark = resolveDark(i.canvas);
    if (dark === i.dark) continue;
    i.dark = dark;
    draw(i, performance.now());
  }
}

function onReduced() {
  reduced = mqReduced?.matches ?? false;
  for (const i of insts) draw(i, performance.now());
  wake();
}

function onTheme() {
  retheme();
}

function setup() {
  if (typeof matchMedia !== 'undefined') {
    mqDark = matchMedia('(prefers-color-scheme: dark)');
    mqReduced = matchMedia('(prefers-reduced-motion: reduce)');
    reduced = mqReduced.matches;
    mqDark.addEventListener('change', onTheme);
    mqReduced.addEventListener('change', onReduced);
  }
  if (typeof IntersectionObserver !== 'undefined') {
    io = new IntersectionObserver(entries => {
      for (const e of entries) {
        const i = byCanvas.get(e.target);
        if (i) i.visible = e.isIntersecting;
      }
      wake();
    });
  }
  if (typeof MutationObserver !== 'undefined') {
    mo = new MutationObserver(records => retheme(records.map(r => r.target)));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'data-theme'], subtree: true });
  }
}

function teardown() {
  cancelAnimationFrame(raf);
  raf = 0;
  io?.disconnect();
  mo?.disconnect();
  mqDark?.removeEventListener('change', onTheme);
  mqReduced?.removeEventListener('change', onReduced);
  io = mo = mqDark = mqReduced = null;
}

// Attach an orb to a canvas; returns the detach function. Paused (or reduced motion) holds one still frame
export function mountOrb(canvas: HTMLCanvasElement, kind: OrbKind, paused: boolean): () => void {
  const ctx = canvas.getContext('2d');
  if (!ctx) return () => {};
  if (!insts.size) setup();
  const dpr = Math.min(2, (typeof devicePixelRatio !== 'undefined' && devicePixelRatio) || 1);
  canvas.width = Math.round(ORB_SIZE * dpr);
  canvas.height = Math.round(ORB_SIZE * dpr);
  const i: Inst = { canvas, ctx, kind, paused, dpr, dark: resolveDark(canvas), visible: !io };
  insts.add(i);
  byCanvas.set(canvas, i);
  draw(i, performance.now());
  io?.observe(canvas);
  wake();
  return () => {
    insts.delete(i);
    byCanvas.delete(canvas);
    io?.unobserve(canvas);
    if (!insts.size) teardown();
  };
}
