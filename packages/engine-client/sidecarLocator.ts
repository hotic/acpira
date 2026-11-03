import { accessSync, chmodSync, constants, existsSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import type { SidecarCommand } from './SidecarClient';

export interface LocateOpts {
  // The extension's install directory: `bin/acpira[.exe]` in every package; a repository checkout also has `dist/sidecar/<os>-<arch>/`
  root: string;
  env: NodeJS.ProcessEnv;
  platform?: NodeJS.Platform;
  arch?: string;
}

// The Rust sidecar the shell starts: ACPIRA_SIDECAR_BIN when set, else the binary the platform package carries, else (a repository
// checkout run with F5) the one `pnpm build:sidecar` left in dist/sidecar/. Empty when there is none: the extension ships one package
// per supported platform, with no other engine to fall back to
export function sidecarCommands(opts: LocateOpts): SidecarCommand[] {
  const override = opts.env.ACPIRA_SIDECAR_BIN?.trim();
  if (override) return [binary(override)];
  const platform = opts.platform ?? process.platform;
  const exe = platform === 'win32' ? 'acpira.exe' : 'acpira';
  const dir = platformDir(platform, opts.arch ?? process.arch);
  const found = [join(opts.root, 'bin', exe), ...(dir ? [join(opts.root, 'dist', 'sidecar', dir, exe)] : [])]
    .find(path => usable(path, platform));
  return found ? [binary(found)] : [];
}

// The most recently landed engine copy under `<home>/engine/` (rust/crates/acpira-host/src/machine_engine/stage.rs): what a window
// whose extension folder an update deleted still reaches the machine's engine with. It never takes the machine over for a local
// build that copy may be (one an SSH session landed is not this machine's to start)
export function landedEngine(home: string, platform: NodeJS.Platform = process.platform): SidecarCommand | undefined {
  const exe = platform === 'win32' ? 'acpira.exe' : 'acpira';
  const root = join(home, 'engine');
  let names: string[];
  try { names = readdirSync(root); } catch { return undefined; }
  const newest = names
    .filter(n => !n.startsWith('.'))
    .map(n => join(root, n, exe))
    .filter(path => usable(path, platform))
    .map(path => ({ path, mtime: statSync(path).mtimeMs }))
    .sort((a, b) => b.mtime - a.mtime)[0];
  return newest && { command: newest.path, args: ['--no-dev-takeover'], label: `landed ${newest.path}` };
}

function binary(path: string): SidecarCommand {
  return { command: path, args: [], label: `binary ${path}` };
}

// scripts/release/sidecar-targets.mjs naming
function platformDir(platform: NodeJS.Platform, arch: string): string | undefined {
  const os = ({ darwin: 'mac', linux: 'linux', win32: 'windows' } as Partial<Record<NodeJS.Platform, string>>)[platform];
  const cpu = ({ arm64: 'arm64', x64: 'x86_64' } as Record<string, string>)[arch];
  return os && cpu ? `${os}-${cpu}` : undefined;
}

// Archives do not always keep the executable bit; restore it once rather than fail to start
function usable(path: string, platform: NodeJS.Platform): boolean {
  if (!existsSync(path) || !statSync(path).isFile()) return false;
  if (platform === 'win32') return true;
  try { accessSync(path, constants.X_OK); return true; } catch { /* fall through */ }
  try { chmodSync(path, 0o755); return true; } catch { return false; }
}
