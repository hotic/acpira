// Small deterministic noise and colour helpers every scene can use
const fract = v => v - Math.floor(v);
const hash = (x, y) => fract(Math.sin(x * 127.1 + y * 311.7) * 43758.5453);
const smooth = v => v * v * (3 - 2 * v);
const noise1 = x => { const i = Math.floor(x), f = x - i; return hash(i, 7.1) + (hash(i + 1, 7.1) - hash(i, 7.1)) * smooth(f); };
const noise2 = (x, y) => {
  const i = Math.floor(x), j = Math.floor(y), u = smooth(x - i), v = smooth(y - j);
  const a = hash(i, j), b = hash(i + 1, j), c = hash(i, j + 1), d = hash(i + 1, j + 1);
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
};
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const mix = (a, b, k) => a + (b - a) * k;
const ramp = (s, v) => s[clamp(Math.floor(v * s.length), 0, s.length - 1)];
const set = (o, ch, r, g, b) => { o.ch = ch; o.r = r; o.g = g; o.b = b; return true; };
const light = (o, r, g, b) => { o.lr += r; o.lg += g; o.lb += b; };
