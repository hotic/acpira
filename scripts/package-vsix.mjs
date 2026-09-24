import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SIDECAR_TARGETS, exeName, hostPlatform, targetFor } from './sidecar-targets.mjs';

// Packages VSIXes from an existing build (`pnpm build` + dist/sidecar/<platform>/ from build-sidecar.mjs):
//   node scripts/package-vsix.mjs                 this machine's platform package
//   node scripts/package-vsix.mjs --all           every platform package
//   node scripts/package-vsix.mjs <vsce-target>…  chosen platform packages
// Each package carries its sidecar at bin/. There is no universal package: a platform without its own package cannot install the
// extension, since the host only exists as the Rust binary
const root = fileURLToPath(new URL('..', import.meta.url));
const { name, version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const args = process.argv.slice(2);
const allTargets = SIDECAR_TARGETS.flatMap(t => t.vsce);
const hostTarget = targetFor(hostPlatform()).vsce[0];
const targets = args.includes('--all') ? allTargets : args.length ? args : [hostTarget];
const bin = join(root, 'bin');

for (const f of ['dist/extension.cjs', 'dist/webview/main.js']) {
  if (!existsSync(join(root, f))) throw new Error(`${f} is missing: run \`pnpm build\` first`);
}

try {
  for (const target of targets) {
    rmSync(bin, { recursive: true, force: true });
    const t = SIDECAR_TARGETS.find(x => x.vsce.includes(target));
    if (!t) throw new Error(`unknown VS Code target ${target}; one of ${allTargets.join(', ')}`);
    const exe = exeName(t.platform);
    const from = join(root, 'dist', 'sidecar', t.platform, exe);
    if (!existsSync(from)) throw new Error(`${from} is missing: run \`node scripts/build-sidecar.mjs ${t.platform}\` first`);
    mkdirSync(bin);
    copyFileSync(from, join(bin, exe));
    if (!exe.endsWith('.exe')) chmodSync(join(bin, exe), 0o755);
    const argv = ['exec', 'vsce', 'package', '--no-dependencies', '--target', target, '-o', `${name}-${version}-${target}.vsix`];
    console.log(`$ pnpm ${argv.join(' ')}`);
    const r = spawnSync('pnpm', argv, { cwd: root, stdio: 'inherit', shell: process.platform === 'win32' });
    if (r.status !== 0) throw new Error(`vsce package ${target} failed`);
  }
} finally {
  rmSync(bin, { recursive: true, force: true });
}
