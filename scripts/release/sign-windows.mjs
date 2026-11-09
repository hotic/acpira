import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { exeName } from './sidecar-targets.mjs';

// Authenticode-signs Windows executables with the Certum SimplySign cloud certificate through `ssign`
// (crates.io `ssign`, plain HTTPS to Certum, works on any OS):
//   node scripts/release/sign-windows.mjs <file.exe>...
// Smart App Control has no "run anyway" and checks every exe it starts, so both the desktop Acpira.exe and the engine
// the VSIX / IntelliJ zips carry must be signed. Credentials come from the environment only, never argv:
// CERTUM_EMAIL plus CERTUM_OTP (the TOTP seed, or its otpauth:// URI) for unattended runs, or CERTUM_TOKEN (a current
// 6-digit code) by hand. Without them the files stay unsigned, as local builds do.

const DESCRIPTION = 'Acpira';
const URL_INFO = 'https://acpira.com';
// Parallel CI jobs share one seed, so two logins can land on the same 6-digit code; a retry after one TOTP period
// gets a fresh one
const ATTEMPTS = 3;
const RETRY_WAIT_MS = 35_000;

/// True when the environment carries Certum credentials
export function windowsSigningConfigured() {
  return Boolean(process.env.CERTUM_EMAIL?.trim() && (process.env.CERTUM_OTP?.trim() || process.env.CERTUM_TOKEN?.trim()));
}

/// Sign each file in place; a no-op without credentials. Throws when signing was configured and failed, so a release
/// never ships an unsigned exe by accident
export function signWindowsFiles(paths) {
  if (!windowsSigningConfigured()) {
    console.log(`not Authenticode-signed (no CERTUM_EMAIL / CERTUM_OTP): ${paths.join(', ')}`);
    return false;
  }
  for (const path of paths) {
    if (!existsSync(path)) throw new Error(`${path} is missing`);
    for (let attempt = 1; ; attempt++) {
      // ssign caches the cloud session for 20 minutes, so every file after the first reuses one login
      const r = spawnSync('ssign', ['-n', DESCRIPTION, '-u', URL_INFO, path], { stdio: 'inherit' });
      if (r.error?.code === 'ENOENT') throw new Error('ssign is not installed: `cargo install ssign --locked`');
      if (r.status === 0) break;
      if (attempt >= ATTEMPTS) throw new Error(`ssign ${path} failed (${r.error?.message ?? `exit ${r.status}`})`);
      console.log(`ssign failed (attempt ${attempt} of ${ATTEMPTS}); retrying with the next one-time code`);
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, RETRY_WAIT_MS);
    }
    console.log(`${path}: Authenticode-signed`);
  }
  return true;
}

/// Sign the engine of a Windows platform's dist/sidecar/ directory; other platforms are left alone
export function signWindowsSidecar(root, platform) {
  if (!platform.startsWith('windows-')) return;
  signWindowsFiles([join(root, 'dist', 'sidecar', platform, exeName(platform))]);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const files = process.argv.slice(2);
  if (!files.length) {
    console.error('usage: node scripts/release/sign-windows.mjs <file.exe>...');
    process.exit(2);
  }
  if (!windowsSigningConfigured()) {
    console.error('set CERTUM_EMAIL and CERTUM_OTP (or CERTUM_TOKEN) first');
    process.exit(2);
  }
  signWindowsFiles(files);
}
