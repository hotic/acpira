// Acpira's backdrop runtime: a glyph grid drawn from a scene (`SCENE`, the script before this one). Each cell holds
// one glyph in one colour (`set`) and soft light blended with its neighbours (`light`). WebGL2 by default (one draw per
// frame); a 2D fallback for thumbnails and hosts without it. The frame cap, the pause while the window is in the
// background or the page asks, and a frozen clock for recording are handled here, so a scene only says what each cell
// looks like. Leave this script as it is.
// Options the page hands the frame (`acpira.options`): cell font, frame cap, renderer, resolution, a frozen clock
const OPTS = Object.assign({}, (window.acpira && window.acpira.options) || {});
const FONT = OPTS.font || 13;
const CW = Math.max(3, Math.round(FONT * 0.62)), CH = Math.max(5, Math.round(FONT * 1.3));
const cv = document.getElementById('c');
const out = { ch: '', r: 0, g: 0, b: 0, lr: 0, lg: 0, lb: 0 };
let dpr = 1, cols = 0, rows = 0, gw = 0, gh = 0, bg, cells, lights, ready = false;
let fps = OPTS.fps || 30, run = OPTS.run !== false, clock = OPTS.clock ?? null, last = -1e9, start = performance.now();
let told = false;
// Glyphs get an index on first use (1-255, 0 = none) and a tile in a white mask atlas
const glyphOf = new Int16Array(65536).fill(-1), glyphs = [''];
const PER_ROW = 32, ATLAS_ROWS = 8;
let mask, mctx, maskDirty = false;
function glyphIndex(ch) {
  const code = ch.charCodeAt(0);
  let i = glyphOf[code];
  if (i >= 0) return i;
  if (glyphs.length >= PER_ROW * ATLAS_ROWS) return 0;
  i = glyphOf[code] = glyphs.length; glyphs.push(ch);
  mctx.fillText(ch, (i % PER_ROW) * gw + gw / 2, Math.floor(i / PER_ROW) * gh + gh / 2 + dpr);
  maskDirty = true;
  return i;
}
const byte = v => v <= 0 ? 0 : v >= 1 ? 255 : (v * 255) | 0;

let gl = OPTS.gl !== false ? cv.getContext('webgl2', { alpha: false, antialias: false, depth: false, stencil: false, premultipliedAlpha: false }) : null;
let cx = gl ? null : cv.getContext('2d', { alpha: false });
let prog, tex = {}, U = {}, small, sctx;
function glSetup() {
  if (!prog) {
    const sh = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s)); return s; };
    prog = gl.createProgram();
    gl.attachShader(prog, sh(gl.VERTEX_SHADER, '#version 300 es\nvoid main(){vec2 p=vec2((gl_VertexID<<1)&2,gl_VertexID&2);gl_Position=vec4(p*2.0-1.0,0.0,1.0);}'));
    gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, [
      '#version 300 es', 'precision highp float;',
      'uniform sampler2D uBg, uCells, uLight, uMask; uniform vec2 uView, uCell, uGrid; uniform float uPerRow;',
      'out vec4 o;',
      'void main(){',
      '  vec2 px = vec2(gl_FragCoord.x, uView.y - gl_FragCoord.y);',
      '  vec3 col = texture(uBg, px / uView).rgb;',
      '  col += texture(uLight, px / (uCell * uGrid)).rgb;',
      '  vec2 cp = px / uCell; ivec2 cell = ivec2(cp);',
      '  vec4 c = texelFetch(uCells, cell, 0);',
      '  float gi = floor(c.a * 255.0 + 0.5);',
      '  if (gi > 0.5) {',
      '    vec2 tile = vec2(mod(gi, uPerRow), floor(gi / uPerRow));',
      '    float a = texelFetch(uMask, ivec2(tile * uCell + floor(fract(cp) * uCell)), 0).a;',
      '    col = mix(col, c.rgb, a);',
      '  }',
      '  o = vec4(col, 1.0);',
      '}'].join('\n')));
    gl.linkProgram(prog);
    gl.useProgram(prog);
    for (const n of ['uBg', 'uCells', 'uLight', 'uMask', 'uView', 'uCell', 'uGrid', 'uPerRow']) U[n] = gl.getUniformLocation(prog, n);
    ['bg', 'cells', 'light', 'mask'].forEach((n, i) => { tex[n] = gl.createTexture(); gl.uniform1i(U['u' + n[0].toUpperCase() + n.slice(1)], i); });
  }
  const set = (n, unit, filter) => { gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, tex[n]); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE); };
  set('bg', 0, gl.NEAREST); gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, bg);
  set('cells', 1, gl.NEAREST); gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, cols, rows, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
  set('light', 2, gl.LINEAR); gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, cols, rows, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
  set('mask', 3, gl.NEAREST); gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, mask);
  gl.viewport(0, 0, cv.width, cv.height);
  gl.uniform2f(U.uView, cv.width, cv.height); gl.uniform2f(U.uCell, gw, gh); gl.uniform2f(U.uGrid, cols, rows); gl.uniform1f(U.uPerRow, PER_ROW);
}
function setup() {
  // A frame not laid out yet (0 x 0) waits for its first resize
  if (!innerWidth || !innerHeight) { ready = false; return; }
  dpr = Math.min(OPTS.dpr || 2, window.devicePixelRatio || 1);
  cv.width = Math.ceil(innerWidth * dpr); cv.height = Math.ceil(innerHeight * dpr);
  cols = Math.ceil(innerWidth / CW) + 1; rows = Math.ceil(innerHeight / CH) + 1;
  // Whole device pixels per cell, so the glyph tiles and the grid line up exactly
  gw = Math.round(CW * dpr); gh = Math.round(CH * dpr);
  bg = document.createElement('canvas'); bg.width = cv.width; bg.height = cv.height;
  const b = bg.getContext('2d'); b.scale(dpr, dpr);
  SCENE.paint(b, innerWidth, innerHeight, cols, rows);
  mask = document.createElement('canvas'); mask.width = gw * PER_ROW; mask.height = gh * ATLAS_ROWS;
  mctx = mask.getContext('2d');
  mctx.font = (FONT * dpr) + 'px ui-monospace, "SF Mono", Menlo, monospace';
  mctx.textAlign = 'center'; mctx.textBaseline = 'middle'; mctx.fillStyle = '#fff';
  // Glyph indices and tinted tiles belong to this atlas and cell size
  glyphOf.fill(-1); glyphs.length = 1; tileCache.clear();
  cells = new Uint8Array(cols * rows * 4); lights = new Uint8Array(cols * rows * 4);
  SCENE.setup?.(cols, rows);
  if (gl) glSetup();
  else { small = document.createElement('canvas'); small.width = cols; small.height = rows; sctx = small.getContext('2d'); }
  ready = true;
}
function fill(t) {
  SCENE.frame?.(t, cols, rows);
  for (let y = 0, i = 0; y < rows; y++) for (let x = 0; x < cols; x++, i += 4) {
    out.ch = ''; out.lr = out.lg = out.lb = 0;
    SCENE.cell(x, y, t, out);
    lights[i] = byte(out.lr); lights[i + 1] = byte(out.lg); lights[i + 2] = byte(out.lb); lights[i + 3] = 255;
    if (out.ch) { cells[i] = byte(out.r); cells[i + 1] = byte(out.g); cells[i + 2] = byte(out.b); cells[i + 3] = glyphIndex(out.ch); }
    else cells[i + 3] = 0;
  }
}
// 2D fallback: per-glyph tiles tinted on the fly would be slow, so glyphs are cached per quantised colour, a bounded
// number of them (a scene sweeping through colours would otherwise keep every tile it ever drew)
const tileCache = new Map(), TILES_MAX = 4096;
function draw2d() {
  cx.drawImage(bg, 0, 0);
  sctx.putImageData(new ImageData(new Uint8ClampedArray(lights.buffer), cols, rows), 0, 0);
  cx.globalCompositeOperation = 'lighter'; cx.imageSmoothingEnabled = true;
  cx.drawImage(small, 0, 0, cols * gw, rows * gh);
  cx.globalCompositeOperation = 'source-over';
  for (let i = 0, n = cols * rows; i < n; i++) {
    const g = cells[i * 4 + 3];
    if (!g) continue;
    const key = ((cells[i * 4] >> 3) << 10 | (cells[i * 4 + 1] >> 3) << 5 | (cells[i * 4 + 2] >> 3)) * 256 + g;
    let c = tileCache.get(key);
    if (!c) {
      c = document.createElement('canvas'); c.width = gw; c.height = gh;
      const k = c.getContext('2d');
      k.drawImage(mask, (g % PER_ROW) * gw, Math.floor(g / PER_ROW) * gh, gw, gh, 0, 0, gw, gh);
      k.globalCompositeOperation = 'source-in';
      k.fillStyle = 'rgb(' + (cells[i * 4] | 7) + ',' + (cells[i * 4 + 1] | 7) + ',' + (cells[i * 4 + 2] | 7) + ')';
      k.fillRect(0, 0, gw, gh);
      if (tileCache.size >= TILES_MAX) tileCache.clear();
      tileCache.set(key, c);
    }
    cx.drawImage(c, (i % cols) * gw, Math.floor(i / cols) * gh);
  }
}
function drawGl() {
  if (maskDirty) { gl.activeTexture(gl.TEXTURE3); gl.bindTexture(gl.TEXTURE_2D, tex.mask); gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, mask); maskDirty = false; }
  gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D, tex.cells); gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, cols, rows, gl.RGBA, gl.UNSIGNED_BYTE, cells);
  gl.activeTexture(gl.TEXTURE2); gl.bindTexture(gl.TEXTURE_2D, tex.light); gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, cols, rows, gl.RGBA, gl.UNSIGNED_BYTE, lights);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
}
function draw(t) {
  if (!ready) return;
  fill(t);
  if (gl) drawGl(); else draw2d();
  // The page waits for the first frame before it uncovers the skin
  if (!told) { told = true; parent.postMessage({ acpira: 'drawn' }, '*'); }
}
function frame(now) {
  requestAnimationFrame(frame);
  if (!run || document.hidden || clock !== null) return;
  // The cap keeps its own beat (last advances by whole intervals), so it does not drift down to the display's divisors
  const step = 1000 / fps;
  if (now - last < step - 2) return;
  last = now - last > step * 2 ? now : last + step;
  draw((now - start) / 1000);
}
// The page pauses the backdrop (window in the background, motion off) and may step a frozen clock for recording
addEventListener('message', e => {
  const m = e.data;
  if (e.source !== parent || !m || m.acpira !== 'backdrop') return;
  if (m.fps) fps = m.fps;
  if (m.run !== undefined) run = m.run;
  if (m.clock !== undefined) { clock = m.clock; if (clock !== null) draw(clock); }
});
addEventListener('resize', () => { setup(); draw(clock ?? (performance.now() - start) / 1000); });
setup();
draw(clock ?? 0);
requestAnimationFrame(frame);
