// The smallest MP4 a <video> plays: one video track, one chunk, `moov` ahead of `mdat`. Written for the GIF-to-video
// worker (`gifVideo.worker.ts`), whose WebCodecs encoder hands over finished samples but no container.

export interface Mp4Sample {
  data: Uint8Array;
  // In `timescale` units
  duration: number;
  key: boolean;
}

export interface Mp4Track {
  // Sample entry type: H.264 ('avc1', `config` = the encoder's AVCDecoderConfigurationRecord) or VP9 ('vp09', `config`
  // = the vpcC payload after its version / flags)
  format: 'avc1' | 'vp09';
  config: Uint8Array;
  width: number;
  height: number;
  timescale: number;
  samples: Mp4Sample[];
  // ISO/IEC 23091-2 codes for a `colr` nclx box. Without one players guess limited-range BT.601, while WebKit's encoder
  // writes full-range BT.709 and leaves the bitstream unlabelled, which shifts every colour
  colour?: { primaries: number; transfer: number; matrix: number; fullRange: boolean };
}

type Part = Uint8Array | number[];

function concat(parts: Part[]): Uint8Array {
  let size = 0;
  for (const p of parts) size += p.length;
  const out = new Uint8Array(size);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

const u8 = (n: number) => [n & 255];
const u16 = (n: number) => [(n >>> 8) & 255, n & 255];
const u32 = (n: number) => [(n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255];
const fourcc = (s: string) => [...s].map(c => c.charCodeAt(0));
const zeros = (n: number) => new Array<number>(n).fill(0);
const matrix = [...u32(0x10000), ...u32(0), ...u32(0), ...u32(0), ...u32(0x10000), ...u32(0), ...u32(0), ...u32(0), ...u32(0x40000000)];

function box(type: string, ...parts: Part[]): Uint8Array {
  const body = concat(parts);
  return concat([u32(body.length + 8), fourcc(type), body]);
}

// A full box: version 0 plus 24-bit flags ahead of the body
function full(type: string, flags: number, ...parts: Part[]): Uint8Array {
  return box(type, u32(flags & 0xffffff), ...parts);
}

function moov(track: Mp4Track, duration: number, mdatData: number): Uint8Array {
  const { samples, width, height, timescale } = track;
  // Runs of equal durations
  const runs: [number, number][] = [];
  for (const s of samples) {
    const last = runs[runs.length - 1];
    if (last && last[1] === s.duration) last[0]++;
    else runs.push([1, s.duration]);
  }
  const keys = samples.flatMap((s, i) => (s.key ? [i + 1] : []));
  const entry = box(track.format,
    zeros(6), u16(1), zeros(16), u16(width), u16(height), u32(0x480000), u32(0x480000), u32(0), u16(1), zeros(32),
    u16(0x18), u16(0xffff),
    // vpcC is a version 1 full box
    track.format === 'avc1' ? box('avcC', track.config) : box('vpcC', u32(0x1000000), track.config),
    track.colour ? box('colr', fourcc('nclx'), u16(track.colour.primaries), u16(track.colour.transfer), u16(track.colour.matrix), u8(track.colour.fullRange ? 0x80 : 0)) : []);
  const stbl = box('stbl',
    full('stsd', 0, u32(1), entry),
    full('stts', 0, u32(runs.length), ...runs.flatMap(([n, d]) => [u32(n), u32(d)])),
    full('stss', 0, u32(keys.length), ...keys.map(u32)),
    full('stsc', 0, u32(1), u32(1), u32(samples.length), u32(1)),
    full('stsz', 0, u32(0), u32(samples.length), ...samples.map(s => u32(s.data.length))),
    full('stco', 0, u32(1), u32(mdatData)));
  const minf = box('minf',
    full('vmhd', 1, u16(0), zeros(6)),
    box('dinf', full('dref', 0, u32(1), full('url ', 1))),
    stbl);
  const mdia = box('mdia',
    full('mdhd', 0, u32(0), u32(0), u32(timescale), u32(duration), u16(0x55c4), u16(0)),
    full('hdlr', 0, u32(0), fourcc('vide'), zeros(12), fourcc('VideoHandler'), u8(0)),
    minf);
  const trak = box('trak',
    full('tkhd', 3, u32(0), u32(0), u32(1), u32(0), u32(duration), zeros(8), u16(0), u16(0), u16(0), u16(0), matrix, u32(width << 16), u32(height << 16)),
    mdia);
  return box('moov',
    full('mvhd', 0, u32(0), u32(0), u32(timescale), u32(duration), u32(0x10000), u16(0x100), zeros(10), matrix, zeros(24), u32(2)),
    trak);
}

export function muxMp4(track: Mp4Track): Uint8Array {
  const duration = track.samples.reduce((sum, s) => sum + s.duration, 0);
  const ftyp = box('ftyp', fourcc('isom'), u32(0x200), fourcc('isom'), fourcc('iso2'), fourcc(track.format), fourcc('mp41'));
  // `moov` has the same size whatever offset it records, so the first pass only measures it
  const size = moov(track, duration, 0).length;
  const head = moov(track, duration, ftyp.length + size + 8);
  const mdatSize = track.samples.reduce((sum, s) => sum + s.data.length, 8);
  return concat([ftyp, head, u32(mdatSize), fourcc('mdat'), ...track.samples.map(s => s.data)]);
}
