import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// The macOS arm64 app and dmg through the Tauri CLI (tauri.conf.json): the page, the picker and the release engine must
// already be built in the repository, and a stale one is called out rather than packaged silently. The engine goes in as
// an external binary, so the bundle's signature covers it with the same identity and hardened runtime as the app. It is
// Contents/MacOS/acpira-engine: plain `acpira` would be the shell's own `Acpira` on a case-insensitive volume.
//
//   node scripts/package.mjs [--layout] [--smoke]
//
// --smoke builds the app alone with the smoke hook compiled in (`node scripts/smoke.mjs --app <the .app>`); never ship it.
//
// The dmg's Finder layout (window size, icon places) is an AppleScript that opens a Finder window on the mounted image;
// it runs only with --layout, so a packaging run in the background leaves the screen alone (Tauri skips it under CI).
//
// Signing: APPLE_SIGNING_IDENTITY names a Developer ID Application identity in the keychain; without it the app is signed
// ad hoc (runs on this Mac only). Notarization and stapling follow when the Tauri CLI finds APPLE_API_KEY /
// APPLE_API_ISSUER / APPLE_API_KEY_PATH or APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID in the environment
const TAURI_CLI = '@tauri-apps/cli@2.12.4';
const TRIPLE = 'aarch64-apple-darwin';
const shell = fileURLToPath(new URL('..', import.meta.url));
const repo = join(shell, '..', '..');

const inputs = [
  ['dist/webview/main.js', 'pnpm build'],
  ['dist/picker.js', 'pnpm build'],
  ['dist/sidecar/mac-arm64/acpira', 'pnpm build:sidecar mac-arm64'],
];
const missing = inputs.filter(([path]) => !existsSync(join(repo, path)));
if (missing.length) {
  for (const [path, how] of missing) console.error(`missing ${path}: run \`${how}\` in the repository root`);
  process.exit(1);
}
for (const [path] of inputs) {
  const age = (Date.now() - statSync(join(repo, path)).mtimeMs) / 3_600_000;
  console.log(`${path}: built ${age < 1 ? `${Math.round(age * 60)} min` : `${age.toFixed(1)} h`} ago`);
}

// Tauri wants an external binary under its target triple's name
mkdirSync(join(shell, 'binaries'), { recursive: true });
copyFileSync(join(repo, 'dist/sidecar/mac-arm64/acpira'), join(shell, 'binaries', `acpira-engine-${TRIPLE}`));

const identity = process.env.APPLE_SIGNING_IDENTITY?.trim() || '-';
if (identity === '-') console.log('no APPLE_SIGNING_IDENTITY: signing ad hoc, the app runs on this Mac only');
const config = { bundle: { externalBin: ['binaries/acpira-engine'], macOS: { signingIdentity: identity } } };
const env = { ...process.env };
if (process.argv.includes('--layout')) env.TAURI_BUNDLER_DMG_IGNORE_CI = 'true';
else env.CI = 'true';
const smoke = process.argv.includes('--smoke');
const what = smoke ? ['--bundles', 'app', '--features', 'smoke'] : ['--bundles', 'app,dmg'];
const r = spawnSync('pnpm', ['dlx', TAURI_CLI, 'build', '--target', TRIPLE, ...what, '--config', JSON.stringify(config)], { cwd: shell, stdio: 'inherit', env });
if (r.status !== 0) process.exit(r.status ?? 1);

// What came out, and how big
const bundle = join(shell, 'target', TRIPLE, 'release', 'bundle');
const du = path => Number(spawnSync('du', ['-sk', path], { encoding: 'utf8' }).stdout.split('\t')[0]) / 1024;
for (const [dir, ext] of [['macos', '.app'], ['dmg', '.dmg']]) {
  if (!existsSync(join(bundle, dir))) continue;
  for (const f of readdirSync(join(bundle, dir)).filter(n => n.endsWith(ext))) {
    console.log(`${dir}/${f}: ${du(join(bundle, dir, f)).toFixed(1)} MB`);
  }
}
