// acquireVsCodeApi() may be called only once per webview; both roots (chat App, settings) go through this memo so a single bundle can host either.
// getState / setState are the webview's own persisted state (survives a reload of the same webview); LAB stubs may omit them
// pathForFile and readClipboard: the desktop app only (apps/desktop/src/init.js), the real path of a file or folder dropped
// from the OS and the clipboard's text without WebKit's paste bubble
export interface VsCodeApi {
  postMessage(msg: unknown): void;
  getState?(): unknown;
  setState?(state: unknown): void;
  // The desktop app answers at once on macOS and through its shell on Windows (a promise)
  pathForFile?(file: File): string | undefined | Promise<string | undefined>;
  readClipboard?(): Promise<string>;
}

declare global {
  interface Window { __acpiraApi?: VsCodeApi }
  function acquireVsCodeApi(): VsCodeApi;
}

// Reached through globalThis (the window in a webview) so modules that only need the memo can also be type-checked and unit-tested without the DOM lib
const scope = globalThis as typeof globalThis & { __acpiraApi?: VsCodeApi };

export function vscodeApi(): VsCodeApi {
  return (scope.__acpiraApi ??= acquireVsCodeApi());
}

// One key of the webview's persisted state: undefined without a host that keeps state, or when never set
export function webviewState(key: string): unknown {
  const state = host()?.getState?.();
  return state && typeof state === 'object' ? (state as Record<string, unknown>)[key] : undefined;
}

// Writes one key (undefined removes it) and keeps the rest of the state as it was
export function setWebviewState(key: string, value: unknown): void {
  const api = host();
  if (!api?.setState) return;
  const previous = api.getState?.();
  const next = { ...(previous && typeof previous === 'object' ? previous as Record<string, unknown> : {}) };
  if (value === undefined) delete next[key];
  else next[key] = value;
  api.setState(next);
}

// The OS path of a file or folder dropped on the page, where the host can tell (the desktop app's init script); undefined
// without such a host, and without any host at all (LAB, tests)
export function droppedPathResolver(): ((file: File) => string | undefined | Promise<string | undefined>) | undefined {
  const api = host();
  return api?.pathForFile && (file => api.pathForFile!(file));
}

// The clipboard's text: through the host where it reads it without asking (the desktop app), else the browser's API
export function readClipboardText(): Promise<string> {
  const read = host()?.readClipboard;
  return (read ? read() : navigator.clipboard.readText()).catch(() => '');
}

function host(): VsCodeApi | undefined {
  try { return vscodeApi(); } catch { return undefined; }
}
