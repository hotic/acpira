import { IMAGE_GEN } from './presets';

// The image generation card's pixel mosaic: a drifting ridged-noise field quantised into square cells with hairline
// gaps, brighter cells growing over their gaps. Same look as img-fx's `pixels-organic` preset (its "Chromium Flow"
// effect in pixel mode) without three.js.
// Credit: ported from img-fx 0.5.1 by Jakub Antalik (https://image.jakubantalik.com), MIT License, Copyright (c) 2026 Jakub Antalik.
// Full notice in THIRD_PARTY_NOTICES.md.
//
// One WebGL 2 context serves every card (contexts are capped per page, and each would link the shader again). A frame
// is two draws per card: the noise field once per cell into a small texture, then a cheap per-pixel pass for gaps,
// highlight, vignette and edge fade. The result is copied into the card's own 2D canvas, as img-fx does. The context
// stays alive after the last card goes, so a later card pays neither the context nor the shader link again.

export type MosaicTheme = 'dark' | 'light';

export interface MosaicLook {
  theme: MosaicTheme;
  /** Opaque surface under the mosaic (#rrggbb); cells close to it in luminance fade out. */
  cardBg: string;
  /** img-fx's palette slots (#rrggbb); this effect blends the first five. */
  colors: readonly string[];
}

// Per-theme tunings of the preset (img-fx 0.5.1, pixels-organic dark / light)
const MODES = {
  dark: { direction: 0, intensity: 1, highlight: 0.2, vignette: 0.26, fill: 0.44, edgeFade: 24 },
  light: { direction: 25, intensity: 0.85, highlight: 0.7, vignette: 0, fill: 0.18, edgeFade: 20 },
} as const;

// Cells across a 320 CSS px edge; cell size stays constant in CSS px whatever the card size
const CELL_SIZE = 0.22;
const REF_DIM = 320;
const BASE_CELLS = (6 + CELL_SIZE * 74) / IMAGE_GEN.pixelScale;
const SPEED = 0.3;
// GL renders at most at 1.25x and is upscaled without smoothing into the card canvas (at most 2x). The gap width is
// quantised to these GL pixels, which is part of the look, and it keeps the fill cost low.
const GL_DPR = 1.25;
const CANVAS_DPR = 2;
// Rendering pauses this far outside the viewport
export const VISIBLE_MARGIN = '64px';

const VERT = `#version 300 es
in vec2 a_pos;
void main() { gl_Position = vec4(a_pos, 0.0, 1.0); }`;

// The effect math follows img-fx 0.5.1 (MIT, Copyright (c) 2026 Jakub Antalik): effect 22, blur 1 (five taps),
// cell-centre highlight. snoise is the 2D simplex noise of webgl-noise (MIT, Copyright (C) 2011 Ashima Arts,
// Stefan Gustavson, Ian McEwan).
const NOISE = `
vec3 mod289(vec3 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec2 mod289(vec2 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec3 permute(vec3 x) { return mod289((x * 34.0 + 1.0) * x); }
float snoise(vec2 v) {
  const vec4 C = vec4(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
  vec2 i = floor(v + dot(v, C.yy));
  vec2 x0 = v - i + dot(i, C.xx);
  vec2 i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
  vec4 x12 = x0.xyxy + C.xxzz;
  x12.xy -= i1;
  i = mod289(i);
  vec3 p = permute(permute(i.y + vec3(0.0, i1.y, 1.0)) + i.x + vec3(0.0, i1.x, 1.0));
  vec3 m = max(0.5 - vec3(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), 0.0);
  m = m * m; m = m * m;
  vec3 x = 2.0 * fract(p * C.www) - 1.0;
  vec3 h = abs(x) - 0.5;
  vec3 a0 = x - floor(x + 0.5);
  m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h);
  vec3 g;
  g.x = a0.x * x0.x + h.x * x0.y;
  g.yz = a0.yz * x12.xz + h.yz * x12.yw;
  return 130.0 * dot(m, g);
}`;

// Pass 1: one fragment per cell. rgb = field colour at the cell centre, a = highlight factor
const CELL_FRAG = `#version 300 es
precision highp float;
uniform vec2 u_grid;
uniform float u_aspect, u_time, u_intensity;
uniform vec2 u_dir;
uniform vec3 u_c1, u_c2, u_c3, u_c4, u_c5;
out vec4 o;
${NOISE}
// Two octaves (complexity 0.2)
float fbm(vec2 p) { return 0.5 * snoise(p) + 0.25 * snoise(p * 2.0); }
// distortion 0.3 -> strength 0.6
vec2 warp(vec2 p, float t) { return vec2(fbm(p + vec2(t * 0.1, 0.0)), fbm(p + vec2(0.0, t * 0.12) + 5.0)) * 0.6; }
vec3 blend(float a, float b, float c) {
  a = clamp(a, 0.0, 1.0); a *= a;
  b = clamp(b, 0.0, 1.0); b *= b;
  c = clamp(c, 0.0, 1.0); c *= c;
  float d = clamp(a * 0.7 + c * 0.3, 0.0, 1.0); d *= d;
  float e = clamp(b * 0.5 + c * 0.5, 0.0, 1.0); e *= e;
  float total = a + b + c + d + e;
  float floorW = max(0.001 - total, 0.0);
  vec3 fallback = (u_c1 + u_c2 + u_c3 + u_c4 + u_c5) * 0.2;
  return (u_c1 * a + u_c2 * b + u_c3 * c + u_c4 * d + u_c5 * e + fallback * floorW) / (total + floorW);
}
// shape 0.52, distortion 0.3, complexity 0.2 folded into the constants
vec3 field(vec2 uv, float t) {
  vec2 p = uv - 0.5;
  p.x *= u_aspect;
  p += u_dir * t * 0.15;
  vec2 w = warp(p * 0.7, t * 0.5);
  vec2 w2 = warp(p * 0.4 + w * 0.3, t * 0.3);
  vec2 wp = p + w * 0.58;
  float n1 = snoise(wp * 1.72 + t * 0.14);
  float n2 = snoise((wp + w2 * 0.12) * 2.4 + vec2(3.0, 7.0) - t * 0.1);
  float r1 = pow(max(1.0 - abs(n1), 0.0), 11.24);
  float r2 = pow(max(1.0 - abs(n2), 0.0), 9.2);
  float base = (n1 + n2) * 0.25 + 0.5;
  return blend((base * 0.6 + r1 * 1.2) * u_intensity, ((1.0 - base) * 0.6 + r2) * u_intensity, (r1 * 0.8 + r2 * 0.6) * u_intensity);
}
void main() {
  vec2 uv = floor(gl_FragCoord.xy) / u_grid + 0.5 / u_grid;
  float t = u_time;
  const float r = 0.02;
  vec3 col = field(uv, t) * 0.4
    + (field(uv + vec2(r, 0.0), t) + field(uv - vec2(r, 0.0), t) + field(uv + vec2(0.0, r), t) + field(uv - vec2(0.0, r), t)) * 0.15;
  vec2 cp = uv - 0.5;
  cp.x *= u_aspect;
  float lw = (sin(cp.x * 3.0 + t * 1.5) * 0.5 + 0.5) * (sin(cp.y * 2.5 - t * 1.1) * 0.5 + 0.5);
  lw += (snoise(cp * 2.0 + t * 0.6) * 0.5 + 0.5) * 0.3;
  float hl = clamp(lw, 0.0, 1.0);
  o = vec4(col, hl * hl);
}`;

// Pass 2: per pixel. gap 0.14, hlScale 0.8, dot opacity 0.68, fade strength 1
const PIXEL_FRAG = `#version 300 es
precision highp float;
uniform sampler2D u_cells;
uniform vec2 u_grid, u_res;
uniform float u_dpr, u_highlight, u_vignette, u_fill, u_edgeFade, u_bgLum;
out vec4 o;
void main() {
  vec2 uv = gl_FragCoord.xy / u_res;
  vec2 cellPos = uv * u_grid;
  vec4 cell = texelFetch(u_cells, ivec2(cellPos), 0);
  vec3 base = cell.rgb;
  vec3 col = base;
  vec2 css = u_res / u_dpr;
  vec2 c = uv * css;
  float edge = min(min(c.x, css.x - c.x), min(c.y, css.y - c.y));
  float range = 40.0 * (1.0 + u_vignette * 3.0);
  col *= mix(1.0, smoothstep(0.0, 1.0, edge * edge / (range * range)), u_vignette);
  float boost = 1.0 + smoothstep(0.2, 0.8, cell.a) * 0.96;
  float gap = 0.049 / boost;
  vec2 l = fract(cellPos);
  float mask = step(gap, l.x) * step(gap, 1.0 - l.x) * step(gap, l.y) * step(gap, 1.0 - l.y);
  float h = cell.a * u_highlight;
  col = col * (1.0 + h * 2.5) + vec3(h * h * 0.3);
  mask *= smoothstep(0.0, u_edgeFade, edge);
  float a = mix(u_fill, 0.68, mask) * smoothstep(0.0, 0.33, abs(dot(base, vec3(0.299, 0.587, 0.114)) - u_bgLum));
  // Stored as straight alpha with the colour already scaled by alpha, as img-fx's blend leaves it: the card sees
  // col * a * a over the surface, which keeps the dim cells dim
  o = vec4(min(col, 1.0) * a, a);
}`;

interface Card {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  w: number;
  h: number;
  look: MosaicLook;
  visible: boolean;
  paused: boolean;
  time: number;
}

interface Programs {
  cell: WebGLProgram;
  pixel: WebGLProgram;
  vao: WebGLVertexArrayObject;
  tex: WebGLTexture;
  fbo: WebGLFramebuffer;
  texW: number;
  texH: number;
  cellU: Record<string, WebGLUniformLocation | null>;
  pixelU: Record<string, WebGLUniformLocation | null>;
}

interface Renderer {
  canvas: HTMLCanvasElement;
  gl: WebGL2RenderingContext;
  progs: Programs | null;
  lost: boolean;
  cards: Set<Card>;
  raf: number;
  lastFrame: number;
  lastTick: number;
}

let shared: Renderer | null | undefined;

function compile(gl: WebGL2RenderingContext, vert: string, frag: string): WebGLProgram {
  const prog = gl.createProgram()!;
  for (const [type, src] of [[gl.VERTEX_SHADER, vert], [gl.FRAGMENT_SHADER, frag]] as const) {
    const sh = gl.createShader(type)!;
    gl.shaderSource(sh, src);
    gl.compileShader(sh);
    gl.attachShader(prog, sh);
    gl.deleteShader(sh);
  }
  gl.bindAttribLocation(prog, 0, 'a_pos');
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS) && !gl.isContextLost()) throw new Error(`pixelMosaic: ${gl.getProgramInfoLog(prog)}`);
  return prog;
}

const locations = (gl: WebGL2RenderingContext, prog: WebGLProgram, names: string[]) =>
  Object.fromEntries(names.map(n => [n, gl.getUniformLocation(prog, n)]));

function build(gl: WebGL2RenderingContext): Programs {
  const cell = compile(gl, VERT, CELL_FRAG);
  const pixel = compile(gl, VERT, PIXEL_FRAG);
  const vao = gl.createVertexArray()!;
  gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  // One triangle covering the viewport
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  const tex = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  const fbo = gl.createFramebuffer()!;
  gl.disable(gl.BLEND);
  gl.disable(gl.DEPTH_TEST);
  return {
    cell, pixel, vao, tex, fbo, texW: 0, texH: 0,
    cellU: locations(gl, cell, ['u_grid', 'u_aspect', 'u_time', 'u_intensity', 'u_dir', 'u_c1', 'u_c2', 'u_c3', 'u_c4', 'u_c5']),
    pixelU: locations(gl, pixel, ['u_cells', 'u_grid', 'u_res', 'u_dpr', 'u_highlight', 'u_vignette', 'u_fill', 'u_edgeFade', 'u_bgLum']),
  };
}

function renderer(): Renderer | null {
  if (shared !== undefined) return shared;
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 8;
  const gl = canvas.getContext('webgl2', { alpha: true, premultipliedAlpha: false, antialias: false, depth: false, stencil: false, preserveDrawingBuffer: false });
  if (!gl) return (shared = null);
  const r: Renderer = { canvas, gl, progs: null, lost: false, cards: new Set(), raf: 0, lastFrame: 0, lastTick: 0 };
  try { r.progs = build(gl); } catch (e) { console.error(e); return (shared = null); }
  canvas.addEventListener('webglcontextlost', e => { e.preventDefault(); r.lost = true; r.progs = null; cancelAnimationFrame(r.raf); r.raf = 0; });
  canvas.addEventListener('webglcontextrestored', () => {
    r.lost = false;
    try { r.progs = build(gl); } catch { return; }
    for (const c of r.cards) draw(r, c);
    wake();
  });
  return (shared = r);
}

/** Creates the shared context and links the shaders now (e.g. on an idle slot); false without WebGL 2. */
export function warmPixelMosaic(): boolean {
  return !!renderer();
}

const hexRgb = (hex: string): [number, number, number] => {
  const v = parseInt(hex.replace('#', '').slice(0, 6), 16) || 0;
  return [(v >> 16 & 255) / 255, (v >> 8 & 255) / 255, (v & 255) / 255];
};
const glDpr = () => Math.min(devicePixelRatio || 1, GL_DPR);
const cells = (px: number, dpr: number) => Math.max(2, Math.floor(BASE_CELLS * (px / dpr) / REF_DIM));

function draw(r: Renderer, card: Card) {
  const { gl, progs } = r;
  if (r.lost || !progs || card.w < 1 || card.h < 1) return;
  const dpr = glDpr();
  const iw = Math.max(1, Math.floor(card.w * dpr)), ih = Math.max(1, Math.floor(card.h * dpr));
  const gx = cells(iw, dpr), gy = cells(ih, dpr);
  const mode = MODES[card.look.theme];
  // The GL canvas only grows, so cards of different sizes do not resize it back and forth
  if (iw > r.canvas.width || ih > r.canvas.height) {
    r.canvas.width = Math.max(iw, r.canvas.width);
    r.canvas.height = Math.max(ih, r.canvas.height);
  }
  if (gx > progs.texW || gy > progs.texH) {
    progs.texW = Math.max(gx, progs.texW);
    progs.texH = Math.max(gy, progs.texH);
    gl.bindTexture(gl.TEXTURE_2D, progs.tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, progs.texW, progs.texH, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.bindFramebuffer(gl.FRAMEBUFFER, progs.fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, progs.tex, 0);
  }
  gl.bindVertexArray(progs.vao);

  // Pass 1: the field per cell
  gl.bindFramebuffer(gl.FRAMEBUFFER, progs.fbo);
  gl.viewport(0, 0, gx, gy);
  gl.useProgram(progs.cell);
  const cu = progs.cellU, rad = mode.direction * Math.PI / 180;
  gl.uniform2f(cu.u_grid!, gx, gy);
  gl.uniform1f(cu.u_aspect!, iw / ih);
  gl.uniform1f(cu.u_time!, card.time * SPEED);
  gl.uniform1f(cu.u_intensity!, mode.intensity);
  gl.uniform2f(cu.u_dir!, Math.cos(rad), Math.sin(rad));
  for (let i = 0; i < 5; i++) gl.uniform3fv(cu[`u_c${i + 1}`]!, hexRgb(card.look.colors[i] ?? card.look.cardBg));
  gl.drawArrays(gl.TRIANGLES, 0, 3);

  // Pass 2: gaps, highlight, vignette, edge fade, into the bottom-left corner of the GL canvas
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.viewport(0, 0, iw, ih);
  gl.useProgram(progs.pixel);
  const pu = progs.pixelU, [br, bg, bb] = hexRgb(card.look.cardBg);
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, progs.tex);
  gl.uniform1i(pu.u_cells!, 0);
  gl.uniform2f(pu.u_grid!, gx, gy);
  gl.uniform2f(pu.u_res!, iw, ih);
  gl.uniform1f(pu.u_dpr!, dpr);
  gl.uniform1f(pu.u_highlight!, mode.highlight);
  gl.uniform1f(pu.u_vignette!, mode.vignette);
  gl.uniform1f(pu.u_fill!, mode.fill);
  gl.uniform1f(pu.u_edgeFade!, mode.edgeFade);
  gl.uniform1f(pu.u_bgLum!, 0.299 * br + 0.587 * bg + 0.114 * bb);
  gl.drawArrays(gl.TRIANGLES, 0, 3);

  // Copy into the card, nearest-neighbour so the cells stay crisp
  const cdpr = Math.min(devicePixelRatio || 1, CANVAS_DPR);
  const cw = Math.max(1, Math.floor(card.w * cdpr)), ch = Math.max(1, Math.floor(card.h * cdpr));
  if (card.canvas.width !== cw || card.canvas.height !== ch) { card.canvas.width = cw; card.canvas.height = ch; }
  card.ctx.clearRect(0, 0, cw, ch);
  card.ctx.imageSmoothingEnabled = false;
  card.ctx.drawImage(r.canvas, 0, r.canvas.height - ih, iw, ih, 0, 0, cw, ch);
}

const live = (c: Card) => c.visible && !c.paused;

const tick = (now: number) => {
  const r = shared;
  if (!r || r.lost || ![...r.cards].some(live)) { if (r) r.raf = 0; return; }
  r.raf = requestAnimationFrame(tick);
  const interval = 1000 / IMAGE_GEN.fps, since = now - r.lastFrame;
  if (since < interval) return;
  r.lastFrame = now - since % interval;
  const dt = (now - r.lastTick) / 1000;
  r.lastTick = now;
  for (const c of r.cards) if (live(c)) { c.time += dt; draw(r, c); }
};

function wake() {
  const r = shared;
  if (!r || r.lost || r.raf) return;
  r.lastTick = r.lastFrame = performance.now();
  r.raf = requestAnimationFrame(tick);
}

export interface MosaicHandle {
  setSize(w: number, h: number): void;
  setLook(look: MosaicLook): void;
  setVisible(visible: boolean): void;
  setPaused(paused: boolean): void;
  destroy(): void;
}

/** Starts a mosaic on `canvas` (CSS size w x h); null without WebGL 2. A paused or off-screen card keeps its last frame. */
export function createMosaic(canvas: HTMLCanvasElement, w: number, h: number, look: MosaicLook): MosaicHandle | null {
  const r = renderer();
  const ctx = canvas.getContext('2d');
  if (!r || !ctx) return null;
  // A random start in the field, so cards side by side differ
  const card: Card = { canvas, ctx, w, h, look, visible: true, paused: false, time: Math.random() * 1000 };
  r.cards.add(card);
  draw(r, card);
  wake();
  const redraw = () => { if (card.visible) draw(r, card); };
  return {
    setSize(nw, nh) { if (nw !== card.w || nh !== card.h) { card.w = nw; card.h = nh; redraw(); } },
    setLook(next) { card.look = next; redraw(); },
    setVisible(v) { card.visible = v; if (v) wake(); },
    setPaused(p) { card.paused = p; if (!p) wake(); },
    destroy() { r.cards.delete(card); },
  };
}
