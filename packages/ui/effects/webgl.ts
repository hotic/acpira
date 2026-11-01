// Probe a WebGL context once so callers fall back to their static look without one. Ask the way the effect does: a
// webview can answer one way and not another (WebKitGTK without a GPU gives a page canvas WebGL but an OffscreenCanvas none)
const probes = new Map<string, boolean>();
function probe(key: string, context: () => unknown): boolean {
  let ok = probes.get(key);
  if (ok === undefined) {
    try { ok = !!context(); } catch { ok = false; }
    probes.set(key, ok);
  }
  return ok;
}

// The image generation mosaic (effects/mosaicGL): WebGL 2 on a page canvas
export function hasWebGL2(): boolean {
  return probe('webgl2', () => document.createElement('canvas').getContext('webgl2'));
}
