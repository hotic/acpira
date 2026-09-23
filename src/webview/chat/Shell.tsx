import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Network, Paperclip, X } from 'lucide-react';
import type { ExternalSessionInfo, AccountInfo, AgentInfo, AuthMethodInfo, Draft, NativeSessionInfo, PermissionBlock, QuestionAnswers, QuestionBlock, QueuedPrompt, SessionControls, SessionStatus, SessionSummary, SlashCommand, Turn, Usage } from '@shared/transcript';
import type { SubagentSummary } from '@shared/subagents';
import type { HiddenMap, SessionScope } from '@shared/settings';
import type { AccountAction, AddAccountVia, EditTurnRequest, FileHit, NativeSessionsState } from '@shared/protocol';
import { AppearanceContext, appearanceDataAttrs, type Appearance } from '../appearance';
import { lookAttrs, ThemeContext, type ShellLook, type Theme } from '../look';
import { t } from '../i18n';
import { ShellLayerContext } from '../ui/Popover';
import { cn } from '../ui/cn';
import { Chip, IconButton } from '../ui/Button';
import { useScrollReveal } from '../ui/useScrollReveal';
import { useStableList } from '../ui/useStableList';
import { scrollerUsable } from './promptStuck';
import { Header } from './Header';
import { SessionList } from './SessionList';
import { AgentMessage } from './Turns';
import { HistoryComposerContext, HistoryContext, HistoryMessage } from './HistoryMessage';
import { Composer, type ComposerProps } from './Composer';
import { Notice } from './Notice';
import { ExternalSessionNotice } from './ExternalSessionNotice';
import { Toast } from '../ui/Toast';
import { Alert, isShortStop } from './Alert';
import { Questions, type OnAnswer } from './Questions';
import { PlanBar } from './PlanBar';
import { PlanDocumentContext } from './PlanDocument';
import { planExecutionId } from '@shared/planExecution';
import { Queue } from './Queue';
import { OpenToolFileContext, BlobUrlContext, OpenBlobContext } from './fileLinks';
import { TurnActionsContext } from './TurnActions';
import { SubagentInspector } from './subagents/SubagentInspector';
import { SubagentGraph } from './subagents/SubagentGraph';
import { breadcrumb, nodesByTurn, subagentTitle } from './subagents/subagentState';

// Every action the webview sends to the host; in the LAB a fake host implements these, the real build swaps in postMessage
export interface ShellHandlers {
  editTurn?: (edit: EditTurnRequest) => Promise<void>;
  send: (text: string, attachments: Draft[]) => void;
  // @ mention lookup over workspace files
  searchFiles: (query: string) => Promise<FileHit[]>;
  stop: () => void;
  permission: (sessionId: string, blockId: string, optionId: string) => void;
  // The question card was closed: answers keyed by question id, or skip
  answer?: (sessionId: string, blockId: string, answers: QuestionAnswers, skip?: boolean) => void;
  buildPlan?: (sessionId: string, planId: string, model?: { configId: string; value: string }, optionId?: string) => void;
  openPlan?: (sessionId: string, planId: string) => void;
  openFile?: (sessionId: string, path: string, line?: number) => void;
  // An agent-emitted image's blob file opens in the editor by its store name (the host resolves the absolute path)
  openBlob?: (sessionId: string, name: string) => void;
  setMode: (id: string) => void;
  setConfig: (configId: string, value: string) => void;
  selectSession: (id: string) => void;
  // Without an agent the host falls back to the configured defaultAgent
  newSession: (agent?: AgentInfo['id']) => void;
  renameSession: (id: string, title: string) => void;
  deleteSession: (id: string) => void;
  restoreSession: (id: string) => void;
  pinSession: (id: string, pinned: boolean) => void;
  // Project scoping of the list: re-home a session into this window's workspace folder
  moveSession?: (id: string) => void;
  // Account layer: selecting an account rebinds the current session; adding an account goes through import / terminal login; removing only deletes the locally saved credential
  selectAccount: (id: string) => void;
  addAccount: (agent: AgentInfo['id'], via: AddAccountVia) => void;
  removeAccount: (id: string) => void;
  // An account list opened: re-read the quotas of that agent's accounts
  refreshQuota?: (agent: AgentInfo['id']) => void;
  compact: () => void;
  login: (methodId?: string) => void;
  retry: () => void;
  // Send the last user turn again after its agent turn ended in error
  retryTurn: () => void;
  // Drop the agent process and resume the same native session (a live connection whose prompts keep failing)
  reconnect: () => void;
  // Queued prompts: drop one / replace one in place (kept attachments by index plus new drafts)
  dequeue?: (sessionId: string, id: string) => void;
  sendQueued?: (sessionId: string, id: string) => void;
  editQueued?: (sessionId: string, id: string, text: string, retainedAttachments: number[], attachments: Draft[]) => void;
  // Start a new session whose transcript is this session's turns through the given agent turn
  forkSession?: (sessionId: string, turnIndex: number) => void;
  // Write the session as Markdown or JSON under the data dir's exports/ and open it in the editor
  exportSession?: (id: string, format: 'markdown' | 'json') => void;
  // Open the session in an editor tab
  openInEditor?: (sessionId: string) => void;
  // History list's import popover: read the agent's own sessions / import one as a local record
  listNativeSessions?: (agent: AgentInfo['id']) => void;
  importNativeSession?: (agent: AgentInfo['id'], s: NativeSessionInfo) => void;
  // Subagent observation: the inspector subscribes to one child's transcript stream at a time
  observeSubagent?: (sessionId: string, subagentId: string) => void;
  unobserveSubagent?: (sessionId: string, subagentId: string) => void;
  cancelSubagent?: (sessionId: string, subagentId: string) => void;
}

export interface ShellProps {
  appearance: Appearance;
  theme: Theme;
  // Rendering preferences from the settings page (fixed theme, font sizes, diff markers, smoothing); the LAB leaves it out
  look?: ShellLook;
  // Lives in the sidebar or the editor area: decides the background level and content width
  host: 'sidebar' | 'editor';
  agent: AgentInfo;
  agents: AgentInfo[];
  // Accounts across all agents; the current session is bound to accountId
  accounts?: AccountInfo[];
  accountId?: string;
  accountAction?: AccountAction;
  // Option families hidden from the composer menus (acpira.hiddenOptions)
  hidden?: HiddenMap;
  title: string;
  status: SessionStatus;
  external?: ExternalSessionInfo;
  error?: string;
  authMethods?: AuthMethodInfo[];
  turns: Turn[];
  running: boolean;
  queued?: QueuedPrompt[];
  controls: SessionControls;
  usage?: Usage;
  // The slash commands the agent advertised for this session (available_commands_update): the composer's / menu,
  // and the context panel only gets a compact button when `compact` is among them
  commands?: SlashCommand[];
  compactAt?: number;
  sessions: SessionSummary[];
  activeSessionId?: string;
  // Workspace root of the session; attachments are labeled relative to it
  cwd?: string;
  // This window's workspace folder and the list scope setting: the session list filters by them (the LAB leaves both out and shows everything)
  workspace?: string;
  sessionScope?: SessionScope;
  // The import popover's current listing (keyed by the agent it was requested for)
  nativeSessions?: NativeSessionsState;
  // Where attachment blobs are served from (the host's sessions directory as a webview URI); absent in the LAB
  blobBase?: string;
  // Delegated children of this session; their transcripts arrive separately under `${sessionId}:${subagentId}`
  subagents?: SubagentSummary[];
  subagentTranscripts?: Record<string, { rev: number; running: boolean; turns: Turn[] }>;
  on: ShellHandlers;
  // Opens the settings page (a local view swap, not a host action — hence not part of ShellHandlers)
  onOpenSettings?: () => void;
  // For replaying the entrance animation: remounts the conversation when it changes
  replayKey?: number | string;
}

// A toast: text plus an optional undo; each dismisses itself, several can stack (an attachment notice must not take the undo of a deletion with it)
interface ToastState {
  key: string;
  text: string;
  icon?: ReactNode;
  undo?: () => void;
}

// Chat and optional session column share one webview. Width determines docking in both host surfaces.
export function Shell(p: ShellProps) {
  const { appearance: a, on } = p;
  const wide = p.host === 'editor';
  const root = useRef<HTMLDivElement>(null);
  const sessionPanel = useRef<HTMLElement>(null);
  const position = p.look?.sessionListPosition ?? (a.sessions === 'drawer' ? 'left' : 'hidden');
  const [canDock, setCanDock] = useState(false);
  // Observe the actual panel, not the browser window: IDE sidebars and split editor tabs resize independently.
  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const minWidth = Number.parseFloat(getComputedStyle(element).getPropertyValue('--session-dock-min'));
    const update = () => setCanDock(element.clientWidth >= minWidth);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const planDock = useRef<HTMLDivElement>(null);
  const dockHeight = useRef(0);
  const threadContent = useRef<HTMLDivElement | null>(null);
  const toastLayer = useRef<HTMLDivElement | null>(null);
  useScrollReveal(root);
  // The dock height becomes the transcript's scroll clearance and the toast offset. Both are written
  // onto the consumers themselves: a custom property on their shared ancestor is inherited, so every
  // dock resize — each frame of the fold animation — would restyle the whole transcript.
  const contentRef = useCallback((el: HTMLDivElement | null) => {
    threadContent.current = el;
    if (el) el.style.paddingBottom = dockHeight.current ? `${dockHeight.current}px` : '';
  }, []);
  const toastRef = useCallback((el: HTMLDivElement | null) => {
    toastLayer.current = el;
    if (el) el.style.setProperty('--thread-dock-height', `${dockHeight.current}px`);
  }, []);
  useLayoutEffect(() => {
    const dock = planDock.current;
    if (!dock) return;
    const sync = () => {
      const height = dock.offsetHeight;
      if (height === dockHeight.current) return;
      dockHeight.current = height;
      const content = threadContent.current;
      if (content) content.style.paddingBottom = height ? `${height}px` : '';
      toastLayer.current?.style.setProperty('--thread-dock-height', `${height}px`);
    };
    sync();
    const observer = new ResizeObserver(sync);
    observer.observe(dock);
    return () => observer.disconnect();
  }, []);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const closeDrawer = useCallback(() => {
    if (sessionPanel.current?.contains(document.activeElement)) {
      root.current?.querySelector<HTMLButtonElement>('[data-session-toggle]')?.focus();
    }
    setDrawerOpen(false);
  }, []);
  useEffect(closeDrawer, [position, canDock, closeDrawer]);
  useEffect(() => {
    if (drawerOpen && !canDock) sessionPanel.current?.querySelector<HTMLInputElement>('input')?.focus();
  }, [drawerOpen, canDock]);
  const [editing, setEditing] = useState<{ sessionId: string; index: number }>();
  useEffect(() => setEditing(undefined), [p.activeSessionId]);
  // The subagent inspector: one open child at a time, docked beside the column when it is wide enough
  const mainColumn = useRef<HTMLDivElement>(null);
  const [canDockInspector, setCanDockInspector] = useState(false);
  useLayoutEffect(() => {
    const element = mainColumn.current;
    if (!element) return;
    const styles = getComputedStyle(element);
    const minWidth = Number.parseFloat(styles.getPropertyValue('--subagent-dock-min'));
    const paneWidth = Number.parseFloat(styles.getPropertyValue('--subagent-pane-w'));
    // Hysteresis: once docked, the column lost the pane's width — count it back so the panel does not flap open/closed
    const update = () => setCanDockInspector(docked => element.clientWidth + (docked ? paneWidth : 0) >= minWidth);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const [inspect, setInspect] = useState<{ id: string }>();
  const [graphOpen, setGraphOpen] = useState(false);
  useEffect(() => setGraphOpen(false), [p.activeSessionId]);
  useEffect(() => setInspect(undefined), [p.activeSessionId]);
  const inspectNode = inspect !== undefined ? p.subagents?.find(n => n.id === inspect.id) : undefined;
  useEffect(() => { if (inspect !== undefined && inspectNode === undefined) setInspect(undefined); }, [inspect, inspectNode]);
  // Observing is exactly the open child; the previous subscription ends before the next one starts
  useEffect(() => {
    if (!inspect || !p.activeSessionId || !on.observeSubagent) return;
    const sid = p.activeSessionId;
    const id = inspect.id;
    on.observeSubagent(sid, id);
    return () => on.unobserveSubagent?.(sid, id);
  }, [inspect?.id, p.activeSessionId, on.observeSubagent, on.unobserveSubagent]);
  const onInspect = useCallback((id: string) => setInspect({ id }), []);
  // Deletion applies immediately, with an undoable toast floating at the bottom (modeled on Codex's archive), no confirmation dialog; refused attachments show up the same way
  const [toasts, setToasts] = useState<ToastState[]>([]);
  const dropToast = useCallback((key: string) => setToasts(ts => ts.filter(t => t.key !== key)), []);
  const pushToast = useCallback((t: ToastState) => setToasts(ts => [...ts.filter(x => x.key !== t.key), t]), []);
  const notice = useCallback((text: string) => pushToast({ key: `n${Date.now()}`, text, icon: <Paperclip className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} /> }), [pushToast]);
  const handlers = useMemo<ShellHandlers>(() => ({
    ...on,
    deleteSession: id => {
      const title = p.sessions.find(s => s.id === id)?.title ?? t('session.fallbackTitle');
      on.deleteSession(id);
      pushToast({ key: id, text: t('session.deleted', { title }), undo: () => { on.restoreSession(id); dropToast(id); } });
    },
    // A moved session leaves a "this project" list at once (or loses its project tag under "all"); the toast says where it went
    moveSession: on.moveSession && (id => {
      const title = p.sessions.find(s => s.id === id)?.title ?? t('session.fallbackTitle');
      on.moveSession!(id);
      pushToast({ key: `m${id}`, text: t('session.moved', { title }) });
    }),
  }), [on, p.sessions, pushToast, dropToast]);
  const blobUrl = useMemo(() => (p.blobBase && p.activeSessionId ? (blob: string) => `${p.blobBase}/${p.activeSessionId}/${blob}` : undefined), [p.blobBase, p.activeSessionId]);
  // Child transcripts share the root session's blob store, so both contexts live above Thread and the inspector
  const openBlob = useMemo(() => p.activeSessionId && on.openBlob ? (name: string) => on.openBlob!(p.activeSessionId!, name) : undefined, [p.activeSessionId, on.openBlob]);
  // The card for a turn that stopped short stands until dismissed or until the transcript moves on; the key ties the dismissal to that one turn.
  // While the session isn't ready the Notice has the floor (a login problem after a failed prompt is its business)
  const lastTurn = p.turns[p.turns.length - 1];
  const alertKey = `${p.activeSessionId}:${p.turns.length}`;
  const [dismissedAlert, setDismissedAlert] = useState<string>();
  const alertTurn = !p.running && p.status === 'ready' && lastTurn?.role === 'agent' && isShortStop(lastTurn) && dismissedAlert !== alertKey ? lastTurn : undefined;
  // The agent's open question card sits above the composer while the turn waits on it; a resolved card stays in the message as the record.
  // A child's open card pins the same slot, tagged with the delegation chain it arrived through
  const rootQuestion = p.running && lastTurn?.role === 'agent' ? lastTurn.blocks.find((b): b is QuestionBlock => b.type === 'question' && !b.outcome) : undefined;
  const childQuestion = p.subagents?.find(n => n.question);
  const question = rootQuestion ?? childQuestion?.question;
  const questionSubtitle = !rootQuestion && childQuestion
    ? t('subagents.provenance', { path: breadcrumb(childQuestion.id, p.subagents ?? []).map(n => subagentTitle(n, t)).join(' › ') })
    : undefined;
  const sessionsPanel = (
    <SessionList
      fill
      sessions={p.sessions}
      agents={p.agents}
      activeId={p.activeSessionId}
      workspace={p.workspace}
      scope={p.sessionScope}
      onSelect={id => { on.selectSession(id); closeDrawer(); }}
      onRename={on.renameSession}
      onDelete={handlers.deleteSession}
      onPin={on.pinSession}
      onMove={handlers.moveSession}
      onExport={on.exportSession}
      activeAgent={p.agent.id}
      nativeSessions={p.nativeSessions}
      onListNative={on.listNativeSessions}
      onImportNative={on.importNativeSession}
    />
  );

  const composerProps: ComposerProps = useMemo(() => ({
    running: p.running, disabled: p.status !== 'ready' && p.status !== 'starting',
    controlsLocked: p.status !== 'ready',
    theme: p.theme, turns: p.turns, controls: p.controls, hidden: p.hidden?.[p.agent.id],
    usage: p.usage, commands: p.commands, compactAt: p.compactAt, cwd: p.cwd ?? '',
    onSend: on.send, onSearchFiles: on.searchFiles, onNotice: notice, onStop: on.stop,
    onSetMode: on.setMode, onSetConfig: on.setConfig, onCompact: on.compact,
  }), [p.running, p.status, p.theme, p.turns, p.controls, p.hidden, p.agent.id, p.usage, p.commands, p.compactAt, p.cwd, on.send, on.searchFiles, notice, on.stop, on.setMode, on.setConfig, on.compact]);
  // Context values above the transcript must not change on every stream push: React walks the whole memoized tree for consumers each time
  const permissions = useStableList(useMemo(() => p.turns.flatMap(t => t.role === 'agent' ? t.blocks.filter((b): b is PermissionBlock => b.type === 'permission') : []), [p.turns]));
  const planDoc = useMemo(() => ({
    controls: p.controls, hidden: p.hidden?.[p.agent.id], running: p.running, ready: p.status === 'ready', theme: p.theme, permissions,
    build: p.activeSessionId && on.buildPlan ? (id: string, model?: { configId: string; value: string }, optionId?: string) => on.buildPlan!(p.activeSessionId!, id, model, optionId) : undefined,
    open: p.activeSessionId && on.openPlan ? (id: string) => on.openPlan!(p.activeSessionId!, id) : undefined,
  }), [p.controls, p.hidden, p.agent.id, p.running, p.status, p.theme, permissions, p.activeSessionId, on.buildPlan, on.openPlan]);
  // Stable across stream pushes (every prompt card subscribes); the composer props go through their own context to the open editor
  const editable = !composerProps.disabled && !composerProps.running;
  const history = useMemo(() => on.editTurn && p.activeSessionId ? {
    sessionId: p.activeSessionId, edit: on.editTurn, editable,
    editing: editing?.sessionId === p.activeSessionId ? editing.index : undefined,
    select: (index: number | undefined) => setEditing(current => index === undefined
      ? current?.sessionId === p.activeSessionId ? undefined : current
      : { sessionId: p.activeSessionId!, index }),
  } : undefined, [on.editTurn, p.activeSessionId, editable, editing]);
  const openToolFile = useMemo(() => p.activeSessionId && on.openFile
    ? (path: string, line?: number) => on.openFile!(p.activeSessionId!, path, line) : undefined, [p.activeSessionId, on.openFile]);
  // Per-turn actions (copy / fork / stats): memoized like the other contexts so a stream push does not re-render consumers;
  // external conversations have no ACP transcript to fork
  const turnActions = useMemo(() => p.activeSessionId ? {
    sessionId: p.activeSessionId, controls: p.controls,
    fork: on.forkSession && !p.external ? (turnIndex: number) => on.forkSession!(p.activeSessionId!, turnIndex) : undefined,
  } : undefined, [p.activeSessionId, p.controls, on.forkSession, p.external]);

  return (
    <AppearanceContext.Provider value={a}>
      <ThemeContext.Provider value={p.theme}>
      <ShellLayerContext.Provider value={root}>
      <BlobUrlContext.Provider value={blobUrl}>
      <OpenBlobContext.Provider value={openBlob}>
        <div
          ref={root}
          className={cn('acp-shell relative flex h-full min-h-0 w-full overflow-hidden', wide && 'acp-wide')}
          data-theme={p.theme}
          data-surface-host={p.host}
          data-agent={p.agent.id}
          {...appearanceDataAttrs(a)}
          {...lookAttrs(p.look)}
        >
          {position !== 'hidden' && (
            <>
              {!canDock && drawerOpen && <div aria-hidden="true" className="absolute inset-0 z-10 bg-black/20" onClick={closeDrawer} />}
              <aside ref={sessionPanel} aria-label={t('session.listAria')} inert={!canDock && !drawerOpen}
                data-session-panel={position} data-docked={canDock}
                onKeyDown={e => { if (e.key === 'Escape' && !e.defaultPrevented && !canDock) { e.stopPropagation(); closeDrawer(); } }}
                className={cn(
                  'flex min-h-0 shrink-0 flex-col bg-bg-0',
                  position === 'right' ? 'order-last border-l border-line' : 'border-r border-line',
                  canDock ? 'w-pop-md' : 'absolute inset-y-0 z-20 w-pop-lg max-w-[calc(100%-var(--ctl))] shadow-pop',
                  !canDock && (position === 'right' ? 'right-0' : 'left-0'),
                  !canDock && !drawerOpen && 'hidden',
                )}>
                <div className="flex h-hdr shrink-0 items-center justify-between gap-gap-half border-b border-line px-pad">
                  <span className="text-2 font-medium text-fg-strong">{t('session.history')}</span>
                  {!canDock && <IconButton onClick={closeDrawer} aria-label={t('common.close')} title={t('common.close')}><X strokeWidth={1.5} /></IconButton>}
                </div>
                <div className="flex min-h-0 flex-1 flex-col p-gap-half">{sessionsPanel}</div>
              </aside>
            </>
          )}
          <div ref={mainColumn} className="relative flex min-w-0 flex-1 flex-col">
            <Header
              title={p.title}
              sessions={p.sessions}
              agent={p.agent}
              agents={p.agents}
              accounts={p.accounts}
              accountId={p.accountId}
              activeSessionId={p.activeSessionId}
              workspace={p.workspace}
              sessionScope={p.sessionScope}
              nativeSessions={p.nativeSessions}
              on={handlers}
              onToggleDrawer={() => setDrawerOpen(o => !o)}
              drawerOpen={drawerOpen}
              sessionPanel={position}
              sessionPanelDocked={canDock}
              onOpenSettings={p.onOpenSettings}
            />
            <div className="relative flex min-h-0 flex-1 flex-col">
              {/* The overlay inspector replaces this block visually; it stays mounted (scroll + draft survive) but inert */}
              <div inert={inspectNode !== undefined && !canDockInspector || undefined} aria-hidden={inspectNode !== undefined && !canDockInspector || undefined} className="flex min-h-0 flex-1 flex-col">
                <div className="relative flex min-h-0 flex-1 flex-col">
                  <PlanDocumentContext.Provider value={planDoc}>
                    <HistoryContext.Provider value={history}>
                    <HistoryComposerContext.Provider value={history?.editing !== undefined ? composerProps : undefined}>
                      <OpenToolFileContext.Provider value={openToolFile}>
                        <TurnActionsContext.Provider value={turnActions}>
                          <Thread key={p.activeSessionId} turns={p.turns} running={p.running} wide={wide} replayKey={p.replayKey} blobUrl={blobUrl} contentRef={contentRef} commands={p.commands}
                            subagents={p.subagents} onInspect={onInspect}
                            onPermission={(blockId, optionId) => { if (p.activeSessionId) on.permission(p.activeSessionId, blockId, optionId); }} />
                        </TurnActionsContext.Provider>
                      </OpenToolFileContext.Provider>
                    </HistoryComposerContext.Provider>
                    </HistoryContext.Provider>
                  </PlanDocumentContext.Provider>
                  <div
                    ref={planDock}
                    data-plan-dock
                    className={cn('pointer-events-none absolute inset-x-0 bottom-0 z-10', wide && a.composer === 'island' && 'mx-auto w-full max-w-[calc(var(--content-w)+2*var(--pad))]')}
                  >
                    <PlanBar key={`plan:${p.activeSessionId}`} turns={p.turns} running={p.running} />
                  </div>
                  {toasts.length > 0 && (
                    <div ref={toastRef} className="pointer-events-none absolute inset-x-0 bottom-[calc(var(--gap)+var(--thread-dock-height,0px))] z-10 flex flex-col items-center gap-1 px-page">
                      {toasts.map(t => <Toast key={t.key} text={t.text} icon={t.icon} onUndo={t.undo} onClose={() => dropToast(t.key)} />)}
                    </div>
                  )}
                </div>
                <div className={cn('shrink-0', wide && a.composer === 'island' && 'mx-auto w-full max-w-[calc(var(--content-w)+2*var(--pad))]')}>
                  {question && on.answer && p.activeSessionId && <Questions key={question.id} block={question} subtitle={questionSubtitle} onAnswer={(blockId, answers, skip) => on.answer!(p.activeSessionId!, blockId, answers, skip)} />}
                  {alertTurn && (
                    <Alert
                      turn={alertTurn}
                      onRetry={on.retryTurn}
                      onReconnect={on.reconnect}
                      onCompact={p.commands?.some(c => c.name === 'compact') ? on.compact : undefined}
                      onContinue={() => on.send(t('alert.continueText'), [])}
                      onDismiss={() => setDismissedAlert(alertKey)}
                    />
                  )}
                  {p.external ? <ExternalSessionNotice info={p.external} /> : <Notice
                    status={p.status} error={p.error} agent={p.agent} authMethods={p.authMethods}
                    accounts={p.accounts?.filter(x => x.agent === p.agent.id)} accountId={p.accountId}
                    accountAction={p.accountAction}
                    onLogin={on.login} onRetry={on.retry} onNewSession={() => on.newSession(p.agent.id)}
                    onSelectAccount={on.selectAccount} onAddAccount={via => on.addAccount(p.agent.id, via)}
                  />}
                  {p.queued?.length && p.activeSessionId
                    ? <Queue key={`queue:${p.activeSessionId}`} items={p.queued} composer={composerProps} blobUrl={blobUrl}
                        on={on.dequeue && on.editQueued ? {
                          remove: id => on.dequeue!(p.activeSessionId!, id),
                          sendNow: on.sendQueued && (id => on.sendQueued!(p.activeSessionId!, id)),
                          edit: (id, text, kept, drafts) => on.editQueued!(p.activeSessionId!, id, text, kept, drafts),
                        } : undefined} />
                    : null}
                  {/* Sibling keys include the component role; duplicate session-only keys leave stale queue rows after reconciliation. */}
                  {!p.external && <Composer key={`composer:${p.activeSessionId}`} {...composerProps} draftKey={p.activeSessionId}
                    toolbarStart={!!p.subagents?.length && <Chip narrow="icon" caret={false} className="shrink-0" icon={<Network strokeWidth={1.5} />}
                      aria-label={`${t('subagents.graph')} · ${t('subagents.entry', { n: p.subagents.length })}`}
                      title={`${t('subagents.graph')} · ${t('subagents.entry', { n: p.subagents.length })}`}
                      aria-haspopup="dialog" aria-expanded={graphOpen} onClick={() => setGraphOpen(true)}>
                      {t('subagents.entry', { n: p.subagents.length })}
                    </Chip>} />}
                </div>
              </div>
              {inspectNode !== undefined && !canDockInspector && inspect !== undefined && p.activeSessionId !== undefined && (
                <div data-subagent-panel="overlay" className="absolute inset-0 z-20 flex flex-col bg-bg-0">
                  <SubagentInspector
                    mode="overlay"
                    node={inspectNode}
                    transcript={p.subagentTranscripts?.[`${p.activeSessionId}:${inspect.id}`]}
                    onGraph={() => setGraphOpen(true)}
                    onClose={() => setInspect(undefined)}
                    onCancel={inspectNode.controls.cancel && on.cancelSubagent ? () => on.cancelSubagent!(p.activeSessionId!, inspectNode.id) : undefined}
                    onPermission={(blockId, optionId) => on.permission(p.activeSessionId!, blockId, optionId)}
                    question={inspectNode.question}
                    onAnswer={on.answer ? (blockId, answers, skip) => on.answer!(p.activeSessionId!, blockId, answers, skip) : undefined}
                    blobUrl={blobUrl}
                  />
                </div>
              )}
            </div>
          </div>
          {inspectNode !== undefined && canDockInspector && inspect !== undefined && p.activeSessionId !== undefined && (
            <aside data-subagent-panel="docked" className="order-last flex w-subagent-pane shrink-0 flex-col border-l border-line bg-bg-0">
              <SubagentInspector
                mode="docked"
                node={inspectNode}
                transcript={p.subagentTranscripts?.[`${p.activeSessionId}:${inspect.id}`]}
                onGraph={() => setGraphOpen(true)}
                onClose={() => setInspect(undefined)}
                onCancel={inspectNode.controls.cancel && on.cancelSubagent ? () => on.cancelSubagent!(p.activeSessionId!, inspectNode.id) : undefined}
                onPermission={(blockId, optionId) => on.permission(p.activeSessionId!, blockId, optionId)}
                blobUrl={blobUrl}
              />
            </aside>
          )}
        </div>
        <SubagentGraph nodes={p.subagents ?? []} sessionTitle={p.title} open={graphOpen && !!p.subagents?.length} onOpenChange={setGraphOpen} onInspect={onInspect} selectedId={inspect?.id} />
      </OpenBlobContext.Provider>
      </BlobUrlContext.Provider>
      </ShellLayerContext.Provider>
      </ThemeContext.Provider>
    </AppearanceContext.Provider>
  );
}

interface ThreadProps {
  turns: Turn[];
  running: boolean;
  wide: boolean;
  replayKey?: number | string;
  blobUrl?: (blob: string) => string;
  contentRef?: (el: HTMLDivElement | null) => void;
  // Advertised slash commands: sent user messages paint their `/name` tokens like the composer does
  commands?: SlashCommand[];
  subagents?: SubagentSummary[];
  onInspect?: (id: string) => void;
  onPermission: (blockId: string, optionId: string) => void;
}

// Entrance stagger caps out at the 12th block, so long sessions don't take seconds
const STAGGER_CAP = 12;

// Conversation flow: stick-to-bottom following only happens on transcript changes (new content / streaming growth); user actions like expand / collapse never touch the scroll position —
// the toggle under the mouse stays put while the content below it moves. Scrolling away from the bottom releases the follow; scrolling back to the bottom restores it
function Thread({ turns, running, wide, replayKey, blobUrl, contentRef, commands, subagents, onInspect, onPermission }: ThreadProps) {
  const ref = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    pinned.current = true;
    const pin = () => { if (scrollerUsable(el) && pinned.current) el.scrollTop = el.scrollHeight; };
    pin();
    // A hidden sidebar collapses this to no box and fires a scroll that looks like "left the bottom".
    const onScroll = () => { if (scrollerUsable(el)) pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48; };
    el.addEventListener('scroll', onScroll, { passive: true });
    // Keep stuck to the bottom when the container itself shrinks (composer grows / panel narrows); observe only the container, not the content
    const ro = new ResizeObserver(pin);
    ro.observe(el);
    return () => { ro.disconnect(); el.removeEventListener('scroll', onScroll); };
  }, [replayKey]);
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && scrollerUsable(el) && pinned.current) el.scrollTop = el.scrollHeight;
  }, [turns, running]);

  // Group the session's subagent nodes by the turn that announced them; unchanged arrays keep their
  // reference so memoized turns do not re-render on an unrelated child update
  const byTurnRef = useRef<Map<number, SubagentSummary[]> | undefined>(undefined);
  const byTurn = nodesByTurn(subagents ?? [], byTurnRef.current);
  byTurnRef.current = byTurn;

  // Each user message sticks only within its own exchange. Automatic commands belong
  // to the preceding exchange so compaction does not replace the user's context.
  let i = 0;
  // A submitted prompt may already be visible below the active /compact turn.
  let activeAgentIndex = turns.length - 1;
  while (activeAgentIndex >= 0 && turns[activeAgentIndex]?.role !== 'agent') activeAgentIndex--;
  const exchanges: { key: number; messages: ReactNode[] }[] = [];
  turns.forEach((turn, ti) => {
    const previous = turns[ti - 1];
    if (planExecutionId(turn, previous)) return;
    const compacting = previous?.role === 'user' && /^\/compact(?:\s|$)/.test(previous.text.trim());
    const index = Math.min(i, STAGGER_CAP);
    i += turn.role === 'agent' ? turn.blocks.length + 1 : 1;
    if (!exchanges.length || (turn.role === 'user' && !turn.auto)) {
      exchanges.push({ key: ti, messages: [] });
    }
    // Fold memory key: the session plus the turn's position and start time, so an edited-away turn at the same index does not inherit a choice
    const memoryKey = replayKey !== undefined ? `${replayKey}:${ti}:${turn.role === 'agent' ? turn.startedAt ?? '' : ''}` : undefined;
    // Session-wide nodes only reach turns that announce one; handing the list to every
    // memoized turn would re-render all of them on each child activity tick.
    const mine = turn.role === 'agent' ? byTurn.get(ti) : undefined;
    exchanges[exchanges.length - 1]!.messages.push(turn.role === 'user'
      ? <HistoryMessage key={turn.id ?? ti} turn={turn} turnIndex={ti} index={index} blobUrl={blobUrl} commands={commands} />
      : <AgentMessage key={ti} turn={turn} index={index} compacting={compacting} running={running && ti === activeAgentIndex && !turn.stop} onPermission={onPermission} memoryKey={memoryKey}
          turnIndex={ti} last={ti === turns.length - 1} settings={previous?.role === 'user' ? previous.settings : undefined}
          subagents={mine} allSubagents={mine ? subagents : undefined} onInspect={onInspect} />);
  });
  return (
    <div ref={ref} data-thread className="scroll-stable min-h-0 min-w-0 flex-1 overflow-y-auto px-page [container-type:size] [overflow-anchor:none]">
      <div key={replayKey} ref={contentRef} className={cn('mx-auto flex flex-col gap-msg pt-pad-y pb-gap', wide && 'max-w-(--content-w)')}>
        {exchanges.map(exchange => (
          // Positioned so the prompt's stuck-state sentinel can sit at the exchange's top edge. Paint containment gives each exchange
          // its own paint offset, so a fold opening mid-thread no longer re-walks every later exchange each frame (see AGENTS.md,
          // transcript render budget); the clip it brings is pushed out by --hit on the sides and bottom, where row hit areas and
          // card shadows reach past the column, and the top edge stays put for the sentinel.
          <section key={exchange.key} className="relative -mx-hit -mb-hit flex min-w-0 flex-col gap-msg px-hit pb-hit contain-paint">
            {exchange.messages}
          </section>
        ))}
      </div>
    </div>
  );
}
