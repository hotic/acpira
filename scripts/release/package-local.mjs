import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { localVersion } from './local-version.mjs';
import { hostPlatform } from './sidecar-targets.mjs';

// This machine's packages from one build: the shared page, picker and release engine are built once, then the VSIX and,
// on an arm64 Mac, the desktop app and dmg (apps/desktop/scripts/package.mjs; its Tauri target is aarch64-apple-darwin only).
//   node scripts/release/package-local.mjs [--vsix | --desktop]
// --vsix / --desktop packages only that one. Installing them is scripts/release/install-local.mjs
const root = fileURLToPath(new URL('../..', import.meta.url));
const args = process.argv.slice(2);
const platform = hostPlatform();
const desktopHere = platform === 'mac-arm64';
const vsix = !args.includes('--desktop');
const desktop = !args.includes('--vsix');
if (args.includes('--desktop') && !desktopHere) {
  console.error(`the desktop app packages on an arm64 Mac only (this is ${platform})`);
  process.exit(1);
}

// One version for the engine binary and the VSIX built below (local-version.mjs); the desktop app keeps its own, which
// its updater compares against releases
const version = localVersion(root);
console.log(`local version ${version}`);
const env = { ...process.env, ACPIRA_VERSION: version };

function run(cmd, argv) {
  console.log(`$ ${cmd} ${argv.join(' ')}`);
  const r = spawnSync(cmd, argv, { cwd: root, stdio: 'inherit', env });
  if (r.status !== 0) process.exit(r.status ?? 1);
}

run('pnpm', ['build']);
run('node', ['scripts/release/build-sidecar.mjs', platform]);
if (vsix) run('node', ['scripts/release/package-vsix.mjs']);
if (desktop && desktopHere) run('node', [join('apps', 'desktop', 'scripts', 'package.mjs')]);
else if (desktop) console.log(`desktop app skipped: it packages on an arm64 Mac only (this is ${platform})`);
