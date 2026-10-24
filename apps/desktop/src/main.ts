import { appendFileSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { app, clipboard, dialog, ipcMain, Menu, nativeTheme, net, Notification, protocol, screen, shell } from 'electron';
import { appearanceFromSettings } from '@acpira/shared/appearance';
import { resolveLocale, translate, type Language, type MsgKey, type Params } from '@acpira/shared/i18n';
import { normalizePreviewUrl, type PageContextOp } from '@acpira/shared/picker';
import type { BrowserLook, BrowserMsg, BrowserPage, ContextItem, HostMsg, ShellCommand, WebviewMsg } from '@acpira/shared/protocol';
import { isSharedSetting, sanitizeSetting } from '@acpira/shared/settings';
import { sessionIdOfLink } from '@acpira/shared/sessionLinks';
import type { BrowserOpened } from '@acpira/shared/sidecar';
import { ContextPayloadError, ContextSlots, pageOwner, parseContextPayload } from '@acpira/engine-client/contextApi';
import { SidecarClient, type SidecarCommand } from '@acpira/engine-client/SidecarClient';
import { engineEndpoint } from '@acpira/engine-client/engine';
import { sidecarCommands } from '@acpira/engine-client/sidecarLocator';
import { BrowserPanelWindow, type BrowserRelayOpts } from './browserWindow';
import { ChatWindow, preloadPath, type ChatWindowOpen } from './chatWindow';
import { confine } from './confine';
import { openInEditor } from './editors';
import { readJson, writeJson } from './jsonFile';
import { resolveLoginEnv } from './loginEnv';
import { buildMenu } from './menu';
import { APP_ORIGIN, pageHtml, THEME_CSS } from './page';
import { CAPABILITIES, handleRequest } from './platform';
import { SharedSettingsFile } from '@acpira/engine-client/sharedSettings';
import { SettingsStore } from './settings';
import { runInTerminal } from './terminal';
import { ideSettingsPaths, readIdeSettings } from './vscodeSettings';
import { placeable, WindowStateStore, type WindowEntry, type WindowsState } from './windowState';

// The desktop app's main process: one connection to the persistent engine (`acpira serve`, its own socket key), any
// number of chat windows (each an engine view), browser panel windows (no view; this side holds their preview leases), and
// the system side of the IDE actions. See docs/dev/desktop.md

// The scheme must be privileged before the app is ready: a standard, secure origin lets the bundle load as ES modules and
// fetch its blobs
protocol.registerSchemesAsPrivileged([
  { scheme: 'acpira-app', privileges: { standard: true, secure: true, supportFetchAPI: true, corsEnabled: true, stream: true } },
]);

// Smoke runs and screenshots keep the app's own settings, window state and log out of the real profile
const userData = process.env.ACPIRA_DESKTOP_USER_DATA?.trim();
if (userData) {
  app.setPath('userData', userData);
  app.setAppLogsPath(join(userData, 'logs'));
}

// dist/main.cjs in a checkout (apps/desktop/dist → the repository root holds dist/webview, dist/picker.js and dist/sidecar); a
// packaged app carries them under its resources (webview/, picker.js, bin/acpira)
const ROOT = app.isPackaged ? process.resourcesPath : resolve(__dirname, '..', '..', '..');
const WEBVIEW = app.isPackaged ? join(ROOT, 'webview') : join(ROOT, 'dist', 'webview');
const PICKER = app.isPackaged ? join(ROOT, 'picker.js') : join(ROOT, 'dist', 'picker.js');
const DATA_HOME = process.env.ACPIRA_HOME?.trim() || join(homedir(), '.acpira');
const HELP_URL = 'https://github.com/hotic/acpira/issues';
// The browser panel's address bar suggestions: pages shown in any panel, most recent first
const MAX_HISTORY = 20;

const logFile = join(app.getPath('logs'), 'desktop.log');
function log(line: string) {
  const stamped = `${new Date().toISOString()} ${line}`;
  console.log(stamped);
  try { mkdirSync(app.getPath('logs'), { recursive: true }); appendFileSync(logFile, `${stamped}\n`); } catch { /* logging must not fail the app */ }
}

// Session links (acpira://session/<id>): macOS hands them to the running app, or to the one it launches for them, as
// open-url, which must be listened for before the app is ready; Windows and Linux start a second instance with the link in
// its arguments (forwarded by the single-instance lock) or put it in the first launch's. Links before the first window wait
const pendingLinks: string[] = [];
let takeLink: ((url: string) => void) | undefined;
const onLink = (url: string) => { if (takeLink) takeLink(url); else pendingLinks.push(url); };
const linksIn = (argv: readonly string[]) => argv.filter(a => /^acpira:\/\//i.test(a));
app.on('open-url', (e, url) => { e.preventDefault(); onLink(url); });

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  for (const link of linksIn(process.argv)) onLink(link);
  void main();
}

async function main() {
  const windows = new Set<ChatWindow>();
  const browsers = new Set<BrowserPanelWindow>();
  let lastFocused: ChatWindow | undefined;
  const dark = () => nativeTheme.shouldUseDarkColors;

  await app.whenReady();
  // The packaged app is the system's handler for session links (its Info.plist declares the scheme); a development run
  // leaves the registration alone and takes links from its arguments instead
  if (app.isPackaged) app.setAsDefaultProtocolClient('acpira');

  // The app's own store keeps the settings that follow the window (SHELL_SETTING_KEYS); everything else is shared with
  // the IDE shells in <ACPIRA_HOME>/settings.json, which the engine writes and this shell reads
  const settings = new SettingsStore(join(app.getPath('userData'), 'settings.json'));
  const shared = new SharedSettingsFile(DATA_HOME);
  const setting = (key: string) => shared.get(key, k => settings.get(k));
  // Until an engine has created the shared file, the first one seeds it from this hello: the extension's values in VS Code
  // (or Cursor) where they exist, so the app starts on the agents and look the IDE already has, else the app's own
  const seed = (): Record<string, unknown> => {
    if (shared.exists()) return {};
    const found = readIdeSettings(ideSettingsPaths(process.platform, process.env, homedir()), log);
    if (!found) return {};
    log(`shared settings will be seeded from ${found.from}`);
    return Object.fromEntries(Object.entries(found.values).filter(([k]) => isSharedSetting(k)));
  };
  const locale = () => resolveLocale(setting('language') as Language | undefined, app.getLocale());
  const t = (key: MsgKey, params?: Params) => translate(locale(), key, params);

  // Read before the engine starts (an engine launched from the Dock would otherwise miss everything ~/.zshrc exports),
  // while the first window is already loading: the shell takes about a second with a typical rc file
  let loginEnv: Record<string, string> | undefined;

  const focusedWindow = () => (lastFocused && windows.has(lastFocused) ? lastFocused : [...windows][0]);
  // A failure dialog at a time; the engine's own restarts come first, this is what follows when they gave up
  let failureShown = false;
  const client: SidecarClient = new SidecarClient({
    commands: () => sidecarCommands({ root: ROOT, env: process.env }).map((c): SidecarCommand => ({ ...c, env: { ...loginEnv, ...c.env } })),
    // macOS / Linux: the persistent engine, keyed apart from every IDE window's; Windows keeps the attached stdio child
    engine: cmd => (process.platform === 'win32' ? undefined : engineEndpoint({ binary: cmd.command, home: DATA_HOME, shell: 'desktop' })),
    hello: () => ({
      client: { name: 'acpira-desktop', version: app.getVersion(), capabilities: CAPABILITIES },
      env: { hostLanguage: app.getLocale(), projects: true, blobBase: `${APP_ORIGIN}/blobs` },
      settings: { ...settings.snapshot(), ...seed() },
    }),
    onRequest: r => handleRequest(r, {
      settings,
      setting,
      log,
      watching: id => [...windows].some(w => w.watching(id)),
      chime: () => focusedWindow()?.post({ type: 'chime' }),
      showSession: id => showSession(id),
      toast,
      openWindow: id => open(id ?? { mostRecent: true }),
      openBrowser,
      terminal: r => runInTerminal(r, log),
      // The login shell's PATH finds an editor's command line; an app started from the Dock has almost none of its own
      openInEditor: (editor, target) => openInEditor(editor, target, { ...process.env, ...loginEnv }, homedir(), log),
      t,
    }),
    log,
    onState: (state, detail) => {
      if (state !== 'failed') return;
      log(`engine failed: ${detail ?? ''}`);
      if (failureShown) return;
      failureShown = true;
      void dialog.showMessageBox({
        type: 'error', message: t('desktop.engine.failed'), detail: detail ?? '',
        buttons: [t('common.retry'), t('desktop.menu.showLog'), t('common.close')], defaultId: 0, cancelId: 2,
      }).then(({ response }) => {
        failureShown = false;
        if (response === 0) client.retry();
        else if (response === 1) showLog();
      });
    },
  });
  settings.onChange((keys, snapshot) => client.event({ type: 'settingsChanged', keys, settings: snapshot }));
  // The menu's language and the browser panel's look follow the shared file, whichever engine wrote it
  shared.watch(keys => {
    if (keys.includes('language') || keys.includes('desktopLayout')) Menu.setApplicationMenu(menu());
    if (keys.some(k => k === 'theme' || k === 'language' || k.startsWith('appearance.') || k === 'uiFontSize' || k === 'codeFontSize' || k === 'fontSmoothing')) {
      for (const b of browsers) b.relay.postLook();
      for (const w of windows) w.postBrowserLook();
    }
  });

  protocol.handle('acpira-app', req => serve(new URL(req.url), client, setting, dark()));

  ipcMain.on('acpira:webview', (e, m: unknown) => {
    const w = [...windows].find(x => x.webContents === e.sender);
    if (w) { w.fromPage(m as WebviewMsg); return; }
    [...browsers].find(x => x.webContents === e.sender)?.relay.fromPage(m as BrowserMsg);
  });

  // A toast lands in the page used last; with no window open it becomes a system notification
  function toast(level: 'info' | 'error', text: string, openFile?: { label: string; path: string }) {
    log(`toast ${level}: ${text}`);
    const w = focusedWindow();
    if (w) { w.post({ type: 'toast', level, text, ...(openFile ? { open: openFile } : {}) }); return; }
    if (Notification.isSupported()) new Notification({ title: 'Acpira', body: text }).show();
  }

  // Window placement: windows open at quit come back as they were, the last one closed by hand lends its place
  const windowStore = new WindowStateStore(join(app.getPath('userData'), 'windows.json'));
  let windowsState: WindowsState = windowStore.read();
  let quitting = false;
  let saveTimer: NodeJS.Timeout | undefined;
  const entryOf = (w: ChatWindow): WindowEntry => ({
    bounds: w.win.getNormalBounds(),
    ...(w.win.isMaximized() ? { maximized: true } : {}),
    ...(w.sessionId ? { session: w.sessionId } : {}),
  });
  // Each window's entry as of the last save: a window closed from its page (window.close) or gone with its renderer
  // emits no close event of its own, and its bounds cannot be read once it is destroyed
  const known = new Map<ChatWindow, WindowEntry>();
  const saveWindows = () => {
    clearTimeout(saveTimer);
    saveTimer = undefined;
    if (quitting) return;
    const live = [...windows].filter(w => !w.win.isDestroyed());
    for (const w of live) known.set(w, entryOf(w));
    windowsState = { windows: live.map(w => known.get(w)!), ...(windowsState.last ? { last: windowsState.last } : {}) };
    windowStore.write(windowsState);
  };
  const scheduleSave = () => { clearTimeout(saveTimer); saveTimer = setTimeout(saveWindows, 500); };
  const placeOf = (e: WindowEntry | undefined): ChatWindowOpen => ({
    ...(e?.bounds ? { bounds: placeable(e.bounds, screen.getAllDisplays().map(d => d.workArea)) } : {}),
    ...(e?.maximized ? { maximized: true } : {}),
  });

  // Live context from pages in the browser panels (window.acpira.context set), one slot per source, shown in every chat
  const contextSlots = new ContextSlots();
  const shareContext = () => settings.get('shareEditorSelection') !== false;
  const broadcast = (m: HostMsg) => { for (const w of windows) w.post(m); };
  const publishContext = (m: HostMsg | undefined) => { if (m && shareContext()) broadcast(m); };

  const open = (initial?: string | { mostRecent: true }, how: ChatWindowOpen = {}) => {
    const w = new ChatWindow({
      client,
      preload: preloadPath(__dirname),
      dark: dark(),
      onFocus: f => { lastFocused = f; client.event({ type: 'windowFocus' }); },
      onReady: r => { if (shareContext()) for (const m of contextSlots.replay()) r.post(m); },
      onChange: scheduleSave,
      browser: browserOpts,
      onClosed: c => {
        windows.delete(c);
        if (lastFocused === c) lastFocused = undefined;
        // A window closed by hand lends its place to the next new one; at quit every window stays in the list
        const entry = known.get(c);
        known.delete(c);
        if (!quitting && entry) windowsState = { ...windowsState, last: entry };
        saveWindows();
      },
      log,
    }, initial, how);
    // The freshest entry when the native close does fire (the close button, ⌘W)
    w.win.on('close', () => { if (!quitting) known.set(w, entryOf(w)); });
    windows.add(w);
    lastFocused ??= w;
    scheduleSave();
    return w;
  };

  // A notification or a session link: the window already showing it, else the focused one switches, else a new one
  function showSession(id: string) {
    const on = [...windows].find(w => w.sessionId === id);
    if (on) { on.reveal(); return; }
    const w = focusedWindow();
    if (w) { w.reveal(); w.post({ type: 'showSession', id } satisfies HostMsg); return; }
    open(id, placeOf(windowsState.last));
  }

  // A link names a session this machine's store has (or a window shows), else the page says the link came from elsewhere
  const sessionsDir = () => client.sessionsDir ?? join(DATA_HOME, 'sessions');
  takeLink = url => {
    const id = sessionIdOfLink(url);
    if (!id) { log(`unhandled link: ${url}`); return; }
    const known = [...windows].some(w => w.sessionId === id)
      || existsSync(join(sessionsDir(), `${id}.json`)) || existsSync(join(DATA_HOME, 'bridges', 'chatgpt', `${id}.json`));
    if (known) { showSession(id); return; }
    const w = focusedWindow() ?? open({ mostRecent: true }, placeOf(windowsState.last));
    w.reveal();
    w.post({ type: 'toast', level: 'error', text: t('sessionLink.notHere') });
  };

  // Browser panels, one per site: opening a page of a site already shown navigates that window
  const historyFile = join(app.getPath('userData'), 'browser-history.json');
  const browserHistory = (): BrowserPage[] => {
    const raw = readJson(historyFile);
    return Array.isArray(raw) ? raw.filter((p): p is BrowserPage => !!p && typeof p === 'object' && typeof (p as BrowserPage).url === 'string' && typeof (p as BrowserPage).title === 'string') : [];
  };
  const visited = (page: BrowserPage) => {
    writeJson(historyFile, [page, ...browserHistory().filter(p => p.url !== page.url)].slice(0, MAX_HISTORY));
    for (const b of browsers) b.relay.postHistory();
    for (const w of windows) w.postBrowserHistory();
  };
  const browserLook = (): BrowserLook => ({
    appearance: appearanceFromSettings(key => setting(`appearance.${key}`)),
    theme: sanitizeSetting('theme', setting('theme')),
    uiFontSize: sanitizeSetting('uiFontSize', setting('uiFontSize')),
    codeFontSize: sanitizeSetting('codeFontSize', setting('codeFontSize')),
    fontSmoothing: sanitizeSetting('fontSmoothing', setting('fontSmoothing')),
    locale: locale(),
  });
  // Picked elements go to the chat used last, like another tool's context; the chat comes up with them
  const addToChat = (m: HostMsg) => {
    const w = focusedWindow() ?? open({ mostRecent: true }, placeOf(windowsState.last));
    w.reveal();
    w.post(m);
  };
  const addPicked = (item: ContextItem) => {
    try {
      const p = parseContextPayload({ source: { id: 'browser', name: t('browser.source') }, items: [item] });
      addToChat({ type: 'addContext', source: p.source, items: p.items });
    } catch (e) {
      toast('error', e instanceof Error ? e.message : String(e));
    }
  };
  const pageContext = (panelId: string, op: PageContextOp, raw: unknown): { shown?: boolean } => {
    const p = parseContextPayload(raw);
    if (op === 'add') {
      if (!p.items.length) throw new ContextPayloadError('acpira.context: add needs at least one item');
      addToChat({ type: 'addContext', source: p.source, items: p.items });
      return {};
    }
    publishContext(contextSlots.set({ ...p, owner: pageOwner(panelId, p.owner) }));
    return { shown: shareContext() };
  };
  const leavePage = (panelId: string) => { for (const m of contextSlots.releasePage(panelId)) publishContext(m); };
  // What every browser page needs, a window of its own or a tab of a chat window's tool panel
  const browserOpts: BrowserRelayOpts = {
    client,
    picker: PICKER,
    look: browserLook,
    history: browserHistory,
    visited,
    onAdd: addPicked,
    onContext: pageContext,
    onLeave: leavePage,
    openExternal: page => { if (/^https?:\/\//i.test(page)) void shell.openExternal(page); },
    log,
  };
  // A loopback page in a browser panel (undefined: an empty one on its address bar), the window already showing its site
  // brought up (navigated when the page differs) or a new one; `focus` false for an agent's open_browser, so the chat
  // keeps the keyboard
  // `folder`: a session folder on an SSH host, whose dev server the page is (a site is matched on its host too)
  function showPreview(url: string | undefined, focus = true, folder?: string): BrowserOpened['reused'] {
    const origin = url && new URL(url).origin;
    const shown = [...browsers].find(b => (origin ? b.target !== undefined && new URL(b.target).origin === origin && b.folder === folder : b.target === undefined));
    const bring = (b: BrowserPanelWindow) => (focus ? b.reveal() : b.win.showInactive());
    if (shown && !url) { bring(shown); return 'revealed'; }
    if (shown && url) {
      bring(shown);
      if (shown.target === url) return 'revealed';
      shown.relay.navigate(url);
      return 'navigated';
    }
    const b = new BrowserPanelWindow({ ...browserOpts, preload: preloadPath(__dirname), dark: dark(), onClosed: closed => browsers.delete(closed) }, url, focus, folder);
    browsers.add(b);
    return undefined;
  }
  // open_browser: a loopback http page in a browser tab of the chat used last (its tool panel opens on it), or a window of
  // its own with no chat open, where elements can be picked into the chat; any other http(s) page in the system browser,
  // which the proxy cannot reach
  // `folder`: the session's folder on an SSH host, whose loopback the page is on
  function openBrowser(url: string, folder?: string): BrowserOpened {
    const page = normalizePreviewUrl(url);
    const w = focusedWindow();
    if (page && w?.ready) {
      const reused = w.showInPanel(page, folder);
      return { url: page, ...(reused ? { reused } : {}) };
    }
    if (page) {
      const reused = showPreview(page, false, folder);
      return { url: page, ...(reused ? { reused } : {}) };
    }
    if (!/^https?:\/\//i.test(url)) throw new Error(`only http(s) pages open in a browser: ${url}`);
    void shell.openExternal(url);
    return { url };
  }

  const showLog = () => { void shell.openPath(logFile); };
  // The menu bar's page commands go to the chat used last (a new window when none is open)
  const command = (c: ShellCommand) => {
    const w = focusedWindow() ?? open({ mostRecent: true }, placeOf(windowsState.last));
    w.reveal();
    w.post({ type: 'shellCommand', command: c });
  };
  // A new window bound to a fresh ChatGPT mirror; its connection prompt goes to the clipboard once the window shows it
  const connectChatgpt = () => {
    let copied = false;
    const w: ChatWindow = open(undefined, {
      watch: s => {
        const prompt = s.external?.connectionPrompt;
        if (copied || !prompt) return;
        copied = true;
        clipboard.writeText(prompt);
        w.post({ type: 'toast', level: 'info', text: t('chatgpt.copied') });
      },
    });
    client.send(w.viewId, { type: 'connectChatgpt' });
  };
  const menu = () => buildMenu(t, {
    newSession: () => command('newSession'),
    newWindow: () => open({ mostRecent: true }),
    openFolder: () => command('openFolder'),
    search: () => command('search'),
    settings: () => command('settings'),
    browser: () => command('browser'),
    toolPanel: () => command('toolPanel'),
    layout: l => command(l === 'files' ? 'layoutFiles' : 'layoutChat'),
    connectChatgpt,
    showLog,
    help: () => { void shell.openExternal(HELP_URL); },
  }, { layout: sanitizeSetting('desktopLayout', setting('desktopLayout')) });

  nativeTheme.on('updated', () => {
    for (const w of windows) w.setTheme(dark());
    for (const b of browsers) b.setTheme(dark());
  });
  Menu.setApplicationMenu(menu());

  app.on('second-instance', (_e, argv) => {
    const links = linksIn(argv);
    if (links.length) { for (const link of links) onLink(link); return; }
    const w = focusedWindow();
    if (w) w.reveal();
    else open({ mostRecent: true }, placeOf(windowsState.last));
  });
  // macOS keeps the app (and its engine connection) when the last window closes; turns go on, notifications still come
  app.on('window-all-closed', () => { if (process.platform !== 'darwin') app.quit(); });
  app.on('activate', () => { if (!windows.size) open({ mostRecent: true }, placeOf(windowsState.last)); });
  // Quitting saves where every window was, then only disconnects: the engine keeps running turns and ends on its own once idle
  app.on('before-quit', () => {
    saveWindows();
    quitting = true;
    void client.dispose();
  });

  // The windows open at the last quit come back; a link that launched the app shows its session in the first one
  const restore = windowsState.windows.length ? windowsState.windows : [windowsState.last ?? {}];
  const opened = restore.map(e => open(e.session ?? { mostRecent: true }, placeOf(e)));
  const shot = process.env.ACPIRA_DESKTOP_SHOT?.trim();
  if (shot && opened[0]) screenshotAndQuit(opened[0], shot);
  for (const link of pendingLinks.splice(0)) takeLink(link);
  loginEnv = await resolveLoginEnv(log);
  client.start();
}

// Development and smoke checks: ACPIRA_DESKTOP_SHOT=<png> captures the first window once it settled, then quits
function screenshotAndQuit(w: ChatWindow, file: string) {
  const delay = Number(process.env.ACPIRA_DESKTOP_SHOT_DELAY_MS) || 3000;
  w.webContents.once('did-finish-load', () => setTimeout(() => {
    void w.webContents.capturePage().then(img => {
      writeFileSync(file, img.toPNG());
      log(`screenshot written: ${file}`);
      app.quit();
    });
  }, delay));
}

// acpira-app://app/…: the page, the bundle, the theme and attachment blobs. Files only from their own roots
async function serve(url: URL, client: SidecarClient, setting: (key: string) => unknown, dark: boolean): Promise<Response> {
  const path = url.pathname;
  if (path === '/index.html') {
    const host = url.searchParams.get('host') === 'browser' ? 'browser' : 'editor';
    const font = setting('appearance.font');
    // A tool panel tab names itself; an id is all it may carry into the page
    const tab = host === 'browser' ? url.searchParams.get('tab') ?? undefined : undefined;
    const html = pageHtml({ host, locale: app.getLocale(), dark, font: typeof font === 'string' ? font : undefined, sessionLinkBase: 'acpira://session/', ...(tab && /^[\w-]{1,64}$/.test(tab) ? { tab } : {}) });
    return new Response(html, { headers: { 'content-type': 'text/html; charset=utf-8' } });
  }
  if (path === '/desktop.css') return new Response(THEME_CSS, { headers: { 'content-type': 'text/css; charset=utf-8' } });
  const file = path.startsWith('/webview/') ? confine(WEBVIEW, path.slice('/webview/'.length))
    : path.startsWith('/blobs/') && client.sessionsDir ? confine(client.sessionsDir, path.slice('/blobs/'.length))
    : undefined;
  if (!file) return new Response('not found', { status: 404 });
  return net.fetch(pathToFileURL(file).toString());
}
