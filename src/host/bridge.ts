import { randomBytes } from 'node:crypto';
import * as vscode from 'vscode';
import type { WebviewHost, WebviewMsg } from '@shared/protocol';
import type { BridgeCore } from './bridgeCore';
import type { HostRuntime } from './runtime';
import type { SessionViewer } from './SessionManager';

// One bridge per VS Code webview: renders the HTML (CSP, bundle URIs, host flag), hands incoming messages to its BridgeCore and posts
// the core's messages back. Routing and every decision live in BridgeCore; this file only knows the vscode.Webview API
export class WebviewBridge implements vscode.Disposable {
  readonly core: BridgeCore;
  private disposables: vscode.Disposable[] = [];

  constructor(
    private webview: vscode.Webview,
    private host: WebviewHost,
    private runtime: HostRuntime,
    private extensionUri: vscode.Uri,
    initial?: string | { mostRecent: true },
  ) {
    webview.options = { enableScripts: true, localResourceRoots: [vscode.Uri.joinPath(extensionUri, 'dist', 'webview'), vscode.Uri.file(runtime.sessionsDir)] };
    webview.html = this.html();
    this.core = runtime.attachView({
      host, initial,
      // Attachment blobs are served to the webview straight from the sessions directory
      blobBase: webview.asWebviewUri(vscode.Uri.file(runtime.sessionsDir)).toString(),
      post: m => { void webview.postMessage(m); },
    });
    this.disposables.push(webview.onDidReceiveMessage((m: WebviewMsg) => { void this.core.handle(m); }));
  }

  get viewer(): SessionViewer { return this.core.viewer; }

  private html(): string {
    const dist = vscode.Uri.joinPath(this.extensionUri, 'dist', 'webview');
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
<html lang="${this.runtime.settings.locale()}">
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
    this.runtime.detachView(this.core);
  }
}
