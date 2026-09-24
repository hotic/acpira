import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

// The sidecar reports its crate version in helloOk and `acpira --version`; a release bumps package.json, rust/Cargo.toml and, through
// `cargo update --workspace`, rust/Cargo.lock together (CI builds with --locked)
describe('rust workspace', () => {
  it('carries the extension version', () => {
    const { version } = JSON.parse(read('package.json')) as { version: string };
    expect(read('rust/Cargo.toml').match(/\[workspace\.package\]\nversion = "([^"]+)"/)?.[1]).toBe(version);
    const lock = read('rust/Cargo.lock');
    for (const name of ['acpira-host', 'acpira-shared']) expect(lock.match(new RegExp(`name = "${name}"\\nversion = "([^"]+)"`))?.[1]).toBe(version);
  });
});
