import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Paperclip } from 'lucide-react';
import type { AccountInfo, AgentInfo, AuthMethodInfo, Draft, PermissionBlock, QuestionAnswers, QuestionBlock, QueuedPrompt, SessionControls, SessionStatus, SessionSummary, SlashCommand, Turn, Usage } from '@shared/transcript';
import type { HiddenMap, SessionScope } from '@shared/settings';
import type { AccountAction, AddAccountVia, EditTurnRequest, FileHit } from '@shared/protocol';
import { AppearanceContext, appearanceDataAttrs, type Appearance } from '../appearance';
import { lookAttrs, ThemeContext, type ShellLook, type Theme } from '../look';
import { t } from '../i18n';
import { ShellLayerContext } from '../ui/Popover';
import { cn } from '../ui/cn';
import { useScrollReveal } from '../ui/useScrollReveal';
import { useStableList } from '../ui/useStableList';
import { scrollerUsable } from './promptStuck';
import { Header } from './Header';
import { SessionList } from './SessionList';
import { AgentMessage } from './Turns';
import { HistoryComposerContext, HistoryContext, HistoryMessage } from './HistoryMessage';
import { Composer, type ComposerProps } from './Composer';
import { Notice } from './Notice';
import { Toast } from '../ui/Toast';
import { Alert, isShortStop } from './Alert';
import { Questions, type OnAnswer } from './Questions';
import { PlanBar } from './PlanBar';
import { PlanDocumentContext } from './PlanDocument';
import { planExecutionId } from '@shared/planExecution';
import { Queue } from './Queue';
import { OpenToolFileContext } from './ToolCall';

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
  setMode: (id: string) => void;
  setConfig: (configId: string, value: string) => void;
  selectAgent: (id: AgentInfo['id']) => void;
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
  // Where attachment blobs are served from (the host's sessions directory as a webview URI); absent in the LAB
  blobBase?: string;
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

// Chat shell: header / conversation flow / composer stacked vertically; the drawer axis adds a column on the left. The shell root doubles as the overlay mount point
export function Shell(p: ShellProps) {
  const { appearance: a, on } = p;
  const wide = p.host === 'editor';
  const root = useRef<HTMLDivElement>(null);
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
  const [editing, setEditing] = useState<{ sessionId: string; index: number }>();
  useEffect(() => setEditing(undefined), [p.activeSessionId]);
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
  // The card for a turn that stopped short stands until dismissed or until the transcript moves on; the key ties the dismissal to that one turn.
  // While the session isn't ready the Notice has the floor (a login problem after a failed prompt is its business)
  const lastTurn = p.turns[p.turns.length - 1];
  const alertKey = `${p.activeSessionId}:${p.turns.length}`;
  const [dismissedAlert, setDismissedAlert] = useState<string>();
  const alertTurn = !p.running && p.status === 'ready' && lastTurn?.role === 'agent' && isShortStop(lastTurn) && dismissedAlert !== alertKey ? lastTurn : undefined;
  // The agent's open question card sits above the composer while the turn waits on it; a resolved card stays in the message as the record
  const question = p.running && lastTurn?.role === 'agent' ? lastTurn.blocks.find((b): b is QuestionBlock => b.type === 'question' && !b.outcome) : undefined;
  const sessionsPanel = (
    <SessionList
      sessions={p.sessions}
      agents={p.agents}
      activeId={p.activeSessionId}
      workspace={p.workspace}
      scope={p.sessionScope}
      onSelect={id => { on.selectSession(id); setDrawerOpen(false); }}
      onRename={on.renameSession}
      onDelete={handlers.deleteSession}
      onPin={on.pinSession}
      onMove={handlers.moveSession}
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

  return (
    <AppearanceContext.Provider value={a}>
      <ThemeContext.Provider value={p.theme}>
      <ShellLayerContext.Provider value={root}>
        <div
          ref={root}
          className={cn('acp-shell relative flex h-full min-h-0 w-full overflow-hidden', wide && 'acp-wide')}
          data-theme={p.theme}
          data-surface-host={p.host}
          data-agent={p.agent.id}
          {...appearanceDataAttrs(a)}
          {...lookAttrs(p.look)}
        >
          {a.sessions === 'drawer' && (
            <aside className={cn(
              'shrink-0 overflow-auto border-r border-line p-2',
              wide ? 'w-56 bg-bg-0' : 'absolute inset-y-0 left-0 z-20 w-64 bg-bg-1 shadow-pop transition-transform',
              !wide && !drawerOpen && '-translate-x-[102%]',
            )}>
              {sessionsPanel}
            </aside>
          )}
          <div className="relative flex min-w-0 flex-1 flex-col">
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
              on={handlers}
              onToggleDrawer={() => setDrawerOpen(o => !o)}
              drawerOpen={drawerOpen}
              onOpenSettings={p.onOpenSettings}
            />
            <div className="relative flex min-h-0 flex-1 flex-col">
              <PlanDocumentContext.Provider value={planDoc}>
                <HistoryContext.Provider value={history}>
                <HistoryComposerContext.Provider value={history?.editing !== undefined ? composerProps : undefined}>
                  <OpenToolFileContext.Provider value={openToolFile}>
                    <Thread key={p.activeSessionId} turns={p.turns} running={p.running} wide={wide} replayKey={p.replayKey} blobUrl={blobUrl} contentRef={contentRef} onPermission={(blockId, optionId) => { if (p.activeSessionId) on.permission(p.activeSessionId, blockId, optionId); }} />
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
              {question && on.answer && p.activeSessionId && <Questions key={question.id} block={question} onAnswer={(blockId, answers, skip) => on.answer!(p.activeSessionId!, blockId, answers, skip)} />}
              {alertTurn && (
                <Alert
                  turn={alertTurn}
                  onRetry={on.retryTurn}
                  onReconnect={on.reconnect}
                  onContinue={() => on.send(t('alert.continueText'), [])}
                  onDismiss={() => setDismissedAlert(alertKey)}
                />
              )}
              <Notice
                status={p.status} error={p.error} agent={p.agent} authMethods={p.authMethods}
                accounts={p.accounts?.filter(x => x.agent === p.agent.id)} accountId={p.accountId}
                accountAction={p.accountAction}
                onLogin={on.login} onRetry={on.retry} onNewSession={() => on.newSession(p.agent.id)}
                onSelectAccount={on.selectAccount} onAddAccount={via => on.addAccount(p.agent.id, via)}
              />
              {p.queued?.length && p.activeSessionId
                ? <Queue key={`queue:${p.activeSessionId}`} items={p.queued} composer={composerProps} blobUrl={blobUrl}
                    on={on.dequeue && on.editQueued ? {
                      remove: id => on.dequeue!(p.activeSessionId!, id),
                      sendNow: on.sendQueued && (id => on.sendQueued!(p.activeSessionId!, id)),
                      edit: (id, text, kept, drafts) => on.editQueued!(p.activeSessionId!, id, text, kept, drafts),
                    } : undefined} />
                : null}
              {/* Sibling keys include the component role; duplicate session-only keys leave stale queue rows after reconciliation. */}
              <Composer key={`composer:${p.activeSessionId}`} {...composerProps} draftKey={p.activeSessionId} />
            </div>
          </div>
        </div>
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
  onPermission: (blockId: string, optionId: string) => void;
}

// Entrance stagger caps out at the 12th block, so long sessions don't take seconds
const STAGGER_CAP = 12;

// Conversation flow: stick-to-bottom following only happens on transcript changes (new content / streaming growth); user actions like expand / collapse never touch the scroll position —
// the toggle under the mouse stays put while the content below it moves. Scrolling away from the bottom releases the follow; scrolling back to the bottom restores it
function Thread({ turns, running, wide, replayKey, blobUrl, contentRef, onPermission }: ThreadProps) {
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

  // Each user message sticks only within its own exchange. Automatic commands belong
  // to the preceding exchange so compaction does not replace the user's context.
  let i = 0;
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
    exchanges[exchanges.length - 1]!.messages.push(turn.role === 'user'
      ? <HistoryMessage key={turn.id ?? ti} turn={turn} turnIndex={ti} index={index} blobUrl={blobUrl} />
      : <AgentMessage key={ti} turn={turn} index={index} compacting={compacting} running={running && ti === turns.length - 1 && !turn.stop} onPermission={onPermission} memoryKey={memoryKey} />);
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
