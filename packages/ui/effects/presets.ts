import type { Appearance } from '../appearance';

// Tuning for the effects under effects/ (Orb, Beam, MetalRing, the image generation mosaic)

// The Orb's two looks (orbEngine.ts): a breathing ring of dots for thought, a turning constellation for connection
export type OrbKind = 'think' | 'fetch';

export const ORB_SIZE = 20 as const;

export const BEAM_SIZE: Record<Exclude<Appearance['beam'], 'none'>, 'md' | 'line' | 'pulse-inner'> = {
  full: 'md',
  line: 'line',
  pulse: 'pulse-inner',
};
export const BEAM_STRENGTH: Record<Appearance['motion'], number> = { none: 0, on: 0.8 };


// Image generation card (picked in lab/image-gen-fx: inset card, pixel mosaic loader, cell dissolve, prism peak).
// The mosaic (effects/mosaicGL) runs while the tool is open, recoloured from the theme: `steps` mix the ink into the card
// surface per palette slot (null = the peak colour, slot 5 is the highlight / mask slot), halved on light paper where
// dark ink reads much heavier. Below 30 fps it stutters visibly at card size. The saved image then dissolves in on a
// canvas, cell by cell from the centre outwards.
export const IMAGE_GEN = {
  pixelScale: 0.8,
  fps: 30,
  steps: [0.1, 0.38, 0.72, 0, null, 0, 0.22],
  lightInk: 0.45,
  dissolve: { ms: 1200, cell: 12, band: 0.18, grow: 0.6 },
} as const;
