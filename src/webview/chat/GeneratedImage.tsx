import { memo, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { ImageGeneration } from 'img-fx';
import type { ImageRef, ToolCallBlock } from '@shared/transcript';
import { toolImages } from '@shared/imageTools';
import { useAppearance } from '../appearance';
import { useTheme } from '../look';
import { t } from '../i18n';
import { IMAGE_GEN } from '../effects/presets';
import { dissolveIn, keepImageGenRenderer, readImageGenColors } from '../effects/imageGen';
import { hasWebGL } from '../effects/webgl';
import { cn } from '../ui/cn';
import { AgentImage } from './AgentImage';
import { BlobUrlContext } from './fileLinks';

// A reveal that never finishes (a stalled decode, a canvas that never gets a frame) still ends in the static image
const REVEAL_TIMEOUT_MS = 6000;
// The card takes the image's shape (the --dur-open width transition) before the dissolve starts on the final crop
const SHAPE_MS = 260;

const reducedMotion = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
const idle = (fn: () => void) => {
  if (typeof requestIdleCallback === 'function') { const id = requestIdleCallback(fn, { timeout: 300 }); return () => cancelIdleCallback(id); }
  const id = setTimeout(fn, 0);
  return () => clearTimeout(id);
};

// The images of one image generation call, outside the process fold so a collapsed history does not hide them.
// Only a call first seen running in this view animates: its placeholder mosaic dissolves into the saved image and then
// hands over to the ordinary AgentImage. History, replay and a remount after completion show the static image directly.
// Memoized on the block reference like ToolCall; the slot keys keep the placeholder's card (and its shader) through completion.
export const GeneratedImages = memo(function GeneratedImages({ block }: { block: ToolCallBlock }) {
  const open = block.status === 'pending' || block.status === 'in_progress';
  const [live] = useState(open);
  const images = toolImages(block);
  const slots: (ImageRef | undefined)[] = open && images.length === 0 ? [undefined] : images;
  if (!slots.length) return null;
  return (
    <div className="flex min-w-0 flex-wrap gap-gap">
      {slots.map((image, i) => live
        ? <GenerationCard key={i} image={image} />
        : image && <AgentImage key={i} image={image} />)}
    </div>
  );
});

type Phase = 'loading' | 'shaping' | 'revealing' | 'done';

function GenerationCard({ image }: { image?: ImageRef }) {
  const blobUrl = useContext(BlobUrlContext);
  const { motion } = useAppearance();
  const [animate] = useState(() => motion !== 'none' && hasWebGL() && !reducedMotion());
  const src = image?.blob && blobUrl ? blobUrl(image.blob) : undefined;
  const [img, setImg] = useState<HTMLImageElement>();
  const [phase, setPhase] = useState<Phase>('loading');
  const finish = useCallback(() => setPhase('done'), []);

  useEffect(() => {
    if (!src || !animate) return;
    let alive = true;
    const el = new Image();
    el.src = src;
    el.decode().then(() => { if (alive) { setImg(el); setPhase(p => (p === 'loading' ? 'shaping' : p)); } }, () => { if (alive) finish(); });
    const timeout = setTimeout(finish, REVEAL_TIMEOUT_MS);
    return () => { alive = false; clearTimeout(timeout); };
  }, [src, animate, finish]);

  useEffect(() => {
    if (phase !== 'shaping') return;
    const timer = setTimeout(() => setPhase('revealing'), SHAPE_MS);
    return () => clearTimeout(timer);
  }, [phase]);

  if (image && (phase === 'done' || !animate)) return <AgentImage image={image} />;
  const label = t('fold.pending', { verb: t('verb.imagegen') });
  // box-content: the border sits outside the image box, as AgentImage's does, so the hand-over does not shift the layout
  const shape = 'relative box-content max-w-full overflow-hidden rounded-md border border-line bg-chip shadow-(--image-gen-card-shadow)';
  if (!animate) {
    return <div role="img" aria-label={label} aria-busy className={cn(shape, 'aspect-square w-agent-image', motion !== 'none' && 'animate-pulse')} />;
  }
  const ratio = img ? img.naturalWidth / Math.max(1, img.naturalHeight) : undefined;
  const style: CSSProperties | undefined = ratio === undefined ? undefined
    : { aspectRatio: String(ratio), width: `min(100%, calc(var(--agent-image) * ${ratio}))` };
  return (
    <div role="img" aria-label={label} aria-busy className={cn(shape, 'transition-[width] duration-(--dur-open) ease-out', ratio === undefined && 'aspect-square w-agent-image')} style={style}>
      <div className={cn('absolute inset-0 transition-opacity duration-(--image-gen-fade) ease-out', phase === 'revealing' && 'opacity-0')}>
        <MosaicLoader />
      </div>
      {phase === 'revealing' && img && <Dissolve img={img} onDone={finish} />}
    </div>
  );
}

// img-fx's mosaic on the card's own colours. It mounts on the next idle slot rather than with the row: the first card
// in a webview builds the shared WebGL renderer and links its shader synchronously, which should not land in the same
// frame as the transcript update; the keeper then holds the renderer for every later card.
function MosaicLoader() {
  const host = useRef<HTMLDivElement>(null);
  const theme = useTheme();
  const [ready, setReady] = useState(false);
  const [colors, setColors] = useState<ReturnType<typeof readImageGenColors>>();
  useEffect(() => idle(() => { keepImageGenRenderer(); setReady(true); }), []);
  useLayoutEffect(() => { if (ready && host.current) setColors(readImageGenColors(host.current)); }, [ready, theme]);
  return (
    <div ref={host} className="size-full">
      {colors && (
        // img-fx's own stylesheet is unlayered, so its inline-block root only yields to an important utility
        <ImageGeneration preset={IMAGE_GEN.preset} pixelScale={IMAGE_GEN.pixelScale} theme={theme} cardBg={colors.cardBg} colors={colors.colors}
          className="block! size-full fade-in">
          <div className="size-full" />
        </ImageGeneration>
      )}
    </div>
  );
}

function Dissolve({ img, onDone }: { img: HTMLImageElement; onDone: () => void }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => dissolveIn(ref.current!, img, onDone), [img, onDone]);
  return <canvas ref={ref} className="absolute inset-0 size-full" />;
}
