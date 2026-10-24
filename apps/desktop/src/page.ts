import { randomBytes } from 'node:crypto';

// The HTML around the webview bundle (dist/webview), served at acpira-app://app/index.html. Mirrors the VS Code shell's
// page (apps/vscode/src/bridge.ts html()): the same CSP shape with the app protocol as the resource source, the same inline
// config the bundle reads before init (window.__acpira), plus `shell: 'desktop'` for the desktop-only branches

export const APP_ORIGIN = 'acpira-app://app';

export interface PageConfig {
  host: 'editor' | 'browser';
  locale: string;
  dark: boolean;
  // appearance.font, validated by the caller
  font?: string;
  sessionLinkBase: string;
  // A browser tab of a chat window's tool panel (host 'browser' in a frame): the page talks through its parent
  // (webview/panel/browserTabs.ts), there being no preload in a frame
  tab?: string;
}

// What VS Code provides around a webview: the --vscode-* roles tokens.css reads and the scheme class on body. Values
// follow VS Code's default dark / light themes (the harness's test/host-preview/host.css)
export const THEME_CSS = `html, body, #root { margin: 0; padding: 0; height: 100%; overflow: hidden; }
body {
  --vscode-foreground: #cccccc;
  --vscode-sideBar-background: #181818;
  --vscode-editor-background: #1f1f1f;
  --vscode-widget-border: #313131;
  --vscode-focusBorder: #0078d4;
  --vscode-editor-font-family: "SF Mono", Menlo, Monaco, Consolas, monospace;
  background: var(--vscode-sideBar-background);
  color: var(--vscode-foreground);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
}
body.vscode-light {
  --vscode-foreground: #3b3b3b;
  --vscode-sideBar-background: #f8f8f8;
  --vscode-editor-background: #ffffff;
  --vscode-widget-border: #d4d4d4;
  --vscode-focusBorder: #005fb8;
}
.acp-shell :focus, .acp-shell :focus-visible { outline: none !important; }
`;

const escapeAttr = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');

export function pageHtml(c: PageConfig): string {
  const nonce = randomBytes(16).toString('base64url');
  const src = 'acpira-app:';
  const csp = [
    "default-src 'none'",
    `img-src ${src} https: data:`,
    // Attachment text blobs are fetched for the peek card; loopback pages are probed before open_browser shows them
    `connect-src ${src} http://localhost:* http://127.0.0.1:* https://localhost:* https://127.0.0.1:*`,
    `style-src ${src} 'unsafe-inline'`,
    `font-src ${src} data:`,
    `script-src 'nonce-${nonce}' 'wasm-unsafe-eval'`,
    'worker-src blob:',
    // The attention chime is a WAV synthesized in the page (effects/chime.ts)
    'media-src blob:',
    // The browser panel frames the preview proxy, which listens on loopback (the engine runs on this machine)
    ...(c.host === 'browser' ? ['frame-src http://127.0.0.1:* http://localhost:*'] : []),
    // The tool panel's browser tabs are this app's own browser page, framed
    ...(c.host === 'editor' ? [`frame-src ${src}`] : []),
  ].join('; ');
  const config = { host: c.host, font: c.font, sessionLinkBase: c.sessionLinkBase, shell: 'desktop', ...(c.tab ? { tab: c.tab } : {}) };
  const json = (v: unknown) => JSON.stringify(v).replace(/</g, '\\u003c');
  return `<!doctype html>
<html lang="${escapeAttr(c.locale)}">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Acpira</title>
<link rel="stylesheet" href="${APP_ORIGIN}/webview/main.css">
<link rel="stylesheet" href="${APP_ORIGIN}/desktop.css">
</head>
<body class="${c.dark ? 'vscode-dark' : 'vscode-light'}">
<div id="root"></div>
<script nonce="${nonce}">window.__acpira=${json(config)}${c.tab ? `;window.__acpiraApi=parent.__acpiraBrowserTab(${json(c.tab)})` : ''}</script>
<script type="module" nonce="${nonce}" src="${APP_ORIGIN}/webview/main.js"></script>
</body>
</html>`;
}
