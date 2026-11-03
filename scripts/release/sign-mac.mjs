import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { exeName } from './sidecar-targets.mjs';

// Signs the macOS sidecar binaries in dist/sidecar/<platform>/ with a Developer ID Application identity:
//   node scripts/release/sign-mac.mjs <platform>...
// macOS keys its privacy grants (App Management, Full Disk Access, …) on the code signature. An ad hoc signature names
// the binary's hash, so every build reads as a new program and a grant made to the engine remote access's login service
// starts is lost on each install; a Developer ID signature names the team and identifier, which survive builds. The
// identifier matches the one the desktop app's bundle gives the same binary (Contents/MacOS/acpira-engine), so a copy
// staged from the VS Code extension and one staged from the desktop app are one program to macOS. Without an identity (or off macOS) the
// binaries keep at least an ad hoc signature, which Apple silicon requires

const IDENTIFIER = 'acpira-engine';

/// The identity to sign with: APPLE_SIGNING_IDENTITY, else the first Developer ID Application identity in the keychain,
/// else null
export function macSigningIdentity() {
  const set = process.env.APPLE_SIGNING_IDENTITY?.trim();
  if (set) return set === '-' ? null : set;
  if (process.platform !== 'darwin') return null;
  const r = spawnSync('security', ['find-identity', '-v', '-p', 'codesigning'], { encoding: 'utf8' });
  return r.stdout?.match(/"(Developer ID Application: [^"]+)"/)?.[1] ?? null;
}

/// Sign one binary under `identifier`; ad hoc when `identity` is null and the binary carries no valid signature yet
export function signMacBinary(path, identifier, identity) {
  if (process.platform !== 'darwin') return;
  if (!identity) {
    if (spawnSync('codesign', ['-v', path]).status === 0) return;
    identity = '-';
  }
  // Hardened runtime as in the desktop bundle; a secure timestamp only in CI, so a local build needs no network
  const flags = identity === '-' ? [] : ['--options', 'runtime', process.env.CI ? '--timestamp' : '--timestamp=none'];
  const r = spawnSync('codesign', ['--force', '--sign', identity, '--identifier', identifier, ...flags, path], { stdio: 'inherit' });
  if (r.status !== 0) throw new Error(`codesign ${path} failed (${r.error?.message ?? `exit ${r.status}`})`);
}

/// Sign the engine of a macOS platform's dist/sidecar/ directory; other platforms are left alone
export function signMacSidecar(root, platform, identity = macSigningIdentity()) {
  if (!platform.startsWith('mac-') || process.platform !== 'darwin') return;
  const path = join(root, 'dist', 'sidecar', platform, exeName(platform));
  if (!existsSync(path)) throw new Error(`${path} is missing: run \`node scripts/release/build-sidecar.mjs ${platform}\` first`);
  signMacBinary(path, IDENTIFIER, identity);
  console.log(`${platform}: signed ${identity ? `with ${identity}` : 'ad hoc (no Developer ID identity found)'}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../..', import.meta.url));
  const platforms = process.argv.slice(2);
  if (!platforms.length) {
    console.error('usage: node scripts/release/sign-mac.mjs <platform>...');
    process.exit(2);
  }
  const identity = macSigningIdentity();
  for (const platform of platforms) signMacSidecar(root, platform, identity);
}
