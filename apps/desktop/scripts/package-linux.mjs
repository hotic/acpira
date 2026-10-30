import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { packPage } from './pack-page.mjs';

// The Linux x64 packages through the Tauri CLI (tauri.conf.json's bundle.linux): a .deb (Debian, Ubuntu), an .rpm
// (Fedora, openSUSE) and an AppImage (one file that carries WebKitGTK with it). The page, the picker and the static musl
// engine must already be built in the repository. The engine goes to <prefix>/libexec/acpira/acpira-engine (where
// src/app.rs looks for it) with remote access's acpira-gateway beside it (where the engine looks for that), the page and
// the picker under <prefix>/lib/Acpira. Not into <prefix>/bin as Tauri's
// external binary: linuxdeploy, which builds the AppImage, sets an rpath with patchelf on every ELF file under usr/bin
// and usr/lib, and the static-pie engine it rewrites crashes at start (linuxdeploy 1-alpha 07333c6, 2026-10-26).
// The shell links the system's glibc, GTK 3 and WebKitGTK 4.1, so build on the oldest system the packages should run
// on (Ubuntu 22.04 for releases; docs/dev/desktop.md, Linux).
//
//   node scripts/package-linux.mjs [--smoke] [--bundles deb,rpm,appimage]
//
// --smoke compiles the smoke hook in (`node scripts/smoke.mjs --app <the AppImage>`); never ship it. The AppImage step
// downloads the AppImage runtime from GitHub on every build; LDAI_RUNTIME_FILE=<a downloaded runtime-x86_64> hands
// linuxdeploy's AppImage plugin a local copy instead (a build box without GitHub).
const TAURI_CLI = '@tauri-apps/cli@2.12.4';
const TRIPLE = 'x86_64-unknown-linux-gnu';
const shell = fileURLToPath(new URL('..', import.meta.url));
const repo = join(shell, '..', '..');
const option = name => {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : undefined;
};

const inputs = [
  ['dist/webview/main.js', 'pnpm build'],
  ['dist/picker.js', 'pnpm build'],
  ['dist/sidecar/linux-x86_64/acpira', 'node scripts/release/build-sidecar.mjs --dist'],
  ['dist/sidecar/linux-x86_64/acpira-gateway', 'node scripts/release/build-sidecar.mjs --dist'],
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

mkdirSync(join(shell, 'binaries'), { recursive: true });
const engine = join(shell, 'binaries', `acpira-engine-${TRIPLE}`);
copyFileSync(join(repo, 'dist/sidecar/linux-x86_64/acpira'), engine);
const gateway = join(shell, 'binaries', `acpira-gateway-${TRIPLE}`);
copyFileSync(join(repo, 'dist/sidecar/linux-x86_64/acpira-gateway'), gateway);
packPage(repo, join(shell, 'target', 'webview-packed'));

const smoke = process.argv.includes('--smoke');
const bundles = option('--bundles') ?? (smoke ? 'appimage' : 'deb,rpm,appimage');
// --config is merged into tauri.conf.json as a JSON merge patch: null drops the plain page from the resources
const resources = { '../../dist/webview/': null, 'target/webview-packed/': 'webview/' };
// Every package carries the engine and the gateway at the same place (bundle.linux.<format>.files: package path → file)
const files = { '/usr/libexec/acpira/acpira-engine': engine, '/usr/libexec/acpira/acpira-gateway': gateway };
const config = { bundle: { resources, linux: { deb: { files }, rpm: { files }, appimage: { files } } } };
// linuxdeploy, which builds the AppImage, is an AppImage itself: extracted and run where FUSE is missing (containers)
const env = { ...process.env, APPIMAGE_EXTRACT_AND_RUN: '1' };
const what = ['--bundles', bundles, ...(smoke ? ['--features', 'smoke'] : [])];
const r = spawnSync('pnpm', ['dlx', TAURI_CLI, 'build', '--target', TRIPLE, ...what, '--config', JSON.stringify(config)], { cwd: shell, stdio: 'inherit', env });
if (r.status !== 0) process.exit(r.status ?? 1);

// What came out, and how big (decimal megabytes, as the other package scripts print them)
const target = process.env.CARGO_TARGET_DIR || join(shell, 'target');
const release = join(target, TRIPLE, 'release');
const mb = n => `${(n / 1e6).toFixed(1)} MB`;
// The shell under tauri.conf.json's mainBinaryName, which the Tauri CLI renames the cargo binary to
const { version, mainBinaryName } = JSON.parse(readFileSync(join(shell, 'tauri.conf.json'), 'utf8'));
console.log(`shell ${mb(statSync(join(release, mainBinaryName)).size)}, engine ${mb(statSync(join(repo, 'dist/sidecar/linux-x86_64/acpira')).size)} (${version})`);
for (const dir of ['deb', 'rpm', 'appimage']) {
  const d = join(release, 'bundle', dir);
  if (!existsSync(d)) continue;
  for (const f of readdirSync(d).filter(n => /\.(deb|rpm|AppImage)$/.test(n))) console.log(`${dir}/${f}: ${mb(statSync(join(d, f)).size)}`);
}
