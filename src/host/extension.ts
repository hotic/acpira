import { join } from 'node:path';
import * as vscode from 'vscode';
import type { SessionView } from '@shared/transcript';
import { FileVault } from './accounts/AccountStore';
import { WebviewBridge } from './bridge';
import { setHostLocale, t } from './i18n';
import { SidecarClient } from './shell/SidecarClient';
import { sidecarCommands } from './shell/sidecarLocator';
import { acpiraHome, migrateOnce } from './store/dataDir';
import { VscodePlatform } from './vscodePlatform';

const VIEW_ID = 'acpira.chat';

let client: SidecarClient | undefined;

// The extension is a shell around the Rust sidecar its platform package carries: sessions, agents, accounts and settings logic run
// there, the extension renders webviews and carries out IDE actions. Same protocol as the IntelliJ plugin
export async function activate(context: vscode.ExtensionContext) {
  const log = vscode.window.createOutputChannel('Acpira', { log: true });
  const platform = new VscodePlatform(context, id => { openEditor(id); });
  setHostLocale(platform.locale());

  // The legacy globalStorage / SecretStorage tree only VS Code can read is merged before the sidecar takes over ~/.acpira
  const home = acpiraHome();
  await migrateOnce({ from: platform.legacy.from, to: home, oldVault: platform.legacy.vault, newVault: new FileVault(join(home, 'secrets.json'), l => log.info(l)), log: l => log.info(l) });

  const sidecar = client = new SidecarClient({
    commands: () => sidecarCommands({ root: context.extensionPath, env: process.env }),
    cwd: () => platform.cwd(),
    hello: () => platform.hello(),
    onRequest: r => platform.handle(r),
    log: line => log.info(line),
    onState: (state, detail) => {
      if (state !== 'failed') return;
      const retry = t('host.sidecarRetry'), show = t('host.showLog');
      void vscode.window.showErrorMessage(t('host.sidecarFailed', { detail: detail ?? '' }), retry, show).then(pick => {
        if (pick === retry) sidecar.retry();
        else if (pick === show) log.show();
      });
    },
  });

  // Every webview (the sidebar, each editor tab) is its own view in the sidecar, so tabs show different sessions side by side
  const sessionsDir = join(home, 'sessions');
  let sidebar: WebviewBridge | undefined;
  const sidebarPending: ((b: WebviewBridge) => void)[] = [];
  const attach = (webview: vscode.Webview, host: 'sidebar' | 'editor', initial?: string | { mostRecent: true }, onSession?: (s: SessionView) => void) =>
    new WebviewBridge(webview, host, {
      client: sidecar, extensionUri: context.extensionUri, sessionsDir, onSession,
      locale: () => platform.locale(),
    }, initial);

  // A new tab is a new conversation: without a session id (title bar / command palette) it opens on a fresh session; a webview passing its
  // id opens that one. The tab title follows the session it shows
  function openEditor(sessionId?: unknown, watch?: (s: SessionView) => void): WebviewBridge {
    const panel = vscode.window.createWebviewPanel('acpira.editor', 'Acpira', vscode.ViewColumn.Active, { retainContextWhenHidden: true });
    panel.iconPath = {
      light: vscode.Uri.joinPath(context.extensionUri, 'media', 'icon-light.svg'),
      dark: vscode.Uri.joinPath(context.extensionUri, 'media', 'icon.svg'),
    };
    const b = attach(panel.webview, 'editor', typeof sessionId === 'string' ? sessionId : undefined, s => { panel.title = s.title; watch?.(s); });
    panel.onDidDispose(() => b.dispose());
    return b;
  }

  // The sidebar exists once VS Code resolves it; a command that needs it before then waits for it
  const withSidebar = (fn: (b: WebviewBridge) => void) => {
    if (sidebar) { fn(sidebar); return; }
    sidebarPending.push(fn);
    void vscode.commands.executeCommand(`${VIEW_ID}.focus`);
  };

  context.subscriptions.push(
    log,
    platform.subscribe(ev => {
      if (ev.type === 'settingsChanged' && ev.keys.includes('language')) setHostLocale(platform.locale());
      sidecar.event(ev);
    }),
    vscode.window.registerWebviewViewProvider(VIEW_ID, {
      resolveWebviewView(view) {
        const b = sidebar = attach(view.webview, 'sidebar', { mostRecent: true });
        view.onDidDispose(() => { if (sidebar === b) sidebar = undefined; b.dispose(); });
        for (const fn of sidebarPending.splice(0)) fn(b);
      },
    }, { webviewOptions: { retainContextWhenHidden: true } }),

    vscode.commands.registerCommand('acpira.openView', () => vscode.commands.executeCommand(`${VIEW_ID}.focus`)),
    vscode.commands.registerCommand('acpira.newSession', () => withSidebar(b => b.send({ type: 'newSession' }))),
    vscode.commands.registerCommand('acpira.showLog', () => log.show()),
    // A new tab bound to a fresh ChatGPT mirror; its connection prompt goes to the clipboard as soon as the tab shows it
    vscode.commands.registerCommand('acpira.connectChatgpt', () => {
      let copied = false;
      openEditor(undefined, s => {
        const prompt = s.external?.connectionPrompt;
        if (copied || !prompt) return;
        copied = true;
        void vscode.env.clipboard.writeText(prompt).then(() => vscode.window.showInformationMessage(t('chatgpt.copied')));
      }).send({ type: 'connectChatgpt' });
    }),
    vscode.commands.registerCommand('acpira.openInEditor', (sessionId?: unknown) => { openEditor(sessionId); }),
    // Same tab, then moved into a fresh auxiliary window — each call makes another floating chat window
    vscode.commands.registerCommand('acpira.openInNewWindow', (sessionId?: unknown) => {
      openEditor(sessionId);
      void vscode.commands.executeCommand('workbench.action.moveEditorToNewWindow');
    }),
  );

  sidecar.start();
}

// VS Code waits for the returned promise: the sidecar gets its shutdown and takes its agent processes with it
export function deactivate(): Promise<void> | undefined {
  const c = client;
  client = undefined;
  return c?.dispose();
}
