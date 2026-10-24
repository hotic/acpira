import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SIDECAR_TARGETS, exeName, hostPlatform, targetFor } from './sidecar-targets.mjs';

// Builds the release Rust sidecar into dist/sidecar/<platform>/: `node scripts/release/build-sidecar.mjs [--all | <platform>...]`, this machine's
// platform by default. Windows targets build on Windows. macOS targets build on macOS, or anywhere else through cargo-zigbuild with
// SDKROOT pointing at a macOS SDK (a copy of the Mac's /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk). Linux targets are static
// musl builds: natively on a Linux machine of that architecture (musl-gcc from musl-tools), through cargo-zigbuild anywhere else
const root = fileURLToPath(new URL('../..', import.meta.url));
const args = process.argv.slice(2);
const platforms = args.includes('--all') ? SIDECAR_TARGETS.map(t => t.platform) : args.length ? args : [hostPlatform()];

function run(cmd, argv, cwd = root) {
  console.log(`$ ${cmd} ${argv.join(' ')}`);
  const r = spawnSync(cmd, argv, { cwd, stdio: 'inherit' });
  if (r.status !== 0) throw new Error(`${cmd} ${argv[0]} failed (${r.error?.message ?? `exit ${r.status}`})`);
}

const fastLocal = !process.env.CI && !args.includes('--all');

for (const platform of platforms) {
  const { triple } = targetFor(platform);
  run('rustup', ['target', 'add', triple]);
  const linuxZig = triple.includes('-linux-') && platform !== hostPlatform();
  const macZig = triple.endsWith('-apple-darwin') && process.platform !== 'darwin';
  if (macZig && !process.env.SDKROOT) throw new Error(`${platform} builds off macOS need SDKROOT set to a macOS SDK`);
  const zig = linuxZig || macZig;
  const fastFlags = fastLocal && !linuxZig ? ['--config', 'profile.release.lto=false', '--config', 'profile.release.incremental=true'] : [];
  run('cargo', [zig ? 'zigbuild' : 'build', '--release', '--locked', '--target', triple, '--bin', 'acpira', ...fastFlags], join(root, 'rust'));
  const exe = exeName(platform);
  const out = join(root, 'dist', 'sidecar', platform);
  mkdirSync(out, { recursive: true });
  copyFileSync(join(root, 'rust', 'target', triple, 'release', exe), join(out, exe));
  if (!exe.endsWith('.exe')) chmodSync(join(out, exe), 0o755);
  console.log(`${platform}: ${(statSync(join(out, exe)).size / 1e6).toFixed(1)} MB → dist/sidecar/${platform}/${exe}`);
}
