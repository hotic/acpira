// The transport the webview bundle expects from a host (packages/ui/vscodeApi.ts): window.__acpiraApi.postMessage up,
// window `message` events down, plus the page's own persisted state. Injected before the page's scripts in the top frame
// of the app's own pages (the shell's initialization script); a browser tab of the tool panel is a frame and talks
// through its parent instead (window.__acpiraBrowserTab, set by the page)
(() => {
  // The app protocol's origin (the shell fills it in): acpira-app://localhost in WKWebView, http://acpira-app.localhost in
  // WebView2. Any other page (a loopback dev page navigated to) gets no transport
  const ORIGIN = '__ACPIRA_APP_ORIGIN__';
  if (window.top !== window || `${location.protocol}//${location.host}` !== ORIGIN || window.__acpiraApi) return;
  const T = window.__TAURI_INTERNALS__;
  const STATE_KEY = 'acpira.webviewState';
  const STATE_SCOPE = '__ACPIRA_STATE_SCOPE__';
  const VIEW_KEY = `acpira.sessionListView${STATE_SCOPE ? `:${STATE_SCOPE}` : ''}`;
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

  // WebView2 (Windows) hands a dropped File to the shell as a CoreWebView2File, whose path comes back here by id
  // (win.rs); WKWebView has no such route, so the paths of a Finder drag are read as it enters the page. WebKitGTK
  // (Linux) shows the page no file of a drag at all, only a text/uri-list that reads empty: its paths come from the
  // shell the same way, and the page takes them whole (dragPaths)
  const webview2 = window.chrome?.webview?.postMessageWithAdditionalObjects ? window.chrome.webview : null;
  const linux = navigator.platform.startsWith('Linux');
  const dropWaiters = new Map();
  let dropSeq = 0;
  window.__acpiraDropPath = (id, path) => {
    const done = dropWaiters.get(id);
    dropWaiters.delete(id);
    done?.(path || undefined);
  };
  const askPath = file => new Promise(resolve => {
    const id = ++dropSeq;
    dropWaiters.set(id, resolve);
    try {
      webview2.postMessageWithAdditionalObjects(`acpira-drop:${id}`, [file]);
    } catch {
      // WebView2 refuses a File that is not on disk (one the page made, a drag from another app's memory): no path
      dropWaiters.delete(id);
      resolve(undefined);
      return;
    }
    setTimeout(() => { if (dropWaiters.delete(id)) resolve(undefined); }, 5000);
  });
  let dragPaths = [];
  document.addEventListener('dragenter', e => {
    if (webview2) return;
    // A drag without files (a session moved in the column) finds none left from an earlier one
    if (!e.dataTransfer?.types.includes(linux ? 'text/uri-list' : 'Files')) { dragPaths = []; return; }
    T.invoke('acpira_drag_paths').then(paths => { dragPaths = paths; }, () => { dragPaths = []; });
  }, true);
  // After the page's own handlers: the drop is over
  addEventListener('drop', () => { dragPaths = []; });

  window.__acpiraApi = Object.freeze({
    postMessage: message => { queue.push(JSON.stringify(message)); flush(); },
    getState: () => {
      let state;
      try { state = JSON.parse(sessionStorage.getItem(STATE_KEY) ?? 'null') ?? undefined; } catch { /* use saved preferences */ }
      // A new window inherits the last list preferences; an existing window keeps its own choices.
      if (state?.sessionListView === undefined) {
        try {
          const view = JSON.parse(localStorage.getItem(VIEW_KEY) ?? 'null');
          if (view != null) state = { ...state, sessionListView: view };
        } catch { /* unavailable or invalid storage: the list uses its defaults */ }
      }
      return state;
    },
    setState: state => {
      let previousView;
      try { previousView = JSON.stringify(JSON.parse(sessionStorage.getItem(STATE_KEY) ?? 'null')?.sessionListView); } catch { /* no previous choice */ }
      try { sessionStorage.setItem(STATE_KEY, JSON.stringify(state)); } catch { /* quota: the state is a convenience */ }
      // Only list preferences outlive the window; drafts, selection and panel state stay window-local.
      if (state?.sessionListView !== undefined) {
        try {
          const view = JSON.stringify(state.sessionListView);
          // An unrelated state write in another window must not undo the last explicit list choice.
          if (view !== previousView || localStorage.getItem(VIEW_KEY) === null) localStorage.setItem(VIEW_KEY, view);
        } catch { /* keep window-local state */ }
      }
    },
    // The system clipboard's text through the shell: WebKit asks the user before navigator.clipboard.readText (a paste
    // bubble), which a terminal's right-click paste must not show
    readClipboard: () => T.invoke('acpira_clipboard'),
    // A GIF on the system clipboard as a file, which the web clipboard would flatten to one PNG frame (commands.rs)
    copyImage: (data, name) => T.invoke('acpira_copy_image', data, { headers: { 'x-acpira-name': encodeURIComponent(name) } }),
    // A dropped file's path: WKWebView gives the page its name and bytes only, so the paths of a Finder drag are read
    // from the system's drag pasteboard as it enters the page and matched by name at the drop; WebView2 answers through
    // the shell, a promise (packages/ui Shell's folder drop takes both)
    pathForFile: file => (webview2 ? askPath(file) : dragPaths.find(p => p.slice(p.lastIndexOf('/') + 1) === file.name)),
    // Linux: the paths of the drag over the page, a folder's ending in '/', empty for a drag without files
    ...(linux ? { dragPaths: () => dragPaths } : {}),
  });

  // The system appearance changed (the shell calls this): the page follows the body class (packages/ui/useVsCodeTheme.ts)
  window.__acpiraTheme = dark => {
    document.body.classList.toggle('vscode-dark', dark);
    document.body.classList.toggle('vscode-light', !dark);
  };

  // The title bar: the column's title row and the sheet's header (data-drag-region) move the window, their controls do
  // not (WebKit has no -webkit-app-region, so the press starts the drag here). A double click does what the system
  // setting says (zoom, minimize or nothing), like a native title bar. On Linux the move starts once the pointer moves:
  // a move begun on the press hands the pointer to the window manager, and the page never sees a double click's
  // second press (WebKitGTK 2.50 under KWin, 2026-10-26)
  const CONTROLS = 'button, input, textarea, select, a, [role="button"], [contenteditable=""], [contenteditable="true"]';
  document.addEventListener('mousedown', e => {
    if (e.button !== 0 || !(e.target instanceof Element)) return;
    const region = e.target.closest('[data-drag-region]');
    if (!region) return;
    const control = e.target.closest(CONTROLS);
    if (control && region.contains(control)) return;
    if (e.target.closest('[data-panel-toggle]') && control) return;
    e.preventDefault();
    if (e.detail === 2 || !linux) {
      T.invoke('acpira_window', { op: e.detail === 2 ? 'titleDoubleClick' : 'drag' });
      return;
    }
    const [x, y] = [e.screenX, e.screenY];
    const stop = () => {
      removeEventListener('mousemove', move, true);
      removeEventListener('mouseup', stop, true);
    };
    const move = m => {
      if (!(m.buttons & 1)) return stop();
      if (Math.abs(m.screenX - x) + Math.abs(m.screenY - y) < 4) return;
      stop();
      T.invoke('acpira_window', { op: 'drag' });
    };
    addEventListener('mousemove', move, true);
    addEventListener('mouseup', stop, true);
  }, true);
})();
