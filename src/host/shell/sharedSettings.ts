import { readFileSync, statSync, unwatchFile, watchFile, type Stats } from 'node:fs';
import { join } from 'node:path';
import { isSharedSetting } from '../../shared/settings';

// The settings every shell shares (<ACPIRA_HOME>/settings.json, rust store/shared_settings.rs), read by a shell for what
// it needs before or beside the engine: its own UI language, the browser panel's look, the page's font, how the desktop
// app opens files. The engine alone writes it (the settings page's setSetting); until the first engine has created it,
// a shell falls back to its own legacy value, the one that engine will seed the file from

export class SharedSettingsFile {
  readonly path: string;
  private values: Record<string, unknown> | undefined;
  private listener: ((cur: Stats, prev: Stats) => void) | undefined;

  constructor(home: string) {
    this.path = join(home, 'settings.json');
    this.values = read(this.path);
  }

  exists(): boolean {
    return this.values !== undefined;
  }

  // A shared key from the file once it exists; otherwise (and for the shell's own keys) the shell's value
  get(key: string, own: (key: string) => unknown): unknown {
    return this.values && isSharedSetting(key) ? this.values[key] : own(key);
  }

  // Polled (one stat a second): the engines replace the file by rename, and ~/.acpira can sit on a volume without
  // reliable native watch events. `onChange` gets the keys whose value moved
  watch(onChange: (keys: string[]) => void): void {
    this.unwatch();
    this.listener = (cur, prev) => {
      if (cur.mtimeMs === prev.mtimeMs && cur.size === prev.size && cur.ino === prev.ino) return;
      const before = this.values ?? {};
      this.values = read(this.path);
      const after = this.values ?? {};
      const keys = [...new Set([...Object.keys(before), ...Object.keys(after)])].filter(k => JSON.stringify(before[k]) !== JSON.stringify(after[k]));
      if (keys.length) onChange(keys);
    };
    watchFile(this.path, { interval: 1000 }, this.listener);
  }

  unwatch(): void {
    if (this.listener) unwatchFile(this.path, this.listener);
    this.listener = undefined;
  }
}

// undefined: no file yet; a file that does not parse reads as empty (the engine replaces it on its next write)
function read(path: string): Record<string, unknown> | undefined {
  try { statSync(path); } catch { return undefined; }
  try {
    const v: unknown = JSON.parse(readFileSync(path, 'utf8'));
    return v && typeof v === 'object' && !Array.isArray(v) ? v as Record<string, unknown> : {};
  } catch {
    return {};
  }
}
