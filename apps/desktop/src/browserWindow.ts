import { randomUUID } from 'node:crypto';
import { BrowserWindow, type WebContents } from 'electron';
import { normalizePreviewUrl, type PageContextOp } from '@acpira/shared/picker';
import type { BrowserHostMsg, BrowserLook, BrowserMsg, BrowserPage, ContextItem } from '@acpira/shared/protocol';
import type { SidecarClient } from '@acpira/engine-client/SidecarClient';
import { guard } from './chatWindow';
import { APP_ORIGIN } from './page';

export interface BrowserRelayOpts {
  client: SidecarClient;
  // dist/picker.js, the script the engine's preview proxy adds to the page
  picker: string;
  look: () => BrowserLook;
  history: () => BrowserPage[];
  visited: (page: BrowserPage) => void;
  // Picked elements with their comment, for the chat used last
  onAdd: (item: ContextItem) => void;
  // The page's own window.acpira.context call: what `set` resolves to, or a throw naming the refused field
  onContext: (panelId: string, op: PageContextOp, payload: unknown) => { shown?: boolean };
  // The page shown so far is gone (another frame, an error, a reloaded panel, the window closed): its live context goes too
  onLeave: (panelId: string) => void;
  openExternal: (url: string) => void;
  log: (line: string) => void;
}

// One browser page (the bundle with host 'browser', webview/browser/BrowserPanel.tsx) showing a loopback dev page through
// the engine's preview proxy, which adds the element picker; however its messages travel. It has no engine view; this side
// holds the proxy lease (SidecarClient.preview asks again after a reconnect). The VS Code shell's BrowserPanel
// (apps/vscode/src/bridge.ts) is the model: same messages, same order. A window of its own (BrowserPanelWindow) or a tab of a chat
// window's tool panel (ChatWindow) carries it
export class BrowserRelay {
  readonly panelId = randomUUID();
  // The page as asked for: what the address bar shows and what a reconnect asks for again; undefined while empty
  target?: string;
  private release?: () => void;
  private ready = false;
  // The frame (or error) waits for the page to listen
  private pending?: BrowserHostMsg;

  constructor(private opts: BrowserRelayOpts, private send: (m: BrowserHostMsg) => void, private onTitle?: (title: string) => void) {}

  // Another page through the same lease: the engine keeps the proxy while the site stays the same
  navigate(url: string) {
    this.target = url;
    this.release = this.opts.client.preview({ panelId: this.panelId, url, picker: this.opts.picker, onOpened: r => this.opened(url, r) });
  }

  postLook() {
    if (this.ready) this.send({ type: 'browserLook', look: this.opts.look() });
  }

  postHistory() {
    if (this.ready) this.send({ type: 'browserHistory', pages: this.opts.history() });
  }

  // A local page reaches the proxy as it is (the engine runs on this machine), so no forwarding step comes between
  private opened(url: string, r: { url: string; token: string } | { error: string }) {
    if (url !== this.target) return;
    this.post('error' in r ? { type: 'browserError', target: url, error: r.error } : { type: 'browserFrame', target: url, src: r.url, token: r.token });
  }

  // A frame or an error, which takes the place of the page shown so far
  private post(m: BrowserHostMsg) {
    this.opts.onLeave(this.panelId);
    this.pending = m;
    if (this.ready) this.send(m);
  }

  fromPage(m: BrowserMsg) {
    switch (m.type) {
      case 'browserReady':
        // A page that starts again (reloaded) shows the page anew; the old document is gone
        this.opts.onLeave(this.panelId);
        this.ready = true;
        this.postLook();
        this.postHistory();
        if (this.pending) this.send(this.pending);
        break;
      case 'browserNavigate': {
        const url = normalizePreviewUrl(m.url);
        if (url) this.navigate(url);
        break;
      }
      case 'browserTitle': {
        const title = m.title.trim().slice(0, 80);
        this.onTitle?.(title || hostOf(m.url));
        if (normalizePreviewUrl(m.url)) this.opts.visited({ url: m.url, title });
        break;
      }
      case 'browserAdd': this.opts.onAdd(m.item); break;
      case 'browserOpenExternal': this.opts.openExternal(m.url); break;
      case 'browserContext': {
        let answer: BrowserHostMsg;
        try {
          answer = { type: 'browserContextDone', id: m.id, ...this.opts.onContext(this.panelId, m.op, m.payload) };
        } catch (e) {
          answer = { type: 'browserContextDone', id: m.id, error: e instanceof Error ? e.message : String(e) };
        }
        this.send(answer);
        break;
      }
    }
  }

  dispose() {
    this.opts.onLeave(this.panelId);
    this.release?.();
    this.release = undefined;
  }
}

export interface BrowserWindowOpts extends BrowserRelayOpts {
  preload: string;
  dark: boolean;
  onClosed: (w: BrowserPanelWindow) => void;
}

// The browser page as a window of its own (the View menu's Browser before the tool panel had tabs, kept for a chat-less
// open_browser)
export class BrowserPanelWindow {
  readonly win: BrowserWindow;
  readonly relay: BrowserRelay;

  // `focus` false: shown without taking the keyboard (an agent's open_browser leaves the caret in the chat)
  constructor(private opts: BrowserWindowOpts, url?: string, focus = true) {
    this.win = new BrowserWindow({
      width: 1200,
      height: 860,
      minWidth: 480,
      minHeight: 360,
      show: false,
      title: url ? new URL(url).host : 'Acpira',
      backgroundColor: opts.dark ? '#181818' : '#f8f8f8',
      webPreferences: { preload: opts.preload, contextIsolation: true, sandbox: true, nodeIntegration: false, spellcheck: false },
    });
    this.relay = new BrowserRelay(opts, m => { if (!this.win.isDestroyed()) this.win.webContents.send('acpira:host', m); },
      title => { if (!this.win.isDestroyed()) this.win.setTitle(title); });
    guard(this.win.webContents);
    this.win.webContents.on('console-message', e => { if (e.level === 'error') opts.log(`browser page error: ${e.message} (${e.sourceId}:${e.lineNumber})`); });
    this.win.once('ready-to-show', () => (focus ? this.win.show() : this.win.showInactive()));
    this.win.on('closed', () => { this.relay.dispose(); this.opts.onClosed(this); });
    void this.win.loadURL(`${APP_ORIGIN}/index.html?host=browser`).catch(e => opts.log(`browser window load failed: ${String(e)}`));
    if (url) this.relay.navigate(url);
  }

  get webContents(): WebContents { return this.win.webContents; }
  get target(): string | undefined { return this.relay.target; }

  reveal() {
    if (this.win.isMinimized()) this.win.restore();
    this.win.show();
    this.win.focus();
  }

  setTheme(dark: boolean) {
    if (!this.win.isDestroyed()) this.win.webContents.send('acpira:theme', dark);
  }
}

// Which of these pages answer right now (anything over HTTP counts), for the new-tab page's dots
export async function answering(urls: string[], timeoutMs = 800): Promise<string[]> {
  const probes = await Promise.all(urls.map(async url => {
    try {
      const res = await fetch(url, { method: 'GET', redirect: 'manual', signal: AbortSignal.timeout(timeoutMs) });
      await res.body?.cancel();
      return url;
    } catch {
      return undefined;
    }
  }));
  return probes.filter((u): u is string => !!u);
}

function hostOf(url: string): string {
  try { return new URL(url).host; } catch { return url; }
}
