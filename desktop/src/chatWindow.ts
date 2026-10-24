import { randomUUID } from 'node:crypto';
import { delimiter, join } from 'node:path';
import { BrowserWindow, dialog, shell, type WebContents } from 'electron';
import type { HostMsg, WebviewMsg } from '@shared/protocol';
import type { SessionView } from '@shared/transcript';
import type { SidecarClient, SidecarState, ShellView } from '../../src/host/shell/SidecarClient';
import { normalizePreviewUrl } from '@shared/picker';
import { answering, BrowserRelay, type BrowserRelayOpts } from './browserWindow';
import { APP_ORIGIN } from './page';
import type { Bounds } from './windowState';

export interface ChatWindowOpts {
  client: SidecarClient;
  preload: string;
  dark: boolean;
  // The page's window got focus: where IDE actions, the chime and "add to chat" go
  onFocus: (w: ChatWindow) => void;
  onClosed: (w: ChatWindow) => void;
  // The page has its init state: the shell's own state for it (live context slots) goes after
  onReady?: (w: ChatWindow) => void;
  // Moved, resized or on another session: the window state is worth saving
  onChange?: (w: ChatWindow) => void;
  // The tool panel's browser tabs, one relay each
  browser?: BrowserRelayOpts;
  log: (line: string) => void;
}

// How a window opens: saved bounds (only the size when the old place is off every display), whether it was maximized, and
// who watches its session views (connecting ChatGPT copies the mirror's connection prompt once it shows)
export interface ChatWindowOpen {
  bounds?: Partial<Bounds>;
  maximized?: boolean;
  watch?: (session: SessionView) => void;
}

// Smoke runs (scripts/smoke.mjs): ACPIRA_DESKTOP_PICK lists folders (path-delimiter separated) that answer the folder
// dialog in turn, across windows, without showing it; once they are used up the real dialog comes back
const SCRIPTED_PICKS = (process.env.ACPIRA_DESKTOP_PICK ?? '').split(delimiter).filter(Boolean);

// One chat window: a BrowserWindow running the webview bundle with host 'editor', and its own view in the engine. The
// relay mirrors the VS Code shell's WebviewBridge (src/host/bridge.ts): page messages go to the view, host messages to the
// page, and a page that already initialized starts over when the engine comes back
export class ChatWindow implements ShellView {
  readonly viewId = randomUUID();
  readonly host = 'editor' as const;
  readonly blobBase = `${APP_ORIGIN}/blobs`;
  readonly win: BrowserWindow;
  private detach: () => void;
  private initialized = false;
  // The page received init since it last loaded: shell messages before that would find no listener and wait instead
  private pageReady = false;
  private shellQueue: HostMsg[] = [];
  // The session the page shows, as last pushed
  private shown?: string;

  private watch?: (session: SessionView) => void;
  // The tool panel's browser tabs by tab id; their pages are frames of this window's page, so a reload ends them all
  private tabs = new Map<string, BrowserRelay>();

  constructor(private opts: ChatWindowOpts, readonly initial?: string | { mostRecent: true }, place: ChatWindowOpen = {}) {
    this.watch = place.watch;
    this.win = new BrowserWindow({
      width: 1180,
      height: 820,
      ...place.bounds,
      minWidth: 420,
      minHeight: 360,
      show: false,
      title: 'Acpira',
      backgroundColor: opts.dark ? '#181818' : '#f8f8f8',
      // macOS: no title bar; the page's column title row and sheet header are the title bar (data-drag-region) and leave the
      // traffic lights their room (desktop.css --lights-w). The lights' centre sits on that row's centre line, y = 30
      // (--sheet-inset 8 + --hdr 44 / 2). macOS 27 draws 14 pt lights 9 pt apart: from x 17 they span 17-77, the first one
      // centred over the column's row icons (x 24 at the default density)
      ...(process.platform === 'darwin' ? { titleBarStyle: 'hiddenInset' as const, trafficLightPosition: TRAFFIC_LIGHTS } : {}),
      webPreferences: { preload: opts.preload, contextIsolation: true, sandbox: true, nodeIntegration: false, spellcheck: false },
    });
    guard(this.win.webContents);
    // Page errors and a crashed renderer land in the main log; a blank window says nothing otherwise
    this.win.webContents.on('console-message', e => { if (e.level === 'error' || e.level === 'warning') opts.log(`page ${e.level}: ${e.message} (${e.sourceId}:${e.lineNumber})`); });
    this.win.webContents.on('render-process-gone', (_e, d) => opts.log(`renderer gone: ${d.reason} (exit ${d.exitCode})`));
    this.win.once('ready-to-show', () => { if (place.maximized) this.win.maximize(); this.win.show(); });
    this.win.on('focus', () => opts.onFocus(this));
    const changed = () => opts.onChange?.(this);
    this.win.on('moved', changed);
    this.win.on('resized', changed);
    this.win.on('maximize', changed);
    this.win.on('unmaximize', changed);
    this.win.on('closed', () => { this.detach(); this.closeTabs(); opts.onClosed(this); });
    // Attach before the page loads: its `ready` must find the view
    this.detach = opts.client.attach(this);
    void this.load();
  }

  get webContents(): WebContents { return this.win.webContents; }

  get sessionId(): string | undefined { return this.shown; }

  // True once the page has its init state (a chime or a toast can show now)
  get ready(): boolean { return this.pageReady; }

  private load() {
    this.pageReady = false;
    this.closeTabs();
    return this.win.loadURL(`${APP_ORIGIN}/index.html?host=${this.host}`).catch(e => this.opts.log(`window load failed: ${String(e)}`));
  }

  // From the page (preload → ipc). viewFocus stays in the shell, like the VS Code bridge, and so does the folder dialog
  fromPage(m: WebviewMsg) {
    if (m.type === 'viewFocus') { this.opts.onFocus(this); return; }
    if (m.type === 'pickFolder') { void this.pickFolder(); return; }
    if (this.panelMessage(m)) return;
    this.opts.client.send(this.viewId, m);
  }

  // The tool panel's messages that stay in the shell: a tab opens on a page (or empty) and its page talks through its
  // relay, the new-tab page asks for the recent pages. Its terminals run in the engine, their messages go on to it
  private panelMessage(m: WebviewMsg): boolean {
    switch (m.type) {
      case 'panelBrowserOpen': {
        const relay = this.tabs.get(m.tab) ?? this.openTab(m.tab);
        const url = m.url && normalizePreviewUrl(m.url);
        if (relay && url && url !== relay.target) relay.navigate(url);
        return true;
      }
      case 'panelBrowser': this.tabs.get(m.tab)?.fromPage(m.msg); return true;
      case 'panelBrowserClose': this.tabs.get(m.tab)?.dispose(); this.tabs.delete(m.tab); return true;
      case 'panelHistory': {
        const pages = this.opts.browser?.history() ?? [];
        void answering(pages.map(p => p.url).filter(u => normalizePreviewUrl(u))).then(running => this.post({ type: 'panelHistory', pages, running }));
        return true;
      }
      default: return false;
    }
  }

  private openTab(tab: string): BrowserRelay | undefined {
    if (!this.opts.browser) return undefined;
    const relay = new BrowserRelay(this.opts.browser, msg => this.post({ type: 'panelBrowser', tab, msg }));
    this.tabs.set(tab, relay);
    return relay;
  }

  private closeTabs() {
    for (const relay of this.tabs.values()) relay.dispose();
    this.tabs.clear();
  }

  // open_browser in this window's tool panel: the tab already on that site navigates (the page brings it forward), else the
  // page opens a tab on it. What happened, as the engine reports it
  showInPanel(url: string): 'revealed' | 'navigated' | undefined {
    const origin = new URL(url).origin;
    const at = [...this.tabs].find(([, r]) => r.target !== undefined && new URL(r.target).origin === origin);
    if (!at) { this.post({ type: 'panelOpenBrowser', url }); return undefined; }
    const [tab, relay] = at;
    const same = relay.target === url;
    if (!same) relay.navigate(url);
    this.post({ type: 'panelOpenBrowser', url, tab });
    return same ? 'revealed' : 'navigated';
  }

  postBrowserLook() { for (const r of this.tabs.values()) r.postLook(); }
  postBrowserHistory() { for (const r of this.tabs.values()) r.postHistory(); }

  // The system folder dialog, as a sheet on this window; pickedFolder carries no path when it was dismissed
  private async pickFolder() {
    const scripted = SCRIPTED_PICKS.shift();
    if (scripted) { this.post({ type: 'pickedFolder', path: scripted }); return; }
    try {
      const r = await dialog.showOpenDialog(this.win, { properties: ['openDirectory', 'createDirectory'] });
      const path = r.canceled ? undefined : r.filePaths[0];
      this.post({ type: 'pickedFolder', ...(path ? { path } : {}) });
    } catch (e) {
      this.opts.log(`folder dialog failed: ${String(e)}`);
      this.post({ type: 'pickedFolder' });
    }
  }

  onHostMessage(m: HostMsg) {
    const session = m.type === 'session' ? m.session : m.type === 'sessionPatch' ? m.patch.view : m.type === 'init' ? m.state.active : undefined;
    const before = this.shown;
    if (session) { this.shown = session.id; this.watch?.(session); }
    if (m.type === 'noSession') this.shown = undefined;
    this.send(m);
    if (this.shown !== before) this.opts.onChange?.(this);
    if (m.type !== 'init') return;
    this.pageReady = true;
    for (const q of this.shellQueue.splice(0)) this.send(q);
    this.opts.onReady?.(this);
  }

  // Shell-originated messages (showSession, chime, toasts, menu commands) take the same way in, once the page listens
  post(m: HostMsg) {
    if (this.pageReady) this.send(m);
    else this.shellQueue.push(m);
  }

  private send(m: HostMsg) {
    if (!this.win.isDestroyed()) this.win.webContents.send('acpira:host', m);
  }

  // Focused and on this session: the user is looking at it
  watching(sessionId: string): boolean {
    return !this.win.isDestroyed() && this.win.isFocused() && this.shown === sessionId;
  }

  onState(state: SidecarState) {
    if (state !== 'ready') return;
    if (this.initialized && !this.win.isDestroyed()) void this.load();
    this.initialized = true;
  }

  setTheme(dark: boolean) {
    if (!this.win.isDestroyed()) this.win.webContents.send('acpira:theme', dark);
  }

  reveal() {
    if (this.win.isMinimized()) this.win.restore();
    this.win.show();
    this.win.focus();
  }
}

// The page never leaves the app: navigation elsewhere is refused and window.open goes nowhere (links reach the system
// browser through the engine's allowlist, `openExternal`)
export function guard(wc: WebContents) {
  wc.on('will-navigate', (e, url) => { if (!url.startsWith(`${APP_ORIGIN}/`)) e.preventDefault(); });
  wc.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//i.test(url)) void shell.openExternal(url);
    return { action: 'deny' };
  });
}

export const preloadPath = (dir: string) => join(dir, 'preload.cjs');

const TRAFFIC_LIGHTS = { x: 17, y: 23 };
