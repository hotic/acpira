// A soft whoosh under a plugin's arrival: a piece flying in from its card (`piece`), a skin uncovered by its circle
// (`skin`, longer and lower). Synthesized like the chime (effects/chime.ts, same <audio> route and its reasons): noise
// through a band-pass whose centre glides upward, swelling then dying away, at about the chime's level. Only a click on
// the card starts the motion it goes with, so autoplay rules let it through. Off with the attention chime (`notifySound`)
const RATE = 22050;
const PEAK = 0.16;
const SHAPES = {
  piece: { seconds: 0.38, from: 600, to: 3200, q: 2.2, swell: 0.35 },
  skin: { seconds: 0.62, from: 300, to: 1900, q: 1.6, swell: 0.4 },
} as const;
// Several pieces landing from one card are one gesture: one whoosh
const QUIET_MS = 250;

let on = true;
let last = 0;
const clips = new Map<keyof typeof SHAPES, HTMLAudioElement>();

export function setWhooshOn(next: boolean) {
  on = next;
}

export function whooshWav(kind: keyof typeof SHAPES): Blob {
  const shape = SHAPES[kind];
  const length = Math.ceil(shape.seconds * RATE);
  const samples = new Float32Array(length);
  // RBJ band-pass (0 dB peak), its centre recomputed per sample along an exponential glide
  let x1 = 0, x2 = 0, y1 = 0, y2 = 0, peak = 0;
  for (let i = 0; i < length; i++) {
    const p = i / length;
    const f0 = shape.from * Math.pow(shape.to / shape.from, p);
    const w0 = (2 * Math.PI * f0) / RATE;
    const alpha = Math.sin(w0) / (2 * shape.q);
    const a0 = 1 + alpha;
    const x = Math.random() * 2 - 1;
    const y = (alpha * x - alpha * x2 + 2 * Math.cos(w0) * y1 - (1 - alpha) * y2) / a0;
    x2 = x1; x1 = x; y2 = y1; y1 = y;
    // Rises to its swell, then falls away smoothly to silence
    const env = p < shape.swell ? Math.sin((Math.PI / 2) * (p / shape.swell)) ** 2 : Math.cos((Math.PI / 2) * ((p - shape.swell) / (1 - shape.swell))) ** 2;
    samples[i] = y * env;
    peak = Math.max(peak, Math.abs(samples[i]!));
  }
  const gain = peak > 0 ? PEAK / peak : 0;
  const view = new DataView(new ArrayBuffer(44 + length * 2));
  const ascii = (at: number, s: string) => { for (let i = 0; i < s.length; i++) view.setUint8(at + i, s.charCodeAt(i)); };
  ascii(0, 'RIFF'); view.setUint32(4, 36 + length * 2, true); ascii(8, 'WAVE');
  ascii(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, RATE, true); view.setUint32(28, RATE * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  ascii(36, 'data'); view.setUint32(40, length * 2, true);
  samples.forEach((s, i) => view.setInt16(44 + i * 2, Math.max(-1, Math.min(1, s * gain)) * 0x7fff, true));
  return new Blob([view.buffer], { type: 'audio/wav' });
}

export function playWhoosh(kind: keyof typeof SHAPES): void {
  const now = performance.now();
  if (!on || now - last < QUIET_MS) return;
  last = now;
  try {
    let audio = clips.get(kind);
    if (!audio) clips.set(kind, (audio = new Audio(URL.createObjectURL(whooshWav(kind)))));
    audio.currentTime = 0;
    // Refused without a gesture or an output device: a nicety, stay silent
    void audio.play().catch(() => {});
  } catch {
    // No media support at all: same
  }
}
