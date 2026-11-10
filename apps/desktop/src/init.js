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
    // Native file save dialog for a rendered PNG screenshot (commands.rs)
    saveImage: (data, name) => T.invoke('acpira_save_image', { name, data: Array.from(data) }),
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
  // A sheet with no header of its own (the settings page) marks itself data-drag-top: its top title-bar height (--hdr)
  // moves the window like the chat sheet's header, whatever content sits or scrolls under it
  const titleBand = e => {
    const sheet = e.target.closest('[data-drag-top]');
    if (!sheet) return null;
    // A press on a scroller's own scrollbar grabs the bar
    const t = e.target;
    if (t.scrollHeight > t.clientHeight && e.offsetX >= t.clientWidth) return null;
    const hdr = parseFloat(getComputedStyle(sheet).getPropertyValue('--hdr')) || 0;
    return e.clientY - sheet.getBoundingClientRect().top < hdr ? sheet : null;
  };
  document.addEventListener('mousedown', e => {
    if (e.button !== 0 || !(e.target instanceof Element)) return;
    const region = e.target.closest('[data-drag-region]') ?? titleBand(e);
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

  // A pinch on a touch display: one View menu zoom step per quarter of scale, and the page under the fingers does not
  // scroll meanwhile. WebKit lets the fingers press the page first (pointerdown, then pointercancel) and a trackpad pinch
  // never does, so a trackpad keeps WebKit's own behaviour
  let pressedAt = -1e9;
  let pinch = null;
  addEventListener('pointerdown', e => { pressedAt = e.timeStamp; }, { capture: true, passive: true });
  addEventListener('gesturestart', e => {
    if (e.timeStamp - pressedAt > 500) return;
    e.preventDefault();
    pinch = 1;
  });
  addEventListener('gesturechange', e => {
    if (pinch === null) return;
    e.preventDefault();
    const step = e.scale / pinch;
    if (step < 1.25 && step > 0.8) return;
    pinch = e.scale;
    T.invoke('acpira_window', { op: step > 1 ? 'zoomIn' : 'zoomOut' });
  });
  addEventListener('gestureend', () => { pinch = null; });
  addEventListener('wheel', e => { if (pinch !== null) e.preventDefault(); }, { capture: true, passive: false });

  // macOS touch displays: a finger cannot start the drag above (it scrolls, and WebKit takes the touch), so the shell
  // lays a native view over the title-bar regions that moves the window by finger (touch.rs). It needs their rectangles
  // with holes wherever a touch belongs to the page: a control inside a region, or anything laid over one (a menu). The
  // holes come from hit-testing a grid over each region, which catches whatever sits on top
  if (webview2 || linux) return;
  const STEP = 12;
  const rectOf = r => [r.left, r.top, r.width, r.height];
  const clip = (r, to) => {
    const left = Math.max(r.left, to.left), top = Math.max(r.top, to.top);
    const right = Math.min(r.right, to.right), bottom = Math.min(r.bottom, to.bottom);
    return right > left && bottom > top ? [left, top, right - left, bottom - top] : null;
  };
  // Each region: its element and rectangle (a data-drag-top sheet's region is its top title-bar band)
  const regionsNow = () => {
    const out = [];
    for (const el of document.querySelectorAll('[data-drag-region]')) {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) out.push({ el, r });
    }
    for (const el of document.querySelectorAll('[data-drag-top]')) {
      const s = el.getBoundingClientRect();
      const hdr = parseFloat(getComputedStyle(el).getPropertyValue('--hdr')) || 0;
      if (s.width > 0 && hdr > 0) out.push({ el, r: new DOMRect(s.left, s.top, s.width, Math.min(hdr, s.height)) });
    }
    return out;
  };
  let sent = '';
  let pending = 0;
  const report = () => {
    pending = 0;
    const regions = [];
    const holes = [];
    const holed = new Set();
    const all = regionsNow();
    for (const { el, r } of all) {
      regions.push(rectOf(r));
      for (const fy of [0.3, 0.7]) {
        for (let x = r.left + STEP / 2; x < r.right; x += STEP) {
          const hit = document.elementFromPoint(x, r.top + r.height * fy);
          if (!hit || holed.has(hit)) continue;
          let hole = null;
          const owner = el.contains(hit) ? el : all.find(o => o.el.contains(hit))?.el;
          if (!owner) {
            // Laid over the region: the outermost ancestor of the hit that does not hold the region
            let layer = hit;
            while (layer.parentElement && !layer.parentElement.contains(el)) layer = layer.parentElement;
            hole = layer;
          } else {
            // Another region laid over this one (the title row over a sheet's header) is still a title bar
            const control = hit.closest(CONTROLS);
            if (control && owner.contains(control)) hole = control;
          }
          if (!hole || holed.has(hole)) continue;
          holed.add(hit).add(hole);
          const c = clip(hole.getBoundingClientRect(), r);
          if (c) holes.push(c);
        }
      }
    }
    const msg = JSON.stringify([innerWidth, regions, holes]);
    if (msg === sent) return;
    sent = msg;
    T.invoke('acpira_touch_regions', { width: innerWidth, regions, holes }).catch(() => {});
  };
  const schedule = () => { if (!pending) pending = requestAnimationFrame(report); };
  // A transition or animation that can move a region or something over one: on, in or around a region, or a layer at
  // the top of the document; a row's hover fade elsewhere does not count
  const near = e => {
    const t = e.target;
    if (!(t instanceof Element)) return;
    if (t.parentElement === document.body || [...watched].some(el => el.contains(t) || t.contains(el))) schedule();
  };
  // When the regions can change: the window resizes, a region resizes, something changes inside a region or arrives at
  // the top of the document (menus and dialogs mount there), a transition ends. A transcript streaming below changes
  // none of these
  const sized = new ResizeObserver(schedule);
  let watched = new Set();
  const watch = () => {
    const now = new Set(regionsNow().map(x => x.el));
    for (const el of watched) if (!now.has(el)) sized.unobserve(el);
    for (const el of now) if (!watched.has(el)) sized.observe(el);
    watched = now;
  };
  new MutationObserver(records => {
    for (const rec of records) {
      const t = rec.target;
      if (t === document.body || t.parentElement === document.body || [...watched].some(el => el.contains(t))
        || (rec.type === 'attributes' && t instanceof Element && (t.hasAttribute('data-drag-region') || t.hasAttribute('data-drag-top')))
        || [...rec.addedNodes, ...rec.removedNodes].some(n => n instanceof Element && (n.matches('[data-drag-region], [data-drag-top]') || n.querySelector('[data-drag-region], [data-drag-top]')))) {
        watch();
        schedule();
        return;
      }
    }
  }).observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-drag-region', 'data-drag-top', 'class', 'style', 'hidden'] });
  addEventListener('resize', schedule);
  addEventListener('transitionend', near, true);
  addEventListener('animationend', near, true);
  addEventListener('DOMContentLoaded', () => { watch(); schedule(); });
  watch();
  schedule();
})();
