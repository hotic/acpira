import type { GifVideoCodec, GifVideoRequest, GifVideoResponse } from './gifVideo.worker';

// Page side of the GIF-to-video worker. One conversion runs at a time, each GIF (by URL) converts once per page and
// its MP4 stays as an object URL for the page's life: a clip is a fraction of its GIF's size, and an in-place
// <video> may hold it at any moment, so nothing is revoked. Any failure resolves undefined and the GIF stays.

export interface GifClip {
  url: string;
  // The GIF's own size, for a placeholder with the GIF's layout
  width: number;
  height: number;
  // 0 loops forever
  plays: number;
}

const clips = new Map<string, Promise<GifClip | undefined>>();
const pending = new Map<number, (response: GifVideoResponse) => void>();
let queue: Promise<unknown> = Promise.resolve();
let sequence = 0;
let worker: Promise<Worker | undefined> | undefined;
let codecs: GifVideoCodec[] | undefined;

// Codecs this page's <video> plays inside MP4, in preference order; the worker then asks its encoder
function playableCodecs(): GifVideoCodec[] {
  if (codecs) return codecs;
  const probe = document.createElement('video');
  codecs = [];
  if (probe.canPlayType('video/mp4; codecs="avc1.42E028"')) codecs.push('avc');
  if (probe.canPlayType('video/mp4; codecs="vp09.00.40.08"')) codecs.push('vp9');
  return codecs;
}

// Inline like the highlighting worker, so every host starts it from a blob without a worker module fetch
function start(): Promise<Worker | undefined> {
  return worker ??= import('./gifVideo.worker?worker&inline').then(({ default: GifWorker }) => {
    const w = new GifWorker({ name: 'acpira-gif-video' });
    w.onmessage = ({ data }: MessageEvent<GifVideoResponse>) => {
      pending.get(data.id)?.(data);
      pending.delete(data.id);
    };
    w.onerror = event => {
      event.preventDefault();
      w.terminate();
      worker = Promise.resolve(undefined);
      for (const [id, resolve] of pending) resolve({ id, skipped: 'worker failed' });
      pending.clear();
    };
    return w;
  }).catch(() => undefined);
}

async function convert(src: string): Promise<GifClip | undefined> {
  // WebCodecs on the page stands in for the worker's: no host exposes one without the other
  if (typeof VideoEncoder === 'undefined' || !playableCodecs().length) return undefined;
  const w = await start();
  if (!w) return undefined;
  const bytes = await (await fetch(src)).arrayBuffer();
  const id = ++sequence;
  const response = await new Promise<GifVideoResponse>(resolve => {
    pending.set(id, resolve);
    w.postMessage({ id, bytes, codecs: playableCodecs() } satisfies GifVideoRequest, [bytes]);
  });
  if (!response.mp4 || !response.width || !response.height) {
    if (response.skipped) console.debug(`[acpira] GIF stays a GIF: ${response.skipped}`);
    // A stall (the page went hidden mid-encode) is no verdict on the GIF: the next mount tries again
    if (response.retry) clips.delete(src);
    return undefined;
  }
  const url = URL.createObjectURL(new Blob([response.mp4], { type: 'video/mp4' }));
  return { url, width: response.width, height: response.height, plays: response.plays ?? 0 };
}

export function gifClip(src: string): Promise<GifClip | undefined> {
  let clip = clips.get(src);
  if (!clip) {
    clip = queue.then(() => convert(src)).catch(() => undefined);
    queue = clip;
    clips.set(src, clip);
  }
  return clip;
}
