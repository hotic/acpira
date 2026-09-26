import type { OrbState } from 'thinking-orbs';
import type { Appearance } from '../appearance';

// All parameters for the Libraries.dev trio live here; props exported from Studio get pasted straight in

// Reuse the Orb's built-in states: a constellation for connection, a breathing ring for thought.
export type OrbKind = 'think' | 'fetch';
export const ORB_STATE: Record<OrbKind, OrbState> = {
  think: 'breathing',
  fetch: 'connecting',
};

export const ORB_SIZE = 20 as const;

export const BEAM_SIZE: Record<Exclude<Appearance['beam'], 'none'>, 'md' | 'line' | 'pulse-inner'> = {
  full: 'md',
  line: 'line',
  pulse: 'pulse-inner',
};
export const BEAM_STRENGTH: Record<Appearance['motion'], number> = { none: 0, on: 0.8 };

// Send button's metal mode: metal-fx's silver ring; the button variant reads the child's radius (--r-md), 1px ring
export const METAL_PRESET = 'silver' as const;
export const METAL_VARIANT = 'button' as const;

// Image generation card (picked in lab/image-gen-fx: inset card, img-fx loader, cell dissolve, prism peak).
// img-fx's pixel mosaic runs while the tool is open, recoloured from the theme: `steps` mix the ink into the card surface
// per palette slot (null = the peak colour, slot 5 is the preset's highlight / mask slot), halved on light paper where
// dark ink reads much heavier. img-fx defaults to 10 fps, which stutters visibly at card size. The saved image then
// dissolves in on a canvas, cell by cell from the centre outwards.
export const IMAGE_GEN = {
  preset: 'pixels-organic',
  pixelScale: 0.8,
  fps: 30,
  steps: [0.1, 0.38, 0.72, 0, null, 0, 0.22],
  lightInk: 0.45,
  dissolve: { ms: 1200, cell: 12, band: 0.18, grow: 0.6 },
} as const;
