import { useEffect, useRef } from 'react';
import { mountOrb } from './orbEngine';
import type { OrbKind } from './presets';

// 20px inline Orb, exactly filling the lead slot, drawn by orbEngine.ts. Theme follows the nearest ancestor data-theme.
// Deliberately outside the motion switch: it is the one "still working" signal, a 20px canvas on its own layer; only
// the OS reduced-motion preference freezes it on a single frame
export function Orb({ kind, paused }: { kind: OrbKind; paused?: boolean }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    return canvas ? mountOrb(canvas, kind, !!paused) : undefined;
  }, [kind, paused]);
  return (
    <span className="flex size-lead items-center justify-center">
      <canvas ref={ref} aria-hidden className="block size-lead" />
    </span>
  );
}
