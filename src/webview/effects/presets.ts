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
