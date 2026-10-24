import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// The acpira.* settings the extension left in VS Code's user settings.json (or Cursor's): while no engine has created the
// shared <ACPIRA_HOME>/settings.json yet, the desktop app's hello carries their shared keys so the first engine seeds the
// file from them (main.ts). settings.json is JSONC: comments and trailing commas are allowed

// Not taken over: the column settings are the desktop app's own (always on the left, over every project), and `agents` is
// the legacy per-machine agent list, which the engine seeds into ~/.acpira/agents.json once. Settings Sync carries it
// between machines, so the Mac's settings.json can hold another OS's commands (Windows paths, seen 2026-10-23); on a
// machine that ran the extension agents.json already exists
const NOT_IMPORTED = new Set(['sessionListPosition', 'sessionScope', 'agents']);

// The user settings files the extension's hosts write, in the order they are tried
export function ideSettingsPaths(platform: NodeJS.Platform, env: NodeJS.ProcessEnv, home: string): string[] {
  const base = platform === 'darwin' ? join(home, 'Library', 'Application Support')
    : platform === 'win32' ? env.APPDATA || join(home, 'AppData', 'Roaming')
    : env.XDG_CONFIG_HOME || join(home, '.config');
  return ['Code', 'Cursor'].map(app => join(base, app, 'User', 'settings.json'));
}

// JSONC to JSON: comments become whitespace and a comma before a closing bracket goes; strings are copied untouched
export function stripJsonc(text: string): string {
  const s = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
  const n = s.length;
  // The next index from j past whitespace and comments
  const skip = (from: number): number => {
    let j = from;
    for (;;) {
      while (j < n && /\s/.test(s[j]!)) j++;
      if (s.startsWith('//', j)) { const end = s.indexOf('\n', j); j = end < 0 ? n : end + 1; continue; }
      if (s.startsWith('/*', j)) { const end = s.indexOf('*/', j + 2); j = end < 0 ? n : end + 2; continue; }
      return j;
    }
  };
  let out = '';
  let i = 0;
  while (i < n) {
    const c = s[i]!;
    if (c === '"') {
      let j = i + 1;
      while (j < n && s[j] !== '"') j += s[j] === '\\' ? 2 : 1;
      out += s.slice(i, j + 1);
      i = j + 1;
    } else if (c === '/' && (s[i + 1] === '/' || s[i + 1] === '*')) {
      out += ' ';
      i = skip(i);
    } else if (c === ',' && (s[skip(i + 1)] === '}' || s[skip(i + 1)] === ']')) {
      i++;
    } else {
      out += c;
      i++;
    }
  }
  return out;
}

// The acpira.* keys of a parsed settings file, without the prefix, minus the ones not taken over
export function acpiraKeys(settings: unknown): Record<string, unknown> {
  if (!settings || typeof settings !== 'object' || Array.isArray(settings)) return {};
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(settings)) {
    if (!key.startsWith('acpira.')) continue;
    const name = key.slice('acpira.'.length);
    if (name && !NOT_IMPORTED.has(name)) out[name] = value;
  }
  return out;
}

// The first settings file that exists and parses, as the keys to import and where they came from
export function readIdeSettings(paths: string[], log: (line: string) => void): { from: string; values: Record<string, unknown> } | undefined {
  for (const path of paths) {
    let text: string;
    try { text = readFileSync(path, 'utf8'); } catch { continue; }
    try {
      return { from: path, values: acpiraKeys(JSON.parse(stripJsonc(text)) as unknown) };
    } catch (e) {
      log(`settings import: ${path} does not parse (${String(e)})`);
    }
  }
  return undefined;
}
