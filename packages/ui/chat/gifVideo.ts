import type { GifVideoCodec, GifVideoRequest, GifVideoResponse } from './gifVideo.worker';

// Page side of the GIF-to-video worker. One conversion runs at a time and each GIF (by URL) converts once while its
// MP4 is kept as an object URL. A <video> showing a clip holds it (`holdClip` / `releaseClip`); clips nobody holds stay
// for a later mount up to `IDLE_BYTES`, then the least recently released go and their URLs are revoked. Any failure
// resolves undefined and the GIF stays.

export interface GifClip {
  url: string;
  // The GIF's own size, for a placeholder with the GIF's layout
  width: number;
  height: number;
  // 0 loops forever
  plays: number;
}

const clips = new Map<string, Promise<GifClip | undefined>>();
// Each converted clip's MP4 size, the <video>s holding it, and the clips nobody holds, least recently released first
const sizes = new Map<string, number>();
const users = new Map<string, number>();
const idle = new Set<string>();
const IDLE_BYTES = 32 * 1024 * 1024;
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
  sizes.set(src, response.mp4.byteLength);
  // The image that asked may be gone already: then nobody holds the clip from the start
  if (!users.has(src)) { idle.add(src); trim(); }
  return { url, width: response.width, height: response.height, plays: response.plays ?? 0 };
}

// Drop idle clips, oldest first, until the rest fit the budget
function trim() {
  let total = 0;
  for (const src of idle) total += sizes.get(src) ?? 0;
  for (const src of idle) {
    if (total <= IDLE_BYTES) break;
    total -= sizes.get(src) ?? 0;
    idle.delete(src);
    sizes.delete(src);
    const clip = clips.get(src);
    clips.delete(src);
    void clip?.then(c => { if (c) URL.revokeObjectURL(c.url); });
  }
}

// A <video> starts showing `src`'s clip
export function holdClip(src: string) {
  users.set(src, (users.get(src) ?? 0) + 1);
  idle.delete(src);
}

// A <video> stopped showing it
export function releaseClip(src: string) {
  const left = (users.get(src) ?? 1) - 1;
  if (left > 0) { users.set(src, left); return; }
  users.delete(src);
  if (!sizes.has(src)) return;
  idle.delete(src);
  idle.add(src);
  trim();
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
