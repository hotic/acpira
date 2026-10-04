import { join } from 'node:path';
import * as vscode from 'vscode';
import type { EditorSelection, HostMsg } from '@shared/protocol';
import type { SessionView } from '@shared/transcript';
import { FileVault } from './accounts/AccountStore';
import { WebviewBridge } from './bridge';
import { setHostLocale, t } from './i18n';
import { SidecarClient } from './shell/SidecarClient';
import { sidecarCommands } from './shell/sidecarLocator';
import { acpiraHome, migrateOnce } from './store/dataDir';
import { EDITOR_VIEW_TYPE, VscodePlatform } from './vscodePlatform';
import { editorSelectionOf } from './editorSelection';

const VIEW_ID = 'acpira.chat';
const EDITOR_STATE_SESSION = 'acpiraSessionId';

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
  // Every live chat webview, for the editor state broadcast below; a page that (re)initializes gets the current selection again
  const bridges = new Set<WebviewBridge>();
  // The chat last focused, clicked into or brought into view: where "Add to Chat" goes. Editor tabs are revealed through their panel
  let lastFocused: WebviewBridge | undefined;
  const panels = new Map<WebviewBridge, vscode.WebviewPanel>();
  const attach = (webview: vscode.Webview, host: 'sidebar' | 'editor', initial?: string | { mostRecent: true }, onSession?: (s: SessionView) => void) => {
    const b = new WebviewBridge(webview, host, {
      client: sidecar, extensionUri: context.extensionUri, sessionsDir, onSession, log: line => log.info(line),
      locale: () => platform.locale(),
      onPageReady: ready => {
        ready.postShell({ type: 'editorSelection', selection: liveSelection });
        if (lastCopy) ready.postShell({ type: 'editorCopy', selection: lastCopy });
      },
      onFocus: focused => { lastFocused = focused; },
    }, initial);
    bridges.add(b);
    return b;
  };
  const broadcast = (m: HostMsg) => { for (const b of bridges) b.postShell(m); };

  // The editor's live selection. Focus moving into a chat keeps the last one (that is when it gets used); only another file editor
  // becoming active, a collapsed selection, or closing the document clears it. Selections are debounced: a drag fires dozens
  const shareSelection = () => vscode.workspace.getConfiguration('acpira').get<boolean>('shareEditorSelection', true);
  let liveSelection: EditorSelection | undefined;
  // Kept for pages that open (or reload after a sidecar restart) after the copy: a paste there must still become a range chip
  let lastCopy: EditorSelection | undefined;
  let selectionTimer: NodeJS.Timeout | undefined;
  const selectionIn = (editor: vscode.TextEditor | undefined): EditorSelection | undefined => {
    // A document closed inside the debounce window must not be offered after its close cleared it
    if (!editor || editor.document.isClosed || editor.document.uri.scheme !== 'file') return undefined;
    const s = editor.selection;
    if (s.isEmpty) return undefined;
    return editorSelectionOf(editor.document.uri.toString(), s.start, s.end, editor.document.getText(s));
  };
  const publishSelection = (next: EditorSelection | undefined) => {
    if (!liveSelection && !next) return;
    liveSelection = next;
    broadcast({ type: 'editorSelection', selection: next });
  };
  const trackSelection = (editor: vscode.TextEditor | undefined) => {
    clearTimeout(selectionTimer);
    selectionTimer = setTimeout(() => publishSelection(shareSelection() ? selectionIn(editor) : undefined), 150);
  };

  // "Add to chat" lands in the chat the user last looked at: an editor tab if one was active since, else the sidebar
  const addToChat = (selection: EditorSelection) => {
    const m: HostMsg = { type: 'addSelection', selection };
    const panel = lastFocused && panels.get(lastFocused);
    if (panel && lastFocused) {
      panel.reveal(undefined, false);
      lastFocused.postShell(m);
      return;
    }
    void vscode.commands.executeCommand(`${VIEW_ID}.focus`);
    withSidebar(b => b.postShell(m));
  };

  const bindEditor = (panel: vscode.WebviewPanel, sessionId?: string, watch?: (s: SessionView) => void): WebviewBridge => {
    panel.iconPath = {
      light: vscode.Uri.joinPath(context.extensionUri, 'media', 'icon-light.svg'),
      dark: vscode.Uri.joinPath(context.extensionUri, 'media', 'icon.svg'),
    };
    const b = attach(panel.webview, 'editor', sessionId, s => { panel.title = s.title; watch?.(s); });
    panels.set(b, panel);
    lastFocused = b;
    panel.onDidChangeViewState(e => { if (e.webviewPanel.active) lastFocused = b; });
    panel.onDidDispose(() => {
      if (lastFocused === b) lastFocused = undefined;
      panels.delete(b);
      bridges.delete(b);
      b.dispose();
    });
    return b;
  };

  // A new tab is a new conversation: without a session id (title bar / command palette) it opens on a fresh session; a webview passing its
  // id opens that one. The tab title follows the session it shows
  function openEditor(sessionId?: unknown, watch?: (s: SessionView) => void): WebviewBridge {
    const panel = vscode.window.createWebviewPanel(EDITOR_VIEW_TYPE, 'Acpira', vscode.ViewColumn.Active, { retainContextWhenHidden: true });
    return bindEditor(panel, typeof sessionId === 'string' ? sessionId : undefined, watch);
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
        // Bringing the sidebar chat into view makes it the "Add to Chat" target again (clicking into it does too, via viewFocus)
        view.onDidChangeVisibility(() => { if (view.visible) lastFocused = b; });
        view.onDidDispose(() => {
          if (sidebar === b) sidebar = undefined;
          if (lastFocused === b) lastFocused = undefined;
          bridges.delete(b);
          b.dispose();
        });
        for (const fn of sidebarPending.splice(0)) fn(b);
      },
    }, { webviewOptions: { retainContextWhenHidden: true } }),
    vscode.window.registerWebviewPanelSerializer(EDITOR_VIEW_TYPE, {
      deserializeWebviewPanel(panel, state) {
        const saved = state && typeof state === 'object' ? (state as Record<string, unknown>)[EDITOR_STATE_SESSION] : undefined;
        const sessionId = typeof saved === 'string' ? saved : undefined;
        bindEditor(panel, sessionId);
        return Promise.resolve();
      },
    }),

    vscode.window.onDidChangeTextEditorSelection(e => trackSelection(e.textEditor)),
    vscode.window.onDidChangeActiveTextEditor(editor => { if (editor) trackSelection(editor); }),
    vscode.workspace.onDidCloseTextDocument(doc => {
      if (liveSelection?.uri !== doc.uri.toString()) return;
      clearTimeout(selectionTimer);
      publishSelection(undefined);
    }),
    vscode.workspace.onDidChangeConfiguration(e => { if (e.affectsConfiguration('acpira.shareEditorSelection')) trackSelection(vscode.window.activeTextEditor); }),
    { dispose: () => clearTimeout(selectionTimer) },
    // Editor context menu: pin the selection (or, without one, the caret's line) into the chat regardless of the live-selection setting
    vscode.commands.registerTextEditorCommand('acpira.addSelection', editor => {
      const s = editor.selection;
      const range = s.isEmpty ? editor.document.lineAt(s.active.line).range : s;
      const picked = editorSelectionOf(editor.document.uri.toString(), range.start, range.end, editor.document.getText(range));
      if (picked) addToChat(picked);
    }),
    // Copy capture: VS Code hands paste providers the copied ranges, which is the only way to learn where clipboard text came from.
    // Nothing is ever pasted by this provider; the webview turns a matching paste into a selection chip (`editorCopy`)
    vscode.languages.registerDocumentPasteEditProvider({ scheme: 'file' }, {
      prepareDocumentPaste(document, ranges) {
        if (ranges.length !== 1) return;
        const range = ranges[0]!;
        const copied = editorSelectionOf(document.uri.toString(), range.start, range.end, document.getText(range));
        if (!copied) return;
        lastCopy = copied;
        broadcast({ type: 'editorCopy', selection: copied });
      },
    }, { providedPasteEditKinds: [], copyMimeTypes: ['application/vnd.acpira.copy'] }),
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
  trackSelection(vscode.window.activeTextEditor);
}

// VS Code waits for the returned promise: the sidecar gets its shutdown and takes its agent processes with it
export function deactivate(): Promise<void> | undefined {
  const c = client;
  client = undefined;
  return c?.dispose();
}
