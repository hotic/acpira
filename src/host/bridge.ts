import { randomBytes, randomUUID } from 'node:crypto';
import * as vscode from 'vscode';
import type { HostMsg, WebviewHost, WebviewMsg } from '@shared/protocol';
import type { SessionView } from '@shared/transcript';
import type { SidecarClient, SidecarState, ShellView } from './shell/SidecarClient';

export interface WebviewBridgeOpts {
  client: SidecarClient;
  extensionUri: vscode.Uri;
  sessionsDir: string;
  locale: () => string;
  // Every session the view shows (init, then each push), for tab titles and one-shot watchers
  onSession?: (session: SessionView) => void;
}

// One bridge per VS Code webview: renders the HTML (CSP, bundle URIs, host flag) and relays between the webview and its view in the
// sidecar. Routing and every decision live in the sidecar; this file only knows the vscode.Webview API
export class WebviewBridge implements vscode.Disposable, ShellView {
  readonly viewId = randomUUID();
  readonly blobBase: string;
  private disposables: vscode.Disposable[] = [];
  private detach: () => void;
  private initialized = false;

  constructor(
    private webview: vscode.Webview,
    readonly host: WebviewHost,
    private opts: WebviewBridgeOpts,
    readonly initial?: string | { mostRecent: true },
  ) {
    const sessions = vscode.Uri.file(opts.sessionsDir);
    webview.options = { enableScripts: true, localResourceRoots: [vscode.Uri.joinPath(opts.extensionUri, 'dist', 'webview'), sessions] };
    webview.html = this.html();
    // Attachment blobs are served to the webview straight from the sessions directory, through this webview's own resource URI
    this.blobBase = webview.asWebviewUri(sessions).toString();
    this.disposables.push(webview.onDidReceiveMessage((m: WebviewMsg) => opts.client.send(this.viewId, m)));
    this.detach = opts.client.attach(this);
  }

  send(m: WebviewMsg) { this.opts.client.send(this.viewId, m); }

  onHostMessage(m: HostMsg) {
    const session = m.type === 'session' ? m.session : m.type === 'init' ? m.state.active : undefined;
    if (session) this.opts.onSession?.(session);
    void this.webview.postMessage(m);
  }

  // The page posts `ready` once, at load; a sidecar that came back after it initialized needs the page to start over
  onState(state: SidecarState) {
    if (state !== 'ready') return;
    if (this.initialized) this.webview.html = this.html();
    this.initialized = true;
  }

  private html(): string {
    const dist = vscode.Uri.joinPath(this.opts.extensionUri, 'dist', 'webview');
    const js = this.webview.asWebviewUri(vscode.Uri.joinPath(dist, 'main.js'));
    const css = this.webview.asWebviewUri(vscode.Uri.joinPath(dist, 'main.css'));
    const nonce = randomBytes(16).toString('base64url');
    const csp = [
      "default-src 'none'",
      `img-src ${this.webview.cspSource} https: data:`,
      // Attachment text blobs are fetched for the peek card from the same source the <img> tags already load them from
      `connect-src ${this.webview.cspSource}`,
      `style-src ${this.webview.cspSource} 'unsafe-inline'`,
      `font-src ${this.webview.cspSource}`,
      `script-src 'nonce-${nonce}' 'wasm-unsafe-eval'`,
      "worker-src blob:",
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
<script nonce="${nonce}">window.__acpira={host:${JSON.stringify(this.host)}}</script>
<script type="module" nonce="${nonce}" src="${js}"></script>
</body>
</html>`;
  }

  dispose() {
    for (const d of this.disposables) d.dispose();
    this.detach();
  }
}
