import { contextBridge, ipcRenderer, webUtils } from 'electron';

// The transport the webview bundle expects from a host (packages/ui/vscodeApi.ts): window.__acpiraApi.postMessage up,
// window `message` events down, plus the webview's own persisted state. Runs sandboxed with context isolation, so the
// page only ever sees these functions. `pathForFile` is the desktop addition: the real path of a file dropped from Finder
// (File.path is gone from current Electron; webUtils is the replacement)

const STATE_KEY = 'acpira.webviewState';

contextBridge.exposeInMainWorld('__acpiraApi', {
  postMessage: (message: unknown) => ipcRenderer.send('acpira:webview', message),
  getState: (): unknown => {
    try { return JSON.parse(sessionStorage.getItem(STATE_KEY) ?? 'null') ?? undefined; } catch { return undefined; }
  },
  setState: (state: unknown) => {
    try { sessionStorage.setItem(STATE_KEY, JSON.stringify(state)); } catch { /* quota: the state is a convenience */ }
  },
  pathForFile: (file: File): string | undefined => {
    try { return webUtils.getPathForFile(file) || undefined; } catch { return undefined; }
  },
});

// Host messages arrive like a VS Code webview's: a structured clone into the page's own world
ipcRenderer.on('acpira:host', (_e, message: unknown) => window.postMessage(message, '*'));

// The system appearance changed: the page follows the body class (packages/ui/useVsCodeTheme.ts)
ipcRenderer.on('acpira:theme', (_e, dark: boolean) => {
  document.body.classList.toggle('vscode-dark', dark);
  document.body.classList.toggle('vscode-light', !dark);
});
