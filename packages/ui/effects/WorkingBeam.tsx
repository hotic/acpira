import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { BorderBeam } from 'border-beam';
import { useAppearance } from '../appearance';
import { BEAM_SIZE, BEAM_STRENGTH } from './presets';

export interface WorkingBeamProps {
  active: boolean;
  theme: 'dark' | 'light';
  // Which field it wraps: a new one (the composer's field remounted for another session) is measured again
  field?: string;
  children: ReactNode;
}

// Beam wrapping the composer: lights up while the composer is focused (including while one of its menus is open), fades out on blur. The radius is measured off the wrapped element; capsule-like large radii clamp to half the height
export function WorkingBeam({ active, theme, field, children }: WorkingBeamProps) {
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
  }, [beam, field]);

  const enabled = beam !== 'none' && motion !== 'none';
  return (
    <div ref={hostRef} className="relative block w-full">
      {children}
      {/* Every mode is border-beam drawn as an overlay sibling: the composer subtree never repaints with the beam's
          animated masks. The full ring is the library's `md` (stroke, 9px inner glow and bloom); a hand-rolled 1px
          rotating ring replaced it on 2026-10-25 and lost the glow, so it is not to be "optimised" back without them. */}
      {enabled && (
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
      )}
    </div>
  );
}
