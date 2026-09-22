import type { ChatGptIntegrationStatus } from '@shared/chatgptIntegration';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { AccountAction, EditTurnRequest, FileHit, HostMsg, InitState, NativeSessionsState, WebviewMsg } from '@shared/protocol';
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
import type { SettingsPage } from './settings/Nav';

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

// The one request/response pair over postMessage: file search for @ mentions. Each request gets a seq; the matching `files` reply resolves it.
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
  const [view, setView] = useState<'chat' | 'settings'>('chat');
  const [settings, setSettings] = useState<SettingsView>();
  const [locale, setLoc] = useState<Locale>('en');
  const [chatgptStatus, setChatgptStatus] = useState<ChatGptIntegrationStatus>();
  const [inventories, setInventories] = useState<Partial<Record<AgentId, AgentInventory>>>({});
  const [controls, setControls] = useState<Partial<Record<AgentId, ConfigControl[]>>>({});
  // The import popover's listing; `agent` ties it to the request it answers so a stale reply can't overwrite a newer request
  const [nativeSessions, setNativeSessions] = useState<NativeSessionsState>();
  const [page, setPage] = useState<SettingsPage>({ kind: 'general' });
  // Agents whose next controls request is a refresh: the flag rides the effect's request so a re-render can't double-spawn the probe
  const freshControls = useRef(new Set<AgentId>());
  // The agents list as last received, for spotting availability flips inside the message handler
  const lastAgents = useRef<AgentInfo[]>([]);
  const hostTheme = useVsCodeTheme();
  // The theme setting resolved against the host; the document carries it too so color-scheme reaches native controls outside the shell
  const { theme } = resolveTheme(settings?.theme ?? 'auto', hostTheme);
  const look = useMemo(() => settings && lookFromSettings(settings), [settings]);

  useEffect(() => { document.documentElement.lang = locale; }, [locale]);
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);

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
        case 'controls': setControls(c => ({ ...c, [m.agent]: m.controls })); break;
        // A reply for an agent the popover has since moved away from is dropped; the effect re-requested the new one already
        case 'nativeSessions': setNativeSessions(cur => cur?.agent === m.agent ? { agent: m.agent, sessions: m.sessions, error: m.error, loading: false } : cur); break;
        case 'chatgptStatus': setChatgptStatus(m.status); break;
        case 'files': settleFiles(m.seq, m.files); break;
      }
    };
    window.addEventListener('message', onMsg);
    post({ type: 'ready' });
    return () => window.removeEventListener('message', onMsg);
  }, []);

  // Observed transcripts belong to the session they streamed from
  const sessionId = session?.id;
  useEffect(() => setSubagentTranscripts({}), [sessionId]);

  // The model lists of an agent page come from the configOptions of its latest session; ask for them on first visit
  useEffect(() => {
    if (view === 'settings' && page.kind === 'agent' && controls[page.id] === undefined) post({ type: 'controls', agent: page.id, fresh: freshControls.current.delete(page.id) || undefined });
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
    send: (text, attachments) => post({ type: 'send', sessionId: activeId.current, text, ...(attachments.length ? { attachments } : {}) }),
    searchFiles,
    stop: () => post({ type: 'stop', sessionId: activeId.current }),
    permission: (sessionId, blockId, optionId) => post({ type: 'permission', sessionId, blockId, optionId }),
    answer: (sessionId, blockId, answers, skip) => post({ type: 'answer', sessionId, blockId, answers, ...(skip ? { skip } : {}) }),
    buildPlan: (sessionId, planId, model, optionId) => post({ type: 'buildPlan', sessionId, planId, model, optionId }),
    openPlan: (sessionId, planId) => post({ type: 'openPlan', sessionId, planId }),
    openFile: (sessionId, path, line) => post({ type: 'openFile', sessionId, path, line }),
    setMode: id => post({ type: 'setMode', sessionId: activeId.current, id }),
    setConfig: (configId, value) => post({ type: 'setConfig', sessionId: activeId.current, configId, value }),
    selectAgent: id => post({ type: 'selectAgent', id }),
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
    compact: () => post({ type: 'compact', sessionId: activeId.current }),
    login: methodId => post({ type: 'login', sessionId: activeId.current, methodId }),
    retry: () => post({ type: 'retry', sessionId: activeId.current }),
    retryTurn: () => post({ type: 'retryTurn', sessionId: activeId.current }),
    reconnect: () => post({ type: 'reconnect', sessionId: activeId.current }),
    dequeue: (sessionId, id) => post({ type: 'dequeue', sessionId, id }),
    sendQueued: (sessionId, id) => post({ type: 'sendQueued', sessionId, id }),
    editQueued: (sessionId, id, text, retainedAttachments, attachments) => post({ type: 'editQueued', sessionId, id, text, retainedAttachments, attachments }),
    forkSession: (sessionId, turnIndex) => post({ type: 'forkSession', sessionId, turnIndex }),
    exportSession: (id, format) => post({ type: 'exportSession', id, format }),
    openInEditor: sessionId => post({ type: 'openInEditor', sessionId }),
    listNativeSessions: agent => { setNativeSessions({ agent, loading: true, sessions: [] }); post({ type: 'listNativeSessions', agent }); },
    importNativeSession: (agent, s) => post({ type: 'importNativeSession', agent, sessionId: s.sessionId, cwd: s.cwd, title: s.title, updatedAt: s.updatedAt }),
    observeSubagent: (sessionId, subagentId) => post({ type: 'observeSubagent', sessionId, subagentId }),
    unobserveSubagent: (sessionId, subagentId) => post({ type: 'unobserveSubagent', sessionId, subagentId }),
    cancelSubagent: (sessionId, subagentId) => post({ type: 'cancelSubagent', sessionId, subagentId }),
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
    // The refresh button: only drop the caches — the controls effect re-requests with fresh (probe process), the inventory one as usual
    refreshAgent: agent => {
      freshControls.current.add(agent);
      setControls(c => { const { [agent]: _drop, ...rest } = c; return rest; });
      setInventories(inv => { const { [agent]: _drop, ...rest } = inv; return rest; });
    },
    selectAccount: id => post({ type: 'selectAccount', id }),
    addAccount: agent => post({ type: 'addAccount', agent, via: 'auto' }),
    removeAccount: id => post({ type: 'removeAccount', id }),
    refreshQuota: agent => post({ type: 'refreshQuota', agent }),
    installAgent: agent => post({ type: 'installAgent', agent }),
    openExternal: url => post({ type: 'openExternal', url }),
  }), []);

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
        controls={controls}
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
      authMethods={session?.authMethods}
      turns={session?.turns ?? []}
      running={session?.running ?? false}
      queued={session?.queued}
      controls={session?.controls ?? { modes: [], options: [] }}
      usage={session?.usage}
      commands={session?.commands}
      compactAt={settings.autoCompact ? settings.compactAtTokens : undefined}
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
