import { PRESETS, createInstance, setFrameRate, setInstancePaused, type Instance } from 'img-fx';
import { IMAGE_GEN } from './presets';
import { composite, imageGenColors, type RGB, type RGBA } from './imageGenPalette';

let probe: CanvasRenderingContext2D | null | undefined;
// Any CSS colour → RGBA through the canvas parser: tokens mix hex, rgba() and color-mix()
function parseColor(css: string): RGBA {
  probe ??= document.createElement('canvas').getContext('2d');
  if (!probe) return [0, 0, 0, 0];
  probe.fillStyle = '#000';
  probe.fillStyle = css.trim() || 'transparent';
  const v = String(probe.fillStyle);
  if (v.startsWith('#')) return [parseInt(v.slice(1, 3), 16), parseInt(v.slice(3, 5), 16), parseInt(v.slice(5, 7), 16), 1];
  const [r = 0, g = 0, b = 0, a = 1] = v.match(/[\d.]+/g)?.map(Number) ?? [];
  return [r, g, b, a];
}

// The theme's image generation colours as painted behind `el`: backgrounds up to the first opaque one, --fg-1, the peak token
export function readImageGenColors(el: HTMLElement) {
  const layers: RGBA[] = [];
  for (let n: HTMLElement | null = el; n; n = n.parentElement) {
    const c = parseColor(getComputedStyle(n).backgroundColor);
    if (c[3] > 0) layers.unshift(c);
    if (c[3] >= 1) break;
  }
  const cs = getComputedStyle(el);
  const rgb = (name: string) => parseColor(cs.getPropertyValue(name)).slice(0, 3) as RGB;
  return imageGenColors(composite(layers), rgb('--fg-1'), rgb('--image-gen-peak'));
}

// img-fx shares one WebGL renderer across instances and disposes it with the last one, so every card would pay the
// context creation and the synchronous shader link again. A paused 1px instance keeps the renderer alive once the first
// card has built it; a paused instance is skipped by the render loop and the GL canvas sizing, so it costs no frames.
let keeper: Instance | undefined;
export function keepImageGenRenderer() {
  if (keeper) return;
  setFrameRate(IMAGE_GEN.fps);
  keeper = createInstance({ canvas: document.createElement('canvas'), cssWidth: 1, cssHeight: 1, preset: PRESETS[IMAGE_GEN.preset].modes.dark });
  setInstancePaused(keeper, true);
}

const hash = (x: number, y: number) => { const s = Math.sin(x * 127.1 + y * 311.7) * 43758.5453; return s - Math.floor(s); };
const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);

// Draws `img` into `canvas` (object-fit: cover) cell by cell: cells land in a noise order pulled outwards from the centre,
// each fading in over a band of the timeline while it grows from `grow` to full size. Returns a cancel function.
export function dissolveIn(canvas: HTMLCanvasElement, img: HTMLImageElement, done: () => void): () => void {
  const { ms, cell, band, grow } = IMAGE_GEN.dissolve;
  const ctx = canvas.getContext('2d');
  if (!ctx) { done(); return () => {}; }
  const dpr = Math.min(2, devicePixelRatio || 1);
  const w = canvas.clientWidth, h = canvas.clientHeight;
  canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const s = Math.max(w / img.naturalWidth, h / img.naturalHeight);
  const sx = (img.naturalWidth - w / s) / 2, sy = (img.naturalHeight - h / s) / 2, k = 1 / s;
  const cols = Math.ceil(w / cell), rows = Math.ceil(h / cell);
  const cx = (cols - 1) / 2, cy = (rows - 1) / 2, reach = Math.hypot(cx, cy) || 1;
  const order = new Float32Array(cols * rows);
  for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++)
    order[y * cols + x] = (hash(x * 1.3, y * 2.7) * 0.6 + (Math.hypot(x - cx, y - cy) / reach) * 0.4) * (1 - band);
  let raf = 0;
  const t0 = performance.now();
  const frame = (now: number) => {
    const p = clamp01((now - t0) / ms);
    ctx.clearRect(0, 0, w, h);
    for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++) {
      const a = clamp01((p - order[y * cols + x]!) / band);
      if (a <= 0) continue;
      ctx.globalAlpha = a;
      const dx = x * cell, dy = y * cell, size = cell * (grow + (1 - grow) * a), inset = (cell - size) / 2;
      ctx.drawImage(img, sx + dx * k, sy + dy * k, cell * k, cell * k, dx + inset, dy + inset, size, size);
    }
    ctx.globalAlpha = 1;
    if (p < 1) raf = requestAnimationFrame(frame); else done();
  };
  raf = requestAnimationFrame(frame);
  return () => cancelAnimationFrame(raf);
}
