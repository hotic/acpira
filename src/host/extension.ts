import * as vscode from 'vscode';
import { WebviewBridge } from './bridge';
import { createHostRuntime } from './runtime';
import { VscodePlatform } from './vscodePlatform';

const VIEW_ID = 'acpira.chat';

export async function activate(context: vscode.ExtensionContext) {
  const log = vscode.window.createOutputChannel('Acpira', { log: true });
  // Everything host-side that is not the vscode API lives in the runtime (registry, accounts, sessions, settings); VscodePlatform is
  // its view of this IDE. Settings changes and window focus reach the runtime through the platform's subscriptions
  const runtime = await createHostRuntime(new VscodePlatform(context, log));
  const manager = runtime.manager;

  // Every webview (the sidebar, each editor tab) is its own bridge with its own viewer, so tabs show different sessions side by side
  let sidebar: WebviewBridge | undefined;
  const attach = (webview: vscode.Webview, host: 'sidebar' | 'editor', initial?: string | { mostRecent: true }) =>
    new WebviewBridge(webview, host, runtime, context.extensionUri, initial);

  // A new tab is a new conversation: without a session id (title bar / command palette) it opens on a fresh session; a webview passing its id opens that one
  const openEditor = (sessionId?: unknown) => {
    const panel = vscode.window.createWebviewPanel('acpira.editor', 'Acpira', vscode.ViewColumn.Active, { retainContextWhenHidden: true });
    panel.iconPath = {
      light: vscode.Uri.joinPath(context.extensionUri, 'media', 'icon-light.svg'),
      dark: vscode.Uri.joinPath(context.extensionUri, 'media', 'icon.svg'),
    };
    const b = attach(panel.webview, 'editor', typeof sessionId === 'string' ? sessionId : undefined);
    const sub = b.viewer.subscribe(ev => { if (ev.type === 'session') panel.title = ev.session.title; });
    panel.onDidDispose(() => { sub(); b.dispose(); });
  };

  context.subscriptions.push(
    log,
    vscode.window.registerWebviewViewProvider(VIEW_ID, {
      resolveWebviewView(view) {
        const b = sidebar = attach(view.webview, 'sidebar', { mostRecent: true });
        view.onDidDispose(() => { if (sidebar === b) sidebar = undefined; b.dispose(); });
      },
    }, { webviewOptions: { retainContextWhenHidden: true } }),

    vscode.commands.registerCommand('acpira.openView', () => vscode.commands.executeCommand('acpira.chat.focus')),
    vscode.commands.registerCommand('acpira.newSession', () => (sidebar?.viewer ?? manager).newSession()),
    vscode.commands.registerCommand('acpira.showLog', () => log.show()),
    vscode.commands.registerCommand('acpira.openInEditor', openEditor),
    // Same tab, then moved into a fresh auxiliary window — each call makes another floating chat window
    vscode.commands.registerCommand('acpira.openInNewWindow', (sessionId?: unknown) => {
      openEditor(sessionId);
      void vscode.commands.executeCommand('workbench.action.moveEditorToNewWindow');
    }),

    { dispose: () => { void runtime.dispose(); } },
  );
}

export function deactivate() {}
