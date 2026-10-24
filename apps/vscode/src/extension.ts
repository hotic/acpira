import { existsSync } from 'node:fs';
import { join } from 'node:path';
import * as vscode from 'vscode';
import { appearanceFromSettings } from '@acpira/shared/appearance';
import { normalizePreviewUrl, type PageContextOp } from '@acpira/shared/picker';
import type { BrowserLook, BrowserPage, ContextItem, EditorSelection, HostMsg } from '@acpira/shared/protocol';
import { sanitizeSetting } from '@acpira/shared/settings';
import type { BrowserOpened, PlatformRequest } from '@acpira/shared/sidecar';
import type { SessionView } from '@acpira/shared/transcript';
import { sessionIdOfLink } from '@acpira/shared/sessionLinks';
import { FileVault } from './accounts/AccountStore';
import { BrowserPanel, WebviewBridge } from './bridge';
import { setHostLocale, t } from './i18n';
import { SidecarClient } from '@acpira/engine-client/SidecarClient';
import { engineEndpoint } from '@acpira/engine-client/engine';
import { sidecarCommands } from '@acpira/engine-client/sidecarLocator';
import { acpiraHome, migrateOnce } from './store/dataDir';
import { EDITOR_VIEW_TYPE, fileColumn, VscodePlatform } from './vscodePlatform';
import { editorSelectionOf } from './editorSelection';
import { ContextPayloadError, ContextSlots, pageOwner, parseContextPayload, type ContextPayload } from '@acpira/engine-client/contextApi';
import { availableChromeCommands, columnPicks, columnsLayout, defaultColumnPick, type ColumnPick } from './multiOpen';
import { showSystemNotification } from './systemNotify';

const VIEW_ID = 'acpira.chat';
const EDITOR_STATE_SESSION = 'acpiraSessionId';
const BROWSER_VIEW_TYPE = 'acpira.browser';
// Kept in sync with webview/browser/BrowserPanel.tsx
const BROWSER_STATE_URL = 'acpiraBrowserUrl';

let client: SidecarClient | undefined;

// A packaged extension carries its page, picker, icons and engine next to its manifest. Run from the repository (F5 on
// apps/vscode) they are the repository's shared dist/ and media/, two levels up
function resourceRoot(context: vscode.ExtensionContext): vscode.Uri {
  return context.extensionMode === vscode.ExtensionMode.Development ? vscode.Uri.joinPath(context.extensionUri, '..', '..') : context.extensionUri;
}

// The extension is a shell around the Rust sidecar its platform package carries: sessions, agents, accounts and settings logic run
// there, the extension renders webviews and carries out IDE actions. Same protocol as the IntelliJ plugin
export async function activate(context: vscode.ExtensionContext) {
  const log = vscode.window.createOutputChannel('Acpira', { log: true });
  const resources = resourceRoot(context);
  const platform = new VscodePlatform(context, id => { openEditor(id); }, r => announce(r), url => probeUrl(url), url => showPreview(url, true));
  setHostLocale(platform.locale());

  // The legacy globalStorage / SecretStorage tree only VS Code can read is merged before the sidecar takes over ~/.acpira
  const home = acpiraHome();
  await migrateOnce({ from: platform.legacy.from, to: home, oldVault: platform.legacy.vault, newVault: new FileVault(join(home, 'secrets.json'), l => log.info(l)), log: l => log.info(l) });

  // Sessions run in a persistent engine on the machine that hosts the workspace (the SSH server for Remote-SSH): closing the
  // window, reloading it or losing the connection leaves running turns going, and the next window for the workspace reconnects.
  // Windows has no Unix sockets for it and keeps the child-process sidecar; ACPIRA_ENGINE=stdio opts out anywhere (debugging)
  const persistent = process.platform !== 'win32' && process.env.ACPIRA_ENGINE !== 'stdio';
  const sidecar = client = new SidecarClient({
    commands: () => sidecarCommands({ root: resources.fsPath, env: process.env }),
    engine: cmd => persistent ? engineEndpoint({ binary: cmd.command, home, cwd: platform.cwd() }) : undefined,
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
  // Copied session links open through this window's URI handler (`cursor://` under Cursor, `vscode-insiders://` …)
  const sessionLinkBase = `${vscode.env.uriScheme}://${context.extension.id}/session/`;
  const attach = (webview: vscode.Webview, host: 'sidebar' | 'editor', visible: () => boolean, initial?: string | { mostRecent: true }, onSession?: (s: SessionView) => void) => {
    const b = new WebviewBridge(webview, host, {
      client: sidecar, extensionUri: resources, sessionsDir, onSession, visible, log: line => log.info(line),
      locale: () => platform.locale(), setting: key => platform.setting(key), sessionLinkBase,
      onPageReady: ready => {
        ready.postShell({ type: 'editorSelection', selection: liveSelection });
        if (lastCopy) ready.postShell({ type: 'editorCopy', selection: lastCopy });
        if (shareSelection()) for (const m of contextSlots.replay()) ready.postShell(m);
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

  // Other extensions' live selections (`acpira.context.set`), one slot per source, under the same sharing switch as the editor's
  const contextSlots = new ContextSlots();
  const publishContext = (m: HostMsg | undefined) => { if (m && shareSelection()) broadcast(m); };
  // A slot whose owner's tab closed without clearing it (the extension crashed, or never clears) goes with the tab
  const tabUris = (tabs: readonly vscode.Tab[]) => tabs.flatMap(tab => {
    const uri = (tab.input as { uri?: unknown } | undefined)?.uri;
    return uri instanceof vscode.Uri ? [uri.toString()] : [];
  });
  const releaseClosedOwners = (closed: readonly vscode.Tab[]) => {
    const owners = contextSlots.owners();
    if (!owners.size) return;
    const open = new Set(tabUris(vscode.window.tabGroups.all.flatMap(g => g.tabs)));
    const gone = new Set(tabUris(closed).filter(u => owners.has(u) && !open.has(u)));
    if (gone.size) for (const m of contextSlots.release(gone)) publishContext(m);
  };

  // "Add to chat" lands in the chat the user last looked at: an editor tab if one was active since, else the sidebar. A chat tab
  // in the `from` column (the browser panel's group) stays behind it: bringing it up would hide the page being picked from
  const addToChat = (m: HostMsg, from?: vscode.ViewColumn) => {
    const panel = lastFocused && panels.get(lastFocused);
    if (panel && lastFocused) {
      if (from === undefined || panel.viewColumn !== from) panel.reveal(undefined, false);
      lastFocused.postShell(m);
      return;
    }
    void vscode.commands.executeCommand(`${VIEW_ID}.focus`);
    withSidebar(b => b.postShell(m));
  };
  // `acpira.context.add` and a page's window.acpira.context.add: a parsed payload pinned into the chat used last
  const pinContext = (p: ContextPayload, from?: vscode.ViewColumn) => {
    if (!p.items.length) throw new ContextPayloadError('acpira.context: add needs at least one item');
    addToChat({ type: 'addContext', source: p.source, items: p.items }, from);
  };

  const bindEditor = (panel: vscode.WebviewPanel, sessionId?: string, watch?: (s: SessionView) => void): WebviewBridge => {
    panel.iconPath = {
      light: vscode.Uri.joinPath(resources, 'media', 'icon-light.svg'),
      dark: vscode.Uri.joinPath(resources, 'media', 'icon.svg'),
    };
    const b = attach(panel.webview, 'editor', () => panel.visible, sessionId, s => { panel.title = s.title; watch?.(s); });
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
  function openEditor(sessionId?: unknown, watch?: (s: SessionView) => void, column: vscode.ViewColumn = vscode.ViewColumn.Active): WebviewBridge {
    const panel = vscode.window.createWebviewPanel(EDITOR_VIEW_TYPE, 'Acpira', column, { retainContextWhenHidden: true });
    return bindEditor(panel, typeof sessionId === 'string' ? sessionId : undefined, watch);
  }

  // The column count, starting on the default: showQuickPick cannot preselect an item in single-select mode (`picked` is
  // multi-select only), so the picker is built by hand with the default as its active item. Undefined when dismissed
  const pickColumns = () => new Promise<ColumnPick | undefined>(resolve => {
    const qp = vscode.window.createQuickPick<ColumnPick>();
    qp.title = t('host.columnsTitle');
    qp.placeholder = t('host.columnsPlaceholder');
    qp.items = columnPicks();
    const initial = defaultColumnPick(qp.items);
    if (initial) qp.activeItems = [initial];
    let chosen: ColumnPick | undefined;
    qp.onDidAccept(() => { chosen = qp.selectedItems[0] ?? qp.activeItems[0]; qp.hide(); });
    // Fires after an accept's hide as well as on Escape / focus loss; the picker is disposed either way
    qp.onDidHide(() => { qp.dispose(); resolve(chosen); });
    qp.show();
  });

  // N fresh chats side by side: the side bars and panel close, every editor closes (dirty files get VS Code's usual save prompt),
  // the editor area becomes one row of N equal groups and each group gets a new tab. Cancelling the pick changes nothing
  const openEditorColumns = async () => {
    const pick = await pickColumns();
    if (!pick) return;
    const n = pick.columns;
    try {
      const registered = await vscode.commands.getCommands(true);
      for (const cmd of availableChromeCommands(registered)) {
        try { await vscode.commands.executeCommand(cmd); } catch (e) { log.warn(`${cmd} failed: ${String(e)}`); }
      }
      await vscode.commands.executeCommand('workbench.action.closeAllEditors');
      await vscode.commands.executeCommand('vscode.setEditorLayout', columnsLayout(n));
      // Created in order so the last tab ends up focused; ViewColumn i + 1 is the i-th group of the new layout
      for (let i = 0; i < n; i++) openEditor(undefined, undefined, (i + 1) as vscode.ViewColumn);
    } catch (e) {
      log.error(`open in editor columns failed: ${String(e)}`);
      void vscode.window.showErrorMessage(t('host.columnsFailed', { detail: e instanceof Error ? e.message : String(e) }));
    }
  };

  // Browser panels (bridge.ts BrowserPanel), one per site: opening a page of a site already shown navigates that panel
  const browsers = new Set<BrowserPanel>();
  // Pages shown in any browser panel, most recent first, for the address bar's suggestions (kept per machine)
  const HISTORY_KEY = 'acpira.browserHistory';
  const MAX_HISTORY = 20;
  const browserHistory = (): BrowserPage[] => context.globalState.get<BrowserPage[]>(HISTORY_KEY) ?? [];
  const visited = (page: BrowserPage) => {
    const pages = [page, ...browserHistory().filter(p => p.url !== page.url)].slice(0, MAX_HISTORY);
    void context.globalState.update(HISTORY_KEY, pages);
    for (const b of browsers) b.postHistory();
  };
  const browserLook = (): BrowserLook => ({
    appearance: appearanceFromSettings(key => platform.setting(`appearance.${key}`)),
    theme: sanitizeSetting('theme', platform.setting('theme')),
    uiFontSize: sanitizeSetting('uiFontSize', platform.setting('uiFontSize')),
    codeFontSize: sanitizeSetting('codeFontSize', platform.setting('codeFontSize')),
    fontSmoothing: sanitizeSetting('fontSmoothing', platform.setting('fontSmoothing')),
    locale: platform.locale(),
  });
  // Elements picked on a page, capped like any other tool's context before a chat sees them
  const addPicked = (browser: BrowserPanel, item: ContextItem) => {
    try {
      const p = parseContextPayload({ source: { id: 'browser', name: t('browser.source') }, items: [item] });
      addToChat({ type: 'addContext', source: p.source, items: p.items }, browser.panel.viewColumn);
    } catch (e) {
      void vscode.window.showWarningMessage(e instanceof Error ? e.message : String(e));
    }
  };
  // A page's own context (window.acpira.context, docs/context-api.md "Web pages"): the commands' payload and checks. Its owner
  // is filed under the panel (pageOwner), and its live slot lasts as long as the page (onLeave)
  const pageContext = (browser: BrowserPanel, op: PageContextOp, raw: unknown): { shown?: boolean } => {
    const p = parseContextPayload(raw);
    if (op === 'add') { pinContext(p, browser.panel.viewColumn); return {}; }
    publishContext(contextSlots.set({ ...p, owner: pageOwner(browser.panelId, p.owner) }));
    return { shown: shareSelection() };
  };
  const leavePage = (panelId: string) => { for (const m of contextSlots.releasePage(panelId)) publishContext(m); };
  // Show a loopback page in a browser panel (undefined: an empty one, on its address bar): the one already showing its site
  // (brought up, navigated when the page differs), else a new panel next to the chat. `preserveFocus` for an agent's
  // open_browser, so the composer keeps the caret
  const showPreview = (url: string | undefined, preserveFocus = false): BrowserOpened['reused'] => {
    const origin = url && new URL(url).origin;
    const shown = [...browsers].find(b => (origin ? b.target !== undefined && new URL(b.target).origin === origin : b.target === undefined));
    if (shown && !url) { shown.panel.reveal(undefined, preserveFocus); return 'revealed'; }
    if (shown && url) {
      shown.panel.reveal(undefined, preserveFocus);
      if (shown.target === url) return 'revealed';
      shown.navigate(url);
      return 'navigated';
    }
    const panel = vscode.window.createWebviewPanel(BROWSER_VIEW_TYPE, url ? new URL(url).host : t('browser.source'), { viewColumn: fileColumn(), preserveFocus }, { retainContextWhenHidden: true });
    bindBrowser(panel, url);
    return undefined;
  };
  // A browser panel's shell side, for a new panel and for one VS Code restores after a window reload (the page keeps its
  // address in the webview state, BROWSER_STATE_URL)
  const bindBrowser = (panel: vscode.WebviewPanel, url: string | undefined) => {
    panel.iconPath = {
      light: vscode.Uri.joinPath(resources, 'media', 'icon-light.svg'),
      dark: vscode.Uri.joinPath(resources, 'media', 'icon.svg'),
    };
    const browser: BrowserPanel = new BrowserPanel(panel, url, {
      client: sidecar,
      extensionUri: resources,
      look: browserLook,
      history: browserHistory,
      visited,
      onAdd: item => addPicked(browser, item),
      onContext: (op, raw) => pageContext(browser, op, raw),
      onLeave: leavePage,
      openExternal: page => { void platform.openInVsCodeBrowser(page).catch((e: unknown) => log.warn(`open in browser failed: ${String(e)}`)); },
      log: line => log.info(line),
    });
    browsers.add(browser);
    browser.onDidDispose(() => browsers.delete(browser));
  };
  // The palette command: a URL argument opens it, none opens the panel on its address bar
  const openPreview = (raw?: unknown) => { showPreview(typeof raw === 'string' ? normalizePreviewUrl(raw) : undefined); };

  // The sidebar exists once VS Code resolves it; a command that needs it before then waits for it
  const withSidebar = (fn: (b: WebviewBridge) => void) => {
    if (sidebar) { fn(sidebar); return; }
    sidebarPending.push(fn);
    void vscode.commands.executeCommand(`${VIEW_ID}.focus`);
  };

  // Bring a session up: the editor tab already showing it, else the sidebar. The page itself switches (showSession), so a
  // sidebar left on the settings page comes back to the chat; an unknown id gets the engine's record-load message there
  const revealSession = (sessionId: string) => {
    const showing = [...bridges].find(b => b.sessionId === sessionId);
    const panel = showing && panels.get(showing);
    if (panel) { panel.reveal(undefined, false); showing.postShell({ type: 'showSession', id: sessionId }); return; }
    void vscode.commands.executeCommand(`${VIEW_ID}.focus`);
    withSidebar(b => b.postShell({ type: 'showSession', id: sessionId }));
  };

  // open_browser's check from the user's machine runs in a chat page (they render there even in a remote window): the chat used
  // last when it is up, else any ready one. Without one the page opens unchecked
  function probeUrl(url: string) {
    const page = lastFocused?.ready ? lastFocused : [...bridges].find(b => b.ready);
    return page ? page.probe(url) : Promise.resolve(undefined);
  }

  // A session needs the user or has settled (`notify`, sent only for the switches that are on). Nothing while this window is
  // focused and a visible chat shows that session. The chime plays in one page (the chat used last when it is up); the system
  // notification is the OS's own, except in a remote window (the extension host runs on the remote machine) or where the OS
  // command is missing or fails: there VS Code's message stands in, and its button brings the session up
  function announce(r: Extract<PlatformRequest, { method: 'notify' }>) {
    if (vscode.window.state.focused && [...bridges].some(b => b.shows(r.sessionId))) return;
    if (r.sound) {
      const target = lastFocused?.ready ? lastFocused : [...bridges].find(b => b.ready);
      target?.postShell({ type: 'chime' });
    }
    if (!r.system) return;
    const inWindow = () => {
      void vscode.window.showInformationMessage(`${r.body} · ${r.title}`, r.open).then(pick => { if (pick === r.open) revealSession(r.sessionId); });
    };
    if (vscode.env.remoteName) { inWindow(); return; }
    void showSystemNotification(r.title, r.body).then(shown => { if (!shown) inWindow(); });
  }

  context.subscriptions.push(
    log,
    platform.subscribe(ev => {
      if (ev.type === 'settingsChanged' && ev.keys.includes('language')) setHostLocale(platform.locale());
      sidecar.event(ev);
    }),
    vscode.window.registerWebviewViewProvider(VIEW_ID, {
      resolveWebviewView(view) {
        const b = sidebar = attach(view.webview, 'sidebar', () => view.visible, { mostRecent: true });
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
    // Browser panels come back after a window reload on the page they showed (or empty)
    vscode.window.registerWebviewPanelSerializer(BROWSER_VIEW_TYPE, {
      deserializeWebviewPanel(panel, state) {
        const saved = state && typeof state === 'object' ? (state as Record<string, unknown>)[BROWSER_STATE_URL] : undefined;
        bindBrowser(panel, typeof saved === 'string' ? normalizePreviewUrl(saved) : undefined);
        return Promise.resolve();
      },
    }),
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
    vscode.workspace.onDidChangeConfiguration(e => {
      if (!e.affectsConfiguration('acpira.shareEditorSelection')) return;
      trackSelection(vscode.window.activeTextEditor);
      for (const m of shareSelection() ? contextSlots.replay() : contextSlots.hidden()) broadcast(m);
    }),
    // The look and language live in the shared settings file, which any engine (another window, the desktop app) writes
    (platform.shared.watch(keys => {
      if (keys.includes('language')) setHostLocale(platform.locale());
      for (const b of browsers) b.postLook();
    }), { dispose: () => platform.shared.unwatch() }),
    { dispose: () => { for (const b of [...browsers]) b.panel.dispose(); } },
    vscode.window.tabGroups.onDidChangeTabs(e => { if (e.closed.length) releaseClosedOwners(e.closed); }),
    { dispose: () => clearTimeout(selectionTimer) },
    // Editor context menu: pin the selection (or, without one, the caret's line) into the chat regardless of the live-selection setting
    vscode.commands.registerTextEditorCommand('acpira.addSelection', editor => {
      const s = editor.selection;
      const range = s.isEmpty ? editor.document.lineAt(s.active.line).range : s;
      const picked = editorSelectionOf(editor.document.uri.toString(), range.start, range.end, editor.document.getText(range));
      if (picked) addToChat({ type: 'addSelection', selection: picked });
    }),
    // The context API other extensions call (docs/context-api.md). Contributed in package.json so a first call activates Acpira;
    // hidden from the palette, since they take a payload. A bad payload rejects the caller's promise with the reason
    vscode.commands.registerCommand('acpira.context.set', (raw: unknown) => {
      publishContext(contextSlots.set(parseContextPayload(raw)));
      return { shown: shareSelection() };
    }),
    vscode.commands.registerCommand('acpira.context.add', (raw: unknown) => { pinContext(parseContextPayload(raw)); }),
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
    // Copied session links (`vscode://hotic.acpira/session/<id>`) open here. VS Code hands the URI to the frontmost window,
    // which may not be the one (or the remote host) the link came from: a session this window's store lacks is said so
    vscode.window.registerUriHandler({
      handleUri(uri) {
        const id = sessionIdOfLink(`${uri.scheme}://${uri.authority}${uri.path}`);
        if (!id) { log.warn(`unhandled URI: ${uri.path}`); return; }
        const known = [...bridges].some(b => b.sessionId === id)
          || existsSync(join(sessionsDir, `${id}.json`)) || existsSync(join(home, 'bridges', 'chatgpt', `${id}.json`));
        if (known) revealSession(id);
        else void vscode.window.showWarningMessage(t('sessionLink.notHere'));
      },
    }),
    // A local page in Acpira's browser panel, where elements can be picked into the chat; without a URL it asks for one
    vscode.commands.registerCommand('acpira.openPreview', (url?: unknown) => openPreview(url)),
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
    vscode.commands.registerCommand('acpira.openInEditorColumns', () => openEditorColumns()),
    // Same tab, then moved into a fresh auxiliary window — each call makes another floating chat window
    vscode.commands.registerCommand('acpira.openInNewWindow', (sessionId?: unknown) => {
      openEditor(sessionId);
      void vscode.commands.executeCommand('workbench.action.moveEditorToNewWindow');
    }),
  );

  sidecar.start();
  trackSelection(vscode.window.activeTextEditor);
}

// VS Code waits for the returned promise. A persistent engine is only disconnected from (its turns keep running and it ends on its own
// once idle); a child-process sidecar gets its shutdown and takes its agent processes with it
export function deactivate(): Promise<void> | undefined {
  const c = client;
  client = undefined;
  return c?.dispose();
}
