import { memo, useContext, useEffect, useRef, useState } from 'react';
import type { ImageRef, ToolCallBlock } from '@acpira/shared/transcript';
import { toolVideos } from '@acpira/shared/imageTools';
import { t, useLocale } from '../i18n';
import { Button } from '../ui/Button';
import { BlobUrlContext, OpenBlobContext } from './fileLinks';
import { useShownBlob } from './stagedBlob';

// Media bytes never enter React state. Only a visible recording gets a source; the browser owns decoding and seeking.
export const ShownVideos = memo(function ShownVideos({ block }: { block: ToolCallBlock }) {
  useLocale();
  const videos = toolVideos(block);
  if (!videos.length) return null;
  return <div className="flex min-w-0 flex-col gap-gap">{videos.map(video => <AgentVideo key={video.blob ?? video.uri} video={video} />)}</div>;
});

function AgentVideo({ video }: { video: ImageRef }) {
  const blobUrl = useContext(BlobUrlContext);
  const openBlob = useContext(OpenBlobContext);
  const player = useRef<HTMLVideoElement>(null);
  const [ready, setReady] = useState(false);
  const [failed, setFailed] = useState(false);
  const name = video.uri?.split(/[\\/]/).pop() ?? video.blob ?? '';
  // A recording kept as a path (a session on an SSH host) is read by the host only once the player comes into view
  const blob = useShownBlob(video.blob, ready ? video.uri : undefined);
  const src = blob && blobUrl ? blobUrl(blob) : undefined;
  const missing = !blobUrl || ((!!video.blob || ready) && blob === null);

  useEffect(() => {
    const el = player.current;
    if (!el) return;
    const observer = new IntersectionObserver(([entry]) => {
      if (entry?.isIntersecting) setReady(true);
      else el.pause();
    });
    observer.observe(el);
    const hidden = () => { if (document.hidden) el.pause(); };
    document.addEventListener('visibilitychange', hidden);
    return () => {
      observer.disconnect();
      document.removeEventListener('visibilitychange', hidden);
      el.pause();
      // Only a player that left the page lets go of its media: on a new `src` (a host recording read after mount) React
      // has already set it, and dropping it here left the player without a source
      if (!el.isConnected) {
        el.removeAttribute('src');
        el.load();
      }
    };
  }, [src, failed]);

  if (missing || failed) return <div role="status" className="flex flex-col items-start gap-gap text-3 text-fg-3">
    <span>{t('panel.error.media')}</span>
    {video.blob && openBlob && <Button onClick={() => openBlob(video.blob!)}>{name}</Button>}
  </div>;

  return <video ref={player} src={ready ? src : undefined} controls playsInline preload="metadata" aria-label={name}
    className="block aspect-video max-h-agent-image w-full rounded-md border border-line bg-code object-contain"
    onError={() => setFailed(true)}
    onLoadedMetadata={e => {
      // WebKit needs a seek to paint a paused first frame; this does not start playback.
      if (!e.currentTarget.currentTime) e.currentTarget.currentTime = 0.001;
    }} />;
}
