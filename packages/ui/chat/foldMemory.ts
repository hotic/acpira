import { vscodeApi } from '../vscodeApi';

// Manual fold choices live outside the component tree: a rebuilt message subtree (or a reloaded webview)
// would otherwise reset the process fold the reader had just opened. Keys are per session + turn, the
// most recent choices win when the store is trimmed, and the host's webview state carries it across reloads.
const MAX_ENTRIES = 200;

let folds: Record<string, boolean> | undefined;

function api(): { getState?(): unknown; setState?(state: unknown): void } | undefined {
  try { return vscodeApi(); } catch { return undefined; }
}

function load(): Record<string, boolean> {
  if (folds) return folds;
  const state = api()?.getState?.();
  const stored = state && typeof state === 'object' ? (state as { folds?: unknown }).folds : undefined;
  folds = stored && typeof stored === 'object' ? { ...(stored as Record<string, boolean>) } : {};
  return folds;
}

export function rememberedFold(key: string): boolean | undefined {
  return load()[key];
}

export function rememberFold(key: string, open: boolean): void {
  const store = load();
  // Re-insert so iteration order doubles as recency when trimming
  delete store[key];
  store[key] = open;
  for (const stale of Object.keys(store).slice(0, Math.max(0, Object.keys(store).length - MAX_ENTRIES))) delete store[stale];
  const host = api();
  if (!host?.setState) return;
  const previous = host.getState?.();
  host.setState({ ...(previous && typeof previous === 'object' ? previous : {}), folds: { ...store } });
}

// Tests and LAB fixtures start from a clean slate
export function resetFoldMemory(): void {
  folds = undefined;
}
