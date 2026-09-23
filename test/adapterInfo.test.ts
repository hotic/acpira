import { chmodSync, mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { readAdapterInfo } from '../src/host/acp/adapterInfo';
import type { AgentDef } from '../src/host/acp/AgentRegistry';

// An npm install tree under a temp dir: <root>/node_modules/.bin/<bin> symlinked into the package, packages as
// <root>/node_modules/<pkg>/package.json. Layouts exercised: hoisted (engine next to the adapter package),
// nested (engine inside the adapter's own node_modules), and the Windows .cmd shim fallback.

const ADAPTER_PKG = '@agentclientprotocol/codex-acp';
const ENGINE_PKG = '@openai/codex';

const DEF: AgentDef = {
  id: 'codex', name: 'Codex', command: 'codex-acp', args: [], candidates: [],
  adapter: { package: ADAPTER_PKG, engine: { package: ENGINE_PKG, name: 'Codex', overrideEnv: 'CODEX_PATH' } },
};

const dirs: string[] = [];
function sandbox() {
  const root = mkdtempSync(join(tmpdir(), 'acpira-adapter-'));
  dirs.push(root);
  return root;
}
afterEach(() => { while (dirs.length) rmSync(dirs.pop()!, { recursive: true, force: true }); });

function writePackage(root: string, pkg: string, version: string): string {
  const dir = join(root, 'node_modules', pkg);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'package.json'), JSON.stringify({ name: pkg, version }));
  return dir;
}

// npm's hoisted layout: .bin/<bin> → ../<pkg>/dist/cli.js
function installAdapter(root: string, version = '1.13.0'): string {
  const pkgDir = writePackage(root, ADAPTER_PKG, version);
  mkdirSync(join(pkgDir, 'dist'), { recursive: true });
  const cli = join(pkgDir, 'dist', 'cli.js');
  writeFileSync(cli, '#!/usr/bin/env node\n');
  chmodSync(cli, 0o755);
  const binDir = join(root, 'node_modules', '.bin');
  mkdirSync(binDir, { recursive: true });
  const shim = join(binDir, 'codex-acp');
  symlinkSync(join('..', ADAPTER_PKG, 'dist', 'cli.js'), shim);
  return shim;
}

describe('readAdapterInfo', () => {
  it('reads the adapter and the bundled engine versions off a hoisted install', async () => {
    const root = sandbox();
    const bin = installAdapter(root);
    writePackage(root, ENGINE_PKG, '0.155.1');
    const info = await readAdapterInfo(bin, DEF, {});
    expect(info?.adapter).toMatchObject({ name: ADAPTER_PKG, version: '1.13.0' });
    // realpath resolves the tmpdir's /var → /private/var symlink
    expect(info?.adapter?.root).toBe(realpathSync(join(root, 'node_modules', ADAPTER_PKG)));
    expect(info?.engine).toMatchObject({ name: 'Codex', version: '0.155.1' });
    expect(info?.engine?.override).toBeUndefined();
  });

  it('finds the engine nested inside the adapter package’s own node_modules', async () => {
    const root = sandbox();
    const bin = installAdapter(root);
    writePackage(join(root, 'node_modules', ADAPTER_PKG), ENGINE_PKG, '0.155.1');
    const info = await readAdapterInfo(bin, DEF, {});
    expect(info?.engine).toMatchObject({ name: 'Codex', version: '0.155.1' });
  });

  it('an override env var replaces the bundled engine version with the override path', async () => {
    const root = sandbox();
    const bin = installAdapter(root);
    writePackage(root, ENGINE_PKG, '0.155.1');
    const info = await readAdapterInfo(bin, DEF, { CODEX_PATH: '/opt/custom/codex' });
    expect(info?.engine).toEqual({ name: 'Codex', override: '/opt/custom/codex', overrideEnv: 'CODEX_PATH' });
  });

  it('a missing engine package leaves the name without a version; a missing adapter package leaves the whole adapter field empty', async () => {
    const root = sandbox();
    const bin = installAdapter(root);
    const info = await readAdapterInfo(bin, DEF, {});
    expect(info?.adapter).toMatchObject({ name: ADAPTER_PKG, version: '1.13.0' });
    expect(info?.engine).toEqual({ name: 'Codex' });
    // A binary nowhere near a matching package.json
    const stray = join(root, 'stray-bin');
    writeFileSync(stray, '#!/bin/sh\n');
    chmodSync(stray, 0o755);
    const none = await readAdapterInfo(stray, DEF, {});
    expect(none).toEqual({ engine: { name: 'Codex' } });
  });

  it('on win32 a .cmd shim resolves via <bin>/node_modules/<pkg>', async () => {
    const root = sandbox();
    // Windows global layout: <prefix>/codex-acp.cmd next to <prefix>/node_modules/<pkg>
    const shim = join(root, 'codex-acp.cmd');
    writeFileSync(shim, '@echo off\n');
    writePackage(root, ADAPTER_PKG, '1.13.0');
    writePackage(root, ENGINE_PKG, '0.155.1');
    const info = await readAdapterInfo(shim, DEF, {}, 'win32');
    expect(info?.adapter).toMatchObject({ name: ADAPTER_PKG, version: '1.13.0' });
    expect(info?.engine).toMatchObject({ name: 'Codex', version: '0.155.1' });
  });

  it('an agent without adapter metadata gets no adapter info', async () => {
    const root = sandbox();
    const bin = installAdapter(root);
    const plain: AgentDef = { id: 'x' as AgentDef['id'], name: 'X', command: 'x', args: [], candidates: [] };
    expect(await readAdapterInfo(bin, plain, {})).toBeUndefined();
  });
});
