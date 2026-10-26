import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SIDECAR_TARGETS, exeName, hostPlatform, targetFor } from './sidecar-targets.mjs';

// Builds the release Rust sidecar into dist/sidecar/<platform>/: `node scripts/release/build-sidecar.mjs [--dist] [--all | <platform>...]`, this machine's
// platform by default. Windows targets build on Windows. macOS targets build on macOS, or anywhere else through cargo-zigbuild with
// SDKROOT pointing at a macOS SDK (a copy of the Mac's /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk). Linux targets are static
// musl builds: natively on a Linux machine of that architecture (musl-gcc from musl-tools), through cargo-zigbuild anywhere else
const root = fileURLToPath(new URL('../..', import.meta.url));
const args = process.argv.slice(2);
const named = args.filter(a => !a.startsWith('--'));
const platforms = args.includes('--all') ? SIDECAR_TARGETS.map(t => t.platform) : named.length ? named : [hostPlatform()];

function run(cmd, argv, cwd = root) {
  console.log(`$ ${cmd} ${argv.join(' ')}`);
  const r = spawnSync(cmd, argv, { cwd, stdio: 'inherit' });
  if (r.status !== 0) throw new Error(`${cmd} ${argv[0]} failed (${r.error?.message ?? `exit ${r.status}`})`);
}

// Shipped builds (CI, --all, or --dist for a local package meant to be installed) use the `dist` profile (fat LTO, one
// codegen unit: smallest binary, slowest build); other local builds keep `release` with LTO off and incremental on
const full = Boolean(process.env.CI) || args.includes('--all') || args.includes('--dist');
const fastLocal = !full;
const profile = full ? 'dist' : 'release';

for (const platform of platforms) {
  const { triple } = targetFor(platform);
  run('rustup', ['target', 'add', triple]);
  const linuxZig = triple.includes('-linux-') && platform !== hostPlatform();
  const macZig = triple.endsWith('-apple-darwin') && process.platform !== 'darwin';
  if (macZig && !process.env.SDKROOT) throw new Error(`${platform} builds off macOS need SDKROOT set to a macOS SDK`);
  const zig = linuxZig || macZig;
  const fastFlags = fastLocal && !linuxZig ? ['--config', 'profile.release.lto=false', '--config', 'profile.release.incremental=true'] : [];
  run('cargo', [zig ? 'zigbuild' : 'build', '--profile', profile, '--locked', '--target', triple, '--bin', 'acpira', ...fastFlags], join(root, 'rust'));
  const exe = exeName(platform);
  const out = join(root, 'dist', 'sidecar', platform);
  mkdirSync(out, { recursive: true });
  copyFileSync(join(root, 'rust', 'target', triple, profile, exe), join(out, exe));
  if (!exe.endsWith('.exe')) chmodSync(join(out, exe), 0o755);
  console.log(`${platform}: ${(statSync(join(out, exe)).size / 1e6).toFixed(1)} MB → dist/sidecar/${platform}/${exe}`);
}
