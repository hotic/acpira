// The attention chime (setting `notifySound`): two soft sine notes synthesized on the spot, so no audio file ships.
// One AudioContext for the page's life. A context the autoplay policy started suspended resumes once the page has had a user
// gesture; switching the setting on plays a preview from the click, which is one
const NOTES = [880, 1318.5];
const GAP = 0.12;
const PEAK = 0.16;
const DECAY = 0.6;

let ctx: AudioContext | undefined;

export function playChime(): void {
  try {
    ctx ??= new AudioContext();
    if (ctx.state === 'suspended') void ctx.resume();
    const at = ctx.currentTime + 0.01;
    NOTES.forEach((freq, i) => {
      const start = at + i * GAP;
      const osc = ctx!.createOscillator();
      const gain = ctx!.createGain();
      osc.type = 'sine';
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0, start);
      gain.gain.linearRampToValueAtTime(PEAK, start + 0.015);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + DECAY);
      osc.connect(gain).connect(ctx!.destination);
      osc.start(start);
      osc.stop(start + DECAY + 0.05);
    });
  } catch {
    // No Web Audio or no output device: the chime is a nicety, stay silent
  }
}
