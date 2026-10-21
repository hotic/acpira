import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, globSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SIDECAR_TARGETS, exeName, hostPlatform, targetFor } from './sidecar-targets.mjs';

// Packages VSIXes from an existing build (`pnpm build` + dist/sidecar/<platform>/ from build-sidecar.mjs):
//   node scripts/package-vsix.mjs                 this machine's platform package
//   node scripts/package-vsix.mjs --all           every platform package
//   node scripts/package-vsix.mjs <vsce-target>…  chosen platform packages
// Each package carries its sidecar at bin/. There is no universal package: a platform without its own package cannot install the
// extension, since the host only exists as the Rust binary.
// vsce runs in a staging folder holding only what .vscodeignore lets through: run at the root it walks the whole tree first,
// rust/target included (49 GB, about a minute of a one-second package, measured 2026-10-21)
const root = fileURLToPath(new URL('..', import.meta.url));
const { name, version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const args = process.argv.slice(2);
const allTargets = SIDECAR_TARGETS.flatMap(t => t.vsce);
const hostTarget = targetFor(hostPlatform()).vsce[0];
const targets = args.includes('--all') ? allTargets : args.length ? args : [hostTarget];
const vsce = join(root, 'node_modules', '.bin', process.platform === 'win32' ? 'vsce.cmd' : 'vsce');

// The files .vscodeignore lets through: it ignores everything (`**`) and lists the package's files as `!` exceptions.
// bin/ is left out, since each target gets its own sidecar
function packageFiles() {
  const patterns = readFileSync(join(root, '.vscodeignore'), 'utf8').split(/\r?\n/).map(l => l.trim()).filter(l => l.startsWith('!')).map(l => l.slice(1));
  return [...new Set(patterns.filter(p => !p.startsWith('bin/')).flatMap(p => globSync(p, { cwd: root })))].filter(f => statSync(join(root, f)).isFile());
}

for (const f of ['dist/extension.cjs', 'dist/picker.js', 'dist/webview/main.js']) {
  if (!existsSync(join(root, f))) throw new Error(`${f} is missing: run \`pnpm build\` first`);
}

const stage = mkdtempSync(join(tmpdir(), 'acpira-vsix-'));
try {
  for (const f of [...packageFiles(), '.vscodeignore']) {
    mkdirSync(dirname(join(stage, f)), { recursive: true });
    copyFileSync(join(root, f), join(stage, f));
  }
  const bin = join(stage, 'bin');
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
    const argv = ['package', '--no-dependencies', '--target', target, '-o', join(root, `${name}-${version}-${target}.vsix`)];
    console.log(`$ vsce ${argv.join(' ')}`);
    // Windows runs the .cmd shim through the shell, which splits unquoted paths at their spaces
    const win = process.platform === 'win32';
    const q = (x) => (win && /\s/.test(x) ? `"${x}"` : x);
    const r = spawnSync(q(vsce), argv.map(q), { cwd: stage, stdio: 'inherit', shell: win });
    if (r.status !== 0) throw new Error(`vsce package ${target} failed`);
  }
} finally {
  rmSync(stage, { recursive: true, force: true });
}
