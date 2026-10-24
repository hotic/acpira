import { spawnSync } from 'node:child_process';
import { existsSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// The macOS arm64 dmg (electron-builder.yml): the desktop bundles are rebuilt here, the page, the picker and the engine
// must already be built in the repository, and a stale one is called out rather than packaged silently
const repo = fileURLToPath(new URL('../../', import.meta.url));
const inputs = [
  ['dist/webview/main.js', 'pnpm build'],
  ['dist/picker.js', 'pnpm build'],
  ['dist/sidecar/mac-arm64/acpira', 'pnpm build:sidecar mac-arm64'],
];
const missing = inputs.filter(([path]) => !existsSync(repo + path));
if (missing.length) {
  for (const [path, how] of missing) console.error(`missing ${path}: run \`${how}\` in the repository root`);
  process.exit(1);
}
for (const [path] of inputs) {
  const age = (Date.now() - statSync(repo + path).mtimeMs) / 3_600_000;
  console.log(`${path}: built ${age < 1 ? `${Math.round(age * 60)} min` : `${age.toFixed(1)} h`} ago`);
}
const run = (cmd, args) => {
  const r = spawnSync(cmd, args, { stdio: 'inherit', cwd: fileURLToPath(new URL('..', import.meta.url)) });
  if (r.status !== 0) process.exit(r.status ?? 1);
};
run('node', ['build.mjs']);
run('pnpm', ['exec', 'electron-builder', '--mac', 'dmg', '--arm64', '--publish', 'never']);
