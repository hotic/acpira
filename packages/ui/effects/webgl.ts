// The WebGL effects (metal-fx, img-fx) throw inside an effect when they get no context, tearing down the whole React tree;
// probe once, the way each library asks for its context, and let callers fall back to their static look. The two ask
// differently, and a webview can answer one and not the other: WebKitGTK (the Linux desktop app) without a GPU gives a
// page canvas WebGL but an OffscreenCanvas none (2.50.4, 2026-10-26)
const probes = new Map<string, boolean>();
function probe(key: string, context: () => unknown): boolean {
  let ok = probes.get(key);
  if (ok === undefined) {
    try { ok = !!context(); } catch { ok = false; }
    probes.set(key, ok);
  }
  return ok;
}

// metal-fx (the send button's ring): WebGL 1 on an OffscreenCanvas wherever the browser has OffscreenCanvas, else on a
// page canvas (metal-fx 1.0.4)
export function hasMetalWebGL(): boolean {
  return probe('metal', () => {
    if (typeof OffscreenCanvas !== 'undefined') return new OffscreenCanvas(1, 1).getContext('webgl');
    const c = document.createElement('canvas');
    return c.getContext('webgl') ?? c.getContext('experimental-webgl');
  });
}

// img-fx (the image generation card): three.js on a page canvas, which needs WebGL 2 (three r163 and later)
export function hasWebGL2(): boolean {
  return probe('webgl2', () => document.createElement('canvas').getContext('webgl2'));
}
