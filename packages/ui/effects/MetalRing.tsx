import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { cn } from '../ui/cn';
import { BASE_H, BASE_W, clock, createRing, destroyRing, sample, updateRing, type MetalTheme, type Ring, type RingGeometry } from './metalPaint';

export interface MetalRingProps {
  theme?: MetalTheme;
  // Freeze on the current frame (the ring stays painted)
  paused?: boolean;
  // Multiplier for every pixel constant, for a magnified copy (the LAB); 1 in the app
  scale?: number;
  className?: string;
  // A plugin style hook (`data-part`) for the box the ring wraps the child in
  part?: string;
  children: ReactNode;
}

// A silver ring around a single child (the send / stop button), plus a soft halo and catch-light that drift along the
// edge toward the ring's brightest spot. The child stays on screen at all times: both the ring and the glow paint onto
// 2D canvases (no WebGL and no SVG <feGaussianBlur>/<mask> filters, which bloated Skia Graphite/Dawn GPU memory by
// ~240 MB and forced high-frequency compositor commits on Windows WebView2).
export function MetalRing({ theme = 'dark', paused = false, scale = 1, className, part, children }: MetalRingProps) {
  const root = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const glowCanvas = useRef<HTMLCanvasElement>(null);
  const ring = useRef<Ring | null>(null);
  const glowState = useRef<GlowState | null>(null);
  const assets = useRef<GlowAssets | null>(null);
  const [geo, setGeo] = useState<RingGeometry | null>(null);
  const live = useRef({ theme, scale });
  live.current = { theme, scale };

  useLayoutEffect(() => {
    const el = root.current, cv = canvas.current;
    if (!el || !cv) return;
    // Layout size, not the transformed one, so a scaled ancestor does not shrink or grow the ring
    const measure = (): RingGeometry => {
      const w = Math.max(1, el.offsetWidth), h = Math.max(1, el.offsetHeight);
      const child = el.lastElementChild?.firstElementChild;
      const r = (child && parseFloat(getComputedStyle(child).borderTopLeftRadius)) || parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0;
      return { w, h, r: Math.min(r, Math.min(w, h) / 2) };
    };
    const first = measure();
    const inst = createRing(cv, first, live.current.theme, live.current.scale, paused);
    const state = newGlowState();
    glowState.current = state;
    inst.onFrame = now => {
      const gcv = glowCanvas.current;
      if (!gcv) return;
      const a = ensureGlowAssets(assets, inst.geo, live.current.scale, inst.dpr, gcv);
      if (a) stepGlow(inst, state, a, gcv, now, live.current.theme);
    };
    ring.current = inst;
    setGeo(first);
    let frame = 0;
    const ro = new ResizeObserver(() => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const g = measure();
        if (g.w === inst.geo.w && g.h === inst.geo.h && g.r === inst.geo.r) return;
        updateRing(inst, { geo: g });
        Object.assign(state, newGlowState());
        setGeo(g);
      });
    });
    ro.observe(el);
    const io = new IntersectionObserver(entries => {
      for (const e of entries) updateRing(inst, { visible: e.isIntersecting });
    }, { rootMargin: '64px' });
    io.observe(el);
    return () => {
      ro.disconnect();
      io.disconnect();
      if (frame) cancelAnimationFrame(frame);
      destroyRing(inst);
      disposeGlowAssets(assets.current);
      assets.current = null;
      glowState.current = null;
      ring.current = null;
    };
  }, []);

  useEffect(() => { if (ring.current) updateRing(ring.current, { paused }); }, [paused]);
  useEffect(() => {
    const inst = ring.current;
    if (!inst) return;
    updateRing(inst, { theme, scale });
    const gcv = glowCanvas.current, st = glowState.current;
    if (gcv && st) {
      const a = ensureGlowAssets(assets, inst.geo, scale, inst.dpr, gcv);
      if (a) stepGlow(inst, st, a, gcv, performance.now(), theme);
    }
  }, [theme, scale, geo]);

  const pad = geo ? glowPad(geo, scale) : 0;
  // data-theme re-scopes the theme tokens under the ring, as metal-fx's wrapper did
  return (
    <div ref={root} data-theme={theme} data-part={part} className={cn('relative isolate inline-flex', className)} style={geo ? { borderRadius: geo.r } : undefined}>
      <canvas ref={canvas} aria-hidden="true" className="pointer-events-none absolute inset-0 block size-full" />
      {geo && (
        <canvas
          ref={glowCanvas}
          aria-hidden="true"
          className="pointer-events-none absolute block"
          style={glowStyle(geo, pad, theme)}
        />
      )}
      <div className="relative inline-flex items-center justify-center">{children}</div>
    </div>
  );
}

// Glow: two stroke groups that travel the edge. The halo (wide blurred strokes) shows fully on the 1 px ring and at
// half strength outside it; the catch-light is a short bright streak under a radial fade. Numbers are metal-fx's.
const REF_PERIM = perimeter(BASE_W, BASE_H, 20);
const HALO = [[26.4, 8.4, 0.385], [15.6, 4.8, 0.595], [7.2, 2.1, 0.7], [3, 0.9, 0.7]] as const; // width, blur, opacity
const CATCH = [[4 / 3, 2 / 3, 0.85], [2 / 3, 1.35 / 3, 1]] as const;
const FADE_R = 13 / 3;
const SAMPLES = 16;
const INSET = 1.5;
// What #808080 and #404040 let through as luminance
const GREY_80 = 128 / 255, GREY_40 = 64 / 255;

const spread = ([width, blur]: readonly [number, number, number], scale: number) => (width / 2 + 3 * blur) * scale + 1;
const reach = (half: number, strokes: readonly (readonly [number, number, number])[], scale: number) => {
  const pad = Math.max(...strokes.map(v => spread(v, scale)));
  return Math.hypot(half + pad, pad);
};

function glowMetrics(geo: RingGeometry, scale: number) {
  const s = perimeter(geo.w, geo.h, geo.r) / REF_PERIM;
  const haloHalf = Math.max(1, 7.8 * s);
  const streakHalf = Math.max(0.6, (9.13952 / 3) * s);
  const haloReach = Math.ceil(reach(haloHalf, HALO, scale));
  const catchReach = Math.ceil(reach(streakHalf, CATCH, scale));
  const pad = Math.ceil(Math.max(haloReach, catchReach + s * scale));
  return { haloHalf, streakHalf, haloReach, catchReach, pad };
}

function glowPad(geo: RingGeometry, scale: number): number {
  return glowMetrics(geo, scale).pad;
}

function glowStyle(geo: RingGeometry, pad: number, theme: MetalTheme): CSSProperties {
  return {
    left: -pad,
    top: -pad,
    width: geo.w + 2 * pad,
    height: geo.h + 2 * pad,
    opacity: theme === 'light' ? 0.2746 : 0.7,
    ...(theme === 'light' ? { mixBlendMode: 'multiply' } : {}),
  };
}

interface GlowAssets {
  key: string;
  pad: number;
  dpr: number;
  haloBase: HTMLCanvasElement;
  haloTinted: HTMLCanvasElement;
  haloTint: string;
  catchBase: HTMLCanvasElement;
  catchTinted: HTMLCanvasElement;
  catchTint: string;
  ringMask: HTMLCanvasElement;
}

function makeCanvas(w: number, h: number): HTMLCanvasElement {
  const c = document.createElement('canvas');
  c.width = Math.max(1, w);
  c.height = Math.max(1, h);
  return c;
}

function disposeGlowAssets(a: GlowAssets | null) {
  if (!a) return;
  a.haloBase.width = a.haloBase.height = 0;
  a.haloTinted.width = a.haloTinted.height = 0;
  a.catchBase.width = a.catchBase.height = 0;
  a.catchTinted.width = a.catchTinted.height = 0;
  a.ringMask.width = a.ringMask.height = 0;
}

// Pre-render one white blurred stroke stack in device pixels via off-canvas shadow casting so each live frame only
// composites two tiny cached sprites instead of running six SVG feGaussianBlur filters on the GPU
function buildSprite(half: number, strokes: readonly (readonly [number, number, number])[], spriteReach: number, scale: number, dpr: number, fadeR?: number): HTMLCanvasElement | null {
  const size = Math.max(2, Math.ceil(spriteReach * 2 * dpr));
  const c = makeCanvas(size, size);
  const ctx = c.getContext('2d');
  if (!ctx) return null;
  const cx = size / 2;
  const cy = size / 2;
  const halfPx = half * dpr;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  ctx.strokeStyle = '#ffffff';
  ctx.shadowColor = '#ffffff';
  for (const [width, blur, opacity] of strokes) {
    const padPx = Math.ceil(spread([width, blur, opacity], scale) * dpr);
    const off = size + padPx + 16;
    ctx.save();
    ctx.globalAlpha = opacity;
    ctx.lineWidth = width * scale * dpr;
    ctx.shadowBlur = blur * 2 * scale * dpr;
    ctx.shadowOffsetX = off;
    ctx.shadowOffsetY = 0;
    ctx.beginPath();
    ctx.moveTo(cx - halfPx - off, cy);
    ctx.lineTo(cx + halfPx - off, cy);
    ctx.stroke();
    ctx.restore();
  }
  if (fadeR !== undefined) {
    const r = fadeR * scale * dpr;
    const grad = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
    grad.addColorStop(0, 'rgba(255,255,255,1)');
    grad.addColorStop(0.3, 'rgba(255,255,255,1)');
    grad.addColorStop(0.65, `rgba(255,255,255,${GREY_40})`);
    grad.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.globalCompositeOperation = 'destination-in';
    ctx.fillStyle = grad;
    ctx.fillRect(0, 0, size, size);
  }
  return c;
}

// Alpha mask matching the ring's geometry: 1 on the 1 px ring, GREY_80 outside it, 0 inside the button
function buildRingMask(geo: RingGeometry, pad: number, dpr: number): HTMLCanvasElement | null {
  const { w, h, r } = geo;
  const cw = Math.max(1, Math.round((w + 2 * pad) * dpr));
  const ch = Math.max(1, Math.round((h + 2 * pad) * dpr));
  const c = makeCanvas(cw, ch);
  const ctx = c.getContext('2d');
  if (!ctx) return null;
  ctx.scale(dpr, dpr);
  // Half strength everywhere outside the ring's midline
  ctx.fillStyle = `rgba(255,255,255,${GREY_80})`;
  ctx.fillRect(0, 0, w + 2 * pad, h + 2 * pad);
  ctx.globalCompositeOperation = 'destination-out';
  ctx.beginPath();
  ctx.roundRect(pad + 0.5, pad + 0.5, Math.max(0, w - 1), Math.max(0, h - 1), Math.max(0, r - 0.5));
  ctx.fill();
  // Full strength on the 1 px ring band (outer rounded rect minus inner rounded rect)
  ctx.globalCompositeOperation = 'source-over';
  ctx.fillStyle = '#ffffff';
  ctx.beginPath();
  ctx.roundRect(pad, pad, w, h, r);
  ctx.roundRect(pad + 1, pad + 1, Math.max(0, w - 2), Math.max(0, h - 2), Math.max(0, r - 1));
  ctx.fill('evenodd');
  return c;
}

function ensureGlowAssets(ref: { current: GlowAssets | null }, geo: RingGeometry, scale: number, dpr: number, target: HTMLCanvasElement): GlowAssets | null {
  const key = `${geo.w}|${geo.h}|${geo.r}|${scale}|${dpr}`;
  if (ref.current?.key === key) {
    const cw = ref.current.ringMask.width, ch = ref.current.ringMask.height;
    if (target.width !== cw) target.width = cw;
    if (target.height !== ch) target.height = ch;
    return ref.current;
  }
  disposeGlowAssets(ref.current);
  ref.current = null;
  const { haloHalf, streakHalf, haloReach, catchReach, pad } = glowMetrics(geo, scale);
  const haloBase = buildSprite(haloHalf, HALO, haloReach, scale, dpr);
  const catchBase = buildSprite(streakHalf, CATCH, catchReach, scale, dpr, FADE_R);
  const ringMask = buildRingMask(geo, pad, dpr);
  if (!haloBase || !catchBase || !ringMask) return null;
  if (target.width !== ringMask.width) target.width = ringMask.width;
  if (target.height !== ringMask.height) target.height = ringMask.height;
  const created: GlowAssets = {
    key, pad, dpr,
    haloBase, haloTinted: makeCanvas(haloBase.width, haloBase.height), haloTint: '',
    catchBase, catchTinted: makeCanvas(catchBase.width, catchBase.height), catchTint: '',
    ringMask,
  };
  ref.current = created;
  return created;
}

function tintSprite(base: HTMLCanvasElement, tinted: HTMLCanvasElement, color: string) {
  const ctx = tinted.getContext('2d');
  if (!ctx) return;
  ctx.globalCompositeOperation = 'source-over';
  ctx.clearRect(0, 0, tinted.width, tinted.height);
  ctx.drawImage(base, 0, 0);
  ctx.globalCompositeOperation = 'source-in';
  ctx.fillStyle = color;
  ctx.fillRect(0, 0, tinted.width, tinted.height);
}

// Bake light-mode's CSS filter (`saturate(5.355) brightness(0.78)`) directly into the stroke RGB so the glow canvas
// needs no GPU filter pass
function satBright(r: number, g: number, b: number, sat: number, bri: number): RGB {
  const l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  const c = (v: number) => Math.max(0, Math.min(255, Math.round((l + (v - l) * sat) * bri)));
  return [c(r), c(g), c(b)];
}

function perimeter(w: number, h: number, r: number) {
  const c = Math.max(0, Math.min(r, Math.min(w, h) / 2));
  return 2 * Math.max(0, w - 2 * c) + 2 * Math.max(0, h - 2 * c) + 2 * Math.PI * c;
}

const pt = { x: 0, y: 0 };
// Point at arc length `s` clockwise from the top edge's left end, `inset` in from the edge (less `out`)
function edgePoint(s: number, { w, h, r }: RingGeometry, inset: number, out: number) {
  const c = Math.max(0, Math.min(r, Math.min(w, h) / 2));
  const top = Math.max(0, w - 2 * c), side = Math.max(0, h - 2 * c), arc = (Math.PI * c) / 2, len = 2 * (top + side) + 4 * arc;
  let m = ((s % len) + len) % len;
  const rad = Math.max(0, c - inset + out), d = inset - out;
  const at = (x: number, y: number) => { pt.x = x; pt.y = y; return pt; };
  const turn = (cx: number, cy: number, from: number) => {
    const a = from + (arc > 0 ? m / arc : 0) * (Math.PI / 2);
    return at(cx + rad * Math.cos(a), cy + rad * Math.sin(a));
  };
  if (m < top) return at(c + m, d);
  if ((m -= top) < arc) return turn(w - c, c, -Math.PI / 2);
  if ((m -= arc) < side) return at(w - d, c + m);
  if ((m -= side) < arc) return turn(w - c, h - c, 0);
  if ((m -= arc) < top) return at(w - c - m, h - d);
  if ((m -= top) < arc) return turn(c, h - c, Math.PI / 2);
  if ((m -= arc) < side) return at(d, h - c - m);
  m -= side;
  return turn(c, c, Math.PI);
}

function tangent(s: number, g: RingGeometry, inset: number) {
  const a = edgePoint(s - 0.1, g, inset, 0), ax = a.x, ay = a.y;
  const b = edgePoint(s + 0.1, g, inset, 0);
  return Math.atan2(b.y - ay, b.x - ax);
}

interface Tween { from: number; to: number; ms: number; at: number; ease: boolean; val: number; done: boolean }
const tween = (from: number, to: number, ms: number, at: number, ease: boolean): Tween => ({ from, to, ms, at, ease, val: from, done: false });
function run(t: Tween, now: number) {
  if (t.done) return t.val;
  const k = Math.min(1, (now - t.at) / t.ms);
  t.val = t.from + (t.to - t.from) * (t.ease ? k * k * (3 - 2 * k) : k);
  if (k >= 1) t.done = true;
  return t.val;
}
const smooth = (a: number, b: number, x: number) => { const k = Math.max(0, Math.min(1, (x - a) / (b - a))); return k * k * (3 - 2 * k); };

type RGB = [number, number, number];
function newGlowState() {
  return {
    idx: 0, next: -1, appeared: 0, opacity: 0, reloc: null as Tween | null,
    wander: 0, wanderTo: 0, wanderFrames: 0,
    // Brightness along the edge is re-read every 1.5 s, like metal-fx's pixel read-back
    lums: new Float64Array(SAMPLES), arcs: new Float64Array(SAMPLES), sampledAt: -Infinity, sampleT: 0,
    tintFrom: [255, 255, 255] as RGB, tintTo: [255, 255, 255] as RGB, tint: null as Tween | null, tintHold: 0,
  };
}
type GlowState = ReturnType<typeof newGlowState>;

const lum = (p: ArrayLike<number>) => (0.2126 * p[0]! + 0.7152 * p[1]! + 0.0722 * p[2]!) / 255;
const glowLevel = (l: number) => 0.34 + 0.51 * smooth(0.08, 0.32, l);

function stepGlow(ring: Ring, st: GlowState, a: GlowAssets, target: HTMLCanvasElement, now: number, theme: MetalTheme) {
  const ctx = target.getContext('2d');
  if (!ctx) return;
  const { geo, scale } = ring;
  const S = perimeter(geo.w, geo.h, geo.r) / REF_PERIM;
  if (now - st.sampledAt >= 1500) {
    st.sampledAt = now;
    st.sampleT = clock(now);
    const len = perimeter(geo.w, geo.h, geo.r);
    for (let i = 0; i < SAMPLES; i++) {
      st.arcs[i] = (i / SAMPLES) * len;
      const p = edgePoint(st.arcs[i]!, geo, INSET * scale, 0);
      st.lums[i] = lum(sample(ring, p.x, p.y, st.sampleT));
    }
  }
  let best = -1, bestIdx = st.idx;
  for (let i = 0; i < SAMPLES; i++) if (st.lums[i]! > best) { best = st.lums[i]!; bestIdx = i; }
  const cur = st.lums[st.idx]!;
  const young = st.appeared > 0 && now - st.appeared < 3000;
  const targetOpacity = glowLevel(cur);
  if (!st.reloc || st.reloc.done) {
    const restart = (idx: number, level: number) => {
      st.idx = idx; st.appeared = now; st.wander = st.wanderTo = st.wanderFrames = 0;
      st.reloc = tween(0, level, 1500, now, true);
    };
    if (st.appeared === 0) restart(bestIdx, targetOpacity);
    else if (st.reloc?.done && st.reloc.to === 0) restart(st.next, glowLevel(st.lums[st.next]!));
    else if (!young && best - cur > 0.05) { st.next = bestIdx; st.reloc = tween(st.opacity, 0, 1500, now, true); }
    else st.opacity += (targetOpacity - st.opacity) * 0.00875;
  }
  if (st.reloc) st.opacity = run(st.reloc, now);
  st.opacity = Math.max(0, Math.min(1, st.opacity));
  if (st.wanderFrames++ >= 120) { st.wanderTo = (Math.random() * 2 - 1) * 15 * S; st.wanderFrames = 0; }
  st.wander += (st.wanderTo - st.wander) * 0.0075;

  const s = st.arcs[st.idx]! + st.wander, inset = INSET * scale;
  const angle = tangent(s, geo, inset);
  const p = edgePoint(s, geo, inset, 0), x = p.x, y = p.y;
  const q = edgePoint(s, geo, inset, S * scale), qx = q.x, qy = q.y;

  // Tint: the ring's colour under the halo; dark holds each colour 2 s and normalises it to full brightness
  const light = theme === 'light';
  const px = sample(ring, x, y, st.sampleT), seen: RGB = [px[0]!, px[1]!, px[2]!];
  if (!st.tint) {
    st.tintFrom = [...seen]; st.tintTo = [...seen]; st.tint = tween(0, 1, 400, now, false); st.tintHold = light ? 0 : now + 2000;
  } else if (st.tint.done && (light || now >= st.tintHold)) {
    const k = st.tint.val;
    st.tintFrom = light ? st.tintFrom.map((v, i) => v + (st.tintTo[i]! - v) * k) as RGB : [...st.tintTo];
    st.tintTo = seen; st.tint = tween(0, 1, 400, now, false);
    if (!light) st.tintHold = now + 2000;
  }
  const k = run(st.tint, now);
  const mix = st.tintFrom.map((v, i) => v + (st.tintTo[i]! - v) * k) as RGB;
  const peak = light ? 255 : Math.max(...mix) || 1;
  const [R, G, B] = mix.map(v => Math.round((255 * v) / peak)) as RGB;
  const [hR, hG, hB] = light ? satBright(R, G, B, 5.355, 0.78) : [R, G, B];
  const haloStroke = `rgb(${hR},${hG},${hB})`;
  if (haloStroke !== a.haloTint) {
    a.haloTint = haloStroke;
    tintSprite(a.haloBase, a.haloTinted, haloStroke);
  }
  let extraStroke = '#ffffff';
  if (light) {
    const [dR, dG, dB] = deepenRgb(R, G, B);
    const [eR, eG, eB] = satBright(dR, dG, dB, 5.355, 0.78);
    extraStroke = `rgb(${eR},${eG},${eB})`;
  }
  if (extraStroke !== a.catchTint) {
    a.catchTint = extraStroke;
    tintSprite(a.catchBase, a.catchTinted, extraStroke);
  }

  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.globalCompositeOperation = 'source-over';
  ctx.globalAlpha = 1;
  ctx.clearRect(0, 0, target.width, target.height);

  const haloAlpha = st.opacity * 0.8;
  const extraAlpha = Math.min(1, st.opacity * 3.51);
  if (haloAlpha <= 0.001 && extraAlpha <= 0.001) return;

  const { pad, dpr } = a;
  if (haloAlpha > 0.001) {
    ctx.save();
    ctx.globalAlpha = haloAlpha;
    ctx.translate((pad + x) * dpr, (pad + y) * dpr);
    ctx.rotate(angle);
    ctx.drawImage(a.haloTinted, -a.haloTinted.width / 2, -a.haloTinted.height / 2);
    ctx.restore();
  }
  if (extraAlpha > 0.001) {
    ctx.save();
    ctx.globalAlpha = extraAlpha;
    ctx.translate((pad + qx) * dpr, (pad + qy) * dpr);
    ctx.rotate(angle);
    ctx.drawImage(a.catchTinted, -a.catchTinted.width / 2, -a.catchTinted.height / 2);
    ctx.restore();
  }
  ctx.globalAlpha = 1;
  ctx.globalCompositeOperation = 'destination-in';
  ctx.drawImage(a.ringMask, 0, 0);
  ctx.globalCompositeOperation = 'source-over';
}

// Light mode's catch-light: the halo colour with saturation x 2.625 and value nudged up (at least 0.31)
function deepenRgb(r: number, g: number, b: number): RGB {
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  let hue = 0;
  if (d) hue = (max === r ? ((g - b) / d + 6) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4) / 6;
  const sat = Math.min(1, (max ? d / max : 0) * 2.625), val = Math.max(0.31, (max / 255) * 1.008);
  const i = Math.floor(hue * 6), f = hue * 6 - i, p = val * (1 - sat), q = val * (1 - f * sat), t = val * (1 - (1 - f) * sat);
  const [x, y, z] = ([[val, t, p], [q, val, p], [p, val, t], [p, q, val], [t, p, val], [val, p, q]] as const)[i % 6]!;
  return [Math.round(x * 255), Math.round(y * 255), Math.round(z * 255)];
}
