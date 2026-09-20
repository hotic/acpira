import { access, constants, stat } from 'node:fs/promises';

// Executable resolution and the child_process spawn shape for agent CLIs. Pure functions — no host imports —
// so the Windows branches can be unit-tested from macOS.

// The path as resolved for spawn, or null. POSIX: p must exist and be executable. Windows has no execute bit and a
// bare name never matches its real file (`grok` is `grok.exe` / `grok.cmd`), so PATHEXT suffixes are tried in order,
// each both as given and lower-cased (the filesystem is case-insensitive but PATHEXT is conventionally upper-case).
export async function resolveExecutable(p: string, platform: NodeJS.Platform, env: NodeJS.ProcessEnv): Promise<string | null> {
  if (platform !== 'win32') return (await canAccess(p, constants.X_OK)) ? p : null;
  const exts = (env.PATHEXT ?? '.COM;.EXE;.BAT;.CMD').split(';').filter(Boolean);
  const variants = [p, ...exts.flatMap(ext => ext === ext.toLowerCase() ? [`${p}${ext}`] : [`${p}${ext}`, `${p}${ext.toLowerCase()}`])];
  for (const candidate of variants) {
    // F_OK alone would accept a directory; a PATH dir holding a folder named like the command is not the binary
    if (await isFile(candidate)) return candidate;
  }
  return null;
}

async function canAccess(p: string, mode: number): Promise<boolean> {
  try { await access(p, mode); return true; } catch { return false; }
}

async function isFile(p: string): Promise<boolean> {
  try { return (await stat(p)).isFile(); } catch { return false; }
}

export interface SpawnSpec {
  command: string;
  args: string[];
  windowsVerbatimArguments?: boolean;
}

// How to spawn the resolved binary. .cmd / .bat files are not executables on Windows — they must go through cmd.exe,
// and Node's own argument quoting corrupts the /c line, so the whole command goes out as one verbatim argument
// escaped the way cross-spawn does it.
// The .cmd path is not yet verified on a real Windows machine; this repo is developed on macOS.
export function spawnSpec(binary: string, args: string[], platform: NodeJS.Platform, env: NodeJS.ProcessEnv): SpawnSpec {
  if (platform === 'win32' && /\.(cmd|bat)$/i.test(binary)) {
    const commandLine = `${escapeCommand(binary)} ${args.map(escapeArgument).join(' ')}`.trimEnd();
    return {
      command: env.ComSpec ?? 'cmd.exe',
      args: ['/d', '/s', '/c', `"${commandLine}"`],
      windowsVerbatimArguments: true,
    };
  }
  return { command: binary, args };
}

// cross-spawn's cmd.exe escaping, verbatim: metacharacters get a caret
function escapeCommand(s: string): string {
  return s.replace(/[()%!^"<>&|]/g, '^$&');
}

function escapeArgument(a: string): string {
  let s = a.replace(/(\\*)"/g, '$1$1\\"').replace(/(\\*)$/, '$1$1');
  s = `"${s}"`;
  return s.replace(/[()%!^"<>&|]/g, '^$&');
}
