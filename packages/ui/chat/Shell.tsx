import { Fragment, memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type DragEvent, type ReactNode, type RefObject } from 'react';
import { flushSync } from 'react-dom';
import { Check, CircleAlert, Network, Paperclip, Settings2, X } from 'lucide-react';
import type { ExternalSessionInfo, AccountInfo, AgentId, AgentInfo, ForkOrigin, AuthMethodInfo, CategoryOp, Draft, FailureAction, FullDiffSource, GoalAction, NativeSessionInfo, PermissionBlock, PlanDocumentBlock, QuestionAnswers, QuestionBlock, QueuedPrompt, SessionCategories, SessionControls, SessionGoal, SessionStatus, SessionSummary, SlashCommand, Turn, Usage, UserTurn } from '@acpira/shared/transcript';
import type { SubagentSummary } from '@acpira/shared/subagents';
import type { ModelShapes } from '@acpira/shared/modelShapes';
import type { HiddenMap, SessionScope } from '@acpira/shared/settings';
import type { AccountAction, AddAccountVia, EditTurnRequest, ExportFormat, FileHit, NativeSessionsState, SessionHit, ShellCommand } from '@acpira/shared/protocol';
import { AppearanceContext, appearanceDataAttrs, type Appearance } from '../appearance';
import { lookAttrs, ThemeContext, type ShellLook, type Theme } from '../look';
import { t, useLocale } from '../i18n';
import { ShellLayerContext } from '../ui/Popover';
import { useShellLayer } from '../ui/Overlay';
import { cn } from '../ui/cn';
import { DockStack } from '../ui/DockStack';
import { Chip, IconButton } from '../ui/Button';
import { useScrollReveal } from '../ui/useScrollReveal';
import { useStableList } from '../ui/useStableList';
import { useMergedRefs } from '../ui/mergeRefs';
import { QuietEntranceContext } from '../ui/Row';
import { FollowContext, useBottomFollow } from './useBottomFollow';
import { captureSliceScroll, restoreSliceScroll, type SliceScroll } from './sliceScroll';
import { Header } from './Header';
import { SessionList, projectName } from './SessionList';
import { isChatFolder, type MoveTarget } from './sessionTree';
import { AgentMessage } from './Turns';
import { HistoryComposerContext, HistoryContext, HistoryMessage } from './HistoryMessage';
import { QuoteToolbar } from './QuoteToolbar';
import { Composer, type ComposerProps } from './Composer';
import type { MentionPersona } from './Mention';
import { ConnectingNotice, Notice } from './Notice';
import { ExternalSessionNotice } from './ExternalSessionNotice';
import { Toast } from '../ui/Toast';
import { writeClipboardLater } from '../ui/useCopied';
import { sessionLink } from '@acpira/shared/sessionLinks';
import { sessionLinkBase } from './sessionLinkBase';
import type { SessionCopyKind } from './SessionMenu';
import { Alert, isShortStop } from './Alert';
import { Questions, type OnAnswer } from './Questions';
import { PlanBar } from './PlanBar';
import { PlanDocumentContext } from './PlanDocument';
import { pendingPlanReview, reviseOption } from './planReview';
import { planExecutionId } from '@acpira/shared/planExecution';
import { Queue } from './Queue';
import { PromptFooter } from './Goal';
import { lastTypedPrompt } from './goalText';
import { heldPrompt } from './heldPrompt';
import { compactingNow } from './usageBreakdown';
import { BUILTIN_AGENT_ID } from '@acpira/shared/providers';
import { imageMimeOf } from '@acpira/shared/attachments';
import { launchable } from '@acpira/shared/agentOrder';
import { LOCAL_RUN_AGENTS, parseRemoteFolder } from '@acpira/shared/remote';
import { connectionOf, useFolderHost } from '../remote/api';
import { RemoteFolderDialog } from '../remote/RemoteFolderDialog';
import { COLUMN_HIDDEN_KEY, DesktopContext, desktopOS, macOS, pageCaption, useShellCommands, useShellToasts } from '../desktop';
import { droppedPathResolver, hostDragPaths, setWebviewState, webviewState } from '../vscodeApi';
import { moveDraft } from './useComposerDraft';
import { ProjectPick, StartPicker, usePresence } from './StartPage';
import { AppMenu, NoProject, ShellToastItems, ShellToasts, TitleRow, TitleSpace, useSessionHistory, type QuickSettings } from './DesktopFrame';
import type { SettingsPage } from '../settings/Nav';
import { Lightbox } from './Lightbox';
import { OpenToolFileContext, AsyncTaskStopContext, BlobUrlContext, DiffSourceContext, OpenBlobContext, OpenSessionContext, SessionTitleContext, ToolFileActionsContext, resolveSessionPath, type ToolFileActions } from './fileLinks';
import { TurnActionsContext } from './TurnActions';
import { ForkBoundary } from './ForkBoundary';
import { SubagentInspector } from './subagents/SubagentInspector';
import { InspectorSplitter, clamp, fitPaneBounds, usePreferredPaneWidth, type PaneBounds } from './subagents/InspectorSplitter';
import { ToolPanelDock, openPlanInPanel, projectPath } from '../panel/ToolPanel';
import { preferMarkdownPreview } from '../panel/FileViewer';
import { markShellResize } from '../panel/api';
import { FilesLayout } from '../panel/FilesLayout';
import { panel, usePanelValue } from '../panel/store';
import { useChatPanel } from '../panel/chat';
import { SubagentGraph } from './subagents/SubagentGraph';
import { breadcrumb, nodesByTurn, subagentTitle } from './subagents/subagentState';
import { ComposerAbove } from '../plugins/ComposerAbove';
import { backdropsShown, useSkin } from '../plugins/skin';
import { useSessionOutline } from '../plugins/outline';
import { SessionRail, SlotPieces } from '../plugins/Pieces';

// Every action the webview sends to the host; in the LAB a fake host implements these, the real build swaps in postMessage
export interface ShellHandlers {
  editTurn?: (edit: EditTurnRequest) => Promise<void>;
  // steer: the composer sent it mid-turn with steering on, so it joins the running turn instead of queueing
  send: (text: string, attachments: Draft[], steer?: boolean) => void;
  // @ mention lookup over workspace files
  searchFiles: (query: string) => Promise<FileHit[]>;
  // History search over saved conversations; without it the session list matches titles only
  searchSessions?: (query: string) => Promise<SessionHit[]>;
  // The whole files behind the `nth` diff of a tool call: page views carry diff sources without their texts
  // subagentId: the diff sits in that subagent's transcript (the inspector), not in the session's own turns
  diffSource?: (sessionId: string, toolCallId: string, nth: number, subagentId?: string) => Promise<FullDiffSource | undefined>;
  stop: () => void;
  permission: (sessionId: string, blockId: string, optionId: string) => void;
  // The question card was closed: answers keyed by question id, or skip
  answer?: (sessionId: string, blockId: string, answers: QuestionAnswers, skip?: boolean) => void;
  buildPlan?: (sessionId: string, planId: string, model?: { configId: string; value: string }, optionId?: string) => void;
  openPlan?: (sessionId: string, planId: string) => void;
  openFile?: (sessionId: string, path: string, line?: number, reveal?: boolean) => void;
  // Stage a local/remote image file into the session's blob store for the Lightbox
  imageBlob?: (sessionId: string, path: string) => Promise<string | undefined>;
  // An agent-emitted image's blob file opens in the editor by its store name (the host resolves the absolute path)
  openBlob?: (sessionId: string, name: string) => void;
  setMode: (id: string) => void;
  setConfig: (configId: string, value: string) => void;
  selectSession: (id: string) => void;
  // Without an agent the host falls back to the configured defaultAgent; with a category the new session is filed there;
  // `cwd` (a project shell, the desktop app) starts it in that folder; `chat` in a fresh folder of its own (no project)
  newSession: (agent?: AgentInfo['id'], category?: string, cwd?: string, chat?: boolean) => void;
  renameSession: (id: string, title: string) => void;
  // Resolves only when deletion commits; true means this engine can undo it. Errors are shown by the host.
  deleteSession: (id: string) => Promise<boolean> | void;
  restoreSession: (id: string) => void;
  pinSession: (id: string, pinned: boolean) => void;
  // Project scoping of the list: re-home a session into another project (absent: this window's workspace folder), optionally
  // filed under one of that project's categories
  moveSession?: (id: string, to?: MoveTarget) => void;
  // User categories of the session list: file a session (null takes it out) / edit the shared category list
  setSessionCategory?: (id: string, category: string | null) => void;
  categoryOp?: (op: CategoryOp, file?: string) => void;
  // The list's view menu: forget every unread dot
  markAllRead?: () => void;
  // Account layer: selecting an account rebinds the current session; adding an account goes through import / terminal login; removing only deletes the locally saved credential
  selectAccount: (id: string) => void;
  addAccount: (agent: AgentInfo['id'], via: AddAccountVia) => void;
  removeAccount: (id: string) => void;
  // An account list opened: re-read the quotas of that agent's accounts
  refreshQuota?: (agent: AgentInfo['id']) => void;
  // AgentInfo.credentialsLocked: unlock the credential store in a host terminal
  unlockCredentials?: (agent: AgentInfo['id']) => void;
  compact: () => void;
  login: (methodId?: string) => void;
  retry: () => void;
  // Send the last user turn again after its agent turn ended in error
  retryTurn: () => void;
  // Drop the agent process and resume the same native session (a live connection whose prompts keep failing)
  // SessionView.canTakeOver: end the other Acpira instance's agent holding the session lock, then start over
  takeOver?: () => void;
  // Queued prompts: drop one / replace one in place (kept attachments by index plus new drafts)
  dequeue?: (sessionId: string, id: string) => void;
  sendQueued?: (sessionId: string, id: string) => void;
  // Inject a queued prompt into the running turn (`steerQueued` setting on an agent with SessionView.canSteer)
  steerQueued?: (sessionId: string, id: string) => void;
  editQueued?: (sessionId: string, id: string, text: string, retainedAttachments: number[], attachments: Draft[]) => void;
  // Start a new session whose transcript is this session's turns through the given agent turn
  // `agent`: the agent the fork continues on (the session's own for a plain fork)
  forkSession?: (sessionId: string, turnIndex: number, agent?: AgentId) => void;
  // Write the session as Markdown or JSON under the data dir's exports/; the toast that follows offers to open it
  exportSession?: (id: string, format: ExportFormat) => void;
  // The session as Markdown (the export's text, nothing written); rejects with the reason it could not be read
  sessionMarkdown?: (id: string) => Promise<string>;
  // "Copy" in the session menu; the Shell supplies it (clipboard write + toast), from sessionMarkdown for the Markdown
  copySession?: (id: string, what: SessionCopyKind) => void;
  // Open the session in an editor tab
  openInEditor?: (sessionId: string) => void;
  // History list's import popover: read the agent's own sessions / import one as a local record
  listNativeSessions?: (agent: AgentInfo['id']) => void;
  importNativeSession?: (agent: AgentInfo['id'], s: NativeSessionInfo) => void;
  // Subagent observation: the inspector subscribes to one child's transcript stream at a time
  observeSubagent?: (sessionId: string, subagentId: string) => void;
  unobserveSubagent?: (sessionId: string, subagentId: string) => void;
  cancelSubagent?: (sessionId: string, subagentId: string) => void;
  // Ask the adapter to stop one AIR async task on this session (host resolves the owning peer session)
  stopAsyncTask?: (sessionId: string, taskId: string) => void;
  // Goal extension controls (SessionView.goalActions); `set` carries the objective
  goal?: (sessionId: string, action: GoalAction, objective?: string) => void;
  // The desktop app: the system folder dialog (the folder picked, undefined when dismissed), a folder shown in the OS file
  // manager, a web page opened in the browser
  pickFolder?: () => Promise<string | undefined>;
  revealPath?: (path: string) => void;
  openExternal?: (url: string) => void;
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
  canTakeOver?: boolean;
  authMethods?: AuthMethodInfo[];
  turns: Turn[];
  running: boolean;
  queued?: QueuedPrompt[];
  // A fork's origin while its copied turns are still there (SessionView.forkedFrom): the thread draws the boundary under them
  forkedFrom?: ForkOrigin;
  // The session goal and the controls the agent advertised for it (SessionView.goal / goalActions)
  goal?: SessionGoal;
  goalActions?: GoalAction[];
  controls: SessionControls;
  // Remembered per-model parameters, so a model switch in the history editor shows that model's own
  modelShapes?: ModelShapes;
  usage?: Usage;
  // The slash commands the agent advertised for this session (available_commands_update): the composer's / menu,
  // and the context panel only gets a compact button when `compact` is among them
  commands?: SlashCommand[];
  compactAt?: number;
  // Settings: the editor's live selection is offered in the main composer
  shareEditorSelection?: boolean;
  // Settings `steerQueued` on an agent that can steer: a queued row's send button steers it into the running turn
  steerQueued?: boolean;
  // Settings → Subagents, the enabled ones: offered in the composer's @ list and summon menu
  personas?: MentionPersona[];
  sessions: SessionSummary[];
  // The session list's user categories (absent: the list shows no categories)
  categories?: SessionCategories;
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
  // Opens the settings page (a local view swap, not a host action — hence not part of ShellHandlers), at `page` when given
  onOpenSettings?: (page?: SettingsPage) => void;
  // For replaying the entrance animation: remounts the conversation when it changes
  replayKey?: number | string;
  // The session on screen is a kept copy shown at once on a switch, and the host's reply with what happened while the
  // reader was away has not landed yet: the thread shows that reply in place, without entrances or paced text
  catchingUp?: boolean;
  // The desktop app (packages/ui/desktop.ts): its own window frame, the start page of an empty session, the project registry
  desktop?: boolean;
  // Mounted on the way back from the settings page, whose rail stood where the column does: a column the window keeps
  // folded comes up open and folds away, instead of the sheet jumping to the window's edge
  fromSettings?: boolean;
  // Desktop: the theme and language the gear menu switches in place (chat/DesktopFrame.tsx AppMenu)
  quickSettings?: QuickSettings;
  // Desktop: the engine has no project to show a session of (noSession), so the sheet asks for a folder
  noProject?: boolean;
  // The user's home folder: the start page's project paths shorten it to ~
  home?: string;
  // Desktop: where chats' folders live (InitState.chats); a session in one belongs to no project
  chats?: string;
  // The staged sample session the interface tour plays over (welcome/SampleShell.tsx): its folder is not real, so the tool
  // panel, which would ask the engine about it, stays away
  sample?: boolean;
}

// A toast: text plus an optional undo; each dismisses itself, several can stack (an attachment notice must not take the undo of a deletion with it)
interface ToastState {
  key: string;
  text: string;
  icon?: ReactNode;
  undo?: () => void;
  ttl?: number;
}

// Chat and optional session column share one webview. Width determines docking in both host surfaces.
export function Shell(p: ShellProps) {
  const { appearance: a, on } = p;
  const wide = p.host === 'editor';
  const root = useRef<HTMLDivElement>(null);
  useShellLayer(root);
  const sessionPanel = useRef<HTMLElement>(null);
  const desktop = !!p.desktop;
  const { shown: skin, leaving: leavingSkin } = useSkin();
  // The desktop app has no position setting (its column folds from the title row), so a value written earlier cannot strand it
  const position = desktop ? 'left' : p.look?.sessionListPosition ?? (a.sessions === 'drawer' ? 'left' : 'hidden');
  const [canDock, setCanDock] = useState(false);
  // Desktop: what the window centres on (acpira.desktopLayout). The files layout (panel/FilesLayout.tsx) needs a project to
  // show, the built-in Files module enabled, and room for its four columns (--files-layout-min); short of any the window keeps the conversation layout
  const [canFiles, setCanFiles] = useState(false);
  const filesOn = p.look?.panelFiles !== false;
  const anyPanelModule = filesOn || p.look?.panelTerminal !== false || p.look?.panelBrowser !== false;
  const filesLayout = desktop && filesOn && p.look?.desktopLayout === 'files';
  const files = filesLayout && canDock && canFiles && !p.noProject && !p.external && !!p.cwd && !!p.activeSessionId;
  // Desktop: the column folded away by its title row's button (kept across reloads); `columnMoving` lets its width ease
  // only while it folds, never under the drag that resizes it. The files layout docks it at the right edge, where the
  // settings rail never stands, so coming back from settings it starts folded
  // The sample session always shows its column: the tour's first step lights it
  const [columnHidden, setColumnHidden] = useState(() => desktop && !p.sample && !(p.fromSettings && !filesLayout) && webviewState(COLUMN_HIDDEN_KEY) === true);
  const [columnMoving, setColumnMoving] = useState(false);
  useEffect(() => {
    if (!columnMoving) return;
    const timer = setTimeout(() => setColumnMoving(false), COLUMN_MOVE_MS);
    return () => clearTimeout(timer);
  }, [columnMoving]);
  // Observe the actual panel, not the browser window: IDE sidebars and split editor tabs resize independently.
  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const styles = getComputedStyle(element);
    const minWidth = Number.parseFloat(styles.getPropertyValue('--session-dock-min'));
    const filesMin = Number.parseFloat(styles.getPropertyValue('--files-layout-min'));
    let lastW = -1;
    const update = (entries?: ResizeObserverEntry[]) => {
      const w = entries?.[0]?.contentRect.width ?? element.clientWidth;
      if (lastW >= 0 && Math.abs(w - lastW) > 1.5) markShellResize();
      lastW = w;
      element.style.setProperty('--shell-w', `${w}px`);
      setCanDock(w >= minWidth);
      setCanFiles(w >= filesMin);
    };
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
    if (!el) return;
    // The dock survives a session switch; measure its new contents before the thread's first bottom pin.
    // Waiting for its observer gives the new thread the previous session's clearance for one painted frame.
    dockHeight.current = Math.round(planDock.current?.getBoundingClientRect().height ?? 0);
    el.style.paddingBottom = threadClearance(dockHeight.current);
    toastLayer.current?.style.setProperty('--thread-dock-height', `${dockHeight.current}px`);
  }, []);
  const toastRef = useCallback((el: HTMLDivElement | null) => {
    toastLayer.current = el;
    if (el) el.style.setProperty('--thread-dock-height', `${dockHeight.current}px`);
  }, []);
  useLayoutEffect(() => {
    const dock = planDock.current;
    if (!dock) return;
    const sync = (entries?: ResizeObserverEntry[]) => {
      const height = Math.round(entries?.[0]?.borderBoxSize?.[0]?.blockSize ?? dock.offsetHeight);
      if (height === dockHeight.current) return;
      dockHeight.current = height;
      const content = threadContent.current;
      if (content) content.style.paddingBottom = threadClearance(height);
      toastLayer.current?.style.setProperty('--thread-dock-height', `${height}px`);
    };
    sync();
    const observer = new ResizeObserver(sync);
    observer.observe(dock);
    return () => observer.disconnect();
  }, []);
  // The docked session column's width is the user's (drag its inner edge). CSS clamps it to the room the shell has, so a
  // window resize re-renders nothing; the drag reads the same bounds
  const [preferredSide, setPreferredSide] = usePreferredPaneWidth('sessionPaneW');
  const [draggedSide, setDraggedSide] = useState<number>();
  const sideBounds = useCallback((): PaneBounds => {
    const element = root.current ?? document.documentElement;
    const px = (name: string) => Number.parseFloat(getComputedStyle(element).getPropertyValue(name));
    return fitPaneBounds(sessionPanel.current, { min: px('--session-pane-min'), max: element.clientWidth - px('--session-main-min'), initial: px('--pop-w-md') });
  }, []);
  const sideWidth = draggedSide ?? preferredSide;
  const commitSide = useCallback((w: number) => { setDraggedSide(undefined); setPreferredSide(w); }, [setPreferredSide]);
  const resetSide = useCallback(() => { setDraggedSide(undefined); setPreferredSide(undefined); }, [setPreferredSide]);
  const hideColumn = useCallback((hidden: boolean) => {
    // While its width eases, the column's content keeps the width it has open (desktop.css), clipped instead of re-wrapped
    const b = sideBounds();
    const open = hidden ? sessionPanel.current?.offsetWidth : clamp(preferredSide ?? b.initial, b.min, b.max);
    if (open) sessionPanel.current?.style.setProperty('--column-w', `${open}px`);
    setColumnMoving(true);
    setColumnHidden(hidden);
    setWebviewState(COLUMN_HIDDEN_KEY, hidden || undefined);
  }, [sideBounds, preferredSide]);
  // Back from settings with the column kept folded: it mounted open at the rail's place, and folds once it has painted
  useEffect(() => {
    if (!desktop || !p.fromSettings || filesLayout || webviewState(COLUMN_HIDDEN_KEY) !== true) return;
    let frame = requestAnimationFrame(() => { frame = requestAnimationFrame(() => hideColumn(true)); });
    return () => cancelAnimationFrame(frame);
    // Mount only: later renders keep whatever the title row's toggle chose
  }, []);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const drawerOpenNow = useRef(drawerOpen);
  drawerOpenNow.current = drawerOpen;
  // Desktop: a narrow window's list page slides only while the user opens or closes it (desktop.css data-drawer-moving),
  // never as the window crosses the width tier. A counter, so a second move restarts the clock instead of cutting it short
  const [drawerMoving, setDrawerMoving] = useState(0);
  useEffect(() => {
    if (!drawerMoving) return;
    const timer = setTimeout(() => setDrawerMoving(0), DRAWER_MOVE_MS);
    return () => clearTimeout(timer);
  }, [drawerMoving]);
  const openDrawer = useCallback(() => {
    if (drawerOpenNow.current) return;
    setDrawerMoving(m => m + 1);
    setDrawerOpen(true);
  }, []);
  // `slide`: the user closed it; the width tier closes it in place
  const closeDrawer = useCallback((slide = true) => {
    if (!drawerOpenNow.current) return;
    if (sessionPanel.current?.contains(document.activeElement)) {
      root.current?.querySelector<HTMLButtonElement>('[data-session-toggle]')?.focus();
    }
    if (slide) setDrawerMoving(m => m + 1);
    setDrawerOpen(false);
  }, []);
  // A tier crossing (or a moved list) closes it in place and ends the motion window: a crossing back within it would slide
  // the page out again
  useEffect(() => {
    closeDrawer(false);
    setDrawerMoving(0);
  }, [position, canDock, closeDrawer]);
  // Desktop: Esc closes a narrow window's list page wherever the focus is (inside it, the list's own handler takes the key
  // first; an open menu stops it before it gets here). The key is consumed: Electron hands a key the page leaves alone to
  // the macOS menu bar, where a synthetic Esc crashed the app in the input method code (the smoke, 2026-10-23)
  useEffect(() => {
    if (!desktop || canDock || !drawerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || e.isComposing) return;
      e.preventDefault();
      closeDrawer();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [desktop, canDock, drawerOpen, closeDrawer]);
  useEffect(() => {
    if (drawerOpen && !canDock) sessionPanel.current?.querySelector<HTMLInputElement>('input')?.focus();
  }, [drawerOpen, canDock]);
  const [editing, setEditing] = useState<{ sessionId: string; index: number }>();
  useEffect(() => setEditing(undefined), [p.activeSessionId]);
  // The subagent inspector: one open child at a time, docked beside the column when it is wide enough.
  // The docked pane's width is the user's (drag its left edge), clamped by CSS so the conversation keeps --subagent-main-min.
  const mainColumn = useRef<HTMLDivElement>(null);
  const inspectorPane = useRef<HTMLElement>(null);
  const [canDockInspector, setCanDockInspector] = useState(false);
  const [preferredPane, setPreferredPane] = usePreferredPaneWidth();
  const [draggedPane, setDraggedPane] = useState<number>();
  useLayoutEffect(() => {
    const element = mainColumn.current;
    if (!element) return;
    const minWidth = Number.parseFloat(getComputedStyle(element).getPropertyValue('--subagent-dock-min'));
    // Hysteresis: once docked, the column lost the pane's width — count it back so the panel does not flap open/closed
    const update = (entries?: ResizeObserverEntry[]) => {
      const w = entries?.[0]?.contentRect.width ?? element.clientWidth;
      const room = w + (inspectorPane.current?.offsetWidth ?? 0);
      setCanDockInspector(room >= minWidth);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const paneBounds = useCallback((): PaneBounds => {
    const styles = getComputedStyle(mainColumn.current ?? document.documentElement);
    const px = (name: string) => Number.parseFloat(styles.getPropertyValue(name));
    const room = (mainColumn.current?.clientWidth ?? 0) + (inspectorPane.current?.offsetWidth ?? 0);
    return fitPaneBounds(inspectorPane.current, { min: px('--subagent-pane-min'), max: room - px('--subagent-main-min'), initial: px('--subagent-pane-w') });
  }, []);
  const paneWidth = draggedPane ?? preferredPane;
  // Desktop, files layout: the chat's width is the user's (drag its left edge, over the stage's rim), kept with the tool
  // panel's state; the stage keeps --stage-min of the room the two share
  const chatW = usePanelValue(s => s.chatW);
  const [draggedChat, setDraggedChat] = useState<number>();
  const chatBounds = useCallback((): PaneBounds => {
    const element = mainColumn.current ?? document.documentElement;
    const px = (name: string) => Number.parseFloat(getComputedStyle(element).getPropertyValue(name));
    const stage = root.current?.querySelector<HTMLElement>('[data-stage]');
    return fitPaneBounds(mainColumn.current, { min: px('--files-chat-min'), max: Math.min(px('--files-chat-max'), element.offsetWidth + (stage?.offsetWidth ?? 0) - px('--stage-min')), initial: px('--files-chat-w') }, px('--files-chat-max'));
  }, []);
  const chatWidth = draggedChat ?? chatW;
  const commitChat = useCallback((w: number) => { setDraggedChat(undefined); panel.setChatW(w); }, []);
  const resetChat = useCallback(() => { setDraggedChat(undefined); panel.setChatW(undefined); }, []);
  // Files layout: the title row's toggle folds the explorer, the pinned one at the top-right corner (⌥⌘B) the session column
  const explorerHidden = usePanelValue(s => s.explorerHidden);
  // Conversation layout: the tool panel docked open beside the chat (ToolPanelDock's own condition, wide window)
  const panelDocked = usePanelValue(s => s.open && (s.files || s.terminal || s.browser)) && desktop && canDock && !files && !p.noProject && !p.external && !p.sample && !!p.cwd && !!p.activeSessionId;
  const toggleSessions = useCallback(() => hideColumn(!columnHidden), [hideColumn, columnHidden]);
  const [inspect, setInspect] = useState<{ id: string }>();
  const [graphOpen, setGraphOpen] = useState(false);
  useEffect(() => setGraphOpen(false), [p.activeSessionId]);
  useEffect(() => setInspect(undefined), [p.activeSessionId]);
  const inspectNode = inspect !== undefined ? p.subagents?.find(n => n.id === inspect.id) : undefined;
  // Desktop docking follows the window tier, not the room left after opening another pane.
  // The inspector and the other columns share that room without replacing the main conversation.
  const dockInspector = desktop ? canDock : canDockInspector;
  const overlayInspect = inspectNode !== undefined && !dockInspector;
  // Wide enough: the child docks beside the column. The shell carries it as data-subagent-docked for desktop.css and
  // panel.css: a :has() over the pane's presence kept matching in the desktop app's WKWebView after the pane was
  // removed, which left the header without the pinned panel toggle's room
  const dockedInspect = inspectNode !== undefined && dockInspector && p.activeSessionId !== undefined;
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
  // Only this request's completion can offer undo: another engine's list change is not a deletion receipt.
  // Refused deletions show the host's error instead. Refused attachments share this stack.
  const [toasts, setToasts] = useState<ToastState[]>([]);
  const deleting = useRef(new Set<string>());
  // The desktop shell's own toasts share the stack (packages/ui/desktop.ts)
  const shellToasts = useShellToasts();
  const dropToast = useCallback((key: string) => setToasts(ts => ts.filter(t => t.key !== key)), []);
  const pushToast = useCallback((t: ToastState) => setToasts(ts => [...ts.filter(x => x.key !== t.key), t]), []);
  useEffect(() => {
    const pending = deleting.current;
    return () => pending.clear();
  }, []);
  const notice = useCallback((text: string) => pushToast({ key: `n${Date.now()}`, text, icon: <Paperclip className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} /> }), [pushToast]);
  // A copy's confirmation, or why it failed. One toast slot: a fresh key per copy restarts the toast's timer and the
  // previous confirmation goes
  const copied = useCallback((write: Promise<unknown>, text: string) => {
    const show = (toast: Omit<ToastState, 'key'>) => setToasts(ts => [...ts.filter(x => !x.key.startsWith('copy:')), { key: `copy:${Date.now()}`, ...toast }]);
    void write.then(
      () => show({ text, icon: <Check className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />, ttl: 2000 }),
      (e: unknown) => show({ text: t('session.copyFailed', { detail: e instanceof Error ? e.message : String(e) }), icon: <CircleAlert className="size-icon shrink-0 text-danger" strokeWidth={1.5} /> }),
    );
  }, []);
  const handlers = useMemo<ShellHandlers>(() => ({
    ...on,
    deleteSession: async id => {
      if (deleting.current.has(id)) return false;
      const target = p.sessions.find(s => s.id === id);
      deleting.current.add(id);
      try {
        const undo = await on.deleteSession(id) === true;
        // An unsent, unlisted session has no saved record to undo. Unmounting also discards the pending toast.
        if (undo && target && deleting.current.has(id)) {
          const title = target.title || t('session.fallbackTitle');
          pushToast({ key: id, text: t('session.deleted', { title }), undo: () => { on.restoreSession(id); dropToast(id); } });
        }
        return undo;
      } finally {
        deleting.current.delete(id);
      }
    },
    // The link and the folder are written inside the click; the Markdown is handed over as pending text (writeClipboardLater)
    copySession: (id, what) => {
      const cwd = p.sessions.find(s => s.id === id)?.cwd || (id === p.activeSessionId ? p.cwd : undefined);
      const write = what === 'link' ? navigator.clipboard.writeText(sessionLink(id, sessionLinkBase()))
        : what === 'cwd' ? (cwd ? navigator.clipboard.writeText(cwd) : Promise.reject(new Error('no working directory')))
        : on.sessionMarkdown ? writeClipboardLater(() => on.sessionMarkdown!(id)) : Promise.reject(new Error('unavailable'));
      copied(write, t(`session.copied.${what}`));
    },
    // A moved session leaves a "this project" list at once (or changes group under "all"); the toast says where it went
    moveSession: on.moveSession && ((id, to) => {
      const title = p.sessions.find(s => s.id === id)?.title ?? t('session.fallbackTitle');
      on.moveSession!(id, to);
      pushToast({ key: `m${id}`, text: to && to.cwd !== p.workspace ? t('session.movedTo', { title, project: projectName(to.cwd) }) : t('session.moved', { title }) });
    }),
  }), [on, p.sessions, p.activeSessionId, p.cwd, pushToast, dropToast, copied]);
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
  // The session waits on the user: a question, or a permission request in the last reply (what plugin pieces hear)
  const waitingOnUser = !!question || (lastTurn?.role === 'agent' && lastTurn.blocks.some(b => b.type === 'permission'));
  const questionSubtitle = !rootQuestion && childQuestion
    ? t('subagents.provenance', { path: breadcrumb(childQuestion.id, p.subagents ?? []).map(n => subagentTitle(n, t)).join(' › ') })
    : undefined;
  // The desktop app's project shell. "New session" opens the start page in a project (the one on screen by default)
  // with the agent last used; a folder picked in the dialog or dropped on the column joins the projects and opens there.
  // The start page shows while the session on screen has not said a word
  const startPage = desktop && !p.noProject && !p.external && !!p.activeSessionId && !p.turns.length && !p.queued?.length;
  const currentAgent = launchable(p.agents).some(x => x.id === p.agent.id && x.available !== false) ? p.agent.id : undefined;
  // Under local run only the built-in agent and Claude Code work in a folder on an SSH host (LOCAL_RUN_AGENTS; the engine
  // refuses others with host.remoteBuiltinOnly), so a session started there keeps one of them and otherwise takes the
  // first that is installed; under remote run the host's own engine runs them all, and a host the list does not know yet
  // counts as local run, the default
  const localReady = LOCAL_RUN_AGENTS.find(id => p.agents.some(a => a.id === id && a.available !== false));
  const agentIn = useCallback((cwd: string | undefined, agent: AgentId | undefined) => {
    const remote = cwd ? parseRemoteFolder(cwd) : undefined;
    if (!remote || connectionOf(remote.alias)?.mode === 'remote' || !localReady) return agent;
    return agent && LOCAL_RUN_AGENTS.includes(agent) ? agent : localReady;
  }, [localReady]);
  const startHost = useFolderHost(startPage ? p.cwd : undefined);
  const startOnly = startHost && startHost.view?.mode !== 'remote' && localReady ? [...LOCAL_RUN_AGENTS] : undefined;
  const [fresh, setFresh] = useState<string>();
  useEffect(() => {
    if (!fresh) return;
    const timer = setTimeout(() => setFresh(undefined), FRESH_MS);
    return () => clearTimeout(timer);
  }, [fresh]);
  // A folded project opens, so the session starting there stays in sight
  const unfold = useCallback((cwd: string) => { if (p.categories?.collapsedProjects.includes(cwd)) on.categoryOp?.({ op: 'collapseProject', cwd, collapsed: false }); }, [p.categories, on.categoryOp]);
  const openProject = useCallback((cwd: string) => {
    on.categoryOp?.({ op: 'addProject', cwd });
    unfold(cwd);
    on.newSession(agentIn(cwd, currentAgent), undefined, cwd);
    setFresh(cwd);
    closeDrawer();
  }, [on.categoryOp, on.newSession, unfold, currentAgent, agentIn, closeDrawer]);
  const addProject = useCallback(() => { void on.pickFolder?.().then(dir => { if (dir) openProject(dir); }); }, [on.pickFolder, openProject]);
  // The column's "remote folder" entries: a folder on that host through the remote folder picker, or a new host on the
  // settings page
  const [remotePick, setRemotePick] = useState<string>();
  const openSettingsPage = p.onOpenSettings;
  const addRemote = useCallback((alias?: string) => {
    closeDrawer();
    if (alias) setRemotePick(alias);
    else openSettingsPage?.({ kind: 'connections' });
  }, [closeDrawer, openSettingsPage]);
  // A chat: a session in a folder of its own that belongs to no project. On a chat's start page already, the composer
  // takes the focus instead
  const chatHere = desktop && isChatFolder(p.chats, p.cwd);
  // An empty chat's folder is no use in the tool panel: fold it there (panel/chat.ts)
  useChatPanel(desktop ? p.chats : undefined, chatHere, p.activeSessionId);
  const newChat = useCallback(() => {
    closeDrawer();
    if (startPage && chatHere) { root.current?.querySelector<HTMLTextAreaElement>('[data-start-dock] textarea')?.focus(); return; }
    on.newSession(currentAgent, undefined, undefined, true);
  }, [closeDrawer, startPage, chatHere, on.newSession, currentAgent]);
  // "New session": in the given project, else the one on screen; from a chat it is another chat, and with no project at
  // all a chat too where the engine makes them, the folder dialog elsewhere
  const newIn = useCallback((cwd?: string) => {
    const dir = cwd ?? p.cwd;
    if (!cwd && (chatHere || (!dir && p.chats))) { newChat(); return; }
    closeDrawer();
    if (!dir) { addProject(); return; }
    // Already on that project's start page: the composer takes the focus instead
    if (startPage && dir === p.cwd) { root.current?.querySelector<HTMLTextAreaElement>('[data-start-dock] textarea')?.focus(); return; }
    unfold(dir);
    on.newSession(agentIn(dir, currentAgent), undefined, dir);
  }, [p.cwd, p.chats, chatHere, newChat, closeDrawer, addProject, startPage, unfold, on.newSession, currentAgent, agentIn]);
  const newHere = useCallback(() => newIn(), [newIn]);
  // The start page starts its empty session over with another agent or in another project (the engine drops the empty one
  // it leaves); what was typed there moves to the new one before its composer mounts
  const carry = useRef<string | undefined>(undefined);
  const startOver = (agent: AgentId, cwd = p.cwd) => {
    carry.current = p.activeSessionId;
    // A chat starts over as a new chat: the engine takes the empty one's folder away with it
    if (chatHere && cwd === p.cwd) { on.newSession(agent, undefined, undefined, true); return; }
    if (cwd) unfold(cwd);
    on.newSession(agent, undefined, cwd);
  };
  const startChat = () => { carry.current = p.activeSessionId; on.newSession(p.agent.id, undefined, undefined, true); };
  if (carry.current && p.activeSessionId && carry.current !== p.activeSessionId) {
    if (!p.turns.length) {
      moveDraft(carry.current, p.activeSessionId);
      panel.moveSession(carry.current, p.activeSessionId);
    }
    carry.current = undefined;
  }
  const start = usePresence(startPage);
  // The start page's projects: the column's registered ones, and the folder on screen if it is not among them
  const startProjects = useMemo(() => {
    const list = p.categories?.projects ?? [];
    return p.cwd && !chatHere && !list.includes(p.cwd) ? [...list, p.cwd] : list;
  }, [p.categories, p.cwd, chatHere]);
  const copyPath = useCallback((path: string) => copied(navigator.clipboard.writeText(path), t('session.project.copied')), [copied]);
  const newFromPanel = useCallback((agent?: AgentId) => { on.newSession(agent); closeDrawer(); }, [on.newSession, closeDrawer]);
  const newInCat = useCallback((category: string) => { on.newSession(p.agent.id, category); closeDrawer(); }, [on.newSession, p.agent.id, closeDrawer]);
  const toggleDrawer = useCallback(() => (drawerOpenNow.current ? closeDrawer() : openDrawer()), [closeDrawer, openDrawer]);
  const trail = useSessionHistory(desktop ? p.activeSessionId : undefined, id => p.sessions.some(s => s.id === id), id => on.selectSession(id));
  const focusSearch = () => {
    const focus = () => sessionPanel.current?.querySelector<HTMLInputElement>('input')?.focus();
    // A narrow window has a drawer, not the docked column: the column's folded flag does not apply there
    if (!canDock) { if (drawerOpen) focus(); else openDrawer(); }
    else if (columnHidden) { hideColumn(false); requestAnimationFrame(focus); }
    else focus();
  };
  // ⌘N new session, ⌘K search, ⌘O open a folder, ⌘, settings, ⌘[ / ⌘] back and forward (Ctrl elsewhere); the latest
  // closures through a ref. A narrow window's title row has no back and forward buttons, so the keys are their way there
  // ⌥⌘B: the tool panel, or the session column at the right edge in the files layout
  const rightSide = files ? toggleSessions : () => panel.toggle();
  const keys = useRef({ newIn, addProject, focusSearch, rightSide, back: trail.back, forward: trail.forward });
  keys.current = { newIn, addProject, focusSearch, rightSide, back: trail.back, forward: trail.forward };
  const openSettings = p.onOpenSettings;
  useEffect(() => {
    if (!desktop) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(macOS ? e.metaKey : e.ctrlKey) || e.altKey || e.shiftKey || e.isComposing) return;
      const key = e.key.toLowerCase();
      const step = key === '[' ? keys.current.back : key === ']' ? keys.current.forward : undefined;
      if (key === 'n') keys.current.newIn();
      else if (key === 'k') keys.current.focusSearch();
      else if (key === 'o' && on.pickFolder) keys.current.addProject();
      else if (key === ',' && openSettings) openSettings();
      else if (step) step();
      else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [desktop, on.pickFolder, openSettings]);
  // The menu bar's commands do what the shortcuts do (settings is App's: it switches the page)
  const runCommand = useCallback((command: ShellCommand) => {
    if (command === 'newSession') keys.current.newIn();
    else if (command === 'search') keys.current.focusSearch();
    else if (command === 'openFolder') keys.current.addProject();
    else if (command === 'toolPanel') keys.current.rightSide();
    else if (command === 'browser') panel.openBrowser();
  }, []);
  useShellCommands(desktop ? runCommand : undefined);
  // A folder dragged in from the OS onto the column joins the projects (the first one opens); only a drop carries the
  // entries, so the frame shows for any file dragged over
  const [folderOver, setFolderOver] = useState(false);
  const pathForFile = desktop ? droppedPathResolver() : undefined;
  // Linux: the drag's files exist only as the shell's paths (VsCodeApi.dragPaths)
  const dragPaths = desktop ? hostDragPaths() : undefined;
  const carriesFiles = (dt: DataTransfer) => dt.types.includes('Files') || !!dragPaths?.().length;
  const folderDrop = pathForFile && {
    onDragOver: (e: DragEvent) => {
      if (!carriesFiles(e.dataTransfer)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';
      if (!folderOver) setFolderOver(true);
    },
    onDragLeave: (e: DragEvent) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFolderOver(false); },
    onDrop: (e: DragEvent) => {
      if (!carriesFiles(e.dataTransfer)) return;
      e.preventDefault();
      setFolderOver(false);
      // The entries are only readable during the drop; their paths may come later (the Windows shell answers async)
      const listed = dragPaths?.() ?? [];
      const asked = listed.length ? listed.filter(d => d.endsWith('/')).map(d => d.slice(0, -1)) : [...e.dataTransfer.items].flatMap(item => {
        const file = item.kind === 'file' && item.webkitGetAsEntry()?.isDirectory ? item.getAsFile() : null;
        return file ? [pathForFile(file)] : [];
      });
      void Promise.all(asked).then(paths => {
        const dirs = paths.filter((d): d is string => !!d);
        for (const dir of dirs) on.categoryOp?.({ op: 'addProject', cwd: dir });
        if (dirs[0]) openProject(dirs[0]);
      });
    },
  };

  // On the left the column is the app's navigation (Codex / Claude desktop): new session, search and import lead it,
  // settings sit at its foot, and the chat header drops its own plus and settings while the column is docked. The files
  // layout docks it at the right edge with the same rows and keeps the gear at the bottom-left, under the explorer
  const nav = position === 'left';
  const gear = useMemo(() => (desktop && p.onOpenSettings
    ? <AppMenu onOpenSettings={p.onOpenSettings} onOpenExternal={on.openExternal} quick={p.quickSettings} />
    : undefined), [desktop, p.onOpenSettings, on.openExternal, p.quickSettings]);
  const openSession = useCallback((id: string) => { on.selectSession(id); closeDrawer(); }, [on.selectSession, closeDrawer]);
  const sessionsPanel = (
    <SessionList
      fill
      nav={nav}
      onNew={newFromPanel}
      sessions={p.sessions}
      agents={p.agents}
      activeId={p.activeSessionId}
      workspace={p.workspace}
      // A project shell lists every project (the engine reads its scope as `all` too)
      scope={desktop ? 'all' : p.sessionScope}
      onSelect={openSession}
      onRename={on.renameSession}
      onDelete={handlers.deleteSession}
      onPin={on.pinSession}
      onMove={handlers.moveSession}
      onExport={on.exportSession}
      onCopy={handlers.copySession}
      activeAgent={p.agent.id}
      nativeSessions={p.nativeSessions}
      onListNative={on.listNativeSessions}
      onImportNative={on.importNativeSession}
      onSearch={on.searchSessions}
      categories={p.categories}
      onFile={on.setSessionCategory}
      onCategoryOp={on.categoryOp}
      onNewInCategory={newInCat}
      onNewIn={desktop ? newIn : undefined}
      onAddProject={desktop && on.pickFolder ? addProject : undefined}
      onAddRemote={desktop ? addRemote : undefined}
      onRevealPath={on.revealPath}
      onCopyPath={copyPath}
      fresh={fresh}
      onMarkAllRead={on.markAllRead}
      chats={desktop ? p.chats : undefined}
      onNewChat={desktop && p.chats ? newChat : undefined}
    />
  );

  // A prompt the session itself holds back (starting, replaying remembered controls) shows as sent, not as a queued row
  const held = useMemo(() => heldPrompt(p.turns, p.running, p.status, p.queued), [p.turns, p.running, p.status, p.queued]);
  // With steering on, a message sent mid-turn joins that turn right away; a queued row's button steers the rest
  const steering = !!p.steerQueued && p.running;
  // A plan card waiting on the user turns the composer into its revision box, as Cursor's "Steer the plan": a send answers
  // the card's Revise (Claude, Codex and the built-in agent then end the turn) and queues the words as the next prompt,
  // so they reach the agent after the rejection instead of being steered into a turn that is blocked on the card
  const review = useMemo(() => pendingPlanReview(p.turns), [p.turns]);
  // Plain ids keep the composer props memo stable between stream pushes
  const reviseBlock = review?.id;
  const reviseChoice = review && reviseOption(review)?.id;
  const turnCount = p.turns.length;
  const isCompacting = useMemo(() => compactingNow(p.turns, p.running), [p.turns, p.running]);
  // Enabled persona names for the sent messages' summon pills; memoized so the HistoryMessage memo chain holds
  const summons = useMemo(() => p.personas?.map(x => x.name), [p.personas]);
  const composerProps: ComposerProps = useMemo(() => ({
    running: p.running, steer: steering, revisePlan: !!reviseChoice, disabled: p.status !== 'ready' && p.status !== 'starting',
    controlsLocked: p.status !== 'ready',
    theme: p.theme, turnCount, compacting: isCompacting, controls: p.controls, hidden: p.hidden?.[p.agent.id],
    usage: p.usage, commands: p.commands, compactAt: p.compactAt, cwd: p.cwd ?? '',
    onSend: reviseBlock && reviseChoice && p.activeSessionId ? (text: string, attachments: Draft[]) => {
      on.permission(p.activeSessionId!, reviseBlock, reviseChoice);
      on.send(text, attachments);
    } : steering ? (text: string, attachments: Draft[]) => on.send(text, attachments, true) : on.send, onSearchFiles: on.searchFiles, onNotice: notice, onStop: on.stop,
    onSetMode: on.setMode, onSetConfig: on.setConfig, onCompact: on.compact, personas: p.personas,
    onAddModel: p.agent.id === BUILTIN_AGENT_ID && p.onOpenSettings ? () => p.onOpenSettings!({ kind: 'agent', id: BUILTIN_AGENT_ID }) : undefined,
  }), [p.running, steering, reviseBlock, reviseChoice, p.activeSessionId, on.permission, p.status, p.theme, turnCount, isCompacting, p.controls, p.hidden, p.agent.id, p.usage, p.commands, p.compactAt, p.cwd, on.send, on.searchFiles, notice, on.stop, on.setMode, on.setConfig, on.compact, p.personas, p.onOpenSettings]);
  // Context values above the transcript must not change on every stream push: React walks the whole memoized tree for consumers each time
  const permissions = useStableList(useMemo(() => p.turns.flatMap(t => t.role === 'agent' ? t.blocks.filter((b): b is PermissionBlock => b.type === 'permission') : []), [p.turns]));
  const plans = useStableList(useMemo(() => p.turns.flatMap(t => t.role === 'agent' ? t.blocks.filter((b): b is PlanDocumentBlock => b.type === 'plan_document') : []), [p.turns]));
  const plansRef = useRef(plans);
  plansRef.current = plans;
  // Desktop: a file of the session's project (or a plan document) opens in the tool panel (panel/ToolPanel.tsx) when the
  // built-in Files module is enabled, anything else in the editor
  const panelRoot = desktop && anyPanelModule && !p.noProject && !p.external && !p.sample ? p.cwd : undefined;
  const filePanelRoot = filesOn ? panelRoot : undefined;
  useEffect(() => {
    if (filePanelRoot && plans.length) panel.syncPlans(filePanelRoot, plans);
  }, [filePanelRoot, plans]);
  const openPlanInEditor = useMemo(() => p.activeSessionId && on.openPlan
    ? (id: string) => on.openPlan!(p.activeSessionId!, id) : undefined, [p.activeSessionId, on.openPlan]);
  const planDoc = useMemo(() => ({
    controls: p.controls, hidden: p.hidden?.[p.agent.id], running: p.running, ready: p.status === 'ready', theme: p.theme, permissions,
    build: p.activeSessionId && on.buildPlan ? (id: string, model?: { configId: string; value: string }, optionId?: string) => on.buildPlan!(p.activeSessionId!, id, model, optionId) : undefined,
    open: p.activeSessionId && (filePanelRoot || on.openPlan)
      ? (plan: PlanDocumentBlock) => {
          if (filePanelRoot) openPlanInPanel(filePanelRoot, plan);
          else on.openPlan?.(p.activeSessionId!, plan.id);
        }
      : undefined,
  }), [p.controls, p.hidden, p.agent.id, p.running, p.status, p.theme, permissions, p.activeSessionId, on.buildPlan, on.openPlan, filePanelRoot]);
  // Stable across stream pushes (every prompt card subscribes); the composer props go through their own context to the open editor
  // Editable while the agent answers too (the edit stops that turn first); a held prompt's synthetic turn is not in the
  // engine's transcript yet, so nothing is editable while one stands in
  const editable = !composerProps.disabled && (p.running || !held.running);
  const history = useMemo(() => on.editTurn && p.activeSessionId ? {
    sessionId: p.activeSessionId, edit: on.editTurn, editable, shapes: p.modelShapes,
    editing: editing?.sessionId === p.activeSessionId ? editing.index : undefined,
    select: (index: number | undefined) => setEditing(current => index === undefined
      ? current?.sessionId === p.activeSessionId ? undefined : current
      : { sessionId: p.activeSessionId!, index }),
  } : undefined, [on.editTurn, p.activeSessionId, editable, editing, p.modelShapes]);
  // Session tool rows name the session they read by its title: keyed by id and by the agent's own session id. An untitled
  // one reads in the current language, so the map is rebuilt when it changes; a ref keeps the context callback stable
  // when unread/running flags change on the session list.
  const locale = useLocale();
  const titlesRef = useRef<Map<string, string>>(new Map());
  titlesRef.current = useMemo(() => {
    const titles = new Map<string, string>();
    for (const s of p.sessions) {
      const title = s.title || t('session.fallbackTitle');
      titles.set(s.id, title);
      if (s.acpSessionId && !titles.has(s.acpSessionId)) titles.set(s.acpSessionId, title);
    }
    return titles;
  }, [p.sessions, locale]);
  const sessionTitle = useCallback((ref: string) => titlesRef.current.get(ref), []);
  const [filePreview, setFilePreview] = useState<{ src: string; name?: string; mimeType?: string; path: string } | null>(null);
  useEffect(() => { setFilePreview(null); }, [p.activeSessionId]);
  const openInEditor = useMemo(() => p.activeSessionId && on.openFile
    ? (path: string, line?: number) => on.openFile!(p.activeSessionId!, path, line) : undefined, [p.activeSessionId, on.openFile]);
  const previewImage = useMemo(() => {
    const sid = p.activeSessionId;
    if (!sid || !blobUrl) return undefined;
    return (path: string, blob?: string, mimeType?: string) => {
      const name = path.split(/[\\/]/).pop() || t('common.image');
      const mime = mimeType ?? imageMimeOf(path);
      if (blob) {
        setFilePreview({ src: blobUrl(blob), name, mimeType: mime, path });
        return;
      }
      if (!on.imageBlob) {
        on.openFile?.(sid, path);
        return;
      }
      void on.imageBlob(sid, path).then(staged => {
        if (staged) setFilePreview({ src: blobUrl(staged), name, mimeType: mime, path });
        else on.openFile?.(sid, path);
      });
    };
  }, [p.activeSessionId, blobUrl, on.imageBlob, on.openFile]);
  const openToolFile = useMemo(() => {
    const sid = p.activeSessionId;
    if (!sid || (!on.openFile && !filePanelRoot && !previewImage)) return undefined;
    return (path: string, line?: number) => {
      if (line === undefined && previewImage && imageMimeOf(path)) {
        previewImage(path);
        return;
      }
      const resolved = resolveSessionPath(path, undefined, p.home);
      const rel = filePanelRoot ? projectPath(filePanelRoot, resolved) : undefined;
      if (filePanelRoot && rel !== undefined) {
        if (/\.plan\.md$/i.test(rel)) preferMarkdownPreview();
        panel.openFile(filePanelRoot, rel, line);
        return;
      }
      const plan = filePanelRoot ? plansRef.current.find(b => b.path === path || b.path === resolved) : undefined;
      if (filePanelRoot && plan) { openPlanInPanel(filePanelRoot, plan, line); return; }
      if (filePanelRoot) {
        if (/\.plan\.md$/i.test(resolved)) preferMarkdownPreview();
        panel.openFile(filePanelRoot, resolved, line);
        return;
      }
      on.openFile?.(sid, path, line);
    };
  }, [p.activeSessionId, on.openFile, filePanelRoot, previewImage, p.home]);
  const activeCwdRef = useRef<string | undefined>(undefined);
  activeCwdRef.current = p.sessions.find(s => s.id === p.activeSessionId)?.cwd || p.cwd;
  const homeRef = useRef(p.home);
  homeRef.current = p.home;
  const toolFileActions = useMemo<ToolFileActions | undefined>(() => {
    const sid = p.activeSessionId;
    if (!sid) return undefined;
    return {
      openInEditor,
      revealInOS: on.openFile ? (path: string) => on.openFile!(sid, path, undefined, true) : undefined,
      copyPath: (path: string) => {
        const raw = activeCwdRef.current ? parseRemoteFolder(activeCwdRef.current)?.path ?? activeCwdRef.current : undefined;
        copied(navigator.clipboard.writeText(resolveSessionPath(path, raw, homeRef.current)), t('session.project.copied'));
      },
      previewImage,
    };
  }, [p.activeSessionId, openInEditor, on.openFile, copied, previewImage]);
  const goalAction = useMemo(() => p.activeSessionId && on.goal
    ? (action: GoalAction) => on.goal!(p.activeSessionId!, action) : undefined, [p.activeSessionId, on.goal]);
  // "Set as goal" under the last prompt: only where the agent takes `set` and no goal is running yet
  const setGoal = useMemo(() => p.activeSessionId && on.goal && !p.external && !p.goal && p.goalActions?.includes('set')
    ? (objective: string) => on.goal!(p.activeSessionId!, 'set', objective) : undefined, [p.activeSessionId, on.goal, p.external, p.goal, p.goalActions]);
  // The capability arrives with the agent process, so a stored session reopening shows no button until the CLI is up.
  // Remember per agent whether its last ready session offered `set`, and hold the button's room while one starts
  useEffect(() => {
    if (p.status === 'ready') goalSetByAgent.set(p.agent.id, !!p.goalActions?.includes('set'));
  }, [p.status, p.agent.id, p.goalActions]);
  const goalRoom = !setGoal && p.status === 'starting' && !!on.goal && !p.external && !p.goal && goalSetByAgent.get(p.agent.id) === true;
  const stopAsyncTask = useMemo(() => p.activeSessionId && on.stopAsyncTask
    ? (taskId: string) => on.stopAsyncTask!(p.activeSessionId!, taskId) : undefined, [p.activeSessionId, on.stopAsyncTask]);
  // Diff sources of the session on screen; the inspector's diffs are looked up in the inspected subagent's transcript,
  // since a child may reuse its parent's tool call ids
  const fetchDiffSource = useMemo(() => p.activeSessionId && on.diffSource
    ? (toolCallId: string, nth: number) => on.diffSource!(p.activeSessionId!, toolCallId, nth) : undefined, [p.activeSessionId, on.diffSource]);
  const inspectId = inspect?.id;
  const fetchSubagentDiffSource = useMemo(() => p.activeSessionId && on.diffSource && inspectId !== undefined
    ? (toolCallId: string, nth: number) => on.diffSource!(p.activeSessionId!, toolCallId, nth, inspectId) : undefined,
  [p.activeSessionId, on.diffSource, inspectId]);
  // An AIR failure notice's actions map onto existing session actions: a real turn retries the prompt,
  // a session-scoped row (no live start) reconnects instead; the other two are the Notice's own verbs
  const lastTurnStarted = lastTurn?.role === 'agent' && lastTurn.startedAt !== undefined;
  const failureAction = useMemo(() => p.activeSessionId ? (action: FailureAction) => {
    if (action === 'login') on.login();
    else if (action === 'new_session') on.newSession(p.agent.id);
    else (lastTurnStarted ? on.retryTurn : on.retry)();
  } : undefined, [p.activeSessionId, p.agent.id, on.login, on.newSession, on.retryTurn, on.retry, lastTurnStarted]);
  const onPermission = useCallback((blockId: string, optionId: string) => {
    if (p.activeSessionId) on.permission(p.activeSessionId, blockId, optionId);
  }, [p.activeSessionId, on.permission]);
  // Per-turn actions (copy / fork / stats): memoized like the other contexts so a stream push does not re-render consumers;
  // external conversations have no ACP transcript to fork
  const turnActions = useMemo(() => p.activeSessionId ? {
    sessionId: p.activeSessionId, controls: p.controls, agents: p.agents, agentId: p.agent.id,
    fork: on.forkSession && !p.external
      ? (turnIndex: number, agent: AgentId) => on.forkSession!(p.activeSessionId!, turnIndex, agent === p.agent.id ? undefined : agent) : undefined,
  } : undefined, [p.activeSessionId, p.controls, p.agents, p.agent.id, on.forkSession, p.external]);

  return (
    <DesktopContext.Provider value={desktop}>
    <AppearanceContext.Provider value={a}>
      <ThemeContext.Provider value={p.theme}>
      <OpenSessionContext.Provider value={openSession}>
      <SessionTitleContext.Provider value={sessionTitle}>
      <ShellLayerContext.Provider value={root}>
      <BlobUrlContext.Provider value={blobUrl}>
      <OpenBlobContext.Provider value={openBlob}>
      <ToolFileActionsContext.Provider value={toolFileActions}>
      <AsyncTaskStopContext.Provider value={stopAsyncTask}>
      <DiffSourceContext.Provider value={fetchDiffSource}>
        <div
          ref={root}
          className={cn('acp-shell relative flex h-full min-h-0 w-full overflow-hidden', wide && 'acp-wide')}
          data-theme={p.theme}
          data-surface-host={p.host}
          data-agent={p.agent.id}
          {...appearanceDataAttrs(a)}
          {...lookAttrs(p.look)}
          // The desktop frame (desktop.css): the OS for the traffic lights' room, whether a docked column sits beside the sheet
          data-desktop={desktop || undefined}
          // The files layout (desktop.css, panel.css): explorer | stage | chat | session column
          data-layout={files ? 'files' : undefined}
          data-os={desktop ? desktopOS : undefined}
          data-caption={desktop && pageCaption ? '' : undefined}
          // A docked column beside the chat, open: on its left, or at the window's right edge in the files layout
          data-column={(desktop && position === 'left' && canDock && !columnHidden) || undefined}
          data-column-moving={columnMoving || undefined}
          // The docked subagent pane is open beside the chat (its sheet joint, header room)
          data-subagent-docked={dockedInspect || undefined}
          // A narrow window (below --session-dock-min): its list is a page over the whole window, the title row keeps only
          // the column's toggle and the header centres its title
          data-narrow={(desktop && !canDock) || undefined}
          data-drawer-open={(desktop && !canDock && drawerOpen) || undefined}
          // Narrow only, so the render that widens the window past the tier drops the transition with the transform: the docked
          // column lands in place, where a close moments before used to slide it in from the left (the smoke, 2026-10-23)
          data-drawer-moving={(desktop && !canDock && drawerMoving > 0) || undefined}
          // A plugin skin (styles/skin.css): its tokens come in the plugins' theme layer, its glass lets the backdrop through
          data-skin={skin?.plugin}
          data-glass={skin || leavingSkin ? (skin ?? leavingSkin!.skin).glass ?? 'veil' : undefined}
          // Not in the files layout: its panes derive their own surfaces from the canvas, which a backdrop would empty. A
          // backdrop leaving in a reveal without View Transitions keeps the surfaces clear until it is gone
          data-backdrop={((skin?.backdrop || leavingSkin) && backdropsShown && !files) || undefined}
        >
          {position !== 'hidden' && (
            <>
              {/* The theme clears Tailwind's own palette, so the scrim takes the dialogs' token, lighter (bg-black/20 drew nothing).
                  Desktop: the list page covers the window, and the sheet under it dims itself (data-sheet-dim) */}
              {!canDock && drawerOpen && !desktop && <div aria-hidden="true" className="absolute inset-0 z-10 bg-scrim/45" onClick={() => closeDrawer()} />}
              {/* The folded flag belongs to the docked column: a narrow window's drawer opens whatever it says */}
              <aside ref={sessionPanel} aria-label={t('session.listAria')} inert={canDock ? desktop && columnHidden : !drawerOpen}
                data-session-panel={position} data-docked={canDock} data-column-hidden={(desktop && canDock && columnHidden) || undefined}
                // On the column itself too: React writes a child's attributes before its parent's, so a flag only on the root
                // could land after the width changed and the width would jump
                data-moving={columnMoving || undefined}
                {...folderDrop}
                // Consumed, so no host takes the key as well (the macOS menu bar in Electron, VS Code's own Esc bindings)
                onKeyDown={e => { if (e.key === 'Escape' && !e.defaultPrevented && !canDock) { e.preventDefault(); e.stopPropagation(); closeDrawer(); } }}
                style={canDock && sideWidth !== undefined ? { width: `clamp(var(--session-pane-min), ${sideWidth}px, calc(100% - var(--session-main-min)))` } : undefined}
                className={cn(
                  'flex min-h-0 flex-col',
                  // Files layout: last, at the window's right edge, a hairline from the chat beside it; it gives way to the
                  // stage's room down to its narrowest (desktop.css)
                  files ? 'order-5 shrink border-l border-line' : desktop && dockedInspect ? 'shrink' : 'shrink-0',
                  position === 'right' && 'order-last',
                  // Desktop: the column has no fill or rule of its own, it lies on the window's canvas (desktop.css)
                  !desktop && cn('bg-bg-0 border-line', position === 'right' ? 'border-l' : 'border-r'),
                  // Desktop, narrow: a page over the whole window, laid out while closed so it can slide (desktop.css)
                  canDock ? 'relative w-pop-md' : desktop ? 'absolute inset-0 z-20' : 'absolute inset-y-0 z-20 w-pop-lg max-w-[calc(100%-var(--ctl))] shadow-pop',
                  !canDock && !desktop && (position === 'right' ? 'right-0' : 'left-0'),
                  !canDock && !drawerOpen && !desktop && 'hidden',
                  // A folder from the OS over the column: the list's drop frame, drawn inset
                  folderOver && 'ring-1 ring-inset ring-fg-1/70',
                )}>
                {canDock && (
                  <InspectorSplitter side={position === 'right' || files ? 'right' : 'left'} bounds={sideBounds} width={sideWidth}
                    onDrag={setDraggedSide} onCommit={commitSide} onReset={resetSide} />
                )}
                {desktop && <TitleSpace />}
                {!desktop && (!nav || !canDock) && (
                  <div className="flex h-hdr shrink-0 items-center justify-between gap-gap-half border-b border-line px-pad">
                    <span className="text-2 font-medium text-fg-strong">{t('session.history')}</span>
                    {!canDock && <IconButton onClick={() => closeDrawer()} aria-label={t('common.close')} title={t('common.close')}><X strokeWidth={1.5} /></IconButton>}
                  </div>
                )}
                {/* Docked nav: the first action row is centred on the chat header's line (desktop: the title row holds that line) */}
                <div className={cn('flex min-h-0 flex-1 flex-col p-gap-half', nav && canDock && !desktop && 'pt-[calc((var(--hdr)-var(--row))/2)]', desktop && 'pb-0')}>{sessionsPanel}</div>
                {/* Plugin pieces at the column's foot (plugins/Pieces.tsx), full width, above the settings gear */}
                <SlotPieces slot="sidebar" className="flex shrink-0 flex-col gap-gap-half px-gap-half pt-gap-half" pieceClassName="w-full" />
                {!files && gear}
                {!desktop && nav && p.onOpenSettings && (
                  <div className="shrink-0 border-t border-line p-gap-half">
                    <button type="button" onClick={() => p.onOpenSettings!()}
                      className="flex min-h-row w-full items-center gap-gap rounded-md px-2 text-left text-2 text-fg-1 transition-colors hover:bg-hover focus-visible:bg-hover">
                      <span className="flex size-lead shrink-0 items-center justify-center text-fg-2"><Settings2 className="size-icon" strokeWidth={1.5} /></span>
                      <span className="min-w-0 truncate">{t('session.settings')}</span>
                    </button>
                  </div>
                )}
              </aside>
            </>
          )}
          {/* Desktop, narrow: out of reach while the list page covers it */}
          <div ref={mainColumn} data-sheet={desktop || undefined} inert={(desktop && !canDock && drawerOpen) || undefined}
            // Files layout: the chat lies on the canvas right of the stage (desktop.css), as wide as the user made it. Beside
            // the docked tool panel its basis is what it keeps there: the panel shrinks first, to its narrowest (panel.css)
            className={cn('relative flex flex-col', desktop && dockedInspect ? 'min-w-(--chat-fit-min)' : files ? 'min-w-(--files-chat-min)' : 'min-w-0', files ? 'order-3 shrink' : cn('flex-1', (panelDocked || dockedInspect) && 'basis-(--tool-main-min)'))}
            style={files ? { width: chatWidth !== undefined ? `clamp(var(--files-chat-min), ${chatWidth}px, var(--files-chat-max))` : 'var(--files-chat-w)' } : undefined}>
            {files && <InspectorSplitter side="right" width={chatWidth} bounds={chatBounds} onDrag={setDraggedChat} onCommit={commitChat} onReset={resetChat} />}
            {/* Covered by the overlay inspector: out of the tab order and the accessibility tree meanwhile */}
            <div inert={overlayInspect || undefined} aria-hidden={overlayInspect || undefined} className="contents">
            <Header
              title={desktop && p.noProject ? '' : p.title}
              // Desktop start page: the project after the title is a dropdown that starts the empty session over there
              projectPick={start.shown && p.cwd ? <ProjectPick cwd={p.cwd} projects={startProjects} home={p.home} onPick={cwd => startOver(agentIn(cwd, p.agent.id) ?? p.agent.id, cwd)} onOpenOther={on.pickFolder ? addProject : undefined}
                chat={chatHere} chats={p.chats} onChat={p.chats && !chatHere ? startChat : undefined} /> : undefined}
              placeName={chatHere ? t('session.group.chats') : undefined}
              sessions={p.sessions}
              categories={p.categories}
              agent={p.agent}
              agents={p.agents}
              accounts={p.accounts}
              accountId={p.accountId}
              activeSessionId={p.activeSessionId}
              workspace={p.workspace}
              sessionScope={desktop ? 'all' : p.sessionScope}
              nativeSessions={p.nativeSessions}
              on={handlers}
              onToggleDrawer={toggleDrawer}
              drawerOpen={drawerOpen}
              narrow={desktop && !canDock}
              sessionPanel={position}
              sessionPanelDocked={canDock && !(desktop && columnHidden)}
              sessionNav={nav && canDock && !(desktop && columnHidden)}
              // Files layout: the gear stands under the explorer, so settings come to the header while that is folded
              gearDocked={files ? !explorerHidden : undefined}
              onOpenSettings={p.onOpenSettings}
              cwd={p.noProject ? undefined : p.cwd}
              onNewIn={desktop ? newHere : undefined}
            />
            </div>
            <div className="relative flex min-h-0 flex-1 flex-col">
              {desktop && p.noProject && <NoProject onOpen={on.pickFolder ? addProject : undefined} onChat={p.chats ? newChat : undefined} />}
              {/* The thread's toast stack is hidden with the thread: the start page shows the shell's own (an unknown session link) */}
              {desktop && p.noProject && <ShellToasts open={on.revealPath} />}
              {/* The overlay inspector replaces this block visually; it stays mounted (scroll + draft survive) but inert */}
              <div inert={overlayInspect || undefined} aria-hidden={overlayInspect || undefined} className={cn('flex min-h-0 flex-1 flex-col', desktop && p.noProject && 'hidden')}>
                <div className="relative flex min-h-0 flex-1 flex-col">
                  {!p.external && <QuoteToolbar root={threadContent} />}
                  <PlanDocumentContext.Provider value={planDoc}>
                    <HistoryContext.Provider value={history}>
                    <HistoryComposerContext.Provider value={history?.editing !== undefined ? composerProps : undefined}>
                      <OpenToolFileContext.Provider value={openToolFile}>
                        <TurnActionsContext.Provider value={turnActions}>
                          <Thread key={p.activeSessionId} session={p.activeSessionId} turns={held.turns} running={held.running} wide={wide} replayKey={p.replayKey} catchingUp={p.catchingUp} blobUrl={blobUrl} contentRef={contentRef} commands={p.commands} summons={summons}
                            onSetGoal={setGoal} goalRoom={goalRoom} subagents={p.subagents} onInspect={onInspect} onFailureAction={failureAction}
                            forkedFrom={p.external ? undefined : p.forkedFrom} agents={p.agents} agentId={p.agent.id}
                            onPermission={onPermission} />
                        </TurnActionsContext.Provider>
                      </OpenToolFileContext.Provider>
                    </HistoryComposerContext.Provider>
                    </HistoryContext.Provider>
                  </PlanDocumentContext.Provider>
                  <div
                    data-plan-dock
                    className={cn('pointer-events-none absolute inset-x-0 bottom-0 z-10', wide && a.composer === 'island' && 'mx-auto w-full max-w-[calc(var(--content-w)+2*var(--page))]')}
                  >
                    {/* Plugin pieces on the dock's top edge (plugins/ComposerAbove.tsx): over the plan card when there is
                        one, else over the question, the queue or the composer, never on top of their controls */}
                    {!p.external && <ComposerAbove running={p.running} waiting={waitingOnUser} />}
                    {/* A held prompt already owns a Working row; the floating startup row would duplicate and overlap it. */}
                    {!p.external && !held.running && <ConnectingNotice status={p.status} agent={p.agent} />}
                    <div ref={planDock}>
                      {/* The session goal heads the floating to-do card, so the dock keeps one silhouette */}
                      <PlanBar key={`plan:${p.activeSessionId}`} turns={p.turns} running={p.running}
                        goal={p.external ? undefined : p.goal} goalActions={p.goalActions} onGoalAction={goalAction} />
                    </div>
                  </div>
                  {/* Plugin rails in the margin beside the conversation column, only while it has room (plugins/Pieces.tsx) */}
                  {!p.external && <SessionRail />}
                  {(toasts.length > 0 || shellToasts.length > 0) && (
                    <div ref={toastRef} className="pointer-events-none absolute inset-x-0 bottom-[calc(var(--gap)+var(--thread-dock-height,0px))] z-10 flex flex-col items-center gap-1 px-page">
                      {toasts.map(t => <Toast key={t.key} text={t.text} icon={t.icon} ttl={t.ttl} onUndo={t.undo} onClose={() => dropToast(t.key)} />)}
                      {/* The desktop's host toasts, and on every host what the page raises itself (a plugin button failing) */}
                      <ShellToastItems open={on.revealPath} />
                    </div>
                  )}
                </div>
                {/* Every surface under the thread (question, alert, notice, queue, composer) shares the DockStack rhythm; none carries its own outer margin */}
                <DockStack className={cn(wide && a.composer === 'island' && 'mx-auto w-full max-w-[calc(var(--content-w)+2*var(--page))]')} data-start-dock={desktop || undefined}>
                  {question && on.answer && p.activeSessionId && <Questions key={question.id} block={question} subtitle={questionSubtitle} onAnswer={(blockId, answers, skip) => on.answer!(p.activeSessionId!, blockId, answers, skip)} />}
                  {alertTurn && (
                    <Alert
                      turn={alertTurn}
                      onRetry={on.retryTurn}
                      onCompact={p.commands?.some(c => c.name === 'compact') ? on.compact : undefined}
                      onContinue={() => on.send(t('alert.continueText'), [])}
                      onDismiss={() => setDismissedAlert(alertKey)}
                      onNewSession={() => on.newSession(p.agent.id)}
                    />
                  )}
                  {p.external ? <ExternalSessionNotice info={p.external} /> : <Notice
                    status={p.status} error={p.error} agent={p.agent} authMethods={p.authMethods}
                    accounts={p.accounts?.filter(x => x.agent === p.agent.id)} accountId={p.accountId}
                    accountAction={p.accountAction}
                    onLogin={on.login} onRetry={on.retry} onNewSession={() => on.newSession(p.agent.id)}
                    onSelectAccount={on.selectAccount} onAddAccount={via => on.addAccount(p.agent.id, via)}
                    onUnlock={on.unlockCredentials && (() => on.unlockCredentials?.(p.agent.id))}
                    onTakeOver={p.canTakeOver ? on.takeOver : undefined}
                  />}
                  {held.queued?.length && p.activeSessionId
                    ? <Queue key={`queue:${p.activeSessionId}`} items={held.queued} composer={composerProps} blobUrl={blobUrl}
                        on={on.dequeue && on.editQueued ? {
                          remove: id => on.dequeue!(p.activeSessionId!, id),
                          sendNow: on.sendQueued && (id => on.sendQueued!(p.activeSessionId!, id)),
                          steer: p.steerQueued && p.running && on.steerQueued ? id => on.steerQueued!(p.activeSessionId!, id) : undefined,
                          edit: (id, text, kept, drafts) => on.editQueued!(p.activeSessionId!, id, text, kept, drafts),
                        } : undefined} />
                    : null}
                  {/* Desktop start page: who answers, right above the composer; it stays mounted while the empty session is
                      started over with another agent, so the segments slide instead of the page replaying its entrance */}
                  {start.mounted && <StartPicker open={start.shown} agents={p.agents} current={p.agent.id} only={startOnly} onPick={agent => startOver(agent)} />}
                  {/* Sibling keys include the component role; duplicate session-only keys leave stale queue rows after reconciliation. */}
                  {!p.external && <Composer resetKey={p.activeSessionId} {...composerProps} draftKey={p.activeSessionId} main shareSelection={p.shareEditorSelection}
                    toolbarStart={!!p.subagents?.length && <Chip narrow="icon" caret={false} className="shrink-0" icon={<Network strokeWidth={1.5} />}
                      aria-label={`${t('subagents.graph')} · ${t('subagents.entry', { n: p.subagents.length })}`}
                      title={`${t('subagents.graph')} · ${t('subagents.entry', { n: p.subagents.length })}`}
                      aria-haspopup="dialog" aria-expanded={graphOpen} onClick={() => setGraphOpen(true)}>
                      {t('subagents.entry', { n: p.subagents.length })}
                    </Chip>} />}
                </DockStack>
              </div>
            </div>
            {/* The overlay inspector takes the whole column, header included, so the child's title replaces the session's */}
            {overlayInspect && inspect !== undefined && inspectNode !== undefined && p.activeSessionId !== undefined && (
              <div data-subagent-panel="overlay" className="absolute inset-0 z-20 flex flex-col bg-bg-0">
                {/* Outside the thread's providers: file references in the child's rows open like the thread's own */}
                <OpenToolFileContext.Provider value={openToolFile}>
                <SubagentInspector
                  mode="overlay"
                  node={inspectNode}
                  transcript={p.subagentTranscripts?.[`${p.activeSessionId}:${inspect.id}`]}
                  onClose={() => setInspect(undefined)}
                  onCancel={inspectNode.controls.cancel && on.cancelSubagent ? () => on.cancelSubagent!(p.activeSessionId!, inspectNode.id) : undefined}
                  onPermission={(blockId, optionId) => on.permission(p.activeSessionId!, blockId, optionId)}
                  question={inspectNode.question}
                  onAnswer={on.answer ? (blockId, answers, skip) => on.answer!(p.activeSessionId!, blockId, answers, skip) : undefined}
                  wide={wide}
                  blobUrl={blobUrl}
                  diffSource={fetchSubagentDiffSource}
                />
                </OpenToolFileContext.Provider>
              </div>
            )}
            {/* Desktop, narrow: the sheet dims as the list page slides over it, the dim moving with the sheet (its corners) */}
            {desktop && !canDock && <div data-sheet-dim aria-hidden="true" className="pointer-events-none absolute inset-0 z-40 bg-scrim/45" />}
          </div>
          {dockedInspect && inspect !== undefined && (
            // Desktop: its card (beside the sheet) or its place on the canvas (files layout) comes from desktop.css
            <aside ref={inspectorPane} data-subagent-panel="docked" className={cn('relative flex w-subagent-pane shrink-0 flex-col', !desktop && 'border-l border-line bg-bg-0', files ? 'order-4' : 'order-last')}
              style={paneWidth !== undefined ? { width: `clamp(var(--subagent-pane-min), ${paneWidth}px, calc(100% - var(--subagent-main-min)))` } : undefined}>
              <InspectorSplitter width={paneWidth ?? paneBounds().initial} bounds={paneBounds}
                onDrag={setDraggedPane}
                onCommit={w => { setDraggedPane(undefined); setPreferredPane(w); }}
                onReset={() => { setDraggedPane(undefined); setPreferredPane(undefined); }} />
              <OpenToolFileContext.Provider value={openToolFile}>
              <SubagentInspector
                mode="docked"
                node={inspectNode}
                transcript={p.subagentTranscripts?.[`${p.activeSessionId}:${inspect.id}`]}
                onClose={() => setInspect(undefined)}
                onCancel={inspectNode.controls.cancel && on.cancelSubagent ? () => on.cancelSubagent!(p.activeSessionId!, inspectNode.id) : undefined}
                onPermission={(blockId, optionId) => on.permission(p.activeSessionId!, blockId, optionId)}
                wide={wide}
                blobUrl={blobUrl}
                diffSource={fetchSubagentDiffSource}
              />
              </OpenToolFileContext.Provider>
            </aside>
          )}
          {panelRoot && (files
            ? <FilesLayout sessionId={p.activeSessionId!} root={panelRoot} running={p.running} searchFiles={on.searchFiles} openEditor={openInEditor}
                openPlan={openPlanInEditor} shareSelection={p.shareEditorSelection} foot={gear} sessionsShown={!columnHidden} onToggleSessions={toggleSessions} />
            : <ToolPanelDock sessionId={p.activeSessionId} cwd={panelRoot} running={p.running} narrow={!canDock} sheet={mainColumn}
                searchFiles={on.searchFiles} openEditor={openInEditor} openPlan={openPlanInEditor} shareSelection={p.shareEditorSelection} />)}
          {/* Last, so it stays over the drawer of a narrow window. The files layout's left column is the explorer */}
          {desktop && <TitleRow onBack={trail.back} onForward={trail.forward}
            column={files
              ? { open: !explorerHidden, drawer: false, onToggle: () => panel.toggleExplorer() }
              : { open: canDock ? !columnHidden : drawerOpen, drawer: !canDock, onToggle: () => canDock ? hideColumn(!columnHidden) : drawerOpen ? closeDrawer() : openDrawer() }} />}
        </div>
        <SubagentGraph nodes={p.subagents ?? []} sessionTitle={p.title} open={graphOpen && !!p.subagents?.length} onOpenChange={setGraphOpen} onInspect={onInspect} selectedId={inspect?.id} />
        {filePreview && (
          <Lightbox
            src={filePreview.src}
            name={filePreview.name}
            mimeType={filePreview.mimeType}
            path={filePreview.path}
            openInEditor={openInEditor ? () => openInEditor(filePreview.path) : undefined}
            onClose={() => setFilePreview(null)}
          />
        )}
        {remotePick && <RemoteFolderDialog alias={remotePick} onClose={() => setRemotePick(undefined)}
          onAdd={folder => { setRemotePick(undefined); openProject(folder); }} />}
      </DiffSourceContext.Provider>
      </AsyncTaskStopContext.Provider>
      </ToolFileActionsContext.Provider>
      </OpenBlobContext.Provider>
      </BlobUrlContext.Provider>
      </ShellLayerContext.Provider>
      </SessionTitleContext.Provider>
      </OpenSessionContext.Provider>
      </ThemeContext.Provider>
    </AppearanceContext.Provider>
    </DesktopContext.Provider>
  );
}

interface ThreadProps {
  // The session shown, for what plugin frames hear of it (plugins/outline.ts)
  session?: string;
  turns: Turn[];
  running: boolean;
  wide: boolean;
  replayKey?: number | string;
  catchingUp?: boolean;
  blobUrl?: (blob: string) => string;
  contentRef?: (el: HTMLDivElement | null) => void;
  // Advertised slash commands: sent user messages paint their `/name` tokens like the composer does
  commands?: SlashCommand[];
  // Enabled subagent persona names: sent user messages paint their `@name` tokens like the composer does
  summons?: readonly string[];
  subagents?: SubagentSummary[];
  // The fork boundary: drawn after the exchange holding the last copied turn
  forkedFrom?: ForkOrigin;
  agents: AgentInfo[];
  agentId: AgentId;
  onInspect?: (id: string) => void;
  onPermission: (blockId: string, optionId: string) => void;
  onFailureAction?: (action: FailureAction) => void;
  // Offers "Set as goal" under the last typed prompt (absent: no goal can be set now)
  onSetGoal?: (objective: string) => void;
  // The button is expected once the starting agent reports its goal capability: keep its room meanwhile
  goalRoom?: boolean;
}

// The thread's tail clearance under the floating plan dock: the dock's height on top of the message gap, so the
// last message sits one message gap from the first dock surface whether or not the plan is showing. Empty falls
// back to the class (`pb-msg`)
function threadClearance(dockHeight: number) {
  return dockHeight ? `calc(var(--msg-gap) + ${dockHeight}px)` : '';
}

// Entrance stagger caps out at the 12th block, so long sessions don't take seconds
const STAGGER_CAP = 12;

// Long transcripts mount the tail exchanges first so the viewport paints within one frame on session switch,
// then prepend earlier exchanges in frame slices while keeping the scroll position anchored. A slice is a count of
// exchanges, and in the largest transcripts one exchange can hold hundreds of KB of tool output: 14 at a time made the
// frames after a switch to a 13 MB session take ~45 ms in WebKit, 6 keep them near 25 ms
const EXCHANGE_WINDOW_THRESHOLD = 10;
const INITIAL_EXCHANGES = 8;
const EXCHANGE_SLICE = 6;

function canWindowExchanges(count: number): boolean {
  return count > EXCHANGE_WINDOW_THRESHOLD
    && typeof requestAnimationFrame === 'function'
    && !/jsdom/i.test(globalThis.navigator?.userAgent ?? '');
}

function useExchangeWindow(
  count: number,
  scrollerRef: RefObject<HTMLDivElement | null>,
  follow: { readonly value: boolean },
  opening: { readonly current: boolean },
) {
  const [start, setStart] = useState(() => canWindowExchanges(count) ? count - INITIAL_EXCHANGES : 0);
  const slicing = useRef(false);
  const [quiet] = useState(() => ({ get current() { return opening.current || slicing.current; } }));
  const before = useRef<SliceScroll | undefined>(undefined);

  useEffect(() => {
    if (start <= 0) return;
    let frame2 = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // Wait until the first frame has painted before prepending earlier slices so the switch paint is never delayed by background mounts.
    const frame1 = requestAnimationFrame(() => {
      frame2 = requestAnimationFrame(() => {
        timer = setTimeout(() => {
          const scroller = scrollerRef.current;
          before.current = scroller ? captureSliceScroll(scroller, follow.value) : undefined;
          slicing.current = true;
          flushSync(() => setStart(cur => Math.max(0, cur - EXCHANGE_SLICE)));
        }, 0);
      });
    });
    return () => {
      cancelAnimationFrame(frame1);
      if (frame2) cancelAnimationFrame(frame2);
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [start, scrollerRef, follow]);

  useLayoutEffect(() => {
    slicing.current = false;
    const kept = before.current;
    before.current = undefined;
    if (!kept) return;
    restoreSliceScroll(kept);
  }, [start]);

  const effectiveStart = start > 0 ? Math.min(start, Math.max(0, count - INITIAL_EXCHANGES)) : 0;
  // Mount every exchange from `k` on now (a plugin's scrollTo an exchange not reached yet)
  const showFrom = useCallback((k: number) => { if (k >= 0) setStart(cur => Math.min(cur, k)); }, []);
  return { start: effectiveStart, quiet, showFrom };
}

// Conversation flow: stick-to-bottom following happens on transcript changes (new content / streaming growth) and on async growth
// (images, diagrams) until the user touches the thread; user actions like expand / collapse never touch the scroll position —
// the toggle under the mouse stays put while the content below it moves. Scrolling away from the bottom releases the follow; scrolling back to the bottom restores it
// Whether each agent's last ready session offered "Set as goal" (see goalRoom in Shell)
const goalSetByAgent = new Map<AgentId, boolean>();

const Thread = memo(function Thread({ session, turns, running, wide, replayKey, catchingUp, blobUrl, contentRef, commands, summons, subagents, forkedFrom, agents, agentId, onInspect, onPermission, onFailureAction, onSetGoal, goalRoom }: ThreadProps) {
  useLocale();
  const ref = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const bodyRef = useMergedRefs(contentRef, body);
  const follow = useBottomFollow(ref, body, replayKey, [turns, running]);
  // Fade the lower edge while the reader is scrolled away from the bottom (thread-fade): text leaving the view above a
  // borderless dock row (the connecting notice) dissolves instead of being cut off flat. Driven by the follow state rather
  // than useScrollFade, whose content observer would flash the fade for a frame on every streamed chunk before the re-pin
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const sync = () => el.toggleAttribute('data-more-below', !follow.value);
    sync();
    return follow.subscribe(sync);
  }, [follow]);
  // The transcript's first commit restores what the session already holds: its rows appear in place, without entrances.
  // A switch back to a kept session mounts the kept copy first, and the host's catch-up for the time away lands a round
  // trip later: that commit is still quiet. The flag is read during render, before this effect of the same commit runs,
  // so the commit in which `catchingUp` turns false still sees it set; anything after that animates as live output
  const opening = useRef(true);
  useLayoutEffect(() => { opening.current = !!catchingUp; });

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
  const exchangeSlots: { key: number; items: { ti: number; index: number }[] }[] = [];
  turns.forEach((turn, ti) => {
    const previous = turns[ti - 1];
    if (planExecutionId(turn, previous)) return;
    const index = Math.min(i, STAGGER_CAP);
    i += turn.role === 'agent' ? turn.blocks.length + 1 : 1;
    if (!exchangeSlots.length || (turn.role === 'user' && !turn.auto)) {
      exchangeSlots.push({ key: ti, items: [] });
    }
    exchangeSlots[exchangeSlots.length - 1]!.items.push({ ti, index });
  });
  const window = useExchangeWindow(exchangeSlots.length, ref, follow, opening);
  const starts = exchangeSlots.map(e => e.key);
  useSessionOutline(ref, session, turns, starts, running, { mount: turn => window.showFrom(starts.indexOf(turn)), release: () => follow.release() });
  // The prompt "Set as goal" hangs under: the last one the user typed, unless it was a slash command
  const goalPrompt = onSetGoal || goalRoom ? lastTypedPrompt(turns) : undefined;
  // The fork boundary follows the exchange that holds the last copied turn, as a row of its own: inside that exchange it
  // would stand after the reply's action row, which the exchange spacing keys off
  const forkAfter = forkedFrom ? exchangeSlots.filter(e => e.key <= forkedFrom.turnIndex).length - 1 : -1;
  // The right padding gives the scrollbar gutter back, so the transcript's edges line up with the header and the dock.
  // Focusable (tabIndex -1) so a press in the transcript takes focus off the composer: VS Code's webview runs ⌘C as
  // execCommand('copy') after refocusing the last focused element whenever the body holds focus (pre/index.html, 1.143),
  // which moved the selection into the composer and copied nothing
  return (
    <div ref={ref} data-thread tabIndex={-1} className="thread-fade scroll-stable min-h-0 min-w-0 flex-1 overflow-y-auto pl-dock pr-dock-gutter outline-none [overflow-anchor:none]">
      {/* The tail clearance equals the message gap, so the last message sits as far from the composer as from the message above it */}
      <div key={replayKey} ref={bodyRef} className={cn('mx-auto flex flex-col gap-msg pt-pad-y pb-msg', wide && 'max-w-chat-column')}><FollowContext.Provider value={follow}><QuietEntranceContext.Provider value={window.quiet}>
        {exchangeSlots.slice(window.start).map((slot, offset) => {
          const k = window.start + offset;
          const messages: ReactNode[] = [];
          for (const { ti, index } of slot.items) {
            const turn = turns[ti]!;
            const previous = turns[ti - 1];
            const compacting = previous?.role === 'user' && /^\/compact(?:\s|$)/.test(previous.text.trim());
            // Fold memory key: the session plus the turn's position and start time, so an edited-away turn at the same index does not inherit a choice
            const memoryKey = replayKey !== undefined ? `${replayKey}:${ti}:${turn.role === 'agent' ? turn.startedAt ?? '' : ''}` : undefined;
            // Session-wide nodes only reach turns that announce one; handing the list to every
            // memoized turn would re-render all of them on each child activity tick.
            const mine = turn.role === 'agent' ? byTurn.get(ti) : undefined;
            // An automatic account switch or a Retry continues the cut-off turn: the pair reads as one reply, with the switch
            // notice or the error row as the only seam, so the cut-off half drops its action row and the continuation joins it
            const next = turns[ti + 1];
            const switchedAway = next?.role === 'user' && isContinue(next);
            // The reply woken by summoned agents' reports hugs the one-line report row above it, like a continuation
            const switchedIn = previous?.role === 'user' && (isContinue(previous) || previous.autoReason === 'subagents');
            // A user turn carries no time of its own; the reply it started began the moment it went out (absent on old transcripts)
            const sentAt = turn.role === 'user' && next?.role === 'agent' ? next.startedAt : undefined;
            // The strip under a prompt: its send time and, on the last typed one, Set as goal (both shown on hover)
            const goalSlot = ti === goalPrompt && turn.role === 'user';
            const goal = goalSlot && onSetGoal ? () => onSetGoal(turn.text.trim()) : undefined;
            if (turn.role === 'user' && !turn.auto && (sentAt !== undefined || goalSlot)) {
              messages.push(
                <HistoryMessage key={turn.id ?? ti} turn={turn} turnIndex={ti} index={index} blobUrl={blobUrl} commands={commands} summons={summons} />,
                <PromptFooter key={`footer${ti}`} sentAt={sentAt} onSetGoal={goal} goalRoom={goalSlot} />,
              );
              continue;
            }
            const last = ti === turns.length - 1;
            messages.push(turn.role === 'user'
              ? <HistoryMessage key={turn.id ?? ti} turn={turn} turnIndex={ti} index={index} blobUrl={blobUrl} commands={commands} summons={summons} />
              : <AgentMessage key={ti} turn={turn} index={index} compacting={compacting} running={running && ti === activeAgentIndex && !turn.stop} onPermission={onPermission} memoryKey={memoryKey}
                  turnIndex={ti} last={last} settings={previous?.role === 'user' ? previous.settings : undefined} actions={!switchedAway} joined={switchedIn}
                  subagents={mine} allSubagents={mine ? subagents : undefined} onInspect={onInspect} onFailureAction={last ? onFailureAction : undefined} />);
          }
          return <Fragment key={slot.key}>{
            // Positioned so the prompt's stuck-state sentinel can sit at the exchange's top edge. Paint containment gives each exchange
            // its own paint offset, so a fold opening mid-thread no longer re-walks every later exchange each frame (see docs/dev/webview.md,
            // transcript render budget); the clip it brings is pushed out by --hit on the left and bottom, where row hit areas and
            // card shadows reach past the column, and on the right only as far as the thread's right padding (--hit-end), and the top edge stays put for the sentinel. An exchange ending in a reply's action
            // row pulls the next one up to a row gap: the row's own height already separates them (TurnActions).
            <section data-exchange data-exchange-start={slot.key} className="relative -ml-hit -mr-hit-end -mb-hit flex min-w-0 flex-col gap-msg pl-hit pr-hit-end pb-hit contain-paint has-[>:last-child>[data-turn-actions]:last-child]:-mb-[calc(var(--spacing-hit)+var(--spacing-msg-join))]">
              {messages}
            </section>
          }{k === forkAfter && forkedFrom && <ForkBoundary origin={forkedFrom} agents={agents} agentId={agentId} />}</Fragment>;
        })}
      </QuietEntranceContext.Provider></FollowContext.Provider></div>
    </div>
  );
});

// A hidden turn that carries the turn before it on (after an account switch, Retry after a failure with output, the
// workspace gate sending its findings back, or an active goal's next turn)
function isContinue(turn: UserTurn): boolean {
  return turn.auto === true && (turn.autoReason === 'accountSwitch' || turn.autoReason === 'retry' || turn.autoReason === 'gate' || (turn.autoReason === 'goal' && turn.goalContinue === true));
}

// Desktop: how long the column's width eases; how long a new project stays lit
const COLUMN_MOVE_MS = 400;
// Outlasts the list page's 320 ms opening (desktop.css): a transition is cut short once its property leaves `transition`
const DRAWER_MOVE_MS = 450;
const FRESH_MS = 2400;
