import { useEffect, useLayoutEffect, useRef } from 'react';
import { cn } from '../ui/cn';
import { VISIBLE_MARGIN, createMosaic, type MosaicHandle, type MosaicTheme } from './mosaicGL';

interface Props {
  theme: MosaicTheme;
  /** Opaque card surface (#rrggbb), also painted behind the mosaic. */
  cardBg: string;
  /** Palette slots (#rrggbb), see `imageGenColors`. */
  colors: readonly string[];
  paused?: boolean;
  className?: string;
}

// The image generation mosaic filling its box. Renders nothing extra without WebGL 2 (callers gate on hasWebGL2)
export function PixelMosaic({ theme, cardBg, colors, paused = false, className }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const mosaic = useRef<MosaicHandle | null>(null);
  const look = { theme, cardBg, colors };
  const lookRef = useRef(look);
  lookRef.current = look;

  useLayoutEffect(() => {
    const el = host.current!;
    const size = () => { const b = el.getBoundingClientRect(); return [Math.max(1, Math.round(b.width)), Math.max(1, Math.round(b.height))] as const; };
    const m = createMosaic(canvas.current!, ...size(), lookRef.current);
    if (!m) return;
    mosaic.current = m;
    const ro = new ResizeObserver(() => m.setSize(...size()));
    ro.observe(el);
    const io = new IntersectionObserver(entries => { for (const e of entries) m.setVisible(e.isIntersecting); }, { rootMargin: VISIBLE_MARGIN });
    io.observe(el);
    return () => { ro.disconnect(); io.disconnect(); m.destroy(); mosaic.current = null; };
  }, []);

  // Joined so a fresh array with the same colours does not redraw
  const key = `${theme} ${cardBg} ${colors.join(' ')}`;
  useEffect(() => { mosaic.current?.setLook(lookRef.current); }, [key]);
  useEffect(() => { mosaic.current?.setPaused(paused); }, [paused]);

  return (
    <div ref={host} className={cn('relative overflow-hidden', className)} style={{ background: cardBg }}>
      <canvas ref={canvas} aria-hidden className="pointer-events-none absolute inset-0 size-full" />
    </div>
  );
}
