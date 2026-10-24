// The attention chime (setting `notifySound`): two soft sine notes synthesized into a small WAV, so no audio file ships.
// It plays through an <audio> element, not an AudioContext: constructing the first AudioContext queries the output device
// synchronously on the page's main thread, which froze the settings page on the first switch-on (briefly on macOS, for
// good on a Windows machine with many audio endpoints). A media element opens its output off the main thread.
// Autoplay rules match the old context: play() is refused until the page has had a user gesture, and switching the
// setting on plays a preview from the click, which is one
const NOTES = [880, 1318.5];
const GAP = 0.12;
const PEAK = 0.16;
const DECAY = 0.6;
const RATE = 22050;

let audio: HTMLAudioElement | undefined;

// Mono 16-bit PCM: each note ramps up over 15 ms, then decays exponentially to silence (the old gain envelope)
function chimeWav(): Blob {
  const attack = 0.015;
  const length = Math.ceil((GAP * (NOTES.length - 1) + DECAY + 0.05) * RATE);
  const samples = new Float32Array(length);
  NOTES.forEach((freq, n) => {
    const offset = Math.round(n * GAP * RATE);
    const end = Math.min(length, offset + Math.ceil((DECAY + 0.05) * RATE));
    for (let i = offset; i < end; i++) {
      const t = (i - offset) / RATE;
      const env = t < attack ? PEAK * (t / attack) : t < DECAY ? PEAK * Math.pow(0.0001 / PEAK, (t - attack) / (DECAY - attack)) : 0;
      samples[i] = (samples[i] ?? 0) + env * Math.sin(2 * Math.PI * freq * t);
    }
  });
  const view = new DataView(new ArrayBuffer(44 + length * 2));
  const ascii = (at: number, s: string) => { for (let i = 0; i < s.length; i++) view.setUint8(at + i, s.charCodeAt(i)); };
  ascii(0, 'RIFF'); view.setUint32(4, 36 + length * 2, true); ascii(8, 'WAVE');
  ascii(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, RATE, true); view.setUint32(28, RATE * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  ascii(36, 'data'); view.setUint32(40, length * 2, true);
  samples.forEach((s, i) => view.setInt16(44 + i * 2, Math.max(-1, Math.min(1, s)) * 0x7fff, true));
  return new Blob([view.buffer], { type: 'audio/wav' });
}

export function playChime(): void {
  try {
    audio ??= new Audio(URL.createObjectURL(chimeWav()));
    audio.currentTime = 0;
    // Refused before any user gesture, or no output device: the chime is a nicety, stay silent
    void audio.play().catch(() => {});
  } catch {
    // No media support at all: same
  }
}
