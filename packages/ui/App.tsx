import type { ChatGptIntegrationStatus } from '@acpira/shared/chatgptIntegration';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { AccountAction, AgentInstallProgress, DetectedAgent, EditTurnRequest, FileHit, HostMsg, InitState, NativeSessionsState, SessionHit, WebviewMsg } from '@acpira/shared/protocol';
import type { AccountInfo, AgentId, AgentInfo, ConfigControl, FullDiffSource, SessionCategories, SessionControls, SessionSummary, SessionView, Turn } from '@acpira/shared/transcript';
import type { ModelShapes } from '@acpira/shared/modelShapes';
import type { HiddenMap, PanelScope, SettingKey, SettingsView } from '@acpira/shared/settings';
import type { AgentInventory } from '@acpira/shared/inventory';
import { isLocale, resolveLocale, type Locale } from '@acpira/shared/i18n';
import { reuse, reuseById } from '@acpira/shared/reuse';
import { effortLabel } from '@acpira/shared/composerControls';
import { SessionViews } from './sessionViews';
import { BASE_APPEARANCE, type Appearance } from './appearance';
import { LocaleContext, setLocale, t } from './i18n';
import { Shell, type ShellHandlers } from './chat/Shell';
import { lookFromSettings, resolveTheme } from './look';
import { useVsCodeTheme } from './useVsCodeTheme';
import { settleProject } from './panel/api';
import { deliverToBrowserTab, installBrowserTabs, setRecentPages } from './panel/browserTabs';
import { panel, panelState } from './panel/store';
import { deliverTerminal, startTerminals, terminalsConnected } from './panel/terminals';
import { vscodeApi } from './vscodeApi';
import { desktopShell, pushShellToast, runShellCommand, setDesktopMenu, setUpdateState, setWindowMaximized } from './desktop';
import { deliverRemote, watchConnections } from './remote/api';
import { deliverRemoteAccess, watchRemoteAccess } from './remote/access';
import type { QuickSettings } from './chat/DesktopFrame';
import { SettingsShell, type SettingsHandlers } from './settings/SettingsShell';
import { contextDrafts, sameContext, sameRange, selectionDraft, setContextSelection, setEditorCopy, setEditorSelection } from './chat/editorContext';
import { updateMainComposer } from './chat/useComposerDraft';
import { playChime } from './effects/chime';
import { probeUrl } from './probeUrl';
import { isWidgetFrame } from './chat/AgentWidget';
import { receivePluginBackdrop, receivePluginTheme, setPluginsView, useSkin } from './plugins/skin';
import { SkinBackdrop } from './plugins/SkinBackdrop';
import { deliverPluginCalled, deliverPluginPost, setPluginSlots } from './pluginSlots';
import type { SettingsPage } from './settings/Nav';
import type { SharedState } from './settings/SharedPage';
import type { StorageState } from './settings/StoragePage';
import type { ProvidersState } from './settings/ProvidersSection';
import { SampleShell } from './welcome/SampleShell';
import { Welcome, type WelcomeMode } from './welcome/Welcome';

declare global {
  // `font`: the configured UI face, for fontPreload.ts (VS Code only says so; other hosts get the default).
  // `sessionLinkBase`: the clickable session-link base the VS Code shell injects (chat/sessionLinkBase.ts).
  // `host: 'browser'`: the browser panel's webview (main.tsx renders browser/BrowserPanel there).
  // `shell: 'desktop'`: the standalone desktop app (rust/crates/acpira-desktop/src/page.rs), which turns on the desktop layout
  // `tab`: a browser page shown as a tab of the desktop tool panel (rust/crates/acpira-desktop/src/page.rs), whose transport comes from the chat
  // `remote`: the page offers SSH hosts without being the desktop app (the browser harness, test/host-preview/shim.js)
  // `fileBase`: where the tool panel loads a local media file by absolute path (the desktop app's /file route)
  // `firstRun`: the shell found no data folder and started no engine; the first-run flow (welcome/) shows the folder as
  // `home`, in the shell's `locale`, until the user says yes (createDataHome) and the engine's init arrives
  // `tourPage`: VS Code's tab of the interface tour (openTour), which has no engine view at all
  // `opensTour`: the shell plays the tour in a tab of its own when the General page asks (VS Code)
  // `imageCopy`: the shell answers copyImage with copiedImage, putting an image on the system clipboard (VS Code, local)
  interface Window { __acpira?: { host: 'sidebar' | 'editor' | 'browser'; font?: Appearance['font']; sessionLinkBase?: string; shell?: 'desktop'; platform?: 'mac' | 'win' | 'linux'; tab?: string; remote?: boolean; fileBase?: string; firstRun?: { home: string; locale: string }; tourPage?: { locale: string }; opensTour?: boolean; imageCopy?: boolean } }
}

// Always through the memo: acquireVsCodeApi() throws when called twice, and other components (Link) reach the api via vscodeApi()
const vscode = vscodeApi();
const post = (m: WebviewMsg) => vscode.postMessage(m);
// The desktop app's page (rust/crates/acpira-desktop/src/page.rs): its frame, start page and project registry (Shell's `desktop`)
const desktop = desktopShell();
// Before any engine (the first run, VS Code's tour tab) the shell's language is the page's until init brings the setting's
const firstRun = window.__acpira?.firstRun;
const tourPage = window.__acpira?.tourPage;
const bootTag = (firstRun ?? tourPage)?.locale;
const bootLocale: Locale | undefined = bootTag === undefined ? undefined : isLocale(bootTag) ? bootTag : resolveLocale(undefined, bootTag);
if (bootLocale) setLocale(bootLocale);

// What the tour's steps light: the sample shell's root (welcome/SampleShell.tsx)
const sampleScope = () => document.querySelector<HTMLElement>('[data-sample-shell] .acp-shell');

// Replies return to the originating webview. Drafts survive a rejected edit.
const editWaits = new Map<string, { resolve: () => void; reject: (error: Error) => void }>();
const editTurn = (edit: EditTurnRequest) => new Promise<void>((resolve, reject) => {
  const requestId = crypto.randomUUID();
  editWaits.set(requestId, { resolve, reject });
  post({ type: 'editTurn', requestId, edit });
});

// Request/response pairs over postMessage: file search for @ mentions and the history list's conversation search. Each
// request gets a seq; the matching `files` / `sessionHits` reply resolves it.
// A reply that never comes (host gone) resolves empty after a while so nothing waits forever
const FILES_TIMEOUT = 5000;
let fileSeq = 0;
const fileWaits = new Map<number, (files: FileHit[]) => void>();
const settleFiles = (seq: number, files: FileHit[]) => { fileWaits.get(seq)?.(files); fileWaits.delete(seq); };
const searchFiles = (query: string) => new Promise<FileHit[]>(resolve => {
  const seq = ++fileSeq;
  fileWaits.set(seq, resolve);
  setTimeout(() => settleFiles(seq, []), FILES_TIMEOUT);
  post({ type: 'searchFiles', query, seq });
});
// The first conversation search reads every saved record, so it gets more time than a file lookup
const SESSION_SEARCH_TIMEOUT = 15000;
let sessionSeq = 0;
const sessionWaits = new Map<number, (hits: SessionHit[]) => void>();
const settleSessions = (seq: number, hits: SessionHit[]) => { sessionWaits.get(seq)?.(hits); sessionWaits.delete(seq); };
const searchSessions = (query: string) => new Promise<SessionHit[]>(resolve => {
  const seq = ++sessionSeq;
  sessionWaits.set(seq, resolve);
  setTimeout(() => settleSessions(seq, []), SESSION_SEARCH_TIMEOUT);
  post({ type: 'searchSessions', query, seq });
});

// The whole files behind a diff, which page views leave out: asked for when a diff's source is copied
const DIFF_SOURCE_TIMEOUT = 10000;
let diffSeq = 0;
const diffWaits = new Map<number, (source: FullDiffSource | undefined) => void>();
const settleDiff = (seq: number, source: FullDiffSource | undefined) => { diffWaits.get(seq)?.(source); diffWaits.delete(seq); };
const diffSource = (sessionId: string, toolCallId: string, nth: number, subagentId?: string) => new Promise<FullDiffSource | undefined>(resolve => {
  const seq = ++diffSeq;
  diffWaits.set(seq, resolve);
  setTimeout(() => settleDiff(seq, undefined), DIFF_SOURCE_TIMEOUT);
  post({ type: 'diffSource', sessionId, toolCallId, nth, ...(subagentId !== undefined ? { subagentId } : {}), seq });
});

// Stage an image file path into the session's blob store when a file reference is clicked for preview
const IMAGE_BLOB_TIMEOUT = 10000;
let imageBlobSeq = 0;
const imageBlobWaits = new Map<number, (blob: string | undefined) => void>();
const settleImageBlob = (seq: number, blob: string | undefined) => { imageBlobWaits.get(seq)?.(blob); imageBlobWaits.delete(seq); };
const imageBlob = (sessionId: string, path: string) => new Promise<string | undefined>(resolve => {
  const seq = ++imageBlobSeq;
  imageBlobWaits.set(seq, resolve);
  setTimeout(() => settleImageBlob(seq, undefined), IMAGE_BLOB_TIMEOUT);
  post({ type: 'imageBlob', sessionId, path, seq });
});

// "Copy as Markdown": the engine renders the export's Markdown and always answers; an error or no answer rejects, so the copy
// toast says it failed instead of leaving the clipboard's pending write hanging
const SESSION_MARKDOWN_TIMEOUT = 15000;
let markdownSeq = 0;
const markdownWaits = new Map<number, { resolve: (text: string) => void; reject: (error: Error) => void }>();
const settleMarkdown = (seq: number, markdown: string | undefined, error: string | undefined) => {
  const wait = markdownWaits.get(seq);
  markdownWaits.delete(seq);
  if (markdown !== undefined) wait?.resolve(markdown);
  else wait?.reject(new Error(error ?? 'no reply'));
};
const sessionMarkdown = (id: string) => new Promise<string>((resolve, reject) => {
  const seq = ++markdownSeq;
  markdownWaits.set(seq, { resolve, reject });
  setTimeout(() => settleMarkdown(seq, undefined, 'timed out'), SESSION_MARKDOWN_TIMEOUT);
  post({ type: 'sessionMarkdown', id, seq });
});

type InitEnv = Pick<InitState, 'host' | 'home' | 'cwd' | 'blobBase' | 'chats'>;

const EMPTY_TURNS: Turn[] = [];
const EMPTY_CONTROLS: SessionControls = { modes: [], options: [] };

function syncPanelScope(incoming: SettingsView): SettingsView {
  panel.setModules({
    files: incoming.panelFiles !== false,
    terminal: incoming.panelTerminal !== false,
    browser: incoming.panelBrowser !== false,
  });
  if (incoming.panelScope) {
    panel.setScope(incoming.panelScope);
    return incoming;
  }
  return { ...incoming, panelScope: panelState().scope };
}

// Root of the real webview: consumes the whole state pushed by the host, posts actions back via postMessage unchanged.
// The settings page is a local view swap over the chat (Codex-style), not a separate webview
export function App() {
  // Only the environment fields of `init` are kept: holding the whole message would pin its `active` session view (and
  // every other list in it) for the page's lifetime, after the session has been switched away and the lists replaced
  const [init, setInit] = useState<InitEnv>();
  const [appearance, setAppearance] = useState<Appearance>(BASE_APPEARANCE);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [categories, setCategories] = useState<SessionCategories>();
  const [accounts, setAccounts] = useState<AccountInfo[]>([]);
  const [accountActions, setAccountActions] = useState<AccountAction[]>([]);
  // In-app agent installs, host-owned: every panel shows the same run
  const [agentInstalls, setAgentInstalls] = useState<AgentInstallProgress[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [hidden, setHidden] = useState<HiddenMap>({});
  const [session, setSession] = useState<SessionView>();
  // A project shell (the desktop app) with no project to show a session of: init came without one, or noSession
  const [noProject, setNoProject] = useState(false);
  // The desktop shell's folder dialog under way, settled by pickedFolder (a newer request settles the older one empty)
  const folderPick = useRef<(path?: string) => void>(undefined);
  // Observed subagent transcripts keyed `${sessionId}:${subagentId}`; the inspector subscribes to one at a time
  const [subagentTranscripts, setSubagentTranscripts] = useState<Record<string, { rev: number; running: boolean; turns: Turn[] }>>({});
  // The session the view is showing right now, readable inside the stable handler object: every session action
  // carries it so a click rendered for one conversation can never be applied to another after a fast switch
  const activeId = useRef<string | undefined>(undefined);
  // A whole view was asked for after a session patch did not fit; cleared when one lands
  const resyncAsked = useRef(false);
  // The view on screen and the last view of each recent session, kept in step with the host's record of them
  const views = useRef(new SessionViews()).current;
  activeId.current = session?.id;
  // Where an editor range pinned by "Add to chat" is labeled relative to
  const cwdRef = useRef<string>('');
  const [view, setView] = useState<'chat' | 'settings'>('chat');
  // The view the last render showed: Shell, mounted fresh on the way back from settings, eases the desktop frame from there
  const lastView = useRef(view);
  const fromSettings = view === 'chat' && lastView.current === 'settings';
  useEffect(() => { lastView.current = view; }, [view]);
  const [settings, setSettings] = useState<SettingsView>();
  const [locale, setLoc] = useState<Locale>(bootLocale ?? 'en');
  // The first-run flow or the interface tour over the sample session (welcome/): which, and whether a replay has ended (the
  // real shell is back under the fading layer). The desktop's first launch gets the intro and the tour; VS Code's sidebar
  // only the folder card
  const [welcome, setWelcome] = useState<{ mode: WelcomeMode; ended: boolean } | undefined>(() =>
    tourPage ? { mode: 'replay', ended: false } : firstRun ? { mode: desktop ? 'first' : 'consent', ended: false } : undefined);
  const [detected, setDetected] = useState<DetectedAgent[]>();
  const [firstRunError, setFirstRunError] = useState<string>();
  const [chatgptStatus, setChatgptStatus] = useState<ChatGptIntegrationStatus>();
  const [inventories, setInventories] = useState<Partial<Record<AgentId, AgentInventory>>>({});
  const [shared, setShared] = useState<SharedState>();
  const [providers, setProviders] = useState<ProvidersState>();
  const [storage, setStorage] = useState<StorageState>();
  const [controls, setControls] = useState<Partial<Record<AgentId, ConfigControl[]>>>({});
  // Each agent's remembered per-model parameters, answered with its controls (the persona page's effort list per model)
  const [modelShapes, setModelShapes] = useState<Partial<Record<AgentId, ModelShapes>>>({});
  const controlsAsked = useRef(new Set<string>());
  // The import popover's listing; `agent` ties it to the request it answers so a stale reply can't overwrite a newer request
  const [nativeSessions, setNativeSessions] = useState<NativeSessionsState>();
  const [page, setPage] = useState<SettingsPage>({ kind: 'general' });
  // Agents whose refresh button is waiting on the probe; their cached inventory / controls stay on screen until the reply replaces them
  const [refreshing, setRefreshing] = useState<ReadonlySet<AgentId>>(() => new Set());
  const refreshingRef = useRef(refreshing);
  refreshingRef.current = refreshing;
  // The agents list as last received, for spotting availability flips inside the message handler
  const lastAgents = useRef<AgentInfo[]>([]);
  const hostTheme = useVsCodeTheme();
  // The theme setting resolved against the host; the document carries it too so color-scheme reaches native controls outside the shell
  // A skin fixes the scheme it was made for (its base); the setting is back once it goes
  const skinBase = useSkin().shown?.base;
  const theme = skinBase ?? resolveTheme(settings?.theme ?? 'auto', hostTheme).theme;
  const look = useMemo(() => settings && lookFromSettings(settings), [settings]);
  // Settings → Orchestration, the enabled subagents as the composer offers them (name, CLI logo, CLI name and model), then
  // the enabled groups with an enabled member, their members' names in place of a CLI, joined with the language's list
  // separator (hence the locale among the deps)
  const subagents = settings?.subagents;
  const subagentGroups = settings?.subagentGroups;
  const personas = useMemo(() => {
    if (!subagents) return undefined;
    const on = subagents.filter(p => p.enabled);
    const groups = (subagentGroups ?? []).flatMap(g => {
      const names = g.members.flatMap(m => on.find(p => p.id === m)?.name ?? []);
      return g.enabled && names.length ? [{ id: g.id, name: g.name, agent: '', cli: names.join(t('common.listSep')), group: true }] : [];
    });
    // The model reads as its CLI names it once that CLI's cached controls arrive (as on the subagents page), the id until then
    const modelName = (agent: AgentId, id: string) =>
      controls[agent]?.find(c => c.category === 'model' || c.id === 'model')?.options.find(o => o.id === id)?.name ?? id;
    return [...on.map(p => ({
      id: p.id, name: p.name, agent: p.agent,
      cli: agents.find(a => a.id === p.agent)?.name ?? p.agent, model: p.model ? modelName(p.agent, p.model) : undefined,
      effort: p.effort ? effortLabel({ id: p.effort, name: p.effort }) : undefined,
    })), ...groups];
  }, [subagents, subagentGroups, agents, controls, locale]);

  useEffect(() => { document.documentElement.lang = locale; }, [locale]);
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);

  // VS Code passes this webview state to the panel serializer after a window reload. Keep the active editor session
  // alongside the other local UI preferences so a revived tab attaches to the same conversation.
  useEffect(() => {
    if (!session?.id || !vscode.setState) return;
    const previous = vscode.getState?.();
    const state = { ...(previous && typeof previous === 'object' ? previous as Record<string, unknown> : {}), acpiraSessionId: session.id };
    if ((previous as { acpiraSessionId?: unknown } | undefined)?.acpiraSessionId !== session.id) vscode.setState(state);
  }, [session?.id]);

  useEffect(() => {
    // The tool panel's browser tabs and terminals live in the desktop app only
    if (window.__acpira?.shell === 'desktop') { installBrowserTabs(); startTerminals(); }
    const onMsg = (e: MessageEvent<HostMsg>) => {
      // A widget frame (show_widget) runs an agent's page: whatever it posts is never a host message
      if (isWidgetFrame(e.source)) return;
      // Host messages come from the page itself (the desktop shell's bridge), the webview's outer frame (VS Code) or a
      // dispatched event (no source); a child frame (a plugin piece, a thumbnail, a browser tab) never speaks as the host
      if (e.source && e.source !== window && e.source !== window.parent) return;
      const m = e.data;
      switch (m.type) {
        case 'editTurnResult': {
          const wait = editWaits.get(m.requestId);
          editWaits.delete(m.requestId);
          if (m.error) wait?.reject(new Error(m.error)); else wait?.resolve();
          break;
        }
        // A sidecar that came back answers nothing it was asked before: pending controls requests go out again
        case 'init': controlsAsked.current.clear(); setInit({ host: m.state.host, home: m.state.home, cwd: m.state.cwd, blobBase: m.state.blobBase, chats: m.state.chats }); setAppearance(m.state.appearance); lastAgents.current = m.state.agents; setAgents(m.state.agents); setSessions(m.state.sessions); setCategories(m.state.categories); setAccounts(m.state.accounts); setAccountActions(m.state.accountActions ?? []); setAgentInstalls(m.state.agentInstalls ?? []); setHidden(m.state.hidden); setSession(views.init(m.state.active)); setNoProject(!m.state.active); setSettings(syncPanelScope(m.state.settings)); setLocale(m.state.locale); setLoc(m.state.locale); terminalsConnected(m.state.home); watchConnections(); watchRemoteAccess(); break;
        case 'appearance': setAppearance(m.appearance); break;
        // An agent whose executable appeared or vanished has a stale inventory (binary path, version); drop it so the page rescans
        case 'agents': {
          const flipped = m.agents.filter(a => lastAgents.current.find(p => p.id === a.id)?.available !== a.available).map(a => a.id);
          lastAgents.current = m.agents;
          setAgents(m.agents);
          if (flipped.length) setInventories(inv => { const next = { ...inv }; for (const id of flipped) delete next[id]; return next; });
          break;
        }
        // Unchanged entries keep their references, so the list's memoized rows skip the ones a push did not touch
        case 'sessions': setSessions(prev => reuseById(prev, m.sessions)); break;
        case 'categories': setCategories(m.categories); break;
        case 'accounts': setAccounts(m.accounts); break;
        case 'accountActions': setAccountActions(m.actions); break;
        case 'agentInstalls': setAgentInstalls(m.installs); break;
        case 'hidden': setHidden(m.hidden); break;
        // Keep unchanged turns / blocks by reference so memoized history skips re-rendering during streaming
        case 'session': resyncAsked.current = false; setNoProject(false); setSession(views.whole(m.session)); break;
        case 'noSession': setNoProject(true); setSession(undefined); break;
        case 'pickedFolder': { const settle = folderPick.current; folderPick.current = undefined; settle?.(m.path); break; }
        case 'toast': pushShellToast(m); break;
        case 'detectedAgents': setDetected(m.agents); break;
        case 'firstRunError': setFirstRunError(m.error); break;
        // The skin and its reveal (plugins/skin.ts); the plugins view, which in-chat plugin cards read their state from
        case 'pluginTheme': receivePluginTheme(m); break;
        // An action from the plugin page that failed (a dynamic check, a theme not in effect) says why
        case 'plugins': setPluginsView(m.view); if (m.error) pushShellToast({ type: 'toast', level: 'error', text: m.error }); break;
        case 'pluginBackdrop': receivePluginBackdrop(m); break;
        // Plugins' interface pieces and their frames' traffic (pluginSlots.ts); the tool panel closes tabs of pieces gone
        case 'pluginUi': setPluginSlots(m.slots); panel.syncPlugins(); break;
        case 'pluginCalled': deliverPluginCalled(m); break;
        case 'pluginPost': deliverPluginPost(m); break;
        case 'updateState': setUpdateState(m.state); break;
        case 'desktopMenu': setDesktopMenu(m.menus); break;
        case 'windowState': setWindowMaximized(m.maximized); break;
        // The menu bar: settings is this page's switch, the rest are the chat's shortcuts (queued while settings is up)
        case 'shellCommand':
          if (m.command === 'settings') { setView('settings'); break; }
          if (m.command === 'about') { setPage({ kind: 'about' }); setView('settings'); break; }
          // The View menu's layout picks: written like the gear's, the page they reach stays up
          if (m.command === 'layoutChat' || m.command === 'layoutFiles') {
            if (m.command === 'layoutFiles') {
              panel.setFiles(true);
              setSettings(s => (s ? { ...s, panelFiles: true, desktopLayout: 'files' } : s));
              post({ type: 'setSetting', key: 'panelFiles', value: true });
            }
            post({ type: 'setSetting', key: 'desktopLayout', value: m.command === 'layoutFiles' ? 'files' : 'chat' });
            break;
          }
          setView('chat');
          runShellCommand(m.command);
          break;
        // Only the changed tail of the transcript comes over; kept turns stay the same objects, so reuse skips them at once.
        // Switching back to a recently shown session is a patch against the view kept from then (sessionViews.ts).
        // A patch that does not fit the held view asks for the whole view (once, until it lands); a stale one is dropped
        case 'sessionPatch': {
          const next = views.patch(m.patch);
          if (next === 'stale') break;
          if (next) { setSession(next); break; }
          if (!resyncAsked.current) { resyncAsked.current = true; queueMicrotask(() => post({ type: 'resync' })); }
          break;
        }
        case 'subagent': {
          const key = `${m.sessionId}:${m.subagentId}`;
          setSubagentTranscripts(current => {
            const prev = current[key];
            if (prev !== undefined && m.rev <= prev.rev) return current;
            return { ...current, [key]: { rev: m.rev, running: m.running, turns: prev !== undefined ? reuse(prev.turns, m.turns) : m.turns } };
          });
          break;
        }
        case 'settings': setSettings(syncPanelScope(m.settings)); setLocale(m.locale); setLoc(m.locale); break;
        case 'inventory': setInventories(inv => ({ ...inv, [m.agent]: m.inventory })); break;
        case 'shared': setShared({ view: m.view, error: m.error }); break;
        // A reply without the review lists (a plain refresh) keeps the ones already shown: a review always re-asks on open
        case 'storage': setStorage(s => ({ outcome: m.outcome, error: m.error, view: { ...m.view, sessions: m.view.sessions ?? s?.view.sessions, residue: m.view.residue ?? s?.view.residue } })); break;
        case 'providers': setProviders(s => ({ view: m.view, error: m.error, probes: s?.probes })); break;
        case 'providerProbed': setProviders(s => s && ({ ...s, probes: { ...s.probes, [m.id]: m.outcome } })); break;
        case 'controls':
          controlsAsked.current.delete(m.agent);
          setControls(c => ({ ...c, [m.agent]: m.controls }));
          setModelShapes(s => ({ ...s, [m.agent]: m.shapes }));
          setRefreshing(r => { if (!r.has(m.agent)) return r; const next = new Set(r); next.delete(m.agent); return next; });
          break;
        // A reply for an agent the popover has since moved away from is dropped; the effect re-requested the new one already
        case 'nativeSessions': setNativeSessions(cur => cur?.agent === m.agent ? { agent: m.agent, sessions: m.sessions, error: m.error, loading: false } : cur); break;
        case 'chatgptStatus': setChatgptStatus(m.status); break;
        case 'files': settleFiles(m.seq, m.files); break;
        case 'sessionHits': settleSessions(m.seq, m.hits); break;
        case 'diffSource': settleDiff(m.seq, m.source); break;
        case 'imageBlob': settleImageBlob(m.seq, m.blob); break;
        case 'projectDir': case 'projectStatus': case 'projectFile': case 'sessionChanges': settleProject(m); break;
        case 'terminalOutput': case 'terminalState': deliverTerminal(m); break;
        // SSH hosts: the pushed list, the ~/.ssh/config hosts and the remote folder picker's listings (remote/api.ts)
        case 'connections': case 'sshHosts': case 'remoteDir': deliverRemote(m); break;
        // Paired devices: the pushed view (remote/access.ts)
        case 'remoteAccess': deliverRemoteAccess(m); break;
        // The desktop tool panel (packages/ui/panel): its browser tabs' messages, the new-tab page's history, open_browser
        case 'panelBrowser': deliverToBrowserTab(m.tab, m.msg); break;
        case 'panelHistory': setRecentPages({ pages: m.pages, running: m.running }); break;
        case 'panelOpenBrowser':
          setView('chat');
          if (!panelState().browser) { if (m.url) post({ type: 'openExternal', url: m.url }); break; }
          if (!(m.tab && panel.showBrowser(m.tab, m.url))) panel.openBrowser(m.url);
          break;
        case 'sessionMarkdown': settleMarkdown(m.seq, m.markdown, m.error); break;
        case 'editorSelection': setEditorSelection(m.selection); break;
        case 'editorCopy': setEditorCopy(m.selection); break;
        case 'contextSelection': setContextSelection(m.source, m.items); break;
        case 'addContext': {
          const pinned = contextDrafts(m.source, m.items);
          setView('chat');
          // Pinning the same context again replaces it in place, so a second pick of an element with a comment (or without one)
          // takes over the earlier chip instead of being dropped as a duplicate
          updateMainComposer(d => [
            ...d.map(y => pinned.find(x => sameContext(x, y)) ?? y),
            ...pinned.filter(x => !d.some(y => sameContext(x, y))),
          ], true);
          break;
        }
        case 'chime': playChime(); break;
        case 'probeUrl': { const { id, url } = m; void probeUrl(url).then(r => post({ type: 'probeResult', id, ...r })); break; }
        case 'showSession': {
          const kept = views.select(m.id);
          if (kept) { setNoProject(false); setSession(kept); }
          post({ type: 'selectSession', id: m.id });
          setView('chat');
          break;
        }
        case 'addSelection': {
          const draft = selectionDraft(m.selection, cwdRef.current);
          setView('chat');
          updateMainComposer(d => (d.some(x => sameRange(x, draft)) ? d : [...d, draft]), true);
          break;
        }
      }
    };
    window.addEventListener('message', onMsg);
    post({ type: 'ready', patches: true });
    // The intro's agent row: the shell looks on PATH itself, there being no engine yet
    if (firstRun && desktop) post({ type: 'detectAgents' });
    // The shell routes "Add to Chat" to the chat used last; a click into an already visible view changes no visibility it could see
    const onFocus = () => post({ type: 'viewFocus' });
    window.addEventListener('focus', onFocus);
    return () => { window.removeEventListener('message', onMsg); window.removeEventListener('focus', onFocus); };
  }, []);

  // Observed transcripts belong to the session they streamed from
  const sessionId = session?.id;
  useEffect(() => setSubagentTranscripts({}), [sessionId]);

  // The model lists of an agent page come from the configOptions of its latest session; ask for them on first visit.
  // The personas' CLIs are asked for wherever the view is: the subagents page picks their models, and the composer's
  // summon menu names them (a cached read in the engine, no CLI is started)
  // Requests in flight are not sent twice while another agent's reply re-runs the effect
  const personaAgents = useMemo(() => [...new Set(settings?.subagents.map(p => p.agent))].sort().join('\n'), [settings?.subagents]);
  useEffect(() => {
    const wanted = personaAgents ? personaAgents.split('\n') : [];
    if (view === 'settings' && page.kind === 'agent') wanted.push(page.id);
    for (const agent of wanted) {
      if (controls[agent] !== undefined || controlsAsked.current.has(agent)) continue;
      controlsAsked.current.add(agent);
      post({ type: 'controls', agent });
    }
  }, [view, page, controls, personaAgents]);

  useEffect(() => {
    if (view !== 'settings' || page.kind !== 'chatgpt') return;
    const refresh = () => { if (!document.hidden) post({ type: 'chatgptStatus' }); };
    refresh();
    const timer = setInterval(refresh, 10_000);
    document.addEventListener('visibilitychange', refresh);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', refresh); };
  }, [view, page.kind]);

  const on = useMemo<ShellHandlers>(() => ({
    editTurn,
    send: (text, attachments, steer) => post({ type: 'send', sessionId: activeId.current, text, ...(attachments.length ? { attachments } : {}), ...(steer ? { steer } : {}) }),
    searchFiles,
    searchSessions,
    diffSource,
    stop: () => post({ type: 'stop', sessionId: activeId.current }),
    permission: (sessionId, blockId, optionId) => post({ type: 'permission', sessionId, blockId, optionId }),
    answer: (sessionId, blockId, answers, skip) => post({ type: 'answer', sessionId, blockId, answers, ...(skip ? { skip } : {}) }),
    buildPlan: (sessionId, planId, model, optionId) => post({ type: 'buildPlan', sessionId, planId, model, optionId }),
    openPlan: (sessionId, planId) => post({ type: 'openPlan', sessionId, planId }),
    openFile: (sessionId, path, line, reveal) => post({ type: 'openFile', sessionId, path, ...(line !== undefined ? { line } : {}), ...(reveal ? { reveal } : {}) }),
    imageBlob,
    openBlob: (sessionId, name) => post({ type: 'openBlob', sessionId, name }),
    setMode: id => post({ type: 'setMode', sessionId: activeId.current, id }),
    setConfig: (configId, value) => post({ type: 'setConfig', sessionId: activeId.current, configId, value }),
    selectSession: id => {
      const kept = views.select(id);
      if (kept) { setNoProject(false); setSession(kept); }
      post({ type: 'selectSession', id });
    },
    newSession: (agent, category, cwd, chat) => post({ type: 'newSession', ...(agent ? { agent } : {}), ...(category ? { category } : {}), ...(cwd ? { cwd } : {}), ...(chat ? { chat } : {}) }),
    renameSession: (id, title) => post({ type: 'renameSession', id, title }),
    deleteSession: id => post({ type: 'deleteSession', id }),
    restoreSession: id => post({ type: 'restoreSession', id }),
    pinSession: (id, pinned) => post({ type: 'pinSession', id, pinned }),
    moveSession: (id, to) => post({ type: 'moveSession', id, ...to }),
    setSessionCategory: (id, category) => post({ type: 'setSessionCategory', id, category }),
    categoryOp: (op, file) => post({ type: 'categoryOp', ...op, ...(file ? { file } : {}) }),
    markAllRead: () => post({ type: 'markAllRead' }),
    selectAccount: id => post({ type: 'selectAccount', sessionId: activeId.current, id }),
    addAccount: (agent, via) => post({ type: 'addAccount', agent, via }),
    removeAccount: id => post({ type: 'removeAccount', id }),
    refreshQuota: agent => post({ type: 'refreshQuota', agent }),
    unlockCredentials: agent => post({ type: 'unlockCredentials', agent }),
    compact: () => post({ type: 'compact', sessionId: activeId.current }),
    login: methodId => post({ type: 'login', sessionId: activeId.current, methodId }),
    retry: () => post({ type: 'retry', sessionId: activeId.current }),
    retryTurn: () => post({ type: 'retryTurn', sessionId: activeId.current }),
    takeOver: () => post({ type: 'takeOverSession', sessionId: activeId.current }),
    dequeue: (sessionId, id) => post({ type: 'dequeue', sessionId, id }),
    sendQueued: (sessionId, id) => post({ type: 'sendQueued', sessionId, id }),
    steerQueued: (sessionId, id) => post({ type: 'steerQueued', sessionId, id }),
    editQueued: (sessionId, id, text, retainedAttachments, attachments) => post({ type: 'editQueued', sessionId, id, text, retainedAttachments, attachments }),
    forkSession: (sessionId, turnIndex, agent) => post({ type: 'forkSession', sessionId, turnIndex, ...(agent ? { agent } : {}) }),
    exportSession: (id, format) => post({ type: 'exportSession', id, format }),
    sessionMarkdown,
    openInEditor: sessionId => post({ type: 'openInEditor', sessionId }),
    listNativeSessions: agent => { setNativeSessions({ agent, loading: true, sessions: [] }); post({ type: 'listNativeSessions', agent }); },
    importNativeSession: (agent, s) => post({ type: 'importNativeSession', agent, sessionId: s.sessionId, cwd: s.cwd, title: s.title, updatedAt: s.updatedAt }),
    observeSubagent: (sessionId, subagentId) => post({ type: 'observeSubagent', sessionId, subagentId }),
    unobserveSubagent: (sessionId, subagentId) => post({ type: 'unobserveSubagent', sessionId, subagentId }),
    cancelSubagent: (sessionId, subagentId) => post({ type: 'cancelSubagent', sessionId, subagentId }),
    stopAsyncTask: (sessionId, taskId) => post({ type: 'stopAsyncTask', sessionId, taskId }),
    goal: (sessionId, action, objective) => post({ type: 'goal', sessionId, action, ...(objective !== undefined ? { objective } : {}) }),
    // A directory's openPath reveals it in the OS file manager (every shell has revealInOS)
    revealPath: path => post({ type: 'openPath', path }),
    openExternal: url => post({ type: 'openExternal', url }),
    pickFolder: desktop ? () => new Promise<string | undefined>(resolve => {
      folderPick.current?.(undefined);
      folderPick.current = resolve;
      post({ type: 'pickFolder' });
    }) : undefined,
  }), []);

  const settingsOn = useMemo<SettingsHandlers>(() => ({
    refreshChatgpt: () => post({ type: 'chatgptStatus' }),
    connectChatgpt: () => { post({ type: 'connectChatgpt' }); setView('chat'); },
    openChatgpt: id => { post({ type: 'selectSession', id }); setView('chat'); },
    setSetting: (key, value) => {
      if (key === 'panelScope') panel.setScope(value as PanelScope);
      if (key === 'panelFiles') panel.setModules({ files: value as boolean });
      if (key === 'panelTerminal') panel.setModules({ terminal: value as boolean });
      if (key === 'panelBrowser') panel.setModules({ browser: value as boolean });
      setSettings(s => (s ? { ...s, [key]: value } : s));
      post({ type: 'setSetting', key, value });
    },
    saveSubagents: (base, personas) => post({ type: 'setSetting', key: 'subagents', value: { base, personas } }),
    saveSubagentGroups: (base, groups) => post({ type: 'setSetting', key: 'subagents', value: { baseGroups: base, groups } }),
    setAppearance: (axis, value) => post({ type: 'setAppearance', axis, value }),
    openPath: path => post({ type: 'openPath', path }),
    // Drop the cached copy first so the page shows the scanning shimmer until the reply lands
    refreshInventory: agent => { setInventories(inv => { const { [agent]: _drop, ...rest } = inv; return rest; }); post({ type: 'inventory', agent }); },
    // The refresh button keeps the cached page visible: the file scan answers first, the probe process (a cold CLI start) replaces
    // controls and then inventory again with the live version; quotas refresh alongside. A second click while probing is ignored
    refreshAgent: agent => {
      if (refreshingRef.current.has(agent)) return;
      setRefreshing(r => new Set(r).add(agent));
      post({ type: 'inventory', agent });
      post({ type: 'controls', agent, fresh: true });
      post({ type: 'refreshQuota', agent });
    },
    selectAccount: id => post({ type: 'selectAccount', id }),
    addAccount: agent => post({ type: 'addAccount', agent, via: 'auto' }),
    removeAccount: id => post({ type: 'removeAccount', id }),
    refreshQuota: agent => post({ type: 'refreshQuota', agent }),
    unlockCredentials: agent => post({ type: 'unlockCredentials', agent }),
    installAgent: agent => post({ type: 'installAgent', agent }),
    cancelInstall: agent => post({ type: 'cancelInstall', agent }),
    installInTerminal: agent => post({ type: 'installInTerminal', agent }),
    openExternal: url => post({ type: 'openExternal', url }),
    shared: () => post({ type: 'shared' }),
    sharedAction: action => post({ type: 'sharedAction', action }),
    storage: detail => post({ type: 'storage', detail }),
    storageAction: (action, detail) => post({ type: 'storageAction', action, detail }),
    providers: () => post({ type: 'providers' }),
    providerAction: action => post({ type: 'providerAction', action }),
    providerProbe: (id, probe) => post({ type: 'providerProbe', id, probe }),
  }), []);

  const openSettings = useCallback((target?: SettingsPage) => {
    if (target) setPage(target);
    setView('settings');
  }, []);

  // Settings' interface tour: the desktop plays it here over the sample session and comes back to the chat; VS Code opens
  // a tab of its own for it (opensTour), the sidebar being too narrow
  const startTour = useCallback(() => {
    if (!desktop) { post({ type: 'openTour' }); return; }
    setView('chat');
    setWelcome({ mode: 'replay', ended: false });
  }, []);

  // Desktop: the gear menu's theme, language, layout and panel-scope switches write the same settings the settings page does
  const quickTheme = settings?.theme, quickLanguage = settings?.language, quickLayout = settings?.desktopLayout, quickScope = settings?.panelScope;
  const quickFiles = settings?.panelFiles, quickTerminal = settings?.panelTerminal, quickBrowser = settings?.panelBrowser;
  const quickSettings = useMemo<QuickSettings | undefined>(() => desktop && quickTheme && quickLanguage
    ? {
        theme: quickTheme,
        language: quickLanguage,
        layout: quickLayout ?? 'chat',
        panelScope: quickScope ?? 'project',
        panelFiles: quickFiles !== false,
        anyPanelModule: quickFiles !== false || quickTerminal !== false || quickBrowser !== false,
        set: (key, value) => {
          if (key === 'panelScope') panel.setScope(value as PanelScope);
          setSettings(s => (s ? { ...s, [key]: value } : s));
          post({ type: 'setSetting', key, value });
        },
      }
    : undefined, [quickTheme, quickLanguage, quickLayout, quickScope, quickFiles, quickTerminal, quickBrowser]);

  cwdRef.current = session?.cwd ?? init?.cwd ?? '';
  // The first-run flow / tour layer sits beside whatever shell is under it (the second child below, in every branch),
  // so it outlives the swap from the sample shell to the real one and fades out over the real one. The first run ends
  // when the engine's init is in; a replay when the user finishes or skips it
  const ready = !!init && !!settings;
  const layer = welcome && (
    <LocaleContext.Provider value={locale}>
      <Welcome mode={welcome.mode} scope={sampleScope} home={firstRun?.home ?? ''} agents={detected} error={firstRunError}
        closing={welcome.mode === 'replay' ? welcome.ended : ready} appearance={appearance} theme={theme} look={look}
        onCreate={() => { setFirstRunError(undefined); post({ type: 'createDataHome' }); }}
        onEnd={() => { setWelcome(w => w && { ...w, ended: true }); if (tourPage) post({ type: 'tourDone' }); }}
        onGone={() => setWelcome(undefined)} />
    </LocaleContext.Provider>
  );
  // VS Code's tour tab only ever shows the sample; elsewhere it gives way once a replay ended or the engine is in
  if (welcome && (tourPage || (welcome.mode === 'replay' ? !welcome.ended : !ready))) {
    const host = init?.host ?? (window.__acpira?.host === 'sidebar' ? 'sidebar' : 'editor');
    return <>
      <LocaleContext.Provider value={locale}><SampleShell appearance={appearance} theme={theme} settings={settings} host={host} desktop={desktop} locale={locale} /></LocaleContext.Provider>
      {layer}
    </>;
  }
  if (!init || !settings) return <>{null}{layer}</>;
  const agent = agents.find(a => a.id === session?.agent) ?? agents[0];
  if (!agent) return <>{null}{layer}</>;
  // A locale change re-renders in place: t() reads module state, and what memo or a cache would keep (rows, joined
  // names) reads the locale from LocaleContext. Remounting on key={locale} replayed every mount animation, closed open
  // menus and reset scroll positions, which read as the window blinking
  // The skin's backdrop, first in both branches below so React keeps it across the swap (plugins/SkinBackdrop.tsx)
  const backdrop = <SkinBackdrop paused={view === 'settings'} motion={appearance.motion} />;
  if (view === 'settings') {
    return <>
      {backdrop}
      <LocaleContext.Provider value={locale}>
      <SettingsShell
        appearance={appearance}
        theme={theme}
        look={look}
        host={init.host}
        locale={locale}
        settings={settings}
        agents={agents}
        accounts={accounts}
        installs={agentInstalls}
        chatgptStatus={chatgptStatus}
        inventories={inventories}
        shared={shared}
        providers={providers}
        storage={storage}
        projects={categories?.projects}
        controls={controls}
        modelShapes={modelShapes}
        refreshing={refreshing}
        env={{ home: init.home, cwd: session?.cwd ?? init.cwd }}
        page={page}
        onPage={setPage}
        onBack={() => setView('chat')}
        desktop={desktop}
        on={settingsOn}
        onTour={desktop || window.__acpira?.opensTour ? startTour : undefined}
      />
      </LocaleContext.Provider>
      {layer}
    </>;
  }
  return <>
    {backdrop}
    <LocaleContext.Provider value={locale}>
    <Shell
      fromSettings={fromSettings}
      quickSettings={quickSettings}
      appearance={appearance}
      theme={theme}
      look={look}
      host={init.host}
      agent={agent}
      agents={agents}
      accounts={accounts}
      accountId={session?.accountId}
      accountAction={accountActions.find(action => action.agent === agent.id)}
      hidden={hidden}
      title={session?.title ?? t('session.untitled')}
      status={session?.status ?? 'starting'}
      external={session?.external}
      error={session?.error}
      canTakeOver={session?.canTakeOver}
      authMethods={session?.authMethods}
      turns={session?.turns ?? EMPTY_TURNS}
      running={session?.running ?? false}
      queued={session?.queued}
      goal={session?.goal}
      goalActions={session?.goalActions}
      forkedFrom={session?.forkedFrom}
      controls={session?.controls ?? EMPTY_CONTROLS}
      modelShapes={session?.modelShapes}
      usage={session?.usage}
      commands={session?.commands}
      compactAt={settings.autoCompact ? settings.compactAtTokens : undefined}
      shareEditorSelection={settings.shareEditorSelection}
      steerQueued={settings.steerQueued && !!session?.canSteer}
      personas={personas}
      sessions={sessions}
      categories={categories}
      activeSessionId={session?.id}
      cwd={session?.cwd}
      workspace={init.cwd}
      sessionScope={settings.sessionScope}
      nativeSessions={nativeSessions}
      blobBase={init.blobBase}
      subagents={session?.subagents}
      subagentTranscripts={subagentTranscripts}
      on={on}
      replayKey={session?.id}
      catchingUp={!!session && views.catchingUp === session.id}
      onOpenSettings={openSettings}
      desktop={desktop}
      noProject={desktop && noProject}
      home={init.home}
      chats={init.chats}
    />
    </LocaleContext.Provider>
    {layer}
  </>;
}
