// The plugin cards' "magic moment" transitions (docs/dev/ui-motion-layout.md): occasional, on a click, so standard
// motion. Both check the motion switch themselves (WAAPI does not hear motion.css)
// - revealSkin: a circle grows from the clicked button and uncovers the new skin. View Transitions keep the old page as
//   a still snapshot while the new one is live underneath; a ring of glyphs rides the circle's edge. Without View
//   Transitions (and on Linux WebKitGTK, where they can crash the UI process) the backdrop layer alone is uncovered by
//   a clip-path circle. With motion off the change is immediate.
// - flyInto: a stand-in from the card flies along an arc onto its slot, lands exactly on it, and the slot glows.

export type Point = { x: number; y: number };

export function motionOff(): boolean {
  return document.querySelector('.acp-shell')?.getAttribute('data-motion') === 'none' || !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
}

// WebKitGTK (the Linux desktop app) can take the UI process down in a view transition (WebKit bugs 321683, 324730)
const webkitGtk = () => /Linux/.test(navigator.userAgent) && /AppleWebKit/.test(navigator.userAgent) && !/Chrome|Chromium/.test(navigator.userAgent);
const vtAllowed = () => 'startViewTransition' in document && !webkitGtk();
// Whether a reveal keeps the old page as a snapshot; without, the caller keeps the old backdrop itself while it runs
export const revealKeepsOld = vtAllowed;

// The circle is seen leaving the button, sweeps the middle and slows into the corners; a front-loaded ease covered the
// window in its first quarter and read as a flash
export const REVEAL_MS = 1300;
const UNDO_MS = 1000;
const FLY_MS = 1100;
const REVEAL_EASE = 'cubic-bezier(.45,.05,.25,1)';
const UNDO_EASE = 'cubic-bezier(.45,0,.55,1)';

let styled = false;
function ensureStyle() {
  if (styled) return;
  styled = true;
  const s = document.createElement('style');
  s.textContent = `
html.skin-vt::view-transition-old(root), html.skin-vt::view-transition-new(root) { animation: none; mix-blend-mode: normal; }
html.skin-vt-back::view-transition-old(root) { z-index: 2; }
html.skin-vt::view-transition-group(skin-ring), html.skin-vt::view-transition-new(skin-ring), html.skin-vt::view-transition-old(skin-ring) { animation: none; }
html.skin-vt::view-transition-group(skin-ring) { z-index: 3; }
.skin-ring { position: fixed; inset: 0; width: 100vw; height: 100vh; pointer-events: none; z-index: 2147483000; view-transition-name: skin-ring; }
.slot-flyer { position: fixed; left: 0; top: 0; pointer-events: none; z-index: 2147483000; transform-origin: 0 0; will-change: transform, opacity; }
.slot-glow { position: fixed; pointer-events: none; z-index: 2147483000; border-radius: 999px; }
`;
  document.head.append(s);
}

const RING_GLYPHS = 'o~.*+°o.~';

// The glyph ring: three bands straddling the circle's edge, drawn only where they are on screen
function drawRing(canvas: HTMLCanvasElement, at: Point, radius: number, colour: string, fade: number, frame: number) {
  const dpr = Math.min(2, devicePixelRatio || 1);
  const w = innerWidth, h = innerHeight;
  if (canvas.width !== Math.round(w * dpr)) { canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr); }
  const c = canvas.getContext('2d')!;
  c.setTransform(dpr, 0, 0, dpr, 0, 0);
  c.clearRect(0, 0, w, h);
  if (radius <= 1 || fade <= 0) return;
  c.font = '13px ui-monospace, "SF Mono", Menlo, monospace';
  c.textAlign = 'center'; c.textBaseline = 'middle';
  c.fillStyle = colour;
  const bands: [number, number][] = [[7, 0.95], [-6, 0.7], [-20, 0.32]];
  for (const [offset, alpha] of bands) {
    const r = radius + offset;
    if (r <= 2) continue;
    const n = Math.max(8, Math.floor((2 * Math.PI * r) / 11));
    for (let i = 0; i < n; i++) {
      const a = (i / n) * 2 * Math.PI;
      const x = at.x + r * Math.cos(a), y = at.y + r * Math.sin(a);
      if (x < -10 || y < -10 || x > w + 10 || y > h + 10) continue;
      // Each glyph flickers on its own beat
      const flick = ((i * 7 + frame + offset) % 9);
      c.globalAlpha = alpha * fade * (flick < 2 ? 0.45 : 1);
      c.fillText(RING_GLYPHS[(i + Math.floor(frame / 3)) % RING_GLYPHS.length]!, x, y);
    }
  }
  c.globalAlpha = 1;
}

const farthest = (at: Point) => Math.hypot(Math.max(at.x, innerWidth - at.x), Math.max(at.y, innerHeight - at.y));

export interface RevealOptions {
  // Uncover (forward) or cover back up toward the button (undo)
  back?: boolean;
  ring: string;
  duration?: number;
  // Fallback only: the layer the clip-path circle uncovers
  backdrop?: () => HTMLElement | null;
}

// Runs `apply` (which must commit the DOM change synchronously, e.g. inside flushSync) under the transition
export async function revealSkin(at: Point, apply: () => void, opts: RevealOptions): Promise<void> {
  ensureStyle();
  const duration = opts.duration ?? (opts.back ? UNDO_MS : REVEAL_MS);
  if (motionOff()) { apply(); return; }
  if (!vtAllowed()) { apply(); await fallbackReveal(at, opts, duration); return; }
  const root = document.documentElement;
  root.classList.add('skin-vt');
  if (opts.back) root.classList.add('skin-vt-back');
  let ring: HTMLCanvasElement | undefined;
  const vt = (document as Document & { startViewTransition(cb: () => void): { ready: Promise<void>; finished: Promise<void>; updateCallbackDone: Promise<void> } }).startViewTransition(() => {
    apply();
    ring = document.createElement('canvas');
    ring.className = 'skin-ring';
    document.body.append(ring);
  });
  try {
    await vt.ready;
  } catch {
    // Skipped (the window hidden, another transition): the update still runs, so the ring it adds is removed after it
    await vt.updateCallbackDone.catch(() => {});
    root.classList.remove('skin-vt', 'skin-vt-back');
    ring?.remove();
    return;
  }
  const max = farthest(at);
  const circle = (r: number) => `circle(${r}px at ${at.x}px ${at.y}px)`;
  const anim = root.animate({ clipPath: opts.back ? [circle(max), circle(0)] : [circle(0), circle(max)] }, {
    duration, easing: opts.back ? UNDO_EASE : REVEAL_EASE, fill: 'forwards',
    pseudoElement: opts.back ? '::view-transition-old(root)' : '::view-transition-new(root)',
  });
  let frame = 0;
  const tick = () => {
    if (!ring) return;
    const p = Number(anim.effect?.getComputedTiming().progress ?? 1);
    const r = (opts.back ? 1 - p : p) * max;
    // Fades over the last fifth, by which time the edge is mostly off screen
    const fade = Math.min(1, (1 - p) * 5, p * 12);
    drawRing(ring, at, r, opts.ring, fade, frame++);
    if (anim.playState === 'running') requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  await anim.finished.catch(() => {});
  await vt.finished.catch(() => {});
  ring?.remove();
  root.classList.remove('skin-vt', 'skin-vt-back');
}

// No View Transitions: the new skin is already applied; uncover its backdrop with a growing circle and ride the ring
// over it. The shell's colours switch at once, the backdrop carries the moment
async function fallbackReveal(at: Point, opts: RevealOptions, duration: number) {
  const layer = opts.backdrop?.();
  const ring = document.createElement('canvas');
  ring.className = 'skin-ring';
  ring.style.viewTransitionName = 'none';
  document.body.append(ring);
  const box = layer?.getBoundingClientRect();
  const local = box ? { x: at.x - box.left, y: at.y - box.top } : at;
  const max = farthest(at);
  const circle = (r: number) => `circle(${r}px at ${local.x}px ${local.y}px)`;
  const anim = layer?.animate({ clipPath: opts.back ? [circle(max), circle(0)] : [circle(0), circle(max)] }, { duration, easing: opts.back ? UNDO_EASE : REVEAL_EASE });
  const start = performance.now();
  let frame = 0;
  await new Promise<void>(done => {
    const tick = () => {
      const lin = Math.min(1, (performance.now() - start) / duration);
      const p = Number(anim?.effect?.getComputedTiming().progress ?? lin);
      drawRing(ring, at, (opts.back ? 1 - p : p) * max, opts.ring, Math.min(1, (1 - lin) * 5, lin * 12), frame++);
      if (lin < 1) requestAnimationFrame(tick); else done();
    };
    requestAnimationFrame(tick);
  });
  ring.remove();
}

// Progress along the arc: an ease that is at the slot by 80 % of the flight and holds still there, so the real piece
// or mark appears under a stand-in that no longer moves. An overshoot (running past the slot and swinging back) read
// as the stand-in missing and correcting itself, and slid it off the piece just as the piece appeared
const settle = (t: number) => { const u = Math.min(1, t / 0.8); return u < 0.5 ? 4 * u ** 3 : 1 - (-2 * u + 2) ** 3 / 2; };

export interface FlyOptions {
  // Built from the source's thumbnail: the element that flies
  flyer: HTMLElement;
  glow: string;
  duration?: number;
  // Both land exactly on the target's box. mark: scales evenly (a mark onto a mark) and the slot answers with a round
  // glow; piece: fits the box an interface piece has reserved, its glow following that box
  fit?: 'mark' | 'piece';
  // Where the stand-in and the glow live: inside the shell they read its theme and skin tokens (default: body)
  host?: HTMLElement;
  // A layer inside the stand-in that thins out as it grows (a piece's fill), so it arrives as a frame, not a slab
  fade?: HTMLElement;
  // Called when the flyer lands: swap the slot's content here
  land?: () => void;
  // The slot went away mid-flight: the stand-in goes, nothing lands, no glow
  signal?: AbortSignal;
}

// Flies `opts.flyer` from `from` into `to` along an arc that bows upward, shrinking to the slot's size, then glows the slot
export async function flyInto(from: DOMRect, to: () => DOMRect | undefined, opts: FlyOptions): Promise<void> {
  ensureStyle();
  const target = to();
  if (!target || motionOff()) { opts.land?.(); return; }
  const duration = opts.duration ?? FLY_MS;
  const piece = opts.fit === 'piece';
  const f = opts.flyer;
  f.classList.add('slot-flyer');
  // Only transform and opacity move, so the flight runs on the compositor and keeps its pace while the page commits the
  // landing underneath. A piece is laid out at the size it lands at and scaled up to it, so its 1 px outline is 1 px
  // where it matters; width / height or border-radius keyframes put the whole flight on the main thread, where the
  // landing's commit made it stutter
  const box = target;
  f.style.width = `${box.width}px`; f.style.height = `${box.height}px`;
  (opts.host ?? document.body).append(f);
  // Centre to centre, the control point a quarter of the distance off the path's middle, on the side that leans up: a
  // flight up-left bows over the content, one straight down swings out sideways (lifting the control point above both
  // ends sent a downward flight up past the top of the window first)
  const a = { x: from.left + from.width / 2, y: from.top + from.height / 2 };
  const b = { x: target.left + target.width / 2, y: target.top + target.height / 2 };
  const dist = Math.hypot(b.x - a.x, b.y - a.y);
  let n = { x: -(b.y - a.y) / (dist || 1), y: (b.x - a.x) / (dist || 1) };
  if (n.y > 0 || (n.y === 0 && n.x > 0)) n = { x: -n.x, y: -n.y };
  const ctl = { x: (a.x + b.x) / 2 + n.x * dist * 0.25, y: (a.y + b.y) / 2 + n.y * dist * 0.25 };
  const even = target.width / from.width;
  const end = piece ? { x: target.width / from.width, y: target.height / from.height } : { x: even, y: even };
  const frames: Keyframe[] = [];
  const N = 36;
  for (let i = 0; i <= N; i++) {
    const t = i / N;
    const uc = settle(t);
    const x = (1 - uc) * (1 - uc) * a.x + 2 * (1 - uc) * uc * ctl.x + uc * uc * b.x;
    const y = (1 - uc) * (1 - uc) * a.y + 2 * (1 - uc) * uc * ctl.y + uc * uc * b.y;
    const k = 1 - Math.pow(1 - Math.min(1, t / 0.7), 2);
    const sx = 1 + (end.x - 1) * k, sy = 1 + (end.y - 1) * k;
    const w = from.width * sx, h = from.height * sy;
    const opacity = t > 0.86 ? String(1 - (t - 0.86) / 0.14) : '1';
    frames.push({ offset: t, transform: `translate(${x - w / 2}px, ${y - h / 2}px) scale(${w / box.width}, ${h / box.height})`, opacity });
  }
  const anim = f.animate(frames, { duration, easing: 'linear', fill: 'forwards' });
  // A piece's fill thins out in an effect of its own, so a main-thread hitch only delays that
  opts.fade?.animate([{ opacity: 1 }, { opacity: 0.2 }], { duration: duration * 0.8, easing: 'ease-in', fill: 'forwards' });
  // Land a little before the stand-in has faded, so the real mark is already there as it dissolves
  const landing = setTimeout(() => { opts.land?.(); glowAt(to() ?? target, opts.glow, piece, opts.host); }, duration * 0.82);
  opts.signal?.addEventListener('abort', () => { clearTimeout(landing); anim.cancel(); }, { once: true });
  await anim.finished.catch(() => {});
  f.remove();
}

// The slot answers the landing: a ring pulses out from it and fades. Around a piece the ring follows its box and grows a
// few pixels out, where a circle as wide as a sidebar module would sweep half the window
function glowAt(r: DOMRect, colour: string, box = false, host: HTMLElement = document.body) {
  const g = document.createElement('div');
  g.className = 'slot-glow';
  if (box) {
    Object.assign(g.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`, borderRadius: '8px', boxShadow: `0 0 0 1.5px ${colour}, 0 0 16px 1px ${colour}` });
    host.append(g);
    const grow = (px: number) => `scale(${1 + (2 * px) / r.width}, ${1 + (2 * px) / r.height})`;
    g.animate([{ transform: grow(0), opacity: 1 }, { transform: grow(9), opacity: 0 }], { duration: 900, easing: 'cubic-bezier(.2,.7,.2,1)' }).finished.then(() => g.remove(), () => g.remove());
    return;
  }
  const size = Math.max(r.width, r.height);
  Object.assign(g.style, { left: `${r.left + r.width / 2 - size / 2}px`, top: `${r.top + r.height / 2 - size / 2}px`, width: `${size}px`, height: `${size}px`, boxShadow: `0 0 0 2px ${colour}, 0 0 18px 4px ${colour}` });
  host.append(g);
  g.animate([{ transform: 'scale(.9)', opacity: 1 }, { transform: 'scale(2.6)', opacity: 0 }], { duration: 760, easing: 'cubic-bezier(.2,.7,.2,1)' }).finished.then(() => g.remove(), () => g.remove());
}
