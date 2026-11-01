import { useEffect, useMemo, useState, type AnimationEvent, type CSSProperties, type ReactNode } from 'react';

// Credit: ported from border-beam 1.3.0 by Jakub Antalik (https://beam.jakubantalik.com), MIT License, Copyright (c) 2026 Jakub Antalik.
// Full notice in THIRD_PARTY_NOTICES.md.
// Animated border beam: a port of border-beam limited to the sizes, colours and
// themes the composer uses. Gradients and tuning are the library's own; what changed is the plumbing: every custom
// property is registered once in styles/beam.css (the library injected a <style> with per-instance @property rules,
// and WebKit restyles the whole document whenever that registry changes), the pulse oscillators run as CSS
// animations instead of a 30 fps rAF loop writing inline styles, and the fade lives on an inner layer so the
// wrapped children never restyle with it.

export type BeamSize = 'md' | 'line' | 'pulse-inner';
export type BeamColor = 'mono' | 'ocean' | 'colorful';
export type BeamTheme = 'dark' | 'light';

export interface BeamProps {
  children?: ReactNode;
  size?: BeamSize;
  colorVariant?: BeamColor;
  // Off: hue-shift animation (always off for mono)
  staticColors?: boolean;
  theme?: BeamTheme;
  borderRadius?: number;
  // Fades in on true, fades out (then unmounts its layers) on false
  active?: boolean;
  // 0-1, scales the beam layers only
  strength?: number;
  className?: string;
}

// Per size and theme: layer opacities, inner shadow, filter brightness / saturation
interface Tune { stroke: number; inner: number; bloom: number; shadow: string; bri: number; sat: number }
const TUNE: Record<BeamSize, Record<BeamTheme, Tune>> = {
  md: {
    dark: { stroke: 0.26, inner: 0.42, bloom: 0.24, shadow: 'rgba(255, 255, 255, 0.27)', bri: 1.3, sat: 1.2 },
    light: { stroke: 0.12, inner: 0.26, bloom: 0.34, shadow: 'rgba(0, 0, 0, 0.14)', bri: 1.3, sat: 1.5 },
  },
  line: {
    dark: { stroke: 1.14, inner: 0.7, bloom: 0.8, shadow: 'rgba(255, 255, 255, 0.1)', bri: 1.3, sat: 1.2 },
    light: { stroke: 0.16, inner: 0.32, bloom: 0.3, shadow: 'rgba(0, 0, 0, 0.14)', bri: 1.3, sat: 1.95 },
  },
  'pulse-inner': {
    dark: { stroke: 1.54, inner: 0.44, bloom: 0.66, shadow: 'transparent', bri: 0.75, sat: 1.2 },
    light: { stroke: 0.32, inner: 0.4, bloom: 0.8, shadow: 'transparent', bri: 1.3, sat: 0.75 },
  },
};

// Colour blobs placed around the border (md stroke / glow, pulse layers), plus the line bloom's spike colours
interface Blob { color: string; pos: string; size: string }
const BORDER: Record<BeamColor, { border: Blob[]; spike: [string, string]; spikeLt: [string, string] }> = {
  colorful: {
    border: [
      { color: 'rgb(255, 50, 100)', pos: '33% -7.4%', size: '70px 40px' },
      { color: 'rgb(40, 140, 255)', pos: '12% -5%', size: '60px 35px' },
      { color: 'rgb(50, 200, 80)', pos: '2.1% 68.3%', size: '40px 70px' },
      { color: 'rgb(30, 185, 170)', pos: '2.1% 68.3%', size: '20px 35px' },
      { color: 'rgb(100, 70, 255)', pos: '74.4% 100%', size: '180px 32px' },
      { color: 'rgb(40, 140, 255)', pos: '55% 100%', size: '85px 26px' },
      { color: 'rgb(255, 120, 40)', pos: '93.9% 0%', size: '74px 32px' },
      { color: 'rgb(240, 50, 180)', pos: '100% 27.1%', size: '26px 42px' },
      { color: 'rgb(180, 40, 240)', pos: '100% 27.1%', size: '52px 48px' },
    ],
    spike: ['rgb(255, 60, 80)', 'rgba(40, 190, 180, 0.98)'],
    spikeLt: ['rgb(200, 30, 60)', 'rgb(20, 150, 140)'],
  },
  mono: {
    border: [
      { color: 'rgb(180, 180, 180)', pos: '33% -7.4%', size: '70px 40px' },
      { color: 'rgb(140, 140, 140)', pos: '12% -5%', size: '60px 35px' },
      { color: 'rgb(160, 160, 160)', pos: '2.1% 68.3%', size: '40px 70px' },
      { color: 'rgb(130, 130, 130)', pos: '2.1% 68.3%', size: '20px 35px' },
      { color: 'rgb(170, 170, 170)', pos: '74.4% 100%', size: '180px 32px' },
      { color: 'rgb(150, 150, 150)', pos: '55% 100%', size: '85px 26px' },
      { color: 'rgb(190, 190, 190)', pos: '93.9% 0%', size: '74px 32px' },
      { color: 'rgb(145, 145, 145)', pos: '100% 27.1%', size: '26px 42px' },
      { color: 'rgb(165, 165, 165)', pos: '100% 27.1%', size: '52px 48px' },
    ],
    spike: ['rgb(200, 200, 200)', 'rgb(170, 170, 170)'],
    spikeLt: ['rgb(80, 80, 80)', 'rgb(120, 120, 120)'],
  },
  ocean: {
    border: [
      { color: 'rgb(100, 80, 220)', pos: '33% -7.4%', size: '70px 40px' },
      { color: 'rgb(60, 120, 255)', pos: '12% -5%', size: '60px 35px' },
      { color: 'rgb(80, 100, 200)', pos: '2.1% 68.3%', size: '40px 70px' },
      { color: 'rgb(50, 140, 220)', pos: '2.1% 68.3%', size: '20px 35px' },
      { color: 'rgb(120, 80, 255)', pos: '74.4% 100%', size: '180px 32px' },
      { color: 'rgb(70, 130, 255)', pos: '55% 100%', size: '85px 26px' },
      { color: 'rgb(140, 100, 240)', pos: '93.9% 0%', size: '74px 32px' },
      { color: 'rgb(90, 110, 230)', pos: '100% 27.1%', size: '26px 42px' },
      { color: 'rgb(130, 70, 255)', pos: '100% 27.1%', size: '52px 48px' },
    ],
    spike: ['rgb(100, 120, 255)', 'rgba(130, 100, 220, 0.98)'],
    spikeLt: ['rgb(60, 60, 180)', 'rgb(80, 100, 200)'],
  },
};

// Line stroke blobs riding the travelling beam: [colour, w, h, dx, dy]
type LineBlob = [string, number, number, number, number];
const LINE_STROKE: Record<BeamColor, Record<BeamTheme, LineBlob[]>> = {
  colorful: {
    dark: [['rgb(255, 50, 100)', 36, 36, 0, 2], ['rgb(40, 180, 220)', 30, 32, 39, 0], ['rgb(50, 200, 80)', 33, 28, -36, 2], ['rgb(180, 40, 240)', 29, 34, -54, 0], ['rgb(255, 160, 30)', 27, 30, 51, -1], ['rgb(100, 70, 255)', 36, 24, 21, 1], ['rgb(40, 140, 255)', 30, 22, -21, 0], ['rgb(240, 50, 180)', 25, 28, 66, 1], ['rgb(30, 185, 170)', 23, 30, -66, -1]],
    light: [['rgb(255, 50, 100)', 45, 36, 0, 2], ['rgb(40, 140, 255)', 35, 32, 65, 0], ['rgb(50, 200, 80)', 40, 28, -60, 2], ['rgb(180, 40, 240)', 35, 34, -90, 0], ['rgb(30, 185, 170)', 38, 30, 85, -1], ['rgb(100, 70, 255)', 50, 24, 35, 1], ['rgb(40, 140, 255)', 40, 22, -35, 0], ['rgb(255, 120, 40)', 35, 28, 110, 1], ['rgb(240, 50, 180)', 30, 30, -110, -1]],
  },
  mono: {
    dark: [['rgb(200, 200, 200)', 36, 36, 0, 2], ['rgb(170, 170, 170)', 30, 32, 39, 0], ['rgb(155, 155, 155)', 33, 28, -36, 2], ['rgb(185, 185, 185)', 29, 34, -54, 0], ['rgb(165, 165, 165)', 27, 30, 51, -1], ['rgb(180, 180, 180)', 36, 24, 21, 1], ['rgb(160, 160, 160)', 30, 22, -21, 0], ['rgb(175, 175, 175)', 25, 28, 66, 1], ['rgb(190, 190, 190)', 23, 30, -66, -1]],
    light: [['rgb(100, 100, 100)', 45, 36, 0, 2], ['rgb(80, 80, 80)', 35, 32, 65, 0], ['rgb(90, 90, 90)', 40, 28, -60, 2], ['rgb(70, 70, 70)', 35, 34, -90, 0], ['rgb(85, 85, 85)', 38, 30, 85, -1], ['rgb(95, 95, 95)', 50, 24, 35, 1], ['rgb(75, 75, 75)', 40, 22, -35, 0], ['rgb(105, 105, 105)', 35, 28, 110, 1], ['rgb(65, 65, 65)', 30, 30, -110, -1]],
  },
  ocean: {
    dark: [['rgb(100, 80, 220)', 36, 36, 0, 2], ['rgb(60, 120, 255)', 30, 32, 39, 0], ['rgb(80, 100, 200)', 33, 28, -36, 2], ['rgb(130, 70, 255)', 29, 34, -54, 0], ['rgb(70, 130, 255)', 27, 30, 51, -1], ['rgb(120, 80, 255)', 36, 24, 21, 1], ['rgb(90, 110, 230)', 30, 22, -21, 0], ['rgb(110, 90, 240)', 25, 28, 66, 1], ['rgb(140, 100, 255)', 23, 30, -66, -1]],
    light: [['rgb(80, 60, 200)', 45, 36, 0, 2], ['rgb(50, 100, 220)', 35, 32, 65, 0], ['rgb(70, 90, 190)', 40, 28, -60, 2], ['rgb(110, 60, 220)', 35, 34, -90, 0], ['rgb(60, 110, 230)', 38, 30, 85, -1], ['rgb(100, 70, 240)', 50, 24, 35, 1], ['rgb(80, 100, 210)', 40, 22, -35, 0], ['rgb(90, 80, 225)', 35, 28, 110, 1], ['rgb(120, 90, 245)', 30, 30, -110, -1]],
  },
};

// Line inner glow blobs (both themes)
const LINE_GLOW: Record<BeamColor, LineBlob[]> = {
  colorful: [['rgba(255, 50, 100, 0.48)', 33, 30, 0, 0], ['rgba(40, 180, 220, 0.42)', 24, 26, 39, -3], ['rgba(50, 200, 80, 0.48)', 27, 24, -36, 0], ['rgba(180, 40, 240, 0.42)', 23, 28, -54, -2], ['rgba(255, 160, 30, 0.50)', 24, 24, 51, -1], ['rgba(100, 70, 255, 0.45)', 30, 20, 21, 0], ['rgba(40, 140, 255, 0.40)', 25, 18, -21, -2], ['rgba(240, 50, 180, 0.45)', 21, 24, 66, 0], ['rgba(30, 185, 170, 0.52)', 18, 26, -66, -1]],
  mono: [['rgba(200, 200, 200, 0.48)', 33, 30, 0, 0], ['rgba(170, 170, 170, 0.42)', 24, 26, 39, -3], ['rgba(155, 155, 155, 0.48)', 27, 24, -36, 0], ['rgba(185, 185, 185, 0.42)', 23, 28, -54, -2], ['rgba(165, 165, 165, 0.50)', 24, 24, 51, -1], ['rgba(180, 180, 180, 0.45)', 30, 20, 21, 0], ['rgba(160, 160, 160, 0.40)', 25, 18, -21, -2], ['rgba(175, 175, 175, 0.45)', 21, 24, 66, 0], ['rgba(190, 190, 190, 0.52)', 18, 26, -66, -1]],
  ocean: [['rgba(100, 80, 220, 0.48)', 33, 30, 0, 0], ['rgba(60, 120, 255, 0.42)', 24, 26, 39, -3], ['rgba(80, 100, 200, 0.48)', 27, 24, -36, 0], ['rgba(130, 70, 255, 0.42)', 23, 28, -54, -2], ['rgba(70, 130, 255, 0.50)', 24, 24, 51, -1], ['rgba(120, 80, 255, 0.45)', 30, 20, 21, 0], ['rgba(90, 110, 230, 0.40)', 25, 18, -21, -2], ['rgba(110, 90, 240, 0.45)', 21, 24, 66, 0], ['rgba(140, 100, 255, 0.52)', 18, 26, -66, -1]],
};

// Line bloom spikes at 36 / 50 / 64 / 78 / 92 %: [colour1, colour2]
const LINE_SPIKES: Record<BeamColor, Record<BeamTheme, [string, string][]>> = {
  colorful: {
    dark: [['rgb(100, 70, 255)', 'rgba(100, 70, 255, 1)'], ['rgba(255, 170, 40, 0.59)', 'rgba(255, 170, 40, 0.29)'], ['rgb(50, 200, 100)', 'rgba(50, 200, 100, 1)'], ['rgba(200, 50, 240, 0.91)', 'rgba(200, 50, 240, 0.45)'], ['rgb(40, 140, 255)', 'rgba(40, 140, 255, 1)']],
    light: [['rgb(80, 50, 200)', 'rgba(80, 50, 200, 0.8)'], ['rgba(210, 130, 0, 0.7)', 'rgba(210, 130, 0, 0.46)'], ['rgb(30, 160, 70)', 'rgba(30, 160, 70, 0.82)'], ['rgb(160, 30, 190)', 'rgba(160, 30, 190, 0.7)'], ['rgb(30, 100, 200)', 'rgba(30, 100, 200, 0.78)']],
  },
  mono: {
    dark: [['rgb(200, 200, 200)', 'rgba(200, 200, 200, 1)'], ['rgba(180, 180, 180, 0.59)', 'rgba(180, 180, 180, 0.29)'], ['rgb(190, 190, 190)', 'rgba(190, 190, 190, 1)'], ['rgba(170, 170, 170, 0.91)', 'rgba(170, 170, 170, 0.45)'], ['rgb(185, 185, 185)', 'rgba(185, 185, 185, 1)']],
    light: [['rgb(80, 80, 80)', 'rgba(80, 80, 80, 0.8)'], ['rgba(100, 100, 100, 0.7)', 'rgba(100, 100, 100, 0.46)'], ['rgb(70, 70, 70)', 'rgba(70, 70, 70, 0.82)'], ['rgb(90, 90, 90)', 'rgba(90, 90, 90, 0.7)'], ['rgb(85, 85, 85)', 'rgba(85, 85, 85, 0.78)']],
  },
  ocean: {
    dark: [['rgb(100, 80, 255)', 'rgb(100, 80, 255)'], ['rgba(80, 130, 220, 0.59)', 'rgba(80, 130, 220, 0.29)'], ['rgb(60, 100, 255)', 'rgb(60, 100, 255)'], ['rgba(90, 120, 200, 0.91)', 'rgba(90, 120, 200, 0.45)'], ['rgb(120, 90, 255)', 'rgb(120, 90, 255)']],
    light: [['rgb(50, 40, 180)', 'rgba(50, 40, 180, 0.8)'], ['rgba(40, 80, 200, 0.7)', 'rgba(40, 80, 200, 0.46)'], ['rgb(30, 50, 190)', 'rgba(30, 50, 190, 0.82)'], ['rgb(60, 90, 180)', 'rgba(60, 90, 180, 0.7)'], ['rgb(70, 60, 200)', 'rgba(70, 60, 200, 0.78)']],
  },
};

const rgbOf = (c: string) => c.match(/^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)/)!.slice(1, 4).join(', ');
const alphaOf = (c: string) => Number(c.match(/^rgba\(.*,\s*([\d.]+)\s*\)$/)?.[1] ?? 1);
// Replace the alpha / multiply it (the library's K / q)
const withAlpha = (c: string, a: number) => `rgba(${rgbOf(c)}, ${a})`;
const scaleAlpha = (c: string, k: number) => `rgba(${rgbOf(c)}, ${(alphaOf(c) * k).toFixed(2)})`;
const off = (n: number) => (n === 0 ? '' : n > 0 ? ` + ${n}px` : ` - ${-n}px`);
const join = (layers: string[]) => layers.join(', ');

// The rotating highlight drawn over the md stroke
const MD_SHEEN = {
  dark: 'conic-gradient(from var(--bb-angle), transparent 0%, transparent 54%, rgba(255, 255, 255, 0.1) 57%, rgba(255, 255, 255, 0.3) 60%, rgba(255, 255, 255, 0.6) 63%, rgba(255, 255, 255, 0.75) 66%, rgba(255, 255, 255, 0.6) 69%, rgba(255, 255, 255, 0.3) 72%, rgba(255, 255, 255, 0.1) 75%, transparent 78%, transparent 100%)',
  light: 'conic-gradient(from var(--bb-angle), transparent 0%, transparent 54%, rgba(0, 0, 0, 0.08) 57%, rgba(0, 0, 0, 0.2) 60%, rgba(0, 0, 0, 0.4) 63%, rgba(0, 0, 0, 0.55) 66%, rgba(0, 0, 0, 0.4) 69%, rgba(0, 0, 0, 0.2) 72%, rgba(0, 0, 0, 0.08) 75%, transparent 78%, transparent 100%)',
};
const MD_BLOOM = {
  dark: 'conic-gradient(from var(--bb-angle), transparent 0%, transparent 58%, rgba(255, 255, 255, 0.03) 62%, rgba(255, 255, 255, 0.08) 65%, rgba(255, 255, 255, 0.2) 67%, rgba(255, 255, 255, 0.45) 69%, rgba(255, 255, 255, 0.85) 70%, rgba(255, 255, 255, 0.85) 70.5%, rgba(255, 255, 255, 0.45) 71.5%, rgba(255, 255, 255, 0.2) 73%, rgba(255, 255, 255, 0.08) 75%, rgba(255, 255, 255, 0.03) 78%, transparent 82%)',
  light: 'conic-gradient(from var(--bb-angle), transparent 0%, transparent 58%, rgba(0, 0, 0, 0.02) 62%, rgba(0, 0, 0, 0.08) 65%, rgba(0, 0, 0, 0.2) 67%, rgba(0, 0, 0, 0.4) 69%, rgba(0, 0, 0, 0.6) 70%, rgba(0, 0, 0, 0.6) 70.5%, rgba(0, 0, 0, 0.4) 71.5%, rgba(0, 0, 0, 0.2) 73%, rgba(0, 0, 0, 0.08) 75%, rgba(0, 0, 0, 0.02) 78%, transparent 82%)',
};

interface Layers { glow: string; stroke: string; bloom: string }

function mdLayers(color: BeamColor, theme: BeamTheme): Layers {
  const blobs = BORDER[color].border;
  const glowAlpha = color === 'mono' ? 0.225 : 0.45;
  return {
    stroke: join([MD_SHEEN[theme], ...blobs.map(b => `radial-gradient(ellipse ${b.size} at ${b.pos}, ${b.color}, transparent)`)]),
    glow: join(blobs.map(b => {
      const size = b.size.split(' ').map(s => `${Math.round(parseInt(s) * 0.9)}px`).join(' ');
      return `radial-gradient(ellipse ${size} at ${b.pos}, ${withAlpha(b.color, glowAlpha)}, transparent)`;
    })),
    bloom: MD_BLOOM[theme],
  };
}

const lineBlob = ([c, w, h, dx, dy]: LineBlob, dyUp = false) =>
  `radial-gradient(ellipse calc(${w}px * var(--bb-w)) calc(${h}px * var(--bb-h)) at calc(var(--bb-x) * 100%${off(dx)}) calc(100%${dyUp ? off(-Math.abs(dy)) : off(dy)}), ${c}, transparent)`;

function lineLayers(color: BeamColor, theme: BeamTheme): Layers {
  const dark = theme === 'dark';
  const sheen = dark
    ? 'radial-gradient(ellipse calc(24px * var(--bb-w)) calc(28px * var(--bb-h)) at calc(var(--bb-x) * 100%) calc(100% + 2px), rgba(255, 255, 255, 0.38) 0%, rgba(255, 255, 255, 0.12) 30%, transparent 65%)'
    : 'radial-gradient(ellipse calc(35px * var(--bb-w)) calc(28px * var(--bb-h)) at calc(var(--bb-x) * 100%) calc(100% + 2px), rgba(0, 0, 0, 0.6) 0%, rgba(0, 0, 0, 0.25) 35%, transparent 70%)';
  return {
    stroke: join([sheen, ...LINE_STROKE[color][theme].map(b => lineBlob(b))]),
    glow: join(LINE_GLOW[color].map(b => lineBlob(b, true))),
    bloom: lineBloom(color, dark),
  };
}

// Spikes rising from the bottom edge plus the travelling hot spot (the library's Ue)
function lineBloom(color: BeamColor, dark: boolean): string {
  const mono = color === 'mono';
  const [primary, secondary] = dark ? BORDER[color].spike : BORDER[color].spikeLt;
  const c = mono ? scaleAlpha(primary, 0.14) : primary;
  const p = mono ? scaleAlpha(primary, 0.09) : primary;
  const b = mono ? scaleAlpha(secondary, 0.12) : secondary;
  const m = mono ? withAlpha(secondary, 0.06) : withAlpha(secondary, 0.49);
  const n = LINE_SPIKES[color][dark ? 'dark' : 'light'].map(([c1, c2]) => (mono ? [scaleAlpha(c1, 0.14), scaleAlpha(c2, 0.14 * 0.7)] : [c1, c2]));
  const [l, d, k, f, g, W, Y, z, H] = mono
    ? ['12px', '14px', '12px', '10px', '42px', '38px', '40px', '32px', '12px']
    : ['0.8px', '2px', '1.2px', '0.6px', '92px', '72px', '85px', '60px', '1px'];
  const second = dark ? m : mono ? scaleAlpha(secondary, 0.09) : withAlpha(secondary, 0.7);
  const first = dark ? p : mono ? scaleAlpha(primary, 0.11) : withAlpha(primary, 0.85);
  const spikes = [
    `radial-gradient(ellipse calc(${l} * var(--bb-spike)) calc(${g} * var(--bb-h)) at 8% calc(100% - 2px), ${c}, ${first} 30%, transparent 88%)`,
    `radial-gradient(ellipse calc(10px * var(--bb-spike2)) calc(35px * var(--bb-h)) at 22% calc(100% - 4px), ${b}, ${second} 50%, transparent 95%)`,
    `radial-gradient(ellipse calc(${d} * (2 - var(--bb-spike))) calc(${W} * var(--bb-h)) at 36% calc(100% - 3px), ${n[0]![0]}, ${n[0]![1]} 40%, transparent 90%)`,
    `radial-gradient(ellipse calc(14px * var(--bb-spike2)) calc(28px * var(--bb-h)) at 50% calc(100% - 2px), ${n[1]![0]}, ${n[1]![1]} 55%, transparent 96%)`,
    `radial-gradient(ellipse calc(${k} * (2 - var(--bb-spike2))) calc(${Y} * var(--bb-h)) at 64% calc(100% - 4px), ${n[2]![0]}, ${n[2]![1]} 35%, transparent 89%)`,
    `radial-gradient(ellipse calc(7px * var(--bb-spike)) calc(45px * var(--bb-h)) at 78% calc(100% - 2px), ${n[3]![0]}, ${n[3]![1]} 48%, transparent 94%)`,
    `radial-gradient(ellipse calc(${dark ? f : H} * (2 - var(--bb-spike))) calc(${z} * var(--bb-h)) at 92% calc(100% - 3px), ${n[4]![0]}, ${n[4]![1]} 42%, transparent 91%)`,
  ];
  if (!dark) {
    return join([...spikes, 'radial-gradient(ellipse calc(50px * var(--bb-w)) calc(32px * var(--bb-h)) at calc(var(--bb-x) * 100%) calc(100%), rgba(0, 0, 0, 0.5) 0%, rgba(0, 0, 0, 0.18) 30%, rgba(0, 0, 0, 0.03) 60%, transparent 85%)']);
  }
  const w = (a: number) => `rgba(255, 255, 255, ${mono ? a / 2 : a})`;
  return join([
    ...spikes,
    `radial-gradient(ellipse calc(21px * var(--bb-spike)) calc(15px * var(--bb-spike2)) at calc(var(--bb-x) * 100%) calc(100% + 1px), ${w(1)} 0%, ${w(0.9)} 20%, ${w(0.5)} 50%, transparent 100%)`,
    `radial-gradient(ellipse calc(42px * var(--bb-w)) calc(40px * var(--bb-h)) at calc(var(--bb-x) * 100%) 100%, ${w(0.3)} 0%, ${w(0.12)} 25%, ${w(0.03)} 55%, transparent 80%)`,
  ]);
}

// Pulse oscillators (the library's sinusoidal rAF loop): [from, to, period s, delay s, unit]. Order:
// w1 h1 x1 y1, w2 h2 x2 y2, w3 h3 x3 y3 (three blob regions), glow height, corner opacities tl tr bl br
type Osc = [number, number, number, number, '' | 'px'];
function pulseOscs(dark: boolean): Osc[] {
  const sp = 0.28, dr = dark ? 33 : 40, op = dark ? 0.48 : 0.45, gh = dark ? 0.34 : 0.22;
  const bs = dark ? 1.9 : 2.6, ss = dark ? 2.6 : 4.6, ghs = dark ? 2.4 : 5.5;
  return [
    [1 - sp, 1 + sp * 1.1, ss * 0.9, 0, ''],
    [1 + sp * 0.9, 1 - sp * 0.85, ss * 1.26, 0, ''],
    [-dr, dr * 0.9, bs * 1.6, 0, 'px'],
    [dr * 0.55, -dr * 0.7, bs * 1.6, 0, 'px'],
    [1 + sp, 1 - sp * 0.85, ss * 1.1, 0, ''],
    [1 - sp * 0.8, 1 + sp * 1.05, ss * 0.81, 0, ''],
    [dr * 0.8, -dr * 0.9, bs * 1.88, 0, 'px'],
    [-dr, dr * 0.65, bs * 1.88, 0, 'px'],
    [1 - sp * 0.6, 1 + sp * 1.15, ss * 0.98, 0, ''],
    [1 + sp * 0.75, 1 - sp, ss * 1.4, 0, ''],
    [-dr * 0.6, dr, bs * 1.45, 0, 'px'],
    [-dr * 0.85, dr * 0.45, bs * 1.45, 0, 'px'],
    [1 - gh, 1 + gh, ghs, 0, ''],
    [1 - op, 1, bs, 0, ''],
    [1 - op, 1, bs * 1.32, bs * 0.28, ''],
    [1 - op, 1, bs * 0.84, bs * 0.55, ''],
    [1 - op, 1, bs * 1.58, bs * 0.83, ''],
  ];
}
const PULSE_HUE_PERIOD = 16;
// Half a cosine per keyframe half, as the library's (1 - cos 2πt) / 2
const SINE = 'cubic-bezier(0.37, 0, 0.63, 1)';
const QUADS = ['tl', 'tr', 'bl', 'br'] as const;
type Quad = (typeof QUADS)[number];
// Region / corner each border blob breathes with, and the glow's own blob sizes
const PULSE_REGION: [number, Quad][] = [[1, 'tl'], [2, 'tl'], [3, 'bl'], [1, 'bl'], [2, 'br'], [3, 'br'], [1, 'tr'], [2, 'tr'], [3, 'tr']];
const PULSE_GLOW_SIZE = [[65, 35], [55, 30], [35, 65], [15, 30], [173, 28], [80, 22], [69, 28], [22, 38], [47, 44]];
// Bloom blobs: [blob index, w, h]
const PULSE_BLOOM: [number, number, number][] = [[0, 84, 48], [1, 72, 42], [2, 48, 84], [4, 216, 38], [5, 102, 31], [6, 89, 38], [8, 62, 58]];

const num = (n: number) => String(+n.toFixed(4));

function pulseLayers(color: BeamColor, theme: BeamTheme): Layers {
  const dark = theme === 'dark';
  const oscs = pulseOscs(dark);
  // Oscillator i as a CSS value: from + (to - from) * phase, the phase animated by bb-o<i>
  const v = (i: number) => {
    const [a, b, , , unit] = oscs[i]!;
    return `calc(${num(a)}${unit} + ${num(b - a)}${unit} * var(--bb-o${i}))`;
  };
  const blob = (c: string, w: number, h: number, region: number, quad: Quad, x: string, y: string) => {
    const r = (region - 1) * 4;
    return `radial-gradient(ellipse calc(${w}px * ${v(r)}) calc(${h}px * ${v(r + 1)} * ${v(12)}) at calc(${x} + ${v(r + 2)}) calc(${y} + ${v(r + 3)}), rgba(${rgbOf(c)}, ${v(13 + QUADS.indexOf(quad))}), transparent)`;
  };
  const blobs = BORDER[color].border;
  const ink = dark ? '255, 255, 255' : '0, 0, 0';
  const cornerAlpha = dark ? 0.18 : 0.08;
  const bloomAlpha = +(1 - (dark ? 0.48 : 0.45) * 0.5).toFixed(3);
  return {
    stroke: join(blobs.map((b, i) => {
      const [x, y] = b.pos.split(' ') as [string, string];
      const [w, h] = b.size.split(' ').map(parseFloat) as [number, number];
      const [region, quad] = PULSE_REGION[i]!;
      return blob(b.color, w, h, region, quad, x, y);
    })),
    glow: join([
      ...blobs.map((b, i) => {
        const [x, y] = b.pos.split(' ') as [string, string];
        const [w, h] = PULSE_GLOW_SIZE[i]!;
        const [region, quad] = PULSE_REGION[i]!;
        return blob(b.color, w!, h!, region, quad, x, y);
      }),
      ...([['0%', '0%', 'tl'], ['100%', '0%', 'tr'], ['0%', '100%', 'bl'], ['100%', '100%', 'br']] as const).map(([x, y, q]) =>
        `radial-gradient(ellipse 60px 60px at ${x} ${y}, rgba(${ink}, calc(${cornerAlpha} * ${v(13 + QUADS.indexOf(q))})), transparent 70%)`),
    ]),
    bloom: join(PULSE_BLOOM.map(([i, w, h]) => {
      const b = blobs[i]!;
      return `radial-gradient(ellipse ${w}px ${h}px at ${b.pos}, rgba(${rgbOf(b.color)}, ${bloomAlpha}), transparent)`;
    })),
  };
}

const LAYERS: Record<BeamSize, (c: BeamColor, t: BeamTheme) => Layers> = { md: mdLayers, line: lineLayers, 'pulse-inner': pulseLayers };
const layerCache = new Map<string, Layers>();
function layersFor(size: BeamSize, color: BeamColor, theme: BeamTheme): Layers {
  const key = `${size}|${color}|${theme}`;
  let hit = layerCache.get(key);
  if (!hit) layerCache.set(key, (hit = LAYERS[size](color, theme)));
  return hit;
}

// Fade-out length (styles/beam.css bb-out) plus slack: unmounts the layers when the motion kill switch drops the
// animation and animationend never fires
const FADE_OUT_FALLBACK_MS = 650;

export function Beam({ children, size = 'md', colorVariant = 'colorful', staticColors = false, theme = 'dark', borderRadius = 16, active = true, strength = 1, className }: BeamProps) {
  // Same state machine as the library: a fade-out always completes before the next fade-in
  const [shown, setShown] = useState(active);
  const [fading, setFading] = useState(false);
  if (active && !shown && !fading) setShown(true);
  else if (!active && shown && !fading) setFading(true);

  const settle = () => {
    setShown(false);
    setFading(false);
  };
  useEffect(() => {
    if (!fading) return;
    const t = setTimeout(settle, FADE_OUT_FALLBACK_MS);
    return () => clearTimeout(t);
  }, [fading]);

  const mono = colorVariant === 'mono';
  const still = mono || staticColors;
  const tune = TUNE[size][theme];
  const dim = mono && size !== 'line' ? 0.5 : 1;
  const layers = layersFor(size, colorVariant, theme);
  const rootStyle = {
    '--bb-r': `${borderRadius}px`,
    '--bb-strength': Math.max(0, Math.min(1, strength)),
    '--bb-bri': tune.bri,
    '--bb-sat': tune.sat,
    '--bb-shadow': tune.shadow,
  } as CSSProperties;

  const onEnd = (e: AnimationEvent<HTMLDivElement>) => {
    if (e.target === e.currentTarget && e.animationName === 'bb-out') settle();
  };
  const body = (
    <>
      <div className="bb-glow" style={{ background: layers.glow, '--bb-k': tune.inner * dim } as CSSProperties} />
      <div className="bb-stroke" style={{ background: layers.stroke, '--bb-k': tune.stroke * dim } as CSSProperties} />
      <div className="bb-bloom" style={{ background: layers.bloom, '--bb-k': tune.bloom * dim } as CSSProperties} />
    </>
  );
  return (
    <div className={className ? `bb ${className}` : 'bb'} data-bb={size} data-bb-still={still ? '' : undefined} data-bb-mono={mono ? '' : undefined} style={rootStyle}>
      {children}
      {shown && (
        <div className="bb-fx" data-state={fading ? 'off' : 'on'} onAnimationEnd={onEnd}>
          {size === 'pulse-inner' ? <PulseClock theme={theme} still={still}>{body}</PulseClock> : body}
        </div>
      )}
    </div>
  );
}

// Runs the pulse oscillators on the page clock (like the library), so every pulse beam breathes in step
function PulseClock({ theme, still, children }: { theme: BeamTheme; still: boolean; children: ReactNode }) {
  const [t0] = useState(() => performance.now() / 1000);
  const animation = useMemo(() => {
    const list = pulseOscs(theme === 'dark').map(([, , period, delay], i) => `bb-o${i} ${num(period)}s ${SINE} ${num(delay - t0)}s infinite`);
    if (!still) list.push(`bb-hue ${PULSE_HUE_PERIOD}s linear ${num(-t0)}s infinite`);
    return list.join(', ');
  }, [theme, still, t0]);
  return <div className="bb-osc" style={{ animation }}>{children}</div>;
}
