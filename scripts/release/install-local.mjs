import { spawnSync } from 'node:child_process';
import { existsSync, globSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { hostPlatform, targetFor } from './sidecar-targets.mjs';

// Installs what scripts/release/package-local.mjs left on this machine: the VSIX into every VS Code / Cursor whose CLI is
// found, and on macOS the packaged Acpira.app into /Applications (a running copy is asked to quit first).
//   node scripts/release/install-local.mjs [--vsix | --desktop]
// Open editor windows keep the old extension until they reload (each offers it once it notices the replaced install);
// the first shell to start the new engine binary hands the machine's engine over to it (machine_engine/ensure.rs)
const root = fileURLToPath(new URL('../..', import.meta.url));
const args = process.argv.slice(2);
const doVsix = !args.includes('--desktop');
const doDesktop = !args.includes('--vsix');
const age = path => {
  const min = (Date.now() - statSync(path).mtimeMs) / 60_000;
  return min < 60 ? `${Math.round(min)} min` : `${(min / 60).toFixed(1)} h`;
};
const which = cmd => spawnSync('/usr/bin/env', ['which', cmd], { encoding: 'utf8' }).stdout.trim() || undefined;
let failed = false;

if (doVsix) {
  const { name } = JSON.parse(readFileSync(join(root, 'apps', 'vscode', 'package.json'), 'utf8'));
  // The newest package for this platform: local ones carry a prerelease version of their own (local-version.mjs)
  const target = targetFor(hostPlatform()).vsce[0];
  const vsix = globSync(`${name}-*-${target}.vsix`, { cwd: root })
    .map(f => join(root, f))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0] ?? join(root, `${name}-<version>-${target}.vsix`);
  if (!existsSync(vsix)) {
    console.error(`${vsix} is missing: run \`pnpm package\` first`);
    failed = true;
  } else {
    console.log(`${vsix}: built ${age(vsix)} ago`);
    // The editors' CLIs: on PATH, else inside the macOS app bundles
    const editors = [
      ['VS Code', which('code') ?? '/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code'],
      ['Cursor', which('cursor') ?? '/Applications/Cursor.app/Contents/Resources/app/bin/cursor'],
    ].filter(([, cli]) => existsSync(cli));
    if (!editors.length) console.log('no VS Code or Cursor CLI found; install the VSIX by hand (Extensions: Install from VSIX…)');
    for (const [editor, cli] of editors) {
      console.log(`$ ${editor}: --install-extension --force`);
      const r = spawnSync(cli, ['--install-extension', vsix, '--force'], { stdio: 'inherit' });
      if (r.status !== 0) failed = true;
    }
  }
}

if (doDesktop && process.platform === 'darwin') {
  const built = join(root, 'apps', 'desktop', 'target', 'aarch64-apple-darwin', 'release', 'bundle', 'macos', 'Acpira.app');
  const dest = '/Applications/Acpira.app';
  if (!existsSync(built)) {
    console.error(`${built} is missing: run \`pnpm package\` first`);
    failed = true;
  } else {
    console.log(`${built}: built ${age(built)} ago`);
    // The window only: the machine's engine may run under the bundle's acpira-engine (machine_engine/launcher.rs) and
    // outlives it
    const running = () => spawnSync('pgrep', ['-f', `^${dest}/Contents/MacOS/Acpira( |$)`]).status === 0;
    if (running()) {
      console.log('Acpira is running: asking it to quit');
      spawnSync('osascript', ['-e', 'tell application id "cc.hotic.acpira.desktop" to quit']);
      const until = Date.now() + 20_000;
      while (running() && Date.now() < until) spawnSync('sleep', ['0.5']);
    }
    if (running()) {
      console.error('Acpira did not quit; quit it and run this again');
      failed = true;
    } else {
      // Copied next to the old bundle first, so a failed copy leaves the installed app alone
      const next = `${dest}.installing`;
      rmSync(next, { recursive: true, force: true });
      const r = spawnSync('ditto', [built, next], { stdio: 'inherit' });
      if (r.status !== 0) {
        rmSync(next, { recursive: true, force: true });
        failed = true;
      } else {
        // The old bundle is moved aside, not deleted: an engine launched from it keeps its privacy grants only while
        // the bundle exists. Earlier ones go once nothing runs from them (the desktop updater prunes the same names)
        const parent = dirname(dest);
        const inUse = dir => {
          const exes = ['Acpira', 'acpira-engine'].map(exe => join(dir, 'Acpira.app', 'Contents', 'MacOS', exe)).filter(existsSync);
          // lsof without a file lists every process
          return exes.length > 0 && spawnSync('lsof', ['-t', ...exes], { encoding: 'utf8' }).stdout.trim() !== '';
        };
        for (const name of readdirSync(parent)) {
          const dir = join(parent, name);
          if (name.startsWith('.Acpira-local-') && name.endsWith('.staging') && !inUse(dir)) rmSync(dir, { recursive: true, force: true });
        }
        if (existsSync(dest)) {
          const aside = join(parent, `.Acpira-local-${Date.now()}.staging`);
          mkdirSync(aside);
          renameSync(dest, join(aside, 'Acpira.app'));
        }
        renameSync(next, dest);
        console.log(`installed ${dest}`);
      }
    }
  }
}

process.exit(failed ? 1 : 0);
