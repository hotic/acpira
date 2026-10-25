import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { BorderBeam } from 'border-beam';
import { useAppearance, type Appearance } from '../appearance';
import { BEAM_SIZE, BEAM_STRENGTH } from './presets';

export interface WorkingBeamProps {
  active: boolean;
  theme: 'dark' | 'light';
  children: ReactNode;
}

const FULL_BEAM_GRADIENT: Record<Appearance['beamColor'], Record<'dark' | 'light', string>> = {
  mono: {
    dark: 'conic-gradient(from 0deg, transparent 0%, transparent 32%, rgba(180,180,180,0.06) 40%, rgba(210,210,210,0.16) 54%, rgba(255,255,255,0.34) 66%, rgba(210,210,210,0.16) 76%, rgba(180,180,180,0.05) 88%, transparent 95%, transparent 100%)',
    light: 'conic-gradient(from 0deg, transparent 0%, transparent 32%, rgba(100,100,100,0.04) 40%, rgba(70,70,70,0.12) 54%, rgba(0,0,0,0.22) 66%, rgba(70,70,70,0.12) 76%, rgba(100,100,100,0.04) 88%, transparent 95%, transparent 100%)',
  },
  ocean: {
    dark: 'conic-gradient(from 0deg, transparent 0%, transparent 30%, rgba(60,120,255,0.14) 38%, rgba(100,80,220,0.36) 52%, rgba(200,220,255,0.62) 66%, rgba(130,70,255,0.36) 78%, rgba(50,140,220,0.12) 90%, transparent 96%, transparent 100%)',
    light: 'conic-gradient(from 0deg, transparent 0%, transparent 30%, rgba(60,120,255,0.1) 38%, rgba(100,80,220,0.24) 52%, rgba(50,80,200,0.42) 66%, rgba(130,70,255,0.24) 78%, rgba(50,140,220,0.08) 90%, transparent 96%, transparent 100%)',
  },
  colorful: {
    dark: 'conic-gradient(from 0deg, transparent 0%, transparent 28%, rgba(40,140,255,0.16) 36%, rgba(180,40,240,0.34) 48%, rgba(255,50,100,0.48) 58%, rgba(255,255,255,0.68) 66%, rgba(255,160,30,0.42) 74%, rgba(50,200,80,0.24) 84%, rgba(30,185,170,0.1) 92%, transparent 97%, transparent 100%)',
    light: 'conic-gradient(from 0deg, transparent 0%, transparent 28%, rgba(40,140,255,0.12) 36%, rgba(180,40,240,0.22) 48%, rgba(255,50,100,0.32) 58%, rgba(200,30,60,0.45) 66%, rgba(255,120,40,0.28) 74%, rgba(50,200,80,0.18) 84%, rgba(30,185,170,0.08) 92%, transparent 97%, transparent 100%)',
  },
};

// Beam wrapping the composer: lights up while the composer is focused (including while one of its menus is open), fades out on blur. The radius is measured off the wrapped element; capsule-like large radii clamp to half the height
export function WorkingBeam({ active, theme, children }: WorkingBeamProps) {
  const { beam, beamColor, motion } = useAppearance();
  const hostRef = useRef<HTMLDivElement>(null);
  const [radius, setRadius] = useState(12);

  useLayoutEffect(() => {
    const el = hostRef.current?.firstElementChild as HTMLElement | null;
    if (!el) return;
    const r = parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0;
    let lastH = -1;
    const measure = (entries?: ResizeObserverEntry[]) => {
      const h = Math.round(entries?.[0]?.borderBoxSize?.[0]?.blockSize ?? el.offsetHeight);
      if (h === lastH) return;
      lastH = h;
      setRadius(Math.min(r, Math.max(h, 32) / 2));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [beam]);

  const enabled = beam !== 'none' && motion !== 'none';
  return (
    <div ref={hostRef} className="relative block w-full">
      {children}
      {enabled && (beam === 'full' ? (
        <div
          aria-hidden="true"
          data-active={active || undefined}
          className="working-beam-ring"
          style={{
            '--beam-radius': `${radius}px`,
            '--beam-strength': BEAM_STRENGTH[motion],
            '--beam-gradient': FULL_BEAM_GRADIENT[beamColor][theme],
          } as CSSProperties}
        >
          <div className="working-beam-disc" />
        </div>
      ) : (
        <BorderBeam
          key={beam}
          className="pointer-events-none absolute! inset-0 contain-strict transform-gpu backface-hidden"
          size={BEAM_SIZE[beam]}
          colorVariant={beamColor}
          staticColors={beamColor === 'mono'}
          theme={theme}
          borderRadius={radius}
          active={active}
          strength={BEAM_STRENGTH[motion]}
        >
          <div className="size-full" />
        </BorderBeam>
      ))}
    </div>
  );
}
