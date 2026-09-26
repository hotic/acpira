// The WebGL effects (metal-fx, img-fx) throw inside an effect when there is no context, tearing down the whole React tree;
// probe once and let callers fall back to their static look
let webgl: boolean | undefined;
export function hasWebGL(): boolean {
  if (webgl === undefined) {
    try {
      const c = document.createElement('canvas');
      webgl = !!(c.getContext('webgl2') ?? c.getContext('webgl'));
    } catch { webgl = false; }
  }
  return webgl;
}
