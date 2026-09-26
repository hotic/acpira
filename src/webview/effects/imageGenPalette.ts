import { IMAGE_GEN } from './presets';

// Pure colour math for the image generation card (no DOM, so the node test suites import it directly)
export type RGB = [number, number, number];
export type RGBA = [number, number, number, number];

// Straight alpha compositing, bottom layer first: the opaque colour a stack of (partly translucent) backgrounds paints
export function composite(layers: RGBA[]): RGB {
  let out: RGB = [0, 0, 0];
  for (const [r, g, b, a] of layers) out = [out[0] + (r - out[0]) * a, out[1] + (g - out[1]) * a, out[2] + (b - out[2]) * a];
  return out;
}

const luminance = ([r, g, b]: RGB) => (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
const hex = (c: RGB) => `#${c.map(v => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0')).join('')}`;
const mix = (a: RGB, b: RGB, t: number): RGB => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];

// img-fx's card colour and 7-slot palette for a surface: ink steps over the paper, the peak in the highlight slot.
// The shader then fades cells that match the card, so the mosaic sits on the real surface instead of its own black
export function imageGenColors(paper: RGB, ink: RGB, peak: RGB): { cardBg: string; colors: string[] } {
  const k = luminance(paper) < 0.5 ? 1 : IMAGE_GEN.lightInk;
  return { cardBg: hex(paper), colors: IMAGE_GEN.steps.map(s => hex(s === null ? peak : mix(paper, ink, s * k))) };
}
