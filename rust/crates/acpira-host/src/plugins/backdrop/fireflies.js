const SCENE = {
  paint(c, w, h) {
    const g = c.createLinearGradient(0, 0, 0, h);
    g.addColorStop(0, '#03070d'); g.addColorStop(0.45, '#081519'); g.addColorStop(0.6, '#0c1d1a'); g.addColorStop(1, '#040b07');
    c.fillStyle = g; c.fillRect(0, 0, w, h);
    const mx = w * 0.8, my = h * 0.16, mr = Math.min(w, h) * 0.055;
    const halo = c.createRadialGradient(mx, my, mr, mx, my, h * 0.5);
    halo.addColorStop(0, 'rgba(206,232,190,.2)'); halo.addColorStop(1, 'rgba(206,232,190,0)');
    c.fillStyle = halo; c.fillRect(0, 0, w, h);
    // The moon: a soft disc, shaded toward its right, a few darker seas
    const disc = c.createRadialGradient(mx - mr * 0.35, my - mr * 0.3, mr * 0.1, mx, my, mr);
    disc.addColorStop(0, '#f1f6e4'); disc.addColorStop(0.8, '#d4dcc4'); disc.addColorStop(1, '#b9c4ab');
    c.fillStyle = disc; c.beginPath(); c.arc(mx, my, mr, 0, Math.PI * 2); c.fill();
    c.fillStyle = 'rgba(120,134,118,.22)';
    for (const [dx, dy, r] of [[-0.3, -0.2, 0.22], [0.25, 0.15, 0.3], [-0.1, 0.42, 0.14]]) { c.beginPath(); c.arc(mx + dx * mr, my + dy * mr, r * mr, 0, Math.PI * 2); c.fill(); }
    // The tree line: rounded crowns along the horizon, darker than the sky behind them
    this.base = h * 0.6;
    this.crowns = [];
    for (let x = -40; x < w + 40; x += 16) this.crowns.push([x, 18 + 34 * noise1(x * 0.011) + 12 * noise1(x * 0.05 + 3)]);
    c.fillStyle = '#030805';
    c.fillRect(0, this.base, w, h - this.base);
    for (const [x, r] of this.crowns) { c.beginPath(); c.arc(x, this.base - r * 0.3, r, 0, Math.PI * 2); c.fill(); }
    this.moon = [mx, my, mr];
  },
  setup(cols, rows) {
    this.cols = cols; this.rows = rows;
    // Per column: where the trees start and where the grass starts, in rows
    this.tree = new Float32Array(cols); this.grass = new Float32Array(cols);
    for (let x = 0; x < cols; x++) {
      const px = (x + 0.5) * CW;
      let top = this.base;
      for (const [cx, r] of this.crowns) { const dx = px - cx; if (Math.abs(dx) < r) top = Math.min(top, this.base - r * 0.3 - Math.sqrt(r * r - dx * dx)); }
      this.tree[x] = top / CH;
      this.grass[x] = rows - rows * (0.16 + 0.22 * noise1(x * 0.07 + 4) + 0.06 * noise1(x * 0.4));
    }
    this.flies = Array.from({ length: Math.round(cols * rows / 120) }, (_, i) => ({
      x: hash(i, 1) * cols, y: rows * (0.34 + 0.62 * hash(i, 2)), a: 4 + 9 * hash(i, 3), b: 1 + 3 * hash(i, 4),
      s: 0.05 + 0.1 * hash(i, 5), p: hash(i, 6) * 6.28, blink: 0.45 + 0.8 * hash(i, 8), q: hash(i, 9) * 6.28,
    }));
    this.light = new Float32Array(cols * rows); this.fly = new Float32Array(cols * rows);
  },
  frame(t, cols, rows) {
    this.light.fill(0); this.fly.fill(0);
    for (const f of this.flies) {
      const x = f.x + f.a * Math.sin(t * f.s + f.p) + 2 * Math.sin(t * f.s * 2.7 + f.q);
      const y = f.y + f.b * Math.sin(t * f.s * 1.3 + f.q) + Math.sin(t * 0.8 + f.p);
      const b = Math.pow(Math.max(0, Math.sin(t * f.blink + f.p * 3)), 2);
      if (b < 0.05) continue;
      const ix = Math.round(x), iy = Math.round(y);
      if (ix >= 0 && iy >= 0 && ix < cols && iy < rows) this.fly[iy * cols + ix] = Math.max(this.fly[iy * cols + ix], b);
      // Light falls on the cells around it
      for (let dy = -4; dy <= 5; dy++) for (let dx = -9; dx <= 9; dx++) {
        const cx = ix + dx, cy = iy + dy;
        if (cx < 0 || cy < 0 || cx >= cols || cy >= rows) continue;
        const d = Math.hypot(dx * 0.5, dy);
        if (d < 4.8) this.light[cy * cols + cx] += b * Math.pow(1 - d / 4.8, 1.6);
      }
    }
  },
  cell(x, y, t, o) {
    const i = y * this.cols + x;
    // The halo: soft light around each firefly, blended across cells
    const halo = Math.min(1.2, this.light[i]);
    if (halo > 0.02) light(o, 0.16 * halo, 0.21 * halo, 0.07 * halo);
    const fb = this.fly[i];
    if (fb > 0) return set(o, fb > 0.6 ? '●' : fb > 0.3 ? '•' : 'o', 0.85 + 0.15 * fb, 1, 0.45 + 0.3 * fb);
    const lit = Math.min(1.2, this.light[i]);
    const px = (x + 0.5) * CW, py = (y + 0.5) * CH;
    const [mx, my, mr] = this.moon;
    const dm = Math.hypot(px - mx, py - my) / mr;
    if (dm < 1.15) return false;
    if (y < this.tree[x]) {
      const h = hash(x, y);
      if (h < 0.016) {
        const tw = 0.55 + 0.45 * Math.sin(t * (0.6 + 2 * hash(y, x)) + h * 500);
        return set(o, h < 0.003 ? '+' : h < 0.008 ? '*' : '.', 0.5 * tw + 0.15, 0.58 * tw + 0.18, 0.66 * tw + 0.2);
      }
      // A faint band of dust across the sky
      const band = Math.exp(-Math.pow(y - this.rows * 0.06 - x * 0.16, 2) / 30);
      if (h < 0.06 + 0.22 * band && dm > 2.2) return set(o, '.', 0.13 + 0.1 * band, 0.17 + 0.12 * band, 0.22 + 0.12 * band);
      return false;
    }
    if (y < this.grass[x]) {
      // Leaves of the tree line and the dark meadow beyond, lit only by fireflies close by
      const tree = y < this.base / CH;
      const n = noise2(x * 0.6, y * 0.8);
      if (tree) return set(o, n > 0.55 ? '%' : n > 0.35 ? '&' : '*', 0.03 + 0.5 * lit, 0.07 + 0.6 * lit, 0.05 + 0.15 * lit);
      return hash(x, y) < 0.45 ? set(o, hash(y, x) < 0.5 ? '.' : ',', 0.05 + 0.5 * lit, 0.12 + 0.55 * lit, 0.07 + 0.12 * lit) : false;
    }
    // Grass: the tip leans with the gust crossing the field, deeper rows darker, warmer where a firefly is near
    const top = this.grass[x], depth = (y - top) / (this.rows - top);
    const gust = Math.sin(t * 0.9 - x * 0.07) * 0.75 + Math.sin(t * 2.3 + x * 0.55) * 0.25;
    const lean = gust * (1 - depth);
    const tip = y - top < 1;
    if (!tip && hash(x, y) > 1 - depth * 0.55) return false;
    const n = noise2(x * 0.15, y * 0.2);
    const ch = tip ? (hash(x, 4) < 0.08 ? '*' : lean > 0.3 ? '/' : lean < -0.3 ? '\\' : hash(x, 9) < 0.5 ? "'" : ',')
      : depth < 0.3 ? (lean > 0.35 ? '/' : lean < -0.35 ? '\\' : '|') : ramp('|!:;', hash(x, y));
    const dim = 1 - depth * 0.82;
    if (tip && hash(x, 4) < 0.08) return set(o, ch, 0.75 * dim + lit, 0.7 * dim + lit, 0.45 * dim);
    return set(o, ch, (0.07 + 0.08 * n) * dim + 0.55 * lit, (0.25 + 0.2 * n) * dim + 0.6 * lit + (tip ? 0.08 : 0), (0.11 + 0.06 * n) * dim + 0.12 * lit);
  },
};
