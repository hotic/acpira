const SCENE = {
  paint(c, w, h) {
    const g = c.createLinearGradient(0, 0, 0, h);
    g.addColorStop(0, '#f6f2e9'); g.addColorStop(1, '#ebe4d5');
    c.fillStyle = g; c.fillRect(0, 0, w, h);
    // Paper fibre
    for (let i = 0; i < w * h / 70; i++) {
      c.fillStyle = hash(i, 1) < 0.5 ? 'rgba(110,92,60,.045)' : 'rgba(255,255,255,.3)';
      c.fillRect(hash(i, 2) * w, hash(i, 3) * h, 1 + 2 * hash(i, 4), 1);
    }
    // The sun, a seal-red disc with a soft edge
    const sx = w * 0.86, sy = h * 0.16, sr = Math.min(w, h) * 0.034;
    const s = c.createRadialGradient(sx, sy, sr * 0.88, sx, sy, sr * 1.04);
    s.addColorStop(0, 'rgba(190,66,46,.82)'); s.addColorStop(1, 'rgba(190,66,46,0)');
    c.fillStyle = s; c.beginPath(); c.arc(sx, sy, sr * 1.1, 0, Math.PI * 2); c.fill();
    this.water = h * 0.84;
    // Ranges far to near: base, height, frequency, seed, ink
    this.ranges = [
      [h * 0.5, h * 0.2, 0.0036, 1.7, [92, 104, 112, 0.22]], [h * 0.6, h * 0.22, 0.0052, 5.3, [72, 80, 86, 0.36]],
      [h * 0.72, h * 0.24, 0.0075, 9.1, [48, 52, 56, 0.58]], [h * 0.82, h * 0.2, 0.011, 13.4, [24, 26, 28, 0.86]],
    ];
    for (const [base, height, f, seed, [r, gg, b, a]] of this.ranges) {
      // Ink heavy at the ridge, washing out toward the foot
      const top = base - height;
      const ink = c.createLinearGradient(0, top, 0, base + height * 0.15);
      ink.addColorStop(0, 'rgba(' + r + ',' + gg + ',' + b + ',' + a + ')');
      ink.addColorStop(0.55, 'rgba(' + r + ',' + gg + ',' + b + ',' + a * 0.45 + ')');
      ink.addColorStop(1, 'rgba(' + r + ',' + gg + ',' + b + ',0)');
      c.fillStyle = ink; c.beginPath(); c.moveTo(0, base + height * 0.2);
      for (let x = 0; x <= w; x += 3) c.lineTo(x, SCENE.ridge(x, base, height, f, seed));
      c.lineTo(w, base + height * 0.2); c.fill();
      // Mist at its foot
      const m = c.createLinearGradient(0, base - height * 0.25, 0, base + height * 0.2);
      m.addColorStop(0, 'rgba(246,242,233,0)'); m.addColorStop(0.7, 'rgba(246,242,233,.75)'); m.addColorStop(1, 'rgba(246,242,233,0)');
      c.fillStyle = m; c.fillRect(0, base - height * 0.25, w, height * 0.45);
    }
    c.fillStyle = 'rgba(236,230,216,.9)'; c.fillRect(0, this.water, w, h - this.water);
  },
  ridge(px, base, height, f, seed) {
    return base - height * (0.5 * Math.pow(Math.abs(Math.sin(px * f + seed)), 1.4) + 0.38 * noise1(px * f * 2.7 + seed) + 0.12 * noise1(px * f * 8 + seed));
  },
  setup(cols, rows) {
    this.cols = cols; this.rows = rows; this.waterRow = Math.ceil(this.water / CH);
    this.r = this.ranges.map(([base, height, f, seed, ink]) => {
      const a = new Float32Array(cols + 2);
      for (let x = -1; x <= cols; x++) a[x + 1] = SCENE.ridge((x + 0.5) * CW, base, height, f, seed) / CH;
      return [a, ink, base / CH];
    });
  },
  frame(t) {
    // A loose V of birds drifting left to right, wings beating out of step
    const bx = fract(t * 0.018 + 0.3) * (this.cols + 40) - 20, by = this.rows * 0.24 + 2 * Math.sin(t * 0.2);
    this.birds = [[0, 0], [-3, 1], [-6, 2], [3, 1], [5, 2.4]].map(([dx, dy], i) => [Math.round(bx + dx), Math.round(by + dy + 0.4 * Math.sin(t * 1.3 + i)), Math.sin(t * 5 + i * 1.9) > 0]);
    this.boat = Math.round(this.cols * (0.2 + 0.05 * Math.sin(t * 0.05)));
  },
  cell(x, y, t, o) {
    for (const [bx, by, up] of this.birds) if (x === bx && y === by) return set(o, up ? 'v' : '-', 0.22, 0.2, 0.19);
    if (y >= this.waterRow) {
      // The boat, the fisher's pole, and still water with long slow ripples
      const row = y - this.waterRow, dx = x - this.boat;
      if (row === 1 && dx >= -2 && dx <= 2) return set(o, dx === -2 ? '\\' : dx === 2 ? '/' : '_', 0.16, 0.15, 0.14);
      if (row === 0 && dx === 1) return set(o, '/', 0.2, 0.19, 0.18);
      if (row === 2 && dx >= -2 && dx <= 2) return hash(x, 2) < 0.6 ? set(o, '-', 0.55, 0.54, 0.5) : false;
      const w = Math.sin(x * 0.11 - t * 0.35 + row * 1.9) * Math.sin(x * 0.03 + row);
      return w > 0.55 ? set(o, w > 0.8 ? '~' : '-', 0.62, 0.62, 0.58) : false;
    }
    // Mist drifting along each range's foot: light on paper is a wash of white
    let lit = 0;
    for (const [, , base] of this.r) {
      const d = (y - base + 1.5) / 2.2;
      if (Math.abs(d) < 1.4) lit = Math.max(lit, (1 - Math.abs(d) / 1.4) * Math.max(0, noise2(x * 0.06 - t * 0.05, y * 0.3 + base) - 0.35) * 1.4);
    }
    if (lit > 0.02) light(o, 0.12 * lit, 0.12 * lit, 0.11 * lit);
    // The ranges are painted; glyphs only dot the nearer ridges and texture their slopes
    for (let i = this.r.length - 1; i >= 0; i--) {
      const [ridge, [r, g, b, a], base] = this.r[i], top = ridge[x + 1];
      if (y + 0.5 < top) continue;
      const k = 1 - a * 0.9;
      const below = y - top;
      // Moss dots (苔点) along the nearer ridges, as a brush would set them
      if (below < 1) return i >= 2 && hash(x, i) < 0.16 ? set(o, hash(i, x) < 0.5 ? '.' : ':', k * 0.45, k * 0.45, k * 0.43) : false;
      if (i >= 2 && below < (base - top) * 0.5 && noise2(x * 0.35, y * 0.5 + i * 7) > 0.7 && hash(x, y) < 0.35)
        return set(o, hash(y, x) < 0.5 ? ';' : ',', k * 0.5, k * 0.5, k * 0.48);
      return false;
    }
    return false;
  },
};
