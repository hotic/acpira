import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { crc32, deflateRawSync } from 'node:zlib';

// The Windows x64 app: one portable Acpira.exe that carries the engine (`acpira_host::cli`) and the page (build.rs embeds
// dist/webview and dist/picker.js), plus Acpira-<version>-windows-x86_64.zip holding it. On Windows the shell builds
// natively (MSVC); elsewhere through cargo-xwin, which needs clang-cl and llvm-rc on PATH (docs/dev/desktop.md, Windows).
// The page and the picker must already be built in the repository.
//
//   node scripts/package-windows.mjs [--smoke]
//
// --smoke compiles the smoke hook in (`node scripts/smoke.mjs --app <Acpira.exe>`); never ship it.
// Output: target/windows/Acpira.exe and target/windows/Acpira-<version>-windows-x86_64.zip (unsigned)
const TRIPLE = 'x86_64-pc-windows-msvc';
const shell = fileURLToPath(new URL('..', import.meta.url));
const repo = join(shell, '..', '..');

for (const [path, how] of [['dist/webview/main.js', 'pnpm build'], ['dist/picker.js', 'pnpm build']]) {
  if (!existsSync(join(repo, path))) {
    console.error(`missing ${path}: run \`${how}\` in the repository root`);
    process.exit(1);
  }
  const age = (Date.now() - statSync(join(repo, path)).mtimeMs) / 3_600_000;
  console.log(`${path}: built ${age < 1 ? `${Math.round(age * 60)} min` : `${age.toFixed(1)} h`} ago`);
}

const smoke = process.argv.includes('--smoke');
const cargo = process.platform === 'win32' ? ['cargo', ['build']] : ['cargo', ['xwin', 'build']];
const args = [...cargo[1], '--release', '--locked', '--target', TRIPLE, ...(smoke ? ['--features', 'smoke'] : [])];
// Fail rather than ship an exe without its page (build.rs only warns, for development builds)
const env = { ...process.env, ACPIRA_EMBED_PAGE: '1' };
const r = spawnSync(cargo[0], args, { cwd: shell, stdio: 'inherit', env });
if (r.status !== 0) process.exit(r.status ?? 1);

const version = JSON.parse(readFileSync(join(shell, 'tauri.conf.json'), 'utf8')).version;
const out = join(shell, 'target', 'windows');
mkdirSync(out, { recursive: true });
const exe = join(out, 'Acpira.exe');
copyFileSync(join(shell, 'target', TRIPLE, 'release', 'acpira-desktop-tauri.exe'), exe);
const zip = join(out, `Acpira-${version}-windows-x86_64${smoke ? '-smoke' : ''}.zip`);
rmSync(zip, { force: true });
if (!zipWith7z(zip, exe)) writeFileSync(zip, zipOne('Acpira.exe', readFileSync(exe), statSync(exe).mtime));

// Decimal megabytes, as scripts/package.mjs prints them
const mb = n => `${(n / 1e6).toFixed(1)} MB`;
console.log(`${exe}: ${mb(statSync(exe).size)}`);
console.log(`${zip}: ${mb(statSync(zip).size)}`);

// The same zip through 7-Zip where one is installed (GitHub's Windows runners have it): its Deflate encoder is still
// method 8, which Explorer's "Extract all" reads, and ~1 MB (6%) smaller than zlib's level 9 on the exe (2026-10-26).
// False when there is none, or it failed
function zipWith7z(zip, exe) {
  const candidates = ['7z', '7zz', ...(process.platform === 'win32' ? ['C:\\Program Files\\7-Zip\\7z.exe'] : [])];
  for (const tool of candidates) {
    const r = spawnSync(tool, ['a', '-tzip', '-mx=9', '-mfb=258', '-mpass=15', zip, 'Acpira.exe'], { cwd: out, stdio: 'ignore' });
    if (r.error) continue;
    if (r.status === 0 && existsSync(zip)) {
      console.log(`zip: ${tool}`);
      return true;
    }
    rmSync(zip, { force: true });
  }
  return false;
}

// A zip archive of one file, deflated (what Explorer's "Extract all" and every unzip read); written here so packaging
// needs no zip tool on any of the three hosts
function zipOne(name, data, mtime) {
  const nameBytes = Buffer.from(name, 'utf8');
  const deflated = deflateRawSync(data, { level: 9 });
  const crc = crc32(data);
  // MS-DOS time and date, local time, two-second steps
  const time = (mtime.getHours() << 11) | (mtime.getMinutes() << 5) | Math.floor(mtime.getSeconds() / 2);
  const date = ((mtime.getFullYear() - 1980) << 9) | ((mtime.getMonth() + 1) << 5) | mtime.getDate();
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(20, 4); // version needed: 2.0 (deflate)
  local.writeUInt16LE(0x0800, 6); // the name is UTF-8
  local.writeUInt16LE(8, 8); // deflate
  local.writeUInt16LE(time, 10);
  local.writeUInt16LE(date, 12);
  local.writeUInt32LE(crc, 14);
  local.writeUInt32LE(deflated.length, 18);
  local.writeUInt32LE(data.length, 22);
  local.writeUInt16LE(nameBytes.length, 26);
  local.writeUInt16LE(0, 28);
  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50, 0);
  central.writeUInt16LE(20, 4); // made by: MS-DOS attributes, 2.0
  central.writeUInt16LE(20, 6);
  central.writeUInt16LE(0x0800, 8);
  central.writeUInt16LE(8, 10);
  central.writeUInt16LE(time, 12);
  central.writeUInt16LE(date, 14);
  central.writeUInt32LE(crc, 16);
  central.writeUInt32LE(deflated.length, 20);
  central.writeUInt32LE(data.length, 24);
  central.writeUInt16LE(nameBytes.length, 28);
  // extra, comment, disk, internal attributes 0; external attributes 0 (a plain file); local header at offset 0
  const end = Buffer.alloc(22);
  const centralSize = central.length + nameBytes.length;
  const centralOffset = local.length + nameBytes.length + deflated.length;
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(1, 8);
  end.writeUInt16LE(1, 10);
  end.writeUInt32LE(centralSize, 12);
  end.writeUInt32LE(centralOffset, 16);
  return Buffer.concat([local, nameBytes, deflated, central, nameBytes, end]);
}
