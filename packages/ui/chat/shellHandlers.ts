import type { AgentId, AgentInfo, CategoryOp, Draft, FullDiffSource, GoalAction, ImageDraft, NativeSessionInfo, QuestionAnswers } from '@acpira/shared/transcript';
import type { AddAccountVia, EditTurnRequest, ExportFormat, FileHit, SessionHit } from '@acpira/shared/protocol';
import type { MoveTarget } from './sessionTree';
import type { SessionCopyKind } from './SessionMenu';

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
  // The question card was closed: answers keyed by question id, or skip; `images` by question id for a card that takes them
  answer?: (sessionId: string, blockId: string, answers: QuestionAnswers, skip?: boolean, images?: Record<string, ImageDraft[]>) => void;
  buildPlan?: (sessionId: string, planId: string, model?: { configId: string; value: string }, optionId?: string) => void;
  openPlan?: (sessionId: string, planId: string) => void;
  openFile?: (sessionId: string, path: string, line?: number, reveal?: boolean) => void;
  // Stage a local/remote image file into the session's blob store for the Lightbox
  // shown: a file the session's transcript names (a row's image, video or widget), staged only when it does
  imageBlob?: (sessionId: string, path: string, shown?: boolean) => Promise<string | undefined>;
  // An agent-emitted image's blob file opens in the editor by its store name (the host resolves the absolute path)
  openBlob?: (sessionId: string, name: string) => void;
  setMode: (id: string) => void;
  setConfig: (configId: string, value: string, also?: Record<string, string>) => void;
  selectSession: (id: string) => void;
  // Without an agent the host falls back to the configured defaultAgent; with a category the new session is filed there;
  // `cwd` (a project shell, the desktop app) starts it in that folder; `chat` in a fresh folder of its own (no project)
  newSession: (agent?: AgentInfo['id'], category?: string, cwd?: string, chat?: boolean) => void;
  renameSession: (id: string, title: string) => void;
  // Resolves only when deletion commits; true means this engine can undo it. Errors are shown by the host.
  deleteSession: (id: string) => Promise<boolean> | void;
  restoreSession: (id: string) => void;
  pinSession: (id: string, pinned: boolean) => void;
  // Archive a session (left out of the list until its archived filter is on) or bring it back
  archiveSession?: (id: string, archived: boolean) => void;
  // The row menu's "mark as unread / read"
  setSessionUnread?: (id: string, unread: boolean) => void;
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
  // AgentInfo.accountKeys: a pasted API key as an account, from the sign-in notice
  addAccountKey?: (agent: AgentInfo['id'], key: string) => void;
  removeAccount: (id: string) => void;
  // An account list opened: re-read the quotas of that agent's accounts (`manual` for an explicit user refresh click)
  refreshQuota?: (agent: AgentInfo['id'], manual?: boolean) => void;
  // AgentInfo.credentialsLocked: unlock the credential store in a host terminal
  unlockCredentials?: (agent: AgentInfo['id']) => void;
  // Run the agent on this machine from now on, in folders on SSH hosts (`sshLocalAgents`) or, where the host of `cwd`
  // keeps it on itself, on that host only (`ConnectionView.localAgents`), and start a session here in `cwd`
  runHere?: (agent: AgentInfo['id'], cwd: string) => void;
  installAgent?: (agent: AgentInfo['id'], installNode?: boolean) => void;
  cancelInstall?: (agent: AgentInfo['id']) => void;
  installInTerminal?: (agent: AgentInfo['id']) => void;
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
  // End one running command and leave the turn running (SessionView.canStopCommands); `subagentId` names the child
  // transcript the row was shown in, absent for the session's own turns
  stopCommand?: (sessionId: string, toolCallId: string, subagentId?: string) => void;
  // Goal extension controls (SessionView.goalActions); `set` carries the objective
  goal?: (sessionId: string, action: GoalAction, objective?: string) => void;
  // The desktop app: the system folder dialog (the folder picked, undefined when dismissed), a folder shown in the OS file
  // manager, a web page opened in the browser
  pickFolder?: () => Promise<string | undefined>;
  revealPath?: (path: string, reveal?: boolean) => void;
  openExternal?: (url: string) => void;
}
