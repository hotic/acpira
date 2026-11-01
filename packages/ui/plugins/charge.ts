import { motionOff, pressIn } from '../effects/magic';

// A plugin card's apply press (PluginCard.tsx): a faint ripple spreads inside the button from the pressed point, and the
// thumbnail's outline lights up and breathes while the engine enables the version. What the plugin brings then leaves
// from that lit thumbnail (landings.ts, skin.ts). Drawn in the landing colour (--land-ink, skin.css). Returns a stop that
// fades the light out: called when the flight starts, when the card settles otherwise, or after HOLD_MS.
const RIPPLE_MS = 650;
const HOLD_MS = 12_000;
const BREATHE_MS = 1100;
// The thumbnail lights as the ripple spreads
const LIGHT_AFTER = 200;

export function chargeCard(card: HTMLElement, button: HTMLElement, thumb: HTMLElement, at?: { x: number; y: number }): () => void {
  if (motionOff()) return () => {};
  const ink = getComputedStyle(card).getPropertyValue('--land-ink').trim() || 'currentColor';
  const box = card.getBoundingClientRect();
  const b = button.getBoundingClientRect();
  const t = thumb.getBoundingClientRect();
  const layer = (style: Partial<CSSStyleDeclaration>, parent: HTMLElement = card) => {
    const el = document.createElement('div');
    Object.assign(el.style, { position: 'absolute', pointerEvents: 'none', zIndex: '4', ...style });
    parent.append(el);
    return el;
  };

  // The ripple, kept faint: a soft light, not a flash
  pressIn(button, 0.04);
  const p = at ?? { x: b.left + b.width / 2, y: b.top + b.height / 2 };
  const d = Math.hypot(b.width, b.height) * 2;
  const clip = layer({ inset: '0', borderRadius: 'inherit', overflow: 'hidden' }, button);
  const wave = layer({
    left: `${p.x - b.left - d / 2}px`, top: `${p.y - b.top - d / 2}px`, width: `${d}px`, height: `${d}px`, borderRadius: '50%',
    background: `radial-gradient(circle, color-mix(in srgb, ${ink} 32%, transparent) 0%, color-mix(in srgb, ${ink} 14%, transparent) 45%, transparent 70%)`,
  }, clip);
  wave.animate([{ transform: 'scale(0)', opacity: 1 }, { transform: 'scale(1)', opacity: 0 }], { duration: RIPPLE_MS, easing: 'cubic-bezier(.2,.7,.2,1)' })
    .finished.then(() => clip.remove(), () => clip.remove());

  // The thumbnail's outline, lit just after the press, then breathing until stopped
  const ring = layer({
    left: `${t.left - box.left}px`, top: `${t.top - box.top}px`, width: `${t.width}px`, height: `${t.height}px`,
    borderRadius: getComputedStyle(thumb).borderRadius, opacity: '0',
    boxShadow: `0 0 0 1px ${ink}, 0 0 14px -2px ${ink}, inset 0 0 18px -6px ${ink}`,
  });
  const lit = ring.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 260, delay: LIGHT_AFTER, easing: 'ease-out', fill: 'forwards' });
  let breathe: Animation | undefined;
  lit.finished.then(() => {
    breathe = ring.animate([{ opacity: 1 }, { opacity: 0.45 }], { duration: BREATHE_MS, direction: 'alternate', iterations: Infinity, easing: 'ease-in-out' });
  }, () => {});

  let stopped = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    clearTimeout(hold);
    const now = Number(getComputedStyle(ring).opacity);
    lit.cancel();
    breathe?.cancel();
    ring.animate([{ opacity: now }, { opacity: 0 }], { duration: 240, easing: 'ease-out', fill: 'forwards' }).finished.then(() => ring.remove(), () => ring.remove());
  };
  const hold = setTimeout(stop, HOLD_MS);
  return stop;
}
