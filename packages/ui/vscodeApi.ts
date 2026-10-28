// acquireVsCodeApi() may be called only once per webview; both roots (chat App, settings) go through this memo so a single bundle can host either.
// getState / setState are the webview's own persisted state (survives a reload of the same webview); LAB stubs may omit them
// pathForFile, dragPaths, readClipboard and copyImage: the desktop app only (apps/desktop/src/init.js), the real paths of
// files and folders dropped from the OS, the clipboard's text without WebKit's paste bubble and an image put on the
// system clipboard as a file
export interface VsCodeApi {
  postMessage(msg: unknown): void;
  getState?(): unknown;
  setState?(state: unknown): void;
  // The desktop app answers at once on macOS and through its shell on Windows (a promise)
  pathForFile?(file: File): string | undefined | Promise<string | undefined>;
  // The desktop app on Linux, whose webview shows the page no file of a drag: the paths of the drag over the page, a
  // folder's ending in '/', empty for a drag without files
  dragPaths?(): string[];
  readClipboard?(): Promise<string>;
  // An image the web clipboard would flatten (an animated GIF keeps only its first frame as PNG): the shell writes the
  // bytes to a temporary file named `name` and puts that file, with the raw image data, on the system clipboard
  copyImage?(data: Uint8Array, name: string): Promise<void>;
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

// The paths of the OS drag over the page where only the host can tell them (VsCodeApi.dragPaths); undefined elsewhere
export function hostDragPaths(): (() => string[]) | undefined {
  const api = host();
  return api?.dragPaths && (() => api.dragPaths!());
}

// The clipboard's text: through the host where it reads it without asking (the desktop app), else the browser's API
export function readClipboardText(): Promise<string> {
  const read = host()?.readClipboard;
  return (read ? read() : navigator.clipboard.readText()).catch(() => '');
}

// The shell's own image copy (VsCodeApi.copyImage): the desktop app's init script, or the VS Code shell answering
// copyImage with copiedImage where its page says it can (window.__acpira.imageCopy); undefined elsewhere
export function hostImageCopy(): ((data: Uint8Array, name: string) => Promise<void>) | undefined {
  const api = host();
  if (api?.copyImage) return (data, name) => api.copyImage!(data, name);
  if (!api || typeof window === 'undefined' || !window.__acpira?.imageCopy) return undefined;
  return (data, name) => new Promise<void>((resolve, reject) => {
    const id = `copy:${++imageCopySeq}`;
    const settle = (e: MessageEvent) => {
      const m = e.data as { type?: string; id?: string; error?: string } | undefined;
      if (m?.type !== 'copiedImage' || m.id !== id) return;
      window.removeEventListener('message', settle);
      if (m.error) reject(new Error(m.error));
      else resolve();
    };
    window.addEventListener('message', settle);
    api.postMessage({ type: 'copyImage', id, data, name });
  });
}

let imageCopySeq = 0;

function host(): VsCodeApi | undefined {
  try { return vscodeApi(); } catch { return undefined; }
}
