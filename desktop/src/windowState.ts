import { readJson, writeJson } from './jsonFile';

// Where the chat windows were and what they showed, so the next launch reopens them (userData/windows.json). Windows open
// at quit are restored as they were; a window closed by hand leaves the list, and the last one closed lends its place to
// the next new window

export interface Bounds { x: number; y: number; width: number; height: number }

export interface WindowEntry {
  bounds?: Bounds;
  maximized?: boolean;
  // The session it showed
  session?: string;
}

export interface WindowsState {
  windows: WindowEntry[];
  last?: WindowEntry;
}

const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

function entryOf(raw: unknown): WindowEntry | undefined {
  if (!raw || typeof raw !== 'object') return undefined;
  const r = raw as Record<string, unknown>;
  const b = r.bounds as Record<string, unknown> | undefined;
  const bounds = b && isNum(b.x) && isNum(b.y) && isNum(b.width) && isNum(b.height) && b.width > 0 && b.height > 0
    ? { x: b.x, y: b.y, width: b.width, height: b.height } : undefined;
  return {
    ...(bounds ? { bounds } : {}),
    ...(r.maximized === true ? { maximized: true } : {}),
    ...(typeof r.session === 'string' && r.session ? { session: r.session } : {}),
  };
}

// The file's content, shape-checked: anything unreadable is left out rather than failing the launch
export function parseWindowsState(raw: unknown): WindowsState {
  const r = raw && typeof raw === 'object' ? raw as Record<string, unknown> : {};
  const windows = Array.isArray(r.windows) ? r.windows.map(entryOf).filter((e): e is WindowEntry => !!e) : [];
  const last = entryOf(r.last);
  return { windows, ...(last ? { last } : {}) };
}

// Saved bounds where they can still show: kept when the middle of the window's top edge lies on a display's work area (a
// monitor unplugged since leaves it off screen), else only the size, capped to the largest work area, for the OS to place
export function placeable(b: Bounds, areas: Bounds[]): Partial<Bounds> {
  const mx = b.x + b.width / 2;
  const on = areas.some(a => mx >= a.x && mx < a.x + a.width && b.y >= a.y && b.y < a.y + a.height);
  if (on) return b;
  const widest = areas.reduce<Bounds | undefined>((m, a) => (!m || a.width * a.height > m.width * m.height ? a : m), undefined);
  return widest ? { width: Math.min(b.width, widest.width), height: Math.min(b.height, widest.height) } : { width: b.width, height: b.height };
}

export class WindowStateStore {
  constructor(readonly file: string) {}

  read(): WindowsState {
    return parseWindowsState(readJson(this.file));
  }

  write(state: WindowsState) {
    writeJson(this.file, state);
  }
}
