import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

// The desktop app's own settings store: flat acpira.* keys without the prefix, like VS Code's settings.json section and
// IntelliJ's application storage. Since 2026-10-23 it holds only the settings that follow the window (SHELL_SETTING_KEYS);
// the shared ones live in <ACPIRA_HOME>/settings.json. The engine reads the snapshot (hello) and writes through writeSetting. It lives in Electron's userData, not in ~/.acpira, because the Storage tab files everything there
// by the record that references it and would call an unreferenced file a leftover

// What a first launch starts with: the session column on the left over every project (the app has no workspace)
export const DESKTOP_DEFAULTS: Readonly<Record<string, unknown>> = { sessionListPosition: 'left', sessionScope: 'all' };

export class SettingsStore {
  private values: Record<string, unknown>;
  private listeners: ((keys: string[], snapshot: Record<string, unknown>) => void)[] = [];
  // True when this run created the file (a first launch)
  readonly fresh: boolean;

  constructor(readonly file: string) {
    const read = SettingsStore.read(file);
    this.fresh = !read;
    this.values = read ?? { ...DESKTOP_DEFAULTS };
    if (this.fresh) this.save();
  }

  private static read(file: string): Record<string, unknown> | undefined {
    let text: string;
    try { text = readFileSync(file, 'utf8'); } catch { return undefined; }
    try {
      const v: unknown = JSON.parse(text);
      return v && typeof v === 'object' && !Array.isArray(v) ? v as Record<string, unknown> : {};
    } catch {
      // A broken file is kept aside rather than overwritten, and the app starts on what it can read: nothing
      try { renameSync(file, `${file}.broken`); } catch { /* best effort */ }
      return {};
    }
  }

  snapshot(): Record<string, unknown> {
    return { ...this.values };
  }

  get(key: string): unknown {
    return this.values[key];
  }

  // `undefined` removes the key (back to the engine's default)
  set(key: string, value: unknown) {
    if (value === undefined) delete this.values[key];
    else this.values[key] = value;
    this.save();
    const snap = this.snapshot();
    for (const l of this.listeners) l([key], snap);
  }

  // Several keys at once (an import), one change event
  merge(values: Record<string, unknown>) {
    const keys = Object.keys(values);
    if (!keys.length) return;
    Object.assign(this.values, values);
    this.save();
    const snap = this.snapshot();
    for (const l of this.listeners) l(keys, snap);
  }

  onChange(listener: (keys: string[], snapshot: Record<string, unknown>) => void) {
    this.listeners.push(listener);
  }

  private save() {
    mkdirSync(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.${process.pid}.tmp`;
    writeFileSync(tmp, `${JSON.stringify(this.values, null, 2)}\n`);
    renameSync(tmp, this.file);
  }
}
