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

  // The Windows desktop exe compiles the engine crates inside apps/desktop's own workspace, so its Cargo.lock must hold
  // every registry crate those crates reach at the version rust/Cargo.lock (what the engine's suites ran against) locks;
  // Tauri's own extra versions may sit beside them, and crates only the engine workspace's other members reach (the
  // gateway's network stack) are not the desktop's. A lock that drifted: seed apps/desktop/Cargo.lock with
  // rust/Cargo.lock's entries
  it('locks the engine crates of the desktop app at the versions the engine tests', () => {
    interface Pkg { name: string; version: string; registry: boolean; deps: string[] }
    const parse = (lock: string): Pkg[] => lock.split('\n[[package]]\n').slice(1).map(block => ({
      name: block.match(/^name = "(.*)"$/m)![1]!,
      version: block.match(/^version = "(.*)"$/m)![1]!,
      registry: block.includes('\nsource = '),
      deps: [...(block.match(/^dependencies = \[\n([\s\S]*?)\n\]/m)?.[1] ?? '').matchAll(/^ "(.*)",?$/gm)].map(m => m[1]!),
    }));
    const engine = parse(read('rust/Cargo.lock'));
    const desktop = parse(read('apps/desktop/Cargo.lock'));
    // From the engine's own crates the desktop builds, everything they reach in the engine's lock. A dependency is
    // written `name` when the lock holds one version of it, `name version` otherwise
    const resolve = (dep: string) => {
      const [name, version] = dep.split(' ');
      return engine.find(p => p.name === name && (!version || p.version === version))!;
    };
    const queue = engine.filter(p => !p.registry && desktop.some(d => d.name === p.name));
    const reached = new Set<Pkg>(queue);
    while (queue.length) {
      for (const dep of queue.pop()!.deps.map(resolve)) {
        if (!reached.has(dep)) {
          reached.add(dep);
          queue.push(dep);
        }
      }
    }
    const locked = new Set(desktop.map(p => `${p.name} ${p.version}`));
    const drifted = [...reached].filter(p => p.registry && desktop.some(d => d.name === p.name) && !locked.has(`${p.name} ${p.version}`));
    expect(drifted.map(p => `${p.name} ${p.version}`)).toEqual([]);
  });
});
