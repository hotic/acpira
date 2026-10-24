import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

// The sidecar reports its crate version in helloOk and `acpira --version`. Every shell ships the same number: `pnpm version:set`
// (scripts/release/set-version.mjs) writes package.json, apps/vscode/package.json, apps/desktop/package.json, rust/Cargo.toml and,
// through `cargo update --workspace`, rust/Cargo.lock (CI builds with --locked). The IntelliJ plugin reads the root package.json
describe('rust workspace', () => {
  it('carries the extension version', () => {
    const { version } = JSON.parse(read('package.json')) as { version: string };
    for (const app of ['apps/vscode', 'apps/desktop']) expect((JSON.parse(read(`${app}/package.json`)) as { version: string }).version).toBe(version);
    expect(read('rust/Cargo.toml').match(/\[workspace\.package\]\nversion = "([^"]+)"/)?.[1]).toBe(version);
    const lock = read('rust/Cargo.lock');
    for (const name of ['acpira-agent', 'acpira-host', 'acpira-rpc', 'acpira-shared']) expect(lock.match(new RegExp(`name = "${name}"\\nversion = "([^"]+)"`))?.[1]).toBe(version);
  });
});
