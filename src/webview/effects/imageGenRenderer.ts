import { PRESETS, createInstance, setFrameRate, setInstancePaused, type Instance } from 'img-fx';
import { IMAGE_GEN } from './presets';

// img-fx and the three.js renderer behind it (about 0.9 MB): a chunk of its own, loaded by the first generation card
// (`GeneratedImage.tsx`) instead of with the page
export { ImageGeneration } from 'img-fx';

// img-fx shares one WebGL renderer across instances and disposes it with the last one, so every card would pay the
// context creation and the synchronous shader link again. A paused 1px instance keeps the renderer alive once the first
// card has built it; a paused instance is skipped by the render loop and the GL canvas sizing, so it costs no frames.
let keeper: Instance | undefined;
export function keepImageGenRenderer() {
  if (keeper) return;
  setFrameRate(IMAGE_GEN.fps);
  keeper = createInstance({ canvas: document.createElement('canvas'), cssWidth: 1, cssHeight: 1, preset: PRESETS[IMAGE_GEN.preset].modes.dark });
  setInstancePaused(keeper, true);
}
