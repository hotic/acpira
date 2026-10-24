import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// build/icon.svg → build/icon.icns (and build/icon.png at 1024 px): every size of the iconset rendered from the SVG
// rather than scaled down from one bitmap. macOS only (iconutil); rsvg-convert comes from librsvg (brew install librsvg)
const build = fileURLToPath(new URL('../build/', import.meta.url));
const svg = join(build, 'icon.svg');
const set = join(mkdtempSync(join(tmpdir(), 'acpira-icon-')), 'icon.iconset');
execFileSync('mkdir', ['-p', set]);
const render = (px, file) => execFileSync('rsvg-convert', ['-w', String(px), '-h', String(px), svg, '-o', file]);
for (const pt of [16, 32, 128, 256, 512]) {
  render(pt, join(set, `icon_${pt}x${pt}.png`));
  render(pt * 2, join(set, `icon_${pt}x${pt}@2x.png`));
}
execFileSync('iconutil', ['-c', 'icns', set, '-o', join(build, 'icon.icns')]);
render(1024, join(build, 'icon.png'));
rmSync(join(set, '..'), { recursive: true, force: true });
console.log('build/icon.icns, build/icon.png');
