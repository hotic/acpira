import { describe, expect, it } from 'vitest';
import { readGif, renderGif } from './gifDecode';
import { muxMp4 } from './mp4Mux';

// A GIF writer for tests: 2-bit colours, LZW kept uncompressed by a clear code every two pixels, so the code size never grows
function gif(width: number, height: number, palette: number[][], frames: {
  pixels: number[]; delay?: number; transparent?: number; disposal?: number; rect?: [number, number, number, number]; interlaced?: boolean;
}[], loop?: number): Uint8Array {
  const out: number[] = [...'GIF89a'].map(c => c.charCodeAt(0));
  const u16 = (n: number) => [n & 255, n >> 8];
  out.push(...u16(width), ...u16(height), 0x81, 0, 0);
  for (let i = 0; i < 4; i++) out.push(...(palette[i] ?? [0, 0, 0]));
  if (loop !== undefined) out.push(0x21, 0xff, 11, ...[...'NETSCAPE2.0'].map(c => c.charCodeAt(0)), 3, 1, ...u16(loop), 0);
  for (const f of frames) {
    const [left, top, w, h] = f.rect ?? [0, 0, width, height];
    out.push(0x21, 0xf9, 4, ((f.disposal ?? 0) << 2) | (f.transparent !== undefined ? 1 : 0), ...u16(f.delay ?? 10), f.transparent ?? 0, 0);
    out.push(0x2c, ...u16(left), ...u16(top), ...u16(w), ...u16(h), f.interlaced ? 0x40 : 0, 2);
    const codes: number[] = [];
    f.pixels.forEach((p, i) => { if (i % 2 === 0) codes.push(4); codes.push(p); });
    codes.push(5);
    const bytes: number[] = [];
    let acc = 0, bits = 0;
    for (const c of codes) {
      acc |= c << bits; bits += 3;
      while (bits >= 8) { bytes.push(acc & 255); acc >>= 8; bits -= 8; }
    }
    if (bits) bytes.push(acc & 255);
    out.push(bytes.length, ...bytes, 0);
  }
  out.push(0x3b);
  return new Uint8Array(out);
}

async function frames(bytes: Uint8Array) {
  const parsed = readGif(bytes);
  const seen: number[][] = [];
  await renderGif(bytes, parsed, canvas => {
    // One number per pixel: its red channel, or -1 where transparent
    const row: number[] = [];
    for (let i = 0; i < canvas.length; i += 4) row.push(canvas[i + 3] ? canvas[i]! : -1);
    seen.push(row);
    return true;
  });
  return { parsed, seen };
}

const PALETTE = [[10, 0, 0], [20, 0, 0], [30, 0, 0], [40, 0, 0]];

describe('GIF decoding', () => {
  it('reads delays the way browsers play them and the loop count', async () => {
    const { parsed } = await frames(gif(2, 1, PALETTE, [{ pixels: [0, 1], delay: 0 }, { pixels: [1, 0], delay: 7 }], 0));
    expect(parsed.frames.map(f => f.delayMs)).toEqual([100, 70]);
    expect(parsed.loopCount).toBe(0);
    expect(readGif(gif(2, 1, PALETTE, [{ pixels: [0, 1] }])).loopCount).toBeUndefined();
  });

  it('composites transparency and disposal like a browser', async () => {
    const { seen } = await frames(gif(2, 2, PALETTE, [
      { pixels: [0, 1, 2, 3] },
      // Transparent index 3 keeps what is below; disposal 3 restores the canvas from before this frame
      { pixels: [3, 2, 1, 3], transparent: 3, disposal: 3 },
      // Disposal 2 clears its own rect to transparent afterwards
      { pixels: [3], rect: [1, 1, 1, 1], disposal: 2 },
      { pixels: [0], rect: [0, 0, 1, 1] },
    ]));
    expect(seen).toEqual([
      [10, 20, 30, 40],
      [10, 30, 20, 40],
      [10, 20, 30, 40],
      [10, 20, 30, -1],
    ]);
  });

  it('puts interlaced rows back in order', async () => {
    // Pass order for 4 rows: 0, then 2, then 1, 3
    const { seen } = await frames(gif(1, 4, PALETTE, [{ pixels: [0, 2, 1, 3], interlaced: true }]));
    expect(seen[0]).toEqual([10, 20, 30, 40]);
  });
});

describe('MP4 muxing', () => {
  it('writes boxes whose sizes nest and a chunk offset that lands on the first sample', () => {
    const samples = [new Uint8Array([1, 2, 3]), new Uint8Array([4, 5])];
    const mp4 = muxMp4({
      format: 'avc1', config: new Uint8Array([1, 66, 0, 40]), width: 4, height: 2, timescale: 1000,
      samples: samples.map((data, i) => ({ data, duration: 100, key: i === 0 })),
    });
    const view = new DataView(mp4.buffer);
    const top: string[] = [];
    for (let at = 0; at < mp4.length;) {
      top.push(String.fromCharCode(...mp4.subarray(at + 4, at + 8)));
      at += view.getUint32(at);
    }
    expect(top).toEqual(['ftyp', 'moov', 'mdat']);
    const stco = mp4.findIndex((_, i) => String.fromCharCode(...mp4.subarray(i, i + 4)) === 'stco');
    const offset = view.getUint32(stco + 12);
    expect([...mp4.subarray(offset, offset + 5)]).toEqual([1, 2, 3, 4, 5]);
  });
});
