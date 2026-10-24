import { randomBytes, randomUUID } from 'node:crypto';
import * as vscode from 'vscode';
import { appearanceFromSettings } from '@shared/appearance';
import type { BrowserHostMsg, BrowserLook, BrowserMsg, BrowserPage, ContextItem, HostMsg, WebviewHost, WebviewMsg } from '@shared/protocol';
import type { SessionView } from '@shared/transcript';
import { normalizePreviewUrl, type PageContextOp } from '@shared/picker';
import type { SidecarClient, SidecarState, ShellView } from './shell/SidecarClient';
import { PostQueue } from './outbox';
import { t } from './i18n';

// The page's own probe gives up sooner (effects/probeUrl.ts); this only covers a page that never answers
const PROBE_WITHIN_MS = 10_000;

export interface WebviewBridgeOpts {
  client: SidecarClient;
  extensionUri: vscode.Uri;
  sessionsDir: string;
  locale: () => string;
  // An acpira.* setting as the shell sees it (shared keys from <ACPIRA_HOME>/settings.json)
  setting: (key: string) => unknown;
  // The clickable session-link base the page copies (`vscode://hotic.acpira/session/`); pages without one copy `acpira://session/`
  sessionLinkBase?: string;
  // Every session the view shows (init, then each push), for tab titles and one-shot watchers
  onSession?: (session: SessionView) => void;
  // The page has its init state; shell-originated state (the editor selection, the last copy) is replayed from here
  onPageReady?: (bridge: WebviewBridge) => void;
  // The page's window got focus (`viewFocus`, kept in the shell)
  onFocus?: (bridge: WebviewBridge) => void;
  // Whether the user can see this webview right now (its panel or sidebar is shown), for the attention check
  visible?: () => boolean;
  log?: (line: string) => void;
}

// One bridge per VS Code webview: renders the HTML (CSP, bundle URIs, host flag) and relays between the webview and its view in the
// sidecar. Routing and every decision live in the sidecar; this file only knows the vscode.Webview API
export class WebviewBridge implements vscode.Disposable, ShellView {
  readonly viewId = randomUUID();
  readonly blobBase: string;
  private disposables: vscode.Disposable[] = [];
  private detach: () => void;
  private initialized = false;
  // Shell messages wait until the page has been initialized: before that nothing listens, and init would overwrite them anyway
  private pageReady = false;
  private shellQueue: HostMsg[] = [];
  // Everything bound for the page, one postMessage in flight at a time (outbox.ts)
  private outbox: PostQueue;
  private resyncing = false;
  // The session the page shows, as last pushed: an attention for it is not announced while the page is in view
  private shown?: string;
  // probeUrl requests waiting for the page's probeResult
  private probes = new Map<string, (r: { ok: boolean; error?: string } | undefined) => void>();

  constructor(
    private webview: vscode.Webview,
    readonly host: WebviewHost,
    private opts: WebviewBridgeOpts,
    readonly initial?: string | { mostRecent: true },
  ) {
    const sessions = vscode.Uri.file(opts.sessionsDir);
    webview.options = { enableScripts: true, localResourceRoots: [vscode.Uri.joinPath(opts.extensionUri, 'dist', 'webview'), sessions] };
    webview.html = this.html();
    // A session push the webview refused leaves the page's patch base behind: ask for the whole view, once until a whole
    // view or init is delivered (a page that refuses everything is not asked again and again)
    this.outbox = new PostQueue(m => webview.postMessage(m), opts.log, undefined, (m, delivered) => {
      if (delivered) { if (m.type === 'session' || m.type === 'init') this.resyncing = false; return; }
      if ((m.type !== 'session' && m.type !== 'sessionPatch') || this.resyncing) return;
      this.resyncing = true;
      this.send({ type: 'resync' });
    });
    // Attachment blobs are served to the webview straight from the sessions directory, through this webview's own resource URI
    this.blobBase = webview.asWebviewUri(sessions).toString();
    this.disposables.push(webview.onDidReceiveMessage((m: WebviewMsg) => {
      if (m.type === 'viewFocus') { opts.onFocus?.(this); return; }
      if (m.type === 'probeResult') { this.probes.get(m.id)?.({ ok: m.ok, error: m.error }); return; }
      opts.client.send(this.viewId, m);
    }));
    this.detach = opts.client.attach(this);
  }

  send(m: WebviewMsg) { this.opts.client.send(this.viewId, m); }

  onHostMessage(m: HostMsg) {
    const session = m.type === 'session' ? m.session : m.type === 'sessionPatch' ? m.patch.view : m.type === 'init' ? m.state.active : undefined;
    if (session) { this.shown = session.id; this.opts.onSession?.(session); }
    this.outbox.push(m);
    if (m.type !== 'init') return;
    this.pageReady = true;
    for (const q of this.shellQueue.splice(0)) this.outbox.push(q);
    this.opts.onPageReady?.(this);
  }

  // In view and showing this session: the user is looking at it (window focus is the caller's half of the check)
  shows(sessionId: string): boolean {
    return this.shown === sessionId && (this.opts.visible?.() ?? true);
  }

  get sessionId(): string | undefined { return this.shown; }

  // A page that has its init state can play the attention chime now; a queued one would chime late, after a reload
  get ready(): boolean { return this.pageReady; }

  // Editor state from the extension itself (never through the sidecar): selection, copy, "Add to chat"
  postShell(m: HostMsg) {
    if (this.pageReady) { this.outbox.push(m); return; }
    // Only the latest live selection matters (per source for other tools'); pinned ranges all wait
    if (m.type === 'editorSelection') this.shellQueue = this.shellQueue.filter(q => q.type !== 'editorSelection');
    if (m.type === 'contextSelection') this.shellQueue = this.shellQueue.filter(q => q.type !== 'contextSelection' || q.source.id !== m.source.id);
    this.shellQueue.push(m);
  }

  // Whether a URL answers from where this page renders (the user's machine); undefined when the page does not say in time
  probe(url: string): Promise<{ ok: boolean; error?: string } | undefined> {
    const id = randomUUID();
    return new Promise(resolve => {
      const done = (r: { ok: boolean; error?: string } | undefined) => { clearTimeout(timer); this.probes.delete(id); resolve(r); };
      const timer = setTimeout(() => done(undefined), PROBE_WITHIN_MS);
      this.probes.set(id, done);
      this.postShell({ type: 'probeUrl', id, url });
    });
  }

  // The page posts `ready` once, at load; a sidecar that came back after it initialized needs the page to start over
  onState(state: SidecarState) {
    if (state !== 'ready') return;
    if (this.initialized) { this.pageReady = false; this.outbox.clear(); this.webview.html = this.html(); }
    this.initialized = true;
  }

  private html(): string {
    const dist = vscode.Uri.joinPath(this.opts.extensionUri, 'dist', 'webview');
    const js = this.webview.asWebviewUri(vscode.Uri.joinPath(dist, 'main.js'));
    const css = this.webview.asWebviewUri(vscode.Uri.joinPath(dist, 'main.css'));
    const nonce = randomBytes(16).toString('base64url');
    // The page requests its UI face before init arrives (webview/fontPreload.ts); validated like the engine's appearance,
    // so only one of the axis' values reaches the inline script
    const font = appearanceFromSettings(key => this.opts.setting(`appearance.${key}`)).font;
    const csp = [
      "default-src 'none'",
      `img-src ${this.webview.cspSource} https: data:`,
      // Attachment text blobs are fetched for the peek card from the same source the <img> tags already load them from;
      // loopback pages are probed before open_browser shows them (probeUrl)
      `connect-src ${this.webview.cspSource} http://localhost:* http://127.0.0.1:* https://localhost:* https://127.0.0.1:*`,
      `style-src ${this.webview.cspSource} 'unsafe-inline'`,
      `font-src ${this.webview.cspSource}`,
      `script-src 'nonce-${nonce}' 'wasm-unsafe-eval'`,
      "worker-src blob:",
      // The attention chime is a WAV synthesized in the page and played from a blob: URL (effects/chime.ts)
      "media-src blob:",
    ].join('; ');
    return `<!doctype html>
<html lang="${this.opts.locale()}">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="${css}">
<style>html,body,#root{margin:0;padding:0;height:100%;overflow:hidden}.acp-shell :focus,.acp-shell :focus-visible{outline:none!important}</style>
</head>
<body>
<div id="root"></div>
<script nonce="${nonce}">window.__acpira={host:${JSON.stringify(this.host)},font:${JSON.stringify(font)},sessionLinkBase:${JSON.stringify(this.opts.sessionLinkBase)}}</script>
<script type="module" nonce="${nonce}" src="${js}"></script>
</body>
</html>`;
  }

  dispose() {
    for (const done of [...this.probes.values()]) done(undefined);
    for (const d of this.disposables) d.dispose();
    this.detach();
  }
}

export interface BrowserPanelOpts {
  client: SidecarClient;
  extensionUri: vscode.Uri;
  look: () => BrowserLook;
  // Recently shown pages, and a page the panel just showed
  history: () => BrowserPage[];
  visited: (page: BrowserPage) => void;
  // Picked elements with their comment, for the chat
  onAdd: (item: ContextItem) => void;
  // The page's own window.acpira.context call (docs/context-api.md): what `set` resolves to, or a throw naming the refused field
  onContext: (op: PageContextOp, payload: unknown) => { shown?: boolean };
  // The page the panel showed is gone (another frame, an error in its place, a restarted panel page, the panel closed): the
  // live context it set goes with it. Called with the panel's id, since it may come before the constructor returns (an
  // engine without previews answers at once)
  onLeave: (panelId: string) => void;
  // The page in VS Code's own browser (the platform's openBrowser)
  openExternal: (url: string) => void;
  log?: (line: string) => void;
}

// The browser panel: a page from the user's own dev server in an iframe, served through the engine's preview proxy, which adds
// the element picker (src/picker) to it. The panel is its own webview running the same bundle (host 'browser',
// webview/browser/BrowserPanel.tsx) and has no sidecar view: the shell holds its proxy lease and asks again after a reconnect.
// In a remote window the proxy runs on the remote, and asExternalUri forwards its port to the user's machine, where the
// iframe loads. Picked elements go to the chat like another extension's `acpira.context.add`, and the page's own
// window.acpira.context calls like the commands themselves. Opened without a page it starts empty, on the address bar and
// its recent pages
export class BrowserPanel implements vscode.Disposable {
  readonly panelId = randomUUID();
  // The page as asked for: what the address bar shows and what a reconnect asks for again; undefined while empty
  target?: string;
  private release?: () => void;
  private ready = false;
  private disposed = false;
  // The frame (or error) waits for the page to listen
  private pending?: BrowserHostMsg;
  private disposables: vscode.Disposable[] = [];
  private onDisposed = new vscode.EventEmitter<void>();
  readonly onDidDispose = this.onDisposed.event;

  constructor(readonly panel: vscode.WebviewPanel, url: string | undefined, private opts: BrowserPanelOpts) {
    panel.webview.options = { enableScripts: true, localResourceRoots: [vscode.Uri.joinPath(opts.extensionUri, 'dist', 'webview')] };
    panel.webview.html = this.html();
    panel.title = url ? titleOf(url) : t('browser.source');
    this.disposables.push(
      panel.webview.onDidReceiveMessage((m: BrowserMsg) => this.onMessage(m)),
      panel.onDidDispose(() => this.dispose()),
    );
    if (url) this.navigate(url);
  }

  // Another page, through the same lease: the engine keeps the proxy when the site stays the same
  navigate(url: string) {
    this.target = url;
    const picker = vscode.Uri.joinPath(this.opts.extensionUri, 'dist', 'picker.js').fsPath;
    this.release = this.opts.client.preview({ panelId: this.panelId, url, picker, onOpened: r => { void this.opened(url, r); } });
  }

  postLook() {
    if (this.ready) void this.panel.webview.postMessage({ type: 'browserLook', look: this.opts.look() } satisfies BrowserHostMsg);
  }

  postHistory() {
    if (this.ready) void this.panel.webview.postMessage({ type: 'browserHistory', pages: this.opts.history() } satisfies BrowserHostMsg);
  }

  private async opened(url: string, r: { url: string; token: string } | { error: string }) {
    if (url !== this.target) return;
    if ('error' in r) { this.post({ type: 'browserError', target: url, error: r.error }); return; }
    let src: string;
    try {
      src = (await vscode.env.asExternalUri(vscode.Uri.parse(r.url, true))).toString(true);
    } catch (e) {
      this.post({ type: 'browserError', target: url, error: e instanceof Error ? e.message : String(e) });
      return;
    }
    if (url === this.target) this.post({ type: 'browserFrame', target: url, src, token: r.token });
  }

  // A frame or an error, which takes the place of the page shown so far
  private post(m: BrowserHostMsg) {
    this.opts.onLeave(this.panelId);
    this.pending = m;
    if (this.ready) void this.panel.webview.postMessage(m);
  }

  private onMessage(m: BrowserMsg) {
    switch (m.type) {
      case 'browserReady':
        // A panel page that starts again (its webview reloaded) shows the page anew; the old document is gone
        this.opts.onLeave(this.panelId);
        this.ready = true;
        this.postLook();
        this.postHistory();
        if (this.pending) void this.panel.webview.postMessage(this.pending);
        break;
      case 'browserNavigate': {
        const url = normalizePreviewUrl(m.url);
        if (url) this.navigate(url);
        break;
      }
      case 'browserTitle': {
        const title = m.title.trim().slice(0, 80);
        this.panel.title = title || titleOf(m.url);
        if (normalizePreviewUrl(m.url)) this.opts.visited({ url: m.url, title });
        break;
      }
      case 'browserAdd': this.opts.onAdd(m.item); break;
      case 'browserOpenExternal': this.opts.openExternal(m.url); break;
      case 'browserContext': {
        let answer: BrowserHostMsg;
        try {
          answer = { type: 'browserContextDone', id: m.id, ...this.opts.onContext(m.op, m.payload) };
        } catch (e) {
          answer = { type: 'browserContextDone', id: m.id, error: e instanceof Error ? e.message : String(e) };
        }
        void this.panel.webview.postMessage(answer);
        break;
      }
    }
  }

  private html(): string {
    const dist = vscode.Uri.joinPath(this.opts.extensionUri, 'dist', 'webview');
    const js = this.panel.webview.asWebviewUri(vscode.Uri.joinPath(dist, 'main.js'));
    const css = this.panel.webview.asWebviewUri(vscode.Uri.joinPath(dist, 'main.css'));
    const nonce = randomBytes(16).toString('base64url');
    const { appearance, locale } = this.opts.look();
    const cspSource = this.panel.webview.cspSource;
    // Frames only from loopback, where the proxy and its forwarded port listen; a remote whose forwarding hands out a public
    // https address (Codespaces) needs https too
    const frames = ['http://127.0.0.1:*', 'http://localhost:*', ...(vscode.env.remoteName ? ['https:'] : [])].join(' ');
    const csp = [
      "default-src 'none'",
      `img-src ${cspSource} data:`,
      `style-src ${cspSource} 'unsafe-inline'`,
      `font-src ${cspSource}`,
      `script-src 'nonce-${nonce}' 'wasm-unsafe-eval'`,
      `frame-src ${frames}`,
    ].join('; ');
    return `<!doctype html>
<html lang="${locale}">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="${css}">
<style>html,body,#root{margin:0;padding:0;height:100%;overflow:hidden}.acp-shell :focus,.acp-shell :focus-visible{outline:none!important}</style>
</head>
<body>
<div id="root"></div>
<script nonce="${nonce}">window.__acpira={host:"browser",font:${JSON.stringify(appearance.font)}}</script>
<script type="module" nonce="${nonce}" src="${js}"></script>
</body>
</html>`;
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.opts.onLeave(this.panelId);
    this.release?.();
    this.release = undefined;
    for (const d of this.disposables.splice(0)) d.dispose();
    this.onDisposed.fire();
    this.onDisposed.dispose();
  }
}

// `localhost:5173` until the page names itself
function titleOf(url: string): string {
  try { return new URL(url).host; } catch { return url; }
}
