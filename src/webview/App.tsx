import type { ChatGptIntegrationStatus } from '@shared/chatgptIntegration';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { AccountAction, EditTurnRequest, FileHit, HostMsg, InitState, NativeSessionsState, SessionHit, WebviewMsg } from '@shared/protocol';
import type { AccountInfo, AgentId, AgentInfo, ConfigControl, SessionSummary, SessionView, Turn } from '@shared/transcript';
import type { HiddenMap, SettingsView } from '@shared/settings';
import type { AgentInventory } from '@shared/inventory';
import type { Locale } from '@shared/i18n';
import { applySession, reuse } from '@shared/reuse';
import { BASE_APPEARANCE, type Appearance } from './appearance';
import { LocaleContext, setLocale, t } from './i18n';
import { Shell, type ShellHandlers } from './chat/Shell';
import { lookFromSettings, resolveTheme } from './look';
import { useVsCodeTheme } from './useVsCodeTheme';
import { vscodeApi } from './vscodeApi';
import { SettingsShell, type SettingsHandlers } from './settings/SettingsShell';
import { sameRange, selectionDraft, setEditorCopy, setEditorSelection } from './chat/editorContext';
import { updateMainComposer } from './chat/useComposerDraft';
import type { SettingsPage } from './settings/Nav';
import type { SharedState } from './settings/SharedPage';

declare global {
  interface Window { __acpira?: { host: 'sidebar' | 'editor' } }
}

// Always through the memo: acquireVsCodeApi() throws when called twice, and other components (Link) reach the api via vscodeApi()
const vscode = vscodeApi();
const post = (m: WebviewMsg) => vscode.postMessage(m);

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

// Root of the real webview: consumes the whole state pushed by the host, posts actions back via postMessage unchanged.
// The settings page is a local view swap over the chat (Codex-style), not a separate webview
export function App() {
  const [init, setInit] = useState<InitState>();
  const [appearance, setAppearance] = useState<Appearance>(BASE_APPEARANCE);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [accounts, setAccounts] = useState<AccountInfo[]>([]);
  const [accountActions, setAccountActions] = useState<AccountAction[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [hidden, setHidden] = useState<HiddenMap>({});
  const [session, setSession] = useState<SessionView>();
  // Observed subagent transcripts keyed `${sessionId}:${subagentId}`; the inspector subscribes to one at a time
  const [subagentTranscripts, setSubagentTranscripts] = useState<Record<string, { rev: number; running: boolean; turns: Turn[] }>>({});
  // The session the view is showing right now, readable inside the stable handler object: every session action
  // carries it so a click rendered for one conversation can never be applied to another after a fast switch
  const activeId = useRef<string | undefined>(undefined);
  activeId.current = session?.id;
  // Where an editor range pinned by "Add to chat" is labeled relative to
  const cwdRef = useRef<string>('');
  const [view, setView] = useState<'chat' | 'settings'>('chat');
  const [settings, setSettings] = useState<SettingsView>();
  const [locale, setLoc] = useState<Locale>('en');
  const [chatgptStatus, setChatgptStatus] = useState<ChatGptIntegrationStatus>();
  const [inventories, setInventories] = useState<Partial<Record<AgentId, AgentInventory>>>({});
  const [shared, setShared] = useState<SharedState>();
  const [controls, setControls] = useState<Partial<Record<AgentId, ConfigControl[]>>>({});
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
  const { theme } = resolveTheme(settings?.theme ?? 'auto', hostTheme);
  const look = useMemo(() => settings && lookFromSettings(settings), [settings]);

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
    const onMsg = (e: MessageEvent<HostMsg>) => {
      const m = e.data;
      switch (m.type) {
        case 'editTurnResult': {
          const wait = editWaits.get(m.requestId);
          editWaits.delete(m.requestId);
          if (m.error) wait?.reject(new Error(m.error)); else wait?.resolve();
          break;
        }
        case 'init': setInit(m.state); setAppearance(m.state.appearance); lastAgents.current = m.state.agents; setAgents(m.state.agents); setSessions(m.state.sessions); setAccounts(m.state.accounts); setAccountActions(m.state.accountActions ?? []); setHidden(m.state.hidden); setSession(current => m.state.active ? applySession(current, m.state.active) : undefined); setSettings(m.state.settings); setLocale(m.state.locale); setLoc(m.state.locale); break;
        case 'appearance': setAppearance(m.appearance); break;
        // An agent whose executable appeared or vanished has a stale inventory (binary path, version); drop it so the page rescans
        case 'agents': {
          const flipped = m.agents.filter(a => lastAgents.current.find(p => p.id === a.id)?.available !== a.available).map(a => a.id);
          lastAgents.current = m.agents;
          setAgents(m.agents);
          if (flipped.length) setInventories(inv => { const next = { ...inv }; for (const id of flipped) delete next[id]; return next; });
          break;
        }
        case 'sessions': setSessions(m.sessions); break;
        case 'accounts': setAccounts(m.accounts); break;
        case 'accountActions': setAccountActions(m.actions); break;
        case 'hidden': setHidden(m.hidden); break;
        // Keep unchanged turns / blocks by reference so memoized history skips re-rendering during streaming
        case 'session': setSession(current => applySession(current, m.session)); break;
        case 'subagent': {
          const key = `${m.sessionId}:${m.subagentId}`;
          setSubagentTranscripts(current => {
            const prev = current[key];
            if (prev !== undefined && m.rev <= prev.rev) return current;
            return { ...current, [key]: { rev: m.rev, running: m.running, turns: prev !== undefined ? reuse(prev.turns, m.turns) : m.turns } };
          });
          break;
        }
        case 'settings': setSettings(m.settings); setLocale(m.locale); setLoc(m.locale); break;
        case 'inventory': setInventories(inv => ({ ...inv, [m.agent]: m.inventory })); break;
        case 'shared': setShared({ view: m.view, error: m.error }); break;
        case 'controls':
          setControls(c => ({ ...c, [m.agent]: m.controls }));
          setRefreshing(r => { if (!r.has(m.agent)) return r; const next = new Set(r); next.delete(m.agent); return next; });
          break;
        // A reply for an agent the popover has since moved away from is dropped; the effect re-requested the new one already
        case 'nativeSessions': setNativeSessions(cur => cur?.agent === m.agent ? { agent: m.agent, sessions: m.sessions, error: m.error, loading: false } : cur); break;
        case 'chatgptStatus': setChatgptStatus(m.status); break;
        case 'files': settleFiles(m.seq, m.files); break;
        case 'sessionHits': settleSessions(m.seq, m.hits); break;
        case 'editorSelection': setEditorSelection(m.selection); break;
        case 'editorCopy': setEditorCopy(m.selection); break;
        case 'addSelection': {
          const draft = selectionDraft(m.selection, cwdRef.current);
          setView('chat');
          updateMainComposer(d => (d.some(x => sameRange(x, draft)) ? d : [...d, draft]), true);
          break;
        }
      }
    };
    window.addEventListener('message', onMsg);
    post({ type: 'ready' });
    // The shell routes "Add to Chat" to the chat used last; a click into an already visible view changes no visibility it could see
    const onFocus = () => post({ type: 'viewFocus' });
    window.addEventListener('focus', onFocus);
    return () => { window.removeEventListener('message', onMsg); window.removeEventListener('focus', onFocus); };
  }, []);

  // Observed transcripts belong to the session they streamed from
  const sessionId = session?.id;
  useEffect(() => setSubagentTranscripts({}), [sessionId]);

  // The model lists of an agent page come from the configOptions of its latest session; ask for them on first visit
  useEffect(() => {
    if (view === 'settings' && page.kind === 'agent' && controls[page.id] === undefined) post({ type: 'controls', agent: page.id });
  }, [view, page, controls]);

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
    stop: () => post({ type: 'stop', sessionId: activeId.current }),
    permission: (sessionId, blockId, optionId) => post({ type: 'permission', sessionId, blockId, optionId }),
    answer: (sessionId, blockId, answers, skip) => post({ type: 'answer', sessionId, blockId, answers, ...(skip ? { skip } : {}) }),
    buildPlan: (sessionId, planId, model, optionId) => post({ type: 'buildPlan', sessionId, planId, model, optionId }),
    openPlan: (sessionId, planId) => post({ type: 'openPlan', sessionId, planId }),
    openFile: (sessionId, path, line) => post({ type: 'openFile', sessionId, path, line }),
    openBlob: (sessionId, name) => post({ type: 'openBlob', sessionId, name }),
    setMode: id => post({ type: 'setMode', sessionId: activeId.current, id }),
    setConfig: (configId, value) => post({ type: 'setConfig', sessionId: activeId.current, configId, value }),
    selectSession: id => post({ type: 'selectSession', id }),
    newSession: agent => post({ type: 'newSession', ...(agent ? { agent } : {}) }),
    renameSession: (id, title) => post({ type: 'renameSession', id, title }),
    deleteSession: id => post({ type: 'deleteSession', id }),
    restoreSession: id => post({ type: 'restoreSession', id }),
    pinSession: (id, pinned) => post({ type: 'pinSession', id, pinned }),
    moveSession: id => post({ type: 'moveSession', id }),
    selectAccount: id => post({ type: 'selectAccount', sessionId: activeId.current, id }),
    addAccount: (agent, via) => post({ type: 'addAccount', agent, via }),
    removeAccount: id => post({ type: 'removeAccount', id }),
    refreshQuota: agent => post({ type: 'refreshQuota', agent }),
    unlockCredentials: agent => post({ type: 'unlockCredentials', agent }),
    compact: () => post({ type: 'compact', sessionId: activeId.current }),
    login: methodId => post({ type: 'login', sessionId: activeId.current, methodId }),
    retry: () => post({ type: 'retry', sessionId: activeId.current }),
    retryTurn: () => post({ type: 'retryTurn', sessionId: activeId.current }),
    reconnect: () => post({ type: 'reconnect', sessionId: activeId.current }),
    takeOver: () => post({ type: 'takeOverSession', sessionId: activeId.current }),
    dequeue: (sessionId, id) => post({ type: 'dequeue', sessionId, id }),
    sendQueued: (sessionId, id) => post({ type: 'sendQueued', sessionId, id }),
    steerQueued: (sessionId, id) => post({ type: 'steerQueued', sessionId, id }),
    editQueued: (sessionId, id, text, retainedAttachments, attachments) => post({ type: 'editQueued', sessionId, id, text, retainedAttachments, attachments }),
    forkSession: (sessionId, turnIndex) => post({ type: 'forkSession', sessionId, turnIndex }),
    exportSession: (id, format) => post({ type: 'exportSession', id, format }),
    openInEditor: sessionId => post({ type: 'openInEditor', sessionId }),
    listNativeSessions: agent => { setNativeSessions({ agent, loading: true, sessions: [] }); post({ type: 'listNativeSessions', agent }); },
    importNativeSession: (agent, s) => post({ type: 'importNativeSession', agent, sessionId: s.sessionId, cwd: s.cwd, title: s.title, updatedAt: s.updatedAt }),
    observeSubagent: (sessionId, subagentId) => post({ type: 'observeSubagent', sessionId, subagentId }),
    unobserveSubagent: (sessionId, subagentId) => post({ type: 'unobserveSubagent', sessionId, subagentId }),
    cancelSubagent: (sessionId, subagentId) => post({ type: 'cancelSubagent', sessionId, subagentId }),
    stopAsyncTask: (sessionId, taskId) => post({ type: 'stopAsyncTask', sessionId, taskId }),
  }), []);

  const settingsOn = useMemo<SettingsHandlers>(() => ({
    refreshChatgpt: () => post({ type: 'chatgptStatus' }),
    connectChatgpt: () => { post({ type: 'connectChatgpt' }); setView('chat'); },
    openChatgpt: id => { post({ type: 'selectSession', id }); setView('chat'); },
    setSetting: (key, value) => post({ type: 'setSetting', key, value }),
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
    openExternal: url => post({ type: 'openExternal', url }),
    shared: () => post({ type: 'shared' }),
    sharedAction: action => post({ type: 'sharedAction', action }),
  }), []);

  cwdRef.current = session?.cwd ?? init?.cwd ?? '';
  if (!init || !settings) return null;
  const agent = agents.find(a => a.id === session?.agent) ?? agents[0];
  if (!agent) return null;
  // A locale change re-renders through a fresh dictionary: t() reads module state, so the tree remounts on key
  if (view === 'settings') {
    return (
      <SettingsShell
        key={locale}
        appearance={appearance}
        theme={theme}
        look={look}
        host={init.host}
        locale={locale}
        settings={settings}
        agents={agents}
        accounts={accounts}
        chatgptStatus={chatgptStatus}
        inventories={inventories}
        shared={shared}
        controls={controls}
        refreshing={refreshing}
        env={{ home: init.home, cwd: session?.cwd ?? init.cwd }}
        page={page}
        onPage={setPage}
        onBack={() => setView('chat')}
        on={settingsOn}
      />
    );
  }
  return (
    <LocaleContext.Provider value={locale}>
    <Shell
      key={locale}
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
      turns={session?.turns ?? []}
      running={session?.running ?? false}
      queued={session?.queued}
      controls={session?.controls ?? { modes: [], options: [] }}
      modelShapes={session?.modelShapes}
      usage={session?.usage}
      commands={session?.commands}
      compactAt={settings.autoCompact ? settings.compactAtTokens : undefined}
      shareEditorSelection={settings.shareEditorSelection}
      steerQueued={settings.steerQueued && !!session?.canSteer}
      sessions={sessions}
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
      onOpenSettings={() => setView('settings')}
    />
    </LocaleContext.Provider>
  );
}
