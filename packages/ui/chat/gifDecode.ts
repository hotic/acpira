// A GIF decoder for the GIF-to-video worker (`gifVideo.worker.ts`). WebKit has no ImageDecoder, so frames are decoded
// here: `readGif` indexes the file without decompressing, `renderGif` composites every frame onto one RGBA canvas the
// way browsers play it (disposal methods, transparency, interlacing).

export interface GifFrameRecord {
  left: number;
  top: number;
  width: number;
  height: number;
  interlaced: boolean;
  palette: Uint8Array;
  // -1 when the frame has no transparent index
  transparent: number;
  disposal: number;
  delayMs: number;
  minCodeSize: number;
  // [start, end) byte ranges of the LZW data sub-blocks
  blocks: number[];
}

export interface Gif {
  width: number;
  height: number;
  // 0 loops forever; undefined means no NETSCAPE block, which browsers play once
  loopCount?: number;
  frames: GifFrameRecord[];
}

// Browsers stretch near-zero delays to 100 ms; the video must keep the speed the GIF showed
function playedDelay(centiseconds: number): number {
  return centiseconds <= 1 ? 100 : centiseconds * 10;
}

export function readGif(bytes: Uint8Array): Gif {
  const ascii = (at: number, n: number) => String.fromCharCode(...bytes.subarray(at, at + n));
  if (bytes.length < 13 || ascii(0, 3) !== 'GIF') throw new Error('not a GIF');
  const u16 = (at: number) => bytes[at]! | (bytes[at + 1]! << 8);
  const width = u16(6);
  const height = u16(8);
  const flags = bytes[10]!;
  let pos = 13;
  let globalPalette: Uint8Array = new Uint8Array(0);
  if (flags & 0x80) {
    const size = 3 * (1 << ((flags & 7) + 1));
    globalPalette = bytes.subarray(pos, pos + size);
    pos += size;
  }
  const gif: Gif = { width, height, frames: [] };
  // The graphic control extension applies to the next image only
  let control = { transparent: -1, disposal: 0, delayMs: 100 };
  const skipBlocks = () => {
    while (pos < bytes.length) {
      const n = bytes[pos++]!;
      if (n === 0) return;
      pos += n;
    }
  };
  while (pos < bytes.length) {
    const kind = bytes[pos++];
    if (kind === 0x3b) break;
    if (kind === 0x21) {
      const label = bytes[pos++];
      if (label === 0xf9 && bytes[pos] === 4) {
        const packed = bytes[pos + 1]!;
        control = {
          disposal: (packed >> 2) & 7,
          delayMs: playedDelay(u16(pos + 2)),
          transparent: packed & 1 ? bytes[pos + 4]! : -1,
        };
        pos += 5;
      } else if (label === 0xff && bytes[pos] === 11 && ascii(pos + 1, 11) === 'NETSCAPE2.0') {
        pos += 12;
        if (bytes[pos] === 3 && bytes[pos + 1] === 1) gif.loopCount = u16(pos + 2);
      }
      // Whatever sub-blocks are left, up to the terminator
      skipBlocks();
      continue;
    }
    if (kind !== 0x2c) throw new Error('corrupt GIF');
    const packed = bytes[pos + 8]!;
    const frame: GifFrameRecord = {
      left: u16(pos), top: u16(pos + 2), width: u16(pos + 4), height: u16(pos + 6),
      interlaced: (packed & 0x40) !== 0,
      palette: globalPalette,
      ...control,
      minCodeSize: 0,
      blocks: [],
    };
    pos += 9;
    if (packed & 0x80) {
      const size = 3 * (1 << ((packed & 7) + 1));
      frame.palette = bytes.subarray(pos, pos + size);
      pos += size;
    }
    frame.minCodeSize = bytes[pos++]!;
    while (pos < bytes.length) {
      const n = bytes[pos++]!;
      if (n === 0) break;
      frame.blocks.push(pos, Math.min(pos + n, bytes.length));
      pos += n;
    }
    gif.frames.push(frame);
    control = { transparent: -1, disposal: 0, delayMs: 100 };
  }
  return gif;
}

// LZW-decodes one frame's colour indices; a truncated stream leaves the rest at index 0, as browsers do
function decodeIndices(bytes: Uint8Array, frame: GifFrameRecord, out: Uint8Array): void {
  const count = frame.width * frame.height;
  out.fill(0, 0, count);
  const min = frame.minCodeSize;
  if (min < 2 || min > 11) return;
  const clear = 1 << min;
  const end = clear + 1;
  const prefix = new Int16Array(4096);
  const suffix = new Uint8Array(4096);
  const stack = new Uint8Array(4097);
  for (let i = 0; i < clear; i++) suffix[i] = i;
  let size = min + 1;
  let mask = (1 << size) - 1;
  let next = clear + 2;
  let old = -1;
  let first = 0;
  let datum = 0;
  let bits = 0;
  let written = 0;
  for (let b = 0; b < frame.blocks.length; b += 2) {
    for (let p = frame.blocks[b]!; p < frame.blocks[b + 1]!; p++) {
      datum |= bytes[p]! << bits;
      bits += 8;
      while (bits >= size) {
        let code = datum & mask;
        datum >>>= size;
        bits -= size;
        if (code === clear) {
          size = min + 1;
          mask = (1 << size) - 1;
          next = clear + 2;
          old = -1;
          continue;
        }
        if (code === end) return;
        if (old === -1) {
          if (code >= clear) return;
          if (written < count) out[written++] = code;
          old = first = code;
          continue;
        }
        const incoming = code;
        let top = 0;
        if (code >= next) {
          if (code > next) return;
          stack[top++] = first;
          code = old;
        }
        while (code >= clear) {
          stack[top++] = suffix[code]!;
          code = prefix[code]!;
        }
        first = suffix[code]!;
        stack[top++] = first;
        if (next < 4096) {
          prefix[next] = old;
          suffix[next] = first;
          next++;
          if ((next & mask) === 0 && next < 4096) {
            size++;
            mask += next;
          }
        }
        old = incoming;
        while (top > 0 && written < count) out[written++] = stack[--top]!;
        if (written >= count) return;
      }
    }
  }
}

// Row order of an interlaced frame: every 8th from 0, every 8th from 4, every 4th from 2, every 2nd from 1
function rowOrder(height: number, interlaced: boolean): number[] {
  const rows: number[] = [];
  if (!interlaced) {
    for (let y = 0; y < height; y++) rows.push(y);
    return rows;
  }
  for (const [start, step] of [[0, 8], [4, 8], [2, 4], [1, 2]] as const) for (let y = start; y < height; y += step) rows.push(y);
  return rows;
}

// Composites each frame onto `canvas` (width * height * 4 RGBA) and hands it to `onFrame` before the next one draws;
// the callback must copy what it keeps. `false` from the callback stops early.
export async function renderGif(bytes: Uint8Array, gif: Gif, onFrame: (canvas: Uint8ClampedArray, index: number) => boolean | Promise<boolean>): Promise<void> {
  const { width: W, height: H } = gif;
  const canvas = new Uint8ClampedArray(W * H * 4);
  let indices = new Uint8Array(0);
  let saved: Uint8ClampedArray | undefined;
  let previous: GifFrameRecord | undefined;
  for (let i = 0; i < gif.frames.length; i++) {
    const frame = gif.frames[i]!;
    // Undo the previous frame per its disposal method
    if (previous?.disposal === 2) {
      for (let y = previous.top; y < Math.min(H, previous.top + previous.height); y++) {
        const from = (y * W + previous.left) * 4;
        canvas.fill(0, from, from + Math.max(0, Math.min(W - previous.left, previous.width)) * 4);
      }
    } else if (previous?.disposal === 3 && saved) {
      canvas.set(saved);
    }
    if (frame.disposal === 3) saved = canvas.slice();
    if (indices.length < frame.width * frame.height) indices = new Uint8Array(frame.width * frame.height);
    decodeIndices(bytes, frame, indices);
    const palette = frame.palette;
    const colours = palette.length / 3;
    const rows = rowOrder(frame.height, frame.interlaced);
    for (let r = 0; r < rows.length; r++) {
      const y = frame.top + rows[r]!;
      if (y >= H) continue;
      const src = r * frame.width;
      for (let x = 0; x < frame.width; x++) {
        const cx = frame.left + x;
        if (cx >= W) break;
        const index = indices[src + x]!;
        if (index === frame.transparent || index >= colours) continue;
        const at = (y * W + cx) * 4;
        canvas[at] = palette[index * 3]!;
        canvas[at + 1] = palette[index * 3 + 1]!;
        canvas[at + 2] = palette[index * 3 + 2]!;
        canvas[at + 3] = 255;
      }
    }
    if (!(await onFrame(canvas, i))) return;
    previous = frame;
  }
}
