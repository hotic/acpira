import { spawn } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// runInTerminal (logins, installs, keychain unlocks): the system's own terminal runs the line the engine resolved. macOS
// opens a .command script in Terminal, which runs it under the user's login shell as VS Code's integrated terminal would,
// and needs no Automation permission (an AppleScript `do script` does). Linux hands the same script to the desktop's
// terminal. Windows starts the engine's prepared invocation (`powershell.exe -NoExit -EncodedCommand …`, environment edits
// folded in) in a console of its own

export interface TerminalRequest {
  title: string;
  command: string;
  args: string[];
  // null removes a variable
  env?: Record<string, string | null>;
}

const quote = (s: string) => (/^[\w./=:@%+-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);
const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

// The POSIX script for a request. It removes itself and its folder first (the shell keeps reading the open file), names the
// window, applies the environment edits and runs the line; the window stays open on its output
export function terminalScript(r: TerminalRequest): string {
  const lines = ['#!/bin/sh', 'rm -f "$0"; rmdir "$(dirname "$0")" 2>/dev/null', `printf '\\033]0;%s\\007' ${quote(r.title)}`];
  for (const [name, value] of Object.entries(r.env ?? {})) {
    // A name no shell can set never reaches the script
    if (!ENV_NAME.test(name)) continue;
    lines.push(value === null ? `unset ${name}` : `export ${name}=${quote(value)}`);
  }
  lines.push([r.command, ...r.args].map(quote).join(' '));
  return `${lines.join('\n')}\n`;
}

// The terminal command for a script on Linux: the Debian alternative most desktops provide, then plain xterm
const LINUX_TERMINALS: [string, string[]][] = [['x-terminal-emulator', ['-e']], ['xterm', ['-e']]];

export function runInTerminal(r: TerminalRequest, log: (line: string) => void, platform = process.platform): void {
  if (platform === 'win32') {
    const child = spawn(r.command, r.args, { detached: true, stdio: 'ignore', windowsHide: false });
    child.on('error', e => log(`terminal failed: ${String(e)}`));
    child.unref();
    return;
  }
  const dir = mkdtempSync(join(tmpdir(), 'acpira-term-'));
  const file = join(dir, platform === 'darwin' ? 'run.command' : 'run.sh');
  writeFileSync(file, terminalScript(r), { mode: 0o700 });
  if (platform === 'darwin') {
    const child = spawn('open', ['-a', 'Terminal', file], { stdio: 'ignore', detached: true });
    child.on('error', e => log(`terminal failed: ${String(e)}`));
    child.unref();
    return;
  }
  const tryAt = (i: number) => {
    const entry = LINUX_TERMINALS[i];
    if (!entry) { log(`terminal failed: no terminal emulator found for ${r.title}`); return; }
    const [bin, flags] = entry;
    const child = spawn(bin, [...flags, 'sh', file], { stdio: 'ignore', detached: true });
    child.on('error', () => tryAt(i + 1));
    child.unref();
  };
  tryAt(0);
}

