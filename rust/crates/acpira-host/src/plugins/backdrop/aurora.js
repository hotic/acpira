const SCENE = {
  paint(c, w, h) {
    this.lake = h * 0.78;
    const g = c.createLinearGradient(0, 0, 0, h);
    g.addColorStop(0, '#02040a'); g.addColorStop(0.5, '#061020'); g.addColorStop(0.78, '#0b1a2a'); g.addColorStop(0.781, '#03080f'); g.addColorStop(1, '#02050a');
    c.fillStyle = g; c.fillRect(0, 0, w, h);
    const glowG = c.createRadialGradient(w * 0.55, h * 0.4, 0, w * 0.55, h * 0.4, w * 0.6);
    glowG.addColorStop(0, 'rgba(60,200,150,.10)'); glowG.addColorStop(1, 'rgba(60,200,150,0)');
    c.fillStyle = glowG; c.fillRect(0, 0, w, this.lake);
    // Two ranges, the far one paler
    this.ranges = [[h * 0.6, h * 0.16, 0.0042, 1.3, '#0b1424'], [h * 0.7, h * 0.12, 0.0068, 4.1, '#060b15']];
    for (const [base, height, f, seed, fill] of this.ranges) {
      c.fillStyle = fill; c.beginPath(); c.moveTo(0, this.lake);
      for (let x = 0; x <= w; x += 4) c.lineTo(x, SCENE.ridge(x, base, height, f, seed));
      c.lineTo(w, this.lake); c.fill();
      // Its reflection, darker
      c.globalAlpha = 0.55; c.beginPath(); c.moveTo(0, this.lake);
      for (let x = 0; x <= w; x += 4) c.lineTo(x, 2 * this.lake - SCENE.ridge(x, base, height, f, seed) * 0.35 - this.lake * 0.65);
      c.lineTo(w, this.lake); c.fill(); c.globalAlpha = 1;
    }
  },
  ridge(px, base, height, f, seed) {
    return base - height * (0.55 * Math.abs(Math.sin(px * f + seed)) + 0.35 * noise1(px * f * 3.1 + seed) + 0.1 * noise1(px * f * 9 + seed));
  },
  setup(cols, rows) {
    this.cols = cols; this.rows = rows;
    this.lakeRow = Math.floor(this.lake / CH);
    this.r = this.ranges.map(([base, height, f, seed]) => {
      const a = new Float32Array(cols + 2);
      for (let x = -1; x <= cols; x++) a[x + 1] = SCENE.ridge((x + 0.5) * CW, base, height, f, seed) / CH;
      return [a, (base - height * 0.74) / CH];
    });
    this.ribbons = [
      { y: rows * 0.4, a: 5, f: 0.03, s: 0.16, h: rows * 0.28, k: 1, p: 0, c: 0.62, w: 0.42 },
      { y: rows * 0.27, a: 3.5, f: 0.05, s: -0.13, h: rows * 0.18, k: 0.8, p: 3, c: 0.22, w: 0.3 },
    ];
  },
  frame(t) {
    // Each ribbon's hem and strength per column, once per frame
    this.hem = this.ribbons.map(R => {
      const hem = new Float32Array(this.cols), pow = new Float32Array(this.cols);
      for (let x = 0; x < this.cols; x++) {
        hem[x] = R.y + R.a * Math.sin(x * R.f + t * R.s) + R.a * 0.5 * Math.sin(x * R.f * 2.3 - t * R.s * 1.7)
          + 2.5 * Math.sin(x * 0.12 + t * 0.5) * Math.sin(x * 0.02 - t * 0.1) + 0.6 * Math.sin(x * 0.3 + t * 0.8);
        // Strongest around the ribbon's centre, pulsing along it, broken into rays
        const env = Math.exp(-Math.pow((x / this.cols - R.c) / R.w, 2));
        pow[x] = R.k * env * clamp(0.6 + 0.4 * Math.sin(x * R.f * 0.7 - t * 0.33 + R.p), 0, 1) * (0.45 + 0.55 * noise1(x * 0.9 + t * 0.6 + R.p * 10));
      }
      return [hem, pow, R.h];
    });
  },
  aurora(x, y) {
    let v = 0, up = 0;
    for (const [hem, pow, h] of this.hem) {
      const above = hem[x] - y;
      if (above < -1.2 || above > h) continue;
      const prof = above < 0 ? 1 + above / 1.2 : Math.pow(1 - above / h, 1.5);
      const k = prof * pow[x];
      if (k > v) { v = k; up = Math.max(0, above) / h; }
    }
    return [v, up];
  },
  tint(v, up) {
    // Green at the hem, teal, then violet toward the top
    const r = up < 0.5 ? mix(0.2, 0.18, up * 2) : mix(0.18, 0.6, (up - 0.5) * 2);
    const g = up < 0.5 ? mix(1, 0.85, up * 2) : mix(0.85, 0.3, (up - 0.5) * 2);
    const b = up < 0.5 ? mix(0.6, 0.85, up * 2) : mix(0.85, 0.95, (up - 0.5) * 2);
    return [r * v, g * v, b * v];
  },
  cell(x, y, t, o) {
    if (y >= this.lakeRow) {
      // The lake: the sky above, mirrored and broken by slow ripples
      const my = 2 * this.lakeRow - y - 1 + Math.sin(x * 0.4 + y * 1.3 - t * 1.2) * 0.8;
      const mx = clamp(Math.round(x + 1.2 * Math.sin(y * 0.9 + t)), 0, this.cols - 1);
      const [v, up] = this.aurora(mx, Math.round(my));
      const depth = (y - this.lakeRow) / (this.rows - this.lakeRow);
      const k = v * 0.55 * (1 - depth * 0.7);
      if (k > 0.02) { const [lr, lg, lb] = this.tint(k * 0.32, up); light(o, lr, lg, lb); }
      if (k < 0.08) return hash(x, y + Math.floor(t * 2)) < 0.02 ? set(o, '.', 0.2, 0.3, 0.4) : false;
      const [r, g, b] = this.tint(k, up);
      return set(o, k > 0.4 ? '~' : k > 0.2 ? '-' : '.', r, g, b);
    }
    for (let i = this.r.length - 1; i >= 0; i--) {
      const [ridge, snowLine] = this.r[i], top = ridge[x + 1];
      if (y + 0.5 < top) continue;
      // Rock: the ridge line drawn by its slope, snow on the high slopes, faint aurora light on the far range
      const slope = ridge[x + 2] - ridge[x];
      const edge = y - top < 1;
      const high = top < snowLine;
      const far = i === 0;
      // Only the snow is drawn; the rock is the painted silhouette
      if (!high) return edge && hash(x, 5) < 0.5 ? set(o, '.', 0.16, 0.22, 0.32) : false;
      // The ridge line by its slope; a caret only on a peak, so a long crest never reads as a row of carets
      const peak = ridge[x + 1] <= ridge[x] && ridge[x + 1] <= ridge[x + 2];
      if (edge) return set(o, peak ? '^' : slope > 0.12 ? '\\' : slope < -0.12 ? '/' : "'", 0.8, 0.86, 0.96);
      const below = y - top;
      if (below < 2.5 + 3 * noise1(x * 0.4 + i)) return hash(x, y) < 0.6 ? set(o, slope < 0 ? '/' : "'", 0.5 - below * 0.06, 0.58 - below * 0.06, 0.74 - below * 0.06) : false;
      return false;
    }
    const [v, up] = this.aurora(x, y);
    // The curtain's own light, soft under its glyphs
    if (v > 0.02) { const [lr, lg, lb] = this.tint(Math.min(1, v * 0.5 + up * 0.1), up); light(o, lr, lg, lb); }
    if (v > 0.06 && hash(x, y + Math.floor(t * 3)) < 0.35 + v * 0.6) {
      // The upper reaches keep their violet even where faint
      const [r, g, b] = this.tint(Math.min(1, v * 1.15 + up * 0.18), up);
      return set(o, v > 0.66 ? '|' : v > 0.42 ? '!' : v > 0.24 ? ':' : up > 0.45 ? "'" : '.', r, g, b);
    }
    const h = hash(x, y);
    if (h < 0.013) {
      const tw = 0.55 + 0.45 * Math.sin(t * (0.5 + 2 * hash(y, x)) + h * 300);
      return set(o, h < 0.003 ? '+' : '.', 0.55 * tw + 0.15, 0.6 * tw + 0.18, 0.75 * tw + 0.2);
    }
    return false;
  },
};
