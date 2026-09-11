import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { BorderBeam } from 'border-beam';
import { useAppearance } from '../appearance';
import { BEAM_SIZE, BEAM_STRENGTH } from './presets';

export interface WorkingBeamProps {
  active: boolean;
  theme: 'dark' | 'light';
  children: ReactNode;
}

// Beam wrapping the composer: lights up while the composer is focused (including while one of its menus is open), fades out on blur. The radius is measured off the wrapped element; capsule-like large radii clamp to half the height
export function WorkingBeam({ active, theme, children }: WorkingBeamProps) {
  const { beam, beamColor, motion } = useAppearance();
  const hostRef = useRef<HTMLDivElement>(null);
  const [radius, setRadius] = useState(12);

  useLayoutEffect(() => {
    const el = hostRef.current?.firstElementChild as HTMLElement | null;
    if (!el) return;
    const measure = () => {
      const r = parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0;
      setRadius(Math.min(r, Math.max(el.offsetHeight, 32) / 2));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [beam]);

  return (
    <BorderBeam
      key={beam}
      className="block! w-full"
      size={beam === 'none' ? 'line' : BEAM_SIZE[beam]}
      colorVariant={beamColor}
      theme={theme}
      borderRadius={radius}
      active={beam !== 'none' && active && motion !== 'none'}
      strength={BEAM_STRENGTH[motion]}
    >
      <div ref={hostRef}>{children}</div>
    </BorderBeam>
  );
}
