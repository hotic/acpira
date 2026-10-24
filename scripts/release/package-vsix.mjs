import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, globSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SIDECAR_TARGETS, exeName, hostPlatform, targetFor } from './sidecar-targets.mjs';

// Packages VSIXes from an existing build (`pnpm build` + dist/sidecar/<platform>/ from build-sidecar.mjs):
//   node scripts/release/package-vsix.mjs                 this machine's platform package
//   node scripts/release/package-vsix.mjs --all           every platform package
//   node scripts/release/package-vsix.mjs <vsce-target>…  chosen platform packages
// Each package carries its sidecar at bin/. There is no universal package: a platform without its own package cannot install the
// extension, since the host only exists as the Rust binary.
// vsce runs in a staging folder holding only what apps/vscode/.vscodeignore lets through: run at the root it walks the whole
// tree first, rust/target included (49 GB, about a minute of a one-second package, measured 2026-10-21). The folder has the
// package's layout: the manifest, its nls files and dist/extension.cjs come from apps/vscode, the shared page, picker, icons,
// README, CHANGELOG and LICENSE from the repository root
const root = fileURLToPath(new URL('../..', import.meta.url));
const app = join(root, 'apps', 'vscode');
const { name, version } = JSON.parse(readFileSync(join(app, 'package.json'), 'utf8'));
const args = process.argv.slice(2);
const allTargets = SIDECAR_TARGETS.flatMap(t => t.vsce);
const hostTarget = targetFor(hostPlatform()).vsce[0];
const targets = args.includes('--all') ? allTargets : args.length ? args : [hostTarget];
const vsce = join(app, 'node_modules', '.bin', process.platform === 'win32' ? 'vsce.cmd' : 'vsce');

// The files .vscodeignore lets through, by their path in the package: it ignores everything (`**`) and lists the package's
// files as `!` exceptions, each looked up in apps/vscode first and then the repository root. bin/ is left out, since each
// target gets its own sidecar
function packageFiles() {
  const patterns = readFileSync(join(app, '.vscodeignore'), 'utf8').split(/\r?\n/).map(l => l.trim()).filter(l => l.startsWith('!')).map(l => l.slice(1));
  const files = new Map();
  for (const base of [app, root]) {
    for (const f of patterns.filter(p => !p.startsWith('bin/')).flatMap(p => globSync(p, { cwd: base }))) {
      if (!files.has(f) && statSync(join(base, f)).isFile()) files.set(f, join(base, f));
    }
  }
  return files;
}

for (const f of [join(app, 'dist/extension.cjs'), join(root, 'dist/picker.js'), join(root, 'dist/webview/main.js')]) {
  if (!existsSync(f)) throw new Error(`${f} is missing: run \`pnpm build\` first`);
}

const stage = mkdtempSync(join(tmpdir(), 'acpira-vsix-'));
try {
  for (const [f, from] of [...packageFiles(), ['.vscodeignore', join(app, '.vscodeignore')]]) {
    mkdirSync(dirname(join(stage, f)), { recursive: true });
    copyFileSync(from, join(stage, f));
  }
  // The shipped manifest leaves out the workspace's build scripts and `workspace:` / `catalog:` dependencies, which only
  // mean something inside this repository
  const manifest = JSON.parse(readFileSync(join(stage, 'package.json'), 'utf8'));
  for (const key of ['scripts', 'dependencies', 'devDependencies']) delete manifest[key];
  writeFileSync(join(stage, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  const bin = join(stage, 'bin');
  for (const target of targets) {
    rmSync(bin, { recursive: true, force: true });
    const t = SIDECAR_TARGETS.find(x => x.vsce.includes(target));
    if (!t) throw new Error(`unknown VS Code target ${target}; one of ${allTargets.join(', ')}`);
    const exe = exeName(t.platform);
    const from = join(root, 'dist', 'sidecar', t.platform, exe);
    if (!existsSync(from)) throw new Error(`${from} is missing: run \`node scripts/release/build-sidecar.mjs ${t.platform}\` first`);
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
