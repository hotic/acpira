import { readFile, realpath } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import type { AdapterInfo } from '@shared/inventory';
import type { AgentDef } from './AgentRegistry';

// Version diagnostics for npm-packaged ACP adapters (codex-acp, claude-agent-acp). Everything is read off
// package.json files — the CLIs are never invoked, and nothing throws: a missing piece stays undefined.

const MAX_UP = 8;

// package.json at dir when it belongs to `name`; version is optional on disk though always present in practice
async function readPackage(dir: string, name: string): Promise<{ version?: string } | undefined> {
  try {
    const pkg: unknown = JSON.parse(await readFile(join(dir, 'package.json'), 'utf8'));
    if (typeof pkg !== 'object' || pkg === null || (pkg as { name?: unknown }).name !== name) return undefined;
    const version = (pkg as { version?: unknown }).version;
    return { version: typeof version === 'string' ? version : undefined };
  } catch {
    return undefined;
  }
}

// The adapter package's install root. realpath turns the .bin symlink into a file inside the package, then the
// walk up finds its package.json. Windows shims (.cmd / .ps1) don't resolve into the package — they sit in the
// npm bin dir with node_modules as a sibling, so that layout is tried instead
async function adapterRoot(binary: string, pkg: string, platform: NodeJS.Platform): Promise<string | undefined> {
  if (platform === 'win32' && /\.(cmd|ps1|bat)$/i.test(binary)) {
    const dir = join(dirname(binary), 'node_modules', pkg);
    return (await readPackage(dir, pkg)) !== undefined ? dir : undefined;
  }
  let dir = await realpath(binary).then(dirname).catch(() => dirname(binary));
  for (let i = 0; i < MAX_UP; i++) {
    if (await readPackage(dir, pkg) !== undefined) return dir;
    const up = dirname(dir);
    if (up === dir) return undefined;
    dir = up;
  }
  return undefined;
}

// The bundled runtime's package.json: nested inside the adapter's own node_modules first, then each shared
// node_modules above it (hoisted npm / pnpm layouts). require.resolve is no use — exports maps can hide package.json
async function engineVersion(fromDir: string, pkg: string): Promise<string | undefined> {
  let dir = fromDir;
  for (let i = 0; i < MAX_UP; i++) {
    const hit = await readPackage(join(dir, 'node_modules', pkg), pkg);
    if (hit !== undefined) return hit.version;
    const up = dirname(dir);
    if (up === dir) return undefined;
    dir = up;
  }
  return undefined;
}

export async function readAdapterInfo(binary: string, def: AgentDef, env: NodeJS.ProcessEnv, platform: NodeJS.Platform = process.platform): Promise<AdapterInfo | undefined> {
  const spec = def.adapter;
  if (!spec) return undefined;
  const out: AdapterInfo = {};
  const root = await adapterRoot(binary, spec.package, platform);
  if (root !== undefined) out.adapter = { name: spec.package, version: (await readPackage(root, spec.package))?.version, root };
  const engine = spec.engine;
  if (engine) {
    // The agent's own env may also set the override (a custom AgentDef.env beats the process environment)
    const override = { ...env, ...def.env }[engine.overrideEnv];
    if (override) out.engine = { name: engine.name, override, overrideEnv: engine.overrideEnv };
    else if (root !== undefined) out.engine = { name: engine.name, version: await engineVersion(root, engine.package) };
    else out.engine = { name: engine.name };
  }
  return out;
}
