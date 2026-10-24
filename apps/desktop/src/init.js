// The transport the webview bundle expects from a host (packages/ui/vscodeApi.ts): window.__acpiraApi.postMessage up,
// window `message` events down, plus the page's own persisted state. Injected before the page's scripts in the top frame
// of the app's own pages (the shell's initialization script); a browser tab of the tool panel is a frame and talks
// through its parent instead (window.__acpiraBrowserTab, set by the page)
(() => {
  if (window.top !== window || location.protocol !== 'acpira-app:' || window.__acpiraApi) return;
  const T = window.__TAURI_INTERNALS__;
  const STATE_KEY = 'acpira.webviewState';
  const encoder = new TextEncoder();

  // Up: one invoke at a time, carrying everything posted meanwhile as one JSON array, so the shell sees the page's
  // messages in the order they were posted (separate invokes are separate requests and may overtake each other)
  let queue = [];
  let busy = true;
  const flush = () => {
    if (busy || !queue.length) return;
    busy = true;
    const batch = `[${queue.join(',')}]`;
    queue = [];
    T.invoke('acpira_post', encoder.encode(batch)).catch(e => console.error('acpira_post failed', e)).finally(() => { busy = false; flush(); });
  };

  // Down: a Tauri channel. Small messages are evaluated straight in, large ones fetched, so each carries its index and
  // the page hands them on in order, like a VS Code webview's message events
  let expected = 0;
  const early = new Map();
  const deliver = data => {
    if (data.end) return;
    window.postMessage(data.message, '*');
  };
  const channel = T.transformCallback(data => {
    if (data.index !== expected) { early.set(data.index, data); return; }
    deliver(data);
    expected++;
    for (let next = early.get(expected); next; next = early.get(expected)) {
      early.delete(expected);
      deliver(next);
      expected++;
    }
  });
  // The channel goes first: nothing is posted before the shell can answer this page
  T.invoke('acpira_attach', { channel: `__CHANNEL__:${channel}` })
    .catch(e => console.error('acpira_attach failed', e))
    .finally(() => { busy = false; flush(); });

  let dragPaths = [];
  document.addEventListener('dragenter', e => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    T.invoke('acpira_drag_paths').then(paths => { dragPaths = paths; }, () => { dragPaths = []; });
  }, true);

  window.__acpiraApi = Object.freeze({
    postMessage: message => { queue.push(JSON.stringify(message)); flush(); },
    getState: () => {
      try { return JSON.parse(sessionStorage.getItem(STATE_KEY) ?? 'null') ?? undefined; } catch { return undefined; }
    },
    setState: state => {
      try { sessionStorage.setItem(STATE_KEY, JSON.stringify(state)); } catch { /* quota: the state is a convenience */ }
    },
    // The system clipboard's text through the shell: WebKit asks the user before navigator.clipboard.readText (a paste
    // bubble), which a terminal's right-click paste must not show
    readClipboard: () => T.invoke('acpira_clipboard'),
    // A dropped file's path: WKWebView gives the page its name and bytes only, so the paths of a Finder drag are read
    // from the system's drag pasteboard as it enters the page and matched by name at the drop (packages/ui Shell's
    // folder drop calls this synchronously)
    pathForFile: file => dragPaths.find(p => p.slice(p.lastIndexOf('/') + 1) === file.name),
  });

  // The system appearance changed (the shell calls this): the page follows the body class (packages/ui/useVsCodeTheme.ts)
  window.__acpiraTheme = dark => {
    document.body.classList.toggle('vscode-dark', dark);
    document.body.classList.toggle('vscode-light', !dark);
  };

  // The title bar: the column's title row and the sheet's header (data-drag-region) move the window, their controls do
  // not (WebKit has no -webkit-app-region, so the press starts the drag here). A double click does what the system
  // setting says (zoom, minimize or nothing), like a native title bar
  const CONTROLS = 'button, input, textarea, select, a, [role="button"], [contenteditable=""], [contenteditable="true"]';
  document.addEventListener('mousedown', e => {
    if (e.button !== 0 || !(e.target instanceof Element)) return;
    const region = e.target.closest('[data-drag-region]');
    if (!region) return;
    const control = e.target.closest(CONTROLS);
    if (control && region.contains(control)) return;
    if (e.target.closest('[data-panel-toggle]') && control) return;
    e.preventDefault();
    T.invoke('acpira_window', { op: e.detail === 2 ? 'titleDoubleClick' : 'drag' });
  }, true);
})();
