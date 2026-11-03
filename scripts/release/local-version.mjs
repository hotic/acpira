import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// The version a local package carries: ACPIRA_VERSION when set, else a prerelease of the next patch stamped with the
// build time (`2.0.7-dev.20261103153000`). It sorts after the release in package.json and before the next one, so the
// machine's engine (rust/crates/acpira-host/src/machine_engine/policy.rs) lets an installed local build replace the
// release it follows, and the next release replace it. CI builds never set it and ship the bare X.Y.Z
export function localVersion(root) {
  if (process.env.ACPIRA_VERSION) return process.env.ACPIRA_VERSION;
  const { version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
  const [major, minor, patch] = version.split('-')[0].split('.').map(Number);
  const stamp = new Date().toISOString().replace(/\D/g, '').slice(0, 14);
  return `${major}.${minor}.${patch + 1}-dev.${stamp}`;
}
