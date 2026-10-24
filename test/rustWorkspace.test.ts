import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

// The sidecar reports its crate version in helloOk and `acpira --version`. Every shell ships the same number: `pnpm version:set`
// (scripts/release/set-version.mjs) writes package.json, apps/vscode/package.json, rust/Cargo.toml,
// apps/desktop's Cargo.toml and tauri.conf.json and, through `cargo update --workspace`, the Cargo.locks (CI builds with
// --locked). The IntelliJ plugin reads the root package.json
describe('rust workspace', () => {
  it('carries the extension version', () => {
    const { version } = JSON.parse(read('package.json')) as { version: string };
    expect((JSON.parse(read('apps/vscode/package.json')) as { version: string }).version).toBe(version);
    expect(read('rust/Cargo.toml').match(/\[workspace\.package\]\nversion = "([^"]+)"/)?.[1]).toBe(version);
    const lock = read('rust/Cargo.lock');
    for (const name of ['acpira-agent', 'acpira-desktop', 'acpira-host', 'acpira-rpc', 'acpira-shared']) expect(lock.match(new RegExp(`name = "${name}"\\nversion = "([^"]+)"`))?.[1]).toBe(version);
    // The desktop app (its own Cargo workspace) and its bundle
    expect(read('apps/desktop/Cargo.toml').match(/\[package\]\n(?:.*\n)*?version = "([^"]+)"/)?.[1]).toBe(version);
    expect((JSON.parse(read('apps/desktop/tauri.conf.json')) as { version: string }).version).toBe(version);
  });
});
