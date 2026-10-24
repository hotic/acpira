import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

// The app's small state files under userData (window placement, the browser panel's recent pages): read leniently, written
// tmp + rename so a crash mid-write never leaves half a file

export function readJson(file: string): unknown {
  try { return JSON.parse(readFileSync(file, 'utf8')) as unknown; } catch { return undefined; }
}

// Best effort: this state is a convenience, and a read-only profile must not break the app
export function writeJson(file: string, value: unknown) {
  try {
    mkdirSync(dirname(file), { recursive: true });
    const tmp = `${file}.${process.pid}.tmp`;
    writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`);
    renameSync(tmp, file);
  } catch { /* see above */ }
}
