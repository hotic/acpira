import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { cn } from '../ui/cn';
import { BASE_H, BASE_W, clock, createRing, destroyRing, sample, updateRing, type MetalTheme, type Ring, type RingGeometry } from './metalPaint';

export interface MetalRingProps {
  theme?: MetalTheme;
  // Freeze on the current frame (the ring stays painted)
  paused?: boolean;
  // Multiplier for every pixel constant, for a magnified copy (the LAB); 1 in the app
  scale?: number;
  className?: string;
  children: ReactNode;
}

// A silver ring around a single child (the send / stop button), plus a soft halo and catch-light that drift along the
// edge toward the ring's brightest spot. The child stays on screen at all times: the ring is painted synchronously on
// mount and needs no WebGL. The wrapper takes the child's corner radius; give it the fill colour, the child keeps none.
export function MetalRing({ theme = 'dark', paused = false, scale = 1, className, children }: MetalRingProps) {
  const root = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const ring = useRef<Ring | null>(null);
  const glow = useRef<GlowNodes>({});
  const [geo, setGeo] = useState<RingGeometry | null>(null);
  const id = useId().replace(/:/g, '');
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
    inst.onFrame = now => stepGlow(inst, state, glow.current, now, live.current.theme);
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
      ring.current = null;
    };
  }, []);

  useEffect(() => { if (ring.current) updateRing(ring.current, { paused }); }, [paused]);
  useEffect(() => { if (ring.current) updateRing(ring.current, { theme, scale }); }, [theme, scale]);

  // data-theme re-scopes the theme tokens under the ring, as metal-fx's wrapper did
  return (
    <div ref={root} data-theme={theme} className={cn('relative isolate inline-flex', className)} style={geo ? { borderRadius: geo.r } : undefined}>
      <canvas ref={canvas} aria-hidden="true" className="pointer-events-none absolute inset-0 block size-full" />
      {geo && <GlowSvg id={id} geo={geo} scale={scale} theme={theme} nodes={glow.current} />}
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
const SVG_STYLE: Record<MetalTheme, CSSProperties> = {
  dark: { inset: 0, width: '100%', height: '100%', opacity: 0.7 },
  // Light mode multiplies a darkened, saturated copy and overscans by 1 px
  light: { inset: -1, width: 'calc(100% + 2px)', height: 'calc(100% + 2px)', opacity: 0.2746, mixBlendMode: 'multiply', filter: 'saturate(5.355) brightness(0.78)' },
};
const MOVE: CSSProperties = { transformOrigin: '0 0', willChange: 'transform', transition: 'transform 100ms linear' };
const FADE: CSSProperties = { willChange: 'opacity', transition: 'opacity 100ms linear' };

interface GlowNodes { halo?: SVGGElement | null; haloInner?: SVGGElement | null; extra?: SVGGElement | null; extraInner?: SVGGElement | null; fade?: SVGCircleElement | null }

function GlowSvg({ id, geo, scale, theme, nodes }: { id: string; geo: RingGeometry; scale: number; theme: MetalTheme; nodes: GlowNodes }) {
  const { w, h, r } = geo;
  const s = perimeter(w, h, r) / REF_PERIM;
  const line = (half: number) => `M ${-half} 0 L ${half} 0`;
  const halo = line(Math.max(1, 7.8 * s)), streak = line(Math.max(0.6, (9.13952 / 3) * s));
  const box = { x: -200 * scale, y: -200 * scale, width: 540 * scale, height: 440 * scale };
  const filter = (key: string, blur: number) => (
    <filter key={key} id={`${id}${key}`} {...box} filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB"><feGaussianBlur stdDeviation={blur * scale} /></filter>
  );
  const stroke = (key: string, d: string, [width, , opacity]: readonly [number, number, number]) => (
    <path key={key} d={d} strokeWidth={width * scale} strokeLinecap="round" strokeLinejoin="round" fill="none" opacity={opacity} filter={`url(#${id}${key})`} />
  );
  return (
    <svg aria-hidden="true" className="pointer-events-none absolute overflow-visible" style={SVG_STYLE[theme]} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      <defs>
        {HALO.map((v, i) => filter(`h${i}`, v[1]))}
        {CATCH.map((v, i) => filter(`c${i}`, v[1]))}
        <radialGradient id={`${id}fg`} cx="0.5" cy="0.5" r="0.5">
          <stop offset="0" stopColor="white" /><stop offset="0.3" stopColor="white" /><stop offset="0.65" stopColor="#404040" /><stop offset="1" stopColor="black" />
        </radialGradient>
        <mask id={`${id}fm`} maskUnits="userSpaceOnUse" {...box}>
          <rect {...box} fill="black" />
          <circle ref={el => { nodes.fade = el; }} cx="0" cy="0" r={FADE_R * scale} fill={`url(#${id}fg)`} style={{ willChange: 'transform' }} />
        </mask>
        {/* Full on the 1 px ring, half outside, none inside */}
        <mask id={`${id}rm`} maskUnits="userSpaceOnUse" {...box}>
          <rect {...box} fill="#808080" />
          <rect x="0" y="0" width={w} height={h} rx={r} ry={r} fill="white" />
          <rect x="1" y="1" width={w - 2} height={h - 2} rx={Math.max(0, r - 1)} ry={Math.max(0, r - 1)} fill="black" />
        </mask>
      </defs>
      {/* The unfilled rects stretch each group's box to the mask region (Safari clips a mask to the element's box) */}
      <g ref={el => { nodes.halo = el; }} mask={`url(#${id}rm)`} opacity="0" style={FADE}>
        <rect {...box} fill="none" />
        <g ref={el => { nodes.haloInner = el; }} stroke="white" style={MOVE}>{HALO.map((v, i) => stroke(`h${i}`, halo, v))}</g>
      </g>
      <g ref={el => { nodes.extra = el; }} mask={`url(#${id}rm)`} opacity="0" style={FADE}>
        <rect {...box} fill="none" />
        <g mask={`url(#${id}fm)`}>
          <g ref={el => { nodes.extraInner = el; }} stroke="white" style={MOVE}>{CATCH.map((v, i) => stroke(`c${i}`, streak, v))}</g>
        </g>
      </g>
    </svg>
  );
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
    haloStroke: '', extraStroke: '',
  };
}
type GlowState = ReturnType<typeof newGlowState>;

const lum = (p: ArrayLike<number>) => (0.2126 * p[0]! + 0.7152 * p[1]! + 0.0722 * p[2]!) / 255;
const glowLevel = (l: number) => 0.34 + 0.51 * smooth(0.08, 0.32, l);

function stepGlow(ring: Ring, st: GlowState, n: GlowNodes, now: number, theme: MetalTheme) {
  if (!n.haloInner || !n.extraInner || !n.halo || !n.extra || !n.fade) return;
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
  const target = glowLevel(cur);
  if (!st.reloc || st.reloc.done) {
    const restart = (idx: number, level: number) => {
      st.idx = idx; st.appeared = now; st.wander = st.wanderTo = st.wanderFrames = 0;
      st.reloc = tween(0, level, 1500, now, true);
    };
    if (st.appeared === 0) restart(bestIdx, target);
    else if (st.reloc?.done && st.reloc.to === 0) restart(st.next, glowLevel(st.lums[st.next]!));
    else if (!young && best - cur > 0.05) { st.next = bestIdx; st.reloc = tween(st.opacity, 0, 1500, now, true); }
    else st.opacity += (target - st.opacity) * 0.00875;
  }
  if (st.reloc) st.opacity = run(st.reloc, now);
  st.opacity = Math.max(0, Math.min(1, st.opacity));
  if (st.wanderFrames++ >= 120) { st.wanderTo = (Math.random() * 2 - 1) * 15 * S; st.wanderFrames = 0; }
  st.wander += (st.wanderTo - st.wander) * 0.0075;

  const s = st.arcs[st.idx]! + st.wander, inset = INSET * scale;
  const angle = tangent(s, geo, inset).toFixed(4);
  const p = edgePoint(s, geo, inset, 0), x = p.x, y = p.y;
  n.haloInner.style.transform = `translate(${x.toFixed(3)}px,${y.toFixed(3)}px) rotate(${angle}rad)`;
  const q = edgePoint(s, geo, inset, S * scale);
  n.extraInner.style.transform = `translate(${q.x.toFixed(3)}px,${q.y.toFixed(3)}px) rotate(${angle}rad)`;
  n.fade.style.transform = `translate(${q.x.toFixed(3)}px,${q.y.toFixed(3)}px)`;

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
  const haloStroke = `rgb(${R},${G},${B})`;
  if (haloStroke !== st.haloStroke) { st.haloStroke = haloStroke; n.haloInner.style.stroke = haloStroke; }
  const extraStroke = light ? deepen(R, G, B) : '#ffffff';
  if (extraStroke !== st.extraStroke) { st.extraStroke = extraStroke; n.extraInner.style.stroke = extraStroke; }
  n.halo.style.opacity = (st.opacity * 0.8).toFixed(3);
  n.extra.style.opacity = Math.min(1, st.opacity * 3.51).toFixed(3);
}

// Light mode's catch-light: the halo colour with saturation x 2.625 and value nudged up (at least 0.31)
function deepen(r: number, g: number, b: number) {
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  let hue = 0;
  if (d) hue = (max === r ? ((g - b) / d + 6) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4) / 6;
  const sat = Math.min(1, (max ? d / max : 0) * 2.625), val = Math.max(0.31, (max / 255) * 1.008);
  const i = Math.floor(hue * 6), f = hue * 6 - i, p = val * (1 - sat), q = val * (1 - f * sat), t = val * (1 - (1 - f) * sat);
  const [x, y, z] = ([[val, t, p], [q, val, p], [p, val, t], [p, q, val], [t, p, val], [val, p, q]] as const)[i % 6]!;
  return `rgb(${Math.round(x * 255)},${Math.round(y * 255)},${Math.round(z * 255)})`;
}
