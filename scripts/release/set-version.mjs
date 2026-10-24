import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// `pnpm version:set X.Y.Z`: the one place a release changes the version. Every shell ships the same number: the
// workspace root (the IntelliJ plugin reads it), the extension manifest, the desktop app and the Rust workspaces, whose
// Cargo.locks follow through `cargo update --workspace` (CI builds with --locked). test/rustWorkspace.test.ts checks them
const root = fileURLToPath(new URL('../..', import.meta.url));
const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error('usage: pnpm version:set X.Y.Z');
  process.exit(1);
}

// Only the manifest's own "version" line changes, so the files keep their layout
function replaceOnce(path, re, to) {
  const file = join(root, path);
  const text = readFileSync(file, 'utf8');
  if (!re.test(text)) throw new Error(`no version in ${path}`);
  writeFileSync(file, text.replace(re, to));
  console.log(`${path}: ${version}`);
}

for (const path of ['package.json', 'apps/vscode/package.json']) {
  replaceOnce(path, /^(  "version": )"[^"]*"/m, `$1"${version}"`);
}
replaceOnce('rust/Cargo.toml', /(\[workspace\.package\]\nversion = )"[^"]*"/, `$1"${version}"`);
// The desktop app: its own Cargo workspace, and the bundle version in tauri.conf.json
replaceOnce('apps/desktop/Cargo.toml', /(\[package\]\n(?:.*\n)*?version = )"[^"]*"/, `$1"${version}"`);
replaceOnce('apps/desktop/tauri.conf.json', /^(  "version": )"[^"]*"/m, `$1"${version}"`);

for (const dir of ['rust', 'apps/desktop']) {
  const cargo = spawnSync('cargo', ['update', '--workspace', '--offline'], { cwd: join(root, dir), stdio: 'inherit' });
  if (cargo.status !== 0) process.exit(cargo.status ?? 1);
}
