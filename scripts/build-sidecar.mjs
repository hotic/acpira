import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SIDECAR_TARGETS, exeName, hostPlatform, targetFor } from './sidecar-targets.mjs';

// Builds the release Rust sidecar into dist/sidecar/<platform>/: `node scripts/build-sidecar.mjs [--all | <platform>...]`, this machine's
// platform by default. macOS targets build on macOS and Windows targets on Windows. Linux targets are static musl builds: natively on a
// Linux machine of that architecture (musl-gcc from musl-tools), through cargo-zigbuild (zig + cargo-zigbuild on PATH) anywhere else
const root = fileURLToPath(new URL('..', import.meta.url));
const args = process.argv.slice(2);
const platforms = args.includes('--all') ? SIDECAR_TARGETS.map(t => t.platform) : args.length ? args : [hostPlatform()];

function run(cmd, argv, cwd = root) {
  console.log(`$ ${cmd} ${argv.join(' ')}`);
  const r = spawnSync(cmd, argv, { cwd, stdio: 'inherit' });
  if (r.status !== 0) throw new Error(`${cmd} ${argv[0]} failed (${r.error?.message ?? `exit ${r.status}`})`);
}

for (const platform of platforms) {
  const { triple } = targetFor(platform);
  run('rustup', ['target', 'add', triple]);
  const zig = triple.includes('-linux-') && platform !== hostPlatform();
  run('cargo', [zig ? 'zigbuild' : 'build', '--release', '--locked', '--target', triple, '--bin', 'acpira'], join(root, 'rust'));
  const exe = exeName(platform);
  const out = join(root, 'dist', 'sidecar', platform);
  mkdirSync(out, { recursive: true });
  copyFileSync(join(root, 'rust', 'target', triple, 'release', exe), join(out, exe));
  if (!exe.endsWith('.exe')) chmodSync(join(out, exe), 0o755);
  console.log(`${platform}: ${(statSync(join(out, exe)).size / 1e6).toFixed(1)} MB → dist/sidecar/${platform}/${exe}`);
}
