/// <reference lib="webworker" />
import { readGif, renderGif } from './gifDecode';
import { muxMp4, type Mp4Sample } from './mp4Mux';

// Re-encodes an animated GIF as an MP4 the page plays in a <video>: WebKit decodes every GIF frame on the CPU, a video
// goes through the hardware decoder. Answers `mp4` undefined (with a reason) whenever the GIF should stay as it is.

export type GifVideoCodec = 'avc' | 'vp9';
export interface GifVideoRequest { id: number; bytes: ArrayBuffer; codecs: GifVideoCodec[] }
export interface GifVideoResponse {
  id: number;
  mp4?: ArrayBuffer;
  width?: number;
  height?: number;
  // 0 loops forever, else the number of plays
  plays?: number;
  skipped?: string;
  // The skip came from the page's state (hidden), not from the GIF: a later view may try again
  retry?: boolean;
}

// Decode work bound: frames x pixels. Above it the GIF keeps playing as a GIF
const MAX_WORK = 1_500_000_000;
// H.264 / VP9 level 4.0 frame size bound, with headroom
const MAX_AREA = 2_000_000;
const MAX_SIDE = 1920;
const TIMESCALE = 1000;
const STALL_MS = 3000;

// Constrained Baseline only: it has no B-frames, and the muxer writes no composition offsets
const CODECS: Record<GifVideoCodec, { strings: string[]; format: 'avc1' | 'vp09' }> = {
  avc: { strings: ['avc1.42E028'], format: 'avc1' },
  vp9: { strings: ['vp09.00.40.08'], format: 'vp09' },
};
// vpcC after version / flags: profile 0, level 4.0, 8-bit 4:2:0 limited range, BT.709 primaries / transfer / matrix
const VPCC = new Uint8Array([0, 40, 0x82, 1, 1, 1, 0, 0]);

// WebCodecs colour space names to ISO/IEC 23091-2 code points; 2 = unspecified
const TRANSFER: Record<string, number> = { bt709: 1, smpte170m: 6, linear: 8, 'iec61966-2-1': 13, pq: 16, hlg: 18 };
const MATRIX: Record<string, number> = { rgb: 0, bt709: 1, bt470bg: 5, smpte170m: 6, 'bt2020-ncl': 9 };

// Primaries stay unspecified: WebKit labelled bt709 + sRGB transfer converts gamut and skews hues (a SMPTE bar GIF came
// back with green 0 -> 46 in magenta), while unspecified primaries play within 1 / 255 of the GIF
function colourCodes(space: VideoColorSpaceInit | undefined) {
  if (!space?.matrix) return undefined;
  return {
    primaries: 2,
    transfer: TRANSFER[space.transfer ?? ''] ?? 2,
    matrix: MATRIX[space.matrix] ?? 2,
    fullRange: space.fullRange ?? false,
  };
}

const even = (n: number) => Math.max(2, Math.round(n / 2) * 2);

// Area-average resample onto the encoder's even-sided frame; also covers the 1 px pad of an odd side
function resample(src: Uint8ClampedArray, w: number, h: number, out: Uint8ClampedArray, dw: number, dh: number): void {
  const sx = w / dw;
  const sy = h / dh;
  for (let y = 0; y < dh; y++) {
    const y0 = Math.min(h - 1, Math.floor(y * sy));
    const y1 = Math.min(h, Math.max(y0 + 1, Math.floor((y + 1) * sy)));
    for (let x = 0; x < dw; x++) {
      const x0 = Math.min(w - 1, Math.floor(x * sx));
      const x1 = Math.min(w, Math.max(x0 + 1, Math.floor((x + 1) * sx)));
      let r = 0, g = 0, b = 0, n = 0;
      for (let yy = y0; yy < y1; yy++) {
        let at = (yy * w + x0) * 4;
        for (let xx = x0; xx < x1; xx++, at += 4) {
          r += src[at]!; g += src[at + 1]!; b += src[at + 2]!; n++;
        }
      }
      const o = (y * dw + x) * 4;
      out[o] = r / n; out[o + 1] = g / n; out[o + 2] = b / n; out[o + 3] = 255;
    }
  }
}

async function pickCodec(codecs: GifVideoCodec[], width: number, height: number, bitrate: number, framerate: number) {
  for (const codec of codecs) {
    for (const string of CODECS[codec].strings) {
      const config: VideoEncoderConfig = { codec: string, width, height, bitrate, framerate, latencyMode: 'quality' };
      if (codec === 'avc') config.avc = { format: 'avc' };
      try {
        if ((await VideoEncoder.isConfigSupported(config)).supported) return { config, format: CODECS[codec].format };
      } catch { /* try the next one */ }
    }
  }
  return undefined;
}

async function convert({ id, bytes, codecs }: GifVideoRequest): Promise<GifVideoResponse> {
  if (typeof VideoEncoder === 'undefined' || typeof VideoFrame === 'undefined') return { id, skipped: 'no WebCodecs' };
  const data = new Uint8Array(bytes);
  const gif = readGif(data);
  const { width: W, height: H, frames } = gif;
  if (frames.length < 2) return { id, skipped: 'still' };
  if (!W || !H || W * H * frames.length > MAX_WORK) return { id, skipped: 'too large' };
  // Each frame decodes into a buffer of its own declared size, which a crafted file can set far past the canvas
  if (frames.some(f => f.width * f.height > W * H) || frames.reduce((n, f) => n + f.width * f.height, 0) > MAX_WORK) return { id, skipped: 'too large' };
  const scale = Math.min(1, MAX_SIDE / Math.max(W, H), Math.sqrt(MAX_AREA / (W * H)));
  const dw = even(W * scale);
  const dh = even(H * scale);
  const totalMs = frames.reduce((sum, f) => sum + f.delayMs, 0);
  const framerate = Math.max(1, Math.min(60, (frames.length * 1000) / totalMs));
  const bitrate = Math.round(Math.min(12e6, Math.max(1e6, dw * dh * framerate * 0.15)));
  const picked = await pickCodec(codecs, dw, dh, bitrate, framerate);
  if (!picked) return { id, skipped: 'no encoder' };

  const samples: Mp4Sample[] = [];
  let description: Uint8Array | undefined;
  let colour: VideoColorSpaceInit | undefined;
  let failure: unknown;
  let reordered = false;
  let lastOut = -1;
  let progress = performance.now();
  // WebKit stops encoding in a page that is not visible; an encoder without output for a while gives up
  const waitForEncoder = async () => {
    await new Promise(resolve => setTimeout(resolve, 2));
    if (performance.now() - progress > STALL_MS) failure ??= 'stalled';
  };
  const encoder = new VideoEncoder({
    output: (chunk, meta) => {
      colour ??= meta?.decoderConfig?.colorSpace;
      const config = meta?.decoderConfig?.description;
      if (config && !description) description = new Uint8Array(ArrayBuffer.isView(config) ? config.buffer.slice(config.byteOffset, config.byteOffset + config.byteLength) : config.slice(0));
      const sample = new Uint8Array(chunk.byteLength);
      chunk.copyTo(sample);
      // Durations are filled in input order after the flush, so output must keep it
      if (chunk.timestamp < lastOut) reordered = true;
      lastOut = chunk.timestamp;
      samples.push({ data: sample, duration: 0, key: chunk.type === 'key' });
      progress = performance.now();
    },
    error: error => { failure = error; },
  });
  encoder.configure(picked.config);
  const resized = dw === W && dh === H ? undefined : new Uint8ClampedArray(dw * dh * 4);
  let transparent = false;
  let timestamp = 0;
  // A keyframe every ~2 s keeps seeking on loop cheap without bloating the file
  const keyEvery = Math.max(1, Math.round(framerate * 2));
  await renderGif(data, gif, async (canvas, index) => {
    // A video has no alpha: a GIF that shows any transparent pixel keeps playing as a GIF
    for (let i = 3; i < canvas.length; i += 4) if (canvas[i] !== 255) { transparent = true; return false; }
    let pixels = canvas;
    if (resized) {
      resample(canvas, W, H, resized, dw, dh);
      pixels = resized;
    }
    const delay = frames[index]!.delayMs;
    const frame = new VideoFrame(pixels, { format: 'RGBA', codedWidth: dw, codedHeight: dh, timestamp: timestamp * 1000, duration: delay * 1000 });
    timestamp += delay;
    encoder.encode(frame, { keyFrame: index % keyEvery === 0 });
    frame.close();
    // Backpressure: frames hold raw pixels until the encoder takes them
    while (encoder.encodeQueueSize > 4 && !failure) await waitForEncoder();
    return !failure;
  });
  if (transparent) {
    encoder.close();
    return { id, skipped: 'transparent' };
  }
  // WebKit's flush() sometimes never settles after the last output, so the last sample arriving ends the wait as well
  if (!failure) {
    let flushed = false;
    encoder.flush().then(() => { flushed = true; }, error => { failure = error; });
    while (!flushed && !failure && samples.length < frames.length) await waitForEncoder();
  }
  if (encoder.state !== 'closed') encoder.close();
  if (failure) return { id, skipped: `encoder: ${String(failure)}`, retry: failure === 'stalled' };
  if (reordered || samples.length !== frames.length || !samples[0]?.key) return { id, skipped: 'encoder output mismatch' };
  if (picked.format === 'avc1' && !description) return { id, skipped: 'no avcC' };
  samples.forEach((s, i) => { s.duration = frames[i]!.delayMs * (TIMESCALE / 1000); });
  const mp4 = muxMp4({ format: picked.format, config: picked.format === 'avc1' ? description! : VPCC, width: dw, height: dh, timescale: TIMESCALE, samples, colour: colourCodes(colour) });
  const plays = gif.loopCount === undefined ? 1 : gif.loopCount === 0 ? 0 : gif.loopCount + 1;
  return { id, mp4: mp4.buffer as ArrayBuffer, width: W, height: H, plays };
}

// One GIF at a time: the page queues requests, so the encoder never competes with itself
self.onmessage = async ({ data }: MessageEvent<GifVideoRequest>) => {
  let response: GifVideoResponse;
  try {
    response = await convert(data);
  } catch (error) {
    response = { id: data.id, skipped: String(error) };
  }
  if (response.mp4) (self as DedicatedWorkerGlobalScope).postMessage(response, [response.mp4]);
  else (self as DedicatedWorkerGlobalScope).postMessage(response);
};
