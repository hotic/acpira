const SCENE = {
  paint(c, w, h) {
    const g = c.createLinearGradient(0, 0, 0, h);
    g.addColorStop(0, '#062a40'); g.addColorStop(0.35, '#041a2c'); g.addColorStop(1, '#01060c');
    c.fillStyle = g; c.fillRect(0, 0, w, h);
    const s = c.createRadialGradient(w * 0.38, -h * 0.25, 0, w * 0.38, -h * 0.25, h * 1.05);
    s.addColorStop(0, 'rgba(90,180,215,.28)'); s.addColorStop(1, 'rgba(90,180,215,0)');
    c.fillStyle = s; c.fillRect(0, 0, w, h);
    // The floor: a dark rise of sand along the bottom
    c.fillStyle = '#02080d'; c.beginPath(); c.moveTo(0, h);
    for (let x = 0; x <= w; x += 6) c.lineTo(x, h * (0.93 - 0.04 * noise1(x * 0.006 + 2) - 0.02 * noise1(x * 0.03)));
    c.lineTo(w, h); c.fill();
  },
  setup(cols, rows) {
    this.cols = cols; this.rows = rows;
    // x, y as fractions; r the bell's radius in rows; tint; phase; speed
    this.jellies = [
      [0.74, 0.42, 5.2, [1, 0.5, 0.86], 0, 0.9], [0.22, 0.66, 3.6, [0.5, 0.86, 1], 2.1, 1.1],
      [0.5, 0.9, 2.5, [0.78, 0.6, 1], 4.2, 1.3], [0.92, 0.82, 2, [0.5, 1, 0.86], 1.3, 1.2],
    ];
    this.kelp = Array.from({ length: Math.max(3, Math.round(cols / 26)) }, (_, i) => [hash(i, 21) * cols, rows * (0.18 + 0.26 * hash(i, 22)), hash(i, 23) * 6]);
  },
  frame(t) {
    const span = this.rows + 34;
    this.J = this.jellies.map(([fx, fy, r, hue, p, s]) => {
      const pulse = Math.sin(t * s * 1.5 + p);
      // Rising slowly, a little faster on each contraction; wrapping below the floor
      const rise = t * 0.22 * s + 0.25 * Math.cos(t * s * 1.5 + p);
      const cy = (((fy * this.rows - rise) % span + span) % span) - 10;
      const R = r * CH * (1 + 0.08 * pulse);
      return [fx * this.cols * CW + 30 * Math.sin(t * 0.11 + p), cy * CH, R, R * (0.82 - 0.1 * pulse), hue, 0.8 + 0.2 * pulse, p, s];
    });
  },
  cell(x, y, t, o) {
    const px = (x + 0.5) * CW, py = (y + 0.5) * CH;
    // Shafts of sunlight, slanting, fading with depth
    const ray = Math.pow(noise1((x + y * 0.45) * 0.07 + t * 0.04), 5) * Math.pow(1 - y / this.rows, 1.6);
    if (ray > 0.01) light(o, 0.05 * ray, 0.2 * ray, 0.26 * ray);
    for (const [cx, cy, R, RY, hue, k, p, s] of this.J) {
      const dx = px - cx, dy = py - cy, d = Math.hypot(dx / R, dy / RY);
      if (d < 2.4) { const g = Math.pow(1 - d / 2.4, 2) * 0.16 * k; light(o, hue[0] * g, hue[1] * g, hue[2] * g); }
      const [hr, hg, hb] = hue;
      if (dy < 0 && d < 1.06) {
        // The bell: its rim by the curve's direction, the inside faint and speckled
        if (d > 0.84) {
          const a = Math.atan2(dy / RY, dx / R);
          const ch = a > -0.45 || a < -2.7 ? (dx < 0 ? '(' : ')') : a > -1.25 ? '\\' : a < -1.9 ? '/' : '-';
          return set(o, ch, hr * k, hg * k, hb * k);
        }
        return hash(x, y) < 0.3 + 0.3 * (1 - d) ? set(o, d < 0.45 ? ':' : '.', hr * 0.55 * k, hg * 0.55 * k, hb * 0.55 * k) : false;
      }
      // The hem, a frill rippling round the bell's mouth
      if (dy >= 0 && dy < CH && Math.abs(dx) < R * 1.02) return set(o, fract(x * 0.5 + t * s) < 0.5 ? '~' : '-', hr * 0.9 * k, hg * 0.9 * k, hb * 0.9 * k);
      // Tentacles trailing below, waving, thinning out
      const L = R * 3.2;
      if (dy >= CH && dy < L && Math.abs(dx) < R * 1.1) {
        const f = dy / L;
        for (let j = 0; j < 5; j++) {
          const wave = Math.sin(dy / CH * 0.42 - t * 2.1 * s + j * 1.7 + p);
          const tx = cx + (j / 4 - 0.5) * R * 1.4 + wave * f * R * 0.32;
          if (Math.abs(px - tx) < CW * 0.55) {
            const slope = Math.cos(dy / CH * 0.42 - t * 2.1 * s + j * 1.7 + p) * f;
            const fade = (1 - f) * 0.85;
            return set(o, slope > 0.25 ? '\\' : slope < -0.25 ? '/' : j % 2 ? ':' : '|', hr * fade, hg * fade, hb * fade);
          }
        }
      }
    }
    // Kelp from the floor, swaying
    for (const [kx, kh, kp] of this.kelp) {
      const up = this.rows - y;
      if (up > kh) continue;
      const sway = Math.sin(y * 0.28 + t * 0.7 + kp) * (up / kh) * 2.2;
      if (Math.abs(x - (kx + sway)) < 0.6) {
        const lean = Math.cos(y * 0.28 + t * 0.7 + kp);
        return set(o, lean > 0.3 ? ')' : lean < -0.3 ? '(' : '|', 0.1, 0.28 + 0.1 * (up / kh), 0.22);
      }
    }
    // Bubbles going up in two streams, smaller as they rise
    for (const [bx, bp] of [[0.12, 0], [0.63, 3]]) {
      const sx = Math.round(bx * this.cols + 1.2 * Math.sin(y * 0.5 + t * 1.6 + bp));
      if (x === sx && hash(bp, Math.floor(y + t * 3.2)) < 0.22) return set(o, y < this.rows * 0.5 ? '°' : 'o', 0.55, 0.8, 0.9);
    }
    // Marine snow, sinking
    if (hash(x, Math.floor(y - t * (0.4 + 0.5 * hash(x, 8)))) < 0.01) return set(o, '.', 0.32, 0.46, 0.52);
    return false;
  },
};
