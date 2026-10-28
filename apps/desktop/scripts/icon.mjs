import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// icons/icon.svg → icons/icon.icns, icons/icon.ico (Windows: the exe's icon, taskbar and Alt-Tab), icons/icon.png at 32 px
// and the Linux packages' hicolor icons (icons/128x128.png, 256x256.png, 512x512.png): every size rendered from the SVG
// rather than scaled down from one bitmap, by icon-render.py so the tile's gradient does not band. Runs on macOS only
// (iconutil); needs rsvg-convert from librsvg (brew install librsvg) and python3 with numpy + Pillow
// icon.png is only Tauri's default window icon, which tauri-codegen decodes into raw RGBA inside the shell binary (a
// 1024 px one was 4 MiB) and macOS never shows (tao's set_window_icon is a no-op there; Dock, Finder and About read the
// icns, and so does a dev run's Dock icon), so it stays tiny. tauri-codegen takes the first PNG of tauri.conf.json's icon
// list, so icon.png stays ahead of the Linux sizes there
const icons = fileURLToPath(new URL('../icons/', import.meta.url));
const svg = join(icons, 'icon.svg');
const set = join(mkdtempSync(join(tmpdir(), 'acpira-icon-')), 'icon.iconset');
execFileSync('mkdir', ['-p', set]);
const renderer = fileURLToPath(new URL('./icon-render.py', import.meta.url));
const render = (px, file) => execFileSync('python3', [renderer, svg, String(px), file]);
for (const pt of [16, 32, 128, 256, 512]) {
  render(pt, join(set, `icon_${pt}x${pt}.png`));
  render(pt * 2, join(set, `icon_${pt}x${pt}@2x.png`));
}
execFileSync('iconutil', ['-c', 'icns', set, '-o', join(icons, 'icon.icns')]);
render(32, join(icons, 'icon.png'));
for (const px of [128, 256, 512]) render(px, join(icons, `${px}x${px}.png`));

// An .ico of PNG frames (Windows Vista and later read PNG entries): the sizes the shell and the taskbar pick from at
// 100-200% scaling; a 256 px frame is stored with width / height 0, as the format says
const sizes = [16, 20, 24, 32, 40, 48, 64, 256];
const frames = sizes.map(px => {
  const file = join(set, `ico_${px}.png`);
  render(px, file);
  return readFileSync(file);
});
const header = Buffer.alloc(6 + 16 * frames.length);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(frames.length, 4);
let offset = header.length;
frames.forEach((png, i) => {
  const at = 6 + 16 * i;
  header.writeUInt8(sizes[i] >= 256 ? 0 : sizes[i], at);
  header.writeUInt8(sizes[i] >= 256 ? 0 : sizes[i], at + 1);
  header.writeUInt16LE(1, at + 4);
  header.writeUInt16LE(32, at + 6);
  header.writeUInt32LE(png.length, at + 8);
  header.writeUInt32LE(offset, at + 12);
  offset += png.length;
});
writeFileSync(join(icons, 'icon.ico'), Buffer.concat([header, ...frames]));
rmSync(join(set, '..'), { recursive: true, force: true });
console.log('icons/icon.icns, icons/icon.ico, icons/icon.png, icons/{128,256,512}x… .png');
