import { useEffect, useRef, useState } from 'react';
import { cn } from '../ui/cn';
import { gifClip, holdClip, releaseClip, type GifClip } from './gifVideo';

// An <img> that plays an animated GIF as a muted looping <video> once it comes into view: WebKit decodes every GIF
// frame on the CPU at full size (a quarter of a core for one 1280 px GIF), the video goes through the hardware
// decoder. The <img> keeps the layout: it swaps to an empty SVG of the GIF's size once the video plays, and the video
// covers it, so the parent must be the positioning box around exactly the image (`relative`, no padding).
// Anything else, or any failure on the way, renders the plain <img>.
export function AnimatedImage({ src, mimeType, alt, className, fit = 'contain' }: {
  src: string;
  mimeType?: string;
  alt: string;
  className?: string;
  fit?: 'contain' | 'cover';
}) {
  const gif = mimeType === 'image/gif';
  const image = useRef<HTMLImageElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const [clip, setClip] = useState<GifClip>();
  const [playing, setPlaying] = useState(false);

  // Convert once the GIF is near the viewport and the page is visible (WebKit stops encoding in a hidden page)
  useEffect(() => {
    setClip(undefined);
    setPlaying(false);
    const el = image.current;
    if (!gif || !el || typeof IntersectionObserver === 'undefined') return;
    let alive = true;
    const observer = new IntersectionObserver(([entry]) => {
      if (!entry?.isIntersecting || document.visibilityState !== 'visible') return;
      observer.disconnect();
      void gifClip(src).then(c => { if (alive && c) setClip(c); });
    }, { rootMargin: '200px' });
    observer.observe(el);
    return () => {
      alive = false;
      observer.disconnect();
    };
  }, [gif, src]);

  // The clip stays converted while this image shows it
  useEffect(() => {
    if (!clip) return;
    holdClip(src);
    return () => releaseClip(src);
  }, [clip, src]);

  // Play while on screen, pause off view or in a hidden page; a refused play() (autoplay policy) keeps the GIF
  useEffect(() => {
    const el = video.current;
    if (!clip || !el) return;
    let visible = false;
    let plays = 0;
    const drop = () => {
      setClip(undefined);
      setPlaying(false);
    };
    const sync = () => {
      if (!visible || document.visibilityState !== 'visible') el.pause();
      // An AbortError only means a pause() overtook the play()
      else el.play().catch((error: unknown) => { if (error instanceof DOMException && error.name !== 'AbortError') drop(); });
    };
    const observer = new IntersectionObserver(([entry]) => {
      visible = !!entry?.isIntersecting;
      sync();
    });
    // A GIF that loops a set number of times: replay until its count is spent, then rest on the last frame like the GIF
    const ended = () => {
      if (clip.plays > 1 && ++plays < clip.plays) {
        el.currentTime = 0;
        sync();
      }
    };
    observer.observe(el);
    document.addEventListener('visibilitychange', sync);
    el.addEventListener('ended', ended);
    el.addEventListener('error', drop);
    return () => {
      el.removeEventListener('error', drop);
      observer.disconnect();
      document.removeEventListener('visibilitychange', sync);
      el.removeEventListener('ended', ended);
      el.pause();
    };
  }, [clip]);

  const placeholder = clip && playing
    ? `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${clip.width}" height="${clip.height}"/>`)}`
    : undefined;
  return (
    <>
      {/* A screenshot shows the GIF itself in place of the empty stand-in and leaves the player out (shareCapture.ts) */}
      <img ref={image} src={placeholder ?? src} data-share-src={placeholder ? src : undefined} alt={alt} className={className} />
      {clip && (
        <video
          ref={video}
          src={clip.url}
          muted
          playsInline
          loop={clip.plays === 0}
          disablePictureInPicture
          aria-hidden
          data-share-skip
          // Shown over the GIF only once a frame plays, so the swap never flashes an empty box
          onPlaying={() => setPlaying(true)}
          className={cn('pointer-events-none absolute inset-0 size-full', fit === 'cover' ? 'object-cover' : 'object-contain', !playing && 'opacity-0')}
        />
      )}
    </>
  );
}
