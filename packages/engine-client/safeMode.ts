import { execFile } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// Safe mode (docs/dev/plugins.md): every engine on the machine unloads the user's plugins. A shell switches it through the
// engine binary's `acpira safe-mode on|off`, which writes ~/.acpira/plugins/state.json, so it works while the engine itself
// is stuck or down; running engines follow within a second

// Whether safe mode is on, from the plugin state (false when there is none or it cannot be read)
export function safeModeOn(home: string): boolean {
  try {
    const state = JSON.parse(readFileSync(join(home, 'plugins', 'state.json'), 'utf8')) as { safeMode?: unknown };
    return state.safeMode === true;
  } catch {
    return false;
  }
}

// Switch safe mode with the engine binary; rejects with its error output
export function setSafeMode(binary: string, home: string, on: boolean): Promise<void> {
  return new Promise((resolve, reject) => {
    execFile(binary, ['safe-mode', on ? 'on' : 'off', '--home', home], { timeout: 15_000, windowsHide: true }, (err, _stdout, stderr) => {
      if (err) reject(new Error(stderr.trim() || err.message));
      else resolve();
    });
  });
}
